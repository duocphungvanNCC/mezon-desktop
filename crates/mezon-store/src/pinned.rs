use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, Global, SharedString, Subscription};
use mezon_client::AppApi;
use mezon_client::transport::ApiPinMessage;

use crate::AppConfig;
use crate::channel::{ChannelEvent, ChannelList};

/// A pinned message for the active channel, ready for the UI.
#[derive(Debug, Clone)]
pub struct PinnedMessage {
    pub id: String,
    pub message_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub avatar_url: String,
    pub avatar_proxied: SharedString,
    pub content: String,
    pub create_time: i64,
}

pub struct PinnedMessagesStore {
    channel_id: Option<String>,
    clan_id: Option<String>,
    messages: Vec<PinnedMessage>,
    loaded_channel: Option<String>,
    loading: bool,
    api: Arc<AppApi>,
    _channel_sub: Subscription,
}

struct GlobalPinnedMessagesStore(Entity<PinnedMessagesStore>);
impl Global for GlobalPinnedMessagesStore {}

impl PinnedMessagesStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalPinnedMessagesStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalPinnedMessagesStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalPinnedMessagesStore>()
            .map(|g| g.0.clone())
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        let channel_sub = cx.subscribe(&ChannelList::global(cx), |this, _list, event, cx| {
            if let ChannelEvent::ActiveChannelChanged(channel_id) = event {
                this.on_active_channel_changed(channel_id.clone(), cx);
            }
        });
        let mut store = Self {
            channel_id: None,
            clan_id: None,
            messages: Vec::new(),
            loaded_channel: None,
            loading: false,
            api,
            _channel_sub: channel_sub,
        };
        if let Some(channel) = ChannelList::global(cx).read(cx).active_channel() {
            store.channel_id = Some(channel.id.clone());
            store.clan_id = Some(channel.clan_id.clone());
        }
        store
    }

    pub fn pinned(&self) -> &[PinnedMessage] {
        &self.messages
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    fn on_active_channel_changed(&mut self, channel_id: Option<String>, cx: &mut Context<Self>) {
        match channel_id {
            None => {
                self.channel_id = None;
                self.clan_id = None;
            }
            Some(id) => {
                let clan_id = ChannelList::global(cx)
                    .read(cx)
                    .find_channel(&id)
                    .map(|c| c.clan_id.clone())
                    .unwrap_or_else(|| "0".to_string());
                self.channel_id = Some(id);
                self.clan_id = Some(clan_id);
            }
        }
        self.messages.clear();
        self.loaded_channel = None;
        cx.notify();
    }

    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.channel_id.clone() else {
            return;
        };
        if self.loading || self.loaded_channel.as_deref() == Some(channel_id.as_str()) {
            return;
        }
        self.fetch(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loaded_channel = None;
        self.fetch(cx);
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.channel_id.clone() else {
            return;
        };
        let Some(clan_id) = self.clan_id.clone() else {
            return;
        };
        if self.loading {
            return;
        }
        self.loading = true;
        cx.notify();

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.get_pin_messages_list(&channel_id, &clan_id).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                if this.channel_id.as_deref() != Some(channel_id.as_str()) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(list) => {
                        let cfg = AppConfig::try_global(cx);
                        this.messages = list.into_iter().map(|m| pinned_from_api(m, cfg)).collect();
                        this.loaded_channel = Some(channel_id);
                    }
                    Err(e) => tracing::error!("get_pin_messages_list failed: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn unpin(&mut self, pin_id: &str, message_id: &str, cx: &mut Context<Self>) {
        let Some(channel_id) = self.channel_id.clone() else {
            return;
        };
        let Some(clan_id) = self.clan_id.clone() else {
            return;
        };
        self.messages.retain(|m| m.id != pin_id);
        cx.notify();

        let api = self.api.clone();
        let pin_id = pin_id.to_string();
        let message_id = message_id.to_string();
        cx.spawn(async move |this, cx| {
            let result = api
                .delete_pin_message(&pin_id, &message_id, &channel_id, &clan_id)
                .await;
            if let Err(e) = result {
                tracing::error!("delete_pin_message failed: {e}");
                let _ = this.update(cx, |this, cx| this.refresh(cx));
            }
        })
        .detach();
    }
}

fn pinned_from_api(m: ApiPinMessage, cfg: Option<&AppConfig>) -> PinnedMessage {
    let avatar_proxied = cfg
        .map(|c| c.avatar_proxy(&m.avatar))
        .unwrap_or_else(|| m.avatar.clone());
    PinnedMessage {
        id: m.id,
        message_id: m.message_id,
        sender_id: m.sender_id,
        sender_name: m.sender_name,
        avatar_url: m.avatar,
        avatar_proxied: avatar_proxied.into(),
        content: m.content,
        create_time: m.create_time,
    }
}
