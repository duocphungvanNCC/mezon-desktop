use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};
use mezon_client::transport::ApiChannelDesc;
use mezon_client::{AppApi, RealtimeEvent};

use crate::clan::{ClanEvent, ClanList};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    Text,
    Voice,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub unread: bool,
    pub private: bool,
    pub clan_id: String,
    pub category_name: String,
    pub category_id: Option<String>,
    pub member_count: u32,
}

#[derive(Debug, Clone, Default)]
pub struct MessageAttachment {
    pub url: String,
    pub filename: String,
    pub filetype: String,
    pub width: u32,
    pub height: u32,
}

impl MessageAttachment {
    pub fn is_image(&self) -> bool {
        self.filetype.starts_with("image/")
            || matches!(
                self.url
                    .split(['?', '#'])
                    .next()
                    .and_then(|u| u.rsplit('.').next())
                    .map(|ext| ext.to_ascii_lowercase())
                    .as_deref(),
                Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif")
            )
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub content: String,
    pub sender_id: String,
    pub sender_name: String,
    pub avatar_url: String,
    pub create_time: i64,
    /// Clock label ("3:04 PM") precomputed at insert so `render` never formats.
    pub timestamp_label: String,
    /// Day label ("June 17, 2026") precomputed at insert for day separators.
    pub day_label: String,
    pub combined_with_prev: bool,
    pub reactions: Vec<String>,
    pub attachments: Vec<MessageAttachment>,
}

pub const COMBINE_TIME_WINDOW: i64 = 300;

pub fn message_combined_with_prev(prev: Option<&Message>, msg: &Message) -> bool {
    match prev {
        Some(prev) => {
            prev.sender_id == msg.sender_id
                && prev.day_label == msg.day_label
                && (msg.create_time - prev.create_time).abs() < COMBINE_TIME_WINDOW
        }
        None => false,
    }
}

pub fn recompute_message_grouping(messages: &mut [Message]) {
    for i in 0..messages.len() {
        let prev = if i > 0 { Some(&messages[i - 1]) } else { None };
        messages[i].combined_with_prev = message_combined_with_prev(prev, &messages[i]);
    }
}

impl Message {
    pub fn new(
        id: impl Into<String>,
        content: impl Into<String>,
        sender_id: impl Into<String>,
        sender_name: impl Into<String>,
        create_time: i64,
    ) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            sender_id: sender_id.into(),
            sender_name: sender_name.into(),
            avatar_url: String::new(),
            create_time,
            timestamp_label: format_clock(create_time),
            day_label: format_day(create_time),
            combined_with_prev: false,
            reactions: Vec::new(),
            attachments: Vec::new(),
        }
    }

    pub fn with_attachments(mut self, attachments: Vec<MessageAttachment>) -> Self {
        self.attachments = attachments;
        self
    }

    pub fn with_avatar(mut self, avatar_url: impl Into<String>) -> Self {
        self.avatar_url = avatar_url.into();
        self
    }
}

fn format_clock(ts: i64) -> String {
    let seconds_since_midnight = ts.rem_euclid(86_400);
    let hours = seconds_since_midnight / 3600;
    let minutes = (seconds_since_midnight % 3600) / 60;
    let period = if hours >= 12 { "PM" } else { "AM" };
    let display_hour = if hours == 0 {
        12
    } else if hours > 12 {
        hours - 12
    } else {
        hours
    };
    format!("{display_hour}:{minutes:02} {period}")
}

fn format_day(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%B %d, %Y").to_string())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Category {
    pub clan_id: String,
    pub name: String,
    pub channels: Vec<Channel>,
}

/// Typed events emitted by [`ChannelList`] (cf. Zed's `ChannelEvent`).
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    ActiveChannelChanged(Option<String>),
    /// A channel gained an unread message (server push).
    Unread(String),
}

/// Channel store — owns the channel tree, fetches it per-clan over REST, self-subscribes to
/// realtime channel events, and reacts to the active clan changing.
///
/// Same Zed-`ChannelStore` shape as [`ClanList`]: registered as a [`Global`], an
/// [`EventEmitter`] of [`ChannelEvent`], holding its subscriptions so they cancel on drop.
pub struct ChannelList {
    pub categories: Vec<Category>,
    pub active_channel_id: Option<String>,
    /// Clan whose channels are currently loaded — guards against redundant refetch.
    loaded_clan_id: Option<String>,
    api: Arc<AppApi>,
    _realtime: Task<()>,
    /// Reacts to `ClanList` active-clan changes (cf. Zed store-to-store `cx.subscribe`).
    _clan_sub: Subscription,
}

struct GlobalChannelList(Entity<ChannelList>);
impl Global for GlobalChannelList {}

impl EventEmitter<ChannelEvent> for ChannelList {}

impl ChannelList {
    /// Create the store and register it as the app-wide global. **Requires [`ClanList::init`]
    /// to have run first** (this store subscribes to the clan store's events).
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalChannelList(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalChannelList>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalChannelList>().map(|g| g.0.clone())
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
                                break; // store dropped
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

        // React to the active clan changing — load that clan's channels.
        let clan_sub = cx.subscribe(&ClanList::global(cx), |this, _clan, event, cx| {
            if let ClanEvent::ActiveClanChanged(Some(clan_id)) = event {
                this.load_for_clan(clan_id.clone(), cx);
            }
        });

        Self {
            categories: Vec::new(),
            active_channel_id: None,
            loaded_clan_id: None,
            api,
            _realtime: realtime,
            _clan_sub: clan_sub,
        }
    }

    /// Fetch channels for a clan (REST + DTO mapping + grouping, all owned by the store).
    /// No-op if already loaded for this clan.
    pub fn load_for_clan(&mut self, clan_id: String, cx: &mut Context<Self>) {
        if self.loaded_clan_id.as_deref() == Some(clan_id.as_str()) {
            return;
        }
        self.loaded_clan_id = Some(clan_id.clone());
        let api = self.api.clone();
        cx.spawn(
            async move |this, cx| match api.list_channel_descs(&clan_id).await {
                Ok(api_channels) => {
                    let channels: Vec<Channel> =
                        api_channels.into_iter().map(Channel::from).collect();
                    let categories = group_channels_by_category(channels);
                    let _ = this.update(cx, |this, cx| {
                        this.categories = categories;
                        cx.notify();
                    });
                }
                Err(e) => tracing::error!("Failed to load channels: {e}"),
            },
        )
        .detach();
    }

    /// Apply a server-pushed realtime event. Cf. `ChannelStore::handle_update_channels`.
    fn handle_event(&mut self, event: RealtimeEvent, cx: &mut Context<Self>) {
        match event {
            RealtimeEvent::ChannelMessage(m) => {
                let id = m.channel_id.to_string();
                if self.active_channel_id.as_deref() != Some(id.as_str())
                    && let Some(ch) = self
                        .categories
                        .iter_mut()
                        .flat_map(|c| &mut c.channels)
                        .find(|ch| ch.id == id)
                    && !ch.unread
                {
                    ch.unread = true;
                    cx.emit(ChannelEvent::Unread(id));
                    cx.notify();
                }
            }
            RealtimeEvent::ChannelCreated(e)
                if self.loaded_clan_id.as_deref() == Some(e.clan_id.to_string().as_str()) =>
            {
                self.reload_current_clan(cx);
            }
            RealtimeEvent::ChannelUpdated(e) => {
                let label = (!e.channel_label.is_empty()).then_some(e.channel_label);
                if update_channel(
                    &mut self.categories,
                    &e.channel_id.to_string(),
                    label,
                    e.channel_private,
                ) {
                    cx.notify();
                }
            }
            RealtimeEvent::ChannelDeleted(e) => {
                let id = e.channel_id.to_string();
                if remove_channel(&mut self.categories, &id) {
                    if self.active_channel_id.as_deref() == Some(id.as_str()) {
                        self.active_channel_id = None;
                        cx.emit(ChannelEvent::ActiveChannelChanged(None));
                    }
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn reload_current_clan(&mut self, cx: &mut Context<Self>) {
        if let Some(clan_id) = self.loaded_clan_id.clone() {
            self.loaded_clan_id = None;
            self.load_for_clan(clan_id, cx);
        }
    }

    fn on_realtime_lagged(&mut self, cx: &mut Context<Self>) {
        tracing::warn!("ChannelList realtime lagged — refetching channels");
        self.reload_current_clan(cx);
    }

    pub fn active_channel(&self) -> Option<&Channel> {
        self.active_channel_id
            .as_ref()
            .and_then(|id| self.find_channel(id))
    }

    pub fn categories_for_clan(&self, clan_id: &str) -> Vec<&Category> {
        self.categories
            .iter()
            .filter(|c| c.clan_id == clan_id)
            .collect()
    }

    pub fn select_channel(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.active_channel_id.as_deref() == Some(id) {
            return;
        }
        self.active_channel_id = Some(id.to_string());
        self.mark_read(id);
        cx.emit(ChannelEvent::ActiveChannelChanged(
            self.active_channel_id.clone(),
        ));
        cx.notify();
    }

    pub fn mark_read(&mut self, id: &str) {
        if let Some(ch) = self
            .categories
            .iter_mut()
            .flat_map(|c| &mut c.channels)
            .find(|ch| ch.id == id)
        {
            ch.unread = false;
        }
    }

    pub fn find_channel(&self, channel_id: &str) -> Option<&Channel> {
        self.categories
            .iter()
            .flat_map(|category| &category.channels)
            .find(|channel| channel.id == channel_id)
    }
}

impl From<ApiChannelDesc> for Channel {
    fn from(c: ApiChannelDesc) -> Self {
        Self {
            id: c.channel_id,
            name: c.channel_label,
            channel_type: ChannelType::Text,
            unread: c.count_mess_unread > 0,
            private: c.channel_private != 0,
            clan_id: c.clan_id,
            category_name: c.category_name,
            category_id: Some(c.category_id).filter(|s| !s.is_empty() && s != "0"),
            member_count: c.member_count as u32,
        }
    }
}

/// Group flat channels into categories by `category_name` (DTO/shape logic belongs in the
/// store, not in views). Channels with an empty `category_name` go into a "General" category.
fn group_channels_by_category(channels: Vec<Channel>) -> Vec<Category> {
    let mut map: HashMap<String, Vec<Channel>> = HashMap::new();

    for ch in channels {
        let cat_name = if ch.category_name.is_empty() {
            "General".to_string()
        } else {
            ch.category_name.clone()
        };
        map.entry(cat_name).or_default().push(ch);
    }

    let mut categories: Vec<Category> = map
        .into_iter()
        .map(|(name, chs)| {
            let clan_id = chs.first().map(|ch| ch.clan_id.clone()).unwrap_or_default();
            Category {
                clan_id,
                name,
                channels: chs,
            }
        })
        .collect();

    categories.sort_by(|a, b| a.name.cmp(&b.name));
    categories
}

fn remove_channel(categories: &mut Vec<Category>, channel_id: &str) -> bool {
    let mut removed = false;
    for cat in categories.iter_mut() {
        let before = cat.channels.len();
        cat.channels.retain(|ch| ch.id != channel_id);
        removed |= cat.channels.len() != before;
    }
    if removed {
        categories.retain(|c| !c.channels.is_empty());
    }
    removed
}

fn update_channel(
    categories: &mut [Category],
    channel_id: &str,
    label: Option<String>,
    private: bool,
) -> bool {
    let Some(ch) = categories
        .iter_mut()
        .flat_map(|c| &mut c.channels)
        .find(|ch| ch.id == channel_id)
    else {
        return false;
    };
    if let Some(label) = label {
        ch.name = label;
    }
    ch.private = private;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: &str, name: &str) -> Channel {
        Channel {
            id: id.into(),
            name: name.into(),
            channel_type: ChannelType::Text,
            unread: false,
            private: false,
            clan_id: "1".into(),
            category_name: "General".into(),
            category_id: None,
            member_count: 0,
        }
    }

    fn categories() -> Vec<Category> {
        vec![Category {
            clan_id: "1".into(),
            name: "General".into(),
            channels: vec![channel("10", "alpha"), channel("11", "beta")],
        }]
    }

    #[test]
    fn remove_channel_drops_it_and_prunes_empty_category() {
        let mut c = categories();
        assert!(remove_channel(&mut c, "10"));
        assert_eq!(c[0].channels.len(), 1);
        assert!(remove_channel(&mut c, "11"));
        assert!(c.is_empty());
    }

    #[test]
    fn remove_channel_unknown_is_noop() {
        let mut c = categories();
        assert!(!remove_channel(&mut c, "999"));
        assert_eq!(c[0].channels.len(), 2);
    }

    #[test]
    fn update_channel_renames_and_sets_private() {
        let mut c = categories();
        assert!(update_channel(&mut c, "10", Some("renamed".into()), true));
        assert_eq!(c[0].channels[0].name, "renamed");
        assert!(c[0].channels[0].private);
    }

    #[test]
    fn update_channel_blank_label_keeps_name() {
        let mut c = categories();
        assert!(update_channel(&mut c, "11", None, true));
        assert_eq!(c[0].channels[1].name, "beta");
        assert!(c[0].channels[1].private);
    }

    #[test]
    fn update_channel_unknown_is_noop() {
        let mut c = categories();
        assert!(!update_channel(&mut c, "999", Some("x".into()), true));
    }

    #[test]
    fn message_precomputes_clock_and_day_labels() {
        // 2021-01-01 00:00:00 UTC = 1_609_459_200; +13h25m = +48_300s.
        let msg = Message::new("1", "hi", "u", "User", 1_609_459_200 + 48_300);
        assert_eq!(msg.timestamp_label, "1:25 PM");
        assert_eq!(msg.day_label, "January 01, 2021");
    }

    #[test]
    fn message_clock_label_handles_midnight() {
        let msg = Message::new("1", "hi", "u", "User", 1_609_459_200);
        assert_eq!(msg.timestamp_label, "12:00 AM");
    }
}
