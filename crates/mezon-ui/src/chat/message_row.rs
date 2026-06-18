use gpui::{AnyElement, Hsla, Pixels, Rgba, div, img, prelude::*, px};
use mezon_store::Message;

use crate::components::primitives::{Avatar, Sizable, Size};
use crate::theme::Theme;

pub enum MessageAttachmentView {
    Image {
        src: String,
        width: Pixels,
        height: Pixels,
        label: String,
    },
    File {
        label: String,
    },
}

fn attachment_box(label: String, bg: Hsla, border: Rgba, color: Rgba) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(240.))
        .h(px(120.))
        .rounded_md()
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_xs()
        .text_color(color)
        .child(label)
        .into_any_element()
}

pub struct MessageRow<'a> {
    message: &'a Message,
    combined: bool,
    reply: bool,
    theme: &'a Theme,
    avatar_src: Option<String>,
    attachments: Vec<MessageAttachmentView>,
}

impl<'a> MessageRow<'a> {
    pub fn new(message: &'a Message, theme: &'a Theme, _current_user_id: &str) -> Self {
        Self {
            message,
            combined: false,
            reply: false,
            theme,
            avatar_src: None,
            attachments: Vec::new(),
        }
    }

    pub fn attachments(mut self, views: Vec<MessageAttachmentView>) -> Self {
        self.attachments = views;
        self
    }

    pub fn avatar_src(mut self, src: impl Into<String>) -> Self {
        let src = src.into();
        self.avatar_src = if src.is_empty() { None } else { Some(src) };
        self
    }

    pub fn combined(mut self, combined: bool) -> Self {
        self.combined = combined;
        self
    }

    pub fn reply(mut self, reply: bool) -> Self {
        self.reply = reply;
        self
    }

    pub fn render(&self) -> impl IntoElement {
        crate::trace_render!("MessageRow {}", self.message.id);
        let msg = self.message;
        let theme = self.theme;
        let time = msg.timestamp_label.clone();

        let display_name = &msg.sender_name;

        let mut avatar = Avatar::new().name(display_name).with_size(Size::Small);
        if let Some(src) = self.avatar_src.as_deref().filter(|s| !s.is_empty()) {
            avatar = avatar.src(src);
        } else if !msg.avatar_url.is_empty() {
            avatar = avatar.src(msg.avatar_url.as_str());
        }

        let name_row = div()
            .flex()
            .flex_row()
            .items_baseline()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text_primary)
                    .child(display_name.to_string()),
            )
            .child(div().text_xs().text_color(theme.text_muted).child(time));

        let content = div()
            .text_sm()
            .text_color(theme.text_secondary)
            .child(msg.content.clone());

        let reply_placeholder = div()
            .id("reply-placeholder")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .mb_1()
            .child(
                div()
                    .w(px(2.))
                    .h_full()
                    .min_h(px(16.))
                    .rounded(px(2.))
                    .bg(gpui::hsla(0., 0., 0., 0.15)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child("Replying to someone"),
            );

        let content_area = div()
            .flex()
            .flex_col()
            .when(!self.combined, |d| d.pl(px(56.)))
            .when(self.combined, |d| d.pl(px(16.)))
            .child(if self.reply {
                reply_placeholder.into_any_element()
            } else {
                div().into_any_element()
            })
            .child(if !self.combined {
                name_row.into_any_element()
            } else {
                div().into_any_element()
            })
            .child(content)
            .when(!msg.reactions.is_empty(), |d| {
                d.child(
                    div()
                        .id("reactions-placeholder")
                        .flex()
                        .flex_row()
                        .gap_1()
                        .mt_1()
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_md()
                                .bg(gpui::hsla(0., 0., 0., 0.05))
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child("[👍 ❤️]"),
                        ),
                )
            })
            .when(!self.attachments.is_empty(), |d| {
                d.child(div().flex().flex_col().gap_2().mt_1().children(
                    self.attachments.iter().map(|view| {
                        match view {
                            MessageAttachmentView::Image {
                                src,
                                width,
                                height,
                                label,
                            } => {
                                if src.is_empty() {
                                    let box_bg = gpui::hsla(0., 0., 0., 0.03);
                                    attachment_box(
                                        label.clone(),
                                        box_bg,
                                        theme.border,
                                        theme.text_muted,
                                    )
                                    .into_any_element()
                                } else {
                                    img(src.clone())
                                        .w(*width)
                                        .h(*height)
                                        .max_w(px(400.))
                                        .rounded_md()
                                        .into_any_element()
                                }
                            }
                            MessageAttachmentView::File { label } => div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(gpui::hsla(0., 0., 0., 0.03))
                                .border_1()
                                .border_color(theme.border)
                                .text_xs()
                                .text_color(theme.text_secondary)
                                .child(label.clone())
                                .into_any_element(),
                        }
                    }),
                ))
            });

        div()
            .flex()
            .flex_row()
            .w_full()
            .px_4()
            .py(px(2.))
            .when(!self.combined, |d| d.pt_3())
            .child(if self.combined {
                div().w(px(40.)).flex_none()
            } else {
                div().absolute().left(px(16.)).top(px(10.)).child(avatar)
            })
            .child(content_area)
    }
}
