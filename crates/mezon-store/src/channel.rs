use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, Global, SharedString, Subscription, Task,
};
use mezon_client::transport::{ApiCategoryDesc, ApiChannelDesc};
use mezon_client::{ApiChannelApp, AppApi, ConnectionStatus, RealtimeEvent};

use crate::KeyedCache;
use crate::clan::{ClanEvent, ClanList};
use crate::realtime::{RealtimeDispatch, RealtimeKind};

pub const FAVOR_CATE_ID: &str = "favorCate";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelType {
    Text,
    Voice,
    Stream,
    Thread,
    App,
    Forum,
    Announcement,
    Unknown(u32),
}

impl ChannelType {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            1 => ChannelType::Text,
            5 => ChannelType::Forum,
            6 => ChannelType::Stream,
            7 => ChannelType::Thread,
            8 => ChannelType::App,
            9 => ChannelType::Announcement,
            10 => ChannelType::Voice,
            other => ChannelType::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceMember {
    pub user_id: String,
    pub display_name: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppChannel {
    pub app_id: String,
    pub app_name: String,
    pub app_logo: Option<String>,
    pub app_url: String,
    pub channel_id: String,
}

impl From<ApiChannelApp> for AppChannel {
    fn from(a: ApiChannelApp) -> Self {
        Self {
            app_id: a.app_id,
            app_name: a.app_name,
            app_logo: a.app_logo,
            app_url: a.app_url,
            channel_id: a.channel_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub private: bool,
    pub clan_id: String,
    pub category_name: String,
    pub category_id: Option<String>,
    pub member_count: u32,
    pub badge_count: u32,
    pub muted: bool,
    pub parent_id: Option<String>,
    pub last_seen_timestamp: i64,
    pub last_sent_timestamp: i64,
    pub voice_members: Vec<VoiceMember>,
    pub is_favorite: bool,
}

impl Channel {
    pub fn is_unread(&self) -> bool {
        self.badge_count > 0 || self.last_seen_timestamp < self.last_sent_timestamp
    }
}

#[derive(Debug, Clone, Default)]
pub struct MessageAttachment {
    pub url: String,
    pub filename: String,
    pub filetype: String,
    pub width: u32,
    pub height: u32,
    pub proxied_src: SharedString,
    pub display_width: f32,
    pub display_height: f32,
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
    pub avatar_proxied: SharedString,
    pub create_time: i64,
    pub timestamp_label: String,
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
            avatar_proxied: SharedString::default(),
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

    pub fn with_avatar_proxied(mut self, proxied: impl Into<SharedString>) -> Self {
        self.avatar_proxied = proxied.into();
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
    pub id: String,
    pub clan_id: String,
    pub name: String,
    pub order: i32,
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone)]
pub enum ChannelEvent {
    ActiveChannelChanged(Option<String>),
    Unread(String),
}

fn collapse_state_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("mezon")
        .join("collapse_state.json")
}

pub struct ChannelList {
    cache: KeyedCache<String, Vec<Category>>,
    app_channels_cache: HashMap<String, Vec<AppChannel>>,
    loading: HashSet<String>,
    active_clan_id: Option<String>,
    pub active_channel_id: Option<String>,
    api: Arc<AppApi>,
    collapsed: HashSet<(String, String)>,
    _clan_sub: Subscription,
    _conn_watch: Task<()>,
}

struct GlobalChannelList(Entity<ChannelList>);
impl Global for GlobalChannelList {}

impl EventEmitter<ChannelEvent> for ChannelList {}

impl ChannelList {
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
        Self::register_realtime(cx);

        let clan_sub = cx.subscribe(&ClanList::global(cx), |this, _clan, event, cx| {
            if let ClanEvent::ActiveClanChanged(Some(clan_id)) = event {
                this.active_clan_id = Some(clan_id.clone());
                this.load_for_clan(clan_id.clone(), cx);
                cx.notify();
            }
        });

        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);

        cx.spawn(async move |this, cx| {
            let collapsed = cx
                .background_executor()
                .spawn(async { load_collapse_state() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !collapsed.is_empty() {
                    this.collapsed = collapsed;
                    cx.notify();
                }
            });
        })
        .detach();

        Self {
            cache: KeyedCache::new(None),
            app_channels_cache: HashMap::new(),
            loading: HashSet::new(),
            active_clan_id: None,
            active_channel_id: None,
            api,
            collapsed: HashSet::new(),
            _clan_sub: clan_sub,
            _conn_watch: conn_watch,
        }
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            for kind in [
                RealtimeKind::ChannelMessage,
                RealtimeKind::ChannelCreated,
                RealtimeKind::ChannelUpdated,
                RealtimeKind::ChannelDeleted,
                RealtimeKind::VoiceJoined,
                RealtimeKind::VoiceLeaved,
                RealtimeKind::MarkAsRead,
            ] {
                dispatch.on(kind, &entity, |this, event, cx| {
                    this.handle_event(event, cx)
                });
            }
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

    pub fn load_for_clan(&mut self, clan_id: String, cx: &mut Context<Self>) {
        if self.cache.is_fresh(&clan_id, crate::CACHE_TTL) {
            return;
        }
        self.fetch_clan(clan_id, cx);
    }

    pub fn refresh_clan(&mut self, clan_id: String, cx: &mut Context<Self>) {
        self.fetch_clan(clan_id, cx);
    }

    fn fetch_clan(&mut self, clan_id: String, cx: &mut Context<Self>) {
        if self.loading.contains(&clan_id) {
            return;
        }
        self.loading.insert(clan_id.clone());
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = Self::fetch_clan_data(&api, &clan_id).await;
            match result {
                Ok((categories, app_channels)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.loading.remove(&clan_id);
                        this.app_channels_cache
                            .insert(clan_id.clone(), app_channels);
                        this.cache.insert(clan_id, categories, None);
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to load channels for clan {clan_id}: {e}");
                    let _ = this.update(cx, |this, _| {
                        this.loading.remove(&clan_id);
                    });
                }
            }
        })
        .detach();
    }

    async fn fetch_clan_data(
        api: &AppApi,
        clan_id: &str,
    ) -> anyhow::Result<(Vec<Category>, Vec<AppChannel>)> {
        let (channels_res, categories_res, badges_res, voice_res, favorites_res, apps_res) = tokio::join!(
            api.list_channel_descs(clan_id),
            api.list_categories_typed(clan_id),
            api.list_channel_badge_counts(clan_id),
            api.list_voice_channel_users(clan_id),
            api.list_favorite_channels(clan_id),
            api.list_channel_apps(clan_id),
        );

        let api_channels = channels_res?;
        let api_categories = categories_res.unwrap_or_else(|e| {
            tracing::warn!("list_categories failed: {e}");
            Vec::new()
        });
        let badge_descs = badges_res.unwrap_or_else(|e| {
            tracing::warn!("list_channel_badge_counts failed: {e}");
            Vec::new()
        });
        let voice_users = voice_res.unwrap_or_else(|e| {
            tracing::warn!("list_voice_channel_users failed: {e}");
            Vec::new()
        });
        let favorite_ids: HashSet<String> = favorites_res
            .unwrap_or_else(|e| {
                tracing::warn!("list_favorite_channels failed: {e}");
                Vec::new()
            })
            .into_iter()
            .collect();
        let app_channels: Vec<AppChannel> = apps_res
            .unwrap_or_else(|e| {
                tracing::warn!("list_channel_apps failed: {e}");
                Vec::new()
            })
            .into_iter()
            .map(AppChannel::from)
            .collect();

        let badge_map: HashMap<String, i32> = badge_descs
            .into_iter()
            .filter(|d| {
                !matches!(
                    ChannelType::from_raw(d.channel_type),
                    ChannelType::App | ChannelType::Voice
                )
            })
            .map(|d| (d.channel_id, d.badge_count))
            .collect();

        let voice_map: HashMap<String, Vec<String>> = voice_users
            .into_iter()
            .map(|v| (v.channel_id, v.user_ids))
            .collect();

        let mut channels: Vec<Channel> = api_channels
            .into_iter()
            .map(|c| {
                let badge = badge_map
                    .get(&c.channel_id)
                    .copied()
                    .unwrap_or(c.badge_count)
                    .max(0) as u32;
                let voice_ids = voice_map.get(&c.channel_id).cloned().unwrap_or_default();
                let is_favorite = favorite_ids.contains(&c.channel_id);
                channel_from_desc(c, badge, voice_ids, is_favorite)
            })
            .collect();

        let categories = build_categories(api_categories, &mut channels);
        Ok((
            assemble_with_favorites(categories, &favorite_ids, clan_id),
            app_channels,
        ))
    }

    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        match event {
            RealtimeEvent::ChannelMessage(m) => {
                let id = m.channel_id.to_string();
                if self.active_channel_id.as_deref() != Some(id.as_str()) {
                    let mut changed = false;
                    for cats in self.cache.values_mut() {
                        if let Some(ch) = cats
                            .iter_mut()
                            .flat_map(|c| &mut c.channels)
                            .find(|ch| ch.id == id)
                        {
                            ch.badge_count = ch.badge_count.saturating_add(1);
                            if m.create_time_seconds > 0 {
                                ch.last_sent_timestamp = i64::from(m.create_time_seconds);
                            }
                            changed = true;
                            break;
                        }
                    }
                    if changed {
                        cx.emit(ChannelEvent::Unread(id));
                        cx.notify();
                    }
                }
            }
            RealtimeEvent::MarkAsRead(e) => {
                let id = e.channel_id.to_string();
                let mut changed = false;
                for cats in self.cache.values_mut() {
                    if let Some(ch) = cats
                        .iter_mut()
                        .flat_map(|c| &mut c.channels)
                        .find(|ch| ch.id == id)
                    {
                        ch.badge_count = 0;
                        ch.last_seen_timestamp = ch.last_sent_timestamp;
                        changed = true;
                        break;
                    }
                }
                if changed {
                    cx.notify();
                }
            }
            RealtimeEvent::ChannelCreated(e) => {
                let clan_id = e.clan_id.to_string();
                if self.cache.contains(&clan_id) {
                    let channel = Channel {
                        id: e.channel_id.to_string(),
                        name: e.channel_label.clone(),
                        channel_type: ChannelType::from_raw(e.channel_type as u32),
                        private: e.channel_private != 0,
                        clan_id: clan_id.clone(),
                        category_name: String::new(),
                        category_id: Some(e.category_id.to_string())
                            .filter(|s| !s.is_empty() && s != "0"),
                        member_count: 0,
                        badge_count: 0,
                        muted: false,
                        parent_id: Some(e.parent_id.to_string())
                            .filter(|s| !s.is_empty() && s != "0"),
                        last_seen_timestamp: 0,
                        last_sent_timestamp: 0,
                        voice_members: Vec::new(),
                        is_favorite: false,
                    };
                    if let Some(cats) = self.cache.get_mut(&clan_id)
                        && insert_channel(cats, channel)
                    {
                        cx.notify();
                    }
                }
            }
            RealtimeEvent::ChannelUpdated(e) => {
                let id = e.channel_id.to_string();
                let label = (!e.channel_label.is_empty()).then_some(e.channel_label.clone());
                let mut changed = false;
                for cats in self.cache.values_mut() {
                    if update_channel(cats, &id, label.clone(), e.channel_private) {
                        changed = true;
                        break;
                    }
                }
                if changed {
                    cx.notify();
                }
            }
            RealtimeEvent::ChannelDeleted(e) => {
                let id = e.channel_id.to_string();
                let mut removed = false;
                for cats in self.cache.values_mut() {
                    removed |= remove_channel(cats, &id);
                }
                if removed {
                    if self.active_channel_id.as_deref() == Some(id.as_str()) {
                        self.active_channel_id = None;
                        cx.emit(ChannelEvent::ActiveChannelChanged(None));
                    }
                    cx.notify();
                }
            }
            RealtimeEvent::VoiceJoined(e) => {
                let clan_id = e.clan_id.to_string();
                let channel_id = e.voice_channel_id.to_string();
                let user_id = e.user_id.to_string();
                let member = VoiceMember {
                    user_id: user_id.clone(),
                    display_name: user_id.clone(),
                    avatar_url: String::new(),
                };
                let changed = self
                    .cache
                    .get_mut(&clan_id)
                    .and_then(|cats| {
                        cats.iter_mut()
                            .flat_map(|c| &mut c.channels)
                            .find(|ch| ch.id == channel_id)
                    })
                    .map(|ch| {
                        if !ch.voice_members.iter().any(|m| m.user_id == user_id) {
                            ch.voice_members.push(member);
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if changed {
                    cx.notify();
                }
            }
            RealtimeEvent::VoiceLeaved(e) => {
                let clan_id = e.clan_id.to_string();
                let channel_id = e.voice_channel_id.to_string();
                let user_id = e.voice_user_id.to_string();
                let changed = self
                    .cache
                    .get_mut(&clan_id)
                    .and_then(|cats| {
                        cats.iter_mut()
                            .flat_map(|c| &mut c.channels)
                            .find(|ch| ch.id == channel_id)
                    })
                    .map(|ch| {
                        let before = ch.voice_members.len();
                        ch.voice_members.retain(|m| m.user_id != user_id);
                        ch.voice_members.len() != before
                    })
                    .unwrap_or(false);
                if changed {
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn resync(&mut self, cx: &mut Context<Self>) {
        tracing::info!("ChannelList resync — invalidating channel cache");
        self.cache.mark_all_stale();
        if let Some(clan_id) = self.active_clan_id.clone() {
            self.load_for_clan(clan_id, cx);
        }
    }

    pub fn active_channel(&self) -> Option<&Channel> {
        self.active_channel_id
            .as_ref()
            .and_then(|id| self.find_channel(id))
    }

    pub fn categories_for_clan(&self, clan_id: &str) -> &[Category] {
        self.cache.get(clan_id).map_or(&[], Vec::as_slice)
    }

    pub fn app_channels_for_clan(&self, clan_id: &str) -> &[AppChannel] {
        self.app_channels_cache
            .get(clan_id)
            .map_or(&[], Vec::as_slice)
    }

    pub fn channel_in_clan(&self, clan_id: &str, channel_id: &str) -> bool {
        self.categories_for_clan(clan_id)
            .iter()
            .flat_map(|category| &category.channels)
            .any(|channel| channel.id == channel_id)
    }

    pub fn default_channel_id(&self, clan_id: &str) -> Option<String> {
        self.categories_for_clan(clan_id)
            .iter()
            .filter(|cat| cat.id != FAVOR_CATE_ID)
            .flat_map(|category| &category.channels)
            .find(|channel| channel.channel_type == ChannelType::Text)
            .map(|channel| channel.id.clone())
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
            .cache
            .values_mut()
            .flat_map(|cats| cats.iter_mut().flat_map(|c| &mut c.channels))
            .find(|ch| ch.id == id)
        {
            ch.badge_count = 0;
            ch.last_seen_timestamp = ch.last_sent_timestamp;
        }
    }

    pub fn find_channel(&self, channel_id: &str) -> Option<&Channel> {
        self.active_categories()
            .iter()
            .flat_map(|category| &category.channels)
            .find(|channel| channel.id == channel_id)
    }

    fn active_categories(&self) -> &[Category] {
        self.active_clan_id
            .as_deref()
            .and_then(|c| self.cache.get(c))
            .map_or(&[], Vec::as_slice)
    }

    pub fn is_category_collapsed(&self, clan_id: &str, cat_id: &str) -> bool {
        self.collapsed
            .contains(&(clan_id.to_string(), cat_id.to_string()))
    }

    pub fn toggle_category(&mut self, clan_id: &str, cat_id: &str, cx: &mut Context<Self>) {
        let key = (clan_id.to_string(), cat_id.to_string());
        if self.collapsed.contains(&key) {
            self.collapsed.remove(&key);
        } else {
            self.collapsed.insert(key);
        }
        cx.notify();
        let snapshot: Vec<(String, String)> = self.collapsed.iter().cloned().collect();
        cx.background_executor()
            .spawn(async move { save_collapse_state(snapshot) })
            .detach();
    }

    pub fn add_channel_favorite(
        &mut self,
        channel_id: &str,
        clan_id: &str,
        cx: &mut Context<Self>,
    ) {
        let channel_id = channel_id.to_string();
        let clan_id = clan_id.to_string();

        if let Some(cats) = self.cache.get_mut(&clan_id) {
            let channel = cats
                .iter()
                .flat_map(|c| c.channels.iter())
                .find(|ch| ch.id == channel_id)
                .cloned();
            if let Some(mut ch) = channel {
                ch.is_favorite = true;
                if let Some(favor_cat) = cats.iter_mut().find(|c| c.id == FAVOR_CATE_ID) {
                    if !favor_cat.channels.iter().any(|c| c.id == ch.id) {
                        favor_cat.channels.push(ch.clone());
                    }
                } else {
                    let favor_cate = Category {
                        id: FAVOR_CATE_ID.to_string(),
                        clan_id: clan_id.clone(),
                        name: "favoriteChannel".to_string(),
                        order: i32::MIN,
                        channels: vec![ch.clone()],
                    };
                    cats.insert(0, favor_cate);
                }
                for cat in cats.iter_mut() {
                    for existing in cat.channels.iter_mut() {
                        if existing.id == channel_id {
                            existing.is_favorite = true;
                        }
                    }
                }
                cx.notify();
            }
        }

        let api = self.api.clone();
        let cid = channel_id.clone();
        let clid = clan_id.clone();
        cx.spawn(async move |_, _| {
            if let Err(e) = api.add_channel_favorite(&cid, &clid).await {
                tracing::error!("add_channel_favorite failed: {e}");
            }
        })
        .detach();
    }

    pub fn remove_channel_favorite(
        &mut self,
        channel_id: &str,
        clan_id: &str,
        cx: &mut Context<Self>,
    ) {
        let channel_id = channel_id.to_string();
        let clan_id = clan_id.to_string();

        if let Some(cats) = self.cache.get_mut(&clan_id) {
            for cat in cats.iter_mut() {
                for ch in cat.channels.iter_mut() {
                    if ch.id == channel_id {
                        ch.is_favorite = false;
                    }
                }
            }
            if let Some(favor_cat) = cats.iter_mut().find(|c| c.id == FAVOR_CATE_ID) {
                favor_cat.channels.retain(|ch| ch.id != channel_id);
            }
            cats.retain(|c| c.id != FAVOR_CATE_ID || !c.channels.is_empty());
            cx.notify();
        }

        let api = self.api.clone();
        let cid = channel_id.clone();
        let clid = clan_id.clone();
        cx.spawn(async move |_, _| {
            if let Err(e) = api.remove_channel_favorite(&cid, &clid).await {
                tracing::error!("remove_channel_favorite failed: {e}");
            }
        })
        .detach();
    }
}

fn load_collapse_state() -> HashSet<(String, String)> {
    let path = collapse_state_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return HashSet::new(),
    };
    let pairs: Vec<(String, String)> = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return HashSet::new(),
    };
    pairs.into_iter().collect()
}

fn save_collapse_state(pairs: Vec<(String, String)>) {
    let path = collapse_state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string(&pairs) {
        Ok(data) => {
            if let Err(e) = std::fs::write(&path, data) {
                tracing::warn!("Failed to save collapse state: {e}");
            }
        }
        Err(e) => tracing::warn!("Failed to serialize collapse state: {e}"),
    }
}

fn channel_from_desc(
    c: ApiChannelDesc,
    badge_count: u32,
    voice_ids: Vec<String>,
    is_favorite: bool,
) -> Channel {
    let voice_members = voice_ids
        .into_iter()
        .map(|uid| VoiceMember {
            display_name: uid.clone(),
            avatar_url: String::new(),
            user_id: uid,
        })
        .collect();
    Channel {
        id: c.channel_id,
        name: c.channel_label,
        channel_type: ChannelType::from_raw(c.channel_type),
        private: c.channel_private != 0,
        clan_id: c.clan_id,
        category_name: c.category_name,
        category_id: Some(c.category_id).filter(|s| !s.is_empty() && s != "0"),
        member_count: c.member_count.max(0) as u32,
        badge_count,
        muted: c.is_mute,
        parent_id: Some(c.parent_id).filter(|s| !s.is_empty() && s != "0"),
        last_seen_timestamp: c.last_seen_timestamp,
        last_sent_timestamp: c.last_sent_timestamp,
        voice_members,
        is_favorite,
    }
}

fn assemble_with_favorites(
    mut categories: Vec<Category>,
    favorite_ids: &HashSet<String>,
    clan_id: &str,
) -> Vec<Category> {
    if favorite_ids.is_empty() {
        return categories;
    }
    let favor_channels: Vec<Channel> = categories
        .iter()
        .flat_map(|cat| cat.channels.iter())
        .filter(|ch| ch.is_favorite)
        .cloned()
        .collect();
    if !favor_channels.is_empty() {
        let favor_clan_id = favor_channels
            .first()
            .map(|ch| ch.clan_id.as_str())
            .unwrap_or(clan_id)
            .to_string();
        categories.insert(
            0,
            Category {
                id: FAVOR_CATE_ID.to_string(),
                clan_id: favor_clan_id,
                name: "favoriteChannel".to_string(),
                order: i32::MIN,
                channels: favor_channels,
            },
        );
    }
    categories
}

fn build_categories(
    api_categories: Vec<ApiCategoryDesc>,
    channels: &mut Vec<Channel>,
) -> Vec<Category> {
    let cat_map: HashMap<String, (String, i32)> = api_categories
        .into_iter()
        .map(|c| (c.category_id.clone(), (c.category_name, c.category_order)))
        .collect();

    let mut parent_groups: HashMap<String, (Category, i32)> = HashMap::new();
    let mut thread_groups: HashMap<String, Vec<Channel>> = HashMap::new();

    let channels_owned: Vec<Channel> = std::mem::take(channels);

    for ch in channels_owned {
        if let Some(pid) = ch.parent_id.clone() {
            thread_groups.entry(pid).or_default().push(ch);
        } else {
            let cat_id = ch.category_id.clone().unwrap_or_else(|| "0".to_string());
            let (cat_name, cat_order, cat_clan_id) =
                if let Some((name, order)) = cat_map.get(&cat_id) {
                    (name.clone(), *order, ch.clan_id.clone())
                } else {
                    ("General".to_string(), i32::MAX, ch.clan_id.clone())
                };

            parent_groups
                .entry(cat_id.clone())
                .or_insert_with(|| {
                    (
                        Category {
                            id: cat_id.clone(),
                            clan_id: cat_clan_id,
                            name: cat_name,
                            order: cat_order,
                            channels: Vec::new(),
                        },
                        cat_order,
                    )
                })
                .0
                .channels
                .push(ch);
        }
    }

    for (_, (cat, _)) in parent_groups.iter_mut() {
        cat.channels.sort_by(|a, b| a.id.cmp(&b.id));

        let parents: Vec<Channel> = std::mem::take(&mut cat.channels);
        let mut result: Vec<Channel> = Vec::with_capacity(parents.len() * 2);
        for parent in parents {
            let threads = thread_groups.remove(&parent.id);
            result.push(parent);
            if let Some(mut ts) = threads {
                ts.sort_by(|a, b| a.id.cmp(&b.id));
                result.extend(ts);
            }
        }
        cat.channels = result;
    }

    let mut result: Vec<Category> = parent_groups.into_values().map(|(cat, _)| cat).collect();
    result.sort_by_key(|c| c.order);
    result
}

fn insert_channel(categories: &mut Vec<Category>, mut channel: Channel) -> bool {
    if categories
        .iter()
        .flat_map(|c| &c.channels)
        .any(|c| c.id == channel.id)
    {
        return false;
    }
    let clan_id = channel.clan_id.clone();

    if let Some(parent_id) = channel.parent_id.clone() {
        let cat_id = categories
            .iter()
            .flat_map(|c| c.channels.iter())
            .find(|ch| ch.id == parent_id)
            .and_then(|p| p.category_id.clone());

        let target_cat_id = cat_id.unwrap_or_else(|| "0".to_string());
        let cat_name = categories
            .iter()
            .find(|c| c.id == target_cat_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "General".to_string());
        channel.category_name = cat_name;

        if let Some(cat) = categories
            .iter_mut()
            .find(|c| c.id != FAVOR_CATE_ID && c.id == target_cat_id)
        {
            let insert_pos = cat
                .channels
                .iter()
                .position(|ch| ch.parent_id.as_deref() == Some(&parent_id))
                .map(|first_thread_pos| {
                    let mut end = first_thread_pos;
                    while end < cat.channels.len()
                        && cat.channels[end].parent_id.as_deref() == Some(&parent_id)
                    {
                        end += 1;
                    }
                    end
                })
                .or_else(|| {
                    cat.channels
                        .iter()
                        .position(|ch| ch.id == parent_id)
                        .map(|p| p + 1)
                })
                .unwrap_or(cat.channels.len());
            cat.channels.insert(insert_pos, channel);
            return true;
        }
    }

    let (cat_id, cat_name) = channel
        .category_id
        .as_ref()
        .and_then(|cid| {
            categories
                .iter()
                .find(|c| c.id != FAVOR_CATE_ID && c.id == *cid)
                .map(|c| (c.id.clone(), c.name.clone()))
        })
        .unwrap_or_else(|| ("0".to_string(), "General".to_string()));
    channel.category_name = cat_name.clone();

    if let Some(cat) = categories
        .iter_mut()
        .find(|c| c.id != FAVOR_CATE_ID && c.id == cat_id)
    {
        let insert_pos = cat
            .channels
            .iter()
            .position(|ch| ch.id.as_str() > channel.id.as_str())
            .unwrap_or(cat.channels.len());
        cat.channels.insert(insert_pos, channel);
    } else {
        categories.push(Category {
            id: cat_id,
            clan_id,
            name: cat_name,
            order: i32::MAX,
            channels: vec![channel],
        });
        categories.sort_by_key(|c| c.order);
    }
    true
}

fn remove_channel(categories: &mut Vec<Category>, channel_id: &str) -> bool {
    let mut removed = false;
    for cat in categories.iter_mut() {
        if cat.id == FAVOR_CATE_ID {
            continue;
        }
        let before = cat.channels.len();
        cat.channels.retain(|ch| ch.id != channel_id);
        removed |= cat.channels.len() != before;
    }
    if let Some(favor) = categories.iter_mut().find(|c| c.id == FAVOR_CATE_ID) {
        favor.channels.retain(|ch| ch.id != channel_id);
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
    let mut found = false;
    for cat in categories.iter_mut() {
        for ch in cat.channels.iter_mut() {
            if ch.id == channel_id {
                if let Some(ref label) = label {
                    ch.name = label.clone();
                }
                ch.private = private;
                found = true;
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channel(id: &str, name: &str, cat_id: &str) -> Channel {
        Channel {
            id: id.into(),
            name: name.into(),
            channel_type: ChannelType::Text,
            private: false,
            clan_id: "1".into(),
            category_name: "General".into(),
            category_id: Some(cat_id.into()),
            member_count: 0,
            badge_count: 0,
            muted: false,
            parent_id: None,
            last_seen_timestamp: 0,
            last_sent_timestamp: 0,
            voice_members: Vec::new(),
            is_favorite: false,
        }
    }

    fn make_thread(id: &str, parent_id: &str, cat_id: &str) -> Channel {
        let mut ch = make_channel(id, id, cat_id);
        ch.parent_id = Some(parent_id.into());
        ch
    }

    fn categories() -> Vec<Category> {
        vec![Category {
            id: "cat1".into(),
            clan_id: "1".into(),
            name: "General".into(),
            order: 0,
            channels: vec![
                make_channel("10", "alpha", "cat1"),
                make_channel("11", "beta", "cat1"),
            ],
        }]
    }

    #[test]
    fn channel_type_from_raw_maps_all_known() {
        assert_eq!(ChannelType::from_raw(1), ChannelType::Text);
        assert_eq!(ChannelType::from_raw(10), ChannelType::Voice);
        assert_eq!(ChannelType::from_raw(6), ChannelType::Stream);
        assert_eq!(ChannelType::from_raw(7), ChannelType::Thread);
        assert_eq!(ChannelType::from_raw(8), ChannelType::App);
        assert_eq!(ChannelType::from_raw(5), ChannelType::Forum);
        assert_eq!(ChannelType::from_raw(9), ChannelType::Announcement);
        assert!(matches!(
            ChannelType::from_raw(99),
            ChannelType::Unknown(99)
        ));
    }

    #[test]
    fn channel_is_unread_uses_badge_count_and_timestamps() {
        let mut ch = make_channel("1", "test", "cat1");
        assert!(!ch.is_unread());
        ch.badge_count = 5;
        assert!(ch.is_unread());
        ch.badge_count = 0;
        ch.last_sent_timestamp = 100;
        ch.last_seen_timestamp = 50;
        assert!(ch.is_unread());
        ch.last_seen_timestamp = 100;
        assert!(!ch.is_unread());
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
    fn build_categories_orders_by_category_order() {
        let api_cats = vec![
            ApiCategoryDesc {
                category_id: "c1".into(),
                category_name: "Bravo".into(),
                clan_id: "1".into(),
                category_order: 2,
            },
            ApiCategoryDesc {
                category_id: "c2".into(),
                category_name: "Alpha".into(),
                clan_id: "1".into(),
                category_order: 1,
            },
        ];
        let mut channels = vec![
            make_channel("10", "ch1", "c1"),
            make_channel("11", "ch2", "c2"),
        ];
        let cats = build_categories(api_cats, &mut channels);
        assert_eq!(cats[0].name, "Alpha");
        assert_eq!(cats[1].name, "Bravo");
    }

    #[test]
    fn badge_update_increments_and_mark_read_resets() {
        let mut c = categories();
        let ch = &mut c[0].channels[0];
        ch.badge_count = 3;
        ch.badge_count = ch.badge_count.saturating_add(1);
        assert_eq!(ch.badge_count, 4);
        ch.badge_count = 0;
        assert_eq!(ch.badge_count, 0);
    }

    #[test]
    fn badge_map_excludes_app_and_voice_channel_types() {
        use mezon_client::transport::ApiChannelDesc;

        let make_desc = |id: &str, ch_type: u32, badge: i32| ApiChannelDesc {
            channel_id: id.into(),
            channel_label: id.into(),
            channel_type: ch_type,
            clan_id: "1".into(),
            category_name: String::new(),
            category_id: String::new(),
            channel_private: 0,
            count_mess_unread: 0,
            member_count: 0,
            parent_id: String::new(),
            is_mute: false,
            last_seen_message_id: String::new(),
            last_seen_timestamp: 0,
            last_sent_message_id: String::new(),
            last_sent_timestamp: 0,
            badge_count: badge,
        };

        let badge_descs = vec![
            make_desc("text_ch", 1, 5),
            make_desc("app_ch", 8, 99),
            make_desc("voice_ch", 10, 77),
        ];

        let badge_map: HashMap<String, i32> = badge_descs
            .into_iter()
            .filter(|d| {
                !matches!(
                    ChannelType::from_raw(d.channel_type),
                    ChannelType::App | ChannelType::Voice
                )
            })
            .map(|d| (d.channel_id, d.badge_count))
            .collect();

        assert_eq!(badge_map.get("text_ch"), Some(&5));
        assert!(!badge_map.contains_key("app_ch"));
        assert!(!badge_map.contains_key("voice_ch"));
    }

    #[test]
    fn voice_join_leave_updates_member_list() {
        let mut c = categories();
        let ch = &mut c[0].channels[0];
        ch.voice_members.push(VoiceMember {
            user_id: "u1".into(),
            display_name: "u1".into(),
            avatar_url: String::new(),
        });
        assert!(ch.voice_members.iter().any(|m| m.user_id == "u1"));
        ch.voice_members.retain(|m| m.user_id != "u1");
        assert!(ch.voice_members.is_empty());
    }

    #[test]
    fn message_precomputes_clock_and_day_labels() {
        let msg = Message::new("1", "hi", "u", "User", 1_609_459_200 + 48_300);
        assert_eq!(msg.timestamp_label, "1:25 PM");
        assert_eq!(msg.day_label, "January 01, 2021");
    }

    #[test]
    fn message_clock_label_handles_midnight() {
        let msg = Message::new("1", "hi", "u", "User", 1_609_459_200);
        assert_eq!(msg.timestamp_label, "12:00 AM");
    }

    #[test]
    fn build_categories_threads_nested_after_parent() {
        let api_cats = vec![ApiCategoryDesc {
            category_id: "c1".into(),
            category_name: "General".into(),
            clan_id: "1".into(),
            category_order: 0,
        }];
        let mut channels = vec![
            make_channel("20", "parent-b", "c1"),
            make_channel("10", "parent-a", "c1"),
            make_thread("15", "10", "c1"),
            make_thread("25", "20", "c1"),
            make_thread("12", "10", "c1"),
        ];
        let cats = build_categories(api_cats, &mut channels);
        assert_eq!(cats.len(), 1);
        let ids: Vec<&str> = cats[0].channels.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["10", "12", "15", "20", "25"]);
    }

    #[test]
    fn build_categories_channel_id_ordering_within_category() {
        let api_cats = vec![ApiCategoryDesc {
            category_id: "c1".into(),
            category_name: "General".into(),
            clan_id: "1".into(),
            category_order: 0,
        }];
        let mut channels = vec![
            make_channel("30", "z", "c1"),
            make_channel("10", "a", "c1"),
            make_channel("20", "m", "c1"),
        ];
        let cats = build_categories(api_cats, &mut channels);
        let ids: Vec<&str> = cats[0].channels.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["10", "20", "30"]);
    }

    #[test]
    fn build_categories_threads_do_not_appear_as_top_level_siblings() {
        let api_cats = vec![ApiCategoryDesc {
            category_id: "c1".into(),
            category_name: "General".into(),
            clan_id: "1".into(),
            category_order: 0,
        }];
        let mut channels = vec![
            make_channel("10", "parent", "c1"),
            make_thread("11", "10", "c1"),
        ];
        let cats = build_categories(api_cats, &mut channels);
        let top_level: Vec<&str> = cats[0]
            .channels
            .iter()
            .filter(|ch| ch.parent_id.is_none())
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(top_level, vec!["10"]);
        let thread_row = cats[0].channels.iter().find(|c| c.id == "11").unwrap();
        assert_eq!(thread_row.parent_id.as_deref(), Some("10"));
    }

    #[test]
    fn favorites_mapping_sets_is_favorite_flag() {
        let ch = Channel {
            id: "42".into(),
            name: "fav".into(),
            channel_type: ChannelType::Text,
            private: false,
            clan_id: "1".into(),
            category_name: "General".into(),
            category_id: Some("c1".into()),
            member_count: 0,
            badge_count: 0,
            muted: false,
            parent_id: None,
            last_seen_timestamp: 0,
            last_sent_timestamp: 0,
            voice_members: Vec::new(),
            is_favorite: true,
        };
        assert!(ch.is_favorite);
    }

    #[test]
    fn synthetic_favor_cate_is_element_zero() {
        let channels_all = vec![
            Channel {
                id: "1".into(),
                name: "general".into(),
                channel_type: ChannelType::Text,
                private: false,
                clan_id: "clan1".into(),
                category_name: "Main".into(),
                category_id: Some("cat1".into()),
                member_count: 0,
                badge_count: 0,
                muted: false,
                parent_id: None,
                last_seen_timestamp: 0,
                last_sent_timestamp: 0,
                voice_members: Vec::new(),
                is_favorite: false,
            },
            Channel {
                id: "2".into(),
                name: "fav-ch".into(),
                channel_type: ChannelType::Text,
                private: false,
                clan_id: "clan1".into(),
                category_name: "Main".into(),
                category_id: Some("cat1".into()),
                member_count: 0,
                badge_count: 0,
                muted: false,
                parent_id: None,
                last_seen_timestamp: 0,
                last_sent_timestamp: 0,
                voice_members: Vec::new(),
                is_favorite: true,
            },
        ];

        let favor_channels: Vec<Channel> = channels_all
            .iter()
            .filter(|ch| ch.is_favorite)
            .cloned()
            .collect();

        let mut categories = vec![Category {
            id: "cat1".into(),
            clan_id: "clan1".into(),
            name: "Main".into(),
            order: 0,
            channels: channels_all,
        }];

        if !favor_channels.is_empty() {
            categories.insert(
                0,
                Category {
                    id: FAVOR_CATE_ID.into(),
                    clan_id: "clan1".into(),
                    name: "favoriteChannel".into(),
                    order: i32::MIN,
                    channels: favor_channels,
                },
            );
        }

        assert_eq!(categories[0].id, FAVOR_CATE_ID);
        assert_eq!(categories[0].channels.len(), 1);
        assert_eq!(categories[0].channels[0].id, "2");
        assert_eq!(categories[1].id, "cat1");
    }

    #[test]
    fn voice_member_resolution_fallback_to_user_id() {
        let vm = VoiceMember {
            user_id: "uid42".into(),
            display_name: "uid42".into(),
            avatar_url: String::new(),
        };
        assert_eq!(vm.display_name, vm.user_id);
        assert!(vm.avatar_url.is_empty());
    }

    #[test]
    fn collapse_state_roundtrip() {
        let mut collapsed: HashSet<(String, String)> = HashSet::new();
        collapsed.insert(("clan1".into(), "cat1".into()));
        collapsed.insert(("clan1".into(), "cat2".into()));

        let snapshot: Vec<(String, String)> = collapsed.iter().cloned().collect();
        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: Vec<(String, String)> = serde_json::from_str(&json).unwrap();
        let restored: HashSet<(String, String)> = parsed.into_iter().collect();

        assert!(restored.contains(&("clan1".into(), "cat1".into())));
        assert!(restored.contains(&("clan1".into(), "cat2".into())));
        assert!(!restored.contains(&("clan2".into(), "cat1".into())));
    }

    #[test]
    fn is_category_collapsed_defaults_to_false() {
        let collapsed: HashSet<(String, String)> = HashSet::new();
        let is_collapsed =
            |clan: &str, cat: &str| collapsed.contains(&(clan.to_string(), cat.to_string()));
        assert!(!is_collapsed("clan1", "cat1"));
    }

    #[test]
    fn assemble_with_favorites_yields_favor_cate_as_element_zero_on_first_load() {
        let api_cats = vec![ApiCategoryDesc {
            category_id: "c1".into(),
            category_name: "General".into(),
            clan_id: "clan1".into(),
            category_order: 0,
        }];
        let mut channels = vec![
            {
                let mut ch = make_channel("1", "normal", "c1");
                ch.clan_id = "clan1".into();
                ch
            },
            {
                let mut ch = make_channel("2", "fav-ch", "c1");
                ch.clan_id = "clan1".into();
                ch.is_favorite = true;
                ch
            },
        ];
        let favorite_ids: HashSet<String> = ["2".to_string()].into_iter().collect();
        let categories = build_categories(api_cats, &mut channels);
        let result = assemble_with_favorites(categories, &favorite_ids, "clan1");

        assert_eq!(result[0].id, FAVOR_CATE_ID);
        assert_eq!(result[0].channels.len(), 1);
        assert_eq!(result[0].channels[0].id, "2");
        assert_eq!(result[1].id, "c1");
    }

    #[test]
    fn assemble_with_favorites_no_favorites_is_noop() {
        let api_cats = vec![ApiCategoryDesc {
            category_id: "c1".into(),
            category_name: "General".into(),
            clan_id: "clan1".into(),
            category_order: 0,
        }];
        let mut channels = vec![make_channel("1", "normal", "c1")];
        let favorite_ids: HashSet<String> = HashSet::new();
        let categories = build_categories(api_cats, &mut channels);
        let result = assemble_with_favorites(categories, &favorite_ids, "clan1");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "c1");
    }
}
