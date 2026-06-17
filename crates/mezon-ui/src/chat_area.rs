use std::rc::Rc;
use std::sync::Arc;

use crate::components::primitives::{InputEvent, InputState};
use gpui::{App, Context, Entity, ListAlignment, ListState, Window, div, prelude::*, px};
use mezon_store::Message;

use crate::chat::ReplyTarget;
use crate::chat::channel_header::ChannelHeader;
use crate::chat::input_bar::InputBar;
use crate::chat::message_list::MessageList;
use crate::theme::Theme;

pub struct ChatArea {
    pub(crate) messages: Rc<Vec<Message>>,
    pub(crate) list_state: ListState,
    pub(crate) input_state: Option<Entity<InputState>>,
    #[allow(dead_code)]
    replying_to: Option<ReplyTarget>,
}

impl Default for ChatArea {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatArea {
    pub fn new() -> Self {
        Self {
            messages: Rc::new(Vec::new()),
            list_state: ListState::new(0, ListAlignment::Bottom, px(200.)),
            input_state: None,
            replying_to: None,
        }
    }

    pub fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<crate::ChatLayout>) {
        if self.input_state.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Message #general"));
            cx.subscribe_in(
                &input,
                window,
                |this: &mut crate::ChatLayout, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter = event {
                        this.send_current_message(window, cx);
                    }
                },
            )
            .detach();
            self.input_state = Some(input);
        }
    }

    pub fn render(
        &self,
        theme: &Theme,
        layout_entity: Entity<crate::ChatLayout>,
        channel_name: &str,
        current_user_id: &str,
        _current_user_name: &str,
    ) -> impl IntoElement {
        let on_send = {
            let handle = layout_entity.clone();
            Arc::new(move |window: &mut Window, cx: &mut App| {
                handle.update(cx, |this, cx| this.send_current_message(window, cx));
            })
        };

        let input_bar = InputBar::new()
            .with_input(self.input_state.clone().unwrap())
            .on_send(on_send);

        let header = ChannelHeader::new(channel_name);

        let message_list = MessageList::new(
            self.list_state.clone(),
            self.messages.clone(),
            theme,
            current_user_id,
        );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header.render(theme))
            .child(message_list.render())
            .child(input_bar.render(theme))
    }
}
