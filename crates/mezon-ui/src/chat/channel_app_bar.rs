use gpui::{App, FontWeight, div, prelude::*, px};

use crate::app::shell::Shell;
use crate::channel_app::{
    is_channel_app_open_id, launch_channel_app_from_store, reset_channel_app_from_store,
};
use crate::components::primitives::{Icon, IconName};
use crate::theme::Theme;

#[derive(Clone)]
pub struct ChannelAppBarTarget {
    pub app_id: i64,
    pub app_url: String,
    pub app_name: String,
    pub clan_id: mezon_store::ClanId,
    pub clan_name: String,
    pub channel_list: gpui::Entity<mezon_store::ChannelList>,
}

pub fn render_channel_app_bar(
    locale: &str,
    target: ChannelAppBarTarget,
    theme: &Theme,
    cx: &App,
) -> impl IntoElement {
    let app_open = is_channel_app_open_id(target.app_id, cx);
    let launch_label = if app_open {
        mezon_i18n::t(locale, "common.resetApp")
    } else {
        mezon_i18n::t(locale, "common.launchApp")
    };
    let help_label = mezon_i18n::t(locale, "common.help");
    let locale_owned = locale.to_string();

    let launch_target = target;
    let help_locale = locale_owned.clone();

    div()
        .flex()
        .flex_row()
        .gap_2()
        .px_3()
        .pt_2()
        .pb_1()
        .w_full()
        .child(
            div()
                .id("channel-app-launch")
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap_2()
                .py_2()
                .px_2()
                .rounded_md()
                .cursor_pointer()
                .bg(theme.bg_hover)
                .hover(|s| s.bg(theme.bg_primary))
                .on_click(move |_, _, cx| {
                    if app_open {
                        reset_channel_app_from_store(
                            launch_target.app_id,
                            launch_target.app_url.clone(),
                            launch_target.app_name.clone(),
                            launch_target.clan_id,
                            launch_target.clan_name.clone(),
                            launch_target.channel_list.clone(),
                            cx,
                        );
                    } else {
                        launch_channel_app_from_store(
                            launch_target.app_id,
                            launch_target.app_url.clone(),
                            launch_target.app_name.clone(),
                            launch_target.clan_id,
                            launch_target.clan_name.clone(),
                            launch_target.channel_list.clone(),
                            cx,
                        );
                    }
                })
                .child(
                    Icon::new(IconName::Joystick)
                        .size(px(24.))
                        .text_color(theme.text_primary),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_primary)
                        .child(launch_label),
                ),
        )
        .child(
            div()
                .id("channel-app-help")
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap_2()
                .py_2()
                .px_2()
                .rounded_md()
                .cursor_pointer()
                .bg(theme.bg_hover)
                .hover(|s| s.bg(theme.bg_primary))
                .on_click(move |_, window, cx| {
                    Shell::global(cx).update(cx, |shell, cx| {
                        shell.show_coming_soon(help_label, &help_locale, window, cx);
                    });
                })
                .child(
                    Icon::new(IconName::AppHelpIcon)
                        .size(px(24.))
                        .text_color(theme.text_primary),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_primary)
                        .child(help_label),
                ),
        )
}
