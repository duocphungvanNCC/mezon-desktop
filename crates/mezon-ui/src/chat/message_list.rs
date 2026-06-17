use std::rc::Rc;

use chrono::DateTime;
use gpui::{AnyElement, ListState, div, list, prelude::*, px};

use mezon_store::Message;

use crate::chat::grouping::is_combined;
use crate::chat::message_row::MessageRow;
use crate::theme::Theme;

pub struct MessageList {
    list_state: ListState,
    messages: Rc<Vec<Message>>,
    theme: Theme,
    current_user_id: String,
}

impl MessageList {
    pub fn new(
        list_state: ListState,
        messages: Rc<Vec<Message>>,
        theme: &Theme,
        current_user_id: &str,
    ) -> Self {
        Self {
            list_state,
            messages,
            theme: theme.clone(),
            current_user_id: current_user_id.to_string(),
        }
    }

    pub fn render(self) -> impl IntoElement {
        if self.list_state.item_count() != self.messages.len() {
            self.list_state.reset(self.messages.len());
        }

        let messages = self.messages;
        let theme = self.theme;
        let current_user_id = self.current_user_id;

        let row_messages = messages.clone();
        let row_theme = theme.clone();
        let list_element = list(self.list_state.clone(), move |ix, _window, _cx| {
            render_row(&row_messages, ix, &row_theme, &current_user_id)
        })
        .size_full();

        div().flex_1().min_h_0().child(list_element)
    }
}

fn render_row(messages: &[Message], ix: usize, theme: &Theme, current_user_id: &str) -> AnyElement {
    let Some(msg) = messages.get(ix) else {
        return div().into_any_element();
    };
    let prev = ix.checked_sub(1).and_then(|p| messages.get(p));

    let day_label = format_date(msg.create_time);
    let show_separator = prev.map(|p| format_date(p.create_time)).as_deref() != Some(&day_label);
    let combined = !show_separator && is_combined(prev, msg);

    let message_row = MessageRow::new(msg.clone(), theme, current_user_id).combined(combined);

    let mut column = div().flex().flex_col().w_full();
    if show_separator {
        column = column.child(date_separator(theme, &day_label));
    }
    column.child(message_row.render()).into_any_element()
}

fn format_date(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%B %d, %Y").to_string())
        .unwrap_or_default()
}

fn date_separator(theme: &Theme, label: &str) -> impl IntoElement {
    div()
        .id("date-separator")
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
                .text_color(theme.text_muted)
                .child(label.to_string()),
        )
        .child(div().flex_1().h(px(1.)).bg(gpui::hsla(0., 0., 0., 0.08)))
}
