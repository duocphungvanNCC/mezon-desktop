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
    discarded: HashSet<(ClanId, ChannelId)>,
    pending_reload: HashSet<(ClanId, ChannelId)>,
    pending_events: HashMap<(ClanId, ChannelId), Vec<RealtimeEvent>>,
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
                discarded: HashSet::new(),
                pending_reload: HashSet::new(),
                pending_events: HashMap::new(),
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

    pub fn reset_clan(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        self.rows
            .retain(|(row_clan_id, _), _| *row_clan_id != clan_id);
        self.pending_reload
            .retain(|(row_clan_id, _)| *row_clan_id != clan_id);
        self.pending_events
            .retain(|(row_clan_id, _), _| *row_clan_id != clan_id);
        self.discarded.extend(
            self.loading
                .iter()
                .filter(|(row_clan_id, _)| *row_clan_id == clan_id)
                .copied(),
        );
        cx.notify();
    }

    pub fn ensure_loaded(&mut self, clan_id: ClanId, parent_id: ChannelId, cx: &mut Context<Self>) {
        let key = (clan_id, parent_id);
        self.discarded.remove(&key);
        if self.rows.contains_key(&key) || !self.loading.insert(key) {
            return;
        }
        self.fetch(key, cx);
    }

    fn reload(&mut self, key: (ClanId, ChannelId), cx: &mut Context<Self>) {
        if !self.loading.insert(key) {
            self.pending_reload.insert(key);
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
                if this.discarded.remove(&key) {
                    this.pending_reload.remove(&key);
                    this.pending_events.remove(&key);
                    return;
                }
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
                if let Some(events) = this.pending_events.remove(&key) {
                    for event in events {
                        this.handle_realtime_event(&event, cx);
                    }
                }
                if this.pending_reload.remove(&key) {
                    this.reload(key, cx);
                }
            });
        })
        .detach();
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(RealtimeKind::ChannelMessage, &entity, |this, event, cx| {
                this.handle_realtime_event(event, cx)
            });
            for kind in [
                RealtimeKind::ChannelCreated,
                RealtimeKind::ChannelUpdated,
                RealtimeKind::ChannelDeleted,
                RealtimeKind::UserChannelAdded,
                RealtimeKind::UserChannelRemoved,
                RealtimeKind::ChannelArchive,
            ] {
                dispatch.on(kind, &entity, |this, event, cx| {
                    this.handle_realtime_event(event, cx)
                });
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
            MessageCode::ChatRemove | MessageCode::DeleteEphemeralMsg
        ) {
            self.patch_message_count(clan_id, channel_id, -1, None, cx);
            return;
        }
        if !counts_as_channel_message(code) {
            return;
        }
        self.patch_message_count(
            clan_id,
            channel_id,
            1,
            Some((UserId(message.sender_id), message.create_time_seconds)),
            cx,
        );
    }

    fn handle_realtime_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        if let Some(key) = self.loading_key_for_event(event)
            && self.loading.contains(&key)
        {
            self.pending_events
                .entry(key)
                .or_default()
                .push(event.clone());
            return;
        }
        if matches!(event, RealtimeEvent::ChannelMessage(_)) {
            self.handle_message(event, cx);
        } else {
            self.handle_channel_event(event, cx);
        }
    }

    fn loading_key_for_event(&self, event: &RealtimeEvent) -> Option<(ClanId, ChannelId)> {
        let target = match event {
            RealtimeEvent::ChannelCreated(event) => {
                (ClanId(event.clan_id), ChannelId(event.parent_id))
            }
            RealtimeEvent::ChannelUpdated(event) => {
                (ClanId(event.clan_id), ChannelId(event.parent_id))
            }
            RealtimeEvent::ChannelDeleted(event) => {
                (ClanId(event.clan_id), ChannelId(event.parent_id))
            }
            RealtimeEvent::ChannelArchive(event) => {
                (ClanId(event.clan_id), ChannelId(event.parent_id))
            }
            RealtimeEvent::UserChannelAdded(event) => {
                let desc = event.channel_desc.as_ref()?;
                (ClanId(event.clan_id), ChannelId(desc.parent_id))
            }
            RealtimeEvent::UserChannelRemoved(event) => {
                self.find_row_key(ClanId(event.clan_id), ChannelId(event.channel_id))?
            }
            RealtimeEvent::ChannelMessage(event) => {
                self.find_row_key(ClanId(event.clan_id), ChannelId(event.channel_id))?
            }
            _ => return None,
        };
        Some(target)
    }

    fn find_row_key(&self, clan_id: ClanId, channel_id: ChannelId) -> Option<(ClanId, ChannelId)> {
        self.rows.iter().find_map(|(&key, rows)| {
            (key.0 == clan_id && rows.iter().any(|row| row.id == channel_id)).then_some(key)
        })
    }

    fn patch_message_count(
        &mut self,
        clan_id: ClanId,
        channel_id: ChannelId,
        delta: i64,
        last_sent: Option<(UserId, u32)>,
        cx: &mut Context<Self>,
    ) {
        let changed = patch_matching_rows(&mut self.rows, clan_id, channel_id, |row| {
            row.message_count = (row.message_count + delta).max(0);
            if let Some((sender_id, timestamp)) = last_sent {
                row.last_sender_id = sender_id;
                row.last_sent_seconds = timestamp;
            }
        });
        self.notify_changed(clan_id, changed, cx);
    }

    fn handle_channel_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let changed = match event {
            RealtimeEvent::ChannelCreated(event) => {
                let clan_id = ClanId(event.clan_id);
                let parent_id = ChannelId(event.parent_id);
                let key = (clan_id, parent_id);
                let Some(rows) = self.rows.get_mut(&key) else {
                    return;
                };
                let row = ChannelSetting {
                    id: ChannelId(event.channel_id),
                    creator_id: UserId(event.creator_id),
                    parent_id,
                    label: event.channel_label.clone(),
                    private: event.channel_private != 0,
                    channel_type: event.channel_type,
                    ..Default::default()
                };
                if rows.iter().any(|existing| existing.id == row.id) {
                    return;
                }
                rows.push(row);
                (clan_id, vec![parent_id])
            }
            RealtimeEvent::ChannelUpdated(event) => {
                let clan_id = ClanId(event.clan_id);
                let changed = patch_matching_rows(
                    &mut self.rows,
                    clan_id,
                    ChannelId(event.channel_id),
                    |row| {
                        row.label = event.channel_label.clone();
                        row.private = event.channel_private;
                        row.channel_type = event.channel_type;
                    },
                );
                (clan_id, changed)
            }
            RealtimeEvent::ChannelDeleted(event) => {
                let clan_id = ClanId(event.clan_id);
                let channel_id = ChannelId(event.channel_id);
                self.rows.remove(&(clan_id, channel_id));
                let changed = remove_matching_rows(&mut self.rows, clan_id, channel_id);
                (clan_id, changed)
            }
            RealtimeEvent::ChannelArchive(event) => {
                let clan_id = ClanId(event.clan_id);
                let channel_id = ChannelId(event.channel_id);
                if event.active == 0 {
                    let changed = remove_matching_rows(&mut self.rows, clan_id, channel_id);
                    (clan_id, changed)
                } else {
                    let parent_id = ChannelId(event.parent_id);
                    let Some(rows) = self.rows.get_mut(&(clan_id, parent_id)) else {
                        return;
                    };
                    if rows.iter().any(|row| row.id == channel_id) {
                        return;
                    }
                    rows.push(ChannelSetting {
                        id: channel_id,
                        creator_id: UserId(event.creator_id),
                        parent_id,
                        label: event.channel_label.clone(),
                        private: event.channel_private,
                        channel_type: event.channel_type,
                        user_ids: event.user_ids.iter().copied().map(UserId).collect(),
                        ..Default::default()
                    });
                    (clan_id, vec![parent_id])
                }
            }
            RealtimeEvent::UserChannelAdded(event) => {
                let Some(desc) = event.channel_desc.as_ref() else {
                    return;
                };
                let clan_id = ClanId(event.clan_id);
                let users = event
                    .users
                    .iter()
                    .map(|user| UserId(user.user_id))
                    .collect::<Vec<_>>();
                let changed = patch_matching_rows(
                    &mut self.rows,
                    clan_id,
                    ChannelId(desc.channel_id),
                    |row| {
                        for user_id in &users {
                            if !row.user_ids.contains(user_id) {
                                row.user_ids.push(*user_id);
                            }
                        }
                    },
                );
                (clan_id, changed)
            }
            RealtimeEvent::UserChannelRemoved(event) => {
                let clan_id = ClanId(event.clan_id);
                let removed = event
                    .user_ids
                    .iter()
                    .copied()
                    .map(UserId)
                    .collect::<HashSet<_>>();
                let changed = patch_matching_rows(
                    &mut self.rows,
                    clan_id,
                    ChannelId(event.channel_id),
                    |row| row.user_ids.retain(|user_id| !removed.contains(user_id)),
                );
                (clan_id, changed)
            }
            _ => return,
        };
        self.notify_changed(changed.0, changed.1, cx);
    }

    fn notify_changed(&self, clan_id: ClanId, parent_ids: Vec<ChannelId>, cx: &mut Context<Self>) {
        if parent_ids.is_empty() {
            return;
        }
        for parent_id in parent_ids {
            cx.emit(ChannelSettingsEvent::Changed { clan_id, parent_id });
        }
        cx.notify();
    }

    fn reload_loaded(&mut self, cx: &mut Context<Self>) {
        let keys = self.rows.keys().copied().collect::<Vec<_>>();
        for key in keys {
            self.reload(key, cx);
        }
    }
}

fn counts_as_channel_message(code: MessageCode) -> bool {
    matches!(
        code,
        MessageCode::Chat
            | MessageCode::Welcome
            | MessageCode::CreateThread
            | MessageCode::CreatePin
            | MessageCode::MessageBuzz
            | MessageCode::Topic
            | MessageCode::AuditLog
            | MessageCode::SendToken
            | MessageCode::UpcomingEvent
            | MessageCode::ShareContact
            | MessageCode::Location
            | MessageCode::Poll
    )
}

fn patch_matching_rows(
    rows_by_key: &mut HashMap<(ClanId, ChannelId), Vec<ChannelSetting>>,
    clan_id: ClanId,
    channel_id: ChannelId,
    mut patch: impl FnMut(&mut ChannelSetting),
) -> Vec<ChannelId> {
    let mut changed = Vec::new();
    for (&(row_clan_id, parent_id), rows) in rows_by_key {
        if row_clan_id == clan_id
            && let Some(row) = rows.iter_mut().find(|row| row.id == channel_id)
        {
            patch(row);
            changed.push(parent_id);
        }
    }
    changed
}

fn remove_matching_rows(
    rows_by_key: &mut HashMap<(ClanId, ChannelId), Vec<ChannelSetting>>,
    clan_id: ClanId,
    channel_id: ChannelId,
) -> Vec<ChannelId> {
    let mut changed = Vec::new();
    for (&(row_clan_id, parent_id), rows) in rows_by_key {
        if row_clan_id != clan_id {
            continue;
        }
        let old_len = rows.len();
        rows.retain(|row| row.id != channel_id);
        if rows.len() != old_len {
            changed.push(parent_id);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, count: i64) -> ChannelSetting {
        ChannelSetting {
            id: ChannelId(id),
            message_count: count,
            ..Default::default()
        }
    }

    #[test]
    fn message_allow_list_excludes_mutations_ephemeral_and_unknown_codes() {
        assert!(counts_as_channel_message(MessageCode::Chat));
        assert!(counts_as_channel_message(MessageCode::Poll));
        assert!(!counts_as_channel_message(MessageCode::ChatUpdate));
        assert!(!counts_as_channel_message(MessageCode::ChatRemove));
        assert!(!counts_as_channel_message(MessageCode::Ephemeral));
        assert!(!counts_as_channel_message(MessageCode::Unknown(99)));
    }

    #[test]
    fn patch_matching_rows_is_scoped_to_clan_and_reports_parent() {
        let mut rows = HashMap::from([
            ((ClanId(1), ChannelId(0)), vec![row(10, 2)]),
            ((ClanId(2), ChannelId(0)), vec![row(10, 7)]),
        ]);
        let changed = patch_matching_rows(&mut rows, ClanId(1), ChannelId(10), |row| {
            row.message_count += 1
        });
        assert_eq!(changed, vec![ChannelId(0)]);
        assert_eq!(rows[&(ClanId(1), ChannelId(0))][0].message_count, 3);
        assert_eq!(rows[&(ClanId(2), ChannelId(0))][0].message_count, 7);
    }

    #[test]
    fn removing_channel_only_changes_its_loaded_parent() {
        let mut rows = HashMap::from([
            ((ClanId(1), ChannelId(0)), vec![row(10, 2)]),
            ((ClanId(1), ChannelId(10)), vec![row(20, 1)]),
        ]);
        let changed = remove_matching_rows(&mut rows, ClanId(1), ChannelId(20));
        assert_eq!(changed, vec![ChannelId(10)]);
        assert!(rows[&(ClanId(1), ChannelId(10))].is_empty());
        assert_eq!(rows[&(ClanId(1), ChannelId(0))].len(), 1);
    }
}
