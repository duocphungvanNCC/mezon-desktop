use std::path::Path;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use mezon_client::transport::ApiClanDesc;
use mezon_client::{AppApi, ConnectionStatus, RealtimeEvent};

use crate::realtime::{RealtimeDispatch, RealtimeKind};

#[derive(Debug, Clone)]
pub struct Clan {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub badge_count: u32,
    pub has_unread: bool,
    pub muted: bool,
    pub welcome_channel_id: Option<String>,
}

impl From<ApiClanDesc> for Clan {
    fn from(c: ApiClanDesc) -> Self {
        let avatar_url = (!c.logo.is_empty()).then_some(c.logo);
        let banner_url = (!c.banner.is_empty()).then_some(c.banner);
        let welcome_channel_id = (!c.welcome_channel_id.is_empty()).then_some(c.welcome_channel_id);
        Self {
            id: c.clan_id,
            name: c.clan_name,
            avatar_url,
            banner_url,
            badge_count: 0,
            has_unread: false,
            muted: false,
            welcome_channel_id,
        }
    }
}

/// Typed events emitted by [`ClanList`] — the analog of Zed's `ChannelEvent`
/// (`channel_store.rs:144`). Other stores/views `cx.subscribe` to react to specific changes.
#[derive(Debug, Clone)]
pub enum ClanEvent {
    /// The active clan changed (or was cleared).
    ActiveClanChanged(Option<String>),
    /// A clan was removed (server push).
    Deleted(String),
}

/// Clan store — owns the clan list, fetches it over REST, and self-subscribes to realtime
/// clan events.
///
/// Native analog of Zed's `ChannelStore` (`crates/channel/src/channel_store.rs`): registered as
/// a [`Global`] (`init`/`global`), an [`EventEmitter`] of [`ClanEvent`], reacting to server
/// pushes in `handle_event`, holding its subscription `Task` so it cancels on drop.
pub struct ClanList {
    pub clans: Vec<Clan>,
    pub active_clan_id: Option<String>,
    api: Arc<AppApi>,
    _connection_watch: Task<()>,
}

struct GlobalClanList(Entity<ClanList>);
impl Global for GlobalClanList {}

impl EventEmitter<ClanEvent> for ClanList {}

impl ClanList {
    /// Create the store and register it as the app-wide global. Cf. `ChannelStore::init`
    /// (`channel_store.rs:25`). Call once during app setup, before any view reads it.
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalClanList(entity.clone()));
        entity
    }

    /// The global clan store. Panics if [`ClanList::init`] hasn't run. Cf. `ChannelStore::global`.
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalClanList>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalClanList>().map(|g| g.0.clone())
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        Self::register_realtime(cx);
        let connection_watch = Self::spawn_connection_watch(api.clone(), cx);
        Self {
            clans: Vec::new(),
            active_clan_id: None,
            api,
            _connection_watch: connection_watch,
        }
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            for kind in [
                RealtimeKind::ClanUpdated,
                RealtimeKind::ClanDeleted,
                RealtimeKind::AddClanUser,
                RealtimeKind::UserClanRemoved,
                RealtimeKind::ChannelMessage,
                RealtimeKind::MarkAsRead,
            ] {
                dispatch.on(kind, &entity, |this, event, cx| {
                    this.handle_event(event, cx)
                });
            }
            dispatch.on_lagged(&entity, |this, cx| {
                tracing::warn!("ClanList realtime lagged — reloading clans");
                this.reload(cx);
            });
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
                    // Reconnected — realtime pushes were missed while offline, so the cached list
                    // is stale: always refetch (not just when empty).
                    if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                        break;
                    }
                } else if !connected {
                    was_connected = false;
                }
            }
        })
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let (clans_result, badges_result) =
                tokio::join!(api.list_clan_descs(), api.list_clan_badge_count());
            let clans = match clans_result {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to load clans: {e}");
                    return;
                }
            };
            let badge_map: std::collections::HashMap<String, (i32, bool)> = badges_result
                .unwrap_or_else(|e| {
                    tracing::warn!("clan badge count fetch failed: {e}");
                    Vec::new()
                })
                .into_iter()
                .map(|(id, badge, has_unread)| (id, (badge, has_unread)))
                .collect();
            let mapped: Vec<Clan> = clans
                .into_iter()
                .map(|c| {
                    let mut clan = Clan::from(c);
                    if let Some(&(badge, has_unread)) = badge_map.get(&clan.id) {
                        clan.badge_count = badge.max(0) as u32;
                        clan.has_unread = has_unread;
                    }
                    clan
                })
                .collect();
            let _ = this.update(cx, |this, cx| this.update_clans(mapped, cx));
        })
        .detach();
    }

    /// Apply a server-pushed realtime event. Cf. `ChannelStore::handle_update_channels`.
    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        match event {
            RealtimeEvent::ClanDeleted(e) => {
                let id = e.clan_id.to_string();
                let before = self.clans.len();
                self.clans.retain(|c| c.id != id);
                if self.clans.len() != before {
                    cx.emit(ClanEvent::Deleted(id.clone()));
                    if self.active_clan_id.as_deref() == Some(id.as_str()) {
                        let next = self.clans.first().map(|c| c.id.clone());
                        self.active_clan_id = next.clone();
                        cx.emit(ClanEvent::ActiveClanChanged(next));
                    }
                    cx.notify();
                }
            }
            RealtimeEvent::ClanUpdated(e) => {
                let name = (!e.clan_name.is_empty()).then_some(e.clan_name.clone());
                if update_clan(
                    &mut self.clans,
                    &e.clan_id.to_string(),
                    name,
                    e.logo.clone(),
                ) {
                    cx.notify();
                }
            }
            RealtimeEvent::AddClanUser(e) => {
                let id = e.clan_id.to_string();
                if !self.clans.iter().any(|c| c.id == id) {
                    self.reload(cx);
                }
            }
            RealtimeEvent::UserClanRemoved(e) => {
                let id = e.clan_id.to_string();
                let before = self.clans.len();
                self.clans.retain(|c| c.id != id);
                if self.clans.len() != before {
                    cx.emit(ClanEvent::Deleted(id.clone()));
                    if self.active_clan_id.as_deref() == Some(id.as_str()) {
                        let next = self.clans.first().map(|c| c.id.clone());
                        self.active_clan_id = next.clone();
                        cx.emit(ClanEvent::ActiveClanChanged(next));
                    }
                    cx.notify();
                }
            }
            RealtimeEvent::ChannelMessage(m) => {
                let clan_id = m.clan_id.to_string();
                if let Some(clan) = self.clans.iter_mut().find(|c| c.id == clan_id)
                    && !clan.muted
                {
                    clan.badge_count = clan.badge_count.saturating_add(1);
                    clan.has_unread = true;
                    cx.notify();
                }
            }
            RealtimeEvent::MarkAsRead(m) => {
                let clan_id = m.clan_id.to_string();
                if let Some(clan) = self.clans.iter_mut().find(|c| c.id == clan_id)
                    && (clan.badge_count > 0 || clan.has_unread)
                {
                    clan.badge_count = 0;
                    clan.has_unread = false;
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    pub fn active_clan(&self) -> Option<&Clan> {
        self.active_clan_id
            .as_ref()
            .and_then(|id| self.clans.iter().find(|c| &c.id == id))
    }

    pub fn active_clan_banner(&self) -> Option<&str> {
        self.active_clan().and_then(|c| c.banner_url.as_deref())
    }

    pub fn is_active_clan(&self, clan_id: &str) -> bool {
        self.active_clan_id.as_deref() == Some(clan_id)
    }

    pub fn welcome_channel_id(&self, clan_id: &str) -> Option<String> {
        self.clans
            .iter()
            .find(|c| c.id == clan_id)
            .and_then(|c| c.welcome_channel_id.clone())
    }

    pub fn select_clan(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.active_clan_id.as_deref() == Some(id) {
            return;
        }
        self.active_clan_id = Some(id.to_string());
        cx.emit(ClanEvent::ActiveClanChanged(self.active_clan_id.clone()));
        cx.notify();
    }

    pub fn update_clans(&mut self, clans: Vec<Clan>, cx: &mut Context<Self>) {
        let prev_active = self.active_clan_id.clone();
        self.clans = clans;
        if !self.clans.is_empty() {
            let active_still_valid = self
                .active_clan_id
                .as_ref()
                .is_some_and(|id| self.clans.iter().any(|c| &c.id == id));
            if !active_still_valid {
                self.active_clan_id = Some(self.clans[0].id.clone());
            }
        }
        if self.active_clan_id != prev_active {
            cx.emit(ClanEvent::ActiveClanChanged(self.active_clan_id.clone()));
        }
        cx.notify();
    }

    pub fn create_clan(
        &mut self,
        name: String,
        logo: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<String, CreateClanError>> {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let trimmed = name.trim().to_string();
            let is_dup = api
                .check_duplicate_clan_name(&trimmed, "0")
                .await
                .map_err(|e| CreateClanError::Other(e.to_string()))?;
            if is_dup {
                return Err(CreateClanError::DuplicateName);
            }
            let desc = api
                .create_clan_desc(&trimmed, &logo, "")
                .await
                .map_err(|e| CreateClanError::Other(e.to_string()))?;
            let clan_id = desc.clan_id.clone();
            this.update(cx, |this, cx| {
                apply_created_clan(&mut this.clans, desc);
                this.select_clan(&clan_id, cx);
            })
            .map_err(|_| CreateClanError::Other("store dropped".into()))?;
            Ok(clan_id)
        })
    }

    pub fn upload_clan_logo(
        &self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<String>> {
        let api = self.api.clone();
        let path = path.to_path_buf();
        cx.spawn(async move |_this, _cx| api.upload_avatar(&path).await)
    }
}

fn update_clan(clans: &mut [Clan], clan_id: &str, name: Option<String>, logo: String) -> bool {
    let Some(clan) = clans.iter_mut().find(|c| c.id == clan_id) else {
        return false;
    };
    if let Some(name) = name {
        clan.name = name;
    }
    clan.avatar_url = (!logo.is_empty()).then_some(logo);
    true
}

#[derive(Debug)]
pub enum CreateClanError {
    DuplicateName,
    Other(String),
}

impl std::fmt::Display for CreateClanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateName => write!(f, "A clan with that name already exists."),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

pub(crate) fn apply_created_clan(clans: &mut Vec<Clan>, desc: ApiClanDesc) {
    let clan = Clan::from(desc);
    if !clans.iter().any(|c| c.id == clan.id) {
        clans.push(clan);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_clan(id: &str, name: &str, avatar_url: Option<&str>) -> Clan {
        Clan {
            id: id.into(),
            name: name.into(),
            avatar_url: avatar_url.map(|s| s.into()),
            banner_url: None,
            badge_count: 0,
            has_unread: false,
            muted: false,
            welcome_channel_id: None,
        }
    }

    fn clans() -> Vec<Clan> {
        vec![
            make_clan("1", "One", None),
            make_clan("2", "Two", Some("old.png")),
        ]
    }

    #[test]
    fn update_clan_sets_name_and_logo() {
        let mut c = clans();
        assert!(update_clan(
            &mut c,
            "1",
            Some("NewName".into()),
            "logo.png".into()
        ));
        assert_eq!(c[0].name, "NewName");
        assert_eq!(c[0].avatar_url.as_deref(), Some("logo.png"));
    }

    #[test]
    fn update_clan_blank_name_keeps_name_and_empty_logo_clears_avatar() {
        let mut c = clans();
        assert!(update_clan(&mut c, "2", None, String::new()));
        assert_eq!(c[1].name, "Two");
        assert_eq!(c[1].avatar_url, None);
    }

    #[test]
    fn update_clan_unknown_is_noop() {
        let mut c = clans();
        assert!(!update_clan(&mut c, "999", Some("x".into()), "y".into()));
    }

    #[test]
    fn clan_from_api_desc_zeroes_badge_and_muted() {
        use mezon_client::transport::ApiClanDesc;
        let desc = ApiClanDesc {
            clan_id: "42".into(),
            clan_name: "Alpha".into(),
            creator_id: String::new(),
            logo: "logo.png".into(),
            banner: String::new(),
            welcome_channel_id: String::new(),
        };
        let clan = Clan::from(desc);
        assert_eq!(clan.badge_count, 0);
        assert!(!clan.has_unread);
        assert!(!clan.muted);
        assert_eq!(clan.avatar_url.as_deref(), Some("logo.png"));
        assert!(clan.welcome_channel_id.is_none());
    }

    #[test]
    fn badge_map_applies_to_clans_on_reload() {
        let mut c = clans();
        let badge_map: std::collections::HashMap<String, (i32, bool)> = [
            ("1".to_string(), (3_i32, true)),
            ("99".to_string(), (5_i32, false)),
        ]
        .into_iter()
        .collect();
        for clan in &mut c {
            if let Some(&(badge, has_unread)) = badge_map.get(&clan.id) {
                clan.badge_count = badge.max(0) as u32;
                clan.has_unread = has_unread;
            }
        }
        assert_eq!(c[0].badge_count, 3);
        assert!(c[0].has_unread);
        assert_eq!(c[1].badge_count, 0);
        assert!(!c[1].has_unread);
    }

    #[test]
    fn channel_message_increments_badge_when_not_muted() {
        use mezon_proto::api;
        let mut c = clans();
        let msg = api::ChannelMessage {
            clan_id: 1,
            ..Default::default()
        };
        let event = RealtimeEvent::ChannelMessage(msg);
        if let RealtimeEvent::ChannelMessage(m) = &event {
            let clan_id = m.clan_id.to_string();
            if let Some(clan) = c.iter_mut().find(|c| c.id == clan_id)
                && !clan.muted
            {
                clan.badge_count = clan.badge_count.saturating_add(1);
                clan.has_unread = true;
            }
        }
        assert_eq!(c[0].badge_count, 1);
        assert!(c[0].has_unread);
    }

    #[test]
    fn channel_message_skipped_when_muted() {
        let mut c = clans();
        c[0].muted = true;
        if let Some(clan) = c.iter_mut().find(|cl| cl.id == "1")
            && !clan.muted
        {
            clan.badge_count = clan.badge_count.saturating_add(1);
            clan.has_unread = true;
        }
        assert_eq!(c[0].badge_count, 0);
        assert!(!c[0].has_unread);
    }

    #[test]
    fn mark_as_read_resets_badge_and_unread() {
        use mezon_proto::realtime;
        let mut c = clans();
        c[0].badge_count = 7;
        c[0].has_unread = true;
        let evt = realtime::MarkAsRead {
            clan_id: 1,
            ..Default::default()
        };
        if let Some(clan) = c.iter_mut().find(|cl| cl.id == evt.clan_id.to_string())
            && (clan.badge_count > 0 || clan.has_unread)
        {
            clan.badge_count = 0;
            clan.has_unread = false;
        }
        assert_eq!(c[0].badge_count, 0);
        assert!(!c[0].has_unread);
    }

    #[test]
    fn mark_as_read_unknown_clan_is_noop() {
        use mezon_proto::realtime;
        let mut c = clans();
        c[0].badge_count = 3;
        let evt = realtime::MarkAsRead {
            clan_id: 999,
            ..Default::default()
        };
        if let Some(clan) = c.iter_mut().find(|cl| cl.id == evt.clan_id.to_string()) {
            clan.badge_count = 0;
            clan.has_unread = false;
        }
        assert_eq!(c[0].badge_count, 3);
    }

    #[test]
    fn apply_created_clan_inserts_new_clan() {
        use mezon_client::transport::ApiClanDesc;
        let mut clans = clans();
        let desc = ApiClanDesc {
            clan_id: "99".into(),
            clan_name: "NewClan".into(),
            creator_id: String::new(),
            logo: "logo.png".into(),
            banner: String::new(),
            welcome_channel_id: String::new(),
        };
        apply_created_clan(&mut clans, desc);
        assert_eq!(clans.len(), 3);
        let inserted = clans.iter().find(|c| c.id == "99").unwrap();
        assert_eq!(inserted.name, "NewClan");
        assert_eq!(inserted.avatar_url.as_deref(), Some("logo.png"));
        assert_eq!(inserted.badge_count, 0);
        assert!(!inserted.has_unread);
    }

    #[test]
    fn apply_created_clan_skips_duplicate_id() {
        use mezon_client::transport::ApiClanDesc;
        let mut clans = clans();
        let desc = ApiClanDesc {
            clan_id: "1".into(),
            clan_name: "SameClan".into(),
            creator_id: String::new(),
            logo: String::new(),
            banner: String::new(),
            welcome_channel_id: String::new(),
        };
        apply_created_clan(&mut clans, desc);
        assert_eq!(clans.len(), 2);
        assert_eq!(clans[0].name, "One");
    }

    #[test]
    fn create_clan_error_display_duplicate_name() {
        let err = CreateClanError::DuplicateName;
        let msg = format!("{err}");
        assert!(msg.contains("already exists"));
    }

    #[test]
    fn create_clan_error_display_other() {
        let err = CreateClanError::Other("network timeout".into());
        let msg = format!("{err}");
        assert_eq!(msg, "network timeout");
    }
}
