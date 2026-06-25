use crate::app::window_controls;
use crate::theme::ActiveTheme;
use gpui::{Context, Entity, MouseButton, Window, div, prelude::*};
use mezon_store::Settings;

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

        div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h_8()
            .bg(theme.title_bar_bg)
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count >= 2 {
                    window.zoom_window();
                } else {
                    window.start_window_move();
                }
            })
            .child(div().flex_1())
            .child(window_controls::render_controls(theme))
    }
}
