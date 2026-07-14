use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use mezon_client::{
    AppApi, AttachmentUploadOutcome, ConnectionStatus, RealtimeEvent, TopicDiscussion, UploadFile,
    UploadThumbnail, UrlAttachment, topic_discussion_from_api,
};
use mezon_proto::{api, realtime};

use crate::channel::{ChannelList, ChannelType};
use crate::channel_permissions::ChannelPermissionsStore;
use crate::message::MessageCode;
use crate::message_time::{normalize_unix_seconds, unix_now_seconds};
use crate::messages::{
    MessagesStore, OutgoingAttachment, OutgoingContent, OutgoingEmoji, OutgoingHashtag,
    OutgoingMention, ReplyDraft, viewer_user_id,
};
use crate::presign;
use crate::realtime::{RealtimeDispatch, RealtimeKind};
use crate::{CACHE_TTL, ChannelId, Message, MessageId, UserId};

const TOPICS_LIMIT: i32 = 50;
const STREAM_MODE_CHANNEL: i32 = 2;
const STREAM_MODE_THREAD: i32 = 6;
const UPDATED_NOTIFY_COALESCE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub enum TopicsEvent {
    Updated,
    Opened,
    Closed,
    ReplySent,
}

#[derive(Debug, Clone)]
pub struct TopicMessageMeta {
    pub rpl: i32,
    pub lsnt: i64,
    pub tp_id: String,
}

#[derive(Default)]
struct TopicCompose {
    origin_message: Option<Message>,
    origin_message_id: Option<MessageId>,
    parent_channel_id: Option<i64>,
    clan_id: Option<i64>,
    mode: i32,
    is_public: bool,
    active_topic_id: Option<i64>,
    submitting: bool,
    creating: bool,
    error: Option<String>,
}

pub struct TopicsStore {
    topics: Vec<mezon_client::TopicDiscussion>,
    topic_index: std::collections::HashMap<String, usize>,
    topic_meta: std::collections::HashMap<String, TopicMessageMeta>,
    clan_id: Option<String>,
    loading: bool,
    fetch_generation: u64,
    fetched_at: Option<Instant>,
    panel_open: bool,
    init_topic_message_id: Option<MessageId>,
    reply_target: Option<ReplyDraft>,
    compose: TopicCompose,
    api: Arc<AppApi>,
    updated_notify_task: Option<Task<()>>,
    _conn_watch: Task<()>,
}

struct GlobalTopicsStore(Entity<TopicsStore>);
impl Global for GlobalTopicsStore {}

impl EventEmitter<TopicsEvent> for TopicsStore {}

impl TopicsStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalTopicsStore(entity.clone()));
        entity
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);
        Self::register_realtime(cx);
        Self {
            topics: Vec::new(),
            topic_index: std::collections::HashMap::new(),
            topic_meta: std::collections::HashMap::new(),
            clan_id: None,
            loading: false,
            fetch_generation: 0,
            fetched_at: None,
            panel_open: false,
            init_topic_message_id: None,
            reply_target: None,
            compose: TopicCompose::default(),
            api,
            updated_notify_task: None,
            _conn_watch: conn_watch,
        }
    }

    fn schedule_updated_notify(&mut self, cx: &mut Context<Self>) {
        if self.updated_notify_task.is_some() {
            return;
        }
        self.updated_notify_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(UPDATED_NOTIFY_COALESCE)
                .await;
            let _ = this.update(cx, |store, cx| {
                store.updated_notify_task = None;
                cx.emit(TopicsEvent::Updated);
                cx.notify();
            });
        }));
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
                    if this
                        .update(cx, |this, cx| this.refetch_active_clan(cx))
                        .is_err()
                    {
                        break;
                    }
                } else if !connected {
                    was_connected = false;
                }
            }
        })
    }

    fn refetch_active_clan(&mut self, cx: &mut Context<Self>) {
        self.fetched_at = None;
        if let Some(clan_id) = self.clan_id.clone() {
            self.fetch(&clan_id, cx);
        }
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalTopicsStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalTopicsStore>().map(|g| g.0.clone())
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.topics.clear();
        self.topic_index.clear();
        self.topic_meta.clear();
        self.clan_id = None;
        self.loading = false;
        self.fetch_generation = self.fetch_generation.wrapping_add(1);
        self.fetched_at = None;
        self.init_topic_message_id = None;
        self.updated_notify_task = None;
        self.close_panel(cx);
        cx.emit(TopicsEvent::Updated);
        cx.notify();
    }

    pub fn is_panel_open(&self) -> bool {
        self.panel_open
    }

    pub fn origin_message(&self) -> Option<&Message> {
        self.compose.origin_message.as_ref()
    }

    pub fn is_init_topic_message(&self, message_id: MessageId) -> bool {
        self.init_topic_message_id == Some(message_id)
    }

    pub fn clear_init_topic_message_if(&mut self, message_id: MessageId) {
        if self.init_topic_message_id == Some(message_id) {
            self.init_topic_message_id = None;
        }
    }

    pub fn reply_target(&self) -> Option<&ReplyDraft> {
        self.reply_target.as_ref()
    }

    pub fn set_reply_to(&mut self, message_id: MessageId, cx: &mut Context<Self>) {
        let Some(draft) = MessagesStore::global(cx)
            .read(cx)
            .reply_draft_for(message_id)
        else {
            return;
        };
        self.reply_target = Some(draft);
        cx.notify();
    }

    pub fn clear_reply(&mut self, cx: &mut Context<Self>) {
        if self.reply_target.take().is_some() {
            cx.notify();
        }
    }

    pub fn active_topic_id(&self) -> Option<i64> {
        self.compose.active_topic_id
    }

    pub fn is_submitting(&self) -> bool {
        self.compose.submitting
    }

    pub fn create_error(&self) -> Option<&str> {
        self.compose.error.as_deref()
    }

    pub fn topic_meta_for_topic(&self, topic_id: ChannelId) -> Option<&TopicMessageMeta> {
        self.topic_meta.get(&topic_id.to_string())
    }

    pub fn topic_last_reply_timestamp(&self, topic_id: ChannelId) -> Option<i64> {
        self.topic_meta_for_topic(topic_id)
            .map(|meta| normalize_unix_seconds(meta.lsnt))
            .filter(|ts| *ts > 0)
    }

    pub fn topic_display_reply_count(&self, topic_id: ChannelId) -> i32 {
        self.topic_meta_for_topic(topic_id)
            .map(|meta| meta.rpl)
            .unwrap_or(0)
    }

    fn upsert_topic_meta(&mut self, topic_id: String, rpl: i32, lsnt: i64, cx: &mut Context<Self>) {
        let lsnt = normalize_unix_seconds(lsnt);
        let merged = self
            .topic_meta
            .get(&topic_id)
            .map(|existing| TopicMessageMeta {
                rpl,
                lsnt: existing.lsnt.max(lsnt),
                tp_id: topic_id.clone(),
            })
            .unwrap_or(TopicMessageMeta {
                rpl,
                lsnt,
                tp_id: topic_id.clone(),
            });
        self.topic_meta.insert(topic_id.clone(), merged);
        if lsnt > 0
            && let Some(idx) = self.topic_index.get(&topic_id).copied()
            && let Some(topic) = self.topics.get_mut(idx)
        {
            topic.last_message_timestamp = lsnt.clamp(0, i64::from(u32::MAX)) as u32;
        }
        self.schedule_updated_notify(cx);
    }

    pub fn touch_topic_last_sent(
        &mut self,
        topic_id: i64,
        timestamp_sec: i64,
        cx: &mut Context<Self>,
    ) {
        if topic_id == 0 {
            return;
        }
        let timestamp_sec = normalize_unix_seconds(timestamp_sec);
        if timestamp_sec <= 0 {
            return;
        }
        let key = topic_id.to_string();
        if let Some(meta) = self.topic_meta.get_mut(&key) {
            meta.lsnt = meta.lsnt.max(timestamp_sec);
        } else {
            self.topic_meta.insert(
                key.clone(),
                TopicMessageMeta {
                    rpl: 0,
                    lsnt: timestamp_sec,
                    tp_id: key,
                },
            );
        }
        if let Some(idx) = self.topic_index.get(&topic_id.to_string()).copied()
            && let Some(topic) = self.topics.get_mut(idx)
        {
            topic.last_message_timestamp = timestamp_sec.clamp(0, i64::from(u32::MAX)) as u32;
        }
        self.schedule_updated_notify(cx);
    }

    pub fn increment_topic_reply_count(
        &mut self,
        topic_id: i64,
        timestamp_sec: i64,
        cx: &mut Context<Self>,
    ) {
        if topic_id == 0 {
            return;
        }
        let timestamp_sec = normalize_unix_seconds(timestamp_sec);
        let key = topic_id.to_string();
        if let Some(meta) = self.topic_meta.get_mut(&key) {
            meta.rpl = meta.rpl.saturating_add(1);
            if timestamp_sec > 0 {
                meta.lsnt = meta.lsnt.max(timestamp_sec);
            }
        } else {
            self.topic_meta.insert(
                key.clone(),
                TopicMessageMeta {
                    rpl: 1,
                    lsnt: timestamp_sec,
                    tp_id: key,
                },
            );
        }
        if timestamp_sec > 0
            && let Some(idx) = self.topic_index.get(&topic_id.to_string()).copied()
            && let Some(topic) = self.topics.get_mut(idx)
        {
            topic.last_message_timestamp = timestamp_sec.clamp(0, i64::from(u32::MAX)) as u32;
        }
        self.schedule_updated_notify(cx);
    }

    pub fn decrement_topic_reply_count(&mut self, topic_id: i64, cx: &mut Context<Self>) {
        if topic_id == 0 {
            return;
        }
        let key = topic_id.to_string();
        if let Some(meta) = self.topic_meta.get_mut(&key) {
            meta.rpl = meta.rpl.saturating_sub(1).max(0);
            self.schedule_updated_notify(cx);
        }
    }

    fn resolve_topic_id_for_origin(&self, origin_message_id: i64, cx: &App) -> Option<String> {
        let origin_key = origin_message_id.to_string();
        if let Some(topic) = self.topics.iter().find(|t| t.message_id == origin_key) {
            return Some(topic.id.clone());
        }
        MessagesStore::try_global(cx).and_then(|store| {
            store
                .read(cx)
                .viewport_message_by_id(MessageId(origin_message_id))
                .and_then(|msg| msg.topic_id.map(|id| id.to_string()))
        })
    }

    pub fn ensure_create_permissions(cx: &mut App) {
        let messages = MessagesStore::global(cx).read(cx);
        let (Some(channel_id), Some(clan_id)) =
            (messages.active_channel_id(), messages.active_clan_id())
        else {
            return;
        };
        if clan_id.is_zero() {
            return;
        }
        ChannelPermissionsStore::global(cx).update(cx, |store, cx| {
            store.ensure_loaded(clan_id, channel_id, cx);
        });
    }

    pub fn can_create_topic(cx: &App) -> bool {
        let messages = MessagesStore::global(cx).read(cx);
        if messages.is_dm() {
            return false;
        }
        let mode = messages.mode();
        if mode != STREAM_MODE_CHANNEL && mode != STREAM_MODE_THREAD {
            return false;
        }
        let (Some(channel_id), Some(clan_id)) =
            (messages.active_channel_id(), messages.active_clan_id())
        else {
            return false;
        };
        if clan_id.is_zero() {
            return false;
        }
        let channel_lookup = ChannelList::global(cx)
            .read(cx)
            .channel(clan_id, channel_id)
            .map(|c| c.channel_type.as_raw());
        let Some(channel_type_raw) = channel_lookup else {
            return false;
        };
        if matches!(
            channel_type_raw,
            x if x == ChannelType::App.as_raw()
                || x == ChannelType::Voice.as_raw()
                || x == ChannelType::Stream.as_raw()
        ) {
            return false;
        }
        ChannelPermissionsStore::global(cx).read(cx).has_permission(
            "send-message",
            clan_id,
            channel_id,
        )
    }

    pub fn message_allows_topic_discussion(msg: &Message) -> bool {
        if matches!(msg.code, MessageCode::Topic | MessageCode::Poll) {
            false
        } else if msg.code.is_system() {
            false
        } else {
            !matches!(
                msg.code,
                MessageCode::CreateThread
                    | MessageCode::CreatePin
                    | MessageCode::MessageBuzz
                    | MessageCode::AuditLog
                    | MessageCode::Welcome
                    | MessageCode::UpcomingEvent
            )
        }
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(RealtimeKind::SdTopicEvent, &entity, |this, event, cx| {
                this.handle_sd_topic_event(event, cx);
            });
            dispatch.on(
                RealtimeKind::TopicInMessageEvent,
                &entity,
                |this, event, cx| {
                    this.handle_topic_in_message_event(event, cx);
                },
            );
        });
    }

    fn handle_sd_topic_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::SdTopicEvent(ev) = event else {
            return;
        };
        let topic = topic_discussion_from_api(sd_topic_from_event(ev));
        self.upsert_topic(topic);
        let lsnt = normalize_unix_seconds(
            ev.last_sent_message
                .as_ref()
                .map(|h| i64::from(h.timestamp_seconds))
                .unwrap_or_else(unix_now_seconds),
        );
        let topic_key = ev.id.to_string();
        if self.topic_meta.contains_key(&topic_key) {
            self.touch_topic_last_sent(ev.id, lsnt, cx);
        } else {
            self.upsert_topic_meta(topic_key, 0, lsnt, cx);
        }
        MessagesStore::global(cx).update(cx, |store, cx| {
            store.mark_message_as_topic(
                ChannelId(ev.channel_id),
                MessageId(ev.message_id),
                ev.id,
                (ev.user_id != 0).then_some(UserId(ev.user_id)),
                cx,
            );
        });
    }

    fn handle_topic_in_message_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::TopicInMessageEvent(ev) = event else {
            return;
        };
        let tp_id = if !ev.tp_id.is_empty() {
            ev.tp_id.clone()
        } else {
            let Some(resolved) = self.resolve_topic_id_for_origin(ev.message_id, cx) else {
                return;
            };
            resolved
        };
        let lsnt = normalize_unix_seconds(ev.lsnt);
        self.upsert_topic_meta(tp_id, ev.rpl, lsnt, cx);
    }

    fn upsert_topic(&mut self, topic: TopicDiscussion) {
        let topic_id = topic.id.clone();
        if let Some(idx) = self.topic_index.get(&topic_id).copied() {
            let existing = &mut self.topics[idx];
            if !topic.content.is_empty() {
                existing.content = topic.content;
            }
            existing.last_message_timestamp = topic.last_message_timestamp;
            if !topic.last_sender_id.is_empty() && topic.last_sender_id != "0" {
                existing.last_sender_id = topic.last_sender_id;
            }
            if !topic.message_id.is_empty() && topic.message_id != "0" {
                existing.message_id = topic.message_id;
            }
        } else {
            let idx = self.topics.len();
            self.topic_index.insert(topic_id, idx);
            self.topics.push(topic);
        }
    }

    pub fn start_create_for_message(&mut self, message_id: MessageId, cx: &mut Context<Self>) {
        let Some(origin) = MessagesStore::global(cx)
            .read(cx)
            .viewport_message_by_id(message_id)
            .cloned()
        else {
            return;
        };
        self.start_create(origin, cx);
    }

    /// Open the discussion panel for `origin` (mezon-react `handleCreateTopic`).
    /// Reuses `origin.topic_id` when already set (topic button or card footer);
    /// otherwise the topic is created on first reply.
    pub fn start_create(&mut self, origin: Message, cx: &mut Context<Self>) {
        let messages = MessagesStore::global(cx);
        let (parent_channel_id, clan_id, mode, is_public) = {
            let store = messages.read(cx);
            (
                store.active_channel_id().map(|c| c.get()),
                store.active_clan_id().map_or(0, |c| c.get()),
                store.mode(),
                store.is_public(),
            )
        };
        let Some(parent_channel_id) = parent_channel_id else {
            return;
        };

        let existing_topic_id = origin.topic_id.map(|t| t.get()).filter(|id| *id != 0);

        self.init_topic_message_id = Some(origin.id);
        self.compose = TopicCompose {
            origin_message_id: Some(origin.id),
            origin_message: Some(origin),
            parent_channel_id: Some(parent_channel_id),
            clan_id: Some(clan_id),
            mode,
            is_public,
            active_topic_id: existing_topic_id,
            submitting: false,
            creating: false,
            error: None,
        };
        self.panel_open = true;

        messages.update(cx, |store, cx| {
            store.set_active_topic(existing_topic_id, cx);
        });
        if let Some(topic_id) = existing_topic_id {
            messages.update(cx, |store, cx| {
                store.fetch_topic_messages(clan_id, parent_channel_id, topic_id, cx);
            });
        }

        cx.emit(TopicsEvent::Opened);
        cx.notify();
    }

    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panel_open && self.compose.origin_message.is_none() {
            return;
        }
        self.close_panel_state(cx);
        MessagesStore::global(cx).update(cx, |store, cx| store.set_active_topic(None, cx));
    }

    pub fn should_close_on_message_deleted(
        &self,
        message_id: MessageId,
        message_topic_id: Option<ChannelId>,
    ) -> bool {
        if !self.panel_open {
            return false;
        }
        let is_origin = self.compose.origin_message_id == Some(message_id);
        let is_topic_anchor = message_topic_id
            .zip(self.compose.active_topic_id.map(ChannelId))
            .is_some_and(|(tid, active)| tid == active);
        is_origin || is_topic_anchor
    }

    pub fn close_panel_from_messages(&mut self, cx: &mut Context<Self>) {
        self.close_panel_state(cx);
    }

    fn close_panel_state(&mut self, cx: &mut Context<Self>) {
        if !self.panel_open && self.compose.origin_message.is_none() {
            return;
        }
        self.panel_open = false;
        self.compose = TopicCompose::default();
        self.init_topic_message_id = None;
        self.reply_target = None;
        cx.emit(TopicsEvent::Closed);
        cx.notify();
    }

    pub fn maybe_close_on_message_deleted(
        &mut self,
        message_id: MessageId,
        message_topic_id: Option<ChannelId>,
        cx: &mut Context<Self>,
    ) {
        if self.should_close_on_message_deleted(message_id, message_topic_id) {
            self.close_panel(cx);
        }
    }

    /// Send a reply into the topic composer. On the first reply the backend topic
    /// is created (mezon-js `CreateSdTopic`) then the message is sent with the
    /// returned topic id; subsequent replies reuse it. Guards against double-submit.
    pub fn submit_reply(
        &mut self,
        content: String,
        content_tokens: OutgoingContent,
        attachments: Vec<OutgoingAttachment>,
        cx: &mut Context<Self>,
    ) {
        let content = content.trim().to_string();
        if (content.is_empty() && attachments.is_empty()) || self.compose.submitting {
            return;
        }
        let (Some(parent_channel_id), Some(clan_id), Some(origin_message_id)) = (
            self.compose.parent_channel_id,
            self.compose.clan_id,
            self.compose.origin_message_id,
        ) else {
            return;
        };
        let mode = self.compose.mode;
        let is_public = self.compose.is_public;
        let existing_topic_id = self.compose.active_topic_id;
        let has_attachments = !attachments.is_empty();
        let reply_ref =
            self.reply_target
                .take()
                .map(|draft| mezon_client::transport::OutgoingReply {
                    message_ref_id: draft.message_ref_id.get(),
                    content: draft.content_preview,
                    has_attachment: draft.has_attachment,
                    message_sender_id: draft.sender_id.get(),
                    message_sender_username: draft.sender_name.clone(),
                    message_sender_avatar: draft.sender_avatar,
                    message_sender_clan_nick: String::new(),
                    message_sender_display_name: draft.sender_name,
                });

        let transport_mentions: Vec<mezon_client::transport::OutgoingMention> = content_tokens
            .mentions
            .into_iter()
            .map(OutgoingMention::into_transport)
            .collect();
        let transport_hashtags: Vec<mezon_client::transport::OutgoingHashtag> = content_tokens
            .hashtags
            .into_iter()
            .map(OutgoingHashtag::into_transport)
            .collect();
        let transport_emojis: Vec<mezon_client::transport::OutgoingEmoji> = content_tokens
            .emojis
            .into_iter()
            .map(OutgoingEmoji::into_transport)
            .collect();

        self.compose.submitting = true;
        self.compose.creating = existing_topic_id.is_none();
        self.compose.error = None;
        cx.notify();

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let topic_id = match existing_topic_id {
                Some(id) => id,
                None => match api
                    .create_sd_topic(origin_message_id.get(), clan_id, parent_channel_id)
                    .await
                {
                    Ok(topic) => topic.id,
                    Err(e) => {
                        tracing::error!("submit_reply create_sd_topic failed: {e}");
                        let _ = this.update(cx, |this, cx| {
                            this.compose.submitting = false;
                            this.compose.creating = false;
                            this.compose.error = Some(e.to_string());
                            cx.notify();
                        });
                        return;
                    }
                },
            };

            let ack = if has_attachments {
                let files: Vec<UploadFile> = attachments
                    .into_iter()
                    .map(|att| {
                        let thumbnail = att.poster_jpeg.map(|jpeg| UploadThumbnail {
                            filename: format!("{}.jpg", att.filename),
                            data: jpeg,
                        });
                        UploadFile {
                            path: att.path,
                            filename: att.filename,
                            filetype: att.filetype,
                            width: att.width,
                            height: att.height,
                            thumbnail,
                        }
                    })
                    .collect();
                let presigned = match api.presign_files(files).await {
                    Ok(presigned) => presigned,
                    Err(e) => {
                        tracing::error!("submit_reply presign failed: {e}");
                        let _ = this.update(cx, |this, cx| {
                            this.compose.submitting = false;
                            this.compose.creating = false;
                            this.compose.error = Some(e.to_string());
                            cx.notify();
                        });
                        return;
                    }
                };
                let msg_attachments: Vec<_> =
                    presigned.iter().map(|p| p.attachment.clone()).collect();
                let keys: Vec<String> = msg_attachments
                    .iter()
                    .map(|a| presign::normalize_presign_key(&a.url))
                    .collect();
                let update_mentions = transport_mentions.clone();
                let sent = match api
                    .send_topic_presigned_message(
                        clan_id,
                        parent_channel_id,
                        &content,
                        is_public,
                        mode,
                        topic_id,
                        msg_attachments,
                        transport_mentions,
                        transport_hashtags,
                        transport_emojis,
                        keys.clone(),
                        reply_ref.clone(),
                    )
                    .await
                {
                    Ok(sent) => sent,
                    Err(e) => {
                        tracing::error!("submit_reply send_topic_presigned_message failed: {e}");
                        let _ = this.update(cx, |this, cx| {
                            this.compose.submitting = false;
                            this.compose.creating = false;
                            this.compose.error = Some(e.to_string());
                            cx.notify();
                        });
                        return;
                    }
                };
                let real_message_id = sent.message_id;
                let create_time_seconds = sent.create_time.max(0) as u32;
                let (on_complete, _completions) =
                    tokio::sync::mpsc::unbounded_channel::<AttachmentUploadOutcome>();
                api.upload_presigned_and_patch(
                    clan_id,
                    parent_channel_id,
                    real_message_id,
                    &content,
                    update_mentions,
                    create_time_seconds,
                    presigned,
                    keys,
                    mode,
                    is_public,
                    on_complete,
                )
                .await;
                sent
            } else {
                match api
                    .send_topic_message(
                        clan_id,
                        parent_channel_id,
                        &content,
                        is_public,
                        mode,
                        topic_id,
                        transport_mentions,
                        transport_hashtags,
                        transport_emojis,
                        reply_ref,
                    )
                    .await
                {
                    Ok(ack) => ack,
                    Err(e) => {
                        tracing::error!("submit_reply send_topic_message failed: {e}");
                        let _ = this.update(cx, |this, cx| {
                            this.compose.submitting = false;
                            this.compose.creating = false;
                            this.compose.error = Some(e.to_string());
                            cx.notify();
                        });
                        return;
                    }
                }
            };

            let _ = this.update(cx, |this, cx| {
                this.compose.submitting = false;
                this.compose.creating = false;
                let is_new_topic = existing_topic_id.is_none();
                this.compose.active_topic_id = Some(topic_id);
                let creator_id = viewer_user_id(cx);
                let reply_timestamp = {
                    let ts = normalize_unix_seconds(ack.create_time);
                    if ts > 0 { ts } else { unix_now_seconds() }
                };
                this.increment_topic_reply_count(topic_id, reply_timestamp, cx);
                let messages = MessagesStore::global(cx);
                messages.update(cx, |store, cx| {
                    store.set_active_topic(Some(topic_id), cx);
                    store.append_topic_message(topic_id, ack, cx);
                    if is_new_topic {
                        store.mark_message_as_topic(
                            ChannelId(parent_channel_id),
                            origin_message_id,
                            topic_id,
                            creator_id,
                            cx,
                        );
                    }
                });
                cx.emit(TopicsEvent::Updated);
                cx.emit(TopicsEvent::ReplySent);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn submit_reply_url_attachment(
        &mut self,
        url: String,
        filename: String,
        filetype: String,
        cx: &mut Context<Self>,
    ) {
        if url.is_empty() || self.compose.submitting {
            return;
        }
        let (Some(parent_channel_id), Some(clan_id), Some(origin_message_id)) = (
            self.compose.parent_channel_id,
            self.compose.clan_id,
            self.compose.origin_message_id,
        ) else {
            return;
        };
        let mode = self.compose.mode;
        let is_public = self.compose.is_public;
        let existing_topic_id = self.compose.active_topic_id;
        let reply_ref =
            self.reply_target
                .take()
                .map(|draft| mezon_client::transport::OutgoingReply {
                    message_ref_id: draft.message_ref_id.get(),
                    content: draft.content_preview,
                    has_attachment: draft.has_attachment,
                    message_sender_id: draft.sender_id.get(),
                    message_sender_username: draft.sender_name.clone(),
                    message_sender_avatar: draft.sender_avatar,
                    message_sender_clan_nick: String::new(),
                    message_sender_display_name: draft.sender_name,
                });

        self.compose.submitting = true;
        self.compose.creating = existing_topic_id.is_none();
        self.compose.error = None;
        cx.notify();

        let api = self.api.clone();
        let attachment = UrlAttachment {
            url,
            filename,
            filetype,
        };
        cx.spawn(async move |this, cx| {
            let topic_id = match existing_topic_id {
                Some(id) => id,
                None => match api
                    .create_sd_topic(origin_message_id.get(), clan_id, parent_channel_id)
                    .await
                {
                    Ok(topic) => topic.id,
                    Err(e) => {
                        tracing::error!("submit_reply_url_attachment create_sd_topic failed: {e}");
                        let _ = this.update(cx, |this, cx| {
                            this.compose.submitting = false;
                            this.compose.creating = false;
                            this.compose.error = Some(e.to_string());
                            cx.notify();
                        });
                        return;
                    }
                },
            };

            let ack = match api
                .send_topic_message_with_attachment_urls(
                    clan_id,
                    parent_channel_id,
                    is_public,
                    mode,
                    topic_id,
                    vec![attachment],
                    reply_ref,
                )
                .await
            {
                Ok(ack) => ack,
                Err(e) => {
                    tracing::error!("submit_reply_url_attachment send failed: {e}");
                    let _ = this.update(cx, |this, cx| {
                        this.compose.submitting = false;
                        this.compose.creating = false;
                        this.compose.error = Some(e.to_string());
                        cx.notify();
                    });
                    return;
                }
            };

            let _ = this.update(cx, |this, cx| {
                this.compose.submitting = false;
                this.compose.creating = false;
                let is_new_topic = existing_topic_id.is_none();
                this.compose.active_topic_id = Some(topic_id);
                let creator_id = viewer_user_id(cx);
                let reply_timestamp = {
                    let ts = normalize_unix_seconds(ack.create_time);
                    if ts > 0 { ts } else { unix_now_seconds() }
                };
                this.increment_topic_reply_count(topic_id, reply_timestamp, cx);
                let messages = MessagesStore::global(cx);
                messages.update(cx, |store, cx| {
                    store.set_active_topic(Some(topic_id), cx);
                    store.append_topic_message(topic_id, ack, cx);
                    if is_new_topic {
                        store.mark_message_as_topic(
                            ChannelId(parent_channel_id),
                            origin_message_id,
                            topic_id,
                            creator_id,
                            cx,
                        );
                    }
                });
                cx.emit(TopicsEvent::Updated);
                cx.emit(TopicsEvent::ReplySent);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn topics(&self) -> &[TopicDiscussion] {
        &self.topics
    }

    pub fn topic_by_id(&self, id: &str) -> Option<&TopicDiscussion> {
        self.topic_index
            .get(id)
            .and_then(|&index| self.topics.get(index))
    }

    pub fn topics_for(&self, clan_id: &str) -> &[TopicDiscussion] {
        if self.clan_id.as_deref() == Some(clan_id) {
            &self.topics
        } else {
            &[]
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    fn is_fresh(&self, clan_id: &str) -> bool {
        self.clan_id.as_deref() == Some(clan_id)
            && self.fetched_at.is_some_and(|t| t.elapsed() < CACHE_TTL)
    }

    pub fn fetch_if_needed(&mut self, clan_id: &str, cx: &mut Context<Self>) {
        if self.is_fresh(clan_id) {
            return;
        }
        self.fetch(clan_id, cx);
    }

    pub fn fetch(&mut self, clan_id: &str, cx: &mut Context<Self>) {
        if self.loading && self.clan_id.as_deref() == Some(clan_id) {
            return;
        }
        if self.clan_id.as_deref() != Some(clan_id) {
            self.topics.clear();
            self.topic_index.clear();
            self.clan_id = Some(clan_id.to_string());
            self.fetched_at = None;
        }
        self.loading = true;
        self.fetch_generation = self.fetch_generation.wrapping_add(1);
        let generation = self.fetch_generation;
        cx.notify();

        let api = self.api.clone();
        let clan_id = clan_id.to_string();
        cx.spawn(async move |this, cx| {
            let result = api.list_sd_topics(&clan_id, TOPICS_LIMIT).await;
            let _ = this.update(cx, |this, cx| {
                this.apply_fetch_result(&clan_id, generation, result, cx);
            });
        })
        .detach();
    }

    fn apply_fetch_result(
        &mut self,
        clan_id: &str,
        generation: u64,
        result: Result<Vec<TopicDiscussion>, anyhow::Error>,
        cx: &mut Context<Self>,
    ) {
        if self.fetch_generation != generation {
            return;
        }
        self.loading = false;
        match result {
            Ok(mut topics) => {
                topics.sort_by_key(|t| std::cmp::Reverse(t.last_message_timestamp));
                self.topics = topics;
                self.topic_index = self
                    .topics
                    .iter()
                    .enumerate()
                    .map(|(index, topic)| (topic.id.clone(), index))
                    .collect();
                self.clan_id = Some(clan_id.to_string());
                self.fetched_at = Some(Instant::now());
                cx.emit(TopicsEvent::Updated);
                cx.notify();
            }
            Err(e) => {
                tracing::error!("list_sd_topics failed: {e}");
                cx.emit(TopicsEvent::Updated);
                cx.notify();
            }
        }
    }
}

fn sd_topic_from_event(ev: &realtime::SdTopicEvent) -> api::SdTopic {
    let content = ev
        .message
        .as_ref()
        .map(|m| m.content.clone())
        .filter(|c| !c.is_empty())
        .or_else(|| {
            ev.last_sent_message
                .as_ref()
                .map(|h| h.content.clone())
                .filter(|c| !c.is_empty())
        })
        .unwrap_or_default();
    let create_time_seconds = ev
        .message
        .as_ref()
        .map(|m| m.create_time_seconds)
        .filter(|t| *t != 0)
        .unwrap_or(0);
    api::SdTopic {
        id: ev.id,
        creator_id: ev.user_id,
        message_id: ev.message_id,
        clan_id: ev.clan_id,
        channel_id: ev.channel_id,
        status: 0,
        create_time_seconds,
        update_time_seconds: ev
            .last_sent_message
            .as_ref()
            .map(|h| h.timestamp_seconds as u32)
            .unwrap_or(0),
        content,
        last_sent_message: ev.last_sent_message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageCode;

    fn sample_message(code: MessageCode) -> Message {
        Message::new(MessageId(1), "hello", "1", "user", 0).with_code(code)
    }

    #[test]
    fn message_allows_topic_discussion_rejects_topic_and_poll() {
        assert!(!TopicsStore::message_allows_topic_discussion(
            &sample_message(MessageCode::Topic)
        ));
        assert!(!TopicsStore::message_allows_topic_discussion(
            &sample_message(MessageCode::Poll)
        ));
    }

    #[test]
    fn message_allows_topic_discussion_rejects_system_codes() {
        assert!(!TopicsStore::message_allows_topic_discussion(
            &sample_message(MessageCode::CreateThread)
        ));
        assert!(!TopicsStore::message_allows_topic_discussion(
            &sample_message(MessageCode::Welcome)
        ));
        assert!(TopicsStore::message_allows_topic_discussion(
            &sample_message(MessageCode::Chat)
        ));
    }
}
