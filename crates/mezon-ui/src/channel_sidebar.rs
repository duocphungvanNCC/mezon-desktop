use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, ListState, SharedString, Window, div, list,
    prelude::*, px,
};
use mezon_store::{ChannelList, ChannelType, ClanList, Settings};

use crate::components::compositions::channel_row::ChannelRow;
use crate::components::primitives::{Icon, IconName};
use crate::theme::ActiveTheme;

#[derive(Clone)]
enum SidebarItem {
    Category {
        name: String,
        collapsed: bool,
    },
    Channel {
        id: String,
        name: String,
        channel_type: ChannelType,
        unread: bool,
        private: bool,
        selected: bool,
    },
}

fn on_channel_click(
    channel_list: Entity<ChannelList>,
    channel_id: String,
    clan_id: Option<String>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        channel_list.update(cx, |m, cx| {
            m.select_channel(&channel_id, cx);
        });
        if let Some(ref cid) = clan_id {
            crate::router::navigate(
                cx,
                crate::router::Route::Channel {
                    clan_id: cid.clone(),
                    channel_id: channel_id.clone(),
                },
            );
        }
    }
}

fn on_category_click(
    sidebar: Entity<ChannelSidebar>,
    category_name: String,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        sidebar.update(cx, |this, cx| {
            if this.collapsed.contains(&category_name) {
                this.collapsed.remove(&category_name);
            } else {
                this.collapsed.insert(category_name.clone());
            }
            this.rebuild_items(cx);
            cx.notify();
        });
    }
}

pub struct ChannelSidebar {
    clan_list: Entity<ClanList>,
    channel_list: Entity<ChannelList>,
    collapsed: std::collections::HashSet<String>,
    items: Rc<Vec<SidebarItem>>,
    list_state: ListState,
    active_clan_name: String,
    active_clan_id: Option<String>,
    channel_list_handle: Entity<ChannelList>,
    sidebar_entity: Entity<ChannelSidebar>,
}

impl ChannelSidebar {
    pub fn new(
        clan_list: Entity<ClanList>,
        channel_list: Entity<ChannelList>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let sidebar_entity = cx.entity().clone();
        let channel_list_handle = channel_list.clone();

        cx.observe(&clan_list, |this, _, cx| {
            this.rebuild_items(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&channel_list, |this, _, cx| {
            this.rebuild_items(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let mut this = Self {
            clan_list,
            channel_list,
            collapsed: std::collections::HashSet::new(),
            items: Rc::new(Vec::new()),
            list_state: ListState::new(0, gpui::ListAlignment::Top, px(32.)),
            active_clan_name: String::new(),
            active_clan_id: None,
            channel_list_handle,
            sidebar_entity,
        };
        this.rebuild_items(cx);
        this
    }

    fn rebuild_items(&mut self, cx: &mut Context<Self>) {
        let clans = self.clan_list.read(cx);
        let channels = self.channel_list.read(cx);

        self.active_clan_name = clans
            .active_clan()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Select a clan".to_string());
        self.active_clan_id = clans.active_clan_id.clone();

        let active_channel_id = channels.active_channel_id.clone();
        let mut items = Vec::new();

        if let Some(clan_id) = clans.active_clan_id.as_ref() {
            for category in channels.categories_for_clan(clan_id) {
                let collapsed = self.collapsed.contains(&category.name);
                items.push(SidebarItem::Category {
                    name: category.name.clone(),
                    collapsed,
                });
                if !collapsed {
                    for ch in &category.channels {
                        items.push(SidebarItem::Channel {
                            id: ch.id.clone(),
                            name: ch.name.clone(),
                            channel_type: ch.channel_type,
                            unread: ch.unread,
                            private: ch.private,
                            selected: active_channel_id.as_deref() == Some(ch.id.as_str()),
                        });
                    }
                }
            }
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
        let sidebar_entity = self.sidebar_entity.clone();
        let list_state = self.list_state.clone();

        let list_element = list(list_state, move |ix, _window, cx| {
            render_sidebar_item(
                &items,
                ix,
                cx,
                channel_list_handle.clone(),
                active_clan_id_for_nav.clone(),
                sidebar_entity.clone(),
            )
        })
        .size_full();

        div()
            .flex()
            .flex_col()
            .w_full()
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
                            .child(self.active_clan_name.clone()),
                    ),
            )
            .child(div().flex_1().min_h_0().child(list_element))
    }
}

fn render_sidebar_item(
    items: &[SidebarItem],
    ix: usize,
    cx: &App,
    channel_list_handle: Entity<ChannelList>,
    active_clan_id_for_nav: Option<String>,
    sidebar_entity: Entity<ChannelSidebar>,
) -> AnyElement {
    let theme = cx.theme();
    let Some(item) = items.get(ix) else {
        return div().into_any_element();
    };

    match item {
        SidebarItem::Category { name, collapsed } => {
            let category_name = name.clone();

            let mut header = div()
                .id(SharedString::from(format!("cat-{}", category_name)))
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .px_3()
                .py_1()
                .cursor_pointer()
                .text_color(theme.text_muted)
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(
                    Icon::new(if *collapsed {
                        IconName::ArrowRight
                    } else {
                        IconName::ArrowDown
                    })
                    .size(px(12.0))
                    .text_color(theme.text_muted),
                )
                .child(div().ml_1().child(category_name.clone()));

            header
                .interactivity()
                .on_click(on_category_click(sidebar_entity.clone(), category_name));

            div().flex().flex_col().child(header).into_any_element()
        }
        SidebarItem::Channel {
            id,
            name,
            channel_type,
            unread,
            private,
            selected,
        } => {
            let ch_id = id.clone();
            let row_handle = channel_list_handle.clone();
            let clan_id_inner = active_clan_id_for_nav.clone();

            let mut row = div().id(SharedString::from(format!("ch-{}", id))).child(
                ChannelRow::new(name.clone(), *channel_type)
                    .selected(*selected)
                    .unread(*unread)
                    .private(*private)
                    .render(theme),
            );

            row.interactivity()
                .on_click(on_channel_click(row_handle, ch_id, clan_id_inner));

            row.into_any_element()
        }
    }
}
