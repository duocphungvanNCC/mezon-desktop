use crate::ids::{ChannelId, ClanId, RoleId, UserId};
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};
use mezon_client::{AppApi, ConnectionStatus, RealtimeEvent};
use mezon_proto::{api, realtime};

use crate::channel::{ChannelEvent, ChannelList};
use crate::clan::ClanList;
use crate::realtime::{RealtimeDispatch, RealtimeKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelMember {
    pub user_id: UserId,
    pub role_ids: Vec<RoleId>,
}

#[derive(Debug, Clone)]
pub enum ChannelMembersEvent {
    Changed { channel_id: ChannelId },
}

#[derive(Default)]
struct ChannelBucket {
    members: Vec<ChannelMember>,
}

impl ChannelBucket {
    fn upsert(&mut self, member: ChannelMember) {
        if let Some(existing) = self
            .members
            .iter_mut()
            .find(|m| m.user_id == member.user_id)
        {
            *existing = member;
        } else {
            self.members.push(member);
        }
    }

    fn remove(&mut self, user_id: UserId) {
        self.members.retain(|m| m.user_id != user_id);
    }
}

pub struct ChannelMembersStore {
    by_channel: HashMap<ChannelId, ChannelBucket>,
    loading: HashMap<ChannelId, bool>,
    api: Arc<AppApi>,
    _channel_sub: Subscription,
    _conn_watch: Task<()>,
}

struct GlobalChannelMembersStore(Entity<ChannelMembersStore>);
impl Global for GlobalChannelMembersStore {}

impl EventEmitter<ChannelMembersEvent> for ChannelMembersStore {}

impl ChannelMembersStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalChannelMembersStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalChannelMembersStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalChannelMembersStore>()
            .map(|g| g.0.clone())
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        Self::register_realtime(cx);

        let channel_sub = cx.subscribe(&ChannelList::global(cx), |this, _channel, event, cx| {
            if let ChannelEvent::ActiveChannelChanged(Some(channel_id)) = event {
                this.ensure_loaded(*channel_id, cx);
            }
        });

        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);

        Self {
            by_channel: HashMap::new(),
            loading: HashMap::new(),
            api,
            _channel_sub: channel_sub,
            _conn_watch: conn_watch,
        }
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            for kind in [
                RealtimeKind::UserChannelAdded,
                RealtimeKind::UserChannelRemoved,
            ] {
                dispatch.on(kind, &entity, |this, event, cx| {
                    this.handle_event(event, cx)
                });
            }
            dispatch.on_lagged(&entity, |this, cx| this.refresh_active(cx));
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
                    if this.update(cx, |this, cx| this.refresh_active(cx)).is_err() {
                        break;
                    }
                } else if !connected {
                    was_connected = false;
                }
            }
        })
    }

    pub fn member_ids(&self, channel_id: ChannelId) -> Vec<UserId> {
        match self.by_channel.get(&channel_id) {
            Some(bucket) => bucket.members.iter().map(|m| m.user_id).collect(),
            None => Vec::new(),
        }
    }

    pub fn has_channel(&self, channel_id: ChannelId) -> bool {
        self.by_channel.contains_key(&channel_id)
    }

    fn refresh_active(&mut self, cx: &mut Context<Self>) {
        if let Some(channel_id) = ChannelList::global(cx).read(cx).active_channel_id {
            self.fetch(channel_id, cx);
        }
    }

    pub fn ensure_loaded(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if !self.by_channel.contains_key(&channel_id) {
            self.fetch(channel_id, cx);
        }
    }

    fn clan_and_type_for_channel(channel_id: ChannelId, cx: &App) -> Option<(ClanId, i32)> {
        let found = ChannelList::global(cx)
            .read(cx)
            .find_channel(channel_id)
            .map(|c| (c.clan_id, c.channel_type.as_raw() as i32));
        match found {
            Some((clan_id, channel_type)) if !clan_id.is_zero() => Some((clan_id, channel_type)),
            Some((_, channel_type)) => ClanList::global(cx)
                .read(cx)
                .active_clan_id
                .map(|clan_id| (clan_id, channel_type)),
            None => ClanList::global(cx)
                .read(cx)
                .active_clan_id
                .map(|clan_id| (clan_id, 0)),
        }
    }

    fn fetch(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if self.loading.get(&channel_id).copied().unwrap_or(false) {
            return;
        }
        let Some((clan_id, channel_type)) = Self::clan_and_type_for_channel(channel_id, cx) else {
            return;
        };
        self.loading.insert(channel_id, true);
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_users(clan_id.get(), channel_id.get(), channel_type)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading.remove(&channel_id);
                match result {
                    Ok(users) => {
                        let mut bucket = ChannelBucket::default();
                        for cu in users {
                            bucket.upsert(channel_member_from_proto(&cu));
                        }
                        this.by_channel.insert(channel_id, bucket);
                        cx.emit(ChannelMembersEvent::Changed { channel_id });
                        cx.notify();
                    }
                    Err(e) => {
                        tracing::error!("list_channel_users failed for {channel_id}: {e}")
                    }
                }
            });
        })
        .detach();
    }

    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        match event {
            RealtimeEvent::UserChannelAdded(e) => {
                let Some(channel_id) = e.channel_desc.as_ref().map(|d| ChannelId(d.channel_id))
                else {
                    return;
                };
                if !self.by_channel.contains_key(&channel_id) {
                    return;
                }
                let bucket = self.by_channel.entry(channel_id).or_default();
                for user in &e.users {
                    bucket.upsert(channel_member_from_redis(user));
                }
                cx.emit(ChannelMembersEvent::Changed { channel_id });
                cx.notify();
            }
            RealtimeEvent::UserChannelRemoved(e) => {
                let channel_id = ChannelId(e.channel_id);
                let Some(bucket) = self.by_channel.get_mut(&channel_id) else {
                    return;
                };
                for uid in &e.user_ids {
                    bucket.remove(UserId(*uid));
                }
                cx.emit(ChannelMembersEvent::Changed { channel_id });
                cx.notify();
            }
            _ => {}
        }
    }
}

fn channel_member_from_proto(cu: &api::channel_user_list::ChannelUser) -> ChannelMember {
    ChannelMember {
        user_id: UserId(cu.user_id),
        role_ids: cu.role_id.iter().map(|id| RoleId(*id)).collect(),
    }
}

fn channel_member_from_redis(user: &realtime::UserProfileRedis) -> ChannelMember {
    ChannelMember {
        user_id: UserId(user.user_id),
        role_ids: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proto_channel_user(user_id: i64) -> api::channel_user_list::ChannelUser {
        api::channel_user_list::ChannelUser {
            user_id,
            role_id: vec![5],
            ..Default::default()
        }
    }

    #[test]
    fn maps_proto_channel_user_to_domain() {
        let member = channel_member_from_proto(&proto_channel_user(9));
        assert_eq!(member.user_id, UserId(9));
        assert_eq!(member.role_ids, vec![RoleId(5)]);
    }

    #[test]
    fn bucket_upsert_dedupes_by_user() {
        let mut bucket = ChannelBucket::default();
        bucket.upsert(channel_member_from_proto(&proto_channel_user(1)));
        bucket.upsert(channel_member_from_proto(&proto_channel_user(2)));
        bucket.upsert(channel_member_from_proto(&proto_channel_user(1)));
        assert_eq!(bucket.members.len(), 2);
    }

    #[test]
    fn bucket_remove_drops_member() {
        let mut bucket = ChannelBucket::default();
        bucket.upsert(channel_member_from_proto(&proto_channel_user(1)));
        bucket.upsert(channel_member_from_proto(&proto_channel_user(2)));
        bucket.remove(UserId(1));
        assert_eq!(bucket.members.len(), 1);
        assert_eq!(bucket.members[0].user_id, UserId(2));
    }

    #[test]
    fn redis_profile_maps_user_id_only() {
        let redis = realtime::UserProfileRedis {
            user_id: 33,
            ..Default::default()
        };
        let member = channel_member_from_redis(&redis);
        assert_eq!(member.user_id, UserId(33));
        assert!(member.role_ids.is_empty());
    }
}
