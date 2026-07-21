use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};
use mezon_client::AppApi;
use mezon_client::notification_setting::{ChannelNotificationSetting, MUTE_UNMUTE};

use crate::ids::{ChannelId, ClanId};

pub use mezon_client::notification_setting::{
    MUTE_FOR_1_HOUR_SEC, MUTE_FOR_3_HOURS_SEC, MUTE_FOR_8_HOURS_SEC, MUTE_FOR_15_MINUTES_SEC,
    MUTE_FOR_24_HOURS_SEC, NOTIFICATION_ALL_MESSAGE, NOTIFICATION_MENTION_MESSAGE,
    NOTIFICATION_NOTHING_MESSAGE,
};
pub use mezon_client::notification_setting::{
    MUTE_INFINITY, MUTE_INFINITY as MUTE_FOREVER, NOTIFICATION_DEFAULT,
};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy)]
pub enum NotificationSettingEvent {
    Changed(ChannelId),
}

struct GlobalNotificationSettingStore(Entity<NotificationSettingStore>);
impl Global for GlobalNotificationSettingStore {}

/// Per-channel notification overrides, mirroring React `notificationSettingChannel.slice`.
/// Clan and category defaults are cached alongside so [`ChannelNotificationSetting::is_muted`]
/// can resolve the same three-level fallback the React `MuteButton` uses.
pub struct NotificationSettingStore {
    settings: HashMap<ChannelId, ChannelNotificationSetting>,
    clan_defaults: HashMap<ClanId, i32>,
    in_flight: HashMap<ChannelId, Task<()>>,
    clan_prefetch: HashMap<ClanId, Task<()>>,
    api: Arc<AppApi>,
    _channel_sub: Subscription,
    _clan_sub: Subscription,
}

impl EventEmitter<NotificationSettingEvent> for NotificationSettingStore {}

impl NotificationSettingStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| {
            let channel_sub = cx.subscribe(
                &crate::channel::ChannelList::global(cx),
                |this: &mut Self, _, event: &crate::channel::ChannelEvent, cx| {
                    if let crate::channel::ChannelEvent::ActiveChannelChanged(Some(channel_id)) =
                        event
                    {
                        this.ensure_loaded(*channel_id, cx);
                    }
                },
            );
            let clan_sub = cx.subscribe(
                &crate::clan::ClanList::global(cx),
                |this: &mut Self, _, event: &crate::clan::ClanEvent, cx| {
                    if let crate::clan::ClanEvent::ActiveClanChanged(Some(clan_id)) = event {
                        this.prefetch_clan(*clan_id, cx);
                    }
                },
            );
            Self {
                settings: HashMap::new(),
                clan_defaults: HashMap::new(),
                in_flight: HashMap::new(),
                clan_prefetch: HashMap::new(),
                api,
                _channel_sub: channel_sub,
                _clan_sub: clan_sub,
            }
        });
        cx.set_global(GlobalNotificationSettingStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalNotificationSettingStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalNotificationSettingStore>()
            .map(|g| g.0.clone())
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.settings.clear();
        self.clan_defaults.clear();
        self.in_flight.clear();
        self.clan_prefetch.clear();
        cx.notify();
    }

    pub fn setting(&self, channel_id: ChannelId) -> Option<ChannelNotificationSetting> {
        self.settings.get(&channel_id).copied()
    }

    pub fn level(&self, channel_id: ChannelId) -> i32 {
        self.setting(channel_id)
            .map(|s| s.level)
            .unwrap_or(NOTIFICATION_DEFAULT)
    }

    pub fn is_muted(&self, channel_id: ChannelId, clan_id: ClanId) -> bool {
        self.setting(channel_id).is_some_and(|setting| {
            setting.is_muted(None, self.clan_defaults.get(&clan_id).copied())
        })
    }

    pub fn muted_until_ms(&self, channel_id: ChannelId) -> Option<i64> {
        self.setting(channel_id)
            .and_then(|s| s.muted_until_ms(now_ms()))
    }

    pub fn clan_default(&self, clan_id: ClanId) -> Option<i32> {
        self.clan_defaults.get(&clan_id).copied()
    }

    pub fn set_clan_default(&mut self, clan_id: ClanId, level: i32) {
        self.clan_defaults.insert(clan_id, level);
    }

    /// Mirrors React `changeCurrentClan`, which batches `fetchMutedChannels` and
    /// `getDefaultNotificationClan` so the bell icon is correct before it is opened.
    pub fn prefetch_clan(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        if clan_id.is_zero() || self.clan_prefetch.contains_key(&clan_id) {
            return;
        }
        let api = self.api.clone();
        let task = cx.spawn(async move |this, cx| {
            let muted = api.list_muted_channels(clan_id.get()).await;
            let clan_default = api.get_notification_clan(clan_id.get()).await;
            let _ = this.update(cx, |store, cx| {
                store.clan_prefetch.remove(&clan_id);
                match muted {
                    Ok(ids) => store.seed_muted_channels(&ids),
                    Err(e) => tracing::warn!(
                        clan_id = clan_id.get(),
                        "failed to list muted channels: {e}"
                    ),
                }
                match clan_default {
                    Ok(level) => store.set_clan_default(clan_id, level),
                    Err(e) => tracing::warn!(
                        clan_id = clan_id.get(),
                        "failed to load clan notification default: {e}"
                    ),
                }
                cx.notify();
            });
        });
        self.clan_prefetch.insert(clan_id, task);
    }

    /// `list_muted_channels` returns ids only, so it can establish that a channel is
    /// muted but not when the mute expires; the exact expiry arrives with
    /// [`Self::ensure_loaded`] for the channel actually being opened.
    fn seed_muted_channels(&mut self, muted_ids: &[String]) {
        for id in muted_ids {
            let Ok(raw) = id.parse::<i64>() else {
                continue;
            };
            let channel_id = ChannelId(raw);
            if channel_id.is_zero() {
                continue;
            }
            let entry = self.settings.entry(channel_id).or_default();
            entry.id = raw;
            if entry.mute_until_ms == 0 {
                entry.mute_until_ms = i64::from(MUTE_INFINITY);
            }
        }
    }

    /// Load the override for a channel unless a request is already in flight.
    pub fn ensure_loaded(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if channel_id.is_zero()
            || self.settings.contains_key(&channel_id)
            || self.in_flight.contains_key(&channel_id)
        {
            return;
        }
        let api = self.api.clone();
        let task = cx.spawn(async move |this, cx| {
            let fetched = api.get_notification_channel(channel_id.get()).await;
            let _ = this.update(cx, |store, cx| {
                store.in_flight.remove(&channel_id);
                match fetched {
                    Ok(setting) => {
                        store.settings.insert(channel_id, setting);
                        cx.emit(NotificationSettingEvent::Changed(channel_id));
                        cx.notify();
                    }
                    Err(e) => {
                        tracing::warn!(
                            channel_id = channel_id.get(),
                            "failed to load channel notification setting: {e}"
                        );
                    }
                }
            });
        });
        self.in_flight.insert(channel_id, task);
    }

    pub fn set_level(
        &mut self,
        channel_id: ChannelId,
        clan_id: ClanId,
        level: i32,
        cx: &mut Context<Self>,
    ) {
        let previous = self.setting(channel_id);
        let entry = self.settings.entry(channel_id).or_default();
        entry.level = level;
        if level != NOTIFICATION_DEFAULT {
            entry.id = channel_id.get();
        }
        cx.emit(NotificationSettingEvent::Changed(channel_id));
        cx.notify();

        let api = self.api.clone();
        let reset_to_default = level == NOTIFICATION_DEFAULT;
        cx.spawn(async move |this, cx| {
            let result = if reset_to_default {
                api.delete_notification_channel(channel_id.get()).await
            } else {
                api.set_notification_channel_setting(channel_id.get(), level, clan_id.get())
                    .await
            };
            if let Err(e) = result {
                tracing::warn!(
                    channel_id = channel_id.get(),
                    "failed to save notification level, rolling back: {e}"
                );
                let _ = this.update(cx, |store, cx| {
                    store.restore(channel_id, previous);
                    cx.emit(NotificationSettingEvent::Changed(channel_id));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn set_mute(
        &mut self,
        channel_id: ChannelId,
        clan_id: ClanId,
        mute_seconds: i32,
        cx: &mut Context<Self>,
    ) {
        let previous = self.setting(channel_id);
        let expiry = ChannelNotificationSetting::expiry_from_duration(now_ms(), mute_seconds);
        let entry = self.settings.entry(channel_id).or_default();
        entry.mute_until_ms = expiry;
        if mute_seconds == MUTE_UNMUTE {
            entry.id = 0;
        } else {
            entry.id = channel_id.get();
        }
        cx.emit(NotificationSettingEvent::Changed(channel_id));
        cx.notify();

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            if let Err(e) = api
                .set_mute_channel(channel_id.get(), mute_seconds, clan_id.get())
                .await
            {
                tracing::warn!(
                    channel_id = channel_id.get(),
                    "failed to save channel mute, rolling back: {e}"
                );
                let _ = this.update(cx, |store, cx| {
                    store.restore(channel_id, previous);
                    cx.emit(NotificationSettingEvent::Changed(channel_id));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn mute_forever(&mut self, channel_id: ChannelId, clan_id: ClanId, cx: &mut Context<Self>) {
        self.set_mute(channel_id, clan_id, MUTE_INFINITY, cx);
    }

    pub fn unmute(&mut self, channel_id: ChannelId, clan_id: ClanId, cx: &mut Context<Self>) {
        self.set_mute(channel_id, clan_id, MUTE_UNMUTE, cx);
    }

    fn restore(&mut self, channel_id: ChannelId, previous: Option<ChannelNotificationSetting>) {
        match previous {
            Some(setting) => {
                self.settings.insert(channel_id, setting);
            }
            None => {
                self.settings.remove(&channel_id);
            }
        }
    }
}
