use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::disk;
use crate::mixer::{AudioMixer, AudioSource};
use crate::platform;
use crate::sink::{
    RECORD_CHANNELS, RECORD_SAMPLE_RATE, RecordError, RecordSink, VideoConfig, VideoFrameRef,
};

const BLOCK_FRAMES: usize = RECORD_SAMPLE_RATE as usize / 100;
const AUDIO_QUEUE_DEPTH: usize = 256;
const REMOTE_STALL: Duration = Duration::from_millis(120);
const VIDEO_STALL: Duration = Duration::from_millis(750);
const WORKER_IDLE_POLL: Duration = Duration::from_millis(5);
const MIN_FREE_BYTES: u64 = 512 * 1024 * 1024;

pub struct RecorderConfig {
    pub path: PathBuf,
    pub video: Option<VideoConfig>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RecordStats {
    pub elapsed: Duration,
    pub dropped_audio_chunks: u64,
    pub video_frames: u64,
    pub video_stalled: bool,
}

struct AudioChunk {
    source: AudioSource,
    samples: Vec<i16>,
    rate: u32,
    channels: u32,
}

struct Shared {
    sink: Mutex<Option<Box<dyn RecordSink>>>,
    audio_frames: AtomicU64,
    video_frames: AtomicU64,
    dropped: AtomicU64,
    last_video_ns: AtomicU64,
    failed: AtomicBool,
    stop: AtomicBool,
}

impl Shared {
    fn audio_pts(&self) -> Duration {
        let frames = self.audio_frames.load(Ordering::Relaxed);
        Duration::from_nanos(frames * 1_000_000_000 / RECORD_SAMPLE_RATE as u64)
    }
}

#[derive(Clone)]
pub struct AudioTap {
    tx: flume::Sender<AudioChunk>,
    shared: Arc<Shared>,
}

impl AudioTap {
    pub fn push(&self, source: AudioSource, samples: &[i16], rate: u32, channels: u32) {
        if samples.is_empty() || self.shared.stop.load(Ordering::Relaxed) {
            return;
        }
        let chunk = AudioChunk {
            source,
            samples: samples.to_vec(),
            rate,
            channels,
        };
        if self.tx.try_send(chunk).is_err() {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[derive(Clone)]
pub struct VideoTap {
    shared: Arc<Shared>,
    video: Option<VideoConfig>,
    min_video_gap: Duration,
}

impl VideoTap {
    pub fn push(&self, frame: VideoFrameRef<'_>) {
        if self.video.is_none()
            || self.shared.stop.load(Ordering::Relaxed)
            || self.shared.failed.load(Ordering::Relaxed)
        {
            return;
        }
        let pts = self.shared.audio_pts();
        let last = Duration::from_nanos(self.shared.last_video_ns.load(Ordering::Relaxed));
        let started = self.shared.video_frames.load(Ordering::Relaxed) > 0;
        if started && pts.saturating_sub(last) < self.min_video_gap {
            return;
        }

        let mut guard = self.shared.sink.lock();
        let Some(sink) = guard.as_mut() else {
            return;
        };
        match sink.push_video(frame, pts) {
            Ok(()) => {
                self.shared
                    .last_video_ns
                    .store(pts.as_nanos() as u64, Ordering::Relaxed);
                self.shared.video_frames.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => {
                tracing::error!("call recorder rejected a video frame: {error}");
                self.shared.failed.store(true, Ordering::Relaxed);
            }
        }
    }
}

pub struct Recorder {
    path: PathBuf,
    shared: Arc<Shared>,
    tx: flume::Sender<AudioChunk>,
    worker: Option<JoinHandle<()>>,
    video: Option<VideoConfig>,
    started: Instant,
    min_video_gap: Duration,
}

impl Recorder {
    pub fn start(config: RecorderConfig) -> Result<Self, RecordError> {
        if let Some(parent) = config.path.parent()
            && disk::free_bytes(parent).is_some_and(|free| free < MIN_FREE_BYTES)
        {
            return Err(RecordError::DiskSpace);
        }

        let part = part_path(&config.path);
        let sink = platform::create_sink(&part, config.video)?;
        let shared = Arc::new(Shared {
            sink: Mutex::new(Some(sink)),
            audio_frames: AtomicU64::new(0),
            video_frames: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            last_video_ns: AtomicU64::new(0),
            failed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });

        let (tx, rx) = flume::bounded::<AudioChunk>(AUDIO_QUEUE_DEPTH);
        let worker_shared = shared.clone();
        let worker = std::thread::Builder::new()
            .name("mezon-record-audio".into())
            .spawn(move || audio_worker(worker_shared, rx))
            .map_err(|_| RecordError::Create)?;

        let fps = config.video.map(|video| video.fps.max(1)).unwrap_or(30);
        Ok(Self {
            path: config.path,
            shared,
            tx,
            worker: Some(worker),
            video: config.video,
            started: Instant::now(),
            min_video_gap: Duration::from_secs_f64(0.9 / fps as f64),
        })
    }

    pub fn audio_tap(&self) -> AudioTap {
        AudioTap {
            tx: self.tx.clone(),
            shared: self.shared.clone(),
        }
    }

    pub fn video_config(&self) -> Option<VideoConfig> {
        self.video
    }

    pub fn video_tap(&self) -> VideoTap {
        VideoTap {
            shared: self.shared.clone(),
            video: self.video,
            min_video_gap: self.min_video_gap,
        }
    }

    pub fn push_video(&self, frame: VideoFrameRef<'_>) {
        self.video_tap().push(frame);
    }

    pub fn stats(&self) -> RecordStats {
        let pts = self.shared.audio_pts();
        let last = Duration::from_nanos(self.shared.last_video_ns.load(Ordering::Relaxed));
        RecordStats {
            elapsed: pts,
            dropped_audio_chunks: self.shared.dropped.load(Ordering::Relaxed),
            video_frames: self.shared.video_frames.load(Ordering::Relaxed),
            video_stalled: self.video.is_some() && pts.saturating_sub(last) > VIDEO_STALL,
        }
    }

    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Relaxed)
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn finish(mut self) -> Result<PathBuf, RecordError> {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let sink = self.shared.sink.lock().take();
        let Some(sink) = sink else {
            return Err(RecordError::Finish);
        };
        sink.finish()?;
        let part = part_path(&self.path);
        std::fs::rename(&part, &self.path).map_err(|error| {
            tracing::error!("could not rename the finished recording: {error}");
            RecordError::Finish
        })?;
        Ok(self.path.clone())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(sink) = self.shared.sink.lock().take() {
            let _ = sink.finish();
        }
    }
}

fn audio_worker(shared: Arc<Shared>, rx: flume::Receiver<AudioChunk>) {
    let mut mixer = AudioMixer::new();
    let mut block = Vec::with_capacity(BLOCK_FRAMES * RECORD_CHANNELS as usize);
    let started = Instant::now();
    let mut last_remote = Instant::now();
    let mut emitted: u64 = 0;

    loop {
        let stopping = shared.stop.load(Ordering::Relaxed);
        let mut received = false;
        while let Ok(chunk) = rx.try_recv() {
            if chunk.source == AudioSource::Remote {
                last_remote = Instant::now();
            }
            mixer.push(chunk.source, &chunk.samples, chunk.rate, chunk.channels);
            received = true;
        }

        let remote_ready = emitted + mixer.remote_frames();
        let target = if last_remote.elapsed() > REMOTE_STALL {
            let wall = (started.elapsed().as_secs_f64() * RECORD_SAMPLE_RATE as f64) as u64;
            remote_ready.max(wall)
        } else {
            remote_ready
        };

        let mut wrote = false;
        while emitted + BLOCK_FRAMES as u64 <= target {
            mixer.drain_block(BLOCK_FRAMES, &mut block);
            let pts = Duration::from_nanos(emitted * 1_000_000_000 / RECORD_SAMPLE_RATE as u64);
            let mut guard = shared.sink.lock();
            let Some(sink) = guard.as_mut() else {
                return;
            };
            if let Err(error) = sink.push_audio(&block, pts) {
                tracing::error!("call recorder rejected an audio block: {error}");
                shared.failed.store(true, Ordering::Relaxed);
                return;
            }
            drop(guard);
            emitted += BLOCK_FRAMES as u64;
            shared.audio_frames.store(emitted, Ordering::Relaxed);
            wrote = true;
        }

        if stopping && !received && !wrote {
            return;
        }
        if !received && !wrote {
            std::thread::sleep(WORKER_IDLE_POLL);
        }
    }
}

pub fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::AudioSource;

    #[test]
    fn an_audio_only_recording_lands_at_the_final_path_with_no_part_file_left() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("call.mp4");
        let recorder = match Recorder::start(RecorderConfig {
            path: path.clone(),
            video: None,
        }) {
            Ok(recorder) => recorder,
            Err(RecordError::Unsupported) => return,
            Err(error) => panic!("could not start the recorder: {error}"),
        };

        let tap = recorder.audio_tap();
        let block = vec![0i16; BLOCK_FRAMES * RECORD_CHANNELS as usize];
        for _ in 0..50 {
            tap.push(
                AudioSource::Remote,
                &block,
                RECORD_SAMPLE_RATE,
                RECORD_CHANNELS,
            );
        }

        let finished = recorder.finish().expect("finish");
        assert_eq!(finished, path);
        assert!(path.exists(), "the recording should be at the final path");
        assert!(
            std::fs::metadata(&path).expect("metadata").len() > 0,
            "the recording should not be empty"
        );
        assert!(
            !part_path(&path).exists(),
            "the .part file should be renamed away on a clean finish"
        );
    }

    #[test]
    #[ignore]
    fn a_long_video_recording_survives_past_the_first_fragments() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("long.mp4");
        let config = VideoConfig {
            width: 1280,
            height: 720,
            fps: 30,
        };
        let recorder = match Recorder::start(RecorderConfig {
            path: path.clone(),
            video: Some(config),
        }) {
            Ok(recorder) => recorder,
            Err(RecordError::Unsupported) => return,
            Err(error) => panic!("start: {error}"),
        };
        let tap = recorder.audio_tap();
        let audio = vec![0i16; BLOCK_FRAMES * RECORD_CHANNELS as usize];
        #[cfg(target_os = "macos")]
        let mut luma = vec![128u8; 1280 * 720];
        #[cfg(target_os = "macos")]
        let mut chroma = vec![128u8; 1280 * 360];
        #[cfg(not(target_os = "macos"))]
        let mut bgra = vec![128u8; 1280 * 720 * 4];

        for tick in 0..2_000 {
            let shade = (tick % 251) as u8;
            #[cfg(target_os = "macos")]
            {
                for (index, pixel) in luma.iter_mut().enumerate() {
                    *pixel = shade.wrapping_add((index % 253) as u8);
                }
                for (index, pixel) in chroma.iter_mut().enumerate() {
                    *pixel = shade.wrapping_mul(3).wrapping_add((index % 247) as u8);
                }
            }
            #[cfg(not(target_os = "macos"))]
            for (index, pixel) in bgra.iter_mut().enumerate() {
                *pixel = shade.wrapping_add((index % 251) as u8);
            }
            tap.push(
                AudioSource::Remote,
                &audio,
                RECORD_SAMPLE_RATE,
                RECORD_CHANNELS,
            );
            recorder.push_video(VideoFrameRef {
                width: config.width,
                height: config.height,
                #[cfg(target_os = "macos")]
                data: crate::sink::PixelData::Nv12 {
                    y: &luma,
                    y_stride: 1280,
                    uv: &chroma,
                    uv_stride: 1280,
                },
                #[cfg(not(target_os = "macos"))]
                data: crate::sink::PixelData::Bgra {
                    data: &bgra,
                    stride: 1280 * 4,
                },
            });
            std::thread::sleep(Duration::from_millis(10));
            if recorder.failed() {
                panic!(
                    "the recorder failed after {:.2}s ({} video frames)",
                    recorder.stats().elapsed.as_secs_f64(),
                    recorder.stats().video_frames
                );
            }
            let _ = tick;
        }
        let stats = recorder.stats();
        println!(
            "survived {:.2}s with {} video frames",
            stats.elapsed.as_secs_f64(),
            stats.video_frames
        );
        recorder.finish().expect("finish");
    }

    #[test]
    fn a_dropped_recorder_leaves_the_part_file_and_never_claims_the_final_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("call.mp4");
        let recorder = match Recorder::start(RecorderConfig {
            path: path.clone(),
            video: None,
        }) {
            Ok(recorder) => recorder,
            Err(RecordError::Unsupported) => return,
            Err(error) => panic!("could not start the recorder: {error}"),
        };
        let tap = recorder.audio_tap();
        tap.push(
            AudioSource::Remote,
            &vec![0i16; BLOCK_FRAMES * RECORD_CHANNELS as usize],
            RECORD_SAMPLE_RATE,
            RECORD_CHANNELS,
        );
        drop(recorder);

        assert!(
            !path.exists(),
            "an aborted recording must not claim the final path"
        );
        assert!(
            part_path(&path).exists(),
            "the partial file should survive for recovery"
        );
    }
}
