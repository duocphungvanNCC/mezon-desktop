use gpui::{
    Context, Entity, FontWeight, Render, SharedString, Subscription, Window, div, prelude::*, px,
};
use mezon_store::{ChannelId, PresenceEvent, PresenceStore, Settings};

use crate::components::primitives::{Icon, IconName};
use crate::theme::ActiveTheme;

pub struct ChannelTyping {
    channel_id: Option<ChannelId>,
    settings: Entity<Settings>,
    _presence_sub: Subscription,
    _settings_sub: Subscription,
}

enum TypingContent {
    One {
        name: SharedString,
        suffix: SharedString,
    },
    Several(SharedString),
}

impl ChannelTyping {
    pub fn new(settings: &Entity<Settings>, cx: &mut Context<Self>) -> Self {
        let _presence_sub = cx.subscribe(&PresenceStore::global(cx), |this, _, event, cx| {
            if let PresenceEvent::TypingChanged { channel_id } = event
                && this.channel_id == Some(*channel_id)
            {
                cx.notify();
            }
        });
        let _settings_sub = cx.observe(settings, |_, _, cx| cx.notify());
        Self {
            channel_id: None,
            settings: settings.clone(),
            _presence_sub,
            _settings_sub,
        }
    }

    pub fn sync(&mut self, channel_id: Option<ChannelId>, cx: &mut Context<Self>) {
        if self.channel_id == channel_id {
            return;
        }
        self.channel_id = channel_id;
        cx.notify();
    }

    fn content(&self, cx: &Context<Self>) -> Option<TypingContent> {
        let channel_id = self.channel_id?;
        let presence = PresenceStore::global(cx);
        let presence = presence.read(cx);
        let users = presence.typing_users(channel_id);
        match users.len() {
            0 => None,
            1 => {
                let locale = &self.settings.read(cx).language;
                Some(TypingContent::One {
                    suffix: SharedString::from(
                        mezon_i18n::t(locale, "common.isTyping")
                            .trim()
                            .trim_end_matches(['.', '…']),
                    ),
                    name: users.into_iter().next()?,
                })
            }
            _ => {
                let locale = &self.settings.read(cx).language;
                Some(TypingContent::Several(SharedString::from(mezon_i18n::t(
                    locale,
                    "common.severalPeopleTyping",
                ))))
            }
        }
    }
}

impl Render for ChannelTyping {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let primary = theme.tokens.text_theme_primary;
        let name_color = theme.tokens.text_theme_primary_hover;
        let bar = div()
            .pl_3()
            .pr_1()
            .h_full()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap_1p5()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_xs()
            .line_height(px(16.))
            .text_color(primary);
        match self.content(cx) {
            Some(TypingContent::One { name, suffix }) => bar
                .child(
                    Icon::new(IconName::TypingIndicator)
                        .w(px(20.))
                        .h(px(10.))
                        .flex_none()
                        .text_color(primary),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .min_w_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .min_w_0()
                                .mr(px(2.))
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(name_color)
                                .child(name),
                        )
                        .child(div().flex_none().text_color(primary).child(suffix)),
                ),
            Some(TypingContent::Several(text)) => bar.child(text),
            None => bar,
        }
    }
}
