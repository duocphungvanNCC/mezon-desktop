use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use mezon_client::{AppApi, ConnectionStatus};
use mezon_proto::api;

use crate::AppConfig;
use crate::KeyedCache;
use crate::clan::upload_image_to_cdn;
use crate::ids::{ChannelId, ClanId, UserId};

const MAX_CACHED_CLANS: usize = 16;
pub const WEBHOOK_NAME_MAX_LENGTH: usize = 64;
pub const MAX_WEBHOOK_AVATAR_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelWebhook {
    pub id: String,
    pub webhook_name: String,
    pub channel_id: ChannelId,
    pub clan_id: ClanId,
    pub url: String,
    pub avatar: String,
    pub creator_id: UserId,
    pub create_time_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClanWebhook {
    pub id: String,
    pub webhook_name: String,
    pub clan_id: ClanId,
    pub url: String,
    pub avatar: String,
    pub creator_id: UserId,
    pub create_time_seconds: i64,
}

#[derive(Debug, Clone)]
pub enum WebhookEvent {
    ChannelWebhooksChanged { clan_id: ClanId },
    ClanWebhooksChanged { clan_id: ClanId },
}

#[derive(Debug, Default)]
struct ClanChannelWebhooks {
    webhooks: Vec<ChannelWebhook>,
}

#[derive(Debug, Default)]
struct ClanClanWebhooks {
    webhooks: Vec<ClanWebhook>,
}

pub struct WebhookStore {
    channel_cache: KeyedCache<ClanId, ClanChannelWebhooks>,
    clan_cache: KeyedCache<ClanId, ClanClanWebhooks>,
    channel_loading: HashSet<ClanId>,
    clan_loading: HashSet<ClanId>,
    api: Arc<AppApi>,
    _conn_watch: Task<()>,
}

struct GlobalWebhookStore(Entity<WebhookStore>);
impl Global for GlobalWebhookStore {}

impl EventEmitter<WebhookEvent> for WebhookStore {}

impl WebhookStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalWebhookStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalWebhookStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalWebhookStore>().map(|g| g.0.clone())
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.channel_cache.clear();
        self.clan_cache.clear();
        self.channel_loading.clear();
        self.clan_loading.clear();
        cx.notify();
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);
        Self {
            channel_cache: KeyedCache::new(Some(MAX_CACHED_CLANS)),
            clan_cache: KeyedCache::new(Some(MAX_CACHED_CLANS)),
            channel_loading: HashSet::new(),
            clan_loading: HashSet::new(),
            api,
            _conn_watch: conn_watch,
        }
    }

    fn spawn_connection_watch(api: Arc<AppApi>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut status_rx = api.status();
            let mut was_connected = false;
            loop {
                if status_rx.changed().await.is_err() {
                    break;
                }
                let connected = *status_rx.borrow() == ConnectionStatus::Connected;
                if connected && !was_connected {
                    was_connected = true;
                    if this.update(cx, |this, _| this.invalidate()).is_err() {
                        break;
                    }
                } else if !connected {
                    was_connected = false;
                }
            }
        })
    }

    fn invalidate(&mut self) {
        self.channel_cache.mark_all_stale();
        self.clan_cache.mark_all_stale();
    }

    pub fn ensure_channel_webhooks_loaded(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        self.channel_cache.touch(&clan_id);
        if !self.channel_cache.is_fresh(&clan_id, crate::CACHE_TTL) {
            self.fetch_channel_webhooks(clan_id, cx);
        }
    }

    pub fn ensure_clan_webhooks_loaded(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        self.clan_cache.touch(&clan_id);
        if !self.clan_cache.is_fresh(&clan_id, crate::CACHE_TTL) {
            self.fetch_clan_webhooks(clan_id, cx);
        }
    }

    pub fn refresh_channel_webhooks(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        self.fetch_channel_webhooks(clan_id, cx);
    }

    pub fn refresh_clan_webhooks(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        self.fetch_clan_webhooks(clan_id, cx);
    }

    pub fn channel_webhooks_loading(&self, clan_id: ClanId) -> bool {
        self.channel_loading.contains(&clan_id)
    }

    pub fn clan_webhooks_loading(&self, clan_id: ClanId) -> bool {
        self.clan_loading.contains(&clan_id)
    }

    pub fn channel_webhooks_for_clan(&self, clan_id: ClanId) -> &[ChannelWebhook] {
        self.channel_cache
            .get(&clan_id)
            .map(|entry| entry.webhooks.as_slice())
            .unwrap_or(&[])
    }

    pub fn channel_webhooks_for_channel(
        &self,
        clan_id: ClanId,
        channel_id: ChannelId,
    ) -> Vec<&ChannelWebhook> {
        self.channel_webhooks_for_clan(clan_id)
            .iter()
            .filter(|webhook| webhook.channel_id == channel_id)
            .collect()
    }

    pub fn clan_webhooks_for_clan(&self, clan_id: ClanId) -> &[ClanWebhook] {
        self.clan_cache
            .get(&clan_id)
            .map(|entry| entry.webhooks.as_slice())
            .unwrap_or(&[])
    }

    fn fetch_channel_webhooks(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        if !self.channel_loading.insert(clan_id) {
            return;
        }
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_webhooks_by_channel(0, clan_id.get())
                .await
                .map(|webhooks| {
                    webhooks
                        .into_iter()
                        .filter_map(|proto| channel_webhook_from_proto(proto, clan_id))
                        .collect()
                });
            let _ = this.update(cx, |this, cx| {
                this.channel_loading.remove(&clan_id);
                match result {
                    Ok(webhooks) => {
                        this.channel_cache.insert(
                            clan_id,
                            ClanChannelWebhooks { webhooks },
                            None,
                        );
                        cx.emit(WebhookEvent::ChannelWebhooksChanged { clan_id });
                        cx.notify();
                    }
                    Err(err) => {
                        tracing::error!("list_webhooks_by_channel failed for {clan_id}: {err}");
                    }
                }
            });
        })
        .detach();
    }

    fn fetch_clan_webhooks(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        if !self.clan_loading.insert(clan_id) {
            return;
        }
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_clan_webhooks(clan_id.get())
                .await
                .map(|webhooks| webhooks.into_iter().filter_map(clan_webhook_from_proto).collect());
            let _ = this.update(cx, |this, cx| {
                this.clan_loading.remove(&clan_id);
                match result {
                    Ok(webhooks) => {
                        this.clan_cache
                            .insert(clan_id, ClanClanWebhooks { webhooks }, None);
                        cx.emit(WebhookEvent::ClanWebhooksChanged { clan_id });
                        cx.notify();
                    }
                    Err(err) => {
                        tracing::error!("list_clan_webhooks failed for {clan_id}: {err}");
                    }
                }
            });
        })
        .detach();
    }

    pub fn create_channel_webhook(
        &mut self,
        clan_id: ClanId,
        channel_id: ChannelId,
        webhook_name: String,
        avatar: String,
        cx: &mut Context<Self>,
    ) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let request = api::WebhookCreateRequest {
                webhook_name,
                channel_id: channel_id.get(),
                clan_id: clan_id.get(),
                avatar,
            };
            let result = api.generate_webhook(request).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.refresh_channel_webhooks(clan_id, cx),
                    Err(err) => tracing::error!("generate_webhook failed: {err}"),
                }
            });
        })
        .detach();
    }

    pub fn update_channel_webhook(
        &mut self,
        webhook: &ChannelWebhook,
        webhook_name: String,
        avatar: String,
        channel_id_update: Option<ChannelId>,
        cx: &mut Context<Self>,
    ) {
        let id = webhook.id.parse::<i64>().unwrap_or(0);
        if id == 0 {
            return;
        }
        let api = self.api.clone();
        let clan_id = webhook.clan_id;
        let channel_id = webhook.channel_id;
        cx.spawn(async move |this, cx| {
            let request = api::WebhookUpdateRequestById {
                id,
                webhook_name,
                avatar,
                channel_id: channel_id.get(),
                channel_id_update: channel_id_update
                    .map(|id| id.get())
                    .unwrap_or_else(|| channel_id.get()),
                clan_id: clan_id.get(),
            };
            let result = api.update_webhook(request).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.refresh_channel_webhooks(clan_id, cx),
                    Err(err) => tracing::error!("update_webhook failed: {err}"),
                }
            });
        })
        .detach();
    }

    pub fn delete_channel_webhook(&mut self, webhook: &ChannelWebhook, cx: &mut Context<Self>) {
        let id = webhook.id.parse::<i64>().unwrap_or(0);
        if id == 0 {
            return;
        }
        let api = self.api.clone();
        let clan_id = webhook.clan_id;
        let channel_id = webhook.channel_id;
        cx.spawn(async move |this, cx| {
            let request = api::WebhookDeleteRequestById {
                id,
                clan_id: clan_id.get(),
                channel_id: channel_id.get(),
            };
            let result = api.delete_webhook(request).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.refresh_channel_webhooks(clan_id, cx),
                    Err(err) => tracing::error!("delete_webhook failed: {err}"),
                }
            });
        })
        .detach();
    }

    pub fn create_clan_webhook(
        &mut self,
        clan_id: ClanId,
        webhook_name: String,
        avatar: String,
        cx: &mut Context<Self>,
    ) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let request = api::GenerateClanWebhookRequest {
                clan_id: clan_id.get(),
                webhook_name,
                avatar,
            };
            let result = api.generate_clan_webhook(request).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.refresh_clan_webhooks(clan_id, cx),
                    Err(err) => tracing::error!("generate_clan_webhook failed: {err}"),
                }
            });
        })
        .detach();
    }

    pub fn update_clan_webhook(
        &mut self,
        webhook: &ClanWebhook,
        webhook_name: String,
        avatar: String,
        reset_token: bool,
        cx: &mut Context<Self>,
    ) {
        let id = webhook.id.parse::<i64>().unwrap_or(0);
        if id == 0 {
            return;
        }
        let api = self.api.clone();
        let clan_id = webhook.clan_id;
        cx.spawn(async move |this, cx| {
            let request = api::UpdateClanWebhookRequest {
                id,
                clan_id: clan_id.get(),
                webhook_name,
                avatar,
                reset_token,
            };
            let result = api.update_clan_webhook(request).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.refresh_clan_webhooks(clan_id, cx),
                    Err(err) => tracing::error!("update_clan_webhook failed: {err}"),
                }
            });
        })
        .detach();
    }

    pub fn upload_webhook_avatar(
        &self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<String, String>> {
        let api = self.api.clone();
        let path = path.to_path_buf();
        let base_img_url = AppConfig::global(cx).base_img_url.clone();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .spawn(async move {
                    upload_image_to_cdn(&api, &base_img_url, &path, MAX_WEBHOOK_AVATAR_BYTES)
                        .await
                })
                .await
        })
    }

    pub fn delete_clan_webhook(&mut self, webhook: &ClanWebhook, cx: &mut Context<Self>) {
        let id = webhook.id.parse::<i64>().unwrap_or(0);
        if id == 0 {
            return;
        }
        let api = self.api.clone();
        let clan_id = webhook.clan_id;
        cx.spawn(async move |this, cx| {
            let result = api.delete_clan_webhook(id, clan_id.get()).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.refresh_clan_webhooks(clan_id, cx),
                    Err(err) => tracing::error!("delete_clan_webhook failed: {err}"),
                }
            });
        })
        .detach();
    }
}

fn channel_webhook_from_proto(proto: api::Webhook, clan_id: ClanId) -> Option<ChannelWebhook> {
    if proto.id == 0 {
        return None;
    }
    Some(ChannelWebhook {
        id: proto.id.to_string(),
        webhook_name: proto.webhook_name,
        channel_id: ChannelId(proto.channel_id),
        clan_id: if proto.clan_id != 0 {
            ClanId(proto.clan_id)
        } else {
            clan_id
        },
        url: proto.url,
        avatar: proto.avatar,
        creator_id: UserId(proto.creator_id),
        create_time_seconds: webhook_create_time_seconds(&proto.create_time),
    })
}

fn clan_webhook_from_proto(proto: api::ClanWebhook) -> Option<ClanWebhook> {
    if proto.id == 0 {
        return None;
    }
    Some(ClanWebhook {
        id: proto.id.to_string(),
        webhook_name: proto.webhook_name,
        clan_id: ClanId(proto.clan_id),
        url: proto.url,
        avatar: proto.avatar,
        creator_id: UserId(proto.creator_id),
        create_time_seconds: webhook_create_time_seconds(&proto.create_time),
    })
}

fn webhook_create_time_seconds(create_time: &str) -> i64 {
    if create_time.is_empty() {
        return 0;
    }
    chrono::DateTime::parse_from_rfc3339(create_time)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}
