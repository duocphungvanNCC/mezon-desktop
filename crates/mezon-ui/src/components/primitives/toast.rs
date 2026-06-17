use gpui::{App, SharedString, Window, div, prelude::*, px};

use super::icon::{Icon, IconName};
use super::stack::h_flex;
use crate::theme::ActiveTheme;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastKind {
    #[default]
    Info,
    Success,
    Error,
}

#[derive(IntoElement)]
pub struct Toast {
    message: SharedString,
    kind: ToastKind,
}

impl Toast {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            kind: ToastKind::Info,
        }
    }

    pub fn kind(mut self, kind: ToastKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn success(message: impl Into<SharedString>) -> Self {
        Self::new(message).kind(ToastKind::Success)
    }

    pub fn error(message: impl Into<SharedString>) -> Self {
        Self::new(message).kind(ToastKind::Error)
    }
}

impl RenderOnce for Toast {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (accent, icon) = match self.kind {
            ToastKind::Info => (theme.text_link, IconName::Inbox),
            ToastKind::Success => (theme.status_online, IconName::Check),
            ToastKind::Error => (theme.status_dnd, IconName::TriangleAlert),
        };

        h_flex()
            .gap_2()
            .items_center()
            .min_w(px(240.))
            .max_w(px(360.))
            .px(px(12.))
            .py(px(10.))
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_floating)
            .shadow_lg()
            .child(Icon::new(icon).size_4().text_color(accent))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(theme.text_primary)
                    .child(self.message),
            )
    }
}
