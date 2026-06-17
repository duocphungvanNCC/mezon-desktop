use gpui::{AnyElement, App, Hsla, Pixels, SharedString, Window, div, img, prelude::*, px};

use super::sizing::{Sizable, Size};
use crate::theme::ActiveTheme;

#[derive(IntoElement)]
pub struct Avatar {
    src: Option<SharedString>,
    name: Option<SharedString>,
    size: Size,
    border_color: Option<Hsla>,
    indicator: Option<AnyElement>,
}

impl Avatar {
    pub fn new() -> Self {
        Self {
            src: None,
            name: None,
            size: Size::Medium,
            border_color: None,
            indicator: None,
        }
    }

    pub fn src(mut self, src: impl Into<SharedString>) -> Self {
        self.src = Some(src.into());
        self
    }

    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn border_color(mut self, color: impl Into<Hsla>) -> Self {
        self.border_color = Some(color.into());
        self
    }

    pub fn indicator<E: IntoElement>(mut self, indicator: impl Into<Option<E>>) -> Self {
        self.indicator = indicator.into().map(IntoElement::into_any_element);
        self
    }
}

impl Default for Avatar {
    fn default() -> Self {
        Self::new()
    }
}

impl Sizable for Avatar {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

fn diameter(size: Size) -> Pixels {
    match size {
        Size::XSmall => px(20.),
        Size::Small => px(40.),
        Size::Medium => px(48.),
        Size::Large => px(80.),
    }
}

fn initials_circle(d: Pixels, bg: Hsla, text_color: Hsla, initials: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(d)
        .rounded_full()
        .bg(bg)
        .text_color(text_color)
        .text_size(d * 0.4)
        .child(initials)
        .into_any_element()
}

impl RenderOnce for Avatar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let border_width = if self.border_color.is_some() {
            px(1.)
        } else {
            px(0.)
        };

        let image_size = diameter(self.size);
        let container_size = image_size + border_width * 2.;

        let bg = Hsla::from(cx.theme().brand);
        let text_color = Hsla::from(cx.theme().text_primary);
        let element_bg = Hsla::from(cx.theme().bg_tertiary);
        let initials = name_initials(self.name.clone().unwrap_or_default().as_ref());

        div()
            .size(container_size)
            .rounded_full()
            .when_some(self.border_color, |this, color| {
                this.border(border_width).border_color(color)
            })
            .child(match self.src {
                Some(src) => {
                    let loading_initials = initials.clone();
                    let fallback_initials = initials;
                    img(src)
                        .size(image_size)
                        .rounded_full()
                        .bg(element_bg)
                        .with_loading(move || {
                            initials_circle(image_size, bg, text_color, loading_initials.clone())
                        })
                        .with_fallback(move || {
                            initials_circle(image_size, bg, text_color, fallback_initials.clone())
                        })
                        .into_any_element()
                }
                None => initials_circle(image_size, bg, text_color, initials),
            })
            .children(self.indicator.map(|indicator| div().child(indicator)))
    }
}

fn name_initials(name: &str) -> String {
    let result: String = name
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-' | '@'))
        .filter(|part| !part.is_empty())
        .take(2)
        .filter_map(|part| part.chars().next())
        .collect::<String>()
        .to_uppercase();
    if result.is_empty() {
        "?".to_string()
    } else {
        result
    }
}
