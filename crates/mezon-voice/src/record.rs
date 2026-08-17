use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::time::Duration;

use mezon_record::{
    AudioSource, AudioTap, PixelData, RecordStats, Recorder, RecorderConfig, VideoConfig,
    VideoFrameRef, VideoTap,
};
use parking_lot::Mutex;
use scap::Target;
use scap::capturer::{Capturer, Options, Resolution};
use scap::frame::FrameType;
#[cfg(not(target_os = "macos"))]
use scap::frame::{BGRAFrame, Frame};

pub const RECORD_WIDTH: u32 = 1280;
pub const RECORD_HEIGHT: u32 = 720;
pub const RECORD_FPS: u32 = 30;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const FRAME_WAIT: Duration = Duration::from_millis(250);

#[derive(Clone, Default)]
pub struct RecordTaps {
    slot: Arc<Mutex<Option<AudioTap>>>,
}

impl RecordTaps {
    pub fn set(&self, tap: Option<AudioTap>) {
        *self.slot.lock() = tap;
    }

    pub fn push(&self, source: AudioSource, samples: &[i16], rate: u32, channels: u32) {
        let Some(guard) = self.slot.try_lock() else {
            return;
        };
        if let Some(tap) = guard.as_ref() {
            tap.push(source, samples, rate, channels);
        }
    }

    pub fn is_active(&self) -> bool {
        self.slot.try_lock().is_some_and(|slot| slot.is_some())
    }
}

#[derive(Debug, Clone)]
pub enum RecordWindow {
    Id(u64),
    Portal,
}

pub struct RecordSession {
    recorder: Option<Recorder>,
    taps: RecordTaps,
    stop: Arc<AtomicBool>,
    video_unavailable: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
}

impl RecordSession {
    pub fn start(
        path: PathBuf,
        taps: RecordTaps,
        window: Option<RecordWindow>,
    ) -> Result<Self, String> {
        let video = window.as_ref().map(|_| VideoConfig {
            width: RECORD_WIDTH,
            height: RECORD_HEIGHT,
            fps: RECORD_FPS,
        });
        let recorder =
            Recorder::start(RecorderConfig { path, video }).map_err(|error| error.to_string())?;
        taps.set(Some(recorder.audio_tap()));

        let stop = Arc::new(AtomicBool::new(false));
        let video_unavailable = Arc::new(AtomicBool::new(false));
        let pump = match window {
            Some(window) => {
                let pump_recorder = recorder.video_tap();
                let pump_stop = stop.clone();
                let pump_failed = video_unavailable.clone();
                std::thread::Builder::new()
                    .name("mezon-record-video".into())
                    .spawn(move || capture_pump(window, pump_recorder, pump_stop, pump_failed))
                    .ok()
            }
            None => None,
        };

        Ok(Self {
            recorder: Some(recorder),
            taps,
            stop,
            video_unavailable,
            pump,
        })
    }

    pub fn stats(&self) -> RecordStats {
        self.recorder
            .as_ref()
            .map(|recorder| recorder.stats())
            .unwrap_or_default()
    }

    pub fn video_unavailable(&self) -> bool {
        self.video_unavailable.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> bool {
        self.recorder
            .as_ref()
            .is_some_and(|recorder| recorder.failed())
    }

    pub fn finish(mut self) -> Result<PathBuf, String> {
        self.taps.set(None);
        self.stop.store(true, Ordering::Relaxed);
        self.pump = None;
        let Some(recorder) = self.recorder.take() else {
            return Err("the recording was already stopped".into());
        };
        recorder.finish().map_err(|error| error.to_string())
    }
}

impl Drop for RecordSession {
    fn drop(&mut self) {
        self.taps.set(None);
        self.stop.store(true, Ordering::Relaxed);
        self.pump = None;
    }
}

fn resolve_target(window: &RecordWindow) -> Result<Option<Target>, String> {
    match window {
        RecordWindow::Portal => Ok(None),
        RecordWindow::Id(id) => {
            let targets = scap::get_all_targets().map_err(|error| error.to_string())?;
            targets
                .into_iter()
                .find_map(|target| match target {
                    Target::Window(candidate) if window_id(&candidate) == *id => {
                        Some(Target::Window(candidate))
                    }
                    _ => None,
                })
                .map(Some)
                .ok_or_else(|| "the Mezon window is not available for capture".to_string())
        }
    }
}

#[cfg(target_os = "linux")]
fn window_id(window: &scap::Window) -> u64 {
    xcb::Xid::resource_id(&window.raw_handle) as u64
}

#[cfg(not(target_os = "linux"))]
fn window_id(window: &scap::Window) -> u64 {
    window.id as u64
}

fn capture_pump(
    window: RecordWindow,
    recorder: VideoTap,
    stop: Arc<AtomicBool>,
    unavailable: Arc<AtomicBool>,
) {
    if !scap::is_supported() {
        tracing::warn!("call recording video is unavailable: screen capture not supported");
        unavailable.store(true, Ordering::Relaxed);
        return;
    }
    if !scap::has_permission() && !scap::request_permission() {
        tracing::warn!("call recording video is unavailable: screen recording permission denied");
        unavailable.store(true, Ordering::Relaxed);
        return;
    }

    let use_portal = matches!(window, RecordWindow::Portal);
    let target = match resolve_target(&window) {
        Ok(target) => target,
        Err(error) => {
            tracing::warn!("call recording video is unavailable: {error}");
            unavailable.store(true, Ordering::Relaxed);
            return;
        }
    };

    let options = Options {
        fps: RECORD_FPS,
        target,
        show_cursor: true,
        show_highlight: false,
        excluded_targets: None,
        #[cfg(target_os = "macos")]
        output_type: FrameType::YUVFrameFullRange,
        #[cfg(not(target_os = "macos"))]
        output_type: FrameType::BGRAFrame,
        output_resolution: Resolution::_720p,
        portal_source_types: Some(2),
        use_portal,
        ..Default::default()
    };

    let mut capturer = match Capturer::build(options) {
        Ok(capturer) => capturer,
        Err(error) => {
            tracing::warn!("call recording video capture failed to start: {error:#}");
            unavailable.store(true, Ordering::Relaxed);
            return;
        }
    };
    capturer.start_capture();

    #[cfg(target_os = "macos")]
    {
        while !stop.load(Ordering::Relaxed) {
            let Ok(captured) = capturer.raw().get_next_pixel_buffer() else {
                break;
            };
            let width = captured.width() as u32 & !1;
            let height = captured.height() as u32 & !1;
            if width < 2 || height < 2 {
                continue;
            }
            let planes = captured.planes();
            let (Some(luma), Some(chroma)) = (planes.first(), planes.get(1)) else {
                continue;
            };
            let luma_stride = luma.bytes_per_row();
            let chroma_stride = chroma.bytes_per_row();
            let luma_data = luma.data();
            let chroma_data = chroma.data();
            recorder.push(VideoFrameRef {
                width,
                height,
                data: PixelData::Nv12 {
                    y: &luma_data,
                    y_stride: luma_stride,
                    uv: &chroma_data,
                    uv_stride: chroma_stride,
                },
            });
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut canvas = vec![0u8; RECORD_WIDTH as usize * RECORD_HEIGHT as usize * 4];
        while !stop.load(Ordering::Relaxed) {
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            let next = capturer.get_next_frame_timeout(FRAME_WAIT);
            #[cfg(target_os = "windows")]
            let next = capturer.get_next_frame().map(Some);

            let frame = match next {
                Ok(Some(frame)) => frame,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!("call recording video capture stopped: {error:#}");
                    break;
                }
            };
            let Some(bgra) = frame_to_bgra(frame) else {
                continue;
            };
            if bgra.data.is_empty() || bgra.width < 2 || bgra.height < 2 {
                continue;
            }
            let stride = bgra.data.len() / bgra.height.max(1) as usize;
            fit_bgra(
                &bgra.data,
                bgra.width as usize,
                bgra.height as usize,
                stride,
                &mut canvas,
                RECORD_WIDTH as usize,
                RECORD_HEIGHT as usize,
            );
            recorder.push(VideoFrameRef {
                width: RECORD_WIDTH,
                height: RECORD_HEIGHT,
                data: PixelData::Bgra {
                    data: &canvas,
                    stride: RECORD_WIDTH as usize * 4,
                },
            });
        }
    }

    capturer.stop_capture();
}

#[cfg(not(target_os = "macos"))]
fn frame_to_bgra(frame: Frame) -> Option<BGRAFrame> {
    match frame {
        Frame::BGRA(frame) => Some(frame),
        Frame::BGRx(frame) => Some(BGRAFrame {
            display_time: frame.display_time,
            width: frame.width,
            height: frame.height,
            data: frame.data,
        }),
        Frame::BGR0(frame) => Some(BGRAFrame {
            display_time: frame.display_time,
            width: frame.width,
            height: frame.height,
            data: frame.data,
        }),
        _ => None,
    }
}

#[cfg(not(target_os = "macos"))]
fn fit_bgra(
    src: &[u8],
    src_width: usize,
    src_height: usize,
    src_stride: usize,
    dst: &mut [u8],
    dst_width: usize,
    dst_height: usize,
) {
    dst.fill(0);
    if src_width == 0 || src_height == 0 || src_stride == 0 {
        return;
    }
    let scale = (dst_width as f64 / src_width as f64).min(dst_height as f64 / src_height as f64);
    let out_width = ((src_width as f64 * scale) as usize).clamp(1, dst_width);
    let out_height = ((src_height as f64 * scale) as usize).clamp(1, dst_height);
    let offset_x = (dst_width - out_width) / 2;
    let offset_y = (dst_height - out_height) / 2;

    for row in 0..out_height {
        let src_row = row * src_height / out_height;
        let src_start = src_row * src_stride;
        let dst_start = ((offset_y + row) * dst_width + offset_x) * 4;
        for column in 0..out_width {
            let src_column = column * src_width / out_width;
            let source = src_start + src_column * 4;
            let target = dst_start + column * 4;
            if source + 4 > src.len() || target + 4 > dst.len() {
                break;
            }
            dst[target..target + 4].copy_from_slice(&src[source..source + 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "macos"))]
    use super::fit_bgra;

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn fit_letterboxes_a_narrow_source_and_leaves_bars_black() {
        let src = vec![255u8; 4 * 4 * 4];
        let mut dst = vec![9u8; 8 * 4 * 4];
        fit_bgra(&src, 4, 4, 16, &mut dst, 8, 4);

        assert_eq!(&dst[0..4], &[0, 0, 0, 0]);
        let center = 2 * 4;
        assert_eq!(&dst[center..center + 4], &[255, 255, 255, 255]);
    }
}
