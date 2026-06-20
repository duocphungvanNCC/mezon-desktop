use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Entity, ListState, MouseButton, MouseDownEvent, SharedString,
    Subscription, WeakEntity, Window, div, list, prelude::*, px,
};
use mezon_store::{ChannelList, ClanList, FAVOR_CATE_ID, Settings};

use crate::components::compositions::channel_row::ChannelRow;
use crate::components::primitives::{Avatar, Icon, IconName, Sizable, Size, context_menu_at};
use crate::theme::ActiveTheme;

mod items;
mod menu;
use items::{AppChannelSlot, SidebarItem, VoiceMemberSlot};
use menu::{OpenMenu, build_channel_menu, on_category_click, on_channel_click};

pub struct ChannelSidebar {
    clan_list: Entity<ClanList>,
    channel_list: Entity<ChannelList>,
    settings: Entity<Settings>,
    items: Rc<Vec<SidebarItem>>,
    list_state: ListState,
    active_clan_name: String,
    active_clan_id: Option<String>,
    channel_list_handle: Entity<ChannelList>,
    open_menu: Option<OpenMenu>,
    _clan_observe: Subscription,
    _channel_observe: Subscription,
    _settings_observe: Subscription,
}

impl ChannelSidebar {
    pub fn new(
        clan_list: Entity<ClanList>,
        channel_list: Entity<ChannelList>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let channel_list_handle = channel_list.clone();

        let clan_observe = cx.observe(&clan_list, |this, _, cx| {
            this.rebuild_items(cx);
            cx.notify();
        });
        let channel_observe = cx.observe(&channel_list, |this, _, cx| {
            this.rebuild_items(cx);
            cx.notify();
        });
        let settings_observe = cx.observe(&settings, |this, _, cx| {
            this.rebuild_items(cx);
            cx.notify();
        });

        let mut this = Self {
            clan_list,
            channel_list,
            settings,
            items: Rc::new(Vec::new()),
            list_state: ListState::new(0, gpui::ListAlignment::Top, px(32.)),
            active_clan_name: String::new(),
            active_clan_id: None,
            channel_list_handle,
            open_menu: None,
            _clan_observe: clan_observe,
            _channel_observe: channel_observe,
            _settings_observe: settings_observe,
        };
        this.rebuild_items(cx);
        this
    }

    fn rebuild_items(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.read(cx).language.clone();
        let clans = self.clan_list.read(cx);
        let channels = self.channel_list.read(cx);

        self.active_clan_name = clans
            .active_clan()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| mezon_i18n::t(&locale, "sidebar.selectClan").to_string());
        self.active_clan_id = clans.active_clan_id.clone();

        let active_channel_id = channels.active_channel_id.clone();
        let mut items = Vec::new();

        let app_channels: Vec<AppChannelSlot> = clans
            .active_clan_id
            .as_deref()
            .map(|cid| {
                channels
                    .app_channels_for_clan(cid)
                    .iter()
                    .map(AppChannelSlot::from)
                    .collect()
            })
            .unwrap_or_default();

        items.push(SidebarItem::BannerAndEvents {
            banner_url: clans.active_clan_banner().map(|s| s.to_string()),
            app_channels,
        });

        if let Some(clan_id) = clans.active_clan_id.as_ref() {
            let categories = channels.categories_for_clan(clan_id);
            if categories.is_empty() {
                items.push(SidebarItem::Skeleton);
            } else {
                for category in categories {
                    let is_favorites = category.id == FAVOR_CATE_ID;
                    let collapsed = channels.is_category_collapsed(clan_id, &category.id);
                    let name = if is_favorites {
                        mezon_i18n::t(&locale, "channelList.favoriteChannel").to_string()
                    } else {
                        category.name.clone()
                    };
                    items.push(SidebarItem::Category {
                        id: category.id.clone(),
                        name,
                        collapsed,
                    });
                    if !collapsed {
                        let ch_slice = &category.channels;
                        for (idx, ch) in ch_slice.iter().enumerate() {
                            let is_thread = !is_favorites && ch.parent_id.is_some();
                            let (line_above, line_below) = if is_thread {
                                let pid = ch.parent_id.as_deref().unwrap_or("");
                                let has_prev = ch_slice[..idx]
                                    .iter()
                                    .any(|c| c.parent_id.as_deref() == Some(pid));
                                let has_next = ch_slice[idx + 1..]
                                    .iter()
                                    .any(|c| c.parent_id.as_deref() == Some(pid));
                                (has_prev, has_next)
                            } else {
                                (false, false)
                            };
                            items.push(SidebarItem::Channel {
                                id: ch.id.clone(),
                                name: truncate_channel_label(&ch.name),
                                channel_type: ch.channel_type,
                                unread: ch.is_unread(),
                                private: ch.private,
                                selected: active_channel_id.as_deref() == Some(ch.id.as_str()),
                                badge_count: ch.badge_count,
                                muted: ch.muted,
                                is_thread,
                                line_above,
                                line_below,
                                voice_members: ch
                                    .voice_members
                                    .iter()
                                    .map(VoiceMemberSlot::from)
                                    .collect(),
                            });
                        }
                    }
                }
            }
        } else {
            items.push(SidebarItem::Skeleton);
        }

        let count = items.len();
        let count_changed = self.items.len() != count;
        self.items = Rc::new(items);
        if count_changed {
            self.list_state.reset(count);
        }
    }
}

impl Render for ChannelSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::trace_render!("ChannelSidebar");
        let theme = cx.theme();
        let items = self.items.clone();
        let channel_list_handle = self.channel_list_handle.clone();
        let active_clan_id_for_nav = self.active_clan_id.clone();
        let list_state = self.list_state.clone();
        let sidebar = cx.entity().downgrade();
        let menu_overlay = self.open_menu.as_ref().map(|menu| {
            (
                menu.position,
                menu.channel_type,
                menu.is_thread,
                self.settings.read(cx).language.clone(),
            )
        });

        let list_element = list(list_state, {
            let sidebar = sidebar.clone();
            move |ix, _window, cx| {
                render_sidebar_item(
                    &items,
                    ix,
                    cx,
                    channel_list_handle.clone(),
                    active_clan_id_for_nav.clone(),
                    sidebar.clone(),
                )
            }
        })
        .size_full();

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .pb(px(68.))
            .bg(theme.bg_secondary)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .h(px(50.))
                    .px_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child(self.active_clan_name.clone()),
                    ),
            )
            .child(div().flex_1().min_h_0().child(list_element))
            .when_some(
                menu_overlay,
                move |el, (position, channel_type, is_thread, locale)| {
                    el.child(context_menu_at(
                        position,
                        build_channel_menu(sidebar, &locale, channel_type, is_thread),
                    ))
                },
            )
    }
}

fn render_skeleton(cx: &App) -> AnyElement {
    let theme = cx.theme();
    let skeleton_bg = theme.bg_tertiary;
    let skeleton_row = || {
        div()
            .flex()
            .flex_row()
            .items_center()
            .px_4()
            .py_1()
            .gap_2()
            .child(div().size(px(14.)).rounded_sm().bg(skeleton_bg))
            .child(div().h(px(12.)).w(px(80.)).rounded_sm().bg(skeleton_bg))
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px_2()
        .py_2()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .px_2()
                .py_1()
                .gap_2()
                .child(div().size(px(12.)).rounded_sm().bg(skeleton_bg))
                .child(div().h(px(10.)).w(px(60.)).rounded_sm().bg(skeleton_bg)),
        )
        .child(skeleton_row())
        .child(skeleton_row())
        .child(skeleton_row())
        .child(skeleton_row())
        .into_any_element()
}

fn nav_row(icon: IconName, label: &'static str, theme: &crate::theme::Theme) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .px_2()
        .h(px(34.))
        .gap_2()
        .rounded(px(4.))
        .cursor_pointer()
        .text_color(theme.text_secondary)
        .child(
            Icon::new(icon)
                .size(px(20.))
                .text_color(theme.text_secondary),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(label),
        )
}

const NUMBER_APPS_SHOW_OFF: usize = 4;
const MAX_CHANNEL_LABEL_CHARS: usize = 20;

fn truncate_channel_label(name: &str) -> String {
    if name.chars().count() > MAX_CHANNEL_LABEL_CHARS {
        let head: String = name.chars().take(MAX_CHANNEL_LABEL_CHARS).collect();
        format!("{head}...")
    } else {
        name.to_string()
    }
}

fn render_banner_and_events(
    banner_url: Option<&str>,
    app_channels: &[AppChannelSlot],
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let divider_color = theme.border;
    let hover_bg = theme.bg_hover;

    let mut col = div().flex().flex_col().w_full();

    if let Some(url) = banner_url {
        col = col.child(
            div().w_full().h(px(136.)).mb_2().child(
                gpui::img(crate::util::imgproxy::proxied(cx, url, 300, 300, "fit"))
                    .w_full()
                    .h_full()
                    .object_fit(gpui::ObjectFit::Cover),
            ),
        );
    }

    let nav_col = div()
        .flex()
        .flex_col()
        .w_full()
        .p_2()
        .gap_1()
        .child(nav_row(IconName::IconEvents, "Events", theme))
        .child(nav_row(IconName::MemberList, "Members", theme));

    col = col.child(nav_col);

    let hr = || div().w_full().h(px(1.)).ml(px(3.)).bg(divider_color);

    if !app_channels.is_empty() {
        let show_list: Vec<&AppChannelSlot> =
            app_channels.iter().take(NUMBER_APPS_SHOW_OFF).collect();
        let has_more = app_channels.len() > NUMBER_APPS_SHOW_OFF + 1;

        let app_row = if show_list.len() < NUMBER_APPS_SHOW_OFF {
            div().flex().flex_row().items_center().gap_2().py_1().px_2()
        } else {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .py_1()
                .px_2()
                .justify_center()
        };

        let mut app_row = app_row;
        for slot in &show_list {
            let icon_el: AnyElement = if let Some(logo) = &slot.app_logo {
                gpui::img(logo.clone())
                    .w(px(24.))
                    .h(px(24.))
                    .into_any_element()
            } else {
                gpui::svg()
                    .path("icons/channel-app-fallback.svg")
                    .w(px(24.))
                    .h(px(24.))
                    .text_color(theme.text_primary)
                    .into_any_element()
            };
            app_row = app_row.child(
                div()
                    .w(px(40.))
                    .h(px(40.))
                    .p_2()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(hover_bg))
                    .child(icon_el),
            );
        }
        if has_more {
            app_row = app_row.child(
                div()
                    .w(px(40.))
                    .h(px(40.))
                    .p_2()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(hover_bg))
                    .child(
                        Icon::new(IconName::RightIcon)
                            .size(px(24.))
                            .text_color(theme.text_primary),
                    ),
            );
        }

        col = col.child(hr()).child(app_row).child(hr());
    } else {
        col = col.child(hr());
    }

    col.into_any_element()
}

fn render_sidebar_item(
    items: &[SidebarItem],
    ix: usize,
    cx: &App,
    channel_list_handle: Entity<ChannelList>,
    active_clan_id_for_nav: Option<String>,
    sidebar: WeakEntity<ChannelSidebar>,
) -> AnyElement {
    let theme = cx.theme();
    let Some(item) = items.get(ix) else {
        return div().into_any_element();
    };

    match item {
        SidebarItem::Skeleton => render_skeleton(cx),

        SidebarItem::BannerAndEvents {
            banner_url,
            app_channels,
        } => render_banner_and_events(banner_url.as_deref(), app_channels, cx),

        SidebarItem::Category {
            id,
            name,
            collapsed,
        } => {
            let category_id = id.clone();
            let category_name = name.clone().to_uppercase();
            let clan_id_for_toggle = active_clan_id_for_nav.clone().unwrap_or_default();

            let mut header = div()
                .id(SharedString::from(format!("cat-{}", category_id)))
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .px_2()
                .py_1()
                .cursor_pointer()
                .text_color(theme.text_muted)
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM);

            let icon = if *collapsed {
                IconName::CaretRight
            } else {
                IconName::CaretDown
            };
            header = header
                .child(Icon::new(icon).size(px(18.0)).text_color(theme.text_muted))
                .child(div().ml_1().child(category_name));

            header.interactivity().on_click(on_category_click(
                channel_list_handle.clone(),
                clan_id_for_toggle,
                category_id,
            ));

            div()
                .pt(px(10.))
                .pb(px(6.))
                .flex()
                .flex_col()
                .child(header)
                .into_any_element()
        }

        SidebarItem::Channel {
            id,
            name,
            channel_type,
            unread,
            private,
            selected,
            badge_count,
            muted,
            is_thread,
            line_above,
            line_below,
            voice_members,
        } => {
            let ch_id = id.clone();
            let row_handle = channel_list_handle.clone();
            let clan_id_inner = active_clan_id_for_nav.clone();
            let selected_bg = theme.bg_primary;
            let brand = theme.brand;
            let text_primary = theme.text_primary;
            let text_color = if *muted {
                theme.text_muted
            } else if *selected {
                theme.text_primary
            } else {
                theme.text_secondary
            };

            let row_content = if *is_thread {
                let line_color = theme.text_muted;
                let line_above_val = *line_above;
                let line_below_val = *line_below;
                const ELBOW_TOP: gpui::Pixels = px(12.);
                const ELBOW_SIZE: gpui::Pixels = px(10.);

                let mut connector = div().relative().flex_none().w(px(20.)).h_full();
                if line_above_val || line_below_val {
                    connector = connector.child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .top(px(0.))
                            .w(px(1.))
                            .when(line_below_val, |el| el.bottom(px(0.)))
                            .when(!line_below_val, |el| el.h(ELBOW_TOP))
                            .bg(line_color),
                    );
                }
                let connector = connector.child(
                    div()
                        .absolute()
                        .left(px(8.))
                        .top(ELBOW_TOP)
                        .w(ELBOW_SIZE)
                        .h(ELBOW_SIZE)
                        .border_l_1()
                        .border_b_1()
                        .border_color(line_color)
                        .rounded_bl_sm(),
                );

                div()
                    .h(px(34.))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_stretch()
                    .when(*selected, move |el| el.bg(selected_bg))
                    .child(div().flex_none().w(px(16.)))
                    .child(connector)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .pr_2()
                            .text_sm()
                            .text_color(text_color)
                            .when(*unread && !*muted, |el| {
                                el.font_weight(gpui::FontWeight::BOLD)
                            })
                            .child(div().flex_1().child(name.clone()))
                            .when(*badge_count > 0 && !*muted, move |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .min_w(px(16.))
                                        .h(px(16.))
                                        .px_1()
                                        .rounded_full()
                                        .bg(brand)
                                        .text_color(text_primary)
                                        .text_xs()
                                        .child(format!("{}", badge_count)),
                                )
                            }),
                    )
                    .into_any_element()
            } else {
                ChannelRow::new(name.clone(), *channel_type)
                    .selected(*selected)
                    .unread(*unread)
                    .private(*private)
                    .badge_count(*badge_count)
                    .muted(*muted)
                    .render(theme)
                    .into_any_element()
            };

            let mut channel_col = div()
                .id(SharedString::from(format!("ch-{}", id)))
                .w_full()
                .flex()
                .flex_col()
                .child(row_content);

            if !voice_members.is_empty() {
                let voice_pl = if *is_thread { px(40.) } else { px(32.) };
                let members_el =
                    div()
                        .flex()
                        .flex_col()
                        .pl(voice_pl)
                        .children(voice_members.iter().map(|m| {
                            let name_text = if m.display_name.is_empty() {
                                m.user_id.clone()
                            } else {
                                m.display_name.clone()
                            };
                            let avatar = if m.avatar_url.is_empty() {
                                Avatar::new().name(name_text.clone())
                            } else {
                                Avatar::new()
                                    .src(m.avatar_url.clone())
                                    .name(name_text.clone())
                            };
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .px_2()
                                .py(px(2.))
                                .gap_1()
                                .child(avatar.with_size(Size::XSmall))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child(name_text),
                                )
                        }));
                channel_col = channel_col.child(members_el);
            }

            let mut channel_col = channel_col.on_mouse_down(MouseButton::Right, {
                let channel_type = *channel_type;
                let is_thread = *is_thread;
                move |event: &MouseDownEvent, _window, cx| {
                    let position = event.position;
                    if let Some(view) = sidebar.upgrade() {
                        view.update(cx, |this, cx| {
                            this.open_menu = Some(OpenMenu {
                                channel_type,
                                is_thread,
                                position,
                            });
                            cx.notify();
                        });
                    }
                }
            });

            channel_col.interactivity().on_click(on_channel_click(
                row_handle,
                ch_id,
                clan_id_inner,
            ));

            channel_col.into_any_element()
        }
    }
}
