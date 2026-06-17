use gpui::{App, ElementId, Hsla, Window, div, prelude::*, px, transparent_black};

use super::stack::h_flex;
use crate::theme::ActiveTheme;

type ToggleHandler = Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    on_click: Option<ToggleHandler>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            checked: false,
            disabled: false,
            on_click: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let is_on = self.checked;
        let enabled = !self.disabled;
        let thumb_color: Hsla = theme.text_primary.into();

        let (bg_color, border_color) = if is_on {
            (Hsla::from(theme.brand), Hsla::from(theme.brand))
        } else {
            (Hsla::from(theme.bg_tertiary), Hsla::from(theme.border))
        };
        let bg_hover_color = if is_on {
            Hsla::from(theme.brand_hover)
        } else {
            Hsla::from(theme.bg_hover)
        };

        let thumb_opacity = match (is_on, self.disabled) {
            (_, true) => 0.2,
            (true, false) => 1.0,
            (false, false) => 0.5,
        };
        let checked = self.checked;

        div()
            .id(self.id)
            .p(px(1.))
            .border_2()
            .border_color(transparent_black())
            .rounded_full()
            .when(enabled, |this| this.cursor_pointer())
            .when_some(self.on_click.filter(|_| enabled), |this, handler| {
                this.on_click(move |_, window, cx| handler(&!checked, window, cx))
            })
            .child(
                h_flex()
                    .w(px(32.))
                    .h(px(20.))
                    .when(is_on, |on| on.justify_end())
                    .when(!is_on, |off| off.justify_start())
                    .items_center()
                    .rounded_full()
                    .px(px(2.))
                    .bg(bg_color)
                    .border_1()
                    .border_color(border_color)
                    .when(enabled, |this| this.hover(|s| s.bg(bg_hover_color)))
                    .child(
                        div()
                            .size(px(12.))
                            .rounded_full()
                            .bg(thumb_color)
                            .opacity(thumb_opacity),
                    ),
            )
    }
}
