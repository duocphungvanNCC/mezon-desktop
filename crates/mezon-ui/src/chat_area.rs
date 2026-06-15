use std::sync::Arc;

use gpui::{App, Context, Entity, Window, div, prelude::*};
use gpui_component::input::InputState;
use mezon_store::Message;

use crate::chat::ReplyTarget;
use crate::chat::channel_header::ChannelHeader;
use crate::chat::input_bar::InputBar;
use crate::chat::message_list::MessageList;
use crate::theme::Theme;

pub struct ChatArea {
    pub(crate) messages: Vec<Message>,
    input_state: Option<Entity<InputState>>,
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
            messages: Vec::new(),
            input_state: None,
            replying_to: None,
        }
    }

    pub fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<crate::ChatLayout>) {
        if self.input_state.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Message #general"));
            self.input_state = Some(input);
        }
    }

    pub fn render(
        &self,
        theme: &Theme,
        layout_entity: Entity<crate::ChatLayout>,
        channel_name: &str,
        current_user_id: &str,
        current_user_name: &str,
    ) -> impl IntoElement {
        let handle = layout_entity.clone();
        let user_id = current_user_id.to_string();
        let user_name = current_user_name.to_string();
        #[allow(clippy::type_complexity)]
        let on_send: Arc<dyn Fn(&str, &mut Window, &mut App) + Send + Sync> = Arc::new(
            move |value: &str, _window: &mut Window, cx: &mut App| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let uid = user_id.clone();
                let uname = user_name.clone();
                let content = value.to_string();
                handle.update(cx, |this, cx| {
                    // Need the active channel (clan_id + channel_id) to address the send.
                    let Some(channel) = this.channel_list.read(cx).active_channel().cloned() else {
                        tracing::warn!("send: no active channel");
                        return;
                    };

                    tracing::info!(
                        "send optimistic echo: sender_id={uid} sender_name={uname} content={content}"
                    );
                    // Optimistic echo so the sender sees their message immediately.
                    this.chat_area.messages.push(Message::new(
                        format!("temp-{now}"),
                        content.clone(),
                        uid,
                        uname,
                        now,
                    ));
                    cx.notify();

                    // Actually send it over the socket (was a no-op mock before).
                    let api = this.api.clone();
                    let clan_id = channel.clan_id.clone();
                    let channel_id = channel.id.clone();
                    let is_public = !channel.private;
                    let temp_id = format!("temp-{now}");
                    cx.spawn(async move |this, cx| {
                        match api
                            .send_channel_message(&clan_id, &channel_id, &content, is_public)
                            .await
                        {
                            Ok(sent) => {
                                let _ = this.update(cx, |this, cx| {
                                    if let Some(slot) = this
                                        .chat_area
                                        .messages
                                        .iter_mut()
                                        .find(|m| m.id == temp_id)
                                    {
                                        slot.id = sent.message_id;
                                        cx.notify();
                                    }
                                });
                            }
                            Err(e) => tracing::error!("send_channel_message failed: {e}"),
                        }
                    })
                    .detach();
                });
            },
        );

        let input_bar = InputBar::new()
            .with_input(self.input_state.clone().unwrap())
            .on_send(on_send);

        let header = ChannelHeader::new(channel_name);
        let message_list = MessageList::new(self.messages.clone(), theme, current_user_id);

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
