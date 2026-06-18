use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, ListState, SharedString, Window, div, list,
    prelude::*, px,
};
use mezon_store::{ClanList, Settings};

use crate::components::primitives::{Avatar, Badge, Icon, IconName, Sizable, Size};
use crate::theme::ActiveTheme;

#[derive(Clone)]
struct ClanRow {
    id: String,
    name: String,
    avatar_url: Option<String>,
    unread: u32,
    is_active: bool,
}

fn on_clan_click(
    clan_list: Entity<ClanList>,
    clan_id: String,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        clan_list.update(cx, |m, cx| {
            m.select_clan(&clan_id, cx);
        });
    }
}

pub struct ClanSidebar {
    clan_list: Entity<ClanList>,
    rows: Rc<Vec<ClanRow>>,
    list_state: ListState,
}

impl ClanSidebar {
    pub fn new(
        clan_list: Entity<ClanList>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&clan_list, |this, clan_list, cx| {
            this.sync_rows(clan_list.read(cx));
            cx.notify();
        })
        .detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let mut this = Self {
            clan_list,
            rows: Rc::new(Vec::new()),
            list_state: ListState::new(0, gpui::ListAlignment::Top, px(48.)),
        };
        this.sync_rows(this.clan_list.read(cx));
        this
    }

    fn sync_rows(&mut self, clan_list_view: &ClanList) {
        let rows: Vec<ClanRow> = clan_list_view
            .clans
            .iter()
            .map(|clan| ClanRow {
                id: clan.id.clone(),
                name: clan.name.clone(),
                avatar_url: clan.avatar_url.clone(),
                unread: clan.unread_count,
                is_active: clan_list_view.is_active_clan(&clan.id),
            })
            .collect();
        let count = rows.len();
        let count_changed = self.rows.len() != count;
        self.rows = Rc::new(rows);
        if count_changed {
            self.list_state.reset(count);
        }
    }
}

impl Render for ClanSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.rows.clone();
        let clan_list_handle = self.clan_list.clone();
        let list_state = self.list_state.clone();

        let list_element = list(list_state, move |ix, _window, cx| {
            render_clan_row(&rows, ix, cx, clan_list_handle.clone())
        })
        .size_full();

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .bg(theme.bg_tertiary)
            .gap_1()
            .p_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Avatar::new().name("U").with_size(Size::Small)),
            )
            .child(div().w_full().h_1().bg(theme.border).my_1())
            .child(div().flex_1().min_h_0().child(list_element))
            .child(Icon::new(IconName::Plus))
    }
}

fn render_clan_row(
    rows: &[ClanRow],
    ix: usize,
    cx: &App,
    clan_list_handle: Entity<ClanList>,
) -> AnyElement {
    let theme = cx.theme();
    let Some(clan) = rows.get(ix) else {
        return div().into_any_element();
    };

    let clan_id = clan.id.clone();
    let clan_list_handle = clan_list_handle.clone();

    let mut clan_div = div()
        .id(SharedString::from(format!("clan-{}", clan.id)))
        .relative()
        .w_full()
        .cursor_pointer()
        .when(clan.is_active, |el| {
            el.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(4.))
                    .bg(theme.brand),
            )
        });

    clan_div
        .interactivity()
        .on_click(on_clan_click(clan_list_handle, clan_id));

    let mut clan_avatar = Avatar::new().name(clan.name.clone()).with_size(Size::Small);
    if let Some(url) = &clan.avatar_url {
        clan_avatar = clan_avatar.src(crate::imgproxy::avatar_url(cx, url));
    }

    clan_div
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .relative()
                .child(clan_avatar)
                .when(clan.unread > 0, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .child(Badge::new().count(clan.unread as usize)),
                    )
                }),
        )
        .into_any_element()
}
