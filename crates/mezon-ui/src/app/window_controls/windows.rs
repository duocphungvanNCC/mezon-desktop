use gpui::{Div, MouseButton, Rgba, div, prelude::*, px, rgb};

use crate::components::primitives::{Icon, IconName};
use crate::theme::Theme;


pub fn render_controls(theme: &Theme) -> impl IntoElement {
    const CLOSE_HOVER: u32 = 0xc42b1c;
    let hover = theme.bg_hover;
    let color = theme.text_secondary;

    div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .child(
            button(color)
                .hover(move |s| s.bg(hover))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.minimize_window())
                .child(
                    Icon::new(IconName::WindowMinimize)
                        .size_4()
                        .text_color(color),
                ),
        )
        .child(
            button(color)
                .hover(move |s| s.bg(hover))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.zoom_window())
                .child(Icon::new(IconName::WindowZoom).size_4().text_color(color)),
        )
        .child(
            button(color)
                .hover(|s| s.bg(rgb(CLOSE_HOVER)).text_color(gpui::white()))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.remove_window())
                .child(Icon::new(IconName::Close).size_4().text_color(color)),
        )
}

fn button(color: Rgba) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(46.))
        .h_full()
        .cursor_pointer()
        .text_color(color)
}
