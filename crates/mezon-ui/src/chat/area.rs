use std::sync::Arc;

use gpui::{AnyView, App, Context, Entity, StyleRefinement, Window, div, prelude::*, px};
// composer: use gpui::{AnyView, App, Context, Entity, StyleRefinement, Subscription, Window, div, prelude::*, px};
use mezon_store::Settings;
// composer: use mezon_store::{MessagesStore, Settings};

use crate::chat::ReplyTarget;
use crate::chat::channel_header::ChannelHeader;
use crate::chat::input_bar::InputBar;
use crate::chat::member_list::{MemberListPanel, MemberSource};
// composer: use crate::chat::mention_input::{MentionInput, MentionInputEvent};
use crate::chat::message::ChannelMessages;
use crate::components::primitives::{InputEvent, InputState};
use crate::theme::Theme;

pub struct ChatArea {
    pub(crate) timeline: Entity<ChannelMessages>,
    pub(crate) input_state: Option<Entity<InputState>>,
    // composer: pub(crate) mention_input: Option<Entity<MentionInput>>,
    member_panel: Option<Entity<MemberListPanel>>,
    member_source: Option<MemberSource>,
    #[allow(dead_code)]
    replying_to: Option<ReplyTarget>,
    settings: Entity<Settings>,
    // composer: _submit_sub: Option<Subscription>,
}

impl ChatArea {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<crate::ChatLayout>) -> Self {
        let timeline = cx.new({
            let settings = settings.clone();
            move |cx| ChannelMessages::new(settings, cx)
        });
        Self {
            timeline,
            input_state: None,
            // composer: mention_input: None,
            member_panel: None,
            member_source: None,
            replying_to: None,
            settings,
            // composer: _submit_sub: None,
        }
    }

    pub fn bind_channel_members(&mut self, cx: &mut Context<crate::ChatLayout>) {
        self.set_member_source(Some(MemberSource::Channel), cx);
    }

    pub fn bind_group_members(&mut self, cx: &mut Context<crate::ChatLayout>) {
        self.set_member_source(Some(MemberSource::Group), cx);
    }

    pub fn clear_member_panel(&mut self) {
        self.member_source = None;
        self.member_panel = None;
    }

    fn set_member_source(
        &mut self,
        source: Option<MemberSource>,
        cx: &mut Context<crate::ChatLayout>,
    ) {
        if self.member_source == source {
            return;
        }
        self.member_source = source;
        self.member_panel = source.map(|source| {
            let settings = self.settings.clone();
            cx.new(move |cx| MemberListPanel::new(source, settings, cx))
        });
    }

    pub fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<crate::ChatLayout>) {
        if self.input_state.is_none() {
            let locale = self.settings.read(cx).language.clone();
            let placeholder = mezon_i18n::t(&locale, "chat.messagePlaceholder");
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
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

    // composer: restore the MentionInput composer by swapping ensure_input for:
    // pub fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<crate::ChatLayout>) {
    //     if self.mention_input.is_none() {
    //         let locale = self.settings.read(cx).language.clone();
    //         let placeholder = mezon_i18n::t(&locale, "chat.messagePlaceholder");
    //         let settings = self.settings.clone();
    //         let mention_input = cx.new(|cx| MentionInput::new(placeholder, settings, window, cx));
    //         let submit_sub = cx.subscribe_in(
    //             &mention_input,
    //             window,
    //             |this: &mut crate::ChatLayout, _, event: &MentionInputEvent, window, cx| match event {
    //                 MentionInputEvent::Submit => this.send_current_message(window, cx),
    //                 MentionInputEvent::SendSticker { url, filename } => {
    //                     this.send_sticker(url.clone(), filename.clone(), cx)
    //                 }
    //             },
    //         );
    //         self._submit_sub = Some(submit_sub);
    //         self.mention_input = Some(mention_input);
    //     }
    // }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        theme: &Theme,
        locale: &str,
        layout_entity: Entity<crate::ChatLayout>,
        channel_name: &str,
        is_dm: bool,
        typing_label: Option<gpui::SharedString>,
        show_members_button: bool,
        show_member_panel: bool,
        // composer: reply_preview: Option<ReplyTarget>,
    ) -> gpui::AnyElement {
        let input_state = match self.input_state.clone() {
            // composer: let mention_input = match self.mention_input.clone() {
            Some(s) => s,
            None => {
                return div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .into_any_element();
            }
        };

        let on_send = {
            let handle = layout_entity.clone();
            Arc::new(move |window: &mut Window, cx: &mut App| {
                handle.update(cx, |this, cx| this.send_current_message(window, cx));
            })
        };

        // composer: let on_cancel_reply = Arc::new(move |_window: &mut Window, cx: &mut App| {
        // composer:     MessagesStore::global(cx).update(cx, |store, cx| store.clear_reply(cx));
        // composer: });

        let input_bar = InputBar::new()
            .with_input(input_state)
            .on_send(on_send)
            .typing_label(typing_label);
        // composer: let input_bar = InputBar::new()
        // composer:     .with_mention_input(mention_input)
        // composer:     .on_send(on_send)
        // composer:     .on_cancel_reply(on_cancel_reply)
        // composer:     .replying_to(reply_preview)
        // composer:     .typing_label(typing_label);

        let header = ChannelHeader::new(channel_name)
            .dm(is_dm)
            .members_action(show_members_button)
            .members_active(show_member_panel)
            .on_toggle_members({
                let handle = layout_entity.clone();
                Arc::new(move |_window: &mut Window, cx: &mut App| {
                    handle.update(cx, |this, cx| this.toggle_member_list(cx));
                })
            });

        let message_column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(div().flex_1().min_h_0().child(
                // Cache the message list so an unrelated sibling/parent notify
                // (presence in the member panel, typing indicator, user info
                // bar, theme change…) does not force a full re-layout of every
                // row. It self-invalidates whenever the timeline itself
                // notifies (scroll, new message, GIF animation), so behaviour
                // is unchanged — only redundant re-renders are skipped.
                AnyView::from(self.timeline.clone()).cached(StyleRefinement::default().size_full()),
            ))
            .child(input_bar.render(theme, locale));

        let body = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .child(message_column)
            .when(show_member_panel, |row| match &self.member_panel {
                // Cache the member panel so it is not re-rendered (and its avatars
                // re-painted) every frame the message timeline notifies during
                // scroll/load-more. GPUI marks the whole ancestor chain of a
                // notified view dirty, so the timeline's churn forces chat_layout
                // to re-render its subtree; caching keeps the panel reused unless
                // the panel itself is notified (member/presence change or scroll).
                Some(panel) => row.child(
                    AnyView::from(panel.clone()).cached(
                        StyleRefinement::default()
                            .w(px(245.))
                            .h_full()
                            .flex_shrink_0(),
                    ),
                ),
                None => row,
            });

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header.render(theme))
            .child(body)
            .into_any_element()
    }
}
