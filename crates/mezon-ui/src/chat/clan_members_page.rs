use std::collections::HashSet;

use gpui::{Context, Entity, FontWeight, Render, Subscription, Window, div, prelude::*, px};
use mezon_store::{
    ClanId, ClanMembersStore, PERMISSION_MANAGE_CLAN, PermissionStore, RoleId, RolesStore,
    Settings, UserId,
};
use ui::Tooltip;

use crate::components::primitives::{Avatar, Input, InputEvent, InputState};
use crate::theme::ActiveTheme;

const PAGE_SIZE: usize = 10;

pub struct ClanMembersPage {
    clan_id: ClanId,
    _settings: Entity<Settings>,
    search: Option<Entity<InputState>>,
    search_sub: Option<Subscription>,
    page: usize,
    newest_first: bool,
    role_picker: Option<UserId>,
}

#[derive(Clone)]
struct MemberRow {
    id: UserId,
    name: String,
    username: String,
    clan_nick: String,
    avatar: String,
    joined_mezon: u32,
    role_ids: Vec<RoleId>,
}

impl ClanMembersPage {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        cx.observe(&ClanMembersStore::global(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe(&RolesStore::global(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe(&PermissionStore::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self {
            clan_id: ClanId(0),
            _settings: settings,
            search: None,
            search_sub: None,
            page: 0,
            newest_first: true,
            role_picker: None,
        }
    }

    pub fn set_clan(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        if self.clan_id == clan_id {
            return;
        }
        self.clan_id = clan_id;
        self.page = 0;
        self.role_picker = None;
        ClanMembersStore::global(cx).update(cx, |store, cx| store.ensure_loaded(clan_id, cx));
        RolesStore::global(cx).update(cx, |store, cx| store.ensure_loaded(clan_id, cx));
        PermissionStore::global(cx)
            .update(cx, |store, cx| store.load_clan_permissions(clan_id, cx));
        cx.notify();
    }

    fn ensure_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search by clan nick, display name or username")
                .embedded(true)
        });
        self.search_sub = Some(cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.page = 0;
                cx.notify();
            }
        }));
        self.search = Some(input);
    }

    fn rows(&self, cx: &Context<Self>) -> Vec<MemberRow> {
        let query = self
            .search
            .as_ref()
            .map(|s| s.read(cx).value().trim().to_lowercase())
            .unwrap_or_default();
        let mut rows: Vec<_> = ClanMembersStore::global(cx)
            .read(cx)
            .members(self.clan_id)
            .into_iter()
            .filter(|m| {
                query.is_empty()
                    || m.user.username.to_lowercase().contains(&query)
                    || m.user.display_name.to_lowercase().contains(&query)
                    || m.clan_nick.to_lowercase().contains(&query)
            })
            .map(|m| MemberRow {
                id: m.id(),
                name: m.name().to_string(),
                username: m.user.username.clone(),
                clan_nick: m.clan_nick.clone(),
                avatar: m.avatar().to_string(),
                joined_mezon: m.user.create_time_seconds,
                role_ids: m.role_ids.clone(),
            })
            .collect();
        rows.sort_by_key(|m| m.joined_mezon);
        if self.newest_first {
            rows.reverse();
        }
        rows
    }

    fn format_date(seconds: u32) -> String {
        chrono::DateTime::from_timestamp(i64::from(seconds), 0)
            .map(|dt| dt.format("%b %d, %Y").to_string())
            .unwrap_or_else(|| "—".into())
    }

    fn role_cell(&self, row: &MemberRow, can_manage: bool, cx: &Context<Self>) -> gpui::AnyElement {
        let roles_store = RolesStore::global(cx);
        let roles = roles_store
            .read(cx)
            .roles_for(self.clan_id, &row.role_ids)
            .into_iter()
            .map(|r| r.name.clone())
            .collect::<Vec<_>>();
        let first = roles.first().cloned().unwrap_or_else(|| "—".into());
        let extra = roles.len().saturating_sub(1);
        let user_id = row.id;
        let clan_id = self.clan_id;
        let picker_open = self.role_picker == Some(user_id);
        let assigned: HashSet<RoleId> = row.role_ids.iter().copied().collect();
        let all_roles = roles_store.read(cx).all_roles(clan_id);
        let mut cell = div()
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(cx.theme().bg_hover)
                    .max_w(px(130.))
                    .overflow_hidden()
                    .child(first),
            );
        if extra > 0 {
            let tooltip = roles.iter().skip(1).cloned().collect::<Vec<_>>().join(", ");
            cell = cell.child(
                div()
                    .id(format!("extra-roles-{}", user_id.get()))
                    .cursor_default()
                    .child(format!("+{extra}"))
                    .tooltip(Tooltip::text(tooltip)),
            );
        }
        if can_manage {
            cell = cell.child(
                div()
                    .id(format!("assign-role-{}", user_id.get()))
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(cx.theme().bg_hover)
                    .child("+")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.role_picker = if picker_open { None } else { Some(user_id) };
                        cx.notify();
                    })),
            );
        }
        if picker_open {
            let mut menu = div()
                .absolute()
                .top(px(30.))
                .right_0()
                .w(px(230.))
                .p_2()
                .rounded(px(6.))
                .bg(cx.theme().bg_floating)
                .border_1()
                .border_color(cx.theme().border)
                .shadow_lg();
            for (role_id, role) in all_roles {
                let checked = assigned.contains(&role_id);
                menu = menu.child(
                    div()
                        .id(format!("role-option-{}", role_id.get()))
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .h(px(34.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(cx.theme().bg_hover))
                        .child(role.name)
                        .child(if checked { "✓" } else { "" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            ClanMembersStore::global(cx).update(cx, |store, cx| {
                                store.set_member_role(clan_id, user_id, role_id, !checked, cx)
                            });
                            this.role_picker = None;
                            cx.notify();
                        })),
                );
            }
            cell = cell.child(menu);
        }
        cell.into_any_element()
    }
}

impl Render for ClanMembersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_search(window, cx);
        let theme = cx.theme();
        let rows = self.rows(cx);
        let total = rows.len();
        let pages = total.div_ceil(PAGE_SIZE).max(1);
        self.page = self.page.min(pages - 1);
        let visible = rows
            .iter()
            .skip(self.page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .cloned()
            .collect::<Vec<_>>();
        let can_manage = PermissionStore::global(cx).read(cx).check_permission(
            self.clan_id,
            PERMISSION_MANAGE_CLAN,
            cx,
        );

        let header_cell = |label: &'static str, width: f32| {
            div()
                .w(px(width))
                .text_size(px(12.))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.text_secondary)
                .child(label)
        };
        let mut table = div().flex().flex_col().w_full();
        table = table.child(
            div()
                .flex()
                .items_center()
                .h(px(46.))
                .border_b_1()
                .border_color(theme.border)
                .child(header_cell("NAME", 420.))
                .child(header_cell("MEMBER SINCE", 180.))
                .child(header_cell("JOINED MEZON", 180.))
                .child(header_cell("ROLES", 260.))
                .child(header_cell("SIGNALS", 120.)),
        );
        for row in visible {
            let name = row.name.clone();
            let username = row.username.clone();
            let avatar = row.avatar.clone();
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .h(px(60.))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .w(px(420.))
                            .child(
                                Avatar::new()
                                    .src(avatar)
                                    .name(name.clone())
                                    .size_px(px(40.)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(div().text_color(theme.brand).child(name))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme.text_secondary)
                                            .child(if row.clan_nick.is_empty() {
                                                username
                                            } else {
                                                format!("{} · {}", row.clan_nick, username)
                                            }),
                                    ),
                            ),
                    )
                    .child(div().w(px(180.)).child("—"))
                    .child(div().w(px(180.)).child(Self::format_date(row.joined_mezon)))
                    .child(
                        div()
                            .w(px(260.))
                            .child(self.role_cell(&row, can_manage, cx)),
                    )
                    .child(
                        div()
                            .w(px(120.))
                            .text_size(px(12.))
                            .text_color(theme.text_secondary)
                            .child("SIGNALS"),
                    ),
            );
        }
        let current = self.page;
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg_primary)
            .child(
                div()
                    .h(px(58.))
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Members"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px_5()
                    .pt_5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pb_3()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Recent Members"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(520.))
                                            .child(Input::new(self.search.as_ref().unwrap())),
                                    )
                                    .child(
                                        div()
                                            .id("sort-members")
                                            .cursor_pointer()
                                            .px_3()
                                            .h(px(40.))
                                            .flex()
                                            .items_center()
                                            .rounded(px(5.))
                                            .bg(theme.brand)
                                            .text_color(theme.bg_primary)
                                            .child(if self.newest_first {
                                                "⇅ Sort newest"
                                            } else {
                                                "⇅ Sort oldest"
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.newest_first = !this.newest_first;
                                                this.page = 0;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("members-table-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(table),
                    )
                    .child(
                        div()
                            .h(px(64.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(format!(
                                "Showing {}–{} of {} members",
                                if total == 0 {
                                    0
                                } else {
                                    current * PAGE_SIZE + 1
                                },
                                ((current + 1) * PAGE_SIZE).min(total),
                                total
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(page_button("‹", current == 0).on_click(cx.listener(
                                        |this, _, _, cx| {
                                            if this.page > 0 {
                                                this.page -= 1;
                                                cx.notify();
                                            }
                                        },
                                    )))
                                    .child(format!("{} / {}", current + 1, pages))
                                    .child(page_button("›", current + 1 >= pages).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            if this.page + 1 < pages {
                                                this.page += 1;
                                                cx.notify();
                                            }
                                        }),
                                    )),
                            ),
                    ),
            )
    }
}

fn page_button(label: &'static str, disabled: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(format!("members-page-{label}"))
        .w(px(42.))
        .h(px(36.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(gpui::rgba(if disabled { 0x45455a88 } else { 0x5865f2ff }))
        .text_color(gpui::white())
        .when(!disabled, |e| e.cursor_pointer())
        .child(label)
}
