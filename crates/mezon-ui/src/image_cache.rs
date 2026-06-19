use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt;
use gpui::{
    App, Asset, AssetLogger, Context, ImageAssetLoader, ImageCache, ImageCacheError,
    ImageCacheItem, RenderImage, Resource, Window, hash,
};

pub const MESSAGE_IMAGE_CACHE_CAPACITY: usize = 96;

pub struct LruImageCache {
    max_items: usize,
    usages: Vec<u64>,
    cache: HashMap<u64, ImageCacheItem>,
}

impl LruImageCache {
    pub fn new(max_items: usize, cx: &mut Context<Self>) -> Self {
        cx.on_release(|cache, cx| {
            for (_, mut item) in std::mem::take(&mut cache.cache) {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();

        Self {
            max_items,
            usages: Vec::with_capacity(max_items),
            cache: HashMap::with_capacity(max_items),
        }
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        for (_, mut item) in std::mem::take(&mut self.cache) {
            if let Some(Ok(image)) = item.get() {
                cx.drop_image(image, Some(window));
            }
        }
        self.usages.clear();
    }

    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let hash = hash(resource);

        if let Some(item) = self.cache.get_mut(&hash) {
            if let Some(pos) = self.usages.iter().position(|item| *item == hash) {
                self.usages.remove(pos);
                self.usages.insert(0, hash);
            }
            return item.get();
        }

        let fut = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
        let task = cx.background_executor().spawn(fut).shared();
        if self.usages.len() == self.max_items {
            let oldest = self.usages.pop().expect("usages in sync with cache");
            let mut image = self
                .cache
                .remove(&oldest)
                .expect("usages in sync with cache");
            if let Some(Ok(image)) = image.get() {
                cx.drop_image(image, Some(window));
            }
        }
        self.cache
            .insert(hash, ImageCacheItem::Loading(task.clone()));
        self.usages.insert(0, hash);

        let entity = window.current_view();
        window
            .spawn(cx, async move |cx| {
                _ = task.await;
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
