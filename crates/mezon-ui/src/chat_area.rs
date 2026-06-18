use std::sync::Arc;

use crate::components::primitives::{InputEvent, InputState};
use gpui::{App, Context, Entity, Window, div, prelude::*};
use mezon_store::Settings;

use crate::chat::ReplyTarget;
use crate::chat::channel_header::ChannelHeader;
use crate::chat::input_bar::InputBar;
use crate::chat::message_list::MessageTimeline;
use crate::theme::Theme;

pub struct ChatArea {
    pub(crate) timeline: Entity<MessageTimeline>,
    pub(crate) input_state: Option<Entity<InputState>>,
    #[allow(dead_code)]
    replying_to: Option<ReplyTarget>,
}

impl ChatArea {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<crate::ChatLayout>) -> Self {
        let timeline = cx.new(|cx| MessageTimeline::new(settings, cx));
        Self {
            timeline,
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
        _current_user_id: &str,
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

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header.render(theme))
            .child(self.timeline.clone())
            .child(input_bar.render(theme))
    }
}
