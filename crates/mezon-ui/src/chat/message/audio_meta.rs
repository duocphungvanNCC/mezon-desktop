use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::{App, AppContext, Context, Entity, Global, SharedString, http_client::HttpClient};
use mezon_store::MessageAttachment;

const AUDIO_META_MAX_BYTES: usize = 32 * 1024 * 1024;
const AUDIO_META_CHUNK: usize = 64 * 1024;

struct GlobalAudioMetaCache(Entity<AudioMetaCache>);

impl Global for GlobalAudioMetaCache {}

#[derive(Clone, Copy)]
struct AudioMeta {
    duration: f64,
    size: u64,
}

pub struct AudioMetaCache {
    known: HashMap<String, AudioMeta>,
    pending: HashSet<String>,
    failed: HashSet<String>,
}

impl AudioMetaCache {
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(global) = cx.try_global::<GlobalAudioMetaCache>() {
            return global.0.clone();
        }
        let entity = cx.new(|_| Self {
            known: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
        });
        cx.set_global(GlobalAudioMetaCache(entity.clone()));
        entity
    }

    fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalAudioMetaCache>()
            .map(|global| global.0.clone())
    }

    pub fn ensure_attachments(attachments: &[MessageAttachment], cx: &mut App) {
        let urls = attachments
            .iter()
            .filter(|att| attachment_needs_audio_probe(att))
            .map(|att| att.url.clone())
            .collect();
        Self::ensure_urls(urls, cx);
    }

    pub fn ensure_urls(urls: Vec<String>, cx: &mut App) {
        if urls.is_empty() {
            return;
        }
        let missing = match Self::try_global(cx) {
            Some(cache) => urls
                .into_iter()
                .filter(|url| !cache.read(cx).contains(url))
                .map(SharedString::from)
                .collect::<Vec<_>>(),
            None => urls.into_iter().map(SharedString::from).collect(),
        };
        if missing.is_empty() {
            return;
        }
        Self::global(cx).update(cx, |cache, cx| {
            for url in missing {
                cache.ensure(url, cx);
            }
        });
    }

    fn contains(&self, url: &str) -> bool {
        self.known.contains_key(url) || self.pending.contains(url) || self.failed.contains(url)
    }

    fn ensure(&mut self, url: SharedString, cx: &mut Context<Self>) {
        let key = url.to_string();
        if key.is_empty() || self.contains(&key) {
            return;
        }
        self.pending.insert(key.clone());
        let client = cx.http_client();
        cx.spawn(async move |this, cx| {
            let probed = cx
                .background_executor()
                .spawn(async move { probe_remote(client, &url).await })
                .await;
            let _ = this.update(cx, |cache, cx| {
                cache.pending.remove(&key);
                match probed {
                    Ok(meta) => {
                        cache.known.insert(key, meta);
                        cx.notify();
                    }
                    Err(err) => {
                        tracing::warn!("audio metadata probe failed: {err}");
                        cache.failed.insert(key);
                    }
                }
            });
        })
        .detach();
    }
}

pub(crate) fn display_audio_duration(att: &MessageAttachment, cx: &App) -> f64 {
    if att.duration > 0 {
        return att.duration.max(0) as f64;
    }
    lookup(cx, &att.url)
        .map(|meta| meta.duration)
        .filter(|duration| *duration > 0.0)
        .unwrap_or(0.0)
}

pub(crate) fn display_attachment_bytes(att: &MessageAttachment, cx: &App) -> u64 {
    if att.size > 0 {
        return att.size;
    }
    if att.is_audio() {
        return lookup(cx, &att.url)
            .map(|meta| meta.size)
            .filter(|size| *size > 0)
            .unwrap_or(0);
    }
    att.size
}

pub(crate) fn attachment_needs_audio_probe(att: &MessageAttachment) -> bool {
    att.is_audio()
        && !att.url.is_empty()
        && !att.uploading
        && !att.presign_pending
        && !att.upload_failed
        && (att.duration <= 0 || att.size == 0)
}

pub(crate) fn urls_needing_probe(attachments: &[MessageAttachment], cx: &App) -> Vec<String> {
    let tracked = |url: &str| {
        AudioMetaCache::try_global(cx).is_some_and(|cache| cache.read(cx).contains(url))
    };
    attachments
        .iter()
        .filter(|att| attachment_needs_audio_probe(att))
        .map(|att| att.url.clone())
        .filter(|url| !tracked(url))
        .collect()
}

pub(crate) fn defer_audio_probe(urls: Vec<String>, cx: &mut App) {
    if urls.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        AudioMetaCache::ensure_urls(urls, cx);
    });
}

fn lookup(cx: &App, url: &str) -> Option<AudioMeta> {
    AudioMetaCache::try_global(cx)?
        .read(cx)
        .known
        .get(url)
        .copied()
}

async fn probe_remote(client: Arc<dyn HttpClient>, url: &str) -> anyhow::Result<AudioMeta> {
    let mut response = client.get(url, ().into(), true).await?;
    if !response.status().is_success() {
        anyhow::bail!("audio metadata status {}", response.status());
    }
    let content_length = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if let Some(size) = content_length.filter(|length| *length > AUDIO_META_MAX_BYTES as u64) {
        let mut prefix = vec![0u8; AUDIO_META_CHUNK];
        let read = response.body_mut().read(&mut prefix).await?;
        prefix.truncate(read);
        let duration = mezon_audio::audio_duration_secs_with_len(&prefix, size).unwrap_or(0.0);
        return Ok(AudioMeta { duration, size });
    }

    let mut body = Vec::new();
    let mut buffer = vec![0u8; AUDIO_META_CHUNK];
    loop {
        let read = response.body_mut().read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        if body.len() + read > AUDIO_META_MAX_BYTES {
            break;
        }
        body.extend_from_slice(&buffer[..read]);
    }
    let size = content_length.unwrap_or(body.len() as u64);
    let duration = mezon_audio::audio_duration_secs_with_len(&body, size).unwrap_or(0.0);
    Ok(AudioMeta { duration, size })
}

#[cfg(test)]
mod tests {
    use super::attachment_needs_audio_probe;
    use mezon_store::MessageAttachment;

    fn audio(url: &str, duration: i32, size: u64) -> MessageAttachment {
        MessageAttachment {
            url: url.into(),
            filetype: "audio/mpeg".into(),
            duration,
            size,
            ..Default::default()
        }
    }

    #[test]
    fn a_sound_without_duration_or_size_needs_a_probe() {
        assert!(attachment_needs_audio_probe(&audio(
            "https://cdn/leave.mp3",
            0,
            0
        )));
        assert!(attachment_needs_audio_probe(&audio(
            "https://cdn/clip.mp3",
            0,
            4096
        )));
        assert!(!attachment_needs_audio_probe(&audio(
            "https://cdn/clip.mp3",
            3,
            4096
        )));
        assert!(!attachment_needs_audio_probe(&audio("", 0, 0)));
    }
}
