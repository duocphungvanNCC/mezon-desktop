use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global};
use mezon_client::{AppApi, RealtimeEvent};
use mezon_proto::api;

use crate::message::MessageCode;
use crate::realtime::{RealtimeDispatch, RealtimeKind};
use crate::{ChannelId, ClanId, UserId};

const API_PAGE_SIZE: i32 = 100;

#[derive(Clone, Debug, Default)]
pub struct ChannelSetting {
    pub id: ChannelId,
    pub creator_id: UserId,
    pub parent_id: ChannelId,
    pub label: String,
    pub private: bool,
    pub channel_type: i32,
    pub user_ids: Vec<UserId>,
    pub message_count: i64,
    pub last_sender_id: UserId,
    pub last_sent_seconds: u32,
}

impl From<api::ChannelSettingItem> for ChannelSetting {
    fn from(item: api::ChannelSettingItem) -> Self {
        let last = item.last_sent_message.unwrap_or_default();
        Self {
            id: ChannelId(item.id),
            creator_id: UserId(item.creator_id),
            parent_id: ChannelId(item.parent_id),
            label: item.channel_label,
            private: item.channel_private != 0,
            channel_type: item.channel_type,
            user_ids: item.user_ids.into_iter().map(UserId).collect(),
            message_count: item.message_count,
            last_sender_id: UserId(last.sender_id),
            last_sent_seconds: last.timestamp_seconds,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ChannelSettingsEvent {
    Changed {
        clan_id: ClanId,
        parent_id: ChannelId,
    },
}

pub struct ChannelSettingsStore {
    rows: HashMap<(ClanId, ChannelId), Vec<ChannelSetting>>,
    loading: HashSet<(ClanId, ChannelId)>,
    api: Arc<AppApi>,
}

struct GlobalChannelSettingsStore(Entity<ChannelSettingsStore>);
impl Global for GlobalChannelSettingsStore {}
impl EventEmitter<ChannelSettingsEvent> for ChannelSettingsStore {}

impl ChannelSettingsStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| {
            let store = Self {
                rows: HashMap::new(),
                loading: HashSet::new(),
                api,
            };
            Self::register_realtime(cx);
            store
        });
        cx.set_global(GlobalChannelSettingsStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalChannelSettingsStore>().0.clone()
    }

    pub fn rows(&self, clan_id: ClanId, parent_id: ChannelId) -> &[ChannelSetting] {
        self.rows
            .get(&(clan_id, parent_id))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn is_loading(&self, clan_id: ClanId, parent_id: ChannelId) -> bool {
        self.loading.contains(&(clan_id, parent_id))
    }

    pub fn ensure_loaded(&mut self, clan_id: ClanId, parent_id: ChannelId, cx: &mut Context<Self>) {
        let key = (clan_id, parent_id);
        if self.rows.contains_key(&key) || !self.loading.insert(key) {
            return;
        }
        self.fetch(key, cx);
    }

    fn reload(&mut self, key: (ClanId, ChannelId), cx: &mut Context<Self>) {
        if !self.loading.insert(key) {
            return;
        }
        self.fetch(key, cx);
    }

    fn fetch(&self, key: (ClanId, ChannelId), cx: &mut Context<Self>) {
        let (clan_id, parent_id) = key;
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let mut page = 1;
            let mut all = Vec::new();
            let result = loop {
                match api
                    .list_channel_setting_page(clan_id.get(), parent_id.get(), API_PAGE_SIZE, page)
                    .await
                {
                    Ok(response) => {
                        let total = if parent_id.get() == 0 {
                            response.channel_count.max(0) as usize
                        } else {
                            response.thread_count.max(0) as usize
                        };
                        let received = response.channel_setting_list.len();
                        all.extend(response.channel_setting_list.into_iter().map(Into::into));
                        if received < API_PAGE_SIZE as usize || all.len() >= total {
                            break Ok(all);
                        }
                        page += 1;
                    }
                    Err(error) => break Err(error),
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.loading.remove(&key);
                match result {
                    Ok(rows) => {
                        this.rows.insert(key, rows);
                        cx.emit(ChannelSettingsEvent::Changed { clan_id, parent_id });
                    }
                    Err(error) => tracing::error!(
                        "ListChannelSetting failed for clan {clan_id}, parent {parent_id}: {error}"
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(RealtimeKind::ChannelMessage, &entity, |this, event, cx| {
                this.handle_message(event, cx)
            });
            for kind in [
                RealtimeKind::ChannelCreated,
                RealtimeKind::ChannelUpdated,
                RealtimeKind::ChannelDeleted,
                RealtimeKind::ChannelArchive,
            ] {
                dispatch.on(kind, &entity, |this, _, cx| this.reload_loaded(cx));
            }
            dispatch.on_lagged(&entity, |this, cx| this.reload_loaded(cx));
        });
    }

    fn handle_message(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::ChannelMessage(message) = event else {
            return;
        };
        let clan_id = ClanId(message.clan_id);
        let channel_id = ChannelId(message.channel_id);
        let code = MessageCode::from_raw(message.code);

        if matches!(
            code,
            MessageCode::Typing
                | MessageCode::Indicator
                | MessageCode::ChatUpdate
                | MessageCode::UpdateEphemeralMsg
        ) {
            return;
        }

        if matches!(
            code,
            MessageCode::ChatRemove | MessageCode::DeleteEphemeralMsg
        ) {
            self.reload_channel(clan_id, channel_id, cx);
            return;
        }

        let mut changed = Vec::new();
        for (&(row_clan_id, parent_id), rows) in &mut self.rows {
            if row_clan_id != clan_id {
                continue;
            }
            if let Some(row) = rows.iter_mut().find(|row| row.id == channel_id) {
                row.message_count = row.message_count.saturating_add(1);
                row.last_sender_id = UserId(message.sender_id);
                row.last_sent_seconds = message.create_time_seconds;
                changed.push(parent_id);
            }
        }
        for parent_id in changed {
            cx.emit(ChannelSettingsEvent::Changed { clan_id, parent_id });
        }
        cx.notify();
    }

    fn reload_channel(&mut self, clan_id: ClanId, channel_id: ChannelId, cx: &mut Context<Self>) {
        let keys = self
            .rows
            .iter()
            .filter(|((row_clan_id, _), rows)| {
                *row_clan_id == clan_id && rows.iter().any(|row| row.id == channel_id)
            })
            .map(|(&key, _)| key)
            .collect::<Vec<_>>();
        for key in keys {
            self.reload(key, cx);
        }
    }

    fn reload_loaded(&mut self, cx: &mut Context<Self>) {
        let keys = self.rows.keys().copied().collect::<Vec<_>>();
        for key in keys {
            self.reload(key, cx);
        }
    }
}
