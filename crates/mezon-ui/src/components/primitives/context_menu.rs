use std::rc::Rc;

use gpui::{
    App, ClickEvent, MouseDownEvent, Pixels, Point, SharedString, Window, anchored, deferred, div,
    img, prelude::*, px,
};
use mezon_store::{MessageId, MessagesStore};

use super::icon::{Icon, IconName};
use super::stack::{h_flex, v_flex};
use crate::theme::ActiveTheme;

type MenuHandler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;
type DismissHandler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

#[derive(Clone)]
struct QuickReaction {
    emoji_id: String,
    shortname: String,
    message_id: MessageId,
}

enum Item {
    Entry {
        label: SharedString,
        leading_icon: Option<IconName>,
        trailing_icon: Option<IconName>,
        danger: bool,
        on_click: MenuHandler,
    },
    Separator,
}

#[derive(IntoElement, Default)]
pub struct ContextMenu {
    items: Vec<Item>,
    quick_reactions: Vec<QuickReaction>,
    on_dismiss: Option<DismissHandler>,
}

impl ContextMenu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn item(
        mut self,
        label: impl Into<SharedString>,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.items.push(Item::Entry {
            label: label.into(),
            leading_icon: None,
            trailing_icon: None,
            danger: false,
            on_click: Rc::new(on_click),
        });
        self
    }

    pub fn item_icon(
        mut self,
        label: impl Into<SharedString>,
        icon: IconName,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.items.push(Item::Entry {
            label: label.into(),
            leading_icon: None,
            trailing_icon: Some(icon),
            danger: false,
            on_click: Rc::new(on_click),
        });
        self
    }

    pub fn item_trailing_icon(
        self,
        label: impl Into<SharedString>,
        icon: IconName,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.item_icon(label, icon, on_click)
    }

    pub fn danger_item(
        mut self,
        label: impl Into<SharedString>,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.items.push(Item::Entry {
            label: label.into(),
            leading_icon: None,
            trailing_icon: None,
            danger: true,
            on_click: Rc::new(on_click),
        });
        self
    }

    pub fn danger_item_icon(
        mut self,
        label: impl Into<SharedString>,
        icon: IconName,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.items.push(Item::Entry {
            label: label.into(),
            leading_icon: None,
            trailing_icon: Some(icon),
            danger: true,
            on_click: Rc::new(on_click),
        });
        self
    }

    pub fn separator(mut self) -> Self {
        self.items.push(Item::Separator);
        self
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(on_dismiss));
        self
    }

    pub fn quick_reactions(
        mut self,
        message_id: MessageId,
        emojis: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.quick_reactions = emojis
            .into_iter()
            .filter(|(id, _)| !id.is_empty())
            .take(4)
            .map(|(emoji_id, shortname)| QuickReaction {
                emoji_id,
                shortname,
                message_id,
            })
            .collect();
        self
    }
}

impl RenderOnce for ContextMenu {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let bg = theme.tokens.bg_theme_contexify;
        let border = theme.border;
        let text = theme.text_primary;
        let muted = theme.text_secondary;
        let hover = theme.bg_hover;
        let danger = theme.status_dnd;
        let dismiss = self.on_dismiss.clone();

        let mut panel = v_flex()
            .min_w(px(220.))
            .p(px(6.))
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(bg)
            .shadow_lg()
            .occlude();

        if let Some(dismiss) = dismiss.clone() {
            panel = panel.on_mouse_down_out(move |_: &MouseDownEvent, window, cx| {
                dismiss(window, cx);
            });
        }

        if !self.quick_reactions.is_empty() {
            let mut reaction_row = h_flex()
                .gap_1()
                .px(px(6.))
                .pt(px(4.))
                .pb(px(6.));
            for (index, reaction) in self.quick_reactions.into_iter().enumerate() {
                let emoji_id = reaction.emoji_id.clone();
                let shortname = reaction.shortname.clone();
                let message_id = reaction.message_id;
                let src = crate::util::imgproxy::emoji_url(cx, &reaction.emoji_id);
                let dismiss_click = dismiss.clone();
                let mut cell = div()
                    .id(("context-menu-reaction", index))
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_1()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(hover))
                    .on_click(move |_: &ClickEvent, window, cx| {
                        MessagesStore::global(cx).update(cx, |store, cx| {
                            store.add_reaction(message_id, emoji_id.clone(), shortname.clone(), cx);
                        });
                        if let Some(dismiss) = &dismiss_click {
                            dismiss(window, cx);
                        }
                    });
                if !src.is_empty() {
                    let fallback_color = muted;
                    cell = cell.child(
                        img(SharedString::from(src))
                            .size(px(24.))
                            .with_fallback(move || {
                                div()
                                    .size(px(24.))
                                    .rounded(px(4.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::ImageThumbnail)
                                            .size(px(16.))
                                            .text_color(fallback_color),
                                    )
                                    .into_any_element()
                            }),
                    );
                }
                reaction_row = reaction_row.child(cell);
            }
            panel = panel.child(reaction_row).child(div().my(px(5.)).h(px(1.)).w_full().bg(border));
        }

        for (index, item) in self.items.into_iter().enumerate() {
            match item {
                Item::Separator => {
                    panel = panel.child(div().my(px(5.)).h(px(1.)).w_full().bg(border));
                }
                Item::Entry {
                    label,
                    leading_icon,
                    trailing_icon,
                    danger: is_danger,
                    on_click,
                } => {
                    let dismiss = dismiss.clone();
                    let label_color = if is_danger { danger } else { text };
                    let icon_color = if is_danger { danger } else { muted };
                    panel = panel.child(
                        h_flex()
                            .id(("context-menu-item", index))
                            .w_full()
                            .justify_between()
                            .items_center()
                            .px(px(10.))
                            .py(px(8.))
                            .rounded(px(4.))
                            .text_sm()
                            .text_color(label_color)
                            .cursor_pointer()
                            .hover(|s| s.bg(hover))
                            .when_some(leading_icon, |row, icon| {
                                row.gap_2().child(
                                    Icon::new(icon).size_4().text_color(icon_color),
                                )
                            })
                            .child(label)
                            .when_some(trailing_icon, |row, icon| {
                                row.child(Icon::new(icon).size_4().text_color(icon_color))
                            })
                            .on_click(move |_: &ClickEvent, window, cx| {
                                on_click(window, cx);
                                if let Some(dismiss) = &dismiss {
                                    dismiss(window, cx);
                                }
                            }),
                    );
                }
            }
        }

        panel
    }
}

pub fn context_menu_at(position: Point<Pixels>, menu: ContextMenu) -> impl IntoElement {
    deferred(anchored().position(position).snap_to_window().child(menu))
}
