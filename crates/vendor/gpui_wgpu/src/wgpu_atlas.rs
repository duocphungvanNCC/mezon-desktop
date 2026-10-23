use anyhow::{Context as _, Result};
use collections::FxHashMap;
use etagere::{BucketedAtlasAllocator, size2};
use gpui::{
    AtlasKey, AtlasTextureId, AtlasTextureKind, AtlasTextureList, AtlasTile, Bounds, DevicePixels,
    PlatformAtlas, Point, Size,
};
use parking_lot::Mutex;
use std::{borrow::Cow, ops, sync::Arc};

use crate::WgpuContext;

fn device_size_to_etagere(size: Size<DevicePixels>) -> etagere::Size {
    size2(size.width.0, size.height.0)
}

fn etagere_point_to_device(point: etagere::Point) -> Point<DevicePixels> {
    Point {
        x: DevicePixels(point.x),
        y: DevicePixels(point.y),
    }
}

pub struct WgpuAtlas(Mutex<WgpuAtlasState>);

struct PendingUpload {
    id: AtlasTextureId,
    bounds: Bounds<DevicePixels>,
    data: Vec<u8>,
}

const POOLED_UPLOAD_MIN_BYTES: usize = 64 * 1024;
const MAX_POOLED_UPLOAD_BUFFERS: usize = 4;

struct WgpuAtlasState {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    max_texture_size: u32,
    color_texture_format: wgpu::TextureFormat,
    storage: WgpuAtlasStorage,
    tiles_by_key: FxHashMap<AtlasKey, AtlasTile>,
    pending_uploads: Vec<PendingUpload>,
    upload_buffer_pool: Vec<Vec<u8>>,
}

pub struct WgpuTextureInfo {
    pub view: wgpu::TextureView,
}

impl WgpuAtlas {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        color_texture_format: wgpu::TextureFormat,
    ) -> Self {
        let max_texture_size = device.limits().max_texture_dimension_2d;
        WgpuAtlas(Mutex::new(WgpuAtlasState {
            device,
            queue,
            max_texture_size,
            color_texture_format,
            storage: WgpuAtlasStorage::default(),
            tiles_by_key: Default::default(),
            pending_uploads: Vec::new(),
            upload_buffer_pool: Vec::new(),
        }))
    }

    pub fn from_context(context: &WgpuContext) -> Self {
        Self::new(
            context.device.clone(),
            context.queue.clone(),
            context.color_texture_format(),
        )
    }

    pub fn before_frame(&self) {
        let mut lock = self.0.lock();
        lock.flush_uploads();
    }

    pub fn get_texture_info(&self, id: AtlasTextureId) -> Option<WgpuTextureInfo> {
        let lock = self.0.lock();
        let texture = lock.storage.get(id)?;
        Some(WgpuTextureInfo {
            view: texture.view.clone(),
        })
    }

    pub fn clear(&self) {
        let mut lock = self.0.lock();
        lock.storage.destroy_all();
        lock.tiles_by_key.clear();
        lock.pending_uploads.clear();
        lock.upload_buffer_pool.clear();
    }

    pub fn handle_device_lost(&self, context: &WgpuContext) {
        let mut lock = self.0.lock();
        lock.device = context.device.clone();
        lock.queue = context.queue.clone();
        lock.color_texture_format = context.color_texture_format();
        lock.storage.destroy_all();
        lock.tiles_by_key.clear();
        lock.pending_uploads.clear();
        lock.upload_buffer_pool.clear();
    }
}

impl PlatformAtlas for WgpuAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        let mut lock = self.0.lock();
        if let Some(tile) = lock.tiles_by_key.get(key) {
            Ok(Some(*tile))
        } else {
            profiling::scope!("new tile");
            let Some((size, bytes)) = build()? else {
                return Ok(None);
            };
            let tile = lock
                .allocate(size, key.texture_kind())
                .context("failed to allocate")?;
            lock.upload_texture(tile.texture_id, tile.bounds, &bytes);
            lock.tiles_by_key.insert(key.clone(), tile);
            Ok(Some(tile))
        }
    }

    fn replace(
        &self,
        key: &AtlasKey,
        size: Size<DevicePixels>,
        bytes: &[u8],
    ) -> anyhow::Result<()> {
        {
            let mut lock = self.0.lock();
            if let Some(tile) = lock.tiles_by_key.get(key).copied()
                && tile.bounds.size == size
            {
                lock.upload_texture(tile.texture_id, tile.bounds, bytes);
                return Ok(());
            }
        }
        self.remove(key);
        Ok(())
    }

    fn remove(&self, key: &AtlasKey) {
        let mut lock = self.0.lock();

        let Some(tile) = lock.tiles_by_key.remove(key) else {
            return;
        };
        let id = tile.texture_id;
        lock.pending_uploads
            .retain(|upload| !(upload.id == id && upload.bounds == tile.bounds));

        let Some(texture_slot) = lock.storage[id.kind].textures.get_mut(id.index as usize) else {
            return;
        };

        if let Some(mut texture) = texture_slot.take() {
            texture.allocator.deallocate(tile.tile_id.into());
            texture.decrement_ref_count();
            if texture.is_unreferenced() {
                lock.pending_uploads
                    .retain(|upload| upload.id != texture.id);
                lock.upload_buffer_pool.clear();
                texture.texture.destroy();
                lock.storage[id.kind]
                    .free_list
                    .push(texture.id.index as usize);
            } else {
                *texture_slot = Some(texture);
            }
        }
    }
}

impl WgpuAtlasState {
    fn allocate(
        &mut self,
        size: Size<DevicePixels>,
        texture_kind: AtlasTextureKind,
    ) -> Option<AtlasTile> {
        const SMALL_IMAGE_TILE_MAX: i32 = 256;
        let texture_kind = if texture_kind == AtlasTextureKind::Image
            && size.width.0 <= SMALL_IMAGE_TILE_MAX
            && size.height.0 <= SMALL_IMAGE_TILE_MAX
        {
            AtlasTextureKind::ImageSmall
        } else {
            texture_kind
        };
        {
            let textures = &mut self.storage[texture_kind];

            if let Some(tile) = textures
                .iter_mut()
                .rev()
                .find_map(|texture| texture.allocate(size))
            {
                return Some(tile);
            }
        }

        let texture = self.push_texture(size, texture_kind);
        texture.allocate(size)
    }

    fn push_texture(
        &mut self,
        min_size: Size<DevicePixels>,
        kind: AtlasTextureKind,
    ) -> &mut WgpuAtlasTexture {
        const DEFAULT_ATLAS_SIZE: Size<DevicePixels> = Size {
            width: DevicePixels(1024),
            height: DevicePixels(1024),
        };
        const DEFAULT_COLOR_ATLAS_SIZE: Size<DevicePixels> = Size {
            width: DevicePixels(512),
            height: DevicePixels(512),
        };
        let default_size = match kind {
            AtlasTextureKind::Polychrome
            | AtlasTextureKind::Image
            | AtlasTextureKind::ImageSmall => DEFAULT_COLOR_ATLAS_SIZE,
            AtlasTextureKind::Monochrome | AtlasTextureKind::Subpixel => DEFAULT_ATLAS_SIZE,
        };
        let max_texture_size = self.max_texture_size as i32;
        let max_atlas_size = Size {
            width: DevicePixels(max_texture_size),
            height: DevicePixels(max_texture_size),
        };

        let size = min_size.min(&max_atlas_size).max(&default_size);
        let format = match kind {
            AtlasTextureKind::Monochrome => wgpu::TextureFormat::R8Unorm,
            AtlasTextureKind::Subpixel
            | AtlasTextureKind::Polychrome
            | AtlasTextureKind::Image
            | AtlasTextureKind::ImageSmall => {
                self.color_texture_format
            }
        };

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: size.width.0 as u32,
                height: size.height.0 as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_list = &mut self.storage[kind];
        let index = texture_list.free_list.pop();

        let atlas_texture = WgpuAtlasTexture {
            id: AtlasTextureId {
                index: index.unwrap_or(texture_list.textures.len()) as u32,
                kind,
            },
            allocator: BucketedAtlasAllocator::new(device_size_to_etagere(size)),
            format,
            texture,
            view,
            live_atlas_keys: 0,
        };

        if let Some(ix) = index {
            texture_list.textures[ix] = Some(atlas_texture);
            texture_list
                .textures
                .get_mut(ix)
                .and_then(|t| t.as_mut())
                .expect("texture must exist")
        } else {
            texture_list.textures.push(Some(atlas_texture));
            texture_list
                .textures
                .last_mut()
                .and_then(|t| t.as_mut())
                .expect("texture must exist")
        }
    }

    fn take_upload_buffer(&mut self, len: usize) -> Vec<u8> {
        if len >= POOLED_UPLOAD_MIN_BYTES
            && let Some(idx) = self
                .upload_buffer_pool
                .iter()
                .enumerate()
                .filter(|(_, buf)| buf.capacity() >= len)
                .min_by_key(|(_, buf)| buf.capacity())
                .map(|(idx, _)| idx)
                .or_else(|| {
                    self.upload_buffer_pool
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, buf)| buf.capacity())
                        .map(|(idx, _)| idx)
                })
        {
            return self.upload_buffer_pool.swap_remove(idx);
        }
        Vec::with_capacity(len)
    }

    fn upload_texture(&mut self, id: AtlasTextureId, bounds: Bounds<DevicePixels>, bytes: &[u8]) {
        let Some(texture) = self.storage.get(id) else {
            return;
        };
        let format = texture.format;
        if let Some(existing) = self
            .pending_uploads
            .iter_mut()
            .find(|upload| upload.id == id && upload.bounds == bounds)
        {
            swizzle_upload_data_into(bytes, format, &mut existing.data);
            return;
        }
        let mut data = self.take_upload_buffer(bytes.len());
        swizzle_upload_data_into(bytes, format, &mut data);
        self.pending_uploads
            .push(PendingUpload { id, bounds, data });
    }

    fn flush_uploads(&mut self) {
        let mut pending = std::mem::take(&mut self.pending_uploads);
        for mut upload in pending.drain(..) {
            if let Some(texture) = self.storage.get(upload.id) {
                let bytes_per_pixel = texture.bytes_per_pixel();

                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: upload.bounds.origin.x.0 as u32,
                            y: upload.bounds.origin.y.0 as u32,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &upload.data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(
                            upload.bounds.size.width.0 as u32 * bytes_per_pixel as u32,
                        ),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: upload.bounds.size.width.0 as u32,
                        height: upload.bounds.size.height.0 as u32,
                        depth_or_array_layers: 1,
                    },
                );
            }
            if upload.data.capacity() >= POOLED_UPLOAD_MIN_BYTES
                && self.upload_buffer_pool.len() < MAX_POOLED_UPLOAD_BUFFERS
            {
                upload.data.clear();
                self.upload_buffer_pool.push(upload.data);
            }
        }
        self.pending_uploads = pending;
    }
}

#[derive(Default)]
struct WgpuAtlasStorage {
    monochrome_textures: AtlasTextureList<WgpuAtlasTexture>,
    subpixel_textures: AtlasTextureList<WgpuAtlasTexture>,
    polychrome_textures: AtlasTextureList<WgpuAtlasTexture>,
    image_textures: AtlasTextureList<WgpuAtlasTexture>,
    image_small_textures: AtlasTextureList<WgpuAtlasTexture>,
}

impl ops::Index<AtlasTextureKind> for WgpuAtlasStorage {
    type Output = AtlasTextureList<WgpuAtlasTexture>;
    fn index(&self, kind: AtlasTextureKind) -> &Self::Output {
        match kind {
            AtlasTextureKind::Monochrome => &self.monochrome_textures,
            AtlasTextureKind::Subpixel => &self.subpixel_textures,
            AtlasTextureKind::Polychrome => &self.polychrome_textures,
            AtlasTextureKind::Image => &self.image_textures,
            AtlasTextureKind::ImageSmall => &self.image_small_textures,
        }
    }
}

impl ops::IndexMut<AtlasTextureKind> for WgpuAtlasStorage {
    fn index_mut(&mut self, kind: AtlasTextureKind) -> &mut Self::Output {
        match kind {
            AtlasTextureKind::Monochrome => &mut self.monochrome_textures,
            AtlasTextureKind::Subpixel => &mut self.subpixel_textures,
            AtlasTextureKind::Polychrome => &mut self.polychrome_textures,
            AtlasTextureKind::Image => &mut self.image_textures,
            AtlasTextureKind::ImageSmall => &mut self.image_small_textures,
        }
    }
}

impl WgpuAtlasStorage {
    fn get(&self, id: AtlasTextureId) -> Option<&WgpuAtlasTexture> {
        self[id.kind]
            .textures
            .get(id.index as usize)
            .and_then(|t| t.as_ref())
    }

    fn destroy_all(&mut self) {
        for kind in [
            AtlasTextureKind::Monochrome,
            AtlasTextureKind::Subpixel,
            AtlasTextureKind::Polychrome,
            AtlasTextureKind::Image,
            AtlasTextureKind::ImageSmall,
        ] {
            for texture in self[kind].textures.drain(..).flatten() {
                texture.texture.destroy();
            }
            self[kind].free_list.clear();
        }
    }
}

impl ops::Index<AtlasTextureId> for WgpuAtlasStorage {
    type Output = WgpuAtlasTexture;
    fn index(&self, id: AtlasTextureId) -> &Self::Output {
        let textures = match id.kind {
            AtlasTextureKind::Monochrome => &self.monochrome_textures,
            AtlasTextureKind::Subpixel => &self.subpixel_textures,
            AtlasTextureKind::Polychrome => &self.polychrome_textures,
            AtlasTextureKind::Image => &self.image_textures,
            AtlasTextureKind::ImageSmall => &self.image_small_textures,
        };
        textures[id.index as usize]
            .as_ref()
            .expect("texture must exist")
    }
}

struct WgpuAtlasTexture {
    id: AtlasTextureId,
    allocator: BucketedAtlasAllocator,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    live_atlas_keys: u32,
}

impl WgpuAtlasTexture {
    fn allocate(&mut self, size: Size<DevicePixels>) -> Option<AtlasTile> {
        let allocation = self.allocator.allocate(device_size_to_etagere(size))?;
        let tile = AtlasTile {
            texture_id: self.id,
            tile_id: allocation.id.into(),
            padding: 0,
            bounds: Bounds {
                origin: etagere_point_to_device(allocation.rectangle.min),
                size,
            },
        };
        self.live_atlas_keys += 1;
        Some(tile)
    }

    fn bytes_per_pixel(&self) -> u8 {
        match self.format {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm => 4,
            _ => 4,
        }
    }

    fn decrement_ref_count(&mut self) {
        self.live_atlas_keys -= 1;
    }

    fn is_unreferenced(&self) -> bool {
        self.live_atlas_keys == 0
    }
}

fn swizzle_upload_data_into(bytes: &[u8], format: wgpu::TextureFormat, out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(bytes);
    if format == wgpu::TextureFormat::Rgba8Unorm {
        for pixel in out.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
}

#[cfg(test)]
fn swizzle_upload_data(bytes: &[u8], format: wgpu::TextureFormat) -> Vec<u8> {
    let mut data = Vec::with_capacity(bytes.len());
    swizzle_upload_data_into(bytes, format, &mut data);
    data
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;
    use gpui::block_on;
    use gpui::{ImageId, RenderImageParams};
    use std::sync::Arc;

    fn test_device_and_queue() -> anyhow::Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
        block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                flags: wgpu::InstanceFlags::default(),
                backend_options: wgpu::BackendOptions::default(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                display: None,
            });
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
                .map_err(|error| anyhow::anyhow!("failed to request adapter: {error}"))?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("wgpu_atlas_test_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults()
                        .using_resolution(adapter.limits())
                        .using_alignment(adapter.limits()),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                })
                .await
                .map_err(|error| anyhow::anyhow!("failed to request device: {error}"))?;
            Ok((Arc::new(device), Arc::new(queue)))
        })
    }

    #[test]
    fn before_frame_skips_uploads_for_removed_texture() -> anyhow::Result<()> {
        let (device, queue) = test_device_and_queue()?;

        let atlas = WgpuAtlas::new(device, queue, wgpu::TextureFormat::Bgra8Unorm);
        let key = AtlasKey::Image(RenderImageParams {
            image_id: ImageId(1),
            frame_index: 0,
        });
        let size = Size {
            width: DevicePixels(1),
            height: DevicePixels(1),
        };
        let mut build = || Ok(Some((size, Cow::Owned(vec![0, 0, 0, 255]))));

        // Regression test: before the fix, this panicked in flush_uploads
        atlas
            .get_or_insert_with(&key, &mut build)?
            .expect("tile should be created");
        atlas.remove(&key);
        atlas.before_frame();
        Ok(())
    }

    #[test]
    fn remove_deallocates_tile_space_for_reuse() -> anyhow::Result<()> {
        let (device, queue) = test_device_and_queue()?;
        let atlas = WgpuAtlas::new(device, queue, wgpu::TextureFormat::Bgra8Unorm);

        let small = Size {
            width: DevicePixels(64),
            height: DevicePixels(64),
        };
        let big = Size {
            width: DevicePixels(700),
            height: DevicePixels(700),
        };

        let make_key = |image_id: usize| {
            AtlasKey::Image(RenderImageParams {
                image_id: ImageId(image_id),
                frame_index: 0,
            })
        };
        let insert = |key: &AtlasKey, size: Size<DevicePixels>| {
            let byte_count = (size.width.0 as usize) * (size.height.0 as usize) * 4;
            atlas
                .get_or_insert_with(key, &mut || {
                    Ok(Some((size, Cow::Owned(vec![0u8; byte_count]))))
                })
                .expect("allocation should succeed")
                .expect("callback returns Some")
        };

        let keeper_key = make_key(1);
        let big_key_a = make_key(2);
        let big_key_b = make_key(3);

        let keeper_tile = insert(&keeper_key, small);
        let tile_a = insert(&big_key_a, big);
        assert_eq!(keeper_tile.texture_id, tile_a.texture_id);

        atlas.remove(&big_key_a);
        let tile_b = insert(&big_key_b, big);
        assert_eq!(tile_b.texture_id, keeper_tile.texture_id);
        Ok(())
    }

    #[test]
    fn swizzle_upload_data_preserves_bgra_uploads() {
        let input = vec![0x10, 0x20, 0x30, 0x40];
        assert_eq!(
            swizzle_upload_data(&input, wgpu::TextureFormat::Bgra8Unorm),
            input
        );
    }

    #[test]
    fn swizzle_upload_data_converts_bgra_to_rgba() {
        let input = vec![0x10, 0x20, 0x30, 0x40, 0xAA, 0xBB, 0xCC, 0xDD];
        assert_eq!(
            swizzle_upload_data(&input, wgpu::TextureFormat::Rgba8Unorm),
            vec![0x30, 0x20, 0x10, 0x40, 0xCC, 0xBB, 0xAA, 0xDD]
        );
    }

    #[cfg(target_os = "linux")]
    fn linux_rss_kb() -> usize {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status.lines().find_map(|line| {
                    line.strip_prefix("VmRSS:")
                        .and_then(|rest| rest.split_whitespace().next())
                        .and_then(|val| val.parse::<usize>().ok())
                })
            })
            .unwrap_or(0)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn replace_while_surface_unpresented_stays_bounded() -> anyhow::Result<()> {
        let (device, queue) = test_device_and_queue()?;
        let atlas = WgpuAtlas::new(device, queue, wgpu::TextureFormat::Bgra8Unorm);
        let key = AtlasKey::Image(RenderImageParams {
            image_id: ImageId(99),
            frame_index: 0,
        });
        let size = Size {
            width: DevicePixels(1920),
            height: DevicePixels(1080),
        };
        let frame_bytes = (size.width.0 as usize) * (size.height.0 as usize) * 4;
        let frame = vec![0x42u8; frame_bytes];
        atlas
            .get_or_insert_with(&key, &mut || Ok(Some((size, Cow::Borrowed(&frame)))))?
            .expect("video tile should be allocated");

        let rss_before_kb = linux_rss_kb();
        for i in 0..200u8 {
            let mut next = frame.clone();
            next[0] = i;
            atlas.replace(&key, size, &next)?;
        }
        let rss_peak_kb = linux_rss_kb();
        let growth_mb = rss_peak_kb.saturating_sub(rss_before_kb) / 1024;
        eprintln!(
            "[DOCKER-REPRO] 200x 1080p replace without submit: rss_before={} MB, rss_after={} MB, leaked_growth={} MB",
            rss_before_kb / 1024,
            rss_peak_kb / 1024,
            growth_mb
        );
        assert!(
            growth_mb < 64,
            "unsubmitted video frame replaces leaked {growth_mb} MB across 200 frames"
        );
        Ok(())
    }
}
