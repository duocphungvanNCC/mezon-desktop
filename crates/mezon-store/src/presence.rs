use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};
use mezon_client::RealtimeEvent;

use crate::channel::ChannelList;
use crate::realtime::{RealtimeDispatch, RealtimeKind};

const TYPING_NOTIFY_DEBOUNCE_MS: u64 = 250;

#[derive(Debug, Clone)]
pub enum PresenceEvent {
    TypingChanged { channel_id: String },
    ChannelPresenceChanged { channel_id: String },
    StatusChanged,
}

#[derive(Debug)]
pub struct PresenceStore {
    pub typing_by_channel: HashMap<String, HashSet<String>>,
    pub channel_online: HashMap<String, HashSet<String>>,
    pub user_online: HashSet<String>,
    typing_notify_tasks: HashMap<String, Task<()>>,
    _channel_sub: Subscription,
}

struct GlobalPresenceStore(Entity<PresenceStore>);
impl Global for GlobalPresenceStore {}

impl EventEmitter<PresenceEvent> for PresenceStore {}

impl PresenceStore {
    pub fn init(api: Arc<mezon_client::AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalPresenceStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalPresenceStore>().0.clone()
    }

    pub fn typing_users(&self, channel_id: &str) -> Vec<String> {
        self.typing_by_channel
            .get(channel_id)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn new(_api: Arc<mezon_client::AppApi>, cx: &mut Context<Self>) -> Self {
        Self::register_realtime(cx);

        let channel_sub = cx.subscribe(&ChannelList::global(cx), |this, _channel, event, cx| {
            if let crate::channel::ChannelEvent::ActiveChannelChanged(Some(_)) = event {
                this.typing_by_channel.clear();
                cx.emit(PresenceEvent::StatusChanged);
                cx.notify();
            }
        });

        Self {
            typing_by_channel: HashMap::new(),
            channel_online: HashMap::new(),
            user_online: HashSet::new(),
            typing_notify_tasks: HashMap::new(),
            _channel_sub: channel_sub,
        }
    }

    /// Register realtime handlers with the central dispatcher (cf. `add_message_handler`).
    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            for kind in [
                RealtimeKind::MessageTyping,
                RealtimeKind::ChannelPresence,
                RealtimeKind::StatusPresence,
            ] {
                dispatch.on(kind, &entity, |this, event, cx| {
                    this.handle_event(event, cx)
                });
            }
            dispatch.on_lagged(&entity, |this, cx| {
                tracing::warn!("PresenceStore realtime lagged — clearing state");
                this.typing_by_channel.clear();
                this.channel_online.clear();
                this.user_online.clear();
                cx.emit(PresenceEvent::StatusChanged);
                cx.notify();
            });
        });
    }

    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        match event {
            RealtimeEvent::MessageTyping(e) => {
                let cid = e.channel_id.to_string();
                let channel_id = self.apply_typing(
                    &cid,
                    &e.sender_display_name,
                    &e.sender_username,
                    &e.sender_id.to_string(),
                    e.mode,
                );
                cx.emit(PresenceEvent::TypingChanged {
                    channel_id: channel_id.clone(),
                });
                self.schedule_typing_notify(channel_id, cx);
            }
            RealtimeEvent::ChannelPresence(e) => {
                let cid = e.channel_id.to_string();
                let joins: Vec<String> = e.joins.iter().map(|u| u.user_id.to_string()).collect();
                let leaves: Vec<String> = e.leaves.iter().map(|u| u.user_id.to_string()).collect();
                self.apply_channel_presence(&cid, &joins, &leaves);
                cx.emit(PresenceEvent::ChannelPresenceChanged { channel_id: cid });
                cx.notify();
            }
            RealtimeEvent::StatusPresence(e) => {
                let joins: Vec<String> = e.joins.iter().map(|u| u.user_id.to_string()).collect();
                let leaves: Vec<String> = e.leaves.iter().map(|u| u.user_id.to_string()).collect();
                self.apply_status_presence(&joins, &leaves);
                cx.emit(PresenceEvent::StatusChanged);
                cx.notify();
            }
            _ => {}
        }
    }

    fn schedule_typing_notify(&mut self, channel_id: String, cx: &mut Context<Self>) {
        if self.typing_notify_tasks.contains_key(&channel_id) {
            return;
        }
        let delay = Duration::from_millis(TYPING_NOTIFY_DEBOUNCE_MS);
        let cid = channel_id.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |store, cx| {
                store.typing_notify_tasks.remove(&cid);
                cx.notify();
            });
        });
        self.typing_notify_tasks.insert(channel_id, task);
    }

    pub(crate) fn apply_typing(
        &mut self,
        channel_id: &str,
        display_name: &str,
        username: &str,
        sender_id: &str,
        mode: i32,
    ) -> String {
        let channel_id = channel_id.to_string();
        let name = if !display_name.is_empty() {
            display_name.to_owned()
        } else if !username.is_empty() {
            username.to_owned()
        } else {
            sender_id.to_owned()
        };
        let entry = self
            .typing_by_channel
            .entry(channel_id.clone())
            .or_default();
        if mode == 0 {
            entry.insert(name);
        } else {
            entry.remove(&name);
            if entry.is_empty() {
                self.typing_by_channel.remove(&channel_id);
            }
        }
        channel_id
    }

    pub(crate) fn apply_channel_presence(
        &mut self,
        channel_id: &str,
        joins: &[String],
        leaves: &[String],
    ) {
        let channel_id = channel_id.to_string();
        let entry = self.channel_online.entry(channel_id.clone()).or_default();
        for uid in joins {
            entry.insert(uid.clone());
            self.user_online.insert(uid.clone());
        }
        for uid in leaves {
            entry.remove(uid);
            self.user_online.remove(uid);
        }
        if entry.is_empty() {
            self.channel_online.remove(&channel_id);
        }
    }

    pub(crate) fn apply_status_presence(&mut self, joins: &[String], leaves: &[String]) {
        for uid in joins {
            self.user_online.insert(uid.clone());
        }
        for uid in leaves {
            self.user_online.remove(uid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_store() -> PresenceStore {
        PresenceStore {
            typing_by_channel: HashMap::new(),
            channel_online: HashMap::new(),
            user_online: HashSet::new(),
            typing_notify_tasks: HashMap::new(),
            _channel_sub: gpui::Subscription::new(|| {}),
        }
    }

    #[test]
    fn typing_start_adds_user_by_display_name() {
        let mut store = empty_store();
        store.apply_typing("ch1", "Alice", "alice_user", "uid1", 0);
        assert!(store.typing_by_channel["ch1"].contains("Alice"));
    }

    #[test]
    fn typing_start_falls_back_to_username_when_no_display_name() {
        let mut store = empty_store();
        store.apply_typing("ch1", "", "alice_user", "uid1", 0);
        assert!(store.typing_by_channel["ch1"].contains("alice_user"));
    }

    #[test]
    fn typing_start_falls_back_to_sender_id_when_no_name() {
        let mut store = empty_store();
        store.apply_typing("ch1", "", "", "uid1", 0);
        assert!(store.typing_by_channel["ch1"].contains("uid1"));
    }

    #[test]
    fn typing_stop_removes_user_and_cleans_empty_channel() {
        let mut store = empty_store();
        store.apply_typing("ch1", "Alice", "", "", 0);
        store.apply_typing("ch1", "Alice", "", "", 1);
        assert!(!store.typing_by_channel.contains_key("ch1"));
    }

    #[test]
    fn typing_stop_leaves_other_users_in_channel() {
        let mut store = empty_store();
        store.apply_typing("ch1", "Alice", "", "", 0);
        store.apply_typing("ch1", "Bob", "", "", 0);
        store.apply_typing("ch1", "Alice", "", "", 1);
        assert!(!store.typing_by_channel["ch1"].contains("Alice"));
        assert!(store.typing_by_channel["ch1"].contains("Bob"));
    }

    #[test]
    fn channel_presence_join_adds_to_channel_and_global() {
        let mut store = empty_store();
        store.apply_channel_presence("ch1", &["u1".into(), "u2".into()], &[]);
        assert!(store.channel_online["ch1"].contains("u1"));
        assert!(store.user_online.contains("u1"));
        assert!(store.user_online.contains("u2"));
    }

    #[test]
    fn channel_presence_leave_removes_from_channel_and_global() {
        let mut store = empty_store();
        store.apply_channel_presence("ch1", &["u1".into()], &[]);
        store.apply_channel_presence("ch1", &[], &["u1".into()]);
        assert!(!store.channel_online.contains_key("ch1"));
        assert!(!store.user_online.contains("u1"));
    }

    #[test]
    fn channel_presence_empty_channel_cleaned_up() {
        let mut store = empty_store();
        store.apply_channel_presence("ch1", &["u1".into()], &[]);
        store.apply_channel_presence("ch1", &[], &["u1".into()]);
        assert!(!store.channel_online.contains_key("ch1"));
    }

    #[test]
    fn status_presence_join_adds_to_user_online() {
        let mut store = empty_store();
        store.apply_status_presence(&["u1".into(), "u2".into()], &[]);
        assert!(store.user_online.contains("u1"));
        assert!(store.user_online.contains("u2"));
    }

    #[test]
    fn status_presence_leave_removes_from_user_online() {
        let mut store = empty_store();
        store.apply_status_presence(&["u1".into()], &[]);
        store.apply_status_presence(&[], &["u1".into()]);
        assert!(!store.user_online.contains("u1"));
    }

    #[test]
    fn typing_users_returns_vec_for_channel() {
        let mut store = empty_store();
        store.apply_typing("ch1", "Alice", "", "", 0);
        let users = store.typing_users("ch1");
        assert_eq!(users, vec!["Alice".to_string()]);
    }

    #[test]
    fn typing_users_returns_empty_for_unknown_channel() {
        let store = empty_store();
        assert!(store.typing_users("unknown").is_empty());
    }
}
