use crate::components::primitives::{Label, Switch, h_flex, v_flex};
use crate::theme::ActiveTheme;
use gpui::{Context, Entity, FontWeight, Window, prelude::*};
use mezon_store::Settings;

pub struct NotificationsPage {
    settings: Entity<Settings>,
}

impl NotificationsPage {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        Self { settings }
    }
}

impl Render for NotificationsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let hide_content = self.settings.read(cx).notifications_hide_content;
        let description_key = if hide_content {
            "common.hideNotificationDesc"
        } else {
            "common.showNotificationDesc"
        };

        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_4()
            .rounded_lg()
            .bg(theme.tokens.theme_setting_nav)
            .border_1()
            .border_color(theme.border)
            .p_4()
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        Label::new(mezon_i18n::t(&locale, "common.hideNotificationsContent"))
                            .text_base()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text_primary),
                    )
                    .child(
                        Label::new(mezon_i18n::t(&locale, description_key))
                            .text_sm()
                            .text_color(theme.tokens.text_theme_primary),
                    ),
            )
            .child(
                Switch::new("hide-notification-content")
                    .checked(hide_content)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.update(cx, |s, _| {
                            s.notifications_hide_content = !s.notifications_hide_content;
                        });
                        mezon_store::schedule_settings_save(&this.settings, cx);
                        cx.notify();
                    })),
            )
    }
}
