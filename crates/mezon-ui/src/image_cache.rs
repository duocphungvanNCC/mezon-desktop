use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use futures::future::{AbortHandle, Abortable};
use futures::{AsyncReadExt as _, FutureExt};
use gpui::{
    App, AppContext, Asset, AssetLogger, Context, Entity, Global, ImageCache, ImageCacheError,
    ImageCacheItem, RenderImage, Resource, Window, hash,
};
use indexmap::IndexMap;

#[derive(Default)]
struct PendingAtlasDrops(Vec<Arc<RenderImage>>);
impl Global for PendingAtlasDrops {}

pub(crate) fn queue_atlas_drop(cx: &mut App, image: Arc<RenderImage>) {
    cx.default_global::<PendingAtlasDrops>().0.push(image);
}

#[derive(Default)]
struct PendingAtlasReplaces(Vec<Arc<RenderImage>>);
impl Global for PendingAtlasReplaces {}

#[cfg_attr(target_os = "macos", expect(dead_code))]
pub(crate) fn queue_atlas_replace(cx: &mut App, image: Arc<RenderImage>) {
    let pending = &mut cx.default_global::<PendingAtlasReplaces>().0;
    if let Some(existing) = pending.iter_mut().find(|queued| queued.id == image.id) {
        *existing = image;
    } else {
        pending.push(image);
    }
}

pub fn flush_atlas_replaces(window: &mut Window, cx: &mut App) {
    let pending = std::mem::take(&mut cx.default_global::<PendingAtlasReplaces>().0);
    for image in pending {
        cx.update_render_image(&image, Some(window));
    }
}

const RELIEF_FLUSH_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

pub fn flush_atlas_drops(window: &mut Window, cx: &mut App) {
    let pending = std::mem::take(&mut cx.default_global::<PendingAtlasDrops>().0);
    if pending.is_empty() {
        return;
    }
    let mut freed_bytes = 0u64;
    for image in pending {
        freed_bytes = freed_bytes.saturating_add(image_bytes(&image));
        cx.drop_image(image, Some(window));
    }
    if freed_bytes >= RELIEF_FLUSH_THRESHOLD_BYTES {
        release_freed_memory_to_os(cx);
    }
}

const IDLE_TRIM_INTERVAL: Duration = Duration::from_secs(30);
const IDLE_TRIM_TTL: Duration = Duration::from_secs(60);

/// Decode-completion notifies are coalesced per view: a burst of images
/// finishing across many frames (fast scroll, channel open) would otherwise
/// trigger one full re-render of the list per completion frame.
const DECODE_NOTIFY_DEBOUNCE: Duration = Duration::from_millis(50);

#[derive(Default)]
struct PendingDecodeNotifies(std::collections::HashSet<gpui::EntityId>);
impl Global for PendingDecodeNotifies {}

fn schedule_decode_notify(entity: gpui::EntityId, cx: &mut App) {
    if !cx
        .default_global::<PendingDecodeNotifies>()
        .0
        .insert(entity)
    {
        return;
    }
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        executor.timer(DECODE_NOTIFY_DEBOUNCE).await;
        cx.update(|cx| {
            cx.default_global::<PendingDecodeNotifies>()
                .0
                .remove(&entity);
            cx.notify(entity);
        });
    })
    .detach();
}

#[derive(Default)]
struct IdleTrimRegistry(Vec<gpui::WeakEntity<LruImageCache>>);
impl Global for IdleTrimRegistry {}

static IDLE_TRIM_STARTED: AtomicBool = AtomicBool::new(false);

pub fn start_idle_trim(cx: &mut App) {
    if IDLE_TRIM_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        loop {
            executor.timer(IDLE_TRIM_INTERVAL).await;
            cx.update(|cx| {
                let registry = std::mem::take(&mut cx.default_global::<IdleTrimRegistry>().0);
                let mut live = Vec::with_capacity(registry.len());
                let mut evicted = false;
                for weak in registry {
                    if let Some(cache) = weak.upgrade() {
                        evicted |=
                            cache.update(cx, |cache, cx| cache.evict_idle(IDLE_TRIM_TTL, cx));
                        live.push(weak);
                    }
                }
                cx.default_global::<IdleTrimRegistry>().0.extend(live);
                if evicted {
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}

const SHARED_AVATAR_CACHE_CAPACITY: usize = 512;
const SHARED_AVATAR_CACHE_BYTES: u64 = 24 * 1024 * 1024;
const SHARED_SMALL_AVATAR_CACHE_BYTES: u64 = 12 * 1024 * 1024;
const SHARED_ROLE_ICON_CACHE_CAPACITY: usize = 512;
const SHARED_ROLE_ICON_CACHE_BYTES: u64 = 4 * 1024 * 1024;
const ROLE_ICON_ENTRY_MAX_BYTES: u64 = 256 * 1024;
const SHARED_EMOJI_CACHE_CAPACITY: usize = 256;
const SHARED_EMOJI_CACHE_BYTES: u64 = 12 * 1024 * 1024;

struct SharedAvatarCache(Entity<LruImageCache>);
impl Global for SharedAvatarCache {}

struct SharedRoleIconCache(Entity<LruImageCache>);
impl Global for SharedRoleIconCache {}

struct SharedRoleIconPreviewCache(Entity<LruImageCache>);
impl Global for SharedRoleIconPreviewCache {}

const OGP_TIMELINE_CACHE_CAPACITY: usize = 12;
const OGP_TIMELINE_CACHE_BYTES: u64 = 6 * 1024 * 1024;
const OGP_AUX_CACHE_CAPACITY: usize = 4;
const OGP_AUX_CACHE_BYTES: u64 = 2 * 1024 * 1024;
const OGP_ENTRY_MAX_BYTES: u64 = 1024 * 1024;

pub fn ogp_timeline_cache(label: &'static str, cx: &mut App) -> Entity<LruImageCache> {
    ogp_preview_cache(
        label,
        OGP_TIMELINE_CACHE_CAPACITY,
        OGP_TIMELINE_CACHE_BYTES,
        cx,
    )
}

pub fn ogp_aux_cache(label: &'static str, cx: &mut App) -> Entity<LruImageCache> {
    ogp_preview_cache(label, OGP_AUX_CACHE_CAPACITY, OGP_AUX_CACHE_BYTES, cx)
}

fn ogp_preview_cache(
    label: &'static str,
    capacity: usize,
    bytes: u64,
    cx: &mut App,
) -> Entity<LruImageCache> {
    cx.new(|cx| LruImageCache::ogp_thumbnail(label, capacity, bytes, OGP_ENTRY_MAX_BYTES, cx))
}

/// Shared decode cache for role icons. They render at 12-20px everywhere, so
/// they go through the `IconThumbnail` loader (decodes at `ICON_DECODE_MAX_PX`)
/// instead of the app-wide `"shared"` cache, whose `LoaderKind::Full` would keep
/// the source resolution resident.
/// Read-only view of the shared role-icon cache for render paths that only hold
/// `&App`. Returns `None` before the first `shared_role_icon_cache` call.
/// One 64pt preview lives in the role-icon picker modal. It needs a 128px decode,
/// so it uses the avatar loader (`AVATAR_DECODE_MAX_PX` = 160) rather than the
/// 64px icon loader, with a budget sized for the single image it holds.
pub fn shared_role_icon_preview_cache(cx: &mut App) -> Entity<LruImageCache> {
    if let Some(existing) = cx.try_global::<SharedRoleIconPreviewCache>() {
        return existing.0.clone();
    }
    let cache = cx.new(|cx| {
        LruImageCache::avatar_thumbnail("role-icon-preview", 16, 1024 * 1024, 256 * 1024, cx)
    });
    cx.set_global(SharedRoleIconPreviewCache(cache.clone()));
    cache
}

pub fn role_icon_cache(cx: &App) -> Option<Entity<LruImageCache>> {
    cx.try_global::<SharedRoleIconCache>().map(|c| c.0.clone())
}

pub fn shared_role_icon_cache(cx: &mut App) -> Entity<LruImageCache> {
    if let Some(existing) = cx.try_global::<SharedRoleIconCache>() {
        return existing.0.clone();
    }
    let cache = cx.new(|cx| {
        LruImageCache::icon_thumbnail(
            "role-icons-shared",
            SHARED_ROLE_ICON_CACHE_CAPACITY,
            SHARED_ROLE_ICON_CACHE_BYTES,
            ROLE_ICON_ENTRY_MAX_BYTES,
            cx,
        )
    });
    cx.set_global(SharedRoleIconCache(cache.clone()));
    cache
}

pub fn shared_avatar_cache(cx: &mut App) -> Entity<LruImageCache> {
    if let Some(existing) = cx.try_global::<SharedAvatarCache>() {
        return existing.0.clone();
    }
    let cache = cx.new(|cx| {
        LruImageCache::avatar_thumbnail(
            "avatar-shared",
            SHARED_AVATAR_CACHE_CAPACITY,
            SHARED_AVATAR_CACHE_BYTES,
            AVATAR_ENTRY_MAX_BYTES,
            cx,
        )
    });
    cx.set_global(SharedAvatarCache(cache.clone()));
    cache
}

struct SharedSmallAvatarCache(Entity<LruImageCache>);
impl Global for SharedSmallAvatarCache {}

struct SharedEmojiCache(Entity<LruImageCache>);
impl Global for SharedEmojiCache {}

/// Shared decode cache for emoji painted by surfaces that carry no cache of
/// their own — context menus and their submenus, voice-room reactions. Without
/// one, `img()` resolves to GPUI's global asset cache, which decodes at full
/// resolution, keeps every animation frame and never evicts, so each distinct
/// emoji the user ever sees stays resident for the rest of the session.
///
/// A `deferred` boundary needs the cache attached *inside* its own subtree: a
/// `DeferredDraw` replay paints with an empty `image_cache_stack`, so an
/// ancestor cache does not reach it.
pub fn shared_emoji_cache(cx: &mut App) -> Entity<LruImageCache> {
    if let Some(existing) = cx.try_global::<SharedEmojiCache>() {
        return existing.0.clone();
    }
    let cache = cx.new(|cx| {
        LruImageCache::avatar_thumbnail(
            "emoji-shared",
            SHARED_EMOJI_CACHE_CAPACITY,
            SHARED_EMOJI_CACHE_BYTES,
            AVATAR_ENTRY_MAX_BYTES,
            cx,
        )
    });
    cx.set_global(SharedEmojiCache(cache.clone()));
    cache
}

pub fn shared_small_avatar_cache(cx: &mut App) -> Entity<LruImageCache> {
    if let Some(existing) = cx.try_global::<SharedSmallAvatarCache>() {
        return existing.0.clone();
    }
    let cache = cx.new(|cx| {
        LruImageCache::avatar_thumbnail_small(
            "avatar-shared-small",
            SHARED_AVATAR_CACHE_CAPACITY,
            SHARED_SMALL_AVATAR_CACHE_BYTES,
            AVATAR_ENTRY_MAX_BYTES,
            cx,
        )
    });
    cx.set_global(SharedSmallAvatarCache(cache.clone()));
    cache
}

pub fn clear_all_image_caches(cx: &mut App) {
    let registry = std::mem::take(&mut cx.default_global::<IdleTrimRegistry>().0);
    let mut live = Vec::with_capacity(registry.len());
    for weak in registry {
        if let Some(cache) = weak.upgrade() {
            cache.update(cx, |cache, cx| cache.clear_app(cx));
            live.push(weak);
        }
    }
    cx.default_global::<IdleTrimRegistry>().0.extend(live);
}

#[cfg(target_os = "macos")]
mod os_mem {
    use std::ffi::c_void;

    unsafe extern "C" {
        fn malloc_default_zone() -> *mut c_void;
        fn malloc_zone_pressure_relief(zone: *mut c_void, goal: usize) -> usize;
    }

    pub fn release_freed_pages() {
        unsafe {
            let zone = malloc_default_zone();
            if !zone.is_null() {
                malloc_zone_pressure_relief(zone, 0);
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod os_mem {
    pub fn release_freed_pages() {
        unsafe {
            libc::malloc_trim(0);
        }
    }
}

#[cfg(target_os = "windows")]
mod os_mem {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessHeap() -> *mut c_void;
        fn HeapCompact(heap: *mut c_void, flags: u32) -> usize;
    }

    pub fn release_freed_pages() {
        unsafe {
            let heap = GetProcessHeap();
            if !heap.is_null() {
                HeapCompact(heap, 0);
            }
        }
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", target_env = "gnu")
))]
static MEMORY_RELIEF_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

pub fn release_freed_memory_to_os(cx: &mut App) {
    #[cfg(any(
        target_os = "macos",
        target_os = "windows",
        all(target_os = "linux", target_env = "gnu")
    ))]
    {
        if MEMORY_RELIEF_IN_FLIGHT.swap(true, Ordering::AcqRel) {
            return;
        }
        cx.background_executor()
            .spawn(async {
                os_mem::release_freed_pages();
                MEMORY_RELIEF_IN_FLIGHT.store(false, Ordering::Release);
            })
            .detach();
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        all(target_os = "linux", target_env = "gnu")
    )))]
    let _ = cx;
}

pub(crate) const AVATAR_FETCH_MAX_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const GALLERY_FETCH_MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MESSAGE_FETCH_MAX_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const VIEWER_FETCH_MAX_BYTES: usize = 32 * 1024 * 1024;
const IMAGE_PIPELINE_CONCURRENCY: usize = 3;
static IMAGE_PIPELINE_PERMITS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(IMAGE_PIPELINE_CONCURRENCY)));

async fn acquire_image_pipeline_permit()
-> Result<tokio::sync::OwnedSemaphorePermit, ImageCacheError> {
    IMAGE_PIPELINE_PERMITS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            ImageCacheError::Other(Arc::new(anyhow::anyhow!("image pipeline semaphore closed")))
        })
}

pub(crate) async fn read_body_limited(
    response: &mut gpui::http_client::Response<gpui::http_client::AsyncBody>,
    limit: usize,
) -> std::io::Result<Vec<u8>> {
    if let Some(length) = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        && length > limit as u64
    {
        return Err(std::io::Error::other(format!(
            "response body of {length} bytes exceeds the {limit} byte transfer limit"
        )));
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .await?;
    if body.len() > limit {
        return Err(std::io::Error::other(format!(
            "response body exceeds the {limit} byte transfer limit"
        )));
    }
    Ok(body)
}

pub const MESSAGE_IMAGE_CACHE_CAPACITY: usize = 48;
pub const MESSAGE_IMAGE_CACHE_BYTES: u64 = 32 * 1024 * 1024;
pub const AVATAR_IMAGE_CACHE_CAPACITY: usize = 256;
pub const AVATAR_IMAGE_CACHE_BYTES: u64 = 8 * 1024 * 1024;

pub const VIEWER_IMAGE_CACHE_CAPACITY: usize = 24;
pub const VIEWER_IMAGE_CACHE_BYTES: u64 = 32 * 1024 * 1024;
pub const VIEWER_IMAGE_ENTRY_MAX_BYTES: u64 = 24 * 1024 * 1024;

/// App-wide fallback cache attached at the root, so any `img`/avatar that does
/// not declare its own cache uses this bounded LRU instead of GPUI's unbounded
/// global asset cache (which never evicts and leaks RAM for every URL seen).
pub const SHARED_IMAGE_CACHE_CAPACITY: usize = 384;
pub const SHARED_IMAGE_CACHE_BYTES: u64 = 24 * 1024 * 1024;
pub const GALLERY_IMAGE_CACHE_CAPACITY: usize = 96;
pub const GALLERY_IMAGE_CACHE_BYTES: u64 = 16 * 1024 * 1024;

pub const PREVIEW_IMAGE_CACHE_CAPACITY: usize = 64;
pub const PREVIEW_IMAGE_CACHE_BYTES: u64 = 32 * 1024 * 1024;
pub const PREVIEW_ENTRY_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Per-image decoded-size caps. A compressed file is tiny on the wire but is
/// stored uncompressed in RAM as `width * height * 4` bytes *per frame*. An
/// animated GIF/WebP therefore explodes: a ~400 KB animated avatar can decode
/// to hundreds of MB once every frame is expanded. When the resizing image
/// proxy is unavailable (dev, or a prod outage) we fall back to the raw,
/// full-resolution file, so we guard against a single pathological image
/// blowing up RAM by refusing to retain anything decoded larger than this and
/// negatively caching it (shown as the initials fallback instead).
pub const AVATAR_ANIMATION_MAX_BYTES: u64 = 4 * 1024 * 1024;
pub const AVATAR_ENTRY_MAX_BYTES: u64 = 2 * 1024 * 1024;
pub const MESSAGE_ENTRY_MAX_BYTES: u64 = 32 * 1024 * 1024;
pub const MESSAGE_ANIMATION_MAX_BYTES: u64 = MESSAGE_IMAGE_CACHE_BYTES / 4;
pub const SHARED_ENTRY_MAX_BYTES: u64 = 12 * 1024 * 1024;

const GRACE_PERIOD: Duration = Duration::from_secs(2);
const FRAME_BUMP_REARM: Duration = Duration::from_millis(100);
const STATS_LOG_INTERVAL: u64 = 600;
const MESSAGE_ANIMATION_MAX_PX: u32 = 400;
const MESSAGE_STATIC_MAX_PX: u32 = 1024;
const SHARED_ANIMATION_MAX_PX: u32 = 400;
const SHARED_STATIC_MAX_PX: u32 = 2048;
/// Longest side (px) that an animated GIF/WebP is downscaled to for the image
/// viewer. Larger than the message cap since the viewer shows media bigger,
/// but still bounded so a long animation cannot expand to hundreds of MB.
const VIEWER_ANIMATION_MAX_PX: u32 = 480;
const VIEWER_STATIC_MAX_PX: u32 = 1600;

#[derive(Default)]
struct CacheMetrics {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub current_bytes: u64,
    pub items: usize,
}

struct CacheEntry {
    item: ImageCacheItem,
    abort: AbortHandle,
    /// Decoded size in bytes, once the image has finished loading.
    bytes: Option<u64>,
    /// The sweep epoch in which this entry was last requested.
    touched_epoch: u64,
    last_used: Instant,
    /// When a transient load failure was first observed. Deterministic
    /// failures — oversized-image rejections (`bytes == Some(0)`), canvas/
    /// dimension guards (`Asset`), decode/limits errors (`Image`), bad SVGs
    /// (`Usvg`) — are never retried; only network-shaped failures (`Io`,
    /// `BadStatus`, `Other`) are retried once per
    /// [`NEGATIVE_CACHE_RETRY_TTL`] while the image keeps being requested.
    failed_at: Option<Instant>,
}

const NEGATIVE_CACHE_RETRY_TTL: Duration = Duration::from_secs(15);

/// Sum of the decoded byte size across all frames of an image.
fn image_bytes(image: &RenderImage) -> u64 {
    (0..image.frame_count())
        .filter_map(|frame| image.as_bytes(frame))
        .map(|buf| buf.len() as u64)
        .sum()
}

fn entry_in_working_set(touched_epoch: u64, epoch: u64) -> bool {
    epoch.wrapping_sub(touched_epoch) <= 1
}

fn entry_is_stale(touched_epoch: u64, epoch: u64, age: Duration, grace: Duration) -> bool {
    touched_epoch != epoch && age > grace
}

fn entry_is_idle(touched_epoch: u64, epoch: u64, age: Duration, ttl: Duration) -> bool {
    touched_epoch != epoch && age > ttl
}

/// An LRU image cache bounded by both an item count and a decoded-byte budget.
///
/// The byte budget is what actually keeps RAM in check: large attachments are
/// evicted as soon as the total decoded size exceeds `max_bytes`, instead of
/// lingering until the (much larger) item count or a channel switch clears them.
static CACHE_INSTANCE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Which decoder a cache uses to turn a resource into a `RenderImage`.
#[derive(Clone, Copy)]
enum LoaderKind {
    /// Bounded general-purpose loader for the app-wide fallback cache (OGP
    /// embeds, misc `img()` without an explicit cache): static images capped at
    /// [`SHARED_STATIC_MAX_PX`], animations at [`SHARED_ANIMATION_MAX_PX`] with
    /// an in-decode byte budget so a pathological file cannot spike RAM while
    /// decoding.
    Full,
    /// Decodes only the first frame and downscales to avatar size, so even an
    /// animated full-resolution source costs ~100 KB of RAM. Used for avatars.
    AvatarThumbnail,
    AvatarThumbnailSmall,
    IconThumbnail,
    GalleryThumbnail,
    /// Aspect-preserving, animation-preserving thumbnail for the sticker picker,
    /// capped at [`STICKER_DECODE_MAX_PX`].
    StickerThumbnail,
    /// Aspect-preserving thumbnail for OGP link previews, capped at
    /// [`OGP_THUMB_DECODE_MAX_PX`].
    OgpThumbnail,
    /// Aspect-preserving thumbnail for Timeline/Events/Event-Detail preview
    /// cards, capped at [`GALLERY_PREVIEW_DECODE_MAX_PX`].
    GalleryPreview,
    Message,
    /// The image-viewer loader: still images keep near-full resolution
    /// ([`VIEWER_STATIC_MAX_PX`]); animated GIF/WebP keep every frame so they
    /// animate, but downscaled to [`VIEWER_ANIMATION_MAX_PX`] and bounded by an
    /// in-decode byte budget.
    Viewer,
}

pub struct LruImageCache {
    label: &'static str,
    instance: u64,
    loader: LoaderKind,
    max_items: usize,
    max_bytes: u64,
    /// Largest decoded size (bytes, summed across frames) a single entry may
    /// have before it is dropped and negatively cached. Protects against a
    /// single huge/animated image consuming hundreds of MB.
    max_entry_bytes: u64,
    total_bytes: u64,
    epoch: u64,
    frame_bump_armed: Option<Instant>,
    frame_elapsed: bool,
    weak: gpui::WeakEntity<Self>,
    sweeps: u64,
    sweep_scheduled: bool,
    metrics: CacheMetrics,
    cache: IndexMap<u64, CacheEntry>,
}

impl LruImageCache {
    pub fn cached_render_image(&self, resource: &Resource) -> Option<Arc<RenderImage>> {
        self.cache
            .get(&hash(resource))
            .and_then(|entry| match &entry.item {
                ImageCacheItem::Loaded(Ok(image)) => Some(image.clone()),
                _ => None,
            })
    }

    pub fn new(max_items: usize, max_bytes: u64, cx: &mut Context<Self>) -> Self {
        Self::labeled("image", max_items, max_bytes, u64::MAX, cx)
    }

    pub fn labeled(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::Full,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    /// A cache for avatars: decodes only the first frame and downscales to
    /// avatar size, so animated or oversized sources can never blow up RAM.
    pub fn avatar_thumbnail(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::AvatarThumbnail,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    pub fn avatar_thumbnail_small(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::AvatarThumbnailSmall,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    pub fn icon_thumbnail(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::IconThumbnail,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    pub fn gallery_thumbnail(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::GalleryThumbnail,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    pub fn sticker_thumbnail(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::StickerThumbnail,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    pub fn ogp_thumbnail(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::OgpThumbnail,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    /// A cache for Timeline/Events/Event-Detail preview cards: aspect-preserving,
    /// downscaled to [`GALLERY_PREVIEW_DECODE_MAX_PX`] so landscape banners and
    /// square grid cells both stay sharp under `object-fit: cover` without the
    /// full-resolution decode blowing the cache's byte budget.
    pub fn gallery_preview(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::GalleryPreview,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    pub fn message(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::Message,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    /// A cache for the image viewer: decodes only the first frame at full
    /// resolution. The viewer renders a single static frame, so this avoids
    /// retaining every frame of an animated GIF/WebP (which the viewer never
    /// shows) while keeping full-resolution quality for still images.
    pub fn viewer(
        label: &'static str,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_loader(
            label,
            LoaderKind::Viewer,
            max_items,
            max_bytes,
            max_entry_bytes,
            cx,
        )
    }

    fn with_loader(
        label: &'static str,
        loader: LoaderKind,
        max_items: usize,
        max_bytes: u64,
        max_entry_bytes: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|cache, cx| {
            for (_, mut entry) in std::mem::take(&mut cache.cache) {
                entry.abort.abort();
                if let Some(Ok(image)) = entry.item.get() {
                    queue_atlas_drop(cx, image);
                }
            }
        })
        .detach();

        let instance = CACHE_INSTANCE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let weak = cx.weak_entity();
        cx.default_global::<IdleTrimRegistry>().0.push(weak.clone());
        Self {
            label,
            instance,
            loader,
            max_items,
            max_bytes,
            max_entry_bytes,
            total_bytes: 0,
            epoch: 0,
            frame_bump_armed: None,
            frame_elapsed: false,
            weak,
            sweeps: 0,
            sweep_scheduled: false,
            metrics: CacheMetrics::default(),
            cache: IndexMap::with_capacity(max_items),
        }
    }

    pub fn stats(&self) -> ImageCacheStats {
        ImageCacheStats {
            hits: self.metrics.hits.load(Ordering::Relaxed),
            misses: self.metrics.misses.load(Ordering::Relaxed),
            evictions: self.metrics.evictions.load(Ordering::Relaxed),
            current_bytes: self.total_bytes,
            items: self.cache.len(),
        }
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        for (_, mut entry) in std::mem::take(&mut self.cache) {
            entry.abort.abort();
            if let Some(Ok(image)) = entry.item.get() {
                cx.drop_image(image, Some(window));
            }
        }
        self.total_bytes = 0;
    }

    pub fn clear_app(&mut self, cx: &mut App) {
        for (_, mut entry) in std::mem::take(&mut self.cache) {
            entry.abort.abort();
            if let Some(Ok(image)) = entry.item.get() {
                queue_atlas_drop(cx, image);
            }
        }
        self.total_bytes = 0;
    }

    fn advance_epoch_on_use(&mut self) {
        if self.frame_elapsed {
            self.frame_elapsed = false;
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    fn begin_frame(&mut self, window: &mut Window) {
        self.advance_epoch_on_use();
        if self
            .frame_bump_armed
            .is_some_and(|at| at.elapsed() < FRAME_BUMP_REARM)
        {
            return;
        }
        self.frame_bump_armed = Some(Instant::now());
        let weak = self.weak.clone();
        window.on_next_frame(move |_, cx| {
            weak.update(cx, |cache, _| {
                cache.frame_bump_armed = None;
                cache.frame_elapsed = true;
            })
            .ok();
        });
    }

    /// Drop every image that has not been requested for [`GRACE_PERIOD`]. Call
    /// this once per render: anything that has scrolled out of the viewport
    /// stops being requested and is freed on the next sweep, so only the
    /// currently-visible images stay in RAM.
    pub fn sweep(&mut self, window: &mut Window, cx: &mut App) {
        self.sweep_with_grace(GRACE_PERIOD, window, cx);
    }

    pub fn sweep_once_per_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sweep_scheduled {
            return;
        }
        self.sweep_scheduled = true;
        self.sweep(window, cx);
        cx.on_next_frame(window, |cache, _, _| cache.sweep_scheduled = false);
    }

    fn sweep_with_grace(&mut self, grace: Duration, window: &mut Window, cx: &mut App) {
        let epoch = self.epoch;
        let metrics = &self.metrics;
        let total_bytes = &mut self.total_bytes;
        self.cache.retain(|_, entry| {
            if !entry_is_stale(entry.touched_epoch, epoch, entry.last_used.elapsed(), grace) {
                return true;
            }
            entry.abort.abort();
            metrics.evictions.fetch_add(1, Ordering::Relaxed);
            if let Some(bytes) = entry.bytes {
                *total_bytes = total_bytes.saturating_sub(bytes);
            }
            if let Some(Ok(image)) = entry.item.get() {
                cx.drop_image(image, Some(&mut *window));
            }
            false
        });
        self.begin_frame(window);
        self.sweeps = self.sweeps.wrapping_add(1);
        if self.sweeps.is_multiple_of(STATS_LOG_INTERVAL) {
            let stats = self.stats();
            tracing::debug!(
                label = self.label,
                instance = self.instance,
                hits = stats.hits,
                misses = stats.misses,
                evictions = stats.evictions,
                current_bytes = stats.current_bytes,
                items = stats.items,
                "image cache stats"
            );
        }
    }

    fn evict_idle(&mut self, ttl: Duration, cx: &mut App) -> bool {
        let previous_len = self.cache.len();
        let metrics = &self.metrics;
        let total_bytes = &mut self.total_bytes;
        self.cache.retain(|_, entry| {
            if !entry_is_idle(
                entry.touched_epoch,
                self.epoch,
                entry.last_used.elapsed(),
                ttl,
            ) {
                return true;
            }
            entry.abort.abort();
            metrics.evictions.fetch_add(1, Ordering::Relaxed);
            if let Some(bytes) = entry.bytes {
                *total_bytes = total_bytes.saturating_sub(bytes);
            }
            if let Some(Ok(image)) = entry.item.get() {
                queue_atlas_drop(cx, image);
            }
            false
        });
        self.cache.len() < previous_len
    }

    pub fn shrink_to(&mut self, max_bytes: u64, window: &mut Window, cx: &mut App) {
        while self.total_bytes > max_bytes {
            let Some(victim) = self.lru_index() else {
                break;
            };
            let Some((_, mut evicted)) = self.cache.swap_remove_index(victim) else {
                break;
            };
            evicted.abort.abort();
            self.metrics.evictions.fetch_add(1, Ordering::Relaxed);
            if let Some(bytes) = evicted.bytes {
                self.total_bytes = self.total_bytes.saturating_sub(bytes);
            }
            if let Some(Ok(image)) = evicted.item.get() {
                cx.drop_image(image, Some(window));
            }
        }
    }

    /// Evict least-recently-used entries until both the item-count and
    /// byte budgets are satisfied. The victim is the entry with the oldest
    /// `last_used` timestamp (map order no longer tracks recency); the final
    /// remaining entry is never evicted, so the image requested this frame
    /// stays resident.
    ///
    /// The in-viewport working set (entries touched this frame or the previous
    /// one — rows below the current paint cursor were last touched one frame
    /// ago) is never a victim: when the visible images alone exceed the byte
    /// budget, evicting one of them just makes the next frame re-decode it and
    /// evict its neighbour, blinking a different visible image every frame.
    /// The sweep and the idle trim remain the bounds that free them once they
    /// scroll out.
    fn lru_index(&self) -> Option<usize> {
        self.cache
            .values()
            .enumerate()
            .filter(|(_, entry)| !entry_in_working_set(entry.touched_epoch, self.epoch))
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(index, _)| index)
    }

    fn evict_to_budget(&mut self, window: &mut Window, cx: &mut App) {
        while self.cache.len() > self.max_items
            || (self.total_bytes > self.max_bytes && self.cache.len() > 1)
        {
            let Some(victim) = self.lru_index() else {
                break;
            };
            let Some((_, mut evicted)) = self.cache.swap_remove_index(victim) else {
                break;
            };
            evicted.abort.abort();
            self.metrics.evictions.fetch_add(1, Ordering::Relaxed);
            if let Some(bytes) = evicted.bytes {
                self.total_bytes = self.total_bytes.saturating_sub(bytes);
            }
            if let Some(Ok(image)) = evicted.item.get() {
                cx.drop_image(image, Some(window));
            }
        }
    }

    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.begin_frame(window);
        let hash = hash(resource);

        if self.cache.contains_key(&hash) {
            let retry_failed = {
                let entry = self.cache.get_mut(&hash).expect("checked contains_key");
                let transient_failure = entry.bytes.is_none()
                    && matches!(
                        entry.item.get(),
                        Some(Err(ImageCacheError::Io(_)
                            | ImageCacheError::BadStatus { .. }
                            | ImageCacheError::Other(_)))
                    );
                if transient_failure {
                    match entry.failed_at {
                        Some(failed_at) => failed_at.elapsed() >= NEGATIVE_CACHE_RETRY_TTL,
                        None => {
                            entry.failed_at = Some(Instant::now());
                            false
                        }
                    }
                } else {
                    false
                }
            };
            if retry_failed {
                self.cache.swap_remove(&hash);
            }
        }

        if self.cache.contains_key(&hash) {
            self.metrics.hits.fetch_add(1, Ordering::Relaxed);

            enum Measured {
                /// Nothing new to account for (already measured, or still loading).
                None,
                /// Newly decoded image of the given size, kept in the cache.
                Kept(u64),
                /// Newly decoded image exceeded the per-entry cap: dropped and
                /// negatively cached. Carries the image to free + the error.
                TooLarge(Arc<RenderImage>, ImageCacheError),
            }

            let (res, measured) = {
                let entry = self.cache.get_mut(&hash).expect("checked contains_key");
                entry.touched_epoch = self.epoch;
                entry.last_used = Instant::now();
                let res = entry.item.get();
                let measured = if entry.bytes.is_none()
                    && let Some(Ok(image)) = res.as_ref()
                {
                    let bytes = image_bytes(image);
                    if bytes > self.max_entry_bytes {
                        let err = ImageCacheError::Other(Arc::new(anyhow::anyhow!(
                            "image decoded to {bytes} bytes, exceeds per-entry cap of {} bytes",
                            self.max_entry_bytes
                        )));
                        entry.item = ImageCacheItem::Loaded(Err(err.clone()));
                        entry.bytes = Some(0);
                        Measured::TooLarge(image.clone(), err)
                    } else {
                        entry.bytes = Some(bytes);
                        Measured::Kept(bytes)
                    }
                } else {
                    Measured::None
                };
                (res, measured)
            };
            match measured {
                Measured::Kept(bytes) => {
                    self.total_bytes = self.total_bytes.saturating_add(bytes);
                    self.evict_to_budget(window, cx);
                    return res;
                }
                Measured::TooLarge(image, err) => {
                    tracing::warn!(
                        "[imgcache:{}#{}] dropping oversized image: {}",
                        self.label,
                        self.instance,
                        err
                    );
                    cx.drop_image(image, Some(window));
                    return Some(Err(err));
                }
                Measured::None => return res,
            }
        }

        self.metrics.misses.fetch_add(1, Ordering::Relaxed);
        let loader = match self.loader {
            LoaderKind::Full => {
                AssetLogger::<SharedImageLoader>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::AvatarThumbnail => {
                AssetLogger::<AvatarImageLoader>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::AvatarThumbnailSmall => {
                AssetLogger::<AvatarImageLoaderSmall>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::IconThumbnail => {
                AssetLogger::<IconImageLoader>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::GalleryThumbnail => {
                AssetLogger::<GalleryImageLoader>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::StickerThumbnail => {
                AssetLogger::<StickerImageLoader>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::OgpThumbnail => {
                AssetLogger::<OgpImageLoader>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::GalleryPreview => {
                AssetLogger::<GalleryPreviewLoader>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::Message => {
                AssetLogger::<MessageImageLoader>::load(resource.clone(), cx).boxed()
            }
            LoaderKind::Viewer => {
                AssetLogger::<ViewerImageLoader>::load(resource.clone(), cx).boxed()
            }
        };
        let task = cx.background_executor().spawn(loader).shared();
        let (abort_handle, abort_reg) = AbortHandle::new_pair();

        self.cache.insert(
            hash,
            CacheEntry {
                item: ImageCacheItem::Loading(task.clone()),
                abort: abort_handle,
                bytes: None,
                touched_epoch: self.epoch,
                last_used: Instant::now(),
                failed_at: None,
            },
        );
        self.evict_to_budget(window, cx);

        let entity = window.current_view();
        let notify_task = task.clone();
        window
            .spawn(cx, async move |cx| {
                let _ = Abortable::new(notify_task, abort_reg).await;
                let _ = cx.update(|_, cx| schedule_decode_notify(entity, cx));
            })
            .detach();

        None
    }
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        LruImageCache::load(self, resource, window, cx)
    }
}

/// Largest dimension (device pixels) an avatar is ever drawn at: the biggest
/// avatar is 80px logical, which is 160px on a 2x display. Decoding to this size
/// keeps a single avatar at ~100 KB regardless of the source file.
const AVATAR_DECODE_MAX_PX: u32 = 160;
const AVATAR_SMALL_DECODE_MAX_PX: u32 = 80;
const ICON_DECODE_MAX_PX: u32 = 64;
const GALLERY_THUMB_DECODE_MAX_PX: u32 = 320;
/// OGP link-preview thumbnails render at ≤200px tall; decode to 512px longest
/// side (aspect-preserving, ~2x for retina) so a large external OG image
/// (typically 1200×630) can never decode oversized in the preview card.
const OGP_THUMB_DECODE_MAX_PX: u32 = 512;
/// Timeline/Events/Event-Detail preview cards render up to ~900px wide
/// (featured banner) at aspect ratio; decode to this longest side so the
/// featured image stays sharp while grid/card thumbnails (much smaller on
/// screen) downscale further for free via `object-fit: cover`.
const GALLERY_PREVIEW_DECODE_MAX_PX: u32 = 768;
/// Sticker cells are 80px logical, so 160px covers a 2x display. The animation
/// budget bounds an animated sticker to ~20 frames at that size; the decoder
/// decimates longer animations instead of dropping them to a still frame.
const STICKER_DECODE_MAX_PX: u32 = 160;
const STICKER_ANIMATION_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// An [`Asset`] loader for avatars that, unlike GPUI's stock [`ImageAssetLoader`],
/// decodes **only the first frame** and **downscales** to avatar size before
/// building the `RenderImage`.
///
/// GPUI's loader expands every frame of an animated GIF/WebP to
/// `width * height * 4` uncompressed bytes and keeps them all, so a ~400 KB
/// animated avatar can decode to hundreds of MB. Avatars never need animation
/// or full resolution, so we sidestep that entirely: `image::load_from_memory`
/// reads a single frame even for animated formats, and we shrink it to at most
/// [`AVATAR_DECODE_MAX_PX`]. The result is a tiny, static image that cannot blow
/// up RAM even when the resizing image proxy is unavailable and we fall back to
/// the raw source file.
fn load_avatar_scaled(
    source: Resource,
    max_px: u32,
    cx: &mut App,
) -> impl Future<Output = Result<Arc<RenderImage>, ImageCacheError>> + Send + 'static {
    let client = cx.http_client();
    let svg_renderer = cx.svg_renderer();
    let asset_source = cx.asset_source().clone();
    async move {
        let _permit = acquire_image_pipeline_permit().await?;
        let bytes = match source.clone() {
            Resource::Path(uri) => {
                if let Some(decoded) = decode_scaled_dynamic_path(uri.as_ref(), max_px) {
                    return Ok(avatar_render_image(decoded, max_px));
                }
                std::fs::read(uri.as_ref())?
            }
            Resource::Uri(uri) => {
                use anyhow::Context as _;

                let mut response = client
                    .get(uri.as_ref(), ().into(), true)
                    .await
                    .with_context(|| format!("loading avatar from {uri:?}"))?;
                let body = read_body_limited(&mut response, AVATAR_FETCH_MAX_BYTES).await?;
                if !response.status().is_success() {
                    let mut body = String::from_utf8_lossy(&body).into_owned();
                    let first_line = body.lines().next().unwrap_or("").trim_end();
                    body.truncate(first_line.len());
                    return Err(ImageCacheError::BadStatus {
                        uri,
                        status: response.status(),
                        body,
                    });
                }
                body
            }
            Resource::Embedded(path) => match asset_source.load(&path).ok().flatten() {
                Some(data) => data.to_vec(),
                None => {
                    return Err(ImageCacheError::Asset(
                        format!("Embedded resource not found: {path}").into(),
                    ));
                }
            },
        };

        if image::guess_format(&bytes).is_ok() {
            let animation_budget = if max_px <= AVATAR_SMALL_DECODE_MAX_PX {
                AVATAR_ANIMATION_MAX_BYTES
            } else {
                AVATAR_ENTRY_MAX_BYTES
            };
            decode_avatar_image(&bytes, max_px, animation_budget)
        } else {
            svg_renderer
                .render_single_frame(&bytes, 1.0)
                .map_err(Into::into)
        }
    }
}

pub enum AvatarImageLoader {}

impl Asset for AvatarImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        load_avatar_scaled(source, AVATAR_DECODE_MAX_PX, cx)
    }
}

pub enum AvatarImageLoaderSmall {}

impl Asset for AvatarImageLoaderSmall {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        load_avatar_scaled(source, AVATAR_SMALL_DECODE_MAX_PX, cx)
    }
}

pub enum IconImageLoader {}

impl Asset for IconImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        load_avatar_scaled(source, ICON_DECODE_MAX_PX, cx)
    }
}

pub enum GalleryImageLoader {}

impl Asset for GalleryImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let client = cx.http_client();
        let svg_renderer = cx.svg_renderer();
        let asset_source = cx.asset_source().clone();
        async move {
            let _permit = acquire_image_pipeline_permit().await?;
            let bytes = match source.clone() {
                Resource::Path(uri) => std::fs::read(uri.as_ref())?,
                Resource::Uri(uri) => {
                    use anyhow::Context as _;

                    let mut response = client
                        .get(uri.as_ref(), ().into(), true)
                        .await
                        .with_context(|| format!("loading gallery image from {uri:?}"))?;
                    let body = read_body_limited(&mut response, GALLERY_FETCH_MAX_BYTES).await?;
                    if !response.status().is_success() {
                        let mut body = String::from_utf8_lossy(&body).into_owned();
                        let first_line = body.lines().next().unwrap_or("").trim_end();
                        body.truncate(first_line.len());
                        return Err(ImageCacheError::BadStatus {
                            uri,
                            status: response.status(),
                            body,
                        });
                    }
                    body
                }
                Resource::Embedded(path) => match asset_source.load(&path).ok().flatten() {
                    Some(data) => data.to_vec(),
                    None => {
                        return Err(ImageCacheError::Asset(
                            format!("Embedded resource not found: {path}").into(),
                        ));
                    }
                },
            };

            if image::guess_format(&bytes).is_ok() {
                let decoded = match decode_scaled_dynamic(&bytes, GALLERY_THUMB_DECODE_MAX_PX) {
                    Some(image) => image,
                    None => image::load_from_memory(&bytes)?,
                };
                let side = decoded
                    .width()
                    .min(decoded.height())
                    .clamp(1, GALLERY_THUMB_DECODE_MAX_PX);
                let mut data = decoded
                    .resize_to_fill(side, side, image::imageops::FilterType::Triangle)
                    .into_rgba8();
                for pixel in data.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                Ok(Arc::new(RenderImage::new(vec![image::Frame::new(data)])))
            } else {
                svg_renderer
                    .render_single_frame(&bytes, 1.0)
                    .map_err(Into::into)
            }
        }
    }
}

fn downscale_dimensions(width: u32, height: u32, max_px: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest == 0 || longest <= max_px {
        (width.max(1), height.max(1))
    } else {
        let scale = max_px as f32 / longest as f32;
        (
            ((width as f32 * scale).round() as u32).max(1),
            ((height as f32 * scale).round() as u32).max(1),
        )
    }
}

fn bgra_frame(decoded: image::DynamicImage) -> image::Frame {
    let mut data = decoded.into_rgba8();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    image::Frame::new(data)
}

fn downscaled_static_frame(decoded: image::DynamicImage, max_px: u32) -> image::Frame {
    let (tw, th) = downscale_dimensions(decoded.width(), decoded.height(), max_px);
    let decoded = if tw == decoded.width() && th == decoded.height() {
        decoded
    } else {
        decoded.resize(tw, th, image::imageops::FilterType::Triangle)
    };
    bgra_frame(decoded)
}

enum AnimationDecodeError {
    BudgetExceeded,
    Image(ImageCacheError),
}

fn downscaled_animation_frames<I>(
    frames: I,
    max_px: u32,
    byte_budget: u64,
) -> Result<Vec<image::Frame>, AnimationDecodeError>
where
    I: Iterator<Item = image::ImageResult<image::Frame>>,
{
    let mut out: Vec<image::Frame> = Vec::new();
    let mut target: Option<(u32, u32)> = None;
    let mut stride = 1usize;
    let mut max_frames = usize::MAX;
    for (source_index, frame) in frames.enumerate() {
        let frame = frame.map_err(|err| AnimationDecodeError::Image(err.into()))?;
        let delay = frame.delay();
        let buffer = frame.into_buffer();
        let (tw, th) = *target
            .get_or_insert_with(|| downscale_dimensions(buffer.width(), buffer.height(), max_px));
        if max_frames == usize::MAX {
            let frame_bytes = u64::from(tw) * u64::from(th) * 4;
            if frame_bytes > byte_budget {
                return Err(AnimationDecodeError::BudgetExceeded);
            }
            max_frames = (byte_budget / frame_bytes).max(2) as usize;
        }

        if source_index.is_multiple_of(stride) && out.len() >= max_frames {
            out = out
                .into_iter()
                .step_by(2)
                .map(|frame| scale_frame_delay(frame, 2))
                .collect();
            stride = stride.saturating_mul(2);
        }
        if !source_index.is_multiple_of(stride) {
            continue;
        }

        let mut buffer = if buffer.width() == tw && buffer.height() == th {
            buffer
        } else {
            image::imageops::resize(&buffer, tw, th, image::imageops::FilterType::Triangle)
        };
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        out.push(image::Frame::from_parts(buffer, 0, 0, delay));
    }
    if out.is_empty() {
        return Err(AnimationDecodeError::Image(ImageCacheError::Other(
            Arc::new(anyhow::anyhow!("animation decoded to zero frames")),
        )));
    }
    Ok(out)
}

fn downscaled_avatar_animation_frames<I>(
    frames: I,
    max_px: u32,
    byte_budget: u64,
) -> Result<Vec<image::Frame>, AnimationDecodeError>
where
    I: Iterator<Item = image::ImageResult<image::Frame>>,
{
    let mut out: Vec<image::Frame> = Vec::new();
    let mut stride = 1usize;
    let mut max_frames = usize::MAX;
    for (source_index, frame) in frames.enumerate() {
        let frame = frame.map_err(|err| AnimationDecodeError::Image(err.into()))?;
        let delay = frame.delay();
        let buffer = frame.into_buffer();
        let side = buffer.width().min(buffer.height()).clamp(1, max_px);
        if max_frames == usize::MAX {
            let frame_bytes = u64::from(side) * u64::from(side) * 4;
            max_frames = (byte_budget / frame_bytes).max(2) as usize;
        }

        if source_index.is_multiple_of(stride) && out.len() >= max_frames {
            out = out
                .into_iter()
                .step_by(2)
                .map(|frame| scale_frame_delay(frame, 2))
                .collect();
            stride = stride.saturating_mul(2);
        }
        if !source_index.is_multiple_of(stride) {
            continue;
        }
        let mut buffer = image::DynamicImage::ImageRgba8(buffer)
            .resize_to_fill(side, side, image::imageops::FilterType::Triangle)
            .into_rgba8();
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let delay = image::Delay::from_saturating_duration(
            std::time::Duration::from(delay).saturating_mul(stride as u32),
        );
        out.push(image::Frame::from_parts(buffer, 0, 0, delay));
    }
    if out.is_empty() {
        return Err(AnimationDecodeError::Image(ImageCacheError::Other(
            Arc::new(anyhow::anyhow!("avatar animation decoded to zero frames")),
        )));
    }
    Ok(out)
}

fn scale_frame_delay(frame: image::Frame, factor: u32) -> image::Frame {
    let delay = image::Delay::from_saturating_duration(
        std::time::Duration::from(frame.delay()).saturating_mul(factor),
    );
    image::Frame::from_parts(frame.into_buffer(), 0, 0, delay)
}

fn scaled_to_dynamic(scaled: mezon_video::ScaledImage) -> Option<image::DynamicImage> {
    let mut rgba = scaled.bgra;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(scaled.width, scaled.height, rgba)?;
    Some(image::DynamicImage::ImageRgba8(buffer))
}

fn decode_scaled_dynamic(bytes: &[u8], max_px: u32) -> Option<image::DynamicImage> {
    scaled_to_dynamic(mezon_video::scaled_image_decode(bytes, max_px)?)
}

fn decode_scaled_dynamic_path(path: &std::path::Path, max_px: u32) -> Option<image::DynamicImage> {
    scaled_to_dynamic(mezon_video::scaled_image_decode_path(path, max_px)?)
}

fn avatar_frame(decoded: image::DynamicImage, max_px: u32) -> image::Frame {
    let side = decoded.width().min(decoded.height()).clamp(1, max_px);
    let mut data = decoded
        .resize_to_fill(side, side, image::imageops::FilterType::Triangle)
        .into_rgba8();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    image::Frame::new(data)
}

fn avatar_render_image(decoded: image::DynamicImage, max_px: u32) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![avatar_frame(decoded, max_px)]))
}

fn decode_static_image(
    bytes: &[u8],
    format: image::ImageFormat,
    max_px: u32,
) -> Result<image::DynamicImage, ImageCacheError> {
    if let Some(scaled) = decode_scaled_dynamic(bytes, max_px) {
        return Ok(scaled);
    }
    Ok(image::load_from_memory_with_format(bytes, format)?)
}

const MAX_DECODE_PIXELS: u64 = 48_000_000;
const MAX_DECODER_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

fn decoder_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(MAX_DECODER_ALLOC_BYTES);
    limits
}

fn reject_oversized_canvas(
    bytes: &[u8],
    format: image::ImageFormat,
) -> Result<(), ImageCacheError> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    reader.set_format(format);
    let (width, height) = reader.into_dimensions()?;
    if width > 16_384 || height > 16_384 {
        return Err(ImageCacheError::Asset(
            format!("image dimension too large to decode: {width}x{height}").into(),
        ));
    }
    if width as u64 * height as u64 > MAX_DECODE_PIXELS {
        return Err(ImageCacheError::Asset(
            format!("image dimensions too large to decode: {width}x{height}").into(),
        ));
    }
    Ok(())
}

fn decode_avatar_image(
    bytes: &[u8],
    max_px: u32,
    animation_byte_budget: u64,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    use image::AnimationDecoder as _;
    let format = image::guess_format(bytes)?;
    reject_oversized_canvas(bytes, format)?;
    let frames = match format {
        image::ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))?;
            image::ImageDecoder::set_limits(&mut decoder, decoder_limits())?;
            match downscaled_avatar_animation_frames(
                decoder.into_frames(),
                max_px,
                animation_byte_budget,
            ) {
                Ok(frames) => frames,
                Err(AnimationDecodeError::BudgetExceeded) => vec![avatar_frame(
                    decode_static_image(bytes, format, max_px)?,
                    max_px,
                )],
                Err(AnimationDecodeError::Image(err)) => return Err(err),
            }
        }
        image::ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes))?;
            image::ImageDecoder::set_limits(&mut decoder, decoder_limits())?;
            if decoder.has_animation() {
                match downscaled_avatar_animation_frames(
                    decoder.into_frames(),
                    max_px,
                    animation_byte_budget,
                ) {
                    Ok(frames) => frames,
                    Err(AnimationDecodeError::BudgetExceeded) => vec![avatar_frame(
                        decode_static_image(bytes, format, max_px)?,
                        max_px,
                    )],
                    Err(AnimationDecodeError::Image(err)) => return Err(err),
                }
            } else {
                vec![avatar_frame(
                    decode_static_image(bytes, format, max_px)?,
                    max_px,
                )]
            }
        }
        _ => vec![avatar_frame(
            decode_static_image(bytes, format, max_px)?,
            max_px,
        )],
    };
    Ok(Arc::new(RenderImage::new(frames)))
}

fn decode_message_image(
    bytes: &[u8],
    animation_max_px: u32,
    static_max_px: u32,
    animation_byte_budget: u64,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    use image::AnimationDecoder as _;
    let format = image::guess_format(bytes)?;
    reject_oversized_canvas(bytes, format)?;
    let frames = match format {
        image::ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))?;
            image::ImageDecoder::set_limits(&mut decoder, decoder_limits())?;
            match downscaled_animation_frames(
                decoder.into_frames(),
                animation_max_px,
                animation_byte_budget,
            ) {
                Ok(frames) => frames,
                Err(AnimationDecodeError::BudgetExceeded) => {
                    let mut decoder =
                        image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))?;
                    image::ImageDecoder::set_limits(&mut decoder, decoder_limits())?;
                    first_frame_fallback(decoder, static_max_px)?
                }
                Err(AnimationDecodeError::Image(err)) => return Err(err),
            }
        }
        image::ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes))?;
            image::ImageDecoder::set_limits(&mut decoder, decoder_limits())?;
            if decoder.has_animation() {
                match downscaled_animation_frames(
                    decoder.into_frames(),
                    animation_max_px,
                    animation_byte_budget,
                ) {
                    Ok(frames) => frames,
                    Err(AnimationDecodeError::BudgetExceeded) => {
                        let mut decoder =
                            image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes))?;
                        image::ImageDecoder::set_limits(&mut decoder, decoder_limits())?;
                        first_frame_fallback(decoder, static_max_px)?
                    }
                    Err(AnimationDecodeError::Image(err)) => return Err(err),
                }
            } else {
                vec![downscaled_static_frame(
                    decode_static_image(bytes, format, static_max_px)?,
                    static_max_px,
                )]
            }
        }
        _ => vec![downscaled_static_frame(
            decode_static_image(bytes, format, static_max_px)?,
            static_max_px,
        )],
    };
    Ok(Arc::new(RenderImage::new(frames)))
}

fn first_frame_fallback<'a, D>(
    decoder: D,
    static_max_px: u32,
) -> Result<Vec<image::Frame>, ImageCacheError>
where
    D: image::AnimationDecoder<'a>,
{
    let frame = decoder
        .into_frames()
        .next()
        .ok_or_else(|| ImageCacheError::Asset("animation has no frames".into()))??;
    let downscaled = downscaled_static_frame(
        image::DynamicImage::ImageRgba8(frame.into_buffer()),
        static_max_px,
    );
    Ok(vec![downscaled])
}

fn message_path_maybe_animated(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("gif") | Some("webp")
    )
}

/// An [`Asset`] loader for message attachments. Animated GIF/WebP are decoded at
/// every frame so they still animate, but each frame is downscaled to at most
/// [`MESSAGE_ANIMATION_MAX_PX`], so a long high-resolution animation cannot
/// expand to hundreds of MB of decoded BGRA. Static images keep full resolution.
/// Fetch + decode an image downscaled to `max_px` on the longest side,
/// **preserving aspect ratio** (unlike [`avatar_render_image`], which crops to
/// a square). Used for OGP banner thumbnails.
fn load_scaled_aspect(
    source: Resource,
    max_px: u32,
    cx: &mut App,
) -> impl Future<Output = Result<Arc<RenderImage>, ImageCacheError>> + Send + 'static {
    let client = cx.http_client();
    let svg_renderer = cx.svg_renderer();
    let asset_source = cx.asset_source().clone();
    async move {
        let _permit = acquire_image_pipeline_permit().await?;
        use anyhow::Context as _;
        let bytes = match source.clone() {
            Resource::Path(uri) => std::fs::read(uri.as_ref())?,
            Resource::Uri(uri) => {
                let mut response = client
                    .get(uri.as_ref(), ().into(), true)
                    .await
                    .with_context(|| format!("loading ogp image from {uri:?}"))?;
                let body = read_body_limited(&mut response, GALLERY_FETCH_MAX_BYTES).await?;
                if !response.status().is_success() {
                    let mut body = String::from_utf8_lossy(&body).into_owned();
                    let first_line = body.lines().next().unwrap_or("").trim_end();
                    body.truncate(first_line.len());
                    return Err(ImageCacheError::BadStatus {
                        uri,
                        status: response.status(),
                        body,
                    });
                }
                body
            }
            Resource::Embedded(path) => match asset_source.load(&path).ok().flatten() {
                Some(data) => data.to_vec(),
                None => {
                    return Err(ImageCacheError::Asset(
                        format!("Embedded resource not found: {path}").into(),
                    ));
                }
            },
        };
        if image::guess_format(&bytes).is_ok() {
            let decoded = match decode_scaled_dynamic(&bytes, max_px) {
                Some(image) => image,
                None => image::load_from_memory(&bytes)?,
            };
            Ok(Arc::new(RenderImage::new(vec![downscaled_static_frame(
                decoded, max_px,
            )])))
        } else {
            svg_renderer
                .render_single_frame(&bytes, 1.0)
                .map_err(Into::into)
        }
    }
}

pub enum OgpImageLoader {}

impl Asset for OgpImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        load_scaled_aspect(source, OGP_THUMB_DECODE_MAX_PX, cx)
    }
}

pub enum GalleryPreviewLoader {}

impl Asset for GalleryPreviewLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        load_scaled_aspect(source, GALLERY_PREVIEW_DECODE_MAX_PX, cx)
    }
}

pub enum MessageImageLoader {}

impl Asset for MessageImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let client = cx.http_client();
        let svg_renderer = cx.svg_renderer();
        let asset_source = cx.asset_source().clone();
        async move {
            let _permit = acquire_image_pipeline_permit().await?;
            let bytes = match source.clone() {
                Resource::Path(uri) => {
                    if !message_path_maybe_animated(uri.as_ref())
                        && let Some(decoded) =
                            decode_scaled_dynamic_path(uri.as_ref(), MESSAGE_STATIC_MAX_PX)
                    {
                        return Ok(Arc::new(RenderImage::new(vec![downscaled_static_frame(
                            decoded,
                            MESSAGE_STATIC_MAX_PX,
                        )])));
                    }
                    std::fs::read(uri.as_ref())?
                }
                Resource::Uri(uri) => {
                    use anyhow::Context as _;

                    let mut response = client
                        .get(uri.as_ref(), ().into(), true)
                        .await
                        .with_context(|| format!("loading image from {uri:?}"))?;
                    let body = read_body_limited(&mut response, MESSAGE_FETCH_MAX_BYTES).await?;
                    if !response.status().is_success() {
                        let mut body = String::from_utf8_lossy(&body).into_owned();
                        let first_line = body.lines().next().unwrap_or("").trim_end();
                        body.truncate(first_line.len());
                        return Err(ImageCacheError::BadStatus {
                            uri,
                            status: response.status(),
                            body,
                        });
                    }
                    body
                }
                Resource::Embedded(path) => match asset_source.load(&path).ok().flatten() {
                    Some(data) => data.to_vec(),
                    None => {
                        return Err(ImageCacheError::Asset(
                            format!("Embedded resource not found: {path}").into(),
                        ));
                    }
                },
            };

            if image::guess_format(&bytes).is_ok() {
                decode_message_image(
                    &bytes,
                    MESSAGE_ANIMATION_MAX_PX,
                    MESSAGE_STATIC_MAX_PX,
                    MESSAGE_ANIMATION_MAX_BYTES,
                )
            } else {
                svg_renderer
                    .render_single_frame(&bytes, 1.0)
                    .map_err(Into::into)
            }
        }
    }
}

/// Fetch + decode aspect-preserving, bounded by a static cap, an animation cap
/// and an in-decode byte budget. Animated GIF/WebP keep their frames (decimated
/// to fit the budget) so they still animate.
fn load_bounded(
    source: Resource,
    animation_max_px: u32,
    static_max_px: u32,
    animation_byte_budget: u64,
    cx: &mut App,
) -> impl Future<Output = Result<Arc<RenderImage>, ImageCacheError>> + Send + 'static {
    let client = cx.http_client();
    let svg_renderer = cx.svg_renderer();
    let asset_source = cx.asset_source().clone();
    async move {
        let _permit = acquire_image_pipeline_permit().await?;
        let bytes = match source.clone() {
            Resource::Path(uri) => std::fs::read(uri.as_ref())?,
            Resource::Uri(uri) => {
                use anyhow::Context as _;

                let mut response = client
                    .get(uri.as_ref(), ().into(), true)
                    .await
                    .with_context(|| format!("loading image from {uri:?}"))?;
                let body = read_body_limited(&mut response, MESSAGE_FETCH_MAX_BYTES).await?;
                if !response.status().is_success() {
                    let mut body = String::from_utf8_lossy(&body).into_owned();
                    let first_line = body.lines().next().unwrap_or("").trim_end();
                    body.truncate(first_line.len());
                    return Err(ImageCacheError::BadStatus {
                        uri,
                        status: response.status(),
                        body,
                    });
                }
                body
            }
            Resource::Embedded(path) => match asset_source.load(&path).ok().flatten() {
                Some(data) => data.to_vec(),
                None => {
                    return Err(ImageCacheError::Asset(
                        format!("Embedded resource not found: {path}").into(),
                    ));
                }
            },
        };

        if image::guess_format(&bytes).is_ok() {
            decode_message_image(
                &bytes,
                animation_max_px,
                static_max_px,
                animation_byte_budget,
            )
        } else {
            svg_renderer
                .render_single_frame(&bytes, 1.0)
                .map_err(Into::into)
        }
    }
}

/// Bounded loader for the app-wide fallback cache (`LoaderKind::Full`).
/// Replaces GPUI's stock `ImageAssetLoader`, which decodes at full resolution
/// and keeps every animation frame: statics are capped at
/// [`SHARED_STATIC_MAX_PX`], animations at [`SHARED_ANIMATION_MAX_PX`] with an
/// in-decode byte budget of [`SHARED_ENTRY_MAX_BYTES`].
pub enum SharedImageLoader {}

impl Asset for SharedImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        load_bounded(
            source,
            SHARED_ANIMATION_MAX_PX,
            SHARED_STATIC_MAX_PX,
            SHARED_ENTRY_MAX_BYTES,
            cx,
        )
    }
}

/// Loader for the sticker picker. Stickers are drawn in an 80px cell, so the
/// app-wide [`SHARED_STATIC_MAX_PX`] decode kept ~40x more pixels than the panel
/// can show and churned the panel's byte budget on every scroll. Unlike the
/// avatar loaders this preserves aspect ratio (stickers are not square and the
/// cell uses `object-fit: contain`) and keeps animation frames.
pub enum StickerImageLoader {}

impl Asset for StickerImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        load_bounded(
            source,
            STICKER_DECODE_MAX_PX,
            STICKER_DECODE_MAX_PX,
            STICKER_ANIMATION_MAX_BYTES,
            cx,
        )
    }
}

/// An [`Asset`] loader for the image viewer. Still images keep full resolution
/// (single frame), while animated GIF/WebP keep every frame but downscaled to
/// [`VIEWER_ANIMATION_MAX_PX`], so they animate in the viewer (matching the old
/// Electron/browser behaviour) without a full-resolution animation expanding to
/// `width * height * 4 * frames` bytes.
pub enum ViewerImageLoader {}

impl Asset for ViewerImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let client = cx.http_client();
        let svg_renderer = cx.svg_renderer();
        let asset_source = cx.asset_source().clone();
        async move {
            let _permit = acquire_image_pipeline_permit().await?;
            let bytes = match source.clone() {
                Resource::Path(uri) => std::fs::read(uri.as_ref())?,
                Resource::Uri(uri) => {
                    use anyhow::Context as _;

                    let mut response = client
                        .get(uri.as_ref(), ().into(), true)
                        .await
                        .with_context(|| format!("loading image from {uri:?}"))?;
                    let body = read_body_limited(&mut response, VIEWER_FETCH_MAX_BYTES).await?;
                    if !response.status().is_success() {
                        let mut body = String::from_utf8_lossy(&body).into_owned();
                        let first_line = body.lines().next().unwrap_or("").trim_end();
                        body.truncate(first_line.len());
                        return Err(ImageCacheError::BadStatus {
                            uri,
                            status: response.status(),
                            body,
                        });
                    }
                    body
                }
                Resource::Embedded(path) => match asset_source.load(&path).ok().flatten() {
                    Some(data) => data.to_vec(),
                    None => {
                        return Err(ImageCacheError::Asset(
                            format!("Embedded resource not found: {path}").into(),
                        ));
                    }
                },
            };

            if image::guess_format(&bytes).is_ok() {
                decode_message_image(
                    &bytes,
                    VIEWER_ANIMATION_MAX_PX,
                    VIEWER_STATIC_MAX_PX,
                    VIEWER_IMAGE_ENTRY_MAX_BYTES,
                )
            } else {
                svg_renderer
                    .render_single_frame(&bytes, 1.0)
                    .map_err(Into::into)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_cache_budgets_bound_the_visible_working_set() {
        assert_eq!(IMAGE_PIPELINE_CONCURRENCY, 3);
        assert_eq!(MESSAGE_FETCH_MAX_BYTES, 32 * 1024 * 1024);
        assert_eq!(VIEWER_FETCH_MAX_BYTES, 32 * 1024 * 1024);
        assert_eq!(MESSAGE_IMAGE_CACHE_BYTES, 32 * 1024 * 1024);
        assert_eq!(VIEWER_IMAGE_CACHE_BYTES, 32 * 1024 * 1024);
        assert_eq!(GALLERY_IMAGE_CACHE_BYTES, 16 * 1024 * 1024);
        assert_eq!(PREVIEW_IMAGE_CACHE_CAPACITY, 64);
        assert_eq!(PREVIEW_IMAGE_CACHE_BYTES, 32 * 1024 * 1024);
        assert_eq!(SHARED_IMAGE_CACHE_BYTES, 24 * 1024 * 1024);
        assert_eq!(OGP_TIMELINE_CACHE_BYTES, 6 * 1024 * 1024);
        assert_eq!(OGP_AUX_CACHE_BYTES, 2 * 1024 * 1024);
        assert_eq!(
            OGP_TIMELINE_CACHE_BYTES + 3 * OGP_AUX_CACHE_BYTES,
            12 * 1024 * 1024,
            "one timeline plus the search and two composer caches must stay near the old single-cache budget"
        );
        assert_eq!(IDLE_TRIM_INTERVAL, Duration::from_secs(30));
        assert_eq!(IDLE_TRIM_TTL, Duration::from_secs(60));
    }

    #[test]
    fn touched_this_epoch_is_never_stale() {
        assert!(!entry_is_stale(
            7,
            7,
            GRACE_PERIOD + Duration::from_secs(5),
            GRACE_PERIOD
        ));
    }

    #[gpui::test]
    fn the_epoch_advances_once_per_use_not_once_per_drawn_frame(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let cache = cx.new(|cx| LruImageCache::new(4, 1024, cx));
            cache.update(cx, |cache, _| {
                cache.frame_elapsed = true;
                cache.advance_epoch_on_use();
                cache.advance_epoch_on_use();
                assert_eq!(
                    cache.epoch, 1,
                    "every image requested during one frame must share an epoch"
                );

                for _ in 0..10 {
                    cache.advance_epoch_on_use();
                }
                assert_eq!(
                    cache.epoch, 1,
                    "frames in which this cache is not used must not age its entries out of \
                     the working set: their images are still on screen in a render-cache hit"
                );

                cache.frame_elapsed = true;
                cache.advance_epoch_on_use();
                assert_eq!(cache.epoch, 2);
            });
        });
    }

    #[gpui::test]
    fn budget_eviction_never_picks_an_entry_requested_this_frame(cx: &mut gpui::TestAppContext) {
        fn entry(touched_epoch: u64, age: Duration) -> CacheEntry {
            let (abort, _reg) = AbortHandle::new_pair();
            CacheEntry {
                item: ImageCacheItem::Loaded(Err(ImageCacheError::Asset("test".into()))),
                abort,
                bytes: Some(0),
                touched_epoch,
                last_used: Instant::now() - age,
                failed_at: None,
            }
        }

        cx.update(|cx| {
            let cache = cx.new(|cx| LruImageCache::new(4, 1024, cx));
            cache.update(cx, |cache, _| {
                cache.epoch = 9;
                cache.cache.insert(1, entry(9, Duration::from_secs(1)));
                cache.cache.insert(2, entry(8, Duration::ZERO));
                cache.cache.insert(3, entry(6, Duration::ZERO));
                assert_eq!(
                    cache.lru_index(),
                    Some(2),
                    "only the entry outside the two-frame working set may be evicted"
                );
                cache.cache.swap_remove(&3);
                assert_eq!(
                    cache.lru_index(),
                    None,
                    "a cache whose view never sweeps must still protect the images the \
                     current frame requested, or budget eviction blinks them"
                );
            });
        });
    }

    #[test]
    fn budget_eviction_spares_the_visible_working_set() {
        assert!(entry_in_working_set(7, 7));
        assert!(entry_in_working_set(6, 7));
        assert!(!entry_in_working_set(5, 7));
        assert!(entry_in_working_set(u64::MAX, 0));
    }

    #[test]
    fn untouched_within_grace_window_is_kept() {
        assert!(!entry_is_stale(6, 7, GRACE_PERIOD / 2, GRACE_PERIOD));
    }

    #[test]
    fn untouched_past_grace_window_is_evicted() {
        assert!(entry_is_stale(
            6,
            7,
            GRACE_PERIOD + Duration::from_millis(1),
            GRACE_PERIOD
        ));
    }

    #[gpui::test]
    fn each_view_gets_its_own_ogp_cache(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let timeline = ogp_timeline_cache("timeline-ogp", cx);
            let composer = ogp_aux_cache("composer-ogp", cx);

            assert_ne!(
                timeline.read(cx).instance,
                composer.read(cx).instance,
                "sharing one OGP cache lets a sweep from the view that rendered this frame \
                 evict previews that are still on screen in a view that did not"
            );
            assert!(
                timeline.read(cx).max_bytes > composer.read(cx).max_bytes,
                "the timeline holds many previews at once; the composer holds at most the pending one"
            );
        });
    }

    #[test]
    fn idle_trim_never_evicts_the_current_visible_epoch() {
        assert!(!entry_is_idle(
            7,
            7,
            IDLE_TRIM_TTL + Duration::from_secs(1),
            IDLE_TRIM_TTL
        ));
        assert!(entry_is_idle(
            6,
            7,
            IDLE_TRIM_TTL + Duration::from_secs(1),
            IDLE_TRIM_TTL
        ));
    }

    #[test]
    fn downscale_keeps_images_within_cap_unchanged() {
        assert_eq!(downscale_dimensions(300, 200, 400), (300, 200));
        assert_eq!(downscale_dimensions(400, 400, 400), (400, 400));
    }

    #[test]
    fn downscale_shrinks_oversized_preserving_aspect() {
        assert_eq!(downscale_dimensions(800, 400, 400), (400, 200));
        assert_eq!(downscale_dimensions(498, 498, 400), (400, 400));
    }

    #[test]
    fn downscale_handles_zero_dimension() {
        assert_eq!(downscale_dimensions(0, 0, 400), (1, 1));
    }

    #[test]
    fn static_frame_downscales_oversized_within_cap() {
        let source = image::DynamicImage::new_rgba8(800, 400);
        let frame = downscaled_static_frame(source, 400);
        let buffer = frame.buffer();
        assert_eq!((buffer.width(), buffer.height()), (400, 200));
    }

    #[test]
    fn static_frame_keeps_small_image_full_size() {
        let source = image::DynamicImage::new_rgba8(320, 240);
        let frame = downscaled_static_frame(source, 1600);
        let buffer = frame.buffer();
        assert_eq!((buffer.width(), buffer.height()), (320, 240));
    }

    #[test]
    fn message_static_cap_bounds_decoded_bytes() {
        let source = image::DynamicImage::new_rgba8(5120, 4096);
        let frame = downscaled_static_frame(source, MESSAGE_STATIC_MAX_PX);
        let buffer = frame.buffer();
        assert_eq!(buffer.width().max(buffer.height()), MESSAGE_STATIC_MAX_PX);
        let image = RenderImage::new(vec![frame]);
        assert!(image_bytes(&image) < MESSAGE_ENTRY_MAX_BYTES);
    }

    #[test]
    fn message_static_cap_covers_two_x_inline_display() {
        const MAX_INLINE_LOGICAL_PX: u32 = 480;
        const _: () = assert!(MESSAGE_STATIC_MAX_PX >= MAX_INLINE_LOGICAL_PX * 2);
    }

    #[test]
    fn picker_decode_caps_cover_two_x_their_largest_cell() {
        const EMOJI_PICKER_LARGEST_LOGICAL_PX: u32 = 28;
        const STICKER_PANEL_CELL_LOGICAL_PX: u32 = 80;
        const _: () = assert!(AVATAR_SMALL_DECODE_MAX_PX >= EMOJI_PICKER_LARGEST_LOGICAL_PX * 2);
        const _: () = assert!(STICKER_DECODE_MAX_PX >= STICKER_PANEL_CELL_LOGICAL_PX * 2);
    }

    #[test]
    fn a_screenful_of_picker_cells_fits_inside_its_cache_budget() {
        const EMOJI_PICKER_VISIBLE_CELLS: u64 = 9 * 12;
        const EMOJI_PICKER_CACHE_BYTES: u64 = 12 * 1024 * 1024;
        const STICKER_PANEL_VISIBLE_CELLS: u64 = 3 * 8;
        const STICKER_PANEL_CACHE_BYTES: u64 = 12 * 1024 * 1024;

        let bytes_at_cap = |max_px: u32| u64::from(max_px) * u64::from(max_px) * 4;

        assert!(
            bytes_at_cap(AVATAR_SMALL_DECODE_MAX_PX) * EMOJI_PICKER_VISIBLE_CELLS
                <= EMOJI_PICKER_CACHE_BYTES,
            "a screenful of emoji must fit the picker's budget: once the visible cells alone \
             exceed it, every frame evicts one of them and blinks a different cell"
        );
        assert!(
            bytes_at_cap(STICKER_DECODE_MAX_PX) * STICKER_PANEL_VISIBLE_CELLS
                <= STICKER_PANEL_CACHE_BYTES,
            "a screenful of stickers must fit the sticker panel's budget for the same reason"
        );
    }

    #[test]
    fn avatar_gif_retains_animation_frames() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).expect("GIF repeat");
            for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
                let buffer = image::RgbaImage::from_pixel(4, 2, image::Rgba(color));
                let frame = image::Frame::from_parts(
                    buffer,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(50, 1),
                );
                encoder.encode_frame(frame).expect("GIF frame");
            }
        }
        let image = decode_avatar_image(&bytes, AVATAR_SMALL_DECODE_MAX_PX, AVATAR_ENTRY_MAX_BYTES)
            .expect("animated avatar");
        assert_eq!(image.frame_count(), 2);
        let size = image.size(0);
        assert_eq!(size.width, size.height);
        assert_eq!(image.delay(0), image::Delay::from_numer_denom_ms(50, 1));
    }

    #[test]
    fn one_animated_attachment_cannot_starve_the_message_cache() {
        const _: () = assert!(
            MESSAGE_ANIMATION_MAX_BYTES * 4 <= MESSAGE_IMAGE_CACHE_BYTES,
            "a single animated attachment that may occupy the whole cache budget forces \
             evict_to_budget to drop every entry outside the working set the moment it is on \
             screen, so scrolling back one row re-fetches images that were visible seconds ago"
        );
        const _: () = assert!(
            MESSAGE_ANIMATION_MAX_BYTES < MESSAGE_ENTRY_MAX_BYTES,
            "the decode budget must stay under the per-entry rejection cap, or a long animation \
             is dropped and negatively cached instead of being decimated"
        );
    }

    #[test]
    fn a_long_message_animation_is_decimated_rather_than_frozen() {
        use image::codecs::gif::{GifEncoder, Repeat};
        const SOURCE_FRAMES: usize = 16;
        const SIDE: u32 = 8;
        let frame_bytes = u64::from(SIDE) * u64::from(SIDE) * 4;
        let budget = frame_bytes * 4;

        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).expect("GIF repeat");
            for i in 0..SOURCE_FRAMES {
                let shade = (i * 16) as u8;
                let buffer =
                    image::RgbaImage::from_pixel(SIDE, SIDE, image::Rgba([shade, 0, 0, 255]));
                let frame = image::Frame::from_parts(
                    buffer,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(50, 1),
                );
                encoder.encode_frame(frame).expect("GIF frame");
            }
        }

        let image = decode_message_image(&bytes, MESSAGE_ANIMATION_MAX_PX, 1024, budget)
            .expect("animated attachment");

        assert!(
            image.frame_count() > 1,
            "a long animation must keep animating at a lower frame rate; bailing to a single \
             static frame is what the budget guard used to do"
        );
        assert!(
            image_bytes(&image) <= budget,
            "decimation must bring the decoded animation inside its byte budget"
        );
        assert_eq!(
            std::time::Duration::from(image.delay(0)),
            std::time::Duration::from_millis(200),
            "dropped frames must have their delay folded into the survivors, so the animation \
             still plays over its original duration"
        );
    }
}
