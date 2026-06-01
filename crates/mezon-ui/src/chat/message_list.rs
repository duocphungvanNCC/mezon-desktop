use gpui::{div, prelude::*, px};

use mezon_store::Message;

use crate::chat::grouping::{group_messages, MessageGroup};
use crate::chat::message_row::MessageRow;
use crate::theme::Theme;

pub struct MessageList {
    messages: Vec<Message>,
    theme: Theme,
    current_user_id: String,
    current_username: String,
    typing_users: Vec<String>,
}

impl MessageList {
    pub fn new(
        messages: Vec<Message>,
        theme: &Theme,
        current_user_id: &str,
        current_username: &str,
    ) -> Self {
        Self {
            messages,
            theme: theme.clone(),
            current_user_id: current_user_id.to_string(),
            current_username: current_username.to_string(),
            typing_users: Vec::new(),
        }
    }

    pub fn with_typing_users(mut self, users: Vec<String>) -> Self {
        self.typing_users = users;
        self
    }

    fn date_separator(theme: &Theme) -> impl IntoElement {
        div()
            .id("date-separator")
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .w_full()
            .child(
                div().flex_1().h(px(1.)).bg(gpui::hsla(0., 0., 0., 0.08)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child("May 31, 2025"),
            )
            .child(
                div().flex_1().h(px(1.)).bg(gpui::hsla(0., 0., 0., 0.08)),
            )
    }

    fn unread_break(_theme: &Theme) -> impl IntoElement {
        div()
            .id("unread-break")
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .w_full()
            .child(div().flex_1().h(px(1.)).bg(gpui::hsla(0., 0., 0., 0.08)))
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(gpui::red())
                    .child("NEW MESSAGES"),
            )
            .child(div().flex_1().h(px(1.)).bg(gpui::red().alpha(0.3)))
    }

    fn typing_indicator(theme: &Theme, users: &[String]) -> impl IntoElement {
        let label = if users.is_empty() {
            "Someone is typing...".to_string()
        } else {
            format!("{} is typing...", users.join(", "))
        };

        div()
            .id("typing-indicator")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .child(
                div()
                    .w(px(3.))
                    .h(px(3.))
                    .rounded_full()
                    .bg(theme.text_muted),
            )
            .child(
                div()
                    .w(px(3.))
                    .h(px(3.))
                    .rounded_full()
                    .bg(theme.text_muted),
            )
            .child(
                div()
                    .w(px(3.))
                    .h(px(3.))
                    .rounded_full()
                    .bg(theme.text_muted),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(label),
            )
    }

    fn render_group(
        group: &MessageGroup,
        theme: &Theme,
        current_user_id: &str,
        current_username: &str,
    ) -> impl IntoElement {
        let mut children: Vec<gpui::AnyElement> = Vec::new();

        for (i, msg) in group.messages.iter().enumerate() {
            let is_combined = group.combined[i];
            let row = MessageRow::new((*msg).clone(), theme, current_user_id, current_username)
                .combined(is_combined);
            let rendered = row.render();
            children.push(rendered.into_any_element());
        }

        div().flex().flex_col().w_full().children(children)
    }

    pub fn render(&self) -> impl IntoElement {
        let groups = group_messages(&self.messages);
        let theme = &self.theme;

        let mut children: Vec<gpui::AnyElement> = Vec::new();

        children.push(Self::date_separator(theme).into_any_element());

        for (i, group) in groups.iter().enumerate() {
            children.push(
                Self::render_group(group, theme, &self.current_user_id, &self.current_username)
                    .into_any_element(),
            );

            if i == 2 {
                children.push(Self::unread_break(theme).into_any_element());
            }
        }

        if !self.typing_users.is_empty() {
            children.push(Self::typing_indicator(theme, &self.typing_users).into_any_element());
        }

        div()
            .id("messages-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .id("messages-wrap")
                    .flex()
                    .flex_col()
                    .min_h_full()
                    .mt_auto()
                    .justify_end()
                    .children(children),
            )
    }
}
