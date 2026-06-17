use std::rc::Rc;

use gpui::{App, SharedString, Window, div, prelude::*, px};

use super::stack::h_flex;
use crate::theme::ActiveTheme;

type SelectHandler = Rc<dyn Fn(usize, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct TabBar {
    tabs: Vec<SharedString>,
    selected: usize,
    on_select: Option<SelectHandler>,
}

impl TabBar {
    pub fn new(tabs: Vec<SharedString>) -> Self {
        Self {
            tabs,
            selected: 0,
            on_select: None,
        }
    }

    pub fn selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for TabBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let selected = self.selected;
        let on_select = self.on_select.clone();

        h_flex()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .children(self.tabs.into_iter().enumerate().map(|(index, label)| {
                let is_active = index == selected;
                let on_select = on_select.clone();
                div()
                    .id(("tab", index))
                    .px(px(12.))
                    .py(px(8.))
                    .text_sm()
                    .cursor_pointer()
                    .border_b_2()
                    .when(is_active, |el| {
                        el.border_color(theme.brand).text_color(theme.text_primary)
                    })
                    .when(!is_active, |el| {
                        el.border_color(gpui::transparent_black())
                            .text_color(theme.text_secondary)
                            .hover(|s| s.text_color(theme.text_primary))
                    })
                    .child(label)
                    .when_some(on_select, |el, handler| {
                        el.on_click(move |_, window, cx| handler(index, window, cx))
                    })
            }))
    }
}
