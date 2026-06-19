use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};
use mezon_client::transport::ApiMessage;
use mezon_client::{AppApi, ConnectionStatus, MezonTransport, RealtimeEvent};

use crate::KeyedCache;
use crate::channel::{
    Channel, ChannelEvent, ChannelList, Message, MessageAttachment, recompute_message_grouping,
};
use crate::realtime::{RealtimeDispatch, RealtimeKind};

const MESSAGE_PAGE_LIMIT: u32 = 50;
const DIRECTION_BEFORE: i32 = 3;
const CHANNEL_TYPE_CHANNEL: i32 = 1;
const MAX_MESSAGES_PER_CHANNEL: usize = 2_000;
const MAX_CACHED_CHANNELS: usize = 30;

#[derive(Debug, Clone)]
pub enum MessagesEvent {
    Reset { count: usize },
    Appended,
    OlderPrepended { count: usize },
}

struct ChannelMessages {
    messages: Vec<Message>,
    has_more: bool,
}

pub struct MessagesStore {
    cache: KeyedCache<String, ChannelMessages>,
    active_channel_id: Option<String>,
    active_clan_id: Option<String>,
    is_public: bool,
    loading: bool,
    loading_more: bool,
    api: Arc<AppApi>,
    _channel_sub: Subscription,
    _conn_watch: Task<()>,
}

struct GlobalMessagesStore(Entity<MessagesStore>);
impl Global for GlobalMessagesStore {}

impl EventEmitter<MessagesEvent> for MessagesStore {}

impl MessagesStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalMessagesStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalMessagesStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalMessagesStore>().map(|g| g.0.clone())
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        Self::register_realtime(cx);

        let channel_sub = cx.subscribe(&ChannelList::global(cx), |this, _channel, event, cx| {
            if let ChannelEvent::ActiveChannelChanged(channel_id) = event {
                this.on_active_channel_changed(channel_id.clone(), cx);
            }
        });

        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);

        Self {
            cache: KeyedCache::new(Some(MAX_CACHED_CHANNELS)),
            active_channel_id: None,
            active_clan_id: None,
            is_public: true,
            loading: false,
            loading_more: false,
            api,
            _channel_sub: channel_sub,
            _conn_watch: conn_watch,
        }
    }

    /// Register realtime handlers with the central dispatcher (cf. `add_message_handler`).
    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(RealtimeKind::ChannelMessage, &entity, |this, event, cx| {
                this.handle_event(event, cx)
            });
            dispatch.on_lagged(&entity, |this, cx| this.resync(cx));
        });
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
                    if this.update(cx, |this, cx| this.resync(cx)).is_err() {
                        break;
                    }
                } else if !connected {
                    was_connected = false;
                }
            }
        })
    }

    pub fn messages(&self) -> &[Message] {
        self.active_channel_id
            .as_ref()
            .and_then(|id| self.cache.get(id))
            .map(|c| c.messages.as_slice())
            .unwrap_or(&[])
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more || self.loading {
            return;
        }
        let Some(channel_id) = self.active_channel_id.clone() else {
            return;
        };
        let Some(clan_id) = self.active_clan_id.clone() else {
            return;
        };
        let Some(channel) = self.cache.get(&channel_id) else {
            return;
        };
        if !channel.has_more {
            return;
        }
        let Some(oldest_id) = channel
            .messages
            .first()
            .map(|m| m.id.clone())
            .filter(|id| !id.starts_with("temp-"))
        else {
            return;
        };

        self.loading_more = true;
        cx.notify();

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_messages(
                    &clan_id,
                    &channel_id,
                    &oldest_id,
                    DIRECTION_BEFORE,
                    MESSAGE_PAGE_LIMIT,
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading_more = false;
                let msgs = match result {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        tracing::error!("Failed to load more messages for {channel_id}: {e}");
                        cx.notify();
                        return;
                    }
                };
                let Some(channel) = this.cache.get_mut(&channel_id) else {
                    return;
                };
                let existing: std::collections::HashSet<String> =
                    channel.messages.iter().map(|m| m.id.clone()).collect();
                let mut older: Vec<Message> = msgs
                    .into_iter()
                    .filter(|m| !existing.contains(&m.message_id))
                    .map(message_from_api)
                    .collect();
                if older.is_empty() {
                    channel.has_more = false;
                    cx.notify();
                    return;
                }
                let prepended = older.len();
                older.append(&mut channel.messages);
                older.sort_by_key(|m| m.create_time);
                trim_messages(&mut older);
                channel.messages = older;
                recompute_message_grouping(&mut channel.messages);
                if this.active_channel_id.as_deref() == Some(channel_id.as_str()) {
                    cx.emit(MessagesEvent::OlderPrepended { count: prepended });
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn send_message(
        &mut self,
        content: String,
        sender_id: String,
        sender_name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.active_channel_id.clone() else {
            return;
        };
        let Some(clan_id) = self.active_clan_id.clone() else {
            return;
        };
        let is_public = self.is_public;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        let temp_id = format!("temp-{now}");

        let Some(channel) = self.cache.get_mut(&channel_id) else {
            return;
        };
        channel.messages.push(Message::new(
            temp_id.clone(),
            content.clone(),
            sender_id,
            sender_name,
            now,
        ));
        trim_messages(&mut channel.messages);
        recompute_message_grouping(&mut channel.messages);
        cx.emit(MessagesEvent::Appended);
        cx.notify();

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            match api
                .send_channel_message(&clan_id, &channel_id, &content, is_public)
                .await
            {
                Ok(sent) => {
                    let _ = this.update(cx, |this, cx| {
                        this.reconcile_temp(&channel_id, &temp_id, message_from_api(sent));
                        if this.active_channel_id.as_deref() == Some(channel_id.as_str()) {
                            cx.emit(MessagesEvent::Appended);
                            cx.notify();
                        }
                    });
                }
                Err(e) => tracing::error!("send_channel_message failed: {e}"),
            }
        })
        .detach();
    }

    fn on_active_channel_changed(&mut self, channel_id: Option<String>, cx: &mut Context<Self>) {
        let Some(channel_id) = channel_id else {
            self.active_channel_id = None;
            self.active_clan_id = None;
            self.loading = false;
            self.loading_more = false;
            cx.emit(MessagesEvent::Reset { count: 0 });
            cx.notify();
            return;
        };

        if self.active_channel_id.as_deref() == Some(channel_id.as_str()) {
            return;
        }

        let Some(channel) = ChannelList::global(cx)
            .read(cx)
            .find_channel(&channel_id)
            .cloned()
        else {
            return;
        };

        self.active_channel_id = Some(channel_id.clone());
        self.active_clan_id = Some(channel.clan_id.clone());
        self.is_public = !channel.private;
        self.loading_more = false;

        self.spawn_join(&channel, cx);

        let fresh = self.cache.is_fresh(&channel_id, crate::CACHE_TTL);
        if fresh {
            self.cache.touch(&channel_id);
            self.loading = false;
            let count = self.messages().len();
            cx.emit(MessagesEvent::Reset { count });
            cx.notify();
            return;
        }

        self.loading = true;
        cx.emit(MessagesEvent::Reset { count: 0 });
        cx.notify();
        self.spawn_initial_fetch(&channel, cx);
    }

    fn spawn_join(&self, channel: &Channel, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let clan_id = channel.clan_id.clone();
        let ch_id = channel.id.clone();
        let is_public = !channel.private;
        cx.spawn(async move |_this, _cx| {
            if let Err(e) = api
                .join_chat(&clan_id, &ch_id, CHANNEL_TYPE_CHANNEL, is_public)
                .await
            {
                tracing::warn!("join_chat failed: {e}");
            }
        })
        .detach();
    }

    fn spawn_initial_fetch(&self, channel: &Channel, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let clan_id = channel.clan_id.clone();
        let ch_id = channel.id.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_messages(&clan_id, &ch_id, "", 0, MESSAGE_PAGE_LIMIT)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.active_channel_id.as_deref() != Some(ch_id.as_str()) {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(msgs) => {
                        let messages = prepare_messages(msgs);
                        let count = messages.len();
                        this.set_channel(&ch_id, messages);
                        cx.emit(MessagesEvent::Reset { count });
                    }
                    Err(e) => {
                        tracing::error!("Failed to fetch messages for {ch_id}: {e}");
                        cx.emit(MessagesEvent::Reset { count: 0 });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::ChannelMessage(m) = event else {
            return;
        };
        let channel_id = m.channel_id.to_string();
        let is_active = self.active_channel_id.as_deref() == Some(channel_id.as_str());
        let Some(channel) = self.cache.get_mut(&channel_id) else {
            return;
        };
        let msg = message_from_api(MezonTransport::message_from_proto(m.clone()));
        if channel.messages.iter().any(|x| x.id == msg.id) {
            return;
        }
        if let Some(slot) = channel.messages.iter_mut().find(|x| {
            x.id.starts_with("temp-") && x.sender_id == msg.sender_id && x.content == msg.content
        }) {
            *slot = msg;
        } else {
            channel.messages.push(msg);
        }
        channel.messages.sort_by_key(|m| m.create_time);
        trim_messages(&mut channel.messages);
        recompute_message_grouping(&mut channel.messages);
        if is_active {
            cx.emit(MessagesEvent::Appended);
            cx.notify();
        }
    }

    fn reconcile_temp(&mut self, channel_id: &str, temp_id: &str, confirmed: Message) {
        let Some(channel) = self.cache.get_mut(channel_id) else {
            return;
        };
        if let Some(slot) = channel.messages.iter_mut().find(|m| {
            m.id == temp_id || (m.id.starts_with("temp-") && m.content == confirmed.content)
        }) {
            *slot = confirmed;
        } else if !channel.messages.iter().any(|m| m.id == confirmed.id) {
            channel.messages.push(confirmed);
            channel.messages.sort_by_key(|m| m.create_time);
            trim_messages(&mut channel.messages);
            recompute_message_grouping(&mut channel.messages);
        }
    }

    fn resync(&mut self, cx: &mut Context<Self>) {
        tracing::info!("MessagesStore resync — clearing message cache");
        self.cache.clear();
        self.refetch_current_messages(cx);
    }

    /// Force a refetch of the open channel ignoring the cache (cf. React `noCache: true`).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refetch_current_messages(cx);
    }

    fn refetch_current_messages(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.active_channel_id.clone() else {
            return;
        };
        let Some(clan_id) = self.active_clan_id.clone() else {
            return;
        };

        self.loading = true;
        self.loading_more = false;
        cx.notify();

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_messages(&clan_id, &channel_id, "", 0, MESSAGE_PAGE_LIMIT)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.active_channel_id.as_deref() != Some(channel_id.as_str()) {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(msgs) => {
                        let messages = prepare_messages(msgs);
                        let count = messages.len();
                        this.set_channel(&channel_id, messages);
                        cx.emit(MessagesEvent::Reset { count });
                    }
                    Err(e) => {
                        tracing::error!("Failed to refetch messages for {channel_id}: {e}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_channel(&mut self, channel_id: &str, messages: Vec<Message>) {
        let active = self.active_channel_id.clone();
        self.cache.insert(
            channel_id.to_string(),
            ChannelMessages {
                messages,
                has_more: true,
            },
            active.as_ref(),
        );
    }
}

fn prepare_messages(msgs: Vec<ApiMessage>) -> Vec<Message> {
    let mut messages: Vec<Message> = msgs.into_iter().map(message_from_api).collect();
    messages.sort_by_key(|m| m.create_time);
    trim_messages(&mut messages);
    recompute_message_grouping(&mut messages);
    messages
}

fn trim_messages(messages: &mut Vec<Message>) {
    if messages.len() <= MAX_MESSAGES_PER_CHANNEL {
        return;
    }
    let drop = messages.len() - MAX_MESSAGES_PER_CHANNEL;
    messages.drain(0..drop);
}

fn message_from_api(m: ApiMessage) -> Message {
    Message::new(
        m.message_id,
        m.content,
        m.sender_id,
        m.sender_name,
        m.create_time,
    )
    .with_avatar(m.avatar)
    .with_attachments(
        m.attachments
            .into_iter()
            .map(MessageAttachment::from_api)
            .collect(),
    )
}

impl MessageAttachment {
    pub(crate) fn from_api(a: mezon_client::transport::ApiAttachment) -> Self {
        Self {
            url: a.url,
            filename: a.filename,
            filetype: a.filetype,
            width: a.width.max(0) as u32,
            height: a.height.max(0) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_from_api_maps_fields() {
        let m = message_from_api(ApiMessage {
            message_id: "1".into(),
            content: "hi".into(),
            sender_id: "u1".into(),
            sender_name: "Alice".into(),
            avatar: "av.png".into(),
            create_time: 100,
            attachments: vec![],
        });
        assert_eq!(m.id, "1");
        assert_eq!(m.content, "hi");
        assert_eq!(m.sender_name, "Alice");
        assert_eq!(m.avatar_url, "av.png");
    }

    #[test]
    fn trim_messages_drops_oldest() {
        let mut msgs: Vec<Message> = (0..MAX_MESSAGES_PER_CHANNEL + 5)
            .map(|i| Message::new(i.to_string(), format!("m{i}"), "u", "User", i as i64))
            .collect();
        trim_messages(&mut msgs);
        assert_eq!(msgs.len(), MAX_MESSAGES_PER_CHANNEL);
        assert_eq!(msgs.first().unwrap().id, "5");
        assert_eq!(msgs.last().unwrap().id, "2004");
    }
}
