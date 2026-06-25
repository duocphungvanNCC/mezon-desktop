use gpui::{Div, MouseButton, Pixels, Rgba, StyleRefinement, Window, div, prelude::*, px, rgb};

use crate::components::primitives::{Icon, IconName};
use crate::theme::Theme;

const BUTTON_SIZE: f32 = 28.0;
const ICON_SIZE: f32 = 12.0;
const CLOSE_HOVER: u32 = 0xc42b1c;

pub fn render_controls(theme: &Theme, window: &Window) -> impl IntoElement {
    let hover = theme.bg_hover;
    let color = theme.text_secondary;
    let icon_size = px(ICON_SIZE);
    let zoom_icon = if window.is_maximized() {
        IconName::WindowRestore
    } else {
        IconName::WindowMaximize
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .h_full()
        .child(window_control_button(
            color,
            icon_size,
            IconName::WindowMinimize,
            move |style| style.bg(hover),
            |window| window.minimize_window(),
        ))
        .child(window_control_button(
            color,
            icon_size,
            zoom_icon,
            move |style| style.bg(hover),
            |window| window.zoom_window(),
        ))
        .child(window_control_button(
            color,
            icon_size,
            IconName::WindowClose,
            |style| style.bg(rgb(CLOSE_HOVER)).text_color(gpui::white()),
            |window| window.remove_window(),
        ))
}

fn window_control_button(
    color: Rgba,
    icon_size: Pixels,
    icon: IconName,
    hover: impl FnOnce(StyleRefinement) -> StyleRefinement + 'static,
    on_click: impl Fn(&mut Window) + 'static,
) -> Div {
    button(color)
        .hover(hover)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            on_click(window);
        })
        .child(Icon::new(icon).size(icon_size).text_color(color))
}

fn button(color: Rgba) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(BUTTON_SIZE))
        .rounded_full()
        .cursor_pointer()
        .text_color(color)
}
