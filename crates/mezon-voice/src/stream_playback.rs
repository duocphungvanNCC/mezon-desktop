use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

use crate::audio::AudioFormat;

struct MixerState {
    format: AudioFormat,
    output: u64,
    tracks: HashMap<u64, VecDeque<i16>>,
}

struct StreamPlaybackMixer {
    state: Mutex<MixerState>,
    volume: AtomicU32,
    muted: AtomicBool,
}

impl StreamPlaybackMixer {
    const MAX_BUFFERED: usize = 48_000;

    fn new(format: AudioFormat, volume: f32, muted: bool) -> Self {
        Self {
            state: Mutex::new(MixerState {
                format,
                output: 0,
                tracks: HashMap::new(),
            }),
            volume: AtomicU32::new((volume.clamp(0.0, 1.0) * 1000.0).round() as u32),
            muted: AtomicBool::new(muted),
        }
    }

    fn format(&self) -> AudioFormat {
        self.state.lock().format
    }

    fn output(&self) -> u64 {
        self.state.lock().output
    }

    fn activate(&self, output: u64, format: AudioFormat) {
        let mut state = self.state.lock();
        state.output = output;
        if state.format != format {
            state.format = format;
            state.tracks.clear();
        }
    }

    fn set_volume(&self, volume: f32) {
        self.volume.store(
            (volume.clamp(0.0, 1.0) * 1000.0).round() as u32,
            Ordering::Relaxed,
        );
    }

    fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    fn push(&self, key: u64, format: AudioFormat, samples: &[i16]) -> bool {
        let mut state = self.state.lock();
        if state.format != format {
            return false;
        }
        let buf = state.tracks.entry(key).or_default();
        buf.extend(samples.iter().copied());
        while buf.len() > Self::MAX_BUFFERED {
            buf.pop_front();
        }
        true
    }

    fn clear(&self, key: u64) {
        self.state.lock().tracks.remove(&key);
    }

    fn clear_all(&self) {
        self.state.lock().tracks.clear();
    }

    fn mix_into(&self, output: u64, out: &mut [i16]) {
        let gain = if self.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            self.volume.load(Ordering::Relaxed) as f32 / 1000.0
        };
        let mut state = self.state.lock();
        if state.output != output {
            out.fill(0);
            return;
        }
        let tracks = &mut state.tracks;
        for slot in out.iter_mut() {
            let mixed = tracks
                .values_mut()
                .map(|buf| buf.pop_front().unwrap_or(0) as i32)
                .fold(0i32, |mixed, sample| {
                    mixed
                        .saturating_add(sample)
                        .clamp(i16::MIN as i32, i16::MAX as i32)
                }) as i16;
            *slot = if gain <= 0.0 {
                0
            } else {
                (mixed as f32 * gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16
            };
        }
        tracks.retain(|_, samples| !samples.is_empty());
    }
}

struct ActiveOutput {
    device_id: Option<String>,
    stream: cpal::Stream,
    failed: Arc<AtomicBool>,
}

pub struct StreamAudioOutput {
    mixer: Arc<StreamPlaybackMixer>,
    active: Mutex<ActiveOutput>,
}

impl StreamAudioOutput {
    pub fn start(output_device_id: Option<String>, volume: f32, muted: bool) -> Result<Self> {
        match Self::open(output_device_id.clone(), volume, muted) {
            Err(error) if output_device_id.is_some() => {
                tracing::warn!(%error, "stream audio output device unavailable, using the system default");
                Self::open(None, volume, muted)
            }
            opened => opened,
        }
    }

    fn open(output_device_id: Option<String>, volume: f32, muted: bool) -> Result<Self> {
        let (device, supported, format) = open_output(output_device_id.as_deref())?;
        let mixer = Arc::new(StreamPlaybackMixer::new(format, volume, muted));
        let failed = Arc::new(AtomicBool::new(false));
        let stream = build_output(&device, &supported, mixer.clone(), 0, failed.clone())?;
        stream.play()?;
        Ok(Self {
            mixer,
            active: Mutex::new(ActiveOutput {
                device_id: output_device_id,
                stream,
                failed,
            }),
        })
    }

    pub fn set_output_device(&self, output_device_id: Option<String>) -> Result<()> {
        let mut active = self.active.lock();
        if output_device_id.is_some()
            && active.device_id == output_device_id
            && !active.failed.load(Ordering::Relaxed)
        {
            return Ok(());
        }
        let (device, supported, format) = open_output(output_device_id.as_deref())?;
        let output = self.mixer.output().wrapping_add(1);
        let failed = Arc::new(AtomicBool::new(false));
        let stream = build_output(
            &device,
            &supported,
            self.mixer.clone(),
            output,
            failed.clone(),
        )?;
        stream.play()?;
        self.mixer.activate(output, format);
        let previous = std::mem::replace(
            &mut *active,
            ActiveOutput {
                device_id: output_device_id,
                stream,
                failed,
            },
        );
        drop(active);
        drop_stream_detached(previous.stream);
        Ok(())
    }

    pub fn device_id(&self) -> Option<String> {
        self.active.lock().device_id.clone()
    }

    pub fn format(&self) -> AudioFormat {
        self.mixer.format()
    }

    pub fn set_volume(&self, volume: f32) {
        self.mixer.set_volume(volume);
    }

    pub fn set_muted(&self, muted: bool) {
        self.mixer.set_muted(muted);
    }

    pub fn push_track(&self, key: u64, format: AudioFormat, samples: &[i16]) -> bool {
        self.mixer.push(key, format, samples)
    }

    pub fn clear(&self) {
        self.mixer.clear_all();
    }

    pub fn clear_track(&self, key: u64) {
        self.mixer.clear(key);
    }
}

fn drop_stream_detached(stream: cpal::Stream) {
    let _ = std::thread::Builder::new()
        .name("mezon-stream-output-drop".into())
        .spawn(move || drop(stream));
}

fn open_output(
    output_device_id: Option<&str>,
) -> Result<(cpal::Device, cpal::SupportedStreamConfig, AudioFormat)> {
    let host = cpal::default_host();
    let device = select_output(&host, output_device_id)?;
    let supported = device.default_output_config()?;
    let format = AudioFormat {
        sample_rate: supported.sample_rate(),
        channels: supported.channels() as u32,
    };
    Ok((device, supported, format))
}

fn select_output(host: &cpal::Host, id: Option<&str>) -> Result<cpal::Device> {
    match id {
        Some(id) => host
            .output_devices()?
            .find(|d| matches!(d.id(), Ok(did) if did.to_string() == id))
            .ok_or_else(|| anyhow!("audio output device {id} is not available")),
        None => host
            .default_output_device()
            .ok_or_else(|| anyhow!("no audio output device available")),
    }
}

fn low_latency_buffer(supported: &cpal::SupportedStreamConfig) -> cpal::BufferSize {
    match supported.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            cpal::BufferSize::Fixed((supported.sample_rate() / 100).clamp(*min, *max))
        }
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
    }
}

fn i16_to_f32(s: i16) -> f32 {
    if s >= 0 {
        s as f32 / i16::MAX as f32
    } else {
        s as f32 / -(i16::MIN as f32)
    }
}

fn build_output(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    mixer: Arc<StreamPlaybackMixer>,
    output: u64,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    let mut config: cpal::StreamConfig = supported.config();
    config.buffer_size = low_latency_buffer(supported);
    let on_error = move |err: cpal::StreamError| {
        failed.store(true, Ordering::Relaxed);
        tracing::warn!("stream audio output error: {err}");
    };
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            let mut tmp: Vec<i16> = Vec::new();
            device.build_output_stream(
                &config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    tmp.clear();
                    tmp.resize(out.len(), 0);
                    mixer.mix_into(output, &mut tmp);
                    for (o, s) in out.iter_mut().zip(tmp.iter().copied()) {
                        *o = i16_to_f32(s);
                    }
                },
                on_error,
                None,
            )?
        }
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            move |out: &mut [i16], _: &cpal::OutputCallbackInfo| {
                mixer.mix_into(output, out);
            },
            on_error,
            None,
        )?,
        cpal::SampleFormat::U16 => {
            let mut tmp: Vec<i16> = Vec::new();
            device.build_output_stream(
                &config,
                move |out: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    tmp.clear();
                    tmp.resize(out.len(), 0);
                    mixer.mix_into(output, &mut tmp);
                    for (o, s) in out.iter_mut().zip(tmp.iter().copied()) {
                        *o = (s as i32 + 32768) as u16;
                    }
                },
                on_error,
                None,
            )?
        }
        other => return Err(anyhow!("unsupported output sample format: {other:?}")),
    };
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEREO_48K: AudioFormat = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    const MONO_44K: AudioFormat = AudioFormat {
        sample_rate: 44_100,
        channels: 1,
    };

    #[test]
    fn mixes_and_removes_tracks_independently() {
        let mixer = StreamPlaybackMixer::new(STEREO_48K, 1.0, false);
        mixer.push(1, STEREO_48K, &[1_000, 1_000]);
        mixer.push(2, STEREO_48K, &[2_000, 2_000]);

        let mut output = [0; 2];
        mixer.mix_into(0, &mut output);
        assert_eq!(output, [3_000, 3_000]);

        mixer.push(1, STEREO_48K, &[4_000, 4_000]);
        mixer.clear(1);
        let mut output = [0; 2];
        mixer.mix_into(0, &mut output);
        assert_eq!(output, [0, 0]);
    }

    #[test]
    fn drops_samples_decoded_for_a_previous_output_format() {
        let mixer = StreamPlaybackMixer::new(STEREO_48K, 1.0, false);
        mixer.push(1, STEREO_48K, &[1_000, 1_000]);

        mixer.activate(1, MONO_44K);
        assert!(!mixer.push(1, STEREO_48K, &[2_000, 2_000]));
        let mut output = [0; 2];
        mixer.mix_into(1, &mut output);
        assert_eq!(output, [0, 0]);

        assert!(mixer.push(1, MONO_44K, &[3_000]));
        let mut output = [0; 1];
        mixer.mix_into(1, &mut output);
        assert_eq!(output, [3_000]);
    }

    #[test]
    fn keeps_buffered_samples_when_the_output_format_is_unchanged() {
        let mixer = StreamPlaybackMixer::new(STEREO_48K, 1.0, false);
        mixer.push(1, STEREO_48K, &[1_000, 1_000]);

        mixer.activate(1, STEREO_48K);
        let mut output = [0; 2];
        mixer.mix_into(1, &mut output);
        assert_eq!(output, [1_000, 1_000]);
    }

    #[test]
    fn only_the_active_output_plays_and_consumes_samples() {
        let mixer = StreamPlaybackMixer::new(STEREO_48K, 1.0, false);
        mixer.push(1, STEREO_48K, &[1_000, 1_000, 2_000, 2_000]);

        let mut starting = [7; 2];
        mixer.mix_into(1, &mut starting);
        assert_eq!(starting, [0, 0]);

        mixer.activate(1, STEREO_48K);
        let mut replaced = [7; 2];
        mixer.mix_into(0, &mut replaced);
        assert_eq!(replaced, [0, 0]);

        let mut active = [0; 4];
        mixer.mix_into(1, &mut active);
        assert_eq!(active, [1_000, 1_000, 2_000, 2_000]);
    }
}
