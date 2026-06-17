use gpui::{App, FontWeight, Window, div, prelude::*, px};

use super::stack::h_flex;
use crate::theme::ActiveTheme;

#[derive(IntoElement)]
pub struct Badge {
    count: usize,
}

impl Badge {
    pub fn new() -> Self {
        Self { count: 0 }
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }
}

impl Default for Badge {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let label = if self.count > 99 {
            "99+".to_string()
        } else {
            self.count.to_string()
        };

        h_flex()
            .h(px(14.))
            .min_w(px(14.))
            .p_px()
            .justify_center()
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(theme.border)
            .bg(theme.mention_badge)
            .shadow_sm()
            .child(
                div()
                    .text_size(px(9.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_primary)
                    .child(label),
            )
    }
}
