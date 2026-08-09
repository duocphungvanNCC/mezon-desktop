use gpui::{Context, Pixels, Render, Rgba, SharedString, Window, div, prelude::*, px};
use unicode_segmentation::UnicodeSegmentation;

pub struct CustomStatusBubble {
    text: SharedString,
    max_width: Pixels,
    background: Rgba,
    border: Rgba,
    text_color: Rgba,
    expandable: bool,
    expanded: bool,
}

impl CustomStatusBubble {
    pub fn new() -> Self {
        Self {
            text: SharedString::default(),
            max_width: px(0.),
            background: gpui::rgba(0x00000000),
            border: gpui::rgba(0x00000000),
            text_color: gpui::rgba(0x00000000),
            expandable: false,
            expanded: false,
        }
    }

    pub fn set_content(
        &mut self,
        text: SharedString,
        max_width: Pixels,
        background: Rgba,
        border: Rgba,
        text_color: Rgba,
        cx: &mut Context<Self>,
    ) {
        if self.text == text
            && self.max_width == max_width
            && self.background == background
            && self.border == border
            && self.text_color == text_color
        {
            return;
        }
        self.text = text;
        self.max_width = max_width;
        self.background = background;
        self.border = border;
        self.text_color = text_color;
        self.expandable = false;
        self.expanded = false;
        cx.notify();
    }

    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let expanded = expanded && self.expandable;
        if self.expanded != expanded {
            self.expanded = expanded;
            cx.notify();
        }
    }

    fn surface(&self, text: SharedString, expanded: bool) -> impl IntoElement {
        div()
            .id(if expanded {
                "custom-status-expanded"
            } else {
                "custom-status-collapsed"
            })
            .w_full()
            .min_w_0()
            .px_4()
            .py_3()
            .rounded_xl()
            .bg(self.background)
            .border_1()
            .border_color(self.border)
            .shadow_md()
            .text_sm()
            .text_color(self.text_color)
            .overflow_hidden()
            .whitespace_normal()
            .when(!expanded, |surface| surface.h(px(64.)))
            .child(
                div()
                    .id(if expanded {
                        "custom-status-expanded-text"
                    } else {
                        "custom-status-collapsed-text"
                    })
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .when(!expanded, |text| text.line_clamp(2).text_ellipsis())
                    .child(text),
            )
    }
}

impl Render for CustomStatusBubble {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let run = window.text_style().to_run(self.text.len());
        let text_width = window
            .text_system()
            .shape_line(self.text.clone(), px(14.), &[run], None)
            .width();
        let width = (text_width + px(40.)).min(self.max_width);
        self.expandable = text_width + px(40.) > self.max_width;
        let show_full = self.expanded || !self.expandable;
        let text = break_long_words(self.text.as_ref());

        div()
            .id("custom-status-bubble")
            .relative()
            .w(width)
            .child(self.surface(text, show_full))
    }
}

fn break_long_words(text: &str) -> SharedString {
    text.split_inclusive(char::is_whitespace)
        .map(|segment| {
            let word = segment.trim_end_matches(char::is_whitespace);
            let whitespace = &segment[word.len()..];
            if UnicodeSegmentation::graphemes(word, true).count() <= 24 {
                segment.to_owned()
            } else {
                let mut breakable = UnicodeSegmentation::graphemes(word, true)
                    .collect::<Vec<_>>()
                    .join("\u{200b}");
                breakable.push_str(whitespace);
                breakable
            }
        })
        .collect::<String>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::break_long_words;

    #[test]
    fn keeps_short_status_text_unchanged() {
        assert_eq!(break_long_words("oh hi").as_ref(), "oh hi");
    }

    #[test]
    fn adds_break_points_only_to_long_tokens() {
        let url = "oremIpsumissimplydummytextoftheprintingandtypesettingindustry";
        assert!(break_long_words(url).contains('\u{200b}'));
        assert!(!break_long_words("Lorem Ipsum is simply dummy text").contains('\u{200b}'));
    }
}
