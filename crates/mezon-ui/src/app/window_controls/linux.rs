use gpui::{App, Div, MouseButton, Pixels, Rgba, StyleRefinement, Window, prelude::*, px, rgb};

use crate::components::primitives::{Icon, IconName};
use crate::theme::Theme;

use super::{CONTROL_CLOSE_HOVER, control_button, controls_row, hide_main_window};

pub fn render_controls(theme: &Theme, window: &Window) -> impl IntoElement {
    let hover = theme.bg_hover;
    let color = theme.text_secondary;
    let icon_size = px(super::CONTROL_ICON_SIZE);
    let zoom_icon = if window.is_maximized() {
        IconName::WindowRestore
    } else {
        IconName::WindowMaximize
    };

    controls_row()
        .child(window_control_button(
            color,
            icon_size,
            IconName::WindowMinimize,
            move |style| style.bg(hover),
            |window, _| window.minimize_window(),
        ))
        .child(window_control_button(
            color,
            icon_size,
            zoom_icon,
            move |style| style.bg(hover),
            |window, _| window.zoom_window(),
        ))
        .child(window_control_button(
            color,
            icon_size,
            IconName::WindowClose,
            |style| style.bg(rgb(CONTROL_CLOSE_HOVER)).text_color(gpui::white()),
            |window, cx| {
                // Hide-to-tray belongs to the main window alone. These controls are shared by
                // every window that uses the app title bar, and hiding a secondary window instead
                // of closing it leaves it alive but unmapped: reopening then re-maps it and the
                // window manager, not the app, picks where it lands.
                let is_main = crate::app::main_window::handle(cx) == Some(window.window_handle());
                if !is_main || !hide_main_window(window, cx) {
                    window.remove_window();
                }
            },
        ))
}

fn window_control_button(
    color: Rgba,
    icon_size: Pixels,
    icon: IconName,
    hover: impl FnOnce(StyleRefinement) -> StyleRefinement + 'static,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    control_button(color)
        .hover(hover)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .child(Icon::new(icon).size(icon_size).text_color(color))
}
