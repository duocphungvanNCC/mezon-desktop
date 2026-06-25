use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Entity, ListState, SharedString, Subscription, Window, div, img,
    list, prelude::*, px,
};
use mezon_store::{ClanId, ClanList, Settings};
use ui::Tooltip;

use crate::app::shell::Shell;
use crate::components::primitives::{Icon, IconName};
use crate::router::{Route, Router};
use crate::theme::ActiveTheme;

mod clan_row;
use clan_row::{ClanRow, render_clan_row, render_pill};

pub struct ClanSidebar {
    clan_list: Entity<ClanList>,
    settings: Entity<Settings>,
    rows: Rc<Vec<ClanRow>>,
    list_state: ListState,
    active_clan_id: Option<ClanId>,
    dm_active: bool,
    _clan_sub: Subscription,
    _settings_sub: Subscription,
    _router_sub: Subscription,
}

impl ClanSidebar {
    pub fn new(
        clan_list: Entity<ClanList>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let clan_sub = cx.observe(&clan_list, |this, clan_list, cx| {
            this.sync_rows(clan_list.read(cx));
            cx.notify();
        });
        let settings_sub = cx.observe(&settings, |_, _, cx| cx.notify());
        let router_sub = cx.observe(&Router::global(cx), |this, router, cx| {
            let new_dm_active = matches!(
                router.read(cx).route(),
                Route::Direct | Route::DirectMessage { .. } | Route::Friends
            );
            if new_dm_active != this.dm_active {
                this.dm_active = new_dm_active;
                cx.notify();
            }
        });

        let initial_dm_active = matches!(
            Router::global(cx).read(cx).route(),
            Route::Direct | Route::DirectMessage { .. } | Route::Friends
        );
        let mut this = Self {
            clan_list,
            settings,
            rows: Rc::new(Vec::new()),
            list_state: ListState::new(0, gpui::ListAlignment::Top, px(48.)),
            active_clan_id: None,
            dm_active: initial_dm_active,
            _clan_sub: clan_sub,
            _settings_sub: settings_sub,
            _router_sub: router_sub,
        };
        this.sync_rows(this.clan_list.read(cx));
        this
    }

    fn sync_rows(&mut self, clan_list_view: &ClanList) {
        let rows: Vec<ClanRow> = clan_list_view
            .clans
            .iter()
            .map(|clan| {
                let id = SharedString::from(clan.id.to_string());
                let row_id = SharedString::from(format!("clan-{}", clan.id));
                let group_name = SharedString::from(format!("clan-group-{}", clan.id));
                ClanRow {
                    id,
                    row_id,
                    group_name,
                    name: SharedString::from(clan.name.clone()),
                    avatar_url: clan
                        .avatar_url
                        .as_deref()
                        .map(|s| SharedString::from(s.to_string())),
                    badge_count: clan.badge_count,
                    has_unread: clan.has_unread,
                    muted: clan.muted,
                }
            })
            .collect();
        let count = rows.len();
        let item_count = count + 1;
        let new_active = clan_list_view.active_clan_id;
        let needs_reset =
            self.list_state.item_count() != item_count || self.active_clan_id != new_active;
        self.rows = Rc::new(rows);
        self.active_clan_id = new_active;
        if needs_reset {
            self.list_state.reset(item_count);
        }
    }
}

impl Render for ClanSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let dm_active = self.dm_active;
        let pill_color = theme.tokens.text_theme_primary;
        let rows = self.rows.clone();
        let clan_list_handle = self.clan_list.clone();
        let list_state = self.list_state.clone();
        let locale = self.settings.read(cx).language.clone();
        let discover_title: SharedString =
            mezon_i18n::t(&locale, "common.discover").to_string().into();
        let create_clan_title: SharedString = mezon_i18n::t(&locale, "common.createClan")
            .to_string()
            .into();
        let clan_list_for_modal = self.clan_list.clone();
        let settings_for_modal = self.settings.clone();

        let clan_count = rows.len();
        let list_element = list(list_state, move |ix, _window, cx| {
            if ix < clan_count {
                render_clan_row(&rows, ix, cx, clan_list_handle.clone())
            } else {
                render_clan_footer(
                    cx,
                    &locale,
                    discover_title.clone(),
                    create_clan_title.clone(),
                    clan_list_for_modal.clone(),
                    settings_for_modal.clone(),
                )
            }
        })
        .size_full();

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .bg(theme.bg_tertiary)
            .items_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .bg(theme.bg_tertiary)
                    .pt_3()
                    .child(
                        div()
                            .id("dm-logo")
                            .group("dm-group")
                            .relative()
                            .w_full()
                            .h(px(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .on_click(|_, _, cx| crate::router::navigate(cx, Route::Direct))
                            .child(render_pill(dm_active, "dm-group".into(), pill_color))
                            .child(
                                img(SharedString::from(
                                    "https://cdn.mezon.ai/landing-page-mezon/logodefault.webp",
                                ))
                                .size(px(40.))
                                .rounded(px(8.))
                                .object_fit(gpui::ObjectFit::Cover),
                            ),
                    )
                    .child(div().w(px(40.)).h(px(1.)).bg(theme.border).mt_3()),
            )
            .child(div().flex_1().min_h_0().w_full().child(list_element))
    }
}

fn render_clan_footer(
    cx: &App,
    locale: &str,
    discover_title: SharedString,
    create_clan_title: SharedString,
    clan_list_for_modal: Entity<ClanList>,
    settings_for_modal: Entity<Settings>,
) -> AnyElement {
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .items_center()
        .w_full()
        .pb(px(68.))
        .child(
            div()
                .id("discover-btn")
                .w(px(40.))
                .h(px(40.))
                .rounded(px(12.))
                .bg(theme.tokens.theme_base_color)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| {
                    s.bg(theme.tokens.bg_button_add_friend)
                        .text_color(gpui::white())
                })
                .tooltip(Tooltip::text(discover_title))
                .on_click({
                    let locale = locale.to_string();
                    move |_, _, cx| {
                        Shell::global(cx).update(cx, |shell, cx| {
                            shell.info(mezon_i18n::t(&locale, "common.comingSoon").to_string(), cx);
                        });
                    }
                })
                .child(
                    Icon::new(IconName::CompassIcon)
                        .size(px(20.))
                        .text_color(theme.tokens.text_theme_primary),
                ),
        )
        .child(
            div()
                .id("create-clan-btn")
                .mt_3()
                .w(px(40.))
                .h(px(40.))
                .rounded(px(12.))
                .bg(theme.tokens.theme_base_color)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| {
                    s.bg(theme.tokens.bg_button_add_friend)
                        .text_color(gpui::white())
                })
                .tooltip(Tooltip::text(create_clan_title))
                .on_click(move |_, window, cx| {
                    use crate::clan::create_clan_modal::CreateClanModal;
                    let modal = cx.new(|cx| {
                        CreateClanModal::new(
                            clan_list_for_modal.clone(),
                            settings_for_modal.clone(),
                            window,
                            cx,
                        )
                    });
                    Shell::global(cx).update(cx, |shell, cx| shell.show_modal(modal.into(), cx));
                })
                .child(
                    div()
                        .text_size(px(24.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.tokens.text_theme_primary)
                        .child("+"),
                ),
        )
        .into_any_element()
}
