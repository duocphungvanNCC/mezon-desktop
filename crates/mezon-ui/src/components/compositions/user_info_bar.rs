use gpui::{App, ClickEvent, Entity, SharedString, Window, div, prelude::*, px};

use crate::components::primitives::{Avatar, Icon, IconName, Sizable, Size};
use crate::theme::Theme;
use mezon_store::{AuthState, PresenceStore};

fn on_settings_click() -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        crate::router::navigate(cx, crate::router::Route::SettingsAccount);
    }
}

pub struct UserInfoBar {
    auth_state: Entity<AuthState>,
    presence: String,
}

impl UserInfoBar {
    pub fn new(auth_state: Entity<AuthState>) -> Self {
        Self {
            auth_state,
            presence: "Offline".to_string(),
        }
    }

    pub fn sync_presence(&mut self, cx: &App) {
        let user_id = match self.auth_state.read(cx) {
            AuthState::Authenticated(session) => session.user_id.clone(),
            _ => {
                self.presence = "Offline".to_string();
                return;
            }
        };
        let online = PresenceStore::global(cx)
            .read(cx)
            .user_online
            .contains(&user_id);
        self.presence = if online {
            "Online".to_string()
        } else {
            "Offline".to_string()
        };
    }

    pub fn render(&self, theme: &Theme, cx: &App) -> impl IntoElement {
        let username = match self.auth_state.read(cx) {
            AuthState::Authenticated(session) => session.username.clone(),
            _ => "Unknown".to_string(),
        };

        let presence_color = match self.presence.as_str() {
            "Online" => theme.status_online,
            "Idle" => theme.status_idle,
            "Dnd" => theme.status_dnd,
            _ => theme.status_offline,
        };

        let mut settings_btn = div()
            .id(SharedString::from("settings-btn"))
            .cursor_pointer()
            .p_1()
            .rounded_md()
            .hover(|s| s.bg(theme.tokens.bg_item_hover))
            .child(
                Icon::new(IconName::SettingProfile)
                    .size(px(20.0))
                    .text_color(theme.text_secondary),
            );
        settings_btn.interactivity().on_click(on_settings_click());

        div()
            .absolute()
            .left(px(12.0))
            .right(px(8.0))
            .bottom(px(12.0))
            .min_h(px(56.0))
            .overflow_hidden()
            .rounded(px(12.0))
            .border_1()
            .border_color(theme.tokens.border_theme_primary)
            .shadow_lg()
            .bg(theme.tokens.bg_surface)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .gap_2()
                    .pl_2()
                    .pr_4()
                    .py_2()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .h(px(40.0))
                            .items_center()
                            .gap_3()
                            .pl_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.tokens.bg_item_hover))
                            .child(
                                div()
                                    .relative()
                                    .child(
                                        Avatar::new().name(username.clone()).with_size(Size::Small),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .bottom_0()
                                            .right_0()
                                            .size_2()
                                            .rounded_full()
                                            .bg(presence_color),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(theme.text_primary)
                                            .child(username),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_muted)
                                            .child(self.presence.clone()),
                                    ),
                            ),
                    )
                    .child(settings_btn),
            )
    }
}
