use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use mezon_record::{
    AudioSource, AudioTap, PixelData, RecordStats, Recorder, RecorderConfig, VideoConfig,
    VideoFrameRef, VideoTap,
};
use parking_lot::RwLock;
use scap::Target;
use scap::capturer::{Capturer, Options, Resolution};
#[cfg(not(target_os = "macos"))]
use scap::frame::BGRAFrame;
use scap::frame::Frame;
use scap::frame::FrameType;

pub const RECORD_WIDTH: u32 = 1280;
pub const RECORD_HEIGHT: u32 = 720;
pub const RECORD_FPS: u32 = 30;

const FRAME_WAIT: Duration = Duration::from_millis(250);

#[derive(Clone, Default)]
pub struct RecordTaps {
    slot: Arc<RwLock<Option<AudioTap>>>,
}

impl RecordTaps {
    pub fn set(&self, tap: Option<AudioTap>) {
        *self.slot.write() = tap;
    }

    pub fn push(&self, source: AudioSource, samples: &[i16], rate: u32, channels: u32) {
        let Some(guard) = self.slot.try_read() else {
            return;
        };
        if let Some(tap) = guard.as_ref() {
            tap.push(source, samples, rate, channels);
        }
    }
}

#[derive(Clone)]
pub struct RecordStarter {
    taps: RecordTaps,
    slot: Arc<RwLock<Option<RecordSession>>>,
}

impl RecordStarter {
    pub fn new(taps: RecordTaps, slot: Arc<RwLock<Option<RecordSession>>>) -> Self {
        Self { taps, slot }
    }

    pub fn start(&self, path: PathBuf, window: Option<RecordWindow>) -> Result<(), String> {
        if self.slot.read().is_some() {
            return Err("a recording is already running".into());
        }
        let session = RecordSession::start(path, self.taps.clone(), window)?;
        *self.slot.write() = Some(session);
        Ok(())
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
                match std::thread::Builder::new()
                    .name("mezon-record-video".into())
                    .spawn(move || capture_pump(window, pump_recorder, pump_stop, pump_failed))
                {
                    Ok(handle) => Some(handle),
                    Err(error) => {
                        tracing::error!("could not start the call recording video pump: {error}");
                        video_unavailable.store(true, Ordering::Relaxed);
                        None
                    }
                }
            }
            None => {
                video_unavailable.store(true, Ordering::Relaxed);
                None
            }
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
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
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
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
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
    let mut canvas = Nv12Canvas::new(RECORD_WIDTH as usize, RECORD_HEIGHT as usize);
    #[cfg(not(target_os = "macos"))]
    let mut canvas = vec![0u8; RECORD_WIDTH as usize * RECORD_HEIGHT as usize * 4];

    while !stop.load(Ordering::Relaxed) {
        let frame = match capturer.get_next_frame_timeout(FRAME_WAIT) {
            Ok(Some(frame)) => frame,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!("call recording video capture stopped: {error:#}");
                break;
            }
        };

        #[cfg(target_os = "macos")]
        {
            let Frame::YUVFrame(yuv) = frame else {
                continue;
            };
            let (width, height) = (yuv.width as usize & !1, yuv.height as usize & !1);
            if width < 2 || height < 2 {
                continue;
            }
            canvas.fit(
                &yuv.luminance_bytes,
                yuv.luminance_stride as usize,
                &yuv.chrominance_bytes,
                yuv.chrominance_stride as usize,
                width,
                height,
            );
            recorder.push(VideoFrameRef {
                width: RECORD_WIDTH,
                height: RECORD_HEIGHT,
                data: PixelData::Nv12 {
                    y: &canvas.luma,
                    y_stride: RECORD_WIDTH as usize,
                    uv: &canvas.chroma,
                    uv_stride: RECORD_WIDTH as usize,
                },
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
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

#[cfg(target_os = "macos")]
struct Nv12Canvas {
    luma: Vec<u8>,
    chroma: Vec<u8>,
    width: usize,
    height: usize,
}

#[cfg(target_os = "macos")]
impl Nv12Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            luma: vec![16u8; width * height],
            chroma: vec![128u8; width * height / 2],
            width,
            height,
        }
    }

    fn fit(
        &mut self,
        luma: &[u8],
        luma_stride: usize,
        chroma: &[u8],
        chroma_stride: usize,
        src_width: usize,
        src_height: usize,
    ) {
        self.luma.fill(16);
        self.chroma.fill(128);
        if src_width == 0 || src_height == 0 || luma_stride == 0 || chroma_stride == 0 {
            return;
        }
        let scale =
            (self.width as f64 / src_width as f64).min(self.height as f64 / src_height as f64);
        let out_width = (((src_width as f64 * scale) as usize) & !1).clamp(2, self.width);
        let out_height = (((src_height as f64 * scale) as usize) & !1).clamp(2, self.height);
        let offset_x = ((self.width - out_width) / 2) & !1;
        let offset_y = ((self.height - out_height) / 2) & !1;

        for row in 0..out_height {
            let src_row = row * src_height / out_height;
            let src_start = src_row * luma_stride;
            let dst_start = (offset_y + row) * self.width + offset_x;
            for column in 0..out_width {
                let src = src_start + column * src_width / out_width;
                if src >= luma.len() {
                    break;
                }
                self.luma[dst_start + column] = luma[src];
            }
        }

        for row in 0..out_height / 2 {
            let src_row = row * (src_height / 2) / (out_height / 2);
            let src_start = src_row * chroma_stride;
            let dst_start = ((offset_y / 2) + row) * self.width + offset_x;
            for pair in 0..out_width / 2 {
                let src = src_start + (pair * (src_width / 2) / (out_width / 2)) * 2;
                if src + 1 >= chroma.len() {
                    break;
                }
                self.chroma[dst_start + pair * 2] = chroma[src];
                self.chroma[dst_start + pair * 2 + 1] = chroma[src + 1];
            }
        }
    }
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
