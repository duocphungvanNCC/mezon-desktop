use gpui::{
    Context, CursorStyle, DragMoveEvent, Entity, FontWeight, Pixels, Render, Subscription, Window,
    div, prelude::*, px,
};
use mezon_store::{ChannelList, ClanId, FAVOR_CATE_ID};

use crate::components::primitives::{h_flex, v_flex};
use crate::theme::{ActiveTheme, Theme};

const ROW_HEIGHT: f32 = 52.0;
const ROW_GAP: f32 = 8.0;
const ROW_STEP: f32 = ROW_HEIGHT + ROW_GAP;
const GRIP_DOT_SIZE: f32 = 3.0;
const GRIP_DOT_GAP: f32 = 3.0;

#[derive(Clone, PartialEq)]
struct CategorySortItem {
    id: String,
    name: String,
    category_id: i64,
}

#[derive(Clone, Copy)]
struct CategoryDrag(usize);

#[derive(Clone)]
struct CategoryDragPreview;

impl Render for CategoryDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

enum RowSurfaceStyle {
    Normal,
    DraggingSource,
    DragPreview,
}

fn render_category_row_surface(
    name: &str,
    theme: &Theme,
    style: RowSurfaceStyle,
    width: Option<Pixels>,
) -> gpui::Div {
    let item_bg = theme.tokens.bg_item_theme_hover;
    let highlight_bg = theme.bg_hover;
    let label = name.to_uppercase();

    let row = h_flex()
        .h(px(ROW_HEIGHT))
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .flex_shrink_0()
        .rounded_lg()
        .when(matches!(style, RowSurfaceStyle::Normal), |el| {
            el.bg(item_bg).cursor(CursorStyle::OpenHand)
        })
        .when(matches!(style, RowSurfaceStyle::DraggingSource), |el| {
            el.opacity(0.0)
        })
        .when(matches!(style, RowSurfaceStyle::DragPreview), |el| {
            el.bg(highlight_bg)
                .shadow_lg()
                .border_1()
                .border_color(theme.status_online)
                .cursor(CursorStyle::ClosedHand)
        })
        .child(drag_grip_icon(theme.text_muted))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .text_ellipsis()
                .text_base()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_primary)
                .child(label),
        );

    match width {
        Some(width) => row.w(width),
        None => row.w_full(),
    }
}

fn drag_grip_icon(color: impl Into<gpui::Background>) -> impl IntoElement {
    fn dot(color: gpui::Background) -> gpui::Div {
        div()
            .size(px(GRIP_DOT_SIZE))
            .rounded_full()
            .bg(color)
    }
    fn row(color: gpui::Background) -> gpui::Div {
        h_flex()
            .gap(px(GRIP_DOT_GAP))
            .child(dot(color.clone()))
            .child(dot(color))
    }
    let color = color.into();
    v_flex()
        .gap(px(GRIP_DOT_GAP))
        .flex_shrink_0()
        .child(row(color.clone()))
        .child(row(color.clone()))
        .child(row(color))
}

pub struct CategorySortPage {
    clan_id: ClanId,
    channel_list: Entity<ChannelList>,
    categories: Vec<CategorySortItem>,
    saved_categories: Vec<CategorySortItem>,
    has_changed: bool,
    saving: bool,
    dragging_index: Option<usize>,
    drag_pointer_y: Option<Pixels>,
    _channel_sub: Subscription,
}

impl CategorySortPage {
    pub fn new(
        clan_id: ClanId,
        channel_list: Entity<ChannelList>,
        cx: &mut Context<Self>,
    ) -> Self {
        channel_list.update(cx, |list, cx| list.load_for_clan(clan_id, cx));
        let categories = Self::build_categories(channel_list.read(cx), clan_id);
        let saved_categories = categories.clone();

        Self {
            clan_id,
            channel_list: channel_list.clone(),
            categories,
            saved_categories,
            has_changed: false,
            saving: false,
            dragging_index: None,
            drag_pointer_y: None,
            _channel_sub: cx.observe(&channel_list, |this, _, cx| {
                this.resync_from_store(cx);
            }),
        }
    }

    pub fn release(&mut self) {}

    fn is_valid_sort_category(id: &str) -> bool {
        id != FAVOR_CATE_ID && id.parse::<i64>().is_ok_and(|id| id > 0)
    }

    fn build_categories(channel_list: &ChannelList, clan_id: ClanId) -> Vec<CategorySortItem> {
        let mut categories: Vec<_> = channel_list
            .categories_for_clan(clan_id)
            .iter()
            .filter(|category| Self::is_valid_sort_category(&category.id))
            .collect();
        categories.sort_by_key(|category| category.order);
        categories
            .into_iter()
            .map(|category| CategorySortItem {
                id: category.id.clone(),
                name: category.name.clone(),
                category_id: category.id.parse().unwrap_or(0),
            })
            .collect()
    }

    fn resync_from_store(&mut self, cx: &mut Context<Self>) {
        if self.has_changed || self.saving {
            return;
        }
        let fresh = Self::build_categories(self.channel_list.read(cx), self.clan_id);
        if fresh != self.categories {
            self.categories = fresh.clone();
            self.saved_categories = fresh;
            cx.notify();
        }
    }

    fn persist_order(&mut self, cx: &mut Context<Self>) {
        if self.saving || !self.has_changed {
            return;
        }

        self.saving = true;
        cx.notify();

        let clan_id = self.clan_id;
        let category_ids: Vec<i64> = self.categories.iter().map(|item| item.category_id).collect();
        let task = self.channel_list.update(cx, |list, cx| {
            list.update_categories_order(clan_id, &category_ids, cx)
        });

        cx.spawn(async move |this, cx| {
            match task.await {
                Ok(()) => {
                    let _ = this.update(cx, |this, cx| {
                        this.saved_categories = this.categories.clone();
                        this.has_changed = false;
                        this.saving = false;
                        cx.notify();
                    });
                }
                Err(err) => {
                    tracing::error!("update categories order failed: {err}");
                    let _ = this.update(cx, |this, cx| {
                        this.categories = this.saved_categories.clone();
                        this.has_changed = false;
                        this.saving = false;
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn handle_drag_move(&mut self, event: &DragMoveEvent<CategoryDrag>, cx: &mut Context<Self>) {
        let CategoryDrag(from) = *event.drag(cx);
        if self.dragging_index.is_none() {
            self.dragging_index = Some(from);
        }
        let Some(current) = self.dragging_index else {
            return;
        };
        let len = self.categories.len();
        if len == 0 {
            return;
        }

        let relative_y = event.event.position.y - event.bounds.top();
        let max_y = len as f32 * ROW_STEP - ROW_GAP;
        self.drag_pointer_y = Some(relative_y.clamp(px(0.0), px(max_y)));

        let mut target = (relative_y / px(ROW_STEP)).floor().max(0.0) as usize;
        target = target.min(len.saturating_sub(1));

        if current != target {
            let item = self.categories.remove(current);
            self.categories.insert(target, item);
            self.dragging_index = Some(target);
            self.has_changed = self.categories != self.saved_categories;
            cx.notify();
        }
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        self.dragging_index = None;
        self.drag_pointer_y = None;
        let should_save = self.has_changed && !self.saving;
        cx.notify();
        if should_save {
            self.persist_order(cx);
        }
    }

    fn render_category_row(
        &self,
        index: usize,
        item: &CategorySortItem,
        theme: &Theme,
    ) -> impl IntoElement {
        let is_dragging = self.dragging_index == Some(index);
        let is_drag_active = self.dragging_index.is_some();
        let style = if is_dragging {
            RowSurfaceStyle::DraggingSource
        } else {
            RowSurfaceStyle::Normal
        };
        let name = item.name.clone();

        let mut row = render_category_row_surface(&name, theme, style, None)
            .when(is_drag_active && !is_dragging, |el| el.opacity(0.72))
            .when(!is_dragging, |el| el.hover(|style| style.bg(theme.bg_hover)))
            .id(("category-sort-row", index));

        row = row.on_drag(CategoryDrag(index), |_, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| CategoryDragPreview)
            });

        row
    }
}

impl Render for CategorySortPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.dragging_index.is_some() && !cx.has_active_drag() {
            self.finish_drag(cx);
        }

        let theme = cx.theme().clone();
        let categories = self.categories.clone();
        let dragging_index = self.dragging_index;
        let drag_pointer_y = self.drag_pointer_y;
        let drag_preview = dragging_index.zip(drag_pointer_y).and_then(|(index, y)| {
            categories.get(index).map(|item| (item.name.clone(), y))
        });

        v_flex()
            .relative()
            .w_full()
            .child(
                div()
                    .id("category-sort-list")
                    .relative()
                    .w_full()
                    .on_drag_move(cx.listener(|this, event: &DragMoveEvent<CategoryDrag>, _, cx| {
                        this.handle_drag_move(event, cx);
                    }))
                    .on_drop(cx.listener(|this, _: &CategoryDrag, _, cx| {
                        this.finish_drag(cx);
                    }))
                    .child(
                        v_flex()
                            .gap(px(ROW_GAP))
                            .children(categories.iter().enumerate().map(|(index, item)| {
                                self.render_category_row(index, item, &theme)
                                    .into_any_element()
                            })),
                    )
                    .when_some(drag_preview, |list, (name, y)| {
                        list.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top(y - px(ROW_HEIGHT / 2.0))
                                .occlude()
                                .child(render_category_row_surface(
                                    &name,
                                    &theme,
                                    RowSurfaceStyle::DragPreview,
                                    None,
                                )),
                        )
                    }),
            )
    }
}
