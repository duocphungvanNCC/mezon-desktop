use gpui::{App, Window, div, prelude::*, px, relative};

use crate::theme::ActiveTheme;

#[derive(IntoElement)]
pub struct Progress {
    value: f32,
}

impl Progress {
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
        }
    }
}

impl RenderOnce for Progress {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .w_full()
            .h(px(6.))
            .rounded_full()
            .bg(theme.bg_tertiary)
            .child(
                div()
                    .h_full()
                    .w(relative(self.value))
                    .rounded_full()
                    .bg(theme.brand),
            )
    }
}
