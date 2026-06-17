use gpui::{App, Div, SharedString, StyleRefinement, Window, div, prelude::*};

#[derive(IntoElement)]
pub struct Label {
    base: Div,
    text: SharedString,
}

impl Label {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            text: text.into(),
        }
    }
}

impl Styled for Label {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Label {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.base.child(self.text)
    }
}
