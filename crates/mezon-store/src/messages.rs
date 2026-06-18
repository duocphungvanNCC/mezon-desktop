use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};
use mezon_client::transport::ApiMessage;
use mezon_client::{AppApi, MezonTransport, RealtimeEvent};

use crate::channel::{
    ChannelEvent, ChannelList, Message, MessageAttachment, recompute_message_grouping,
};

const MESSAGE_PAGE_LIMIT: u32 = 50;
const DIRECTION_BEFORE: i32 = 3;
const CHANNEL_TYPE_CHANNEL: i32 = 1;
const MAX_MESSAGES_PER_CHANNEL: usize = 2_000;

#[derive(Debug, Clone)]
pub enum MessagesEvent {
    Reset { count: usize },
    Appended,
    OlderPrepended { count: usize },
}

pub struct MessagesStore {
    pub messages: Vec<Message>,
    loaded_channel_id: Option<String>,
    loaded_clan_id: Option<String>,
    is_public: bool,
    pub has_more: bool,
    pub loading: bool,
    pub loading_more: bool,
    api: Arc<AppApi>,
    _realtime: Task<()>,
    _channel_sub: Subscription,
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
        let realtime = {
            let api = api.clone();
            cx.spawn(async move |this, cx| {
                let mut rx = api.subscribe();
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if this
                                .update(cx, |this, cx| this.handle_event(event, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if this
                                .update(cx, |this, cx| this.on_realtime_lagged(cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };

        let channel_sub = cx.subscribe(&ChannelList::global(cx), |this, _channel, event, cx| {
            if let ChannelEvent::ActiveChannelChanged(channel_id) = event {
                this.on_active_channel_changed(channel_id.clone(), cx);
            }
        });

        Self {
            messages: Vec::new(),
            loaded_channel_id: None,
            loaded_clan_id: None,
            is_public: true,
            has_more: true,
            loading: false,
            loading_more: false,
            api,
            _realtime: realtime,
            _channel_sub: channel_sub,
        }
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        if !self.has_more || self.loading_more || self.loading {
            return;
        }
        let Some(channel_id) = self.loaded_channel_id.clone() else {
            return;
        };
        let Some(clan_id) = self.loaded_clan_id.clone() else {
            return;
        };
        let Some(oldest_id) = self
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
                if this.loaded_channel_id.as_deref() != Some(channel_id.as_str()) {
                    return;
                }
                let existing: std::collections::HashSet<String> =
                    this.messages.iter().map(|m| m.id.clone()).collect();
                let mut older: Vec<Message> = msgs
                    .into_iter()
                    .filter(|m| !existing.contains(&m.message_id))
                    .map(message_from_api)
                    .collect();
                if older.is_empty() {
                    this.has_more = false;
                    cx.notify();
                    return;
                }
                let prepended = older.len();
                older.append(&mut this.messages);
                older.sort_by_key(|m| m.create_time);
                trim_messages(&mut older);
                this.messages = older;
                recompute_message_grouping(&mut this.messages);
                cx.emit(MessagesEvent::OlderPrepended { count: prepended });
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
        let Some(channel_id) = self.loaded_channel_id.clone() else {
            return;
        };
        let Some(clan_id) = self.loaded_clan_id.clone() else {
            return;
        };
        let is_public = self.is_public;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        let temp_id = format!("temp-{now}");
        self.messages.push(Message::new(
            temp_id.clone(),
            content.clone(),
            sender_id,
            sender_name,
            now,
        ));
        trim_messages(&mut self.messages);
        recompute_message_grouping(&mut self.messages);
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
                        if this.loaded_channel_id.as_deref() != Some(channel_id.as_str()) {
                            return;
                        }
                        this.reconcile_temp(&temp_id, message_from_api(sent));
                        cx.emit(MessagesEvent::Appended);
                        cx.notify();
                    });
                }
                Err(e) => tracing::error!("send_channel_message failed: {e}"),
            }
        })
        .detach();
    }

    fn on_active_channel_changed(&mut self, channel_id: Option<String>, cx: &mut Context<Self>) {
        let Some(channel_id) = channel_id else {
            self.messages.clear();
            self.loaded_channel_id = None;
            self.loaded_clan_id = None;
            self.has_more = false;
            self.loading = false;
            self.loading_more = false;
            cx.emit(MessagesEvent::Reset { count: 0 });
            cx.notify();
            return;
        };

        if self.loaded_channel_id.as_deref() == Some(channel_id.as_str()) {
            return;
        }

        let Some(channel) = ChannelList::global(cx)
            .read(cx)
            .find_channel(&channel_id)
            .cloned()
        else {
            return;
        };

        self.loaded_channel_id = Some(channel_id.clone());
        self.loaded_clan_id = Some(channel.clan_id.clone());
        self.is_public = !channel.private;
        self.messages.clear();
        self.has_more = true;
        self.loading = true;
        self.loading_more = false;
        cx.emit(MessagesEvent::Reset { count: 0 });
        cx.notify();

        let api_join = self.api.clone();
        let api_fetch = self.api.clone();
        let clan_id_join = channel.clan_id.clone();
        let clan_id_fetch = channel.clan_id.clone();
        let ch_id_join = channel.id.clone();
        let ch_id_fetch = channel.id.clone();
        let is_public = self.is_public;

        cx.spawn(async move |_this, _cx| {
            if let Err(e) = api_join
                .join_chat(&clan_id_join, &ch_id_join, CHANNEL_TYPE_CHANNEL, is_public)
                .await
            {
                tracing::warn!("join_chat failed: {e}");
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            let result = api_fetch
                .list_channel_messages(&clan_id_fetch, &ch_id_fetch, "", 0, MESSAGE_PAGE_LIMIT)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                if this.loaded_channel_id.as_deref() != Some(ch_id_fetch.as_str()) {
                    return;
                }
                match result {
                    Ok(msgs) => {
                        let mut store_msgs: Vec<Message> =
                            msgs.into_iter().map(message_from_api).collect();
                        store_msgs.sort_by_key(|m| m.create_time);
                        trim_messages(&mut store_msgs);
                        recompute_message_grouping(&mut store_msgs);
                        this.has_more = true;
                        this.messages = store_msgs;
                        cx.emit(MessagesEvent::Reset {
                            count: this.messages.len(),
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to fetch messages for {ch_id_fetch}: {e}");
                        this.has_more = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn handle_event(&mut self, event: RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::ChannelMessage(m) = event else {
            return;
        };
        let channel_id = m.channel_id.to_string();
        if self.loaded_channel_id.as_deref() != Some(channel_id.as_str()) {
            return;
        }
        let msg = message_from_api(MezonTransport::message_from_proto(m));
        if self.messages.iter().any(|x| x.id == msg.id) {
            return;
        }
        if let Some(slot) = self.messages.iter_mut().find(|x| {
            x.id.starts_with("temp-") && x.sender_id == msg.sender_id && x.content == msg.content
        }) {
            *slot = msg;
        } else {
            self.messages.push(msg);
        }
        self.messages.sort_by_key(|m| m.create_time);
        trim_messages(&mut self.messages);
        recompute_message_grouping(&mut self.messages);
        cx.emit(MessagesEvent::Appended);
        cx.notify();
    }

    fn reconcile_temp(&mut self, temp_id: &str, confirmed: Message) {
        if let Some(slot) = self.messages.iter_mut().find(|m| {
            m.id == temp_id || (m.id.starts_with("temp-") && m.content == confirmed.content)
        }) {
            *slot = confirmed;
        } else if !self.messages.iter().any(|m| m.id == confirmed.id) {
            self.messages.push(confirmed);
            self.messages.sort_by_key(|m| m.create_time);
            trim_messages(&mut self.messages);
            recompute_message_grouping(&mut self.messages);
        }
    }

    fn on_realtime_lagged(&mut self, cx: &mut Context<Self>) {
        tracing::warn!("MessagesStore realtime lagged — refetching current channel");
        self.refetch_current_messages(cx);
    }

    fn refetch_current_messages(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.loaded_channel_id.clone() else {
            return;
        };
        let Some(clan_id) = self.loaded_clan_id.clone() else {
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
                this.loading = false;
                if this.loaded_channel_id.as_deref() != Some(channel_id.as_str()) {
                    return;
                }
                match result {
                    Ok(msgs) => {
                        let mut store_msgs: Vec<Message> =
                            msgs.into_iter().map(message_from_api).collect();
                        store_msgs.sort_by_key(|m| m.create_time);
                        trim_messages(&mut store_msgs);
                        recompute_message_grouping(&mut store_msgs);
                        this.has_more = true;
                        this.messages = store_msgs;
                        cx.emit(MessagesEvent::Reset {
                            count: this.messages.len(),
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to refetch messages for {channel_id}: {e}");
                        this.has_more = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
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
