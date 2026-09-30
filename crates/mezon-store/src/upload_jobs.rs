use std::path::PathBuf;
use std::sync::Mutex;

use mezon_client::transport::{OutgoingEmoji, OutgoingHashtag, OutgoingMention};
use mezon_client::{AttachmentUploadOutcome, ResumableUpload};
use serde::{Deserialize, Serialize};

use crate::ids::UserId;
use crate::presign::PRESIGN_PENDING_MAX_AGE_SEC;

const PRESIGNED_URL_LIFETIME_SEC: i64 = 15 * 60;
const FILE_NAME: &str = "upload_jobs.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadJob {
    pub user_id: UserId,
    pub clan_id: i64,
    pub channel_id: i64,
    pub topic_id: i64,
    pub message_id: i64,
    pub mode: i32,
    pub is_public: bool,
    pub content: String,
    pub mentions: Vec<OutgoingMention>,
    pub hashtags: Vec<OutgoingHashtag>,
    pub emojis: Vec<OutgoingEmoji>,
    pub create_time_seconds: u32,
    pub started_at: i64,
    pub finished: Vec<String>,
    pub pending: Vec<PendingUpload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpload {
    pub key: String,
    pub upload: ResumableUpload,
}

impl UploadJob {
    pub fn is_topic(&self) -> bool {
        self.topic_id != 0
    }

    pub fn pending_keys(&self) -> Vec<String> {
        self.pending.iter().map(|p| p.key.clone()).collect()
    }

    pub fn resumable_at(&self, now: i64) -> bool {
        now - self.started_at < PRESIGN_PENDING_MAX_AGE_SEC
    }

    pub fn worth_keeping_at(&self, now: i64) -> bool {
        now - self.started_at < PRESIGNED_URL_LIFETIME_SEC
    }

    pub fn record(&mut self, outcome: &AttachmentUploadOutcome) {
        let (key, uploaded) = match outcome {
            AttachmentUploadOutcome::Uploaded(key) => (key, true),
            AttachmentUploadOutcome::Failed(key) => (key, false),
        };
        self.pending.retain(|p| &p.key != key);
        if uploaded && !self.finished.contains(key) {
            self.finished.push(key.clone());
        }
    }
}

#[derive(Debug, Default)]
pub struct RestorePlan {
    pub replaced: Vec<i64>,
    pub keep: Vec<UploadJob>,
    pub resume: Vec<UploadJob>,
}

pub fn plan_restore(
    saved: Vec<UploadJob>,
    user_id: UserId,
    now: i64,
    is_running: impl Fn(&UploadJob) -> bool,
) -> RestorePlan {
    let mut plan = RestorePlan::default();
    for job in saved {
        if is_running(&job) || !job.worth_keeping_at(now) {
            continue;
        }
        plan.replaced.push(job.message_id);
        if job.user_id != user_id {
            plan.keep.push(job);
        } else if job.resumable_at(now) {
            plan.resume.push(job);
        }
    }
    plan
}

pub fn prune(jobs: &mut Vec<UploadJob>, now: i64, is_running: impl Fn(&UploadJob) -> bool) {
    jobs.retain(|job| job.worth_keeping_at(now) || is_running(job));
}

fn jobs_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mezon")
        .join(FILE_NAME)
}

pub fn load() -> Vec<UploadJob> {
    if cfg!(test) {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(jobs_path()) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        tracing::warn!(%error, "saved upload jobs are unreadable; starting without them");
        Vec::new()
    })
}

static LAST_WRITTEN: Mutex<u64> = Mutex::new(0);

pub fn save(jobs: &[UploadJob], generation: u64) {
    if cfg!(test) {
        return;
    }
    let mut last_written = LAST_WRITTEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if generation <= *last_written {
        return;
    }
    *last_written = generation;
    let path = jobs_path();
    if jobs.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Err(error) = write_private(&path, jobs) {
        tracing::warn!(%error, "could not save upload jobs");
    }
}

fn write_private(path: &PathBuf, jobs: &[UploadJob]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(jobs)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(started_at: i64) -> UploadJob {
        UploadJob {
            user_id: UserId(7),
            clan_id: 1,
            channel_id: 2,
            topic_id: 0,
            message_id: 3,
            mode: 2,
            is_public: true,
            content: "clip".into(),
            mentions: Vec::new(),
            hashtags: Vec::new(),
            emojis: Vec::new(),
            create_time_seconds: 0,
            started_at,
            finished: vec!["a".into()],
            pending: Vec::new(),
        }
    }

    #[test]
    fn a_job_resumes_only_inside_the_presign_window() {
        let job = job(1_000);
        assert!(job.resumable_at(1_000 + PRESIGN_PENDING_MAX_AGE_SEC - 1));
        assert!(!job.resumable_at(1_000 + PRESIGN_PENDING_MAX_AGE_SEC));
    }

    #[test]
    fn a_job_is_kept_while_its_presigned_urls_still_work() {
        let job = job(1_000);
        assert!(job.worth_keeping_at(1_000 + PRESIGNED_URL_LIFETIME_SEC - 1));
        assert!(!job.worth_keeping_at(1_000 + PRESIGNED_URL_LIFETIME_SEC));
    }

    #[test]
    fn an_upload_moves_its_key_to_finished_once() {
        let mut job = job(0);
        let upload: ResumableUpload = serde_json::from_str(
            r#"{"plan":{"Single":{"put_url":"u","path":"/p","content_type":"video/mp4"}}}"#,
        )
        .expect("plan");
        job.pending.push(PendingUpload {
            key: "b".into(),
            upload,
        });
        job.record(&AttachmentUploadOutcome::Uploaded("b".into()));
        job.record(&AttachmentUploadOutcome::Uploaded("b".into()));
        assert!(job.pending.is_empty());
        assert_eq!(job.finished, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn a_failed_upload_leaves_the_job_without_finishing_it() {
        let mut job = job(0);
        job.record(&AttachmentUploadOutcome::Failed("a2".into()));
        assert_eq!(job.finished, vec!["a".to_string()]);
    }

    fn job_for(user: i64, message_id: i64, started_at: i64) -> UploadJob {
        UploadJob {
            user_id: UserId(user),
            message_id,
            ..job(started_at)
        }
    }

    #[test]
    fn restoring_resumes_only_the_signed_in_users_fresh_jobs() {
        let now = 10_000;
        let saved = vec![
            job_for(7, 1, now - 60),
            job_for(7, 2, now - PRESIGN_PENDING_MAX_AGE_SEC - 1),
            job_for(8, 3, now - 60),
            job_for(7, 4, now - PRESIGNED_URL_LIFETIME_SEC),
            job_for(7, 5, now - 30),
        ];
        let plan = plan_restore(saved, UserId(7), now, |job| job.message_id == 5);
        let ids = |jobs: &[UploadJob]| jobs.iter().map(|j| j.message_id).collect::<Vec<_>>();
        assert_eq!(ids(&plan.resume), vec![1]);
        assert_eq!(ids(&plan.keep), vec![3]);
        assert_eq!(plan.replaced, vec![1, 2, 3]);
    }

    #[test]
    fn pruning_drops_jobs_whose_urls_lapsed_unless_still_running() {
        let now = 10_000;
        let mut jobs = vec![
            job_for(7, 1, now - 60),
            job_for(8, 2, now - PRESIGNED_URL_LIFETIME_SEC),
            job_for(7, 3, now - PRESIGNED_URL_LIFETIME_SEC - 5),
        ];
        prune(&mut jobs, now, |job| job.message_id == 3);
        let ids: Vec<i64> = jobs.iter().map(|j| j.message_id).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn a_job_survives_a_json_round_trip() {
        let original = job(42);
        let json = serde_json::to_string(std::slice::from_ref(&original)).expect("serialize");
        let restored: Vec<UploadJob> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].message_id, original.message_id);
        assert_eq!(restored[0].user_id, original.user_id);
        assert_eq!(restored[0].started_at, 42);
        assert_eq!(restored[0].finished, original.finished);
    }
}
