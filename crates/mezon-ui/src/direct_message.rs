use gpui::{AnyView, Context, Entity, StyleRefinement, Window, div, prelude::*, px};
use mezon_store::{AuthState, ClanList, PresenceEvent, PresenceStore, Settings};

use crate::ClanSidebar;
use crate::components::compositions::user_info_bar::UserInfoBar;
use crate::components::primitives::{Icon, IconName};
use crate::theme::ActiveTheme;

pub struct DirectMessageScreen {
    clan_sidebar: Entity<ClanSidebar>,
    user_info_bar: UserInfoBar,
}

impl DirectMessageScreen {
    pub fn new(
        clan_list: Entity<ClanList>,
        auth_state: Entity<AuthState>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let clan_sidebar = {
            let clan_list = clan_list.clone();
            let settings = settings.clone();
            cx.new(move |cx| ClanSidebar::new(clan_list, settings, cx))
        };

        let user_info_bar = UserInfoBar::new(auth_state.clone());

        cx.subscribe(&PresenceStore::global(cx), |this, _, event, cx| {
            if matches!(event, PresenceEvent::TypingChanged { .. }) {
                return;
            }
            this.user_info_bar.sync_presence(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&auth_state, |this, _, cx| {
            this.user_info_bar.sync_presence(cx);
            cx.notify();
        })
        .detach();

        let mut this = Self {
            clan_sidebar,
            user_info_bar,
        };
        this.user_info_bar.sync_presence(cx);
        this
    }
}

impl Render for DirectMessageScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_row()
            .flex_1()
            .w_full()
            .min_h_0()
            .bg(theme.bg_primary)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(312.0))
                    .h_full()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_1()
                            .min_h_0()
                            .child(
                                div().w(px(72.0)).h_full().child(
                                    AnyView::from(self.clan_sidebar.clone())
                                        .cached(StyleRefinement::default().size_full()),
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .w(px(240.0))
                                    .h_full()
                                    .bg(theme.bg_secondary)
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .w_full()
                                            .px_3()
                                            .py_3()
                                            .child(
                                                div()
                                                    .text_base()
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_color(theme.text_primary)
                                                    .child("Direct Messages"),
                                            ),
                                    ),
                            ),
                    )
                    .child(self.user_info_bar.render(theme, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .bg(theme.bg_primary)
                    .child(
                        div()
                            .flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .flex_col()
                            .gap_4()
                            .child(
                                Icon::new(IconName::CircleUser)
                                    .size_8()
                                    .text_color(theme.text_muted),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text_primary)
                                    .child("DM View"),
                            ),
                    ),
            )
    }
}
