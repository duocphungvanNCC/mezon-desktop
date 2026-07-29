use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

use crate::audio::AudioFormat;

struct StreamPlaybackMixer {
    samples: Mutex<VecDeque<i16>>,
    volume: AtomicU32,
    muted: AtomicBool,
}

impl StreamPlaybackMixer {
    const MAX_BUFFERED: usize = 48_000;

    fn new(volume: f32, muted: bool) -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
            volume: AtomicU32::new((volume.clamp(0.0, 1.0) * 1000.0).round() as u32),
            muted: AtomicBool::new(muted),
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

    fn push(&self, samples: &[i16]) {
        let mut buf = self.samples.lock();
        buf.extend(samples.iter().copied());
        while buf.len() > Self::MAX_BUFFERED {
            buf.pop_front();
        }
    }

    fn clear(&self) {
        self.samples.lock().clear();
    }

    fn mix_into(&self, out: &mut [i16]) {
        let gain = if self.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            self.volume.load(Ordering::Relaxed) as f32 / 1000.0
        };
        let mut buf = self.samples.lock();
        for slot in out.iter_mut() {
            let sample = buf.pop_front().unwrap_or(0);
            *slot = if gain <= 0.0 {
                0
            } else {
                (sample as f32 * gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16
            };
        }
    }
}

pub struct StreamAudioOutput {
    mixer: Arc<StreamPlaybackMixer>,
    _stream: cpal::Stream,
    format: AudioFormat,
}

impl StreamAudioOutput {
    pub fn start(output_device_id: Option<String>, volume: f32, muted: bool) -> Result<Self> {
        let mixer = Arc::new(StreamPlaybackMixer::new(volume, muted));
        let host = cpal::default_host();
        let device = select_output(&host, output_device_id.as_deref())?;
        let supported = device.default_output_config()?;
        let format = AudioFormat {
            sample_rate: supported.sample_rate(),
            channels: supported.channels() as u32,
        };
        let mixer_for_stream = mixer.clone();
        let stream = build_output(&device, &supported, mixer_for_stream)?;
        stream.play()?;
        Ok(Self {
            mixer,
            _stream: stream,
            format,
        })
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }

    pub fn set_volume(&self, volume: f32) {
        self.mixer.set_volume(volume);
    }

    pub fn set_muted(&self, muted: bool) {
        self.mixer.set_muted(muted);
    }

    pub fn push(&self, samples: &[i16]) {
        self.mixer.push(samples);
    }

    pub fn clear(&self) {
        self.mixer.clear();
    }
}

fn select_output(host: &cpal::Host, id: Option<&str>) -> Result<cpal::Device> {
    if let Some(id) = id
        && let Ok(mut devices) = host.output_devices()
        && let Some(device) = devices.find(|d| matches!(d.id(), Ok(did) if did.to_string() == id))
    {
        return Ok(device);
    }
    host.default_output_device()
        .ok_or_else(|| anyhow!("no audio output device available"))
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
) -> Result<cpal::Stream> {
    let mut config: cpal::StreamConfig = supported.config();
    config.buffer_size = low_latency_buffer(supported);
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            let mut tmp: Vec<i16> = Vec::new();
            device.build_output_stream(
                &config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    tmp.clear();
                    tmp.resize(out.len(), 0);
                    mixer.mix_into(&mut tmp);
                    for (o, s) in out.iter_mut().zip(tmp.iter().copied()) {
                        *o = i16_to_f32(s);
                    }
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            move |out: &mut [i16], _: &cpal::OutputCallbackInfo| {
                mixer.mix_into(out);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => {
            let mut tmp: Vec<i16> = Vec::new();
            device.build_output_stream(
                &config,
                move |out: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    tmp.clear();
                    tmp.resize(out.len(), 0);
                    mixer.mix_into(&mut tmp);
                    for (o, s) in out.iter_mut().zip(tmp.iter().copied()) {
                        *o = (s as i32 + 32768) as u16;
                    }
                },
                err_fn,
                None,
            )?
        }
        other => return Err(anyhow!("unsupported output sample format: {other:?}")),
    };
    Ok(stream)
}

fn err_fn(err: cpal::StreamError) {
    tracing::warn!("stream audio output error: {err}");
}
