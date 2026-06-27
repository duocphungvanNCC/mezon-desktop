use crate::ids::{ChannelId, UserId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global};
use mezon_client::{AppApi, RealtimeEvent};
use mezon_proto::{api, realtime};

use crate::clan_members::User;
use crate::realtime::{RealtimeDispatch, RealtimeKind};

const GROUP_MEMBER_FETCH_LIMIT: i32 = 500;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupMember {
    pub user: User,
    pub online: bool,
}

impl GroupMember {
    pub fn id(&self) -> UserId {
        self.user.id
    }

    pub fn name(&self) -> &str {
        if !self.user.display_name.is_empty() {
            &self.user.display_name
        } else {
            &self.user.username
        }
    }

    pub fn avatar(&self) -> &str {
        &self.user.avatar_url
    }
}

#[derive(Debug, Clone)]
pub enum GroupMembersEvent {
    Changed { channel_id: ChannelId },
}

#[derive(Default)]
struct GroupBucket {
    members: Vec<GroupMember>,
    by_id: HashMap<UserId, usize>,
}

impl GroupBucket {
    fn from_members(members: Vec<GroupMember>) -> Self {
        let mut bucket = Self {
            members,
            by_id: HashMap::new(),
        };
        bucket.reindex();
        bucket
    }

    fn reindex(&mut self) {
        self.by_id.clear();
        self.by_id.reserve(self.members.len());
        for (i, m) in self.members.iter().enumerate() {
            self.by_id.insert(m.user.id, i);
        }
    }

    fn as_slice(&self) -> &[GroupMember] {
        &self.members
    }

    fn get(&self, user_id: UserId) -> Option<&GroupMember> {
        let idx = *self.by_id.get(&user_id)?;
        self.members.get(idx)
    }

    fn upsert(&mut self, member: GroupMember) {
        if let Some(&idx) = self.by_id.get(&member.user.id) {
            self.members[idx] = member;
        } else {
            self.by_id.insert(member.user.id, self.members.len());
            self.members.push(member);
        }
    }

    fn remove_ids(&mut self, user_ids: &[UserId]) {
        let before = self.members.len();
        self.members.retain(|m| !user_ids.contains(&m.user.id));
        if self.members.len() != before {
            self.reindex();
        }
    }
}

pub struct GroupMembersStore {
    by_channel: HashMap<ChannelId, GroupBucket>,
    loading: HashSet<ChannelId>,
    api: Arc<AppApi>,
}

struct GlobalGroupMembersStore(Entity<GroupMembersStore>);
impl Global for GlobalGroupMembersStore {}

impl EventEmitter<GroupMembersEvent> for GroupMembersStore {}

impl GroupMembersStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalGroupMembersStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalGroupMembersStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalGroupMembersStore>()
            .map(|g| g.0.clone())
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        Self::register_realtime(cx);
        Self {
            by_channel: HashMap::new(),
            loading: HashSet::new(),
            api,
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
        });
    }

    pub fn members(&self, channel_id: ChannelId) -> &[GroupMember] {
        self.by_channel
            .get(&channel_id)
            .map(GroupBucket::as_slice)
            .unwrap_or(&[])
    }

    pub fn member(&self, channel_id: ChannelId, user_id: UserId) -> Option<&GroupMember> {
        self.by_channel.get(&channel_id)?.get(user_id)
    }

    pub fn ensure_loaded(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if !self.by_channel.contains_key(&channel_id) {
            self.fetch(channel_id, cx);
        }
    }

    pub fn refresh(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        self.fetch(channel_id, cx);
    }

    fn fetch(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if !self.loading.insert(channel_id) {
            return;
        }
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_users_uc(channel_id.get(), GROUP_MEMBER_FETCH_LIMIT)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading.remove(&channel_id);
                match result {
                    Ok(resp) => {
                        let members = group_members_from_proto(&resp);
                        this.by_channel
                            .insert(channel_id, GroupBucket::from_members(members));
                        cx.emit(GroupMembersEvent::Changed { channel_id });
                        cx.notify();
                    }
                    Err(e) => {
                        tracing::error!("list_channel_users_uc failed for {channel_id}: {e}")
                    }
                }
            });
        })
        .detach();
    }

    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let changed_channel = match event {
            RealtimeEvent::UserChannelAdded(e) => {
                let Some(channel_id) = e.channel_desc.as_ref().map(|d| ChannelId(d.channel_id))
                else {
                    return;
                };
                apply_add_members(&mut self.by_channel, channel_id, &e.users).then_some(channel_id)
            }
            RealtimeEvent::UserChannelRemoved(e) => {
                let channel_id = ChannelId(e.channel_id);
                let ids: Vec<UserId> = e.user_ids.iter().map(|id| UserId(*id)).collect();
                apply_remove_members(&mut self.by_channel, channel_id, &ids).then_some(channel_id)
            }
            _ => None,
        };
        if let Some(channel_id) = changed_channel {
            cx.emit(GroupMembersEvent::Changed { channel_id });
            cx.notify();
        }
    }
}

fn group_members_from_proto(resp: &api::AllUsersAddChannelResponse) -> Vec<GroupMember> {
    resp.user_ids
        .iter()
        .enumerate()
        .filter_map(|(i, &uid)| {
            if uid == 0 {
                return None;
            }
            Some(GroupMember {
                user: User {
                    id: UserId(uid),
                    username: resp.usernames.get(i).cloned().unwrap_or_default(),
                    display_name: resp.display_names.get(i).cloned().unwrap_or_default(),
                    avatar_url: resp.avatars.get(i).cloned().unwrap_or_default(),
                    about_me: String::new(),
                    create_time_seconds: 0,
                },
                online: resp.onlines.get(i).copied().unwrap_or(false),
            })
        })
        .collect()
}

fn group_member_from_redis(user: &realtime::UserProfileRedis) -> Option<GroupMember> {
    if user.user_id == 0 {
        return None;
    }
    Some(GroupMember {
        user: User {
            id: UserId(user.user_id),
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            avatar_url: user.avatar.clone(),
            about_me: String::new(),
            create_time_seconds: user.create_time_second,
        },
        online: user.online,
    })
}

fn apply_add_members(
    by_channel: &mut HashMap<ChannelId, GroupBucket>,
    channel_id: ChannelId,
    users: &[realtime::UserProfileRedis],
) -> bool {
    let Some(bucket) = by_channel.get_mut(&channel_id) else {
        return false;
    };
    for user in users {
        let Some(member) = group_member_from_redis(user) else {
            continue;
        };
        bucket.upsert(member);
    }
    true
}

fn apply_remove_members(
    by_channel: &mut HashMap<ChannelId, GroupBucket>,
    channel_id: ChannelId,
    user_ids: &[UserId],
) -> bool {
    let Some(bucket) = by_channel.get_mut(&channel_id) else {
        return false;
    };
    bucket.remove_ids(user_ids);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proto_response(ids: &[i64]) -> api::AllUsersAddChannelResponse {
        api::AllUsersAddChannelResponse {
            channel_id: 1,
            user_ids: ids.to_vec(),
            limit: 500,
            usernames: ids.iter().map(|id| format!("user{id}")).collect(),
            display_names: ids.iter().map(|id| format!("User {id}")).collect(),
            avatars: ids.iter().map(|id| format!("{id}.png")).collect(),
            onlines: ids.iter().map(|id| id % 2 == 0).collect(),
        }
    }

    #[test]
    fn maps_parallel_arrays_to_members() {
        let members = group_members_from_proto(&proto_response(&[10, 11]));
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].id(), UserId(10));
        assert_eq!(members[0].name(), "User 10");
        assert_eq!(members[0].avatar(), "10.png");
        assert!(members[0].online);
        assert!(!members[1].online);
    }

    #[test]
    fn maps_robustly_when_arrays_shorter_than_user_ids() {
        let mut resp = proto_response(&[10, 11, 12]);
        resp.usernames = vec!["only-one".into()];
        resp.display_names = vec![];
        resp.avatars = vec![];
        resp.onlines = vec![];
        let members = group_members_from_proto(&resp);
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].user.username, "only-one");
        assert_eq!(members[1].user.username, "");
        assert_eq!(members[0].name(), "only-one");
        assert_eq!(members[1].name(), "");
        assert!(!members[0].online);
    }

    #[test]
    fn skips_zero_user_ids() {
        let members = group_members_from_proto(&proto_response(&[0, 5]));
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].id(), UserId(5));
    }

    fn assert_index_consistent(bucket: &GroupBucket) {
        assert_eq!(bucket.by_id.len(), bucket.members.len());
        for (i, m) in bucket.members.iter().enumerate() {
            assert_eq!(bucket.by_id.get(&m.user.id), Some(&i));
        }
    }

    #[test]
    fn add_members_applies_to_loaded_group() {
        let mut by_channel = HashMap::from([(ChannelId(1), GroupBucket::default())]);
        let users = vec![realtime::UserProfileRedis {
            user_id: 7,
            username: "bob".into(),
            ..Default::default()
        }];
        assert!(apply_add_members(&mut by_channel, ChannelId(1), &users));
        let bucket = &by_channel[&ChannelId(1)];
        assert_eq!(bucket.members.len(), 1);
        assert_eq!(bucket.members[0].id(), UserId(7));
        assert_eq!(bucket.get(UserId(7)).map(GroupMember::id), Some(UserId(7)));
        assert_index_consistent(bucket);
    }

    #[test]
    fn add_members_ignored_for_unloaded_group() {
        let mut by_channel: HashMap<ChannelId, GroupBucket> = HashMap::new();
        let users = vec![realtime::UserProfileRedis {
            user_id: 7,
            ..Default::default()
        }];
        assert!(!apply_add_members(&mut by_channel, ChannelId(1), &users));
        assert!(by_channel.is_empty());
    }

    #[test]
    fn add_members_dedupes_existing_user() {
        let mut by_channel = HashMap::from([(ChannelId(1), GroupBucket::default())]);
        let users = vec![realtime::UserProfileRedis {
            user_id: 7,
            username: "bob".into(),
            ..Default::default()
        }];
        apply_add_members(&mut by_channel, ChannelId(1), &users);
        apply_add_members(&mut by_channel, ChannelId(1), &users);
        assert_eq!(by_channel[&ChannelId(1)].members.len(), 1);
        assert_index_consistent(&by_channel[&ChannelId(1)]);
    }

    #[test]
    fn remove_members_drops_users() {
        let mut by_channel = HashMap::from([(
            ChannelId(1),
            GroupBucket::from_members(group_members_from_proto(&proto_response(&[1, 2, 3]))),
        )]);
        assert!(apply_remove_members(
            &mut by_channel,
            ChannelId(1),
            &[UserId(2)]
        ));
        let bucket = &by_channel[&ChannelId(1)];
        let ids: Vec<UserId> = bucket.members.iter().map(GroupMember::id).collect();
        assert_eq!(ids, vec![UserId(1), UserId(3)]);
        assert!(bucket.get(UserId(2)).is_none());
        assert_index_consistent(bucket);
    }
}
