use gpui::{App, ClickEvent, Entity, Pixels, Point, WeakEntity, Window};
use mezon_store::{ChannelId, ChannelList, ChannelType, ClanId};

use super::ChannelSidebar;
use crate::app::shell::Shell;
use crate::components::primitives::{ContextMenu, SubmenuOption};

pub(super) fn on_channel_click(
    channel_id: String,
    clan_id: Option<ClanId>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        if let Some(ref cid) = clan_id {
            crate::router::navigate(
                cx,
                crate::router::Route::Channel {
                    clan_id: *cid,
                    channel_id: channel_id.parse().unwrap_or_default(),
                },
            );
        }
    }
}

pub(super) fn on_category_click(
    channel_list: Entity<ChannelList>,
    clan_id: ClanId,
    category_id: String,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        channel_list.update(cx, |m, cx| {
            m.toggle_category(clan_id, &category_id, cx);
        });
    }
}

#[derive(Clone)]
pub(super) struct OpenMenu {
    pub(super) channel_type: ChannelType,
    pub(super) is_thread: bool,
    pub(super) position: Point<Pixels>,
    pub(super) channel_id: ChannelId,
    pub(super) clan_id: ClanId,
    pub(super) mute_sub_open: bool,
    pub(super) noti_sub_open: bool,
}

fn coming_soon_toast(message: String) -> impl Fn(&mut Window, &mut App) + 'static {
    move |_window: &mut Window, cx: &mut App| {
        let message = message.clone();
        Shell::global(cx).update(cx, move |shell, cx| shell.info(message, cx));
    }
}

fn coming_soon_modal(title: String, locale: String) -> impl Fn(&mut Window, &mut App) + 'static {
    move |window: &mut Window, cx: &mut App| {
        let title = title.clone();
        let locale = locale.clone();
        Shell::global(cx).update(cx, |shell, cx| {
            shell.show_coming_soon(title, &locale, window, cx);
        });
    }
}

const MUTE_DURATIONS: &[(i32, &str)] = &[
    (
        mezon_store::notification_setting::MUTE_FOR_15_MINUTES_SEC,
        "channelMenu.menu.notification.for15Minutes",
    ),
    (
        mezon_store::notification_setting::MUTE_FOR_1_HOUR_SEC,
        "channelMenu.menu.notification.for1Hour",
    ),
    (
        mezon_store::notification_setting::MUTE_FOR_3_HOURS_SEC,
        "channelMenu.menu.notification.for3Hours",
    ),
    (
        mezon_store::notification_setting::MUTE_FOR_8_HOURS_SEC,
        "channelMenu.menu.notification.for8Hours",
    ),
    (
        mezon_store::notification_setting::MUTE_FOR_24_HOURS_SEC,
        "channelMenu.menu.notification.for24Hours",
    ),
    (
        mezon_store::notification_setting::MUTE_FOREVER,
        "channelMenu.menu.notification.untilTurnedBackOn",
    ),
];

const NOTI_LEVELS: &[(i32, &str)] = &[
    (
        mezon_store::notification_setting::NOTIFICATION_DEFAULT,
        "channelMenu.menu.notification.useCategoryDefault",
    ),
    (
        mezon_store::notification_setting::NOTIFICATION_ALL_MESSAGE,
        "channelMenu.menu.notification.all",
    ),
    (
        mezon_store::notification_setting::NOTIFICATION_MENTION_MESSAGE,
        "channelMenu.menu.notification.onlyMention",
    ),
    (
        mezon_store::notification_setting::NOTIFICATION_NOTHING_MESSAGE,
        "channelMenu.menu.notification.nothing",
    ),
];

/// Mirrors React `PanelChannel`: the row's subText shows the level actually in
/// effect, resolving `DEFAULT` through the clan default.
fn effective_level_label(locale: &str, level: i32, clan_default: Option<i32>) -> String {
    use mezon_store::notification_setting as ns;
    let effective = if level == ns::NOTIFICATION_DEFAULT {
        clan_default.unwrap_or(ns::NOTIFICATION_DEFAULT)
    } else {
        level
    };
    let key = match effective {
        v if v == ns::NOTIFICATION_ALL_MESSAGE => "channelMenu.menu.notification.all",
        v if v == ns::NOTIFICATION_MENTION_MESSAGE => "channelMenu.menu.notification.onlyMention",
        v if v == ns::NOTIFICATION_NOTHING_MESSAGE => "channelMenu.menu.notification.nothing",
        _ => "channelMenu.menu.notification.useCategoryDefault",
    };
    mezon_i18n::t(locale, key).to_string()
}

fn submenu_options(
    locale: &str,
    source: &[(i32, &'static str)],
    selected: i32,
) -> Vec<SubmenuOption> {
    source
        .iter()
        .map(|(value, key)| SubmenuOption {
            value: *value,
            label: mezon_i18n::t(locale, key).into(),
            selected: *value == selected,
        })
        .collect()
}

fn set_submenu_open(
    sidebar: WeakEntity<ChannelSidebar>,
    mute: bool,
) -> impl Fn(&mut Window, &mut App) + 'static {
    move |_window: &mut Window, cx: &mut App| {
        let _ = sidebar.update(cx, |this, cx| {
            if let Some(menu) = this.open_menu.as_mut() {
                let (m, n) = if mute { (true, false) } else { (false, true) };
                if menu.mute_sub_open != m || menu.noti_sub_open != n {
                    menu.mute_sub_open = m;
                    menu.noti_sub_open = n;
                    cx.notify();
                }
            }
        });
    }
}

fn apply_mute(
    channel_id: ChannelId,
    clan_id: ClanId,
) -> impl Fn(i32, &mut Window, &mut App) + 'static {
    move |seconds: i32, _window: &mut Window, cx: &mut App| {
        if let Some(store) = mezon_store::NotificationSettingStore::try_global(cx) {
            store.update(cx, |store, cx| {
                store.set_mute(channel_id, clan_id, seconds, cx)
            });
        }
    }
}

fn apply_level(
    channel_id: ChannelId,
    clan_id: ClanId,
) -> impl Fn(i32, &mut Window, &mut App) + 'static {
    move |level: i32, _window: &mut Window, cx: &mut App| {
        if let Some(store) = mezon_store::NotificationSettingStore::try_global(cx) {
            store.update(cx, |store, cx| {
                store.set_level(channel_id, clan_id, level, cx)
            });
        }
    }
}

fn mute_label(locale: &str, is_thread: bool, muted: bool) -> String {
    let key = match (is_thread, muted) {
        (true, true) => "channelMenu.menu.notification.unmuteThreadStatus",
        (true, false) => "channelMenu.menu.notification.muteThreadStatus",
        (false, true) => "channelMenu.menu.notification.unmuteChannelStatus",
        (false, false) => "channelMenu.menu.notification.muteChannelStatus",
    };
    mezon_i18n::t(locale, key).to_string()
}

pub(super) fn build_channel_menu(
    sidebar: WeakEntity<ChannelSidebar>,
    locale: &str,
    channel_type: ChannelType,
    is_thread: bool,
    channel_id: ChannelId,
    clan_id: ClanId,
    muted: bool,
    muted_until: Option<String>,
    level: i32,
    mute_sub_open: bool,
    noti_sub_open: bool,
    clan_default: Option<i32>,
) -> ContextMenu {
    let t = |key: &'static str| mezon_i18n::t(locale, key).to_string();
    let coming_soon = t("common.comingSoon");
    let locale_owned = locale.to_string();
    let sidebar_dismiss = sidebar.clone();

    let mut menu = ContextMenu::new().on_dismiss(move |_window, cx| {
        if let Some(view) = sidebar_dismiss.upgrade() {
            view.update(cx, |this, cx| {
                this.open_menu = None;
                cx.notify();
            });
        }
    });

    menu = menu
        .item(
            t("channelMenu.menu.watchMenu.markAsRead"),
            coming_soon_toast(coming_soon.clone()),
        )
        .separator()
        .item(
            t("channelMenu.menu.inviteMenu.copyLink"),
            coming_soon_toast(coming_soon.clone()),
        )
        .separator();

    if is_thread {
        let edit_label = t("channelMenu.menu.manageThreadMenu.editThread");
        let leave_label = t("channelMenu.menu.manageThreadMenu.leaveThread");
        let delete_label = t("channelMenu.menu.manageThreadMenu.deleteThread");
        menu = menu
            .item(
                t("channelMenu.menu.notification.archiveThread"),
                coming_soon_modal(
                    t("channelMenu.menu.notification.archiveThread"),
                    locale_owned.clone(),
                ),
            )
            .submenu(
                mute_label(locale, true, muted),
                muted_until.clone().map(Into::into),
                submenu_options(locale, MUTE_DURATIONS, -2),
                mute_sub_open,
                set_submenu_open(sidebar.clone(), true),
                apply_mute(channel_id, clan_id),
            )
            .submenu(
                t("channelMenu.menu.notification.notification"),
                Some(effective_level_label(locale, level, clan_default).into()),
                submenu_options(locale, NOTI_LEVELS, level),
                noti_sub_open,
                set_submenu_open(sidebar.clone(), false),
                apply_level(channel_id, clan_id),
            )
            .danger_item(
                leave_label.clone(),
                coming_soon_modal(leave_label, locale_owned.clone()),
            )
            .separator()
            .item(
                edit_label.clone(),
                coming_soon_modal(edit_label, locale_owned.clone()),
            )
            .danger_item(
                delete_label.clone(),
                coming_soon_modal(delete_label, locale_owned.clone()),
            );
    } else {
        let edit_label = t("channelMenu.menu.organizationMenu.edit");
        let delete_label = t("channelMenu.menu.organizationMenu.deleteChannel");
        menu = menu
            .item(
                t("channelMenu.menu.notification.archiveChannel"),
                coming_soon_modal(
                    t("channelMenu.menu.notification.archiveChannel"),
                    locale_owned.clone(),
                ),
            )
            .submenu(
                mute_label(locale, false, muted),
                muted_until.clone().map(Into::into),
                submenu_options(locale, MUTE_DURATIONS, -2),
                mute_sub_open,
                set_submenu_open(sidebar.clone(), true),
                apply_mute(channel_id, clan_id),
            )
            .submenu(
                t("channelMenu.menu.notification.notification"),
                Some(effective_level_label(locale, level, clan_default).into()),
                submenu_options(locale, NOTI_LEVELS, level),
                noti_sub_open,
                set_submenu_open(sidebar.clone(), false),
                apply_level(channel_id, clan_id),
            )
            .item(
                t("channelMenu.menu.inviteMenu.markFavorite"),
                coming_soon_toast(coming_soon.clone()),
            )
            .separator()
            .item(
                edit_label.clone(),
                coming_soon_modal(edit_label, locale_owned.clone()),
            );

        let create_label = match channel_type {
            ChannelType::Voice => Some(t("channelMenu.menu.organizationMenu.createVoiceChannel")),
            ChannelType::Text => Some(t("channelMenu.menu.organizationMenu.createTextChannel")),
            _ => None,
        };
        if let Some(create_label) = create_label {
            menu = menu.item(
                create_label.clone(),
                coming_soon_modal(create_label, locale_owned.clone()),
            );
        }

        menu = menu.danger_item(
            delete_label.clone(),
            coming_soon_modal(delete_label, locale_owned.clone()),
        );
    }

    menu
}
