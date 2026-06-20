use gpui::{Context, FocusHandle, SharedString, Window, div, prelude::*, px};

use super::Shell;
use crate::components::primitives::{Button, ButtonVariants, h_flex, v_flex};
use crate::theme::ActiveTheme;

pub(super) struct ComingSoonModal {
    pub(super) focus_handle: FocusHandle,
    pub(super) title: SharedString,
    pub(super) message: SharedString,
    pub(super) close_label: SharedString,
}

impl Render for ComingSoonModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("menu")
            .on_action(cx.listener(|_, _: &::menu::Cancel, _window, cx| {
                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
            }))
            .w(px(440.))
            .gap_4()
            .p(px(20.))
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_floating)
            .shadow_lg()
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text_primary)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_secondary)
                    .child(self.message.clone()),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("coming-soon-close")
                        .label(self.close_label.clone())
                        .primary()
                        .on_click(|_, _window, cx| {
                            Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                        }),
                ),
            )
    }
}
