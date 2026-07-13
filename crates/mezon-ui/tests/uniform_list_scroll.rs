use gpui::{
    Context, Pixels, Render, ScrollStrategy, TestAppContext, UniformListScrollHandle, Window, div,
    prelude::*, px, size, uniform_list,
};
use ui::{ScrollAxes, Scrollbars, WithScrollbar};

const ROW_PX: f32 = 52.;
const ROWS: usize = 200;
const TARGET_ROW: usize = 20;

struct Probe {
    scroll: UniformListScrollHandle,
    block_parent: bool,
    list_h_full: bool,
    scrollbars: bool,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = uniform_list("probe", ROWS, |range, _, _| {
            range
                .map(|_| div().h(px(ROW_PX)).w_full().into_any_element())
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll)
        .flex_1();
        let list = if self.list_h_full {
            list.h_full()
        } else {
            list
        };

        let column = div().flex_1().min_w_0().h_full().px_4().pb_4();
        let column = if self.block_parent {
            column
        } else {
            column.flex().flex_col()
        };
        let column = column.child(list);
        let column = if self.scrollbars {
            column
                .custom_scrollbars(
                    Scrollbars::always_visible(ScrollAxes::Vertical)
                        .tracked_scroll_handle(&self.scroll),
                    window,
                    cx,
                )
                .into_any_element()
        } else {
            column.into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            .child(column)
    }
}

fn offset_after_strict_scroll(
    cx: &mut TestAppContext,
    block_parent: bool,
    list_h_full: bool,
    scrollbars: bool,
) -> Pixels {
    cx.update(|cx| ::theme::init(::theme::LoadThemes::JustBase, cx));
    let scroll = UniformListScrollHandle::new();
    let (view, cx) = cx.add_window_view({
        let scroll = scroll.clone();
        move |_, _| Probe {
            scroll,
            block_parent,
            list_h_full,
            scrollbars,
        }
    });
    cx.simulate_resize(size(px(500.), px(400.)));
    cx.run_until_parked();

    scroll.scroll_to_item_strict(TARGET_ROW, ScrollStrategy::Top);
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    scroll.0.borrow().base_handle.offset().y
}

#[gpui::test]
fn emoji_layout_flex_column_pins_item_to_top(cx: &mut TestAppContext) {
    let offset = offset_after_strict_scroll(cx, false, false, false);
    assert_eq!(
        offset,
        px(-(TARGET_ROW as f32) * ROW_PX),
        "flex-column parent + flex_1 list should pin the target row to the top"
    );
}

#[gpui::test]
fn sound_layout_block_parent_pins_item_to_top(cx: &mut TestAppContext) {
    let offset = offset_after_strict_scroll(cx, true, true, false);
    assert_eq!(
        offset,
        px(-(TARGET_ROW as f32) * ROW_PX),
        "block parent + h_full list should pin the target row to the top"
    );
}

#[gpui::test]
fn tracked_scrollbars_do_not_defeat_strict_scroll(cx: &mut TestAppContext) {
    let offset = offset_after_strict_scroll(cx, true, true, true);
    assert_eq!(
        offset,
        px(-(TARGET_ROW as f32) * ROW_PX),
        "custom_scrollbars tracking the same handle must not clobber the strict scroll"
    );
}
