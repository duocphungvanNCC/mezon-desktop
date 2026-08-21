use std::rc::Rc;

use gpui::{App, Context, FocusHandle, SharedString, Task, Window, div, prelude::*, px};

use super::Shell;
use crate::components::primitives::{Button, ButtonVariants, h_flex, v_flex};
use crate::theme::ActiveTheme;

pub(super) type ConfirmAction = Rc<dyn Fn(&mut App) -> Task<anyhow::Result<()>>>;
pub(super) type ConfirmEpilogue = Rc<dyn Fn(&mut App)>;

pub(super) struct ConfirmDestructiveModal {
    pub(super) focus_handle: FocusHandle,
    pub(super) cancel_id: SharedString,
    pub(super) confirm_id: SharedString,
    pub(super) title: SharedString,
    pub(super) description: SharedString,
    pub(super) cancel_label: SharedString,
    pub(super) confirm_label: SharedString,
    pub(super) failed_message: SharedString,
    pub(super) action: ConfirmAction,
    pub(super) after_success: ConfirmEpilogue,
    pub(super) running: bool,
}

impl Render for ConfirmDestructiveModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let running = self.running;

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
                        Button::new(self.cancel_id.clone())
                            .label(self.cancel_label.clone())
                            .ghost()
                            .on_click(|_, _window, cx| {
                                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                            }),
                    )
                    .child(
                        Button::new(self.confirm_id.clone())
                            .label(self.confirm_label.clone())
                            .danger()
                            .disabled(running)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                if this.running {
                                    return;
                                }
                                this.running = true;
                                cx.notify();
                                let failed = this.failed_message.clone();
                                let after_success = this.after_success.clone();
                                let task = (this.action)(cx);
                                cx.spawn(async move |this, cx| {
                                    let result = task.await;
                                    let owner = this.entity_id();
                                    let _ = this.update(cx, |this, cx| {
                                        this.running = false;
                                        cx.notify();
                                    });
                                    cx.update(|cx| {
                                        Shell::global(cx).update(cx, |shell, cx| {
                                            shell.close_modal_if_current(owner, cx);
                                            if result.is_err() {
                                                shell.error(failed, cx);
                                            }
                                        });
                                        if result.is_ok() {
                                            after_success(cx);
                                        }
                                    });
                                })
                                .detach();
                            })),
                    ),
            )
    }
}
