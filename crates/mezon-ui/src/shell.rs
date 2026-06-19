//! App shell: a per-window aggregate (cf. Zed's `Workspace`) owning the cross-cutting overlay
//! layers — toasts and the modal stack — rendered on top of everything by `RootView`.
//!
//! Any view can surface a toast or a modal from anywhere via [`Shell::global`], instead of each
//! page wiring its own local toast/dialog state.

use std::time::Duration;

use gpui::{
    AnyView, App, AppContext, Context, Entity, Global, MouseButton, SharedString, deferred, div,
    hsla, prelude::*, px,
};

use crate::components::primitives::{Toast, ToastKind};

const TOAST_TTL: Duration = Duration::from_secs(4);

struct ToastItem {
    id: usize,
    message: SharedString,
    kind: ToastKind,
}

/// Owns the window-level overlay layers (toasts + active modal). Registered as a [`Global`].
pub struct Shell {
    toasts: Vec<ToastItem>,
    modal: Option<AnyView>,
    next_id: usize,
}

struct GlobalShell(Entity<Shell>);
impl Global for GlobalShell {}

impl Shell {
    pub fn init(cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|_| Self {
            toasts: Vec::new(),
            modal: None,
            next_id: 0,
        });
        cx.set_global(GlobalShell(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalShell>().0.clone()
    }

    /// Show a transient toast; it auto-dismisses after [`TOAST_TTL`].
    pub fn toast(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.toasts.push(ToastItem {
            id,
            message: message.into(),
            kind,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_TTL).await;
            let _ = this.update(cx, |this, cx| {
                this.toasts.retain(|t| t.id != id);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn info(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast(ToastKind::Info, message, cx);
    }

    pub fn success(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast(ToastKind::Success, message, cx);
    }

    pub fn error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast(ToastKind::Error, message, cx);
    }

    /// Show `view` as the active modal (backdrop click dismisses). The view renders its own card.
    pub fn show_modal(&mut self, view: AnyView, cx: &mut Context<Self>) {
        self.modal = Some(view);
        cx.notify();
    }

    pub fn close_modal(&mut self, cx: &mut Context<Self>) {
        if self.modal.take().is_some() {
            cx.notify();
        }
    }

    pub fn has_modal(&self) -> bool {
        self.modal.is_some()
    }

    /// The overlay (modal backdrop + toast stack), rendered on top by `RootView`.
    pub fn render_overlay(&self) -> impl IntoElement {
        let modal = self.modal.clone();
        let toasts: Vec<(SharedString, ToastKind)> = self
            .toasts
            .iter()
            .map(|t| (t.message.clone(), t.kind))
            .collect();

        div()
            .when_some(modal, |el, view| {
                el.child(deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(hsla(0., 0., 0., 0.5))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                        })
                        .child(div().occlude().child(view)),
                ))
            })
            .when(!toasts.is_empty(), |el| {
                el.child(deferred(
                    div()
                        .absolute()
                        .top(px(44.))
                        .right(px(16.))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(
                            toasts
                                .into_iter()
                                .map(|(message, kind)| Toast::new(message).kind(kind)),
                        ),
                ))
            })
    }
}
