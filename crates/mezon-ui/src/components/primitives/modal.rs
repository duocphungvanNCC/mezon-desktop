use std::rc::Rc;

use gpui::{
    AnyElement, App, MouseButton, SharedString, Window, deferred, div, hsla, prelude::*, px,
};

use super::stack::{h_flex, v_flex};
use crate::theme::ActiveTheme;

type DismissHandler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Modal {
    title: SharedString,
    body: Option<AnyElement>,
    actions: Vec<AnyElement>,
    on_dismiss: Option<DismissHandler>,
}

impl Modal {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            body: None,
            actions: Vec::new(),
            on_dismiss: None,
        }
    }

    pub fn child(mut self, body: impl IntoElement) -> Self {
        self.body = Some(body.into_any_element());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(on_dismiss));
        self
    }
}

impl RenderOnce for Modal {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let dismiss = self.on_dismiss.clone();

        let card = v_flex()
            .w(px(440.))
            .gap_4()
            .p(px(20.))
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_floating)
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text_primary)
                    .child(self.title),
            )
            .when_some(self.body, |el, body| el.child(body))
            .when(!self.actions.is_empty(), |el| {
                el.child(h_flex().justify_end().gap_2().children(self.actions))
            });

        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(hsla(0., 0., 0., 0.5))
                .when_some(dismiss, |el, handler| {
                    el.on_mouse_down(MouseButton::Left, move |_, window, cx| handler(window, cx))
                })
                .child(card),
        )
    }
}
