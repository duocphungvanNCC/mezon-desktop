use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use libwebrtc::native::apm::AudioProcessingModule;
use mezon_ns::{Mezon48k, MezonNSConfig, MezonNSEngine};
use parking_lot::{Condvar, Mutex};
use sha2::{Digest, Sha256};

use crate::VoiceEvent;
use crate::runtime;

const WEB_SUPPRESSION_INTENSITY: f32 = 1.6;
const MODEL_INPUT_TARGET_DBFS: f32 = -20.0;
const MAX_ATTENUATION_DB: f32 = 15.0;
const FILTERED_SAMPLE_RATE: i32 = (mezon_ns::FRAME_SIZE_48K * 100) as i32;
const MODEL_FILE: &str = "mezon_ns_asym_babble.onnx";
const MODEL_META_FILE: &str = "mezon_ns_asym_babble.json";
const MODEL_URL: &str = "https://cdn.komu.vn/ns/mezon_ns_asym_babble.onnx";
const MODEL_WAIT: Duration = Duration::from_secs(15);

struct ModelSlot {
    model: Option<Arc<[u8]>>,
    loading: bool,
    error: Option<String>,
}

struct ModelFile {
    model: Vec<u8>,
    etag: Option<String>,
}

static MODEL: Mutex<ModelSlot> = Mutex::new(ModelSlot {
    model: None,
    loading: false,
    error: None,
});
static MODEL_CHANGED: Condvar = Condvar::new();

fn prefetch_model() {
    let mut slot = MODEL.lock();
    if !slot.loading {
        start_model_load(&mut slot);
    }
}

fn model_bytes(wait: Duration) -> Result<Arc<[u8]>, String> {
    let deadline = Instant::now() + wait;
    let mut slot = MODEL.lock();
    if slot.model.is_none() && !slot.loading {
        start_model_load(&mut slot);
    }
    loop {
        if let Some(model) = &slot.model {
            return Ok(model.clone());
        }
        if !slot.loading {
            return Err(slot.error.clone().unwrap_or_else(|| "Mezon-NS model unavailable".into()));
        }
        if Instant::now() >= deadline {
            return Err("Mezon-NS model is still downloading".into());
        }
        MODEL_CHANGED.wait_until(&mut slot, deadline);
    }
}

fn start_model_load(slot: &mut ModelSlot) {
    slot.loading = true;
    let spawned = std::thread::Builder::new()
        .name("mezon-ns-model".into())
        .spawn(|| {
            let loaded = load_model();
            let mut slot = MODEL.lock();
            slot.loading = false;
            match loaded {
                Ok(model) => {
                    slot.model = Some(model.into());
                    slot.error = None;
                }
                Err(error) => {
                    tracing::warn!("Mezon-NS model unavailable: {error}");
                    slot.error = Some(error);
                }
            }
            MODEL_CHANGED.notify_all();
        });
    if let Err(error) = spawned {
        slot.loading = false;
        slot.error = Some(error.to_string());
    }
}

fn load_model() -> Result<Vec<u8>, String> {
    let dir = dirs::cache_dir().map(|base| base.join("mezon").join("ns"));
    let cached = dir.as_deref().and_then(read_cached_model);
    let etag = cached.as_ref().and_then(|cached| cached.etag.clone());
    let fresh = runtime::runtime()
        .block_on(download_model(etag))
        .and_then(|downloaded| match downloaded {
            Some(file) => create_filter(&file.model).map(|_| Some(file)),
            None => Ok(None),
        });
    match (fresh, cached) {
        (Ok(Some(file)), _) => {
            tracing::info!(bytes = file.model.len(), "Mezon-NS model downloaded");
            if let Some(dir) = &dir
                && let Err(error) = save_model(dir, &file)
            {
                tracing::warn!("Mezon-NS model cache write failed: {error}");
            }
            Ok(file.model)
        }
        (Ok(None), Some(cached)) => {
            tracing::info!("Mezon-NS model is up to date");
            Ok(cached.model)
        }
        (Err(error), Some(cached)) => {
            tracing::warn!("Mezon-NS model refresh failed, using the cached copy: {error}");
            Ok(cached.model)
        }
        (Ok(None), None) => Err("Mezon-NS model cache is missing".into()),
        (Err(error), None) => Err(error),
    }
}

fn read_cached_model(dir: &Path) -> Option<ModelFile> {
    let meta = std::fs::read(dir.join(MODEL_META_FILE)).ok()?;
    let meta: serde_json::Value = serde_json::from_slice(&meta).ok()?;
    let model = std::fs::read(dir.join(MODEL_FILE)).ok()?;
    if meta.get("sha256")?.as_str()? != sha256_hex(&model) {
        return None;
    }
    let etag = meta.get("etag").and_then(serde_json::Value::as_str).map(str::to_owned);
    Some(ModelFile { model, etag })
}

async fn download_model(etag: Option<String>) -> Result<Option<ModelFile>, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(MODEL_URL);
    if let Some(etag) = &etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let model = response.bytes().await.map_err(|error| error.to_string())?;
    Ok(Some(ModelFile {
        model: model.to_vec(),
        etag,
    }))
}

fn save_model(dir: &Path, file: &ModelFile) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    write_atomically(&dir.join(MODEL_FILE), &file.model)?;
    let meta = serde_json::json!({ "etag": file.etag, "sha256": sha256_hex(&file.model) });
    write_atomically(&dir.join(MODEL_META_FILE), meta.to_string().as_bytes())
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let partial = path.with_extension("part");
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, path)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn create_filter(model: &[u8]) -> Result<Mezon48k, String> {
    let mut config = MezonNSConfig::default();
    config.suppression_intensity = WEB_SUPPRESSION_INTENSITY;
    config.enable_noise_gate = 0;
    config.attenuation_limit_db = MAX_ATTENUATION_DB;
    MezonNSEngine::create_from_memory(model, Some(config))
        .and_then(|mut engine| {
            let input = [0i16; mezon_ns::FRAME_SIZE];
            let mut output = [0i16; mezon_ns::FRAME_SIZE];
            engine.process_frame_int16(&input, &mut output)?;
            engine.reset();
            engine.set_model_input_target_dbfs(MODEL_INPUT_TARGET_DBFS);
            Ok(Mezon48k::new(engine))
        })
        .map_err(|e| e.to_string())
}

enum Setting {
    Enabled { enabled: bool, generation: u64 },
    Reset,
}

pub(super) struct FilteredFrame {
    pub samples: Vec<i16>,
    pub generation: u64,
}

pub(super) struct NoiseProcessor {
    requested: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    ready: Arc<AtomicBool>,
    settings_tx: flume::Sender<Setting>,
    frames_tx: flume::Sender<Vec<i16>>,
    pub output_rx: flume::Receiver<FilteredFrame>,
}

impl NoiseProcessor {
    pub fn start(events: flume::Sender<VoiceEvent>) -> Self {
        prefetch_model();
        let requested = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let ready = Arc::new(AtomicBool::new(false));
        let (settings_tx, settings_rx) = flume::unbounded();
        // A short cushion for OS scheduling bursts without adding unbounded voice latency.
        let (frames_tx, frames_rx) = flume::bounded(8);
        let (output_tx, output_rx) = flume::bounded(8);
        let stale_output_rx = output_rx.clone();
        let current_requested = requested.clone();
        let current_generation = generation.clone();
        let current_ready = ready.clone();
        std::thread::Builder::new()
            .name("mezon-ns".into())
            .spawn(move || {
                enum Event {
                    Setting(Setting),
                    Frame(Vec<i16>),
                    Stop,
                }
                let mut filter: Option<Mezon48k> = None;
                let mut enabled = false;
                let mut active_generation = 0;
                let mut output_overruns = 0u64;
                let mut output_window_start = Instant::now();
                let mut output_window_drops = 0u32;
                let mut agc = AudioProcessingModule::new(false, true, false, false);
                loop {
                    let event = flume::Selector::new()
                        .recv(&settings_rx, |r| {
                            r.map(Event::Setting).unwrap_or(Event::Stop)
                        })
                        .recv(&frames_rx, |r| r.map(Event::Frame).unwrap_or(Event::Stop))
                        .wait();
                    match event {
                        Event::Setting(Setting::Enabled {
                            enabled: next,
                            generation,
                        }) => {
                            enabled = next;
                            active_generation = generation;
                            output_overruns = 0;
                            output_window_start = Instant::now();
                            output_window_drops = 0;
                            current_ready.store(false, Ordering::Release);
                            if !enabled {
                                if let Some(filter) = &mut filter {
                                    filter.reset();
                                }
                                while frames_rx.try_recv().is_ok() {}
                                while stale_output_rx.try_recv().is_ok() {}
                                let _ = events.send(VoiceEvent::NoiseSuppressionReady {
                                    generation,
                                    result: Ok(()),
                                });
                                continue;
                            }
                            let result = if let Some(filter) = &mut filter {
                                filter.reset();
                                filter.set_suppression_intensity(WEB_SUPPRESSION_INTENSITY);
                                Ok(())
                            } else {
                                model_bytes(MODEL_WAIT)
                                    .and_then(|model| create_filter(&model))
                                    .map(|created| filter = Some(created))
                            };
                            if result.is_err() {
                                enabled = false;
                            }
                            if result.is_ok()
                                && current_generation.load(Ordering::Acquire) == generation
                            {
                                current_ready.store(true, Ordering::Release);
                            }
                            let _ = events
                                .send(VoiceEvent::NoiseSuppressionReady { generation, result });
                        }
                        Event::Setting(Setting::Reset) => {
                            if let Some(filter) = &mut filter {
                                filter.reset();
                            }
                            while frames_rx.try_recv().is_ok() {}
                            while stale_output_rx.try_recv().is_ok() {}
                        }
                        Event::Frame(mut samples) => {
                            if !enabled
                                || current_generation.load(Ordering::Acquire) != active_generation
                            {
                                continue;
                            }
                            let Some(filter) = &mut filter else { continue };
                            let mut failed = None;
                            for chunk in samples.chunks_mut(mezon_ns::FRAME_SIZE_48K) {
                                let mut block = [0i16; mezon_ns::FRAME_SIZE_48K];
                                block[..chunk.len()].copy_from_slice(chunk);
                                if let Err(e) = filter.process_frame(&mut block) {
                                    failed = Some(e.to_string());
                                    break;
                                }
                                let _ = agc.process_stream(&mut block, FILTERED_SAMPLE_RATE, 1);
                                chunk.copy_from_slice(&block[..chunk.len()]);
                            }
                            if let Some(error) = failed {
                                enabled = false;
                                current_ready.store(false, Ordering::Release);
                                let _ = events.send(VoiceEvent::NoiseSuppressionReady {
                                    generation: active_generation,
                                    result: Err(error),
                                });
                                continue;
                            }
                            match output_tx.try_send(FilteredFrame {
                                samples,
                                generation: active_generation,
                            }) {
                                Ok(()) => {}
                                Err(_) => {
                                    output_overruns += 1;
                                    if output_window_start.elapsed() >= std::time::Duration::from_secs(2) {
                                        output_window_start = Instant::now();
                                        output_window_drops = 0;
                                    }
                                    output_window_drops += 1;
                                    if output_overruns == 1 || output_overruns % 20 == 0 {
                                        tracing::warn!(
                                            generation = active_generation,
                                            output_overruns,
                                            drops_in_window = output_window_drops,
                                            queued_outputs = output_tx.len(),
                                            "Mezon-NS output queue full; filtered frame skipped"
                                        );
                                    }
                                    if output_window_drops >= 12 {
                                        enabled = false;
                                        current_ready.store(false, Ordering::Release);
                                        current_requested.store(false, Ordering::Release);
                                        tracing::error!(
                                            generation = active_generation,
                                            "Mezon-NS disabled after sustained filtered output loss"
                                        );
                                        let _ = events.send(VoiceEvent::NoiseSuppressionReady {
                                            generation: active_generation,
                                            result: Err("Noise filter output could not keep up with live audio".into()),
                                        });
                                    }
                                }
                            }
                        }
                        Event::Stop => break,
                    }
                }
            })
            .expect("spawn Mezon-NS audio worker");
        Self {
            requested,
            generation,
            ready,
            settings_tx,
            frames_tx,
            output_rx,
        }
    }

    pub fn set_enabled(&self, enabled: bool, generation: u64) {
        self.generation.store(generation, Ordering::Release);
        self.requested.store(enabled, Ordering::Release);
        self.ready.store(false, Ordering::Release);
        let _ = self.settings_tx.send(Setting::Enabled {
            enabled,
            generation,
        });
    }

    pub fn reset(&self) {
        let _ = self.settings_tx.send(Setting::Reset);
    }

    pub fn requested(&self) -> Arc<AtomicBool> {
        self.requested.clone()
    }
    pub fn generation(&self) -> Arc<AtomicU64> {
        self.generation.clone()
    }
    pub fn ready(&self) -> Arc<AtomicBool> {
        self.ready.clone()
    }
    pub fn frames(&self) -> flume::Sender<Vec<i16>> {
        self.frames_tx.clone()
    }
}
