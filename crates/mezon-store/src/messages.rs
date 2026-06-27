use crate::ids::{ChannelId, ClanId};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, Global, SharedString, Subscription, Task,
};
use mezon_client::transport::{
    ApiMessage, ApiMessageContent, OutgoingEmoji as TransportEmoji,
    OutgoingHashtag as TransportHashtag, OutgoingMention as TransportMention, OutgoingReply,
    detect_markdown, emoji_content_tokens, hashtag_content_tokens, markdown_content_tokens,
    mention_content_tokens,
};
use mezon_client::{
    AppApi, ConnectionStatus, MezonTransport, RealtimeEvent, UploadFile, UrlAttachment,
};

use crate::AppConfig;
use crate::KeyedCache;
use crate::channel::{ChannelEvent, ChannelList};
use crate::message::{
    Message, MessageAttachment, MessageCode, MessageReference, aggregate_reactions,
    apply_reaction_event, message_combined_with_prev, message_sort_key, parse_spans,
    recompute_message_grouping, sort_messages,
};
use crate::realtime::{RealtimeDispatch, RealtimeKind};

const MESSAGE_PAGE_LIMIT: u32 = 50;
const DIRECTION_BEFORE: i32 = 3;
const DIRECTION_AFTER: i32 = 1;
/// `Direction_Mode.AROUND_TIMESTAMP` — fetch a window centered on a message
/// (used by jump-to-message when the target is not loaded).
const DIRECTION_AROUND: i32 = 2;
const CHANNEL_TYPE_CHANNEL: i32 = 1;
const STICKER_FILETYPE: &str = "sticker";
const MAX_MESSAGES_PER_CHANNEL: usize = 100;
const MAX_CACHED_CHANNELS: usize = 30;

#[derive(Debug, Clone)]
pub enum MessagesEvent {
    /// The whole viewport was replaced (channel switch / fetch). `count` is the
    /// new row count.
    Reset { count: usize },
    /// The viewport window slid: rows were added/removed at either edge. The UI
    /// applies the matching splices so the visible scroll position is preserved.
    Shifted {
        added_top: usize,
        removed_top: usize,
        added_bottom: usize,
        removed_bottom: usize,
    },
    /// An in-place change to an existing row (e.g. a reaction add/remove) that
    /// does not alter the row count — the UI just needs to re-render.
    Updated,
    /// Scroll to and briefly highlight a message that is now in the buffer
    /// (cf. React `idMessageToJump`). Emitted by [`MessagesStore::jump_to_message`]
    /// once the target is loaded — either it was already present, or an
    /// AROUND fetch (which emits `Reset` first) just brought it in.
    JumpTo { message_id: SharedString },
}

/// The message currently being replied to (composer state), mirroring React's
/// reply reference draft in `references.slice`.
#[derive(Debug, Clone)]
pub struct ReplyDraft {
    pub message_ref_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub sender_avatar: String,
    pub content_preview: String,
    pub has_attachment: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutgoingMention {
    pub user_id: String,
    pub role_id: String,
    pub display: String,
    pub s: i32,
    pub e: i32,
}

impl OutgoingMention {
    fn into_transport(self) -> TransportMention {
        TransportMention {
            user_id: self.user_id,
            role_id: self.role_id,
            username: self.display,
            s: self.s,
            e: self.e,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutgoingHashtag {
    pub channel_id: String,
    pub s: i32,
    pub e: i32,
}

impl OutgoingHashtag {
    fn into_transport(self) -> TransportHashtag {
        TransportHashtag {
            channel_id: self.channel_id,
            s: self.s,
            e: self.e,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutgoingEmoji {
    pub emoji_id: String,
    pub s: i32,
    pub e: i32,
}

impl OutgoingEmoji {
    fn into_transport(self) -> TransportEmoji {
        TransportEmoji {
            emoji_id: self.emoji_id,
            s: self.s,
            e: self.e,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OutgoingContent {
    pub mentions: Vec<OutgoingMention>,
    pub hashtags: Vec<OutgoingHashtag>,
    pub emojis: Vec<OutgoingEmoji>,
}

impl OutgoingContent {
    pub fn is_empty(&self) -> bool {
        self.mentions.is_empty() && self.hashtags.is_empty() && self.emojis.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct OutgoingAttachment {
    pub path: PathBuf,
    pub filename: String,
    pub filetype: String,
}

struct ChannelMessages {
    messages: Vec<Message>,
    /// More history exists above (older). Mirrors React `hasMoreTop`.
    has_more: bool,
    /// More messages exist below (newer) that are not loaded — only true after
    /// a jump-to-message loads a window that does not reach the newest message.
    /// Mirrors React `selectHasMoreBottomByChannelId`; `false` in normal flow
    /// (the newest message is always loaded), so the bottom network-load path
    /// stays inert until jump-to-message is wired.
    has_more_bottom: bool,
}

const STREAM_MODE_CHANNEL: i32 = 2;

pub struct MessagesStore {
    cache: KeyedCache<ChannelId, ChannelMessages>,
    active_channel_id: Option<ChannelId>,
    active_clan_id: Option<ClanId>,
    is_public: bool,
    is_dm: bool,
    mode: i32,
    loading: bool,
    loading_more: bool,
    /// Throttle state for older-history paging: when the backend answers very
    /// fast (<100ms) and the user flings the scrollbar, back off progressively
    /// so we don't blast through the whole history (cf. React `handleOnChange`).
    last_load_more: Option<Instant>,
    consecutive_loads: u32,
    fetch_generation: u64,
    /// Active reply target for the composer, if any.
    reply_target: Option<ReplyDraft>,
    joined_channels: HashSet<ChannelId>,
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
                this.on_active_channel_changed(*channel_id, cx);
            }
        });

        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);

        Self {
            cache: KeyedCache::new(Some(MAX_CACHED_CHANNELS)),
            active_channel_id: None,
            active_clan_id: None,
            is_public: true,
            is_dm: false,
            mode: STREAM_MODE_CHANNEL,
            loading: false,
            loading_more: false,
            last_load_more: None,
            consecutive_loads: 0,
            fetch_generation: 0,
            reply_target: None,
            joined_channels: HashSet::new(),
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
            dispatch.on(RealtimeKind::MessageReaction, &entity, |this, event, cx| {
                this.handle_reaction(event, cx)
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

    /// Full message buffer for the active channel (internal cache; may be large).
    pub fn messages(&self) -> &[Message] {
        self.active_channel_id
            .as_ref()
            .and_then(|id| self.cache.get(id))
            .map(|c| c.messages.as_slice())
            .unwrap_or(&[])
    }

    /// The messages exposed to the UI. The buffer is already bounded to
    /// `MAX_MESSAGES_PER_CHANNEL` — it *is* the sliding window — and `gpui::list`
    /// virtualizes painting, so the UI mirrors the whole buffer 1:1. Older/newer
    /// rows enter and leave the buffer as the user pages (cf. React's bounded
    /// `selectMessageViewportIdsByChannelId`).
    pub fn viewport_messages(&self) -> &[Message] {
        self.messages()
    }

    /// Emit the splice for a single row appended at the bottom, accounting for
    /// any front-trim that dropped the oldest rows to keep the buffer within the
    /// cap. `old_len` is the buffer length before the push.
    fn emit_appended(&mut self, old_len: usize, cx: &mut Context<Self>) {
        let new_len = self.messages().len();
        if new_len <= old_len {
            // Not a real append (replace / no-op).
            cx.emit(MessagesEvent::Updated);
            cx.notify();
            return;
        }
        // Exactly one row was pushed; the cap may have dropped some off the
        // front. removed_top = old + 1 - new.
        let removed_top = (old_len + 1).saturating_sub(new_len);
        cx.emit(MessagesEvent::Shifted {
            added_top: 0,
            removed_top,
            added_bottom: 1,
            removed_bottom: 0,
        });
        cx.notify();
    }

    /// Called by the timeline when the user scrolls to the top: fetch the next
    /// older page from the server. The buffer is the whole window, so there is
    /// no local "reveal" step — reaching the top always pages over the network.
    pub fn scroll_reached_top(&mut self, cx: &mut Context<Self>) {
        if self.active_channel_id.is_none() {
            return;
        }
        self.load_more(cx);
    }

    /// True when newer messages exist on the server that are not yet loaded
    /// (only after a jump-to-message lands on an older window). Mirrors React
    /// `selectHasMoreBottomByChannelId`. `false` in normal flow.
    pub fn has_more_bottom(&self) -> bool {
        self.active_channel_id
            .as_ref()
            .and_then(|id| self.cache.get(id))
            .map(|c| c.has_more_bottom)
            .unwrap_or(false)
    }

    /// Called by the timeline when the user scrolls to the bottom: fetch the
    /// next newer page from the server (only relevant after a jump-to-message,
    /// when the newest message is not loaded). This is a network load — there is
    /// no local "reveal newer", since in normal flow the newest is always shown.
    pub fn scroll_reached_bottom(&mut self, cx: &mut Context<Self>) {
        tracing::debug!(
            has_more_bottom = self.has_more_bottom(),
            "scroll_reached_bottom"
        );
        self.load_more_bottom(cx);
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// True while an older-history (load-more) fetch is in flight.
    pub fn is_loading_more(&self) -> bool {
        self.loading_more
    }

    fn channel_has_more(&self) -> bool {
        self.active_channel_id
            .as_ref()
            .and_then(|id| self.cache.get(id))
            .map(|c| c.has_more)
            .unwrap_or(false)
    }

    /// True while there is more history to show above the current viewport —
    /// either cached rows not yet revealed, or older pages still on the server.
    /// Mirrors React `selectHasMoreMessageByChannelId` (drives the persistent
    /// top loading skeleton).
    pub fn has_more_top(&self) -> bool {
        self.channel_has_more()
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more || self.loading {
            // Guard against duplicate fetches while one is already in flight
            // (cf. React `debounce`/loadingStatus). Logged to verify no dup call.
            tracing::debug!(
                loading_more = self.loading_more,
                loading = self.loading,
                "load_more skipped: fetch already in flight"
            );
            return;
        }
        let Some(channel_id) = self.active_channel_id else {
            return;
        };
        let Some(clan_id) = self.active_clan_id else {
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

        // Progressive backoff (cf. React `handleOnChange`): if loads keep firing
        // in quick succession (the user is flinging the scrollbar and the
        // backend answers in <100ms), delay each successive fetch a bit more so
        // we don't auto-page through the whole channel. Resets once the user
        // pauses for >300ms.
        let now = Instant::now();
        let rapid = self
            .last_load_more
            .map(|t| now.duration_since(t) < Duration::from_millis(300))
            .unwrap_or(false);
        self.consecutive_loads = if rapid {
            (self.consecutive_loads + 1).min(3)
        } else {
            0
        };
        self.last_load_more = Some(now);
        let backoff = Duration::from_millis(u64::from(self.consecutive_loads) * 333);

        self.loading_more = true;
        cx.notify();
        tracing::debug!(
            channel_id = channel_id.get(),
            before_message_id = %oldest_id,
            backoff_ms = backoff.as_millis() as u64,
            "load_more: fetching older page"
        );

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            if !backoff.is_zero() {
                cx.background_executor().timer(backoff).await;
            }
            let result = api
                .list_channel_messages(
                    clan_id.get(),
                    channel_id.get(),
                    oldest_id.parse::<i64>().unwrap_or(0),
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
                tracing::debug!(
                    channel_id = channel_id.get(),
                    fetched = msgs.len(),
                    "load_more: page received"
                );
                let cfg = AppConfig::try_global(cx);
                let (prepended, dropped_bottom) = {
                    let Some(channel) = this.cache.get_mut(&channel_id) else {
                        return;
                    };
                    let existing: std::collections::HashSet<&str> =
                        channel.messages.iter().map(|m| m.id.as_str()).collect();
                    let mut older: Vec<Message> = msgs
                        .into_iter()
                        .filter(|m| !existing.contains(m.message_id.to_string().as_str()))
                        .map(|m| message_from_api(m, cfg))
                        .collect();
                    if older.is_empty() {
                        channel.has_more = false;
                        // No more history above: tell the UI so it can drop the
                        // persistent top loading skeleton.
                        cx.emit(MessagesEvent::Updated);
                        cx.notify();
                        return;
                    }
                    let prepended = older.len();
                    older.append(&mut channel.messages);
                    sort_messages(&mut older);
                    // Drop the NEWEST rows (back) to stay within the cap, so the
                    // older rows we just loaded are kept. The dropped newest can
                    // be re-fetched when scrolling back down.
                    let dropped_bottom = trim_messages_back(&mut older);
                    channel.messages = older;
                    recompute_message_grouping(&mut channel.messages);
                    if dropped_bottom > 0 {
                        channel.has_more_bottom = true;
                    }
                    // Reached the channel start once the oldest row is the
                    // FIRST_MESSAGE sentinel (cf. React `hasMore` check).
                    channel.has_more = has_more_from_oldest(&channel.messages);
                    (prepended, dropped_bottom)
                };
                if this.active_channel_id == Some(channel_id) {
                    // Older rows were prepended; the cap may have dropped the same
                    // many newest rows off the back. Emit the exact splice so the
                    // UI window matches the buffer 1:1 — the prepend re-anchors to
                    // the prior first row, and the back-trim removes off-screen
                    // rows below.
                    cx.emit(MessagesEvent::Shifted {
                        added_top: prepended,
                        removed_top: 0,
                        added_bottom: 0,
                        removed_bottom: dropped_bottom,
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetch the next newer page from the server and append it (the bottom
    /// counterpart of [`Self::load_more`]). Only active after a jump-to-message,
    /// where `has_more_bottom` is set because the newest message is not loaded.
    pub fn load_more_bottom(&mut self, cx: &mut Context<Self>) {
        if self.loading_more || self.loading {
            tracing::debug!(
                loading_more = self.loading_more,
                loading = self.loading,
                "load_more_bottom skipped: fetch already in flight"
            );
            return;
        }
        let Some(channel_id) = self.active_channel_id else {
            return;
        };
        let Some(clan_id) = self.active_clan_id else {
            return;
        };
        let Some(channel) = self.cache.get(&channel_id) else {
            return;
        };
        if !channel.has_more_bottom {
            tracing::debug!("load_more_bottom skipped: has_more_bottom=false");
            return;
        }
        let Some(newest_id) = channel
            .messages
            .last()
            .map(|m| m.id.clone())
            .filter(|id| !id.starts_with("temp-"))
        else {
            tracing::debug!("load_more_bottom skipped: no non-temp newest id");
            return;
        };

        self.loading_more = true;
        cx.notify();
        // Dump the whole buffer id list so we can verify the anchor is really the
        // newest loaded row and the list is ordered ascending by id.
        let buffer_ids: Vec<&str> = channel.messages.iter().map(|m| m.id.as_str()).collect();
        tracing::debug!(
            channel_id = channel_id.get(),
            after_message_id = %newest_id,
            buffer_len = buffer_ids.len(),
            buffer_ids = ?buffer_ids,
            "load_more_bottom: fetching newer page"
        );

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_messages(
                    clan_id.get(),
                    channel_id.get(),
                    newest_id.parse::<i64>().unwrap_or(0),
                    DIRECTION_AFTER,
                    MESSAGE_PAGE_LIMIT,
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading_more = false;
                let msgs = match result {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        tracing::error!("Failed to load newer messages for {channel_id}: {e}");
                        cx.notify();
                        return;
                    }
                };
                tracing::debug!(
                    channel_id = channel_id.get(),
                    anchor_after = %newest_id,
                    fetched = msgs.len(),
                    raw_first = msgs.first().map(|m| m.message_id).unwrap_or(0),
                    raw_last = msgs.last().map(|m| m.message_id).unwrap_or(0),
                    raw_min = msgs.iter().map(|m| m.message_id).min().unwrap_or(0),
                    raw_max = msgs.iter().map(|m| m.message_id).max().unwrap_or(0),
                    "load_more_bottom: page received (raw server ids)"
                );
                let cfg = AppConfig::try_global(cx);
                let (added, dropped) = {
                    let Some(channel) = this.cache.get_mut(&channel_id) else {
                        return;
                    };
                    let existing: std::collections::HashSet<&str> =
                        channel.messages.iter().map(|m| m.id.as_str()).collect();
                    let fetched = msgs.len();
                    let mut newer: Vec<Message> = msgs
                        .into_iter()
                        .filter(|m| !existing.contains(m.message_id.to_string().as_str()))
                        .map(|m| message_from_api(m, cfg))
                        .collect();
                    // A short page means we've reached the newest message.
                    channel.has_more_bottom = fetched >= MESSAGE_PAGE_LIMIT as usize;
                    if newer.is_empty() {
                        cx.emit(MessagesEvent::Updated);
                        cx.notify();
                        return;
                    }
                    let added = newer.len();
                    channel.messages.append(&mut newer);
                    sort_messages(&mut channel.messages);
                    // Appending newer drops the oldest (front) at the cap; those
                    // older rows then become re-fetchable from the top again.
                    let dropped = trim_messages(&mut channel.messages);
                    if dropped > 0 {
                        channel.has_more = true;
                    }
                    recompute_message_grouping(&mut channel.messages);
                    (added, dropped)
                };
                if this.active_channel_id == Some(channel_id) {
                    if let Some(ch) = this.cache.get(&channel_id) {
                        tracing::debug!(
                            anchor_after = %newest_id,
                            added,
                            dropped,
                            buffer_oldest = ch.messages.first().map(|m| m.id.as_str()).unwrap_or(""),
                            buffer_newest = ch.messages.last().map(|m| m.id.as_str()).unwrap_or(""),
                            "load_more_bottom: appended newer page"
                        );
                    }
                    // Newer rows were appended; the cap may have dropped the same
                    // many oldest rows off the front. Emit the exact splice so the
                    // UI window matches the buffer 1:1 and the scroll stays
                    // anchored to the prior content.
                    cx.emit(MessagesEvent::Shifted {
                        added_top: 0,
                        removed_top: dropped,
                        added_bottom: added,
                        removed_bottom: 0,
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Jump to a message (cf. React `jumpToMessage`, used by reply previews).
    /// If the target is already in the buffer, emit [`MessagesEvent::JumpTo`] so
    /// the UI scrolls to it. Otherwise fetch a window centered on it
    /// (`AROUND_TIMESTAMP`), replace the buffer, and emit `Reset` then `JumpTo`.
    pub fn jump_to_message(&mut self, message_id: String, cx: &mut Context<Self>) {
        let Some(channel_id) = self.active_channel_id else {
            return;
        };
        // Already loaded → just scroll to it.
        if self.messages().iter().any(|m| m.id == message_id) {
            cx.emit(MessagesEvent::JumpTo {
                message_id: message_id.into(),
            });
            return;
        }
        if self.loading_more || self.loading {
            return;
        }
        let Some(clan_id) = self.active_clan_id else {
            return;
        };
        let Ok(anchor) = message_id.parse::<i64>() else {
            return;
        };

        self.loading_more = true;
        cx.notify();
        tracing::debug!(
            channel_id = channel_id.get(),
            %message_id,
            "jump_to_message: fetching AROUND window"
        );

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_messages(
                    clan_id.get(),
                    channel_id.get(),
                    anchor,
                    DIRECTION_AROUND,
                    MESSAGE_PAGE_LIMIT,
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading_more = false;
                let msgs = match result {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        tracing::error!(
                            "jump_to_message AROUND fetch failed for {channel_id}: {e}"
                        );
                        cx.notify();
                        return;
                    }
                };
                let cfg = AppConfig::try_global(cx);
                let mut window: Vec<Message> =
                    msgs.into_iter().map(|m| message_from_api(m, cfg)).collect();
                sort_messages(&mut window);
                // Centered trim if the window somehow exceeds the cap, keeping the
                // target near the middle so both directions stay scrollable.
                if window.len() > MAX_MESSAGES_PER_CHANNEL {
                    let target = window.iter().position(|m| m.id == message_id).unwrap_or(0);
                    let half = MAX_MESSAGES_PER_CHANNEL / 2;
                    let start = target
                        .saturating_sub(half)
                        .min(window.len() - MAX_MESSAGES_PER_CHANNEL);
                    window = window[start..start + MAX_MESSAGES_PER_CHANNEL].to_vec();
                }
                let found = window.iter().any(|m| m.id == message_id);
                if !found {
                    tracing::warn!(%message_id, "jump_to_message: target not in AROUND window");
                    cx.notify();
                    return;
                }
                recompute_message_grouping(&mut window);
                let has_more = has_more_from_oldest(&window);
                if let Some(channel) = this.cache.get_mut(&channel_id) {
                    channel.messages = window;
                    channel.has_more = has_more;
                    // We landed on an older window, so newer messages exist that
                    // are not loaded yet (scroll down pages them in). This
                    // self-corrects to false once the newest page is reached.
                    channel.has_more_bottom = true;
                }
                if this.active_channel_id == Some(channel_id) {
                    let count = this.messages().len();
                    cx.emit(MessagesEvent::Reset { count });
                    cx.emit(MessagesEvent::JumpTo {
                        message_id: message_id.into(),
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Current composer reply target (React reply reference draft).
    pub fn reply_target(&self) -> Option<&ReplyDraft> {
        self.reply_target.as_ref()
    }

    /// Set the composer reply target (from a "Reply" action on a message).
    pub fn set_reply(&mut self, draft: ReplyDraft, cx: &mut Context<Self>) {
        self.reply_target = Some(draft);
        cx.notify();
    }

    /// Clear the composer reply target.
    pub fn clear_reply(&mut self, cx: &mut Context<Self>) {
        if self.reply_target.take().is_some() {
            cx.notify();
        }
    }

    pub fn send_message(
        &mut self,
        content: String,
        sender_id: String,
        sender_name: String,
        content_tokens: OutgoingContent,
        attachments: Vec<OutgoingAttachment>,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.active_channel_id else {
            return;
        };
        let Some(clan_id) = self.active_clan_id else {
            return;
        };
        let is_public = self.is_public;
        let mode = self.mode;
        let has_attachments = !attachments.is_empty();
        // Consume the active reply target (one reply per sent message).
        let reply = self.reply_target.take();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        let temp_id = format!("temp-{now}");

        let Some(channel) = self.cache.get_mut(&channel_id) else {
            return;
        };
        let OutgoingContent {
            mentions,
            hashtags,
            emojis,
        } = content_tokens;
        let transport_mentions: Vec<TransportMention> = mentions
            .into_iter()
            .map(OutgoingMention::into_transport)
            .collect();
        let transport_hashtags: Vec<TransportHashtag> = hashtags
            .into_iter()
            .map(OutgoingHashtag::into_transport)
            .collect();
        let transport_emojis: Vec<TransportEmoji> = emojis
            .into_iter()
            .map(OutgoingEmoji::into_transport)
            .collect();
        let markdowns = detect_markdown(&content);
        let mut optimistic = Message::new(
            temp_id.clone(),
            content.clone(),
            sender_id,
            sender_name,
            now,
        );
        if !transport_mentions.is_empty()
            || !transport_hashtags.is_empty()
            || !transport_emojis.is_empty()
            || !markdowns.is_empty()
        {
            let tokens = ApiMessageContent {
                t: content.clone(),
                mentions: mention_content_tokens(&transport_mentions),
                hg: hashtag_content_tokens(&transport_hashtags),
                ej: emoji_content_tokens(&transport_emojis),
                mk: markdown_content_tokens(&markdowns),
                ..Default::default()
            };
            optimistic = optimistic.with_spans(parse_spans(&tokens));
        }
        if let Some(draft) = &reply {
            optimistic = optimistic.with_references(vec![MessageReference {
                message_ref_id: draft.message_ref_id.clone(),
                sender_id: draft.sender_id.clone(),
                sender_name: draft.sender_name.clone(),
                sender_avatar: draft.sender_avatar.clone(),
                content: draft.content_preview.clone(),
                has_attachment: draft.has_attachment,
            }]);
        }
        let old_len = channel.messages.len();
        channel.messages.push(optimistic);
        trim_messages(&mut channel.messages);
        recompute_message_grouping(&mut channel.messages);
        self.emit_appended(old_len, cx);

        let api = self.api.clone();
        let reply_ref = reply.map(|draft| OutgoingReply {
            message_ref_id: draft.message_ref_id.parse::<i64>().unwrap_or(0),
            content: draft.content_preview,
            has_attachment: draft.has_attachment,
            message_sender_id: draft.sender_id.parse::<i64>().unwrap_or(0),
            message_sender_username: draft.sender_name.clone(),
            message_sender_avatar: draft.sender_avatar,
            message_sender_clan_nick: String::new(),
            message_sender_display_name: draft.sender_name,
        });
        cx.spawn(async move |this, cx| {
            let result = if has_attachments {
                let files = cx
                    .background_spawn(async move {
                        attachments
                            .into_iter()
                            .filter_map(|att| {
                                std::fs::read(&att.path)
                                    .inspect_err(|e| {
                                        tracing::error!(
                                            "attachment read failed for {:?}: {e}",
                                            att.path
                                        )
                                    })
                                    .ok()
                                    .map(|data| UploadFile {
                                        filename: att.filename,
                                        filetype: att.filetype,
                                        data,
                                    })
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                api.send_message_with_attachments(
                    clan_id.get(),
                    channel_id.get(),
                    &content,
                    is_public,
                    mode,
                    files,
                )
                .await
            } else if let Some(reply_ref) = reply_ref {
                api.send_channel_message_reply(
                    clan_id.get(),
                    channel_id.get(),
                    &content,
                    is_public,
                    mode,
                    reply_ref,
                    transport_mentions,
                    transport_hashtags,
                    transport_emojis,
                )
                .await
            } else {
                api.send_channel_message(
                    clan_id.get(),
                    channel_id.get(),
                    &content,
                    is_public,
                    mode,
                    transport_mentions,
                    transport_hashtags,
                    transport_emojis,
                )
                .await
            };
            match result {
                Ok(sent) => {
                    let _ = this.update(cx, |this, cx| {
                        let confirmed = message_from_api(sent, AppConfig::try_global(cx));
                        this.reconcile_temp(channel_id, &temp_id, confirmed, cx);
                    });
                }
                Err(e) => {
                    tracing::error!("send_channel_message failed: {e}");
                    let _ = this.update(cx, |this, cx| {
                        this.remove_temp(channel_id, &temp_id, cx);
                    });
                }
            }
        })
        .detach();
    }

    pub fn send_sticker(
        &mut self,
        url: String,
        filename: String,
        sender_id: String,
        sender_name: String,
        cx: &mut Context<Self>,
    ) {
        if url.is_empty() {
            return;
        }
        let Some(channel_id) = self.active_channel_id else {
            return;
        };
        let Some(clan_id) = self.active_clan_id else {
            return;
        };
        let is_public = self.is_public;
        let mode = self.mode;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        let temp_id = format!("temp-{now}");

        let optimistic_attachment = MessageAttachment::from_api(
            mezon_client::transport::ApiAttachment {
                url: url.clone(),
                filename: filename.clone(),
                filetype: STICKER_FILETYPE.to_string(),
                width: 0,
                height: 0,
            },
            AppConfig::try_global(cx),
        );

        let Some(channel) = self.cache.get_mut(&channel_id) else {
            return;
        };
        let optimistic = Message::new(temp_id.clone(), String::new(), sender_id, sender_name, now)
            .with_attachments(vec![optimistic_attachment]);
        let old_len = channel.messages.len();
        channel.messages.push(optimistic);
        trim_messages(&mut channel.messages);
        recompute_message_grouping(&mut channel.messages);
        self.emit_appended(old_len, cx);

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .send_message_with_attachment_urls(
                    clan_id.get(),
                    channel_id.get(),
                    is_public,
                    mode,
                    vec![UrlAttachment {
                        url,
                        filename,
                        filetype: STICKER_FILETYPE.to_string(),
                    }],
                )
                .await;
            match result {
                Ok(sent) => {
                    let _ = this.update(cx, |this, cx| {
                        let confirmed = message_from_api(sent, AppConfig::try_global(cx));
                        this.reconcile_temp(channel_id, &temp_id, confirmed, cx);
                    });
                }
                Err(e) => {
                    tracing::error!("send sticker failed: {e}");
                    let _ = this.update(cx, |this, cx| {
                        this.remove_temp(channel_id, &temp_id, cx);
                    });
                }
            }
        })
        .detach();
    }

    fn on_active_channel_changed(&mut self, channel_id: Option<ChannelId>, cx: &mut Context<Self>) {
        let Some(channel_id) = channel_id else {
            self.active_channel_id = None;
            self.active_clan_id = None;
            self.is_dm = false;
            self.loading = false;
            self.loading_more = false;
            self.reply_target = None;
            cx.emit(MessagesEvent::Reset { count: 0 });
            cx.notify();
            return;
        };
        self.open_channel(channel_id, cx);
    }

    /// Open a clan channel as the active conversation (looks up clan/privacy from `ChannelList`).
    pub fn open_channel(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if self.active_channel_id == Some(channel_id) && !self.is_dm {
            if self.loading {
                return;
            }
            let empty = self
                .cache
                .get(&channel_id)
                .map(|c| c.messages.is_empty())
                .unwrap_or(true);
            if !empty || self.cache.is_fresh(&channel_id, crate::CACHE_TTL) {
                return;
            }
            self.refetch_current_messages(cx);
            return;
        }
        let Some(channel) = ChannelList::global(cx)
            .read(cx)
            .find_channel(channel_id)
            .cloned()
        else {
            return;
        };
        self.activate(
            channel.clan_id,
            channel_id,
            !channel.private,
            false,
            CHANNEL_TYPE_CHANNEL,
            STREAM_MODE_CHANNEL,
            cx,
        );
    }

    /// Open a direct message / group conversation (clan_id = 0) as the active conversation.
    /// `channel_type` is the raw DM type (3 = DM, 2 = group).
    pub fn open_direct(
        &mut self,
        channel_id: ChannelId,
        channel_type: i32,
        cx: &mut Context<Self>,
    ) {
        if self.active_channel_id == Some(channel_id) && self.is_dm {
            if self.loading {
                return;
            }
            let empty = self
                .cache
                .get(&channel_id)
                .map(|c| c.messages.is_empty())
                .unwrap_or(true);
            if !empty || self.cache.is_fresh(&channel_id, crate::CACHE_TTL) {
                return;
            }
            self.refetch_current_messages(cx);
            return;
        }
        let mode = if channel_type == 2 { 3 } else { 4 };
        self.activate(ClanId(0), channel_id, false, true, channel_type, mode, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn activate(
        &mut self,
        clan_id: ClanId,
        channel_id: ChannelId,
        is_public: bool,
        is_dm: bool,
        join_type: i32,
        mode: i32,
        cx: &mut Context<Self>,
    ) {
        self.active_channel_id = Some(channel_id);
        self.active_clan_id = Some(clan_id);
        self.is_public = is_public;
        self.is_dm = is_dm;
        self.mode = mode;
        self.loading_more = false;
        self.reply_target = None;
        self.fetch_generation = self.fetch_generation.wrapping_add(1);
        let generation = self.fetch_generation;

        if !self.joined_channels.contains(&channel_id) {
            self.joined_channels.insert(channel_id);
            self.spawn_join(clan_id, channel_id, join_type, is_public, cx);
        }

        if self.cache.is_fresh(&channel_id, crate::CACHE_TTL) {
            self.cache.touch(&channel_id);
            self.loading = false;
            let count = self.messages().len();
            cx.emit(MessagesEvent::Reset { count });
            cx.notify();
            return;
        }

        self.loading = true;
        if self.cache.contains(&channel_id) {
            self.cache.touch(&channel_id);
            let count = self.messages().len();
            cx.emit(MessagesEvent::Reset { count });
        } else {
            cx.emit(MessagesEvent::Reset { count: 0 });
        }
        cx.notify();
        self.spawn_initial_fetch(clan_id, channel_id, generation, cx);
    }

    fn spawn_join(
        &self,
        clan_id: ClanId,
        channel_id: ChannelId,
        join_type: i32,
        is_public: bool,
        cx: &mut Context<Self>,
    ) {
        let api = self.api.clone();
        cx.spawn(async move |_this, _cx| {
            if let Err(e) = api
                .join_chat(clan_id.get(), channel_id.get(), join_type, is_public)
                .await
            {
                tracing::warn!("join_chat failed: {e}");
            }
        })
        .detach();
    }

    fn spawn_initial_fetch(
        &self,
        clan_id: ClanId,
        channel_id: ChannelId,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_messages(clan_id.get(), channel_id.get(), 0, 0, MESSAGE_PAGE_LIMIT)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_initial_fetch_result(channel_id, generation, result, cx);
            });
        })
        .detach();
    }

    fn apply_initial_fetch_result(
        &mut self,
        channel_id: ChannelId,
        generation: u64,
        result: Result<Vec<ApiMessage>, anyhow::Error>,
        cx: &mut Context<Self>,
    ) {
        let is_active = self.active_channel_id == Some(channel_id);
        let is_current = is_active && self.fetch_generation == generation;

        match result {
            Ok(msgs) => {
                let messages = prepare_messages(msgs, AppConfig::try_global(cx));
                self.set_channel(channel_id, messages);
                if is_current {
                    self.loading = false;
                    let count = self.messages().len();
                    cx.emit(MessagesEvent::Reset { count });
                    cx.notify();
                }
            }
            Err(e) => {
                tracing::error!("Failed to fetch messages for {channel_id}: {e}");
                if is_current {
                    self.loading = false;
                    let count = self.messages().len();
                    cx.emit(MessagesEvent::Reset { count });
                    cx.notify();
                }
            }
        }
    }

    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::ChannelMessage(m) = event else {
            return;
        };
        let channel_id = ChannelId(m.channel_id);
        let is_active = self.active_channel_id == Some(channel_id);
        let cfg = AppConfig::try_global(cx);
        let Some(channel) = self.cache.get_mut(&channel_id) else {
            return;
        };
        let msg = message_from_api(MezonTransport::message_from_proto(m.clone()), cfg);
        if channel.messages.iter().any(|x| x.id == msg.id) {
            return;
        }
        let old_len = channel.messages.len();
        let appended = if let Some(slot) = channel.messages.iter_mut().find(|x| {
            x.id.starts_with("temp-") && x.sender_id == msg.sender_id && x.content == msg.content
        }) {
            *slot = msg;
            sort_messages(&mut channel.messages);
            trim_messages(&mut channel.messages);
            recompute_message_grouping(&mut channel.messages);
            false
        } else {
            push_message_grouped(&mut channel.messages, msg);
            true
        };
        if is_active {
            if appended {
                self.emit_appended(old_len, cx);
            } else {
                cx.emit(MessagesEvent::Updated);
                cx.notify();
            }
        }
    }

    fn handle_reaction(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::MessageReaction(r) = event else {
            return;
        };
        let channel_id = ChannelId(r.channel_id);
        let is_active = self.active_channel_id == Some(channel_id);
        let Some(channel) = self.cache.get_mut(&channel_id) else {
            return;
        };
        let msg_id = r.message_id.to_string();
        let Some(msg) = channel.messages.iter_mut().find(|m| m.id == msg_id) else {
            return;
        };
        apply_reaction_event(
            &mut msg.reactions,
            &r.emoji_id.to_string(),
            &r.emoji,
            &r.sender_id.to_string(),
            r.action,
        );
        if is_active {
            cx.emit(MessagesEvent::Updated);
            cx.notify();
        }
    }

    fn reconcile_temp(
        &mut self,
        channel_id: ChannelId,
        temp_id: &str,
        confirmed: Message,
        cx: &mut Context<Self>,
    ) {
        let (pushed, old_len) = {
            let Some(channel) = self.cache.get_mut(&channel_id) else {
                return;
            };
            let old_len = channel.messages.len();
            if let Some(slot) = channel.messages.iter_mut().find(|m| m.id == temp_id) {
                *slot = confirmed;
                (false, old_len)
            } else if !channel.messages.iter().any(|m| m.id == confirmed.id) {
                channel.messages.push(confirmed);
                sort_messages(&mut channel.messages);
                trim_messages(&mut channel.messages);
                recompute_message_grouping(&mut channel.messages);
                (true, old_len)
            } else {
                (false, old_len)
            }
        };
        if self.active_channel_id != Some(channel_id) {
            return;
        }
        if pushed {
            self.emit_appended(old_len, cx);
        } else {
            // In-place swap of the temp row for the confirmed one.
            cx.emit(MessagesEvent::Updated);
            cx.notify();
        }
    }

    fn remove_temp(&mut self, channel_id: ChannelId, temp_id: &str, cx: &mut Context<Self>) {
        let removed = {
            let Some(channel) = self.cache.get_mut(&channel_id) else {
                return;
            };
            let before = channel.messages.len();
            channel.messages.retain(|m| m.id != temp_id);
            let removed = before != channel.messages.len();
            if removed {
                recompute_message_grouping(&mut channel.messages);
            }
            removed
        };
        if removed && self.active_channel_id == Some(channel_id) {
            // The optimistic row left the bottom of the window.
            cx.emit(MessagesEvent::Shifted {
                added_top: 0,
                removed_top: 0,
                added_bottom: 0,
                removed_bottom: 1,
            });
            cx.notify();
        }
    }

    fn resync(&mut self, cx: &mut Context<Self>) {
        tracing::info!("MessagesStore resync — marking message cache stale");
        self.cache.mark_all_stale();
        self.joined_channels.clear();
        self.refetch_current_messages(cx);
    }

    /// Force a refetch of the open channel ignoring the cache (cf. React `noCache: true`).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refetch_current_messages(cx);
    }

    fn refetch_current_messages(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.active_channel_id else {
            return;
        };
        let Some(clan_id) = self.active_clan_id else {
            return;
        };

        self.loading = true;
        self.loading_more = false;
        self.fetch_generation = self.fetch_generation.wrapping_add(1);
        let generation = self.fetch_generation;
        cx.notify();

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_messages(clan_id.get(), channel_id.get(), 0, 0, MESSAGE_PAGE_LIMIT)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_initial_fetch_result(channel_id, generation, result, cx);
            });
        })
        .detach();
    }

    fn set_channel(&mut self, channel_id: ChannelId, messages: Vec<Message>) {
        let active = self.active_channel_id;
        let has_more = has_more_from_oldest(&messages);
        self.cache.insert(
            channel_id,
            ChannelMessages {
                messages,
                has_more,
                // Normal open loads the newest page, so nothing newer exists yet.
                // Jump-to-message will set this when it loads an older window.
                has_more_bottom: false,
            },
            active.as_ref(),
        );
    }
}

/// Whether there is more history above the loaded buffer, mirroring React
/// `hasMore = lastLoadMessage?.code !== EMessageCode.FIRST_MESSAGE`
/// (`messages.slice.ts`). The very first message of a channel carries code 4
/// (`FIRST_MESSAGE`, which we map to `MessageCode::Indicator`); once it is the
/// oldest loaded row there is nothing older to fetch. An empty buffer has
/// nothing more to load.
fn has_more_from_oldest(messages: &[Message]) -> bool {
    messages
        .first()
        .is_some_and(|m| m.code != MessageCode::Indicator)
}

fn prepare_messages(msgs: Vec<ApiMessage>, cfg: Option<&AppConfig>) -> Vec<Message> {
    let mut messages: Vec<Message> = msgs.into_iter().map(|m| message_from_api(m, cfg)).collect();
    sort_messages(&mut messages);
    trim_messages(&mut messages);
    recompute_message_grouping(&mut messages);
    messages
}

fn push_message_grouped(messages: &mut Vec<Message>, msg: Message) {
    // Ordered by id (React id-ascending): a newly received row whose id is >=
    // the current newest goes straight to the back (the common case); otherwise
    // re-sort.
    let in_order = messages
        .last()
        .map(|last| message_sort_key(last) <= message_sort_key(&msg))
        .unwrap_or(true);
    messages.push(msg);
    if in_order {
        trim_messages(messages);
        let last = messages.len() - 1;
        let combined = {
            let prev = last.checked_sub(1).map(|i| &messages[i]);
            message_combined_with_prev(prev, &messages[last])
        };
        messages[last].combined_with_prev = combined;
    } else {
        sort_messages(messages);
        trim_messages(messages);
        recompute_message_grouping(messages);
    }
}

/// Cap the buffer to `MAX_MESSAGES_PER_CHANNEL`, dropping the oldest rows.
/// Returns how many rows were dropped from the front. Used when newer rows are
/// appended (the window slides toward the present).
fn trim_messages(messages: &mut Vec<Message>) -> usize {
    if messages.len() <= MAX_MESSAGES_PER_CHANNEL {
        return 0;
    }
    let drop = messages.len() - MAX_MESSAGES_PER_CHANNEL;
    messages.drain(0..drop);
    drop
}

/// Cap the buffer to `MAX_MESSAGES_PER_CHANNEL`, dropping the newest rows.
/// Returns how many rows were dropped from the back. Used when older rows are
/// prepended (the window slides toward history) so the just-loaded older rows
/// are kept; the dropped newest rows can be re-fetched via `load_more_bottom`.
fn trim_messages_back(messages: &mut Vec<Message>) -> usize {
    if messages.len() <= MAX_MESSAGES_PER_CHANNEL {
        return 0;
    }
    let drop = messages.len() - MAX_MESSAGES_PER_CHANNEL;
    messages.truncate(MAX_MESSAGES_PER_CHANNEL);
    drop
}

fn message_from_api(m: ApiMessage, cfg: Option<&AppConfig>) -> Message {
    let avatar_proxied = cfg
        .map(|c| c.avatar_proxy(&m.avatar))
        .unwrap_or_else(|| m.avatar.clone());
    let spans = parse_spans(&m.content_tokens);
    let references = m
        .references
        .iter()
        .map(|r| message_reference_from_api(r, cfg))
        .collect();
    let reactions = aggregate_reactions(&m.reactions);
    Message::new(
        m.message_id.to_string(),
        m.content,
        m.sender_id.to_string(),
        m.sender_name,
        m.create_time,
    )
    .with_message_id(m.message_id)
    .with_code(MessageCode::from_raw(m.code))
    .with_spans(spans)
    .with_references(references)
    .with_reactions(reactions)
    .with_edited(m.update_time, m.hide_editted)
    .with_avatar(m.avatar)
    .with_avatar_proxied(avatar_proxied)
    .with_attachments(
        m.attachments
            .into_iter()
            .map(|a| MessageAttachment::from_api(a, cfg))
            .collect(),
    )
}

fn message_reference_from_api(
    r: &mezon_client::transport::ApiMessageRef,
    cfg: Option<&AppConfig>,
) -> MessageReference {
    let sender_name = if !r.message_sender_clan_nick.is_empty() {
        r.message_sender_clan_nick.clone()
    } else if !r.message_sender_display_name.is_empty() {
        r.message_sender_display_name.clone()
    } else {
        r.message_sender_username.clone()
    };
    // The reference content is itself a JSON `IExtendedMessage`; extract its text.
    let content = serde_json::from_str::<mezon_client::transport::ApiMessageContent>(&r.content)
        .map(|c| c.t)
        .unwrap_or_else(|_| r.content.clone());
    let sender_avatar = cfg
        .map(|c| c.avatar_proxy(&r.message_sender_avatar))
        .unwrap_or_else(|| r.message_sender_avatar.clone());
    MessageReference {
        message_ref_id: r.message_ref_id.to_string(),
        sender_id: r.message_sender_id.to_string(),
        sender_name,
        sender_avatar,
        content,
        has_attachment: r.has_attachment,
    }
}

impl MessageAttachment {
    pub(crate) fn from_api(
        a: mezon_client::transport::ApiAttachment,
        cfg: Option<&AppConfig>,
    ) -> Self {
        let width = a.width.max(0) as u32;
        let height = a.height.max(0) as u32;
        let (proxied_src, display_width, display_height) = cfg
            .map(|c| c.attachment_proxy(&a.url, width, height))
            .unwrap_or_else(|| {
                let (w, h) = crate::config::attachment_display_dimensions(width, height);
                (a.url.clone(), w, h)
            });
        Self {
            url: a.url,
            filename: a.filename,
            filetype: a.filetype,
            width,
            height,
            proxied_src: proxied_src.into(),
            display_width,
            display_height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageSpan;

    #[test]
    fn outgoing_mention_maps_to_transport_with_utf16_offsets() {
        let mention = OutgoingMention {
            user_id: "42".into(),
            role_id: String::new(),
            display: "@bob".into(),
            s: 2,
            e: 6,
        };
        let transport = mention.into_transport();
        assert_eq!(transport.user_id, "42");
        assert_eq!(transport.username, "@bob");
        assert_eq!(transport.s, 2);
        assert_eq!(transport.e, 6);
    }

    #[test]
    fn sticker_attachment_is_recognized_as_image() {
        let attachment = MessageAttachment::from_api(
            mezon_client::transport::ApiAttachment {
                url: "https://cdn/1.webp".into(),
                filename: "1".into(),
                filetype: STICKER_FILETYPE.into(),
                width: 0,
                height: 0,
            },
            None,
        );
        assert_eq!(attachment.filetype, "sticker");
        assert_eq!(attachment.url, "https://cdn/1.webp");
        assert!(attachment.is_image());
        assert_eq!(
            (attachment.display_width, attachment.display_height),
            (280.0, 150.0)
        );
    }

    #[test]
    fn optimistic_mention_tokens_round_trip_to_a_coloured_span() {
        let mentions = vec![OutgoingMention {
            user_id: "42".into(),
            role_id: String::new(),
            display: "@bob".into(),
            s: 0,
            e: 4,
        }];
        let transport: Vec<TransportMention> = mentions
            .into_iter()
            .map(OutgoingMention::into_transport)
            .collect();
        let tokens = ApiMessageContent {
            t: "@bob hi".into(),
            mentions: mention_content_tokens(&transport),
            ..Default::default()
        };
        let spans = parse_spans(&tokens);
        assert_eq!(
            spans,
            vec![
                MessageSpan::Mention {
                    display: "@bob".into(),
                    user_id: Some("42".into()),
                    role_id: None,
                },
                MessageSpan::Text(" hi".into()),
            ]
        );
    }

    #[test]
    fn message_from_api_maps_fields() {
        let m = message_from_api(
            ApiMessage {
                message_id: 1,
                content: "hi".into(),
                content_tokens: mezon_client::transport::ApiMessageContent {
                    t: "hi".into(),
                    ..Default::default()
                },
                code: 0,
                sender_id: 1,
                sender_name: "Alice".into(),
                avatar: "av.png".into(),
                create_time: 100,
                update_time: 0,
                hide_editted: false,
                attachments: vec![],
                references: vec![],
                reactions: vec![],
            },
            None,
        );
        assert_eq!(m.id, "1");
        assert_eq!(m.content, "hi");
        assert_eq!(m.sender_name, "Alice");
        assert_eq!(m.avatar_url, "av.png");
        assert_eq!(m.avatar_proxied, "av.png");
    }

    #[test]
    fn push_message_grouped_appends_in_order() {
        let mut msgs = vec![
            Message::new("1", "a", "u1", "U1", 100),
            Message::new("2", "b", "u1", "U1", 110),
        ];
        recompute_message_grouping(&mut msgs);
        push_message_grouped(&mut msgs, Message::new("3", "c", "u1", "U1", 120));
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].id, "3");
        assert!(msgs[2].combined_with_prev);
    }

    #[test]
    fn push_message_grouped_resorts_when_out_of_order() {
        let mut msgs = vec![
            Message::new("1", "a", "u1", "U1", 100),
            Message::new("3", "c", "u1", "U1", 120),
        ];
        recompute_message_grouping(&mut msgs);
        push_message_grouped(&mut msgs, Message::new("2", "b", "u1", "U1", 110));
        let ids: Vec<&str> = msgs.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["1", "2", "3"]);
    }

    #[test]
    fn push_message_grouped_breaks_group_for_different_sender() {
        let mut msgs = vec![Message::new("1", "a", "u1", "U1", 100)];
        recompute_message_grouping(&mut msgs);
        push_message_grouped(&mut msgs, Message::new("2", "b", "u2", "U2", 105));
        assert!(!msgs[1].combined_with_prev);
    }

    #[test]
    fn trim_messages_drops_oldest() {
        let mut msgs: Vec<Message> = (0..MAX_MESSAGES_PER_CHANNEL + 5)
            .map(|i| Message::new(i.to_string(), format!("m{i}"), "u", "User", i as i64))
            .collect();
        trim_messages(&mut msgs);
        assert_eq!(msgs.len(), MAX_MESSAGES_PER_CHANNEL);
        assert_eq!(msgs.first().unwrap().id, "5");
        assert_eq!(
            msgs.last().unwrap().id,
            (MAX_MESSAGES_PER_CHANNEL + 4).to_string()
        );
    }

    fn channel_msgs(msgs: Vec<Message>) -> ChannelMessages {
        ChannelMessages {
            messages: msgs,
            has_more: false,
            has_more_bottom: false,
        }
    }

    fn remove_temp_in(ch: &mut ChannelMessages, temp_id: &str) {
        let before = ch.messages.len();
        ch.messages.retain(|m| m.id != temp_id);
        if ch.messages.len() != before {
            recompute_message_grouping(&mut ch.messages);
        }
    }

    fn reconcile_temp_in(ch: &mut ChannelMessages, temp_id: &str, confirmed: Message) {
        if let Some(slot) = ch.messages.iter_mut().find(|m| m.id == temp_id) {
            *slot = confirmed;
        } else if !ch.messages.iter().any(|m| m.id == confirmed.id) {
            ch.messages.push(confirmed);
            sort_messages(&mut ch.messages);
            trim_messages(&mut ch.messages);
            recompute_message_grouping(&mut ch.messages);
        }
    }

    #[test]
    fn remove_temp_drops_message_by_id() {
        let mut ch = channel_msgs(vec![
            Message::new("temp-1", "hello", "u1", "U", 100),
            Message::new("msg-2", "world", "u1", "U", 200),
        ]);
        remove_temp_in(&mut ch, "temp-1");
        assert_eq!(ch.messages.len(), 1);
        assert_eq!(ch.messages[0].id, "msg-2");
    }

    #[test]
    fn remove_temp_noop_when_id_not_found() {
        let mut ch = channel_msgs(vec![Message::new("msg-1", "hello", "u1", "U", 100)]);
        remove_temp_in(&mut ch, "temp-999");
        assert_eq!(ch.messages.len(), 1);
    }

    #[test]
    fn reconcile_temp_matches_only_by_temp_id_not_content() {
        let mut ch = channel_msgs(vec![
            Message::new("temp-1", "same text", "u1", "U", 100),
            Message::new("temp-2", "same text", "u1", "U", 110),
        ]);
        let confirmed = Message::new("server-42", "same text", "u1", "U", 120);
        reconcile_temp_in(&mut ch, "temp-1", confirmed);
        assert_eq!(ch.messages.len(), 2);
        assert_eq!(ch.messages[0].id, "server-42");
        assert_eq!(ch.messages[1].id, "temp-2");
    }
}
