use std::collections::HashSet;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use mezon_client::{AppApi, ConnectionStatus};

use crate::ids::{ChannelId, UserId};
use crate::{CACHE_TTL, KeyedCache};

const MAX_CACHED_CHANNELS: usize = 64;

const CHANNEL_USER_FETCH_LIMIT: i32 = 500;

#[derive(Debug, Clone)]
pub enum ChannelUsersEvent {
    Changed { channel_id: ChannelId },
}

pub struct ChannelUsersStore {
    cache: KeyedCache<ChannelId, Vec<UserId>>,
    loading: HashSet<ChannelId>,
    api: Arc<AppApi>,
    _conn_watch: Task<()>,
}

struct GlobalChannelUsersStore(Entity<ChannelUsersStore>);
impl Global for GlobalChannelUsersStore {}

impl EventEmitter<ChannelUsersEvent> for ChannelUsersStore {}

impl ChannelUsersStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalChannelUsersStore(entity.clone()));
        entity
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);
        Self {
            cache: KeyedCache::new(Some(MAX_CACHED_CHANNELS)),
            loading: HashSet::new(),
            api,
            _conn_watch: conn_watch,
        }
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalChannelUsersStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalChannelUsersStore>()
            .map(|g| g.0.clone())
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.cache.clear();
        self.loading.clear();
        cx.notify();
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
                    if this.update(cx, |this, _| this.invalidate()).is_err() {
                        break;
                    }
                } else if !connected {
                    was_connected = false;
                }
            }
        })
    }

    fn invalidate(&mut self) {
        self.cache.mark_all_stale();
    }

    pub fn user_ids(&self, channel_id: ChannelId) -> &[UserId] {
        self.cache
            .get(&channel_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn is_loaded(&self, channel_id: ChannelId) -> bool {
        self.cache.contains(&channel_id)
    }

    pub fn is_loading(&self, channel_id: ChannelId) -> bool {
        self.loading.contains(&channel_id)
    }

    pub fn ensure_loaded(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if channel_id.get() == 0 || self.cache.is_fresh(&channel_id, CACHE_TTL) {
            return;
        }
        if !self.loading.insert(channel_id) {
            return;
        }
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_users_uc(channel_id.get(), CHANNEL_USER_FETCH_LIMIT)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading.remove(&channel_id);
                match result {
                    Ok(response) => {
                        let user_ids = response
                            .user_ids
                            .into_iter()
                            .map(UserId)
                            .collect::<Vec<_>>();
                        this.cache.insert(channel_id, user_ids, None);
                        cx.emit(ChannelUsersEvent::Changed { channel_id });
                        cx.notify();
                    }
                    Err(error) => {
                        tracing::error!("list_channel_users_uc failed for {channel_id}: {error}")
                    }
                }
            });
        })
        .detach();
    }

    pub fn add_users(
        &mut self,
        channel_id: ChannelId,
        user_ids: &[UserId],
        cx: &mut Context<Self>,
    ) {
        let Some(existing) = self.cache.get_mut(&channel_id) else {
            return;
        };
        if apply_add(existing, user_ids) {
            cx.emit(ChannelUsersEvent::Changed { channel_id });
            cx.notify();
        }
    }

    pub fn remove_users(
        &mut self,
        channel_id: ChannelId,
        user_ids: &[UserId],
        cx: &mut Context<Self>,
    ) {
        let Some(existing) = self.cache.get_mut(&channel_id) else {
            return;
        };
        if apply_remove(existing, user_ids) {
            cx.emit(ChannelUsersEvent::Changed { channel_id });
            cx.notify();
        }
    }
}

fn apply_add(existing: &mut Vec<UserId>, user_ids: &[UserId]) -> bool {
    let mut changed = false;
    for user_id in user_ids {
        if !existing.contains(user_id) {
            existing.push(*user_id);
            changed = true;
        }
    }
    changed
}

fn apply_remove(existing: &mut Vec<UserId>, user_ids: &[UserId]) -> bool {
    let before = existing.len();
    existing.retain(|id| !user_ids.contains(id));
    existing.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_users_skips_duplicates_and_reports_change() {
        let mut existing = vec![UserId(1), UserId(2)];
        assert!(apply_add(&mut existing, &[UserId(3)]));
        assert_eq!(existing, vec![UserId(1), UserId(2), UserId(3)]);
        assert!(!apply_add(&mut existing, &[UserId(2), UserId(3)]));
        assert_eq!(existing, vec![UserId(1), UserId(2), UserId(3)]);
    }

    #[test]
    fn remove_users_reports_change_only_when_present() {
        let mut existing = vec![UserId(1), UserId(2), UserId(3)];
        assert!(apply_remove(&mut existing, &[UserId(2)]));
        assert_eq!(existing, vec![UserId(1), UserId(3)]);
        assert!(!apply_remove(&mut existing, &[UserId(99)]));
        assert_eq!(existing, vec![UserId(1), UserId(3)]);
    }
}
