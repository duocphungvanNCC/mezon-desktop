use std::sync::Arc;

use futures::FutureExt;
use futures::future::{AbortHandle, Abortable};
use gpui::{
    App, Asset, AssetLogger, Context, ImageAssetLoader, ImageCache, ImageCacheError,
    ImageCacheItem, RenderImage, Resource, Window, hash,
};
use indexmap::IndexMap;

pub const MESSAGE_IMAGE_CACHE_CAPACITY: usize = 96;

struct CacheEntry {
    item: ImageCacheItem,
    abort: AbortHandle,
}

pub struct LruImageCache {
    max_items: usize,
    cache: IndexMap<u64, CacheEntry>,
}

impl LruImageCache {
    pub fn new(max_items: usize, cx: &mut Context<Self>) -> Self {
        cx.on_release(|cache, cx| {
            for (_, mut entry) in std::mem::take(&mut cache.cache) {
                entry.abort.abort();
                if let Some(Ok(image)) = entry.item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();

        Self {
            max_items,
            cache: IndexMap::with_capacity(max_items),
        }
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        for (_, mut entry) in std::mem::take(&mut self.cache) {
            entry.abort.abort();
            if let Some(Ok(image)) = entry.item.get() {
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
        let hash = hash(resource);

        if let Some(entry) = self.cache.shift_remove(&hash) {
            self.cache.insert(hash, entry);
            return self.cache.get_mut(&hash).and_then(|e| e.item.get());
        }

        if self.cache.len() == self.max_items
            && let Some((_, mut evicted)) = self.cache.shift_remove_index(0)
        {
            evicted.abort.abort();
            if let Some(Ok(image)) = evicted.item.get() {
                cx.drop_image(image, Some(window));
            }
        }

        let fut = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
        let task = cx.background_executor().spawn(fut).shared();
        let (abort_handle, abort_reg) = AbortHandle::new_pair();

        self.cache.insert(
            hash,
            CacheEntry {
                item: ImageCacheItem::Loading(task.clone()),
                abort: abort_handle,
            },
        );

        let entity = window.current_view();
        let notify_task = task.clone();
        window
            .spawn(cx, async move |cx| {
                let _ = Abortable::new(notify_task, abort_reg).await;
                cx.on_next_frame(move |_, cx| {
                    cx.notify(entity);
                });
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
