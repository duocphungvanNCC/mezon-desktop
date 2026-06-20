use crate::components::primitives::{Icon, IconName};
use crate::theme::ActiveTheme;
use gpui::{Context, Entity, MouseButton, Window, div, prelude::*, rgb};
use mezon_store::Settings;

/// Custom frameless title bar.
pub struct TitleBar {}

impl TitleBar {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        Self {}
    }
}

impl Render for TitleBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let bg_hover = theme.bg_hover;

        div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h_8()
            .bg(theme.title_bar_bg)
            // Drag region — move window by dragging the title bar
            .on_mouse_down(MouseButton::Left, |_, window, _| {
                window.start_window_move();
            })
            .child(div().flex_1())
            // Window controls (Windows/Linux only — macOS hides traffic lights)
            .when(cfg!(not(target_os = "macos")), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .h_full()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .w_11()
                                .h_full()
                                .text_sm()
                                .text_color(theme.text_secondary)
                                .hover(move |s| s.bg(bg_hover))
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                    window.minimize_window();
                                })
                                .child(
                                    Icon::new(IconName::WindowMinimize)
                                        .size_4()
                                        .text_color(theme.text_secondary),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .w_11()
                                .h_full()
                                .text_sm()
                                .text_color(theme.text_secondary)
                                .hover(move |s| s.bg(bg_hover))
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                    window.zoom_window();
                                })
                                .child(
                                    Icon::new(IconName::WindowZoom)
                                        .size_4()
                                        .text_color(theme.text_secondary),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .w_11()
                                .h_full()
                                .text_sm()
                                .text_color(theme.text_secondary)
                                .hover(|s| s.bg(rgb(0xc42b1c)))
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                    window.remove_window();
                                })
                                .child(
                                    Icon::new(IconName::Close)
                                        .size_4()
                                        .text_color(theme.text_secondary),
                                ),
                        ),
                )
            })
    }
}
