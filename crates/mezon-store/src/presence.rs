use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};
use mezon_client::RealtimeEvent;

use crate::channel::ChannelList;

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
    _realtime: Task<()>,
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

    fn new(api: Arc<mezon_client::AppApi>, cx: &mut Context<Self>) -> Self {
        let realtime = {
            cx.spawn(async move |this, cx| {
                let mut rx = api.subscribe();
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if this
                                .update(cx, |this, cx| this.handle_event(event, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if this
                                .update(cx, |this, cx| {
                                    tracing::warn!(
                                        "PresenceStore realtime lagged — clearing state"
                                    );
                                    this.typing_by_channel.clear();
                                    this.channel_online.clear();
                                    this.user_online.clear();
                                    cx.emit(PresenceEvent::StatusChanged);
                                    cx.notify();
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
        };

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
            _realtime: realtime,
            _channel_sub: channel_sub,
        }
    }

    fn handle_event(&mut self, event: RealtimeEvent, cx: &mut Context<Self>) {
        match event {
            RealtimeEvent::MessageTyping(e) => {
                let channel_id = e.channel_id.to_string();
                let name = if !e.sender_display_name.is_empty() {
                    e.sender_display_name
                } else if !e.sender_username.is_empty() {
                    e.sender_username
                } else {
                    e.sender_id.to_string()
                };
                let entry = self
                    .typing_by_channel
                    .entry(channel_id.clone())
                    .or_default();
                if e.mode == 0 {
                    entry.insert(name);
                } else {
                    entry.remove(&name);
                    if entry.is_empty() {
                        self.typing_by_channel.remove(&channel_id);
                    }
                }
                cx.emit(PresenceEvent::TypingChanged { channel_id });
                cx.notify();
            }
            RealtimeEvent::ChannelPresence(e) => {
                let channel_id = e.channel_id.to_string();
                let entry = self.channel_online.entry(channel_id.clone()).or_default();
                for join in e.joins {
                    entry.insert(join.user_id.to_string());
                    self.user_online.insert(join.user_id.to_string());
                }
                for leave in e.leaves {
                    let uid = leave.user_id.to_string();
                    entry.remove(&uid);
                    self.user_online.remove(&uid);
                }
                if entry.is_empty() {
                    self.channel_online.remove(&channel_id);
                }
                cx.emit(PresenceEvent::ChannelPresenceChanged { channel_id });
                cx.notify();
            }
            RealtimeEvent::StatusPresence(e) => {
                for join in e.joins {
                    self.user_online.insert(join.user_id.to_string());
                }
                for leave in e.leaves {
                    self.user_online.remove(&leave.user_id.to_string());
                }
                cx.emit(PresenceEvent::StatusChanged);
                cx.notify();
            }
            _ => {}
        }
    }
}
