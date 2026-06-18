use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use mezon_client::transport::ApiClanDesc;
use mezon_client::{AppApi, ConnectionStatus, RealtimeEvent};

#[derive(Debug, Clone)]
pub struct Clan {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub unread_count: u32,
}

impl From<ApiClanDesc> for Clan {
    fn from(c: ApiClanDesc) -> Self {
        let avatar_url = (!c.logo.is_empty()).then_some(c.logo);
        Self {
            id: c.clan_id,
            name: c.clan_name,
            avatar_url,
            unread_count: 0,
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
    _realtime: Task<()>,
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
        let realtime = Self::spawn_realtime(api.clone(), cx);
        let connection_watch = Self::spawn_connection_watch(api.clone(), cx);
        Self {
            clans: Vec::new(),
            active_clan_id: None,
            api,
            _realtime: realtime,
            _connection_watch: connection_watch,
        }
    }

    /// Subscribe to the realtime broadcast and dispatch each event to `handle_event`.
    /// Cf. `ChannelStore::new` registering `client.add_message_handler(...)`.
    fn spawn_realtime(api: Arc<AppApi>, cx: &mut Context<Self>) -> Task<()> {
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
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if this
                            .update(cx, |this, cx| {
                                tracing::warn!("ClanList realtime lagged — reloading clans");
                                this.reload(cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
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
                            if this.clans.is_empty() {
                                this.reload(cx);
                            }
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

    /// Fetch the clan list over REST. DTO→domain mapping (`Clan::from`) is owned by the
    /// store, not the UI.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| match api.list_clan_descs().await {
            Ok(clans) => {
                let mapped: Vec<Clan> = clans.into_iter().map(Clan::from).collect();
                let _ = this.update(cx, |this, cx| this.update_clans(mapped, cx));
            }
            Err(e) => tracing::error!("Failed to load clans: {e}"),
        })
        .detach();
    }

    /// Apply a server-pushed realtime event. Cf. `ChannelStore::handle_update_channels`.
    fn handle_event(&mut self, event: RealtimeEvent, cx: &mut Context<Self>) {
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
                let name = (!e.clan_name.is_empty()).then_some(e.clan_name);
                if update_clan(&mut self.clans, &e.clan_id.to_string(), name, e.logo) {
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
                if self.clans.iter().any(|c| c.id == id) {
                    self.reload(cx);
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

    pub fn is_active_clan(&self, clan_id: &str) -> bool {
        self.active_clan_id.as_deref() == Some(clan_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn clans() -> Vec<Clan> {
        vec![
            Clan {
                id: "1".into(),
                name: "One".into(),
                avatar_url: None,
                unread_count: 0,
            },
            Clan {
                id: "2".into(),
                name: "Two".into(),
                avatar_url: Some("old.png".into()),
                unread_count: 0,
            },
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
}
