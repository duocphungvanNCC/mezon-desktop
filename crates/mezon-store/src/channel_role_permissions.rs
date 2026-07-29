use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use mezon_client::{AppApi, ConnectionStatus};
use mezon_proto::api;

use crate::KeyedCache;
use crate::ids::{ChannelId, ClanId, RoleId, UserId};
use crate::permissions::PermissionStore;

const MAX_CACHED_ENTITIES: usize = 128;

pub const OVERRIDE_TYPE_NEUTRAL: i32 = 0;
pub const OVERRIDE_TYPE_ALLOW: i32 = 1;
pub const OVERRIDE_TYPE_DENY: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionEntity {
    Role(RoleId),
    User(UserId),
}

impl PermissionEntity {
    fn role_id(&self) -> i64 {
        match self {
            Self::Role(id) => id.get(),
            Self::User(_) => 0,
        }
    }

    fn user_id(&self) -> i64 {
        match self {
            Self::Role(_) => 0,
            Self::User(id) => id.get(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct EntityKey {
    channel_id: ChannelId,
    entity: PermissionEntity,
}

#[derive(Debug, Clone)]
pub enum ChannelRolePermissionsEvent {
    Changed {
        channel_id: ChannelId,
        entity: PermissionEntity,
    },
    SaveFailed {
        channel_id: ChannelId,
        entity: PermissionEntity,
    },
}

pub struct ChannelRolePermissionsStore {
    cache: KeyedCache<EntityKey, HashMap<i64, bool>>,
    loading: HashSet<EntityKey>,
    saving: HashSet<EntityKey>,
    api: Arc<AppApi>,
    _conn_watch: Task<()>,
}

struct GlobalChannelRolePermissionsStore(Entity<ChannelRolePermissionsStore>);
impl Global for GlobalChannelRolePermissionsStore {}

impl EventEmitter<ChannelRolePermissionsEvent> for ChannelRolePermissionsStore {}

impl ChannelRolePermissionsStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalChannelRolePermissionsStore(entity.clone()));
        entity
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);
        Self {
            cache: KeyedCache::new(Some(MAX_CACHED_ENTITIES)),
            loading: HashSet::new(),
            saving: HashSet::new(),
            api,
            _conn_watch: conn_watch,
        }
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalChannelRolePermissionsStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalChannelRolePermissionsStore>()
            .map(|g| g.0.clone())
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.cache.clear();
        self.loading.clear();
        self.saving.clear();
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
                    if this
                        .update(cx, |this, cx| {
                            this.cache.clear();
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                } else if !connected {
                    was_connected = false;
                }
            }
        })
    }

    pub fn is_loaded(&self, channel_id: ChannelId, entity: PermissionEntity) -> bool {
        self.cache.contains(&EntityKey { channel_id, entity })
    }

    pub fn is_saving(&self, channel_id: ChannelId, entity: PermissionEntity) -> bool {
        self.saving.contains(&EntityKey { channel_id, entity })
    }

    pub fn permission_active(
        &self,
        channel_id: ChannelId,
        entity: PermissionEntity,
        permission_id: i64,
    ) -> Option<bool> {
        self.cache
            .get(&EntityKey { channel_id, entity })
            .and_then(|overrides| overrides.get(&permission_id).copied())
    }

    pub fn ensure_loaded(
        &mut self,
        channel_id: ChannelId,
        entity: PermissionEntity,
        cx: &mut Context<Self>,
    ) {
        let key = EntityKey { channel_id, entity };
        if self.cache.contains(&key) || !self.loading.insert(key) {
            return;
        }
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .get_permission_by_role_id_channel_id(
                    entity.role_id(),
                    channel_id.get(),
                    entity.user_id(),
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading.remove(&key);
                match result {
                    Ok(response) => {
                        this.cache
                            .insert(key, overrides_from_response(&response), None);
                        cx.emit(ChannelRolePermissionsEvent::Changed { channel_id, entity });
                        cx.notify();
                    }
                    Err(error) => tracing::error!(
                        "get_permission_by_role_id_channel_id failed for {channel_id}: {error}"
                    ),
                }
            });
        })
        .detach();
    }

    pub fn save(
        &mut self,
        clan_id: ClanId,
        channel_id: ChannelId,
        entity: PermissionEntity,
        pending: &HashMap<i64, i32>,
        cx: &mut Context<Self>,
    ) {
        let key = EntityKey { channel_id, entity };
        if !self.cache.contains(&key) {
            cx.emit(ChannelRolePermissionsEvent::SaveFailed { channel_id, entity });
            cx.notify();
            return;
        }
        if !self.saving.insert(key) {
            return;
        }
        let definitions = PermissionStore::try_global(cx)
            .map(|store| {
                store
                    .read(cx)
                    .channel_scoped_definitions()
                    .map(|definition| (definition.id, definition.slug.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let persisted = self.cache.get(&key);
        let updates = build_permission_snapshot(&definitions, pending, persisted);
        let max_permission_id = resolve_max_permission_id(clan_id, cx);
        let applied = applied_overrides(&updates);

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .set_role_channel_permission(
                    entity.role_id(),
                    channel_id.get(),
                    entity.user_id(),
                    max_permission_id,
                    updates,
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.saving.remove(&key);
                match result {
                    Ok(()) => {
                        this.cache.insert(key, applied, None);
                        cx.emit(ChannelRolePermissionsEvent::Changed { channel_id, entity });
                        cx.notify();
                    }
                    Err(error) => {
                        tracing::error!(
                            "set_role_channel_permission failed for {channel_id}: {error}"
                        );
                        cx.emit(ChannelRolePermissionsEvent::SaveFailed { channel_id, entity });
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

fn resolve_max_permission_id(clan_id: ClanId, cx: &App) -> i64 {
    crate::roles::RolesStore::try_global(cx)
        .map(|roles| roles.read(cx).resolve_max_permission_id(clan_id, cx))
        .unwrap_or(0)
}

fn overrides_from_response(
    response: &api::PermissionRoleChannelListEventResponse,
) -> HashMap<i64, bool> {
    response
        .permission_role_channel
        .iter()
        .map(|entry| (entry.permission_id, entry.active))
        .collect()
}

fn build_permission_snapshot(
    definitions: &[(i64, String)],
    pending: &HashMap<i64, i32>,
    persisted: Option<&HashMap<i64, bool>>,
) -> Vec<api::PermissionUpdate> {
    definitions
        .iter()
        .map(|(id, slug)| {
            let r#type = match pending.get(id) {
                Some(pending_type) => *pending_type,
                None => match persisted.and_then(|overrides| overrides.get(id)) {
                    Some(true) => OVERRIDE_TYPE_ALLOW,
                    Some(false) => OVERRIDE_TYPE_DENY,
                    None => OVERRIDE_TYPE_NEUTRAL,
                },
            };
            api::PermissionUpdate {
                permission_id: *id,
                slug: slug.clone(),
                r#type,
            }
        })
        .collect()
}

fn applied_overrides(updates: &[api::PermissionUpdate]) -> HashMap<i64, bool> {
    updates
        .iter()
        .filter(|update| update.r#type != OVERRIDE_TYPE_NEUTRAL)
        .map(|update| (update.permission_id, update.r#type == OVERRIDE_TYPE_ALLOW))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definitions() -> Vec<(i64, String)> {
        vec![
            (1, "send-message".into()),
            (2, "delete-message".into()),
            (3, "manage-thread".into()),
        ]
    }

    #[test]
    fn snapshot_without_persisted_overrides_would_wipe_them_all() {
        let updates = build_permission_snapshot(&definitions(), &HashMap::new(), None);
        assert!(
            updates.iter().all(|u| u.r#type == OVERRIDE_TYPE_NEUTRAL),
            "an unloaded entity snapshots to all-neutral, which is why save() must refuse it"
        );
    }

    #[test]
    fn snapshot_sends_every_channel_scoped_permission_not_just_the_diff() {
        let pending = HashMap::from([(1, OVERRIDE_TYPE_DENY)]);
        let persisted = HashMap::from([(2, true)]);
        let updates = build_permission_snapshot(&definitions(), &pending, Some(&persisted));

        assert_eq!(updates.len(), 3);
        let by_id = updates
            .iter()
            .map(|u| (u.permission_id, u.r#type))
            .collect::<HashMap<_, _>>();
        assert_eq!(by_id[&1], OVERRIDE_TYPE_DENY);
        assert_eq!(by_id[&2], OVERRIDE_TYPE_ALLOW);
        assert_eq!(by_id[&3], OVERRIDE_TYPE_NEUTRAL);
        assert_eq!(updates[0].slug, "send-message");
    }

    #[test]
    fn snapshot_falls_back_to_neutral_without_persisted_overrides() {
        let updates = build_permission_snapshot(&definitions(), &HashMap::new(), None);
        assert!(updates.iter().all(|u| u.r#type == OVERRIDE_TYPE_NEUTRAL));
    }

    #[test]
    fn applied_overrides_drop_neutral_and_map_allow_deny() {
        let updates = vec![
            api::PermissionUpdate {
                permission_id: 1,
                slug: "send-message".into(),
                r#type: OVERRIDE_TYPE_ALLOW,
            },
            api::PermissionUpdate {
                permission_id: 2,
                slug: "delete-message".into(),
                r#type: OVERRIDE_TYPE_DENY,
            },
            api::PermissionUpdate {
                permission_id: 3,
                slug: "manage-thread".into(),
                r#type: OVERRIDE_TYPE_NEUTRAL,
            },
        ];
        let applied = applied_overrides(&updates);

        assert_eq!(applied.get(&1), Some(&true));
        assert_eq!(applied.get(&2), Some(&false));
        assert_eq!(applied.get(&3), None);
    }

    #[test]
    fn entity_maps_to_exactly_one_id_field() {
        let role = PermissionEntity::Role(RoleId(7));
        assert_eq!((role.role_id(), role.user_id()), (7, 0));
        let user = PermissionEntity::User(UserId(9));
        assert_eq!((user.role_id(), user.user_id()), (0, 9));
    }
}
