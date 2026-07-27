use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use mezon_client::{AppApi, ConnectionStatus, RealtimeEvent};
use mezon_proto::api;

use crate::realtime::{RealtimeDispatch, RealtimeKind};
use crate::{ChannelId, ChannelList, ClanId, UserId};

const EVENT_CREATED: i32 = 1;
const EVENT_UPDATED: i32 = 2;
const EVENT_DELETED: i32 = 3;
const EVENT_INTERESTED: i32 = 4;
const EVENT_UNINTERESTED: i32 = 5;
const EVENT_COMPLETED: i32 = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct ClanEventItem {
    pub id: i64,
    pub title: String,
    pub logo: String,
    pub description: String,
    pub creator_id: UserId,
    pub channel_voice_id: Option<ChannelId>,
    pub address: String,
    pub start_time_seconds: u32,
    pub end_time_seconds: u32,
    pub user_ids: Vec<UserId>,
    pub channel_id: Option<ChannelId>,
    pub event_status: i32,
    pub is_private: bool,
    pub external_link: String,
}

impl ClanEventItem {
    fn from_api(event: api::EventManagement) -> Self {
        Self {
            id: event.id,
            title: event.title,
            logo: event.logo,
            description: event.description,
            creator_id: UserId(event.creator_id),
            channel_voice_id: nonzero_channel(event.channel_voice_id),
            address: event.address,
            start_time_seconds: event.start_time_seconds,
            end_time_seconds: event.end_time_seconds,
            user_ids: event.user_ids.into_iter().map(UserId).collect(),
            channel_id: nonzero_channel(event.channel_id),
            event_status: event.event_status,
            is_private: event.is_private,
            external_link: event
                .meet_room
                .map(|room| room.external_link)
                .unwrap_or_default(),
        }
    }

    fn from_realtime(event: &api::CreateEventRequest) -> Self {
        Self {
            id: event.event_id,
            title: event.title.clone(),
            logo: event.logo.clone(),
            description: event.description.clone(),
            creator_id: UserId(event.creator_id),
            channel_voice_id: nonzero_channel(event.channel_voice_id),
            address: event.address.clone(),
            start_time_seconds: event.start_time_seconds,
            end_time_seconds: event.end_time_seconds,
            user_ids: vec![UserId(event.creator_id)],
            channel_id: nonzero_channel(event.channel_id),
            event_status: event.event_status,
            is_private: event.is_private,
            external_link: event
                .meet_room
                .as_ref()
                .map(|room| room.external_link.clone())
                .unwrap_or_default(),
        }
    }

    fn apply_realtime(&mut self, event: &api::CreateEventRequest) {
        self.title.clone_from(&event.title);
        self.logo.clone_from(&event.logo);
        self.description.clone_from(&event.description);
        self.creator_id = UserId(event.creator_id);
        self.channel_voice_id = nonzero_channel(event.channel_voice_id);
        self.address.clone_from(&event.address);
        self.start_time_seconds = event.start_time_seconds;
        self.end_time_seconds = event.end_time_seconds;
        self.channel_id = nonzero_channel(event.channel_id);
        if event.event_status != 0 {
            self.event_status = event.event_status;
        }
        self.is_private = event.is_private;
        if let Some(room) = &event.meet_room {
            self.external_link.clone_from(&room.external_link);
        }
    }
}

fn nonzero_channel(id: i64) -> Option<ChannelId> {
    (id != 0).then_some(ChannelId(id))
}

#[derive(Clone, Debug)]
pub enum EventsEvent {
    Changed { clan_id: ClanId },
}

pub struct EventsStore {
    events: HashMap<ClanId, Vec<ClanEventItem>>,
    loaded: HashSet<ClanId>,
    loading: HashSet<ClanId>,
    api: Arc<AppApi>,
    _connection_watch: Task<()>,
    _clock_task: Task<()>,
}

struct GlobalEventsStore(Entity<EventsStore>);
impl Global for GlobalEventsStore {}
impl EventEmitter<EventsEvent> for EventsStore {}

impl EventsStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalEventsStore(entity.clone()));
        entity
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        Self::register_realtime(cx);
        let connection_watch = Self::spawn_connection_watch(api.clone(), cx);
        let clock_task = Self::spawn_clock(cx);
        Self {
            events: HashMap::new(),
            loaded: HashSet::new(),
            loading: HashSet::new(),
            api,
            _connection_watch: connection_watch,
            _clock_task: clock_task,
        }
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalEventsStore>().0.clone()
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(
                RealtimeKind::ClanEventCreated,
                &entity,
                |this, event, cx| this.handle_realtime(event, cx),
            );
            dispatch.on_lagged(&entity, |this, cx| this.refresh_loaded(cx));
        });
    }

    fn spawn_connection_watch(api: Arc<AppApi>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut status = api.status();
            let mut connected_before = *status.borrow() == ConnectionStatus::Connected;
            loop {
                if status.changed().await.is_err() {
                    break;
                }
                let connected = *status.borrow() == ConnectionStatus::Connected;
                if connected && !connected_before {
                    if this.update(cx, |this, cx| this.refresh_loaded(cx)).is_err() {
                        break;
                    }
                }
                connected_before = connected;
            }
        })
    }

    fn spawn_clock(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update(cx, |this, cx| {
                        let changed = this.remove_expired();
                        for clan_id in changed {
                            cx.emit(EventsEvent::Changed { clan_id });
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    fn remove_expired(&mut self) -> Vec<ClanId> {
        let now = chrono::Utc::now().timestamp().max(0) as u32;
        let mut changed = Vec::new();
        for (clan_id, events) in &mut self.events {
            let old_len = events.len();
            events.retain(|event| {
                event.event_status != EVENT_COMPLETED
                    && (event.end_time_seconds == 0 || event.end_time_seconds > now)
            });
            if events.len() != old_len {
                changed.push(*clan_id);
            }
        }
        changed
    }

    fn handle_realtime(&mut self, realtime: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::ClanEventCreated(event) = realtime else {
            return;
        };
        let clan_id = ClanId(event.clan_id);
        let events = self.events.entry(clan_id).or_default();
        match event.action {
            EVENT_CREATED => {
                let item = ClanEventItem::from_realtime(event);
                if let Some(existing) = events.iter_mut().find(|item| item.id == event.event_id) {
                    *existing = item;
                } else {
                    events.push(item);
                }
            }
            EVENT_UPDATED => {
                if event.event_status == EVENT_COMPLETED {
                    events.retain(|item| item.id != event.event_id);
                } else if let Some(existing) =
                    events.iter_mut().find(|item| item.id == event.event_id)
                {
                    existing.apply_realtime(event);
                } else {
                    events.push(ClanEventItem::from_realtime(event));
                }
            }
            EVENT_DELETED => events.retain(|item| item.id != event.event_id),
            EVENT_INTERESTED => {
                if let Some(existing) = events.iter_mut().find(|item| item.id == event.event_id) {
                    let user = UserId(event.user_id);
                    if !existing.user_ids.contains(&user) {
                        existing.user_ids.push(user);
                    }
                }
            }
            EVENT_UNINTERESTED => {
                if let Some(existing) = events.iter_mut().find(|item| item.id == event.event_id) {
                    existing.user_ids.retain(|user| user.0 != event.user_id);
                }
            }
            _ => self.fetch(clan_id, cx),
        }
        self.remove_expired();
        cx.emit(EventsEvent::Changed { clan_id });
        cx.notify();
    }

    pub fn events(&self, clan_id: ClanId) -> &[ClanEventItem] {
        self.events.get(&clan_id).map_or(&[], Vec::as_slice)
    }

    pub fn ensure_loaded(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        if !self.loaded.contains(&clan_id) {
            self.fetch(clan_id, cx);
        }
    }

    fn refresh_loaded(&mut self, cx: &mut Context<Self>) {
        let clans: Vec<_> = self.loaded.iter().copied().collect();
        for clan_id in clans {
            self.fetch(clan_id, cx);
        }
    }

    fn fetch(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        if !self.loading.insert(clan_id) {
            return;
        }
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.list_events(clan_id.0).await;
            let _ = this.update(cx, |this, cx| {
                this.loading.remove(&clan_id);
                match result {
                    Ok(items) => {
                        let items = items
                            .into_iter()
                            .map(ClanEventItem::from_api)
                            .filter(|event| event.event_status != EVENT_COMPLETED)
                            .collect();
                        this.events.insert(clan_id, items);
                        this.loaded.insert(clan_id);
                        this.remove_expired();
                    }
                    Err(error) => tracing::warn!(%error, %clan_id, "failed to load events"),
                }
                cx.emit(EventsEvent::Changed { clan_id });
                cx.notify();
            });
        })
        .detach();
    }

    pub fn visible_events(
        &self,
        clan_id: ClanId,
        current_user: Option<UserId>,
        cx: &App,
    ) -> Vec<ClanEventItem> {
        let channels = ChannelList::global(cx);
        let channels = channels.read(cx);
        let now = chrono::Utc::now().timestamp().max(0) as u32;
        self.events(clan_id)
            .iter()
            .filter(|event| {
                event.event_status != EVENT_COMPLETED
                    && (event.end_time_seconds == 0 || event.end_time_seconds > now)
                    && (!event.is_private || Some(event.creator_id) == current_user)
                    && event
                        .channel_id
                        .is_none_or(|channel_id| channels.channel_in_clan(clan_id, channel_id))
            })
            .cloned()
            .collect()
    }
}
