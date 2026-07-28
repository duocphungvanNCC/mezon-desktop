use gpui::{Context, FocusHandle, SharedString, WeakEntity, Window, div, prelude::*, px};

use super::Shell;
use crate::clan::settings::CommunitySettingPage;
use crate::components::primitives::{Button, ButtonVariants, h_flex, v_flex};
use crate::theme::ActiveTheme;

pub(super) struct DisableClanCommunityModal {
    pub(super) focus_handle: FocusHandle,
    pub(super) page: WeakEntity<CommunitySettingPage>,
    pub(super) title: SharedString,
    pub(super) description: SharedString,
    pub(super) cancel_label: SharedString,
    pub(super) confirm_label: SharedString,
}

impl Render for DisableClanCommunityModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let page = self.page.clone();

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
                    .child(self.description.clone()),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("disable-clan-community-cancel")
                            .label(self.cancel_label.clone())
                            .ghost()
                            .on_click(|_, _window, cx| {
                                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                            }),
                    )
                    .child(
                        Button::new("disable-clan-community-confirm")
                            .label(self.confirm_label.clone())
                            .danger()
                            .on_click(move |_, _window, cx| {
                                let already_saving =
                                    page.upgrade().is_some_and(|page| page.read(cx).is_saving());
                                if already_saving {
                                    return;
                                }
                                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                                let _ = page.update(cx, |page, cx| {
                                    page.disable_community(cx);
                                });
                            }),
                    ),
            )
    }
}
