use mezon_proto::api;

pub const NOTIFICATION_DEFAULT: i32 = 0;
pub const NOTIFICATION_ALL_MESSAGE: i32 = 1;
pub const NOTIFICATION_MENTION_MESSAGE: i32 = 2;
pub const NOTIFICATION_NOTHING_MESSAGE: i32 = 3;

pub const MUTE_UNMUTE: i32 = 0;
pub const MUTE_INFINITY: i32 = -1;

pub const MUTE_FOR_15_MINUTES_SEC: i32 = 15 * 60;
pub const MUTE_FOR_1_HOUR_SEC: i32 = 60 * 60;
pub const MUTE_FOR_3_HOURS_SEC: i32 = 3 * 60 * 60;
pub const MUTE_FOR_8_HOURS_SEC: i32 = 8 * 60 * 60;
pub const MUTE_FOR_24_HOURS_SEC: i32 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelNotificationSetting {
    pub id: i64,
    pub level: i32,
    pub mute_until_ms: i64,
}

impl ChannelNotificationSetting {
    pub fn from_api(dto: &api::NotificationUserChannel) -> Self {
        Self {
            id: dto.id,
            level: dto.notification_setting_type,
            mute_until_ms: i64::from(dto.time_mute_seconds),
        }
    }

    /// React stores `Date.now() + duration*1000` on write, so this field is an epoch
    /// in milliseconds despite the wire name. `MUTE_INFINITY` never yields a date.
    pub fn muted_until_ms(&self, now_ms: i64) -> Option<i64> {
        (self.mute_until_ms > now_ms).then_some(self.mute_until_ms)
    }

    pub fn expiry_from_duration(now_ms: i64, mute_seconds: i32) -> i64 {
        match mute_seconds {
            MUTE_UNMUTE => 0,
            MUTE_INFINITY => i64::from(MUTE_INFINITY),
            seconds => now_ms + i64::from(seconds) * 1000,
        }
    }

    /// Mirrors React `MuteButton`: an explicit per-channel override wins, otherwise
    /// the category default is consulted, then the clan default.
    pub fn is_muted(&self, category_default: Option<i32>, clan_default: Option<i32>) -> bool {
        if self.mute_until_ms == 0 {
            if self.level == NOTIFICATION_NOTHING_MESSAGE {
                return true;
            }
        } else if self.id != 0 {
            return true;
        }
        if self.id != 0 {
            return false;
        }
        match category_default {
            Some(level) => level == NOTIFICATION_NOTHING_MESSAGE,
            None => clan_default == Some(NOTIFICATION_NOTHING_MESSAGE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(id: i64, level: i32, mute_until_ms: i64) -> ChannelNotificationSetting {
        ChannelNotificationSetting {
            id,
            level,
            mute_until_ms,
        }
    }

    #[test]
    fn expiry_from_duration_matches_react_write_semantics() {
        let now = 1_700_000_000_000;
        assert_eq!(
            ChannelNotificationSetting::expiry_from_duration(now, MUTE_FOR_15_MINUTES_SEC),
            now + 900_000
        );
        assert_eq!(
            ChannelNotificationSetting::expiry_from_duration(now, MUTE_UNMUTE),
            0
        );
        assert_eq!(
            ChannelNotificationSetting::expiry_from_duration(now, MUTE_INFINITY),
            -1
        );
    }

    #[test]
    fn muted_until_is_hidden_when_expired_or_infinite() {
        let now = 1_700_000_000_000;
        assert_eq!(
            setting(5, NOTIFICATION_DEFAULT, now + 60_000).muted_until_ms(now),
            Some(now + 60_000)
        );
        assert_eq!(
            setting(5, NOTIFICATION_DEFAULT, now - 60_000).muted_until_ms(now),
            None
        );
        assert_eq!(
            setting(5, NOTIFICATION_DEFAULT, -1).muted_until_ms(now),
            None
        );
        assert_eq!(
            setting(0, NOTIFICATION_DEFAULT, 0).muted_until_ms(now),
            None
        );
    }

    #[test]
    fn nothing_message_without_mute_timer_is_muted() {
        assert!(setting(5, NOTIFICATION_NOTHING_MESSAGE, 0).is_muted(None, None));
    }

    #[test]
    fn override_with_mute_timer_is_muted() {
        assert!(setting(5, NOTIFICATION_ALL_MESSAGE, 900).is_muted(None, None));
    }

    #[test]
    fn explicit_override_ignores_category_and_clan_defaults() {
        let s = setting(5, NOTIFICATION_ALL_MESSAGE, 0);
        assert!(!s.is_muted(Some(NOTIFICATION_NOTHING_MESSAGE), None));
        assert!(!s.is_muted(None, Some(NOTIFICATION_NOTHING_MESSAGE)));
    }

    #[test]
    fn without_override_category_default_decides() {
        let s = setting(0, NOTIFICATION_DEFAULT, 0);
        assert!(s.is_muted(Some(NOTIFICATION_NOTHING_MESSAGE), None));
        assert!(!s.is_muted(Some(NOTIFICATION_ALL_MESSAGE), None));
    }

    #[test]
    fn without_override_or_category_clan_default_decides() {
        let s = setting(0, NOTIFICATION_DEFAULT, 0);
        assert!(s.is_muted(None, Some(NOTIFICATION_NOTHING_MESSAGE)));
        assert!(!s.is_muted(None, Some(NOTIFICATION_ALL_MESSAGE)));
        assert!(!s.is_muted(None, None));
    }

    #[test]
    fn from_api_maps_every_field() {
        let dto = api::NotificationUserChannel {
            id: 7,
            notification_setting_type: NOTIFICATION_MENTION_MESSAGE,
            time_mute_seconds: 3600,
            ..Default::default()
        };
        assert_eq!(
            ChannelNotificationSetting::from_api(&dto),
            setting(7, NOTIFICATION_MENTION_MESSAGE, 3600)
        );
    }
}
