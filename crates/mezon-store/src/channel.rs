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

#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub content: String,
    pub sender_id: String,
    pub sender_name: String,
    pub create_time: i64,
    pub reactions: Vec<String>,
    pub attachments: Vec<String>,
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
            create_time,
            reactions: Vec::new(),
            attachments: Vec::new(),
        }
    }
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
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
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
            async move |this, cx| match api.list_channel_by_user_id().await {
                Ok(api_channels) => {
                    let channels: Vec<Channel> = api_channels
                        .into_iter()
                        .filter(|c| c.clan_id == clan_id)
                        .map(Channel::from)
                        .collect();
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
        // Mark a channel unread when a message lands in it (unless it is the open one).
        if let RealtimeEvent::ChannelMessage(m) = event {
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
        // TODO: ChannelCreated / ChannelDeleted / ChannelUpdated handlers go here.
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
