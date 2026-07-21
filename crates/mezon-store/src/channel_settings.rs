use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global};
use mezon_client::AppApi;
use mezon_proto::api;

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
        let entity = cx.new(|_| Self {
            rows: HashMap::new(),
            loading: HashSet::new(),
            api,
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
}
