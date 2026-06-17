use gpui::{App, Hsla, Refineable as _, StyleRefinement, Window, div, prelude::*};

use crate::theme::ActiveTheme;

#[derive(Clone, Copy, PartialEq)]
enum DividerDirection {
    Horizontal,
    Vertical,
}

#[derive(Default, Clone, Copy)]
pub enum DividerColor {
    Border,
    BorderFaded,
    #[default]
    BorderVariant,
}

impl DividerColor {
    fn hsla(self, cx: &App) -> Hsla {
        let border: Hsla = cx.theme().border.into();
        match self {
            DividerColor::Border | DividerColor::BorderVariant => border,
            DividerColor::BorderFaded => border.opacity(0.6),
        }
    }
}

#[derive(IntoElement)]
pub struct Divider {
    direction: DividerDirection,
    color: DividerColor,
    style: StyleRefinement,
    inset: bool,
}

impl Divider {
    pub fn horizontal() -> Self {
        Self {
            direction: DividerDirection::Horizontal,
            color: DividerColor::default(),
            style: StyleRefinement::default(),
            inset: false,
        }
    }

    pub fn vertical() -> Self {
        Self {
            direction: DividerDirection::Vertical,
            color: DividerColor::default(),
            style: StyleRefinement::default(),
            inset: false,
        }
    }

    pub fn inset(mut self) -> Self {
        self.inset = true;
        self
    }

    pub fn color(mut self, color: DividerColor) -> Self {
        self.color = color;
        self
    }
}

impl Styled for Divider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Divider {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self.color.hsla(cx);
        let mut base = match self.direction {
            DividerDirection::Horizontal => div()
                .min_w_0()
                .h_px()
                .max_h_px()
                .w_full()
                .when(self.inset, |this| this.mx_1p5()),
            DividerDirection::Vertical => div()
                .min_w_0()
                .w_px()
                .h_full()
                .when(self.inset, |this| this.my_1p5()),
        };
        base.style().refine(&self.style);
        base.bg(color)
    }
}
