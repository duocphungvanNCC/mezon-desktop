use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, Global, Task};
use mezon_client::transport::ApiDirectChannel;
use mezon_client::{AppApi, ConnectionStatus, RealtimeEvent};

use crate::realtime::{RealtimeDispatch, RealtimeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectKind {
    Dm,
    Group,
}

impl DirectKind {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            2 => DirectKind::Group,
            _ => DirectKind::Dm,
        }
    }

    /// `ChannelStreamMode` used when sending (DM = 4, GROUP = 3).
    pub fn stream_mode(self) -> i32 {
        match self {
            DirectKind::Dm => 4,
            DirectKind::Group => 3,
        }
    }

    /// `ChannelType` used when joining the socket room (DM = 3, GROUP = 2).
    pub fn channel_type(self) -> i32 {
        match self {
            DirectKind::Dm => 3,
            DirectKind::Group => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirectChannel {
    pub id: String,
    pub label: String,
    pub kind: DirectKind,
    pub avatar: String,
    pub peer_user_id: Option<String>,
    pub online: bool,
    pub member_count: u32,
    pub last_sent_timestamp: i64,
    pub last_seen_timestamp: i64,
}

impl DirectChannel {
    pub fn is_unread(&self) -> bool {
        self.last_sent_timestamp > 0 && self.last_seen_timestamp < self.last_sent_timestamp
    }
}

/// Holds the user's direct-message / group conversations (clan_id = 0). Self-subscribes to the
/// realtime broadcast (cf. `ChannelStore`): fetches the list on connect and keeps it ordered by
/// most-recent activity.
pub struct DirectMessageStore {
    channels: Vec<DirectChannel>,
    loading: bool,
    api: Arc<AppApi>,
    _conn_watch: Task<()>,
}

struct GlobalDirectMessageStore(Entity<DirectMessageStore>);
impl Global for GlobalDirectMessageStore {}

impl DirectMessageStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalDirectMessageStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalDirectMessageStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalDirectMessageStore>()
            .map(|g| g.0.clone())
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        Self::register_realtime(cx);
        let conn_watch = Self::spawn_connection_watch(api.clone(), cx);
        Self {
            channels: Vec::new(),
            loading: false,
            api,
            _conn_watch: conn_watch,
        }
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(RealtimeKind::ChannelMessage, &entity, |this, event, cx| {
                this.handle_event(event, cx)
            });
            dispatch.on_lagged(&entity, |this, cx| this.refresh(cx));
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
                    if this.update(cx, |this, cx| this.fetch(cx)).is_err() {
                        break;
                    }
                } else if !connected {
                    was_connected = false;
                }
            }
        })
    }

    pub fn channels(&self) -> &[DirectChannel] {
        &self.channels
    }

    pub fn find(&self, id: &str) -> Option<&DirectChannel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.fetch(cx);
    }

    /// Fetch the DM list lazily on navigation: refetches while the list is still empty (covers a
    /// missed/failed connect-time fetch) but stops once we have data.
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if !self.loading && self.channels.is_empty() {
            self.fetch(cx);
        }
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.list_dm_channels().await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(list) => {
                        tracing::info!("DirectMessageStore: fetched {} DM channels", list.len());
                        let mut channels: Vec<DirectChannel> =
                            list.into_iter().map(direct_from_api).collect();
                        sort_by_recent(&mut channels);
                        this.channels = channels;
                        cx.notify();
                    }
                    Err(e) => tracing::error!("list_dm_channels failed: {e}"),
                }
            });
        })
        .detach();
    }

    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::ChannelMessage(m) = event else {
            return;
        };
        let id = m.channel_id.to_string();
        let Some(pos) = self.channels.iter().position(|c| c.id == id) else {
            return;
        };
        if m.create_time_seconds > 0 {
            self.channels[pos].last_sent_timestamp = i64::from(m.create_time_seconds);
        }
        sort_by_recent(&mut self.channels);
        cx.notify();
    }
}

fn sort_by_recent(channels: &mut [DirectChannel]) {
    channels.sort_by_key(|c| std::cmp::Reverse(c.last_sent_timestamp));
}

fn dm_peer_index(c: &ApiDirectChannel) -> usize {
    c.avatars
        .iter()
        .enumerate()
        .rfind(|(_, avatar)| !avatar.is_empty())
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn direct_from_api(c: ApiDirectChannel) -> DirectChannel {
    let kind = DirectKind::from_raw(c.channel_type);
    let (avatar, peer_user_id, online) = match kind {
        DirectKind::Group => (c.channel_avatar.clone(), None, false),
        DirectKind::Dm => {
            let peer_idx = dm_peer_index(&c);
            (
                c.avatars
                    .get(peer_idx)
                    .filter(|avatar| !avatar.is_empty())
                    .cloned()
                    .unwrap_or_default(),
                c.user_ids.get(peer_idx).cloned(),
                c.onlines.get(peer_idx).copied().unwrap_or(false),
            )
        }
    };
    let label = if !c.channel_label.is_empty() {
        c.channel_label.clone()
    } else if !c.display_names.is_empty() {
        c.display_names.join(", ")
    } else {
        c.usernames.join(", ")
    };
    DirectChannel {
        id: c.channel_id,
        label,
        kind,
        avatar,
        peer_user_id,
        online,
        member_count: c.member_count.max(0) as u32,
        last_sent_timestamp: c.last_sent_timestamp,
        last_seen_timestamp: c.last_seen_timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_dm(id: &str, label: &str, ty: u32) -> ApiDirectChannel {
        ApiDirectChannel {
            channel_id: id.into(),
            channel_label: label.into(),
            channel_type: ty,
            channel_avatar: String::new(),
            avatars: vec!["peer.png".into()],
            usernames: vec!["peer".into()],
            display_names: vec!["Peer".into()],
            user_ids: vec!["42".into()],
            onlines: vec![true],
            member_count: 2,
            count_mess_unread: 0,
            last_sent_timestamp: 0,
            last_seen_timestamp: 0,
        }
    }

    #[test]
    fn dm_maps_peer_avatar_and_id() {
        let dm = direct_from_api(api_dm("1", "Peer", 3));
        assert_eq!(dm.kind, DirectKind::Dm);
        assert_eq!(dm.avatar, "peer.png");
        assert_eq!(dm.peer_user_id.as_deref(), Some("42"));
        assert!(dm.online);
    }

    #[test]
    fn dm_uses_last_nonempty_avatar_when_multiple() {
        let mut api = api_dm("1", "Peer", 3);
        api.avatars = vec!["".into(), "peer.png".into()];
        api.user_ids = vec!["self".into(), "42".into()];
        api.onlines = vec![true, false];
        let dm = direct_from_api(api);
        assert_eq!(dm.avatar, "peer.png");
        assert_eq!(dm.peer_user_id.as_deref(), Some("42"));
        assert!(!dm.online);
    }

    #[test]
    fn dm_skips_trailing_empty_avatar() {
        let mut api = api_dm("1", "Peer", 3);
        api.avatars = vec!["peer.png".into(), String::new()];
        let dm = direct_from_api(api);
        assert_eq!(dm.avatar, "peer.png");
    }

    #[test]
    fn group_uses_channel_avatar_and_no_peer() {
        let mut api = api_dm("2", "Group", 2);
        api.channel_avatar = "group.png".into();
        let group = direct_from_api(api);
        assert_eq!(group.kind, DirectKind::Group);
        assert_eq!(group.avatar, "group.png");
        assert_eq!(group.peer_user_id, None);
    }

    #[test]
    fn empty_label_falls_back_to_display_names() {
        let api = api_dm("3", "", 3);
        let dm = direct_from_api(api);
        assert_eq!(dm.label, "Peer");
    }

    #[test]
    fn stream_mode_and_channel_type_match_react() {
        assert_eq!(DirectKind::Dm.stream_mode(), 4);
        assert_eq!(DirectKind::Group.stream_mode(), 3);
        assert_eq!(DirectKind::Dm.channel_type(), 3);
        assert_eq!(DirectKind::Group.channel_type(), 2);
    }

    #[test]
    fn sort_orders_most_recent_first() {
        let mut chans = vec![
            DirectChannel {
                id: "a".into(),
                label: "a".into(),
                kind: DirectKind::Dm,
                avatar: String::new(),
                peer_user_id: None,
                online: false,
                member_count: 0,
                last_sent_timestamp: 100,
                last_seen_timestamp: 0,
            },
            DirectChannel {
                id: "b".into(),
                label: "b".into(),
                kind: DirectKind::Dm,
                avatar: String::new(),
                peer_user_id: None,
                online: false,
                member_count: 0,
                last_sent_timestamp: 200,
                last_seen_timestamp: 0,
            },
        ];
        sort_by_recent(&mut chans);
        assert_eq!(chans[0].id, "b");
        assert_eq!(chans[1].id, "a");
    }
}
