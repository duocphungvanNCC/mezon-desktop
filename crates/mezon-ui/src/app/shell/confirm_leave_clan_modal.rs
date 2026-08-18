use gpui::{Context, FocusHandle, FontWeight, SharedString, Window, div, prelude::*, px};
use mezon_store::{ClanId, ClanList};

use super::Shell;
use crate::components::primitives::{Button, ButtonVariants, h_flex, v_flex};
use crate::router::{Route, navigate};
use crate::theme::ActiveTheme;

/// Confirm-then-leave a clan — React's `ModalConfirm` as wired by `PanelClan` (clan rail
/// context menu) and `ClanHeader` (clan name dropdown).
pub(super) struct ConfirmLeaveClanModal {
    pub(super) focus_handle: FocusHandle,
    pub(super) clan_id: ClanId,
    pub(super) title: SharedString,
    pub(super) description: SharedString,
    pub(super) cancel_label: SharedString,
    pub(super) confirm_label: SharedString,
    pub(super) error_message: SharedString,
}

impl ConfirmLeaveClanModal {
    fn leave(&self, cx: &mut Context<Self>) {
        let clan_id = self.clan_id;
        let error_message = self.error_message.clone();
        // React captures `currentClanId` at click time and only routes away when the clan it
        // left is the one being viewed (`PanelClan.handleLeaveClan`).
        let was_active = ClanList::global(cx).read(cx).is_active_clan(clan_id);
        let task = ClanList::global(cx).update(cx, |list, cx| list.leave_clan(clan_id, cx));
        cx.spawn(async move |_, cx| {
            let result = task.await;
            cx.update(|cx| match result {
                Ok(()) => {
                    if was_active {
                        navigate(cx, Route::Friends);
                    }
                }
                Err(error) => {
                    tracing::error!("leave clan {clan_id} failed: {error}");
                    Shell::global(cx).update(cx, |shell, cx| shell.error(error_message, cx));
                }
            });
        })
        .detach();
        Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
    }
}

impl Render for ConfirmLeaveClanModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("menu")
            .on_action(cx.listener(|_, _: &::menu::Cancel, _window, cx| {
                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
            }))
            .on_action(cx.listener(|this, _: &::menu::Confirm, _window, cx| this.leave(cx)))
            .w(px(440.))
            .gap_2()
            .p(px(24.))
            .rounded_xl()
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_floating)
            .shadow_lg()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_primary)
                    .truncate()
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
                    .pt(px(12.))
                    .justify_end()
                    .gap_3()
                    .child(
                        Button::new("leave-clan-cancel")
                            .label(self.cancel_label.clone())
                            .ghost()
                            .on_click(|_, _window, cx| {
                                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                            }),
                    )
                    .child(
                        Button::new("leave-clan-confirm")
                            .label(self.confirm_label.clone())
                            .danger()
                            .on_click(cx.listener(|this, _, _window, cx| this.leave(cx))),
                    ),
            )
    }
}
