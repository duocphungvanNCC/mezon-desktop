use gpui::{AnyElement, Hsla, Rgba, div, prelude::*, px};
use mezon_store::Message;

use crate::components::primitives::{Avatar, Sizable, Size};
use crate::theme::Theme;

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

pub struct MessageRow {
    message: Message,
    combined: bool,
    reply: bool,
    theme: Theme,
}

impl MessageRow {
    pub fn new(message: Message, theme: &Theme, _current_user_id: &str) -> Self {
        Self {
            message,
            combined: false,
            reply: false,
            theme: theme.clone(),
        }
    }

    pub fn combined(mut self, combined: bool) -> Self {
        self.combined = combined;
        self
    }

    pub fn reply(mut self, reply: bool) -> Self {
        self.reply = reply;
        self
    }

    fn format_timestamp(ts: i64) -> String {
        let seconds_since_midnight = ts % 86400;
        let hours = seconds_since_midnight / 3600;
        let minutes = (seconds_since_midnight % 3600) / 60;

        let period = if hours >= 12 { "PM" } else { "AM" };
        let display_hour = if hours == 0 {
            12
        } else if hours > 12 {
            hours - 12
        } else {
            hours
        };
        format!("{}:{:02} {}", display_hour, minutes, period)
    }

    pub fn render(&self) -> impl IntoElement {
        let msg = &self.message;
        let theme = &self.theme;
        let time = Self::format_timestamp(msg.create_time);

        let display_name = &msg.sender_name;

        let mut avatar = Avatar::new().name(display_name).with_size(Size::Small);
        if !msg.avatar_url.is_empty() {
            avatar = avatar.src(msg.avatar_url.clone());
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
            .when(!msg.attachments.is_empty(), |d| {
                d.child(div().flex().flex_col().gap_2().mt_1().children(
                    msg.attachments.iter().map(|att| {
                        if att.is_image() {
                            let name = if att.filename.is_empty() {
                                "image".to_string()
                            } else {
                                att.filename.clone()
                            };
                            let box_bg = gpui::hsla(0., 0., 0., 0.03);
                            let box_border = theme.border;
                            let box_color = theme.text_muted;
                            // let loading_name = name.clone();
                            // img(att.url.clone())
                            //     .max_w(px(400.))
                            //     .max_h(px(300.))
                            //     .rounded_md()
                            //     .with_loading(move || {
                            //         attachment_box(
                            //             format!("Loading {loading_name}…"),
                            //             box_bg,
                            //             box_border,
                            //             box_color,
                            //         )
                            //     })
                            //     .with_fallback(move || {
                            //         attachment_box(
                            //             format!("Couldn't load {name}"),
                            //             box_bg,
                            //             box_border,
                            //             box_color,
                            //         )
                            //     })
                            //     .into_any_element()
                            attachment_box(name, box_bg, box_border, box_color).into_any_element()
                        } else {
                            let label = if att.filename.is_empty() {
                                "Attachment".to_string()
                            } else {
                                att.filename.clone()
                            };
                            div()
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
                                .child(label)
                                .into_any_element()
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
