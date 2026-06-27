use gpui::{AnyElement, SharedString, div, prelude::*, px};
use mezon_store::Message;

use super::content::render_message_content;
use super::context::{
    AVATAR_LEFT, AVATAR_SIZE, CONTENT_INSET, CONTENT_RIGHT_PAD, DEFAULT_DISPLAY_NAME_COLOR, RowCtx,
};
use super::parts::{
    avatar_element, render_attachments, render_head, render_hover_actions, render_reactions,
    render_reply,
};
use crate::components::primitives::{Icon, IconName};

/// Render a normal user message row (React `MessageWithUser`), including reply
/// quote, avatar/head (unless grouped), rich-text body, attachments and
/// reactions. `combined` collapses the avatar/head for consecutive messages.
pub fn render_user_message(msg: &Message, combined: bool, ctx: &RowCtx) -> AnyElement {
    let theme = ctx.theme;
    let has_reply = !msg.references.is_empty();
    let show_head = !combined;
    let group_name = SharedString::from(format!("msg-{}", msg.id));

    let mut body_column = div()
        .flex()
        .flex_col()
        .w_full()
        .pl(px(CONTENT_INSET))
        .pr(px(CONTENT_RIGHT_PAD));

    if msg.is_forwarded {
        body_column = body_column.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .mb_0p5()
                .text_xs()
                .text_color(theme.text_muted)
                .child(
                    Icon::new(IconName::ReplyCorner)
                        .size_4()
                        .text_color(theme.text_muted),
                )
                .child(mezon_i18n::t(ctx.locale, "chat.forwarded")),
        );
    }

    if show_head {
        body_column = body_column.child(render_head(msg, theme, DEFAULT_DISPLAY_NAME_COLOR));
    }

    body_column = body_column.child(render_message_content(msg, ctx));

    if let Some(attachments) = render_attachments(msg, theme) {
        body_column = body_column.child(attachments);
    }
    if let Some(reactions) = render_reactions(msg, ctx) {
        body_column = body_column.child(reactions);
    }

    let body = div()
        .relative()
        .w_full()
        .when(show_head, |d| {
            d.child(
                div()
                    .absolute()
                    .left(px(AVATAR_LEFT))
                    .top(px(2.))
                    .w(px(AVATAR_SIZE))
                    .h(px(AVATAR_SIZE))
                    .id(SharedString::from(format!("msg-avatar-{}", msg.sender_id)))
                    .child(avatar_element(msg, ctx)),
            )
        })
        .child(body_column);

    let hover_bg = theme.bg_hover;
    let highlighted = ctx
        .highlight_id
        .as_ref()
        .is_some_and(|id| id.as_ref() == msg.id);
    div()
        .id(SharedString::from(format!("msg-row-{}", msg.id)))
        .group(group_name.clone())
        .relative()
        .w_full()
        .py(px(2.))
        .when(!combined, |d| d.mt(px(10.)).pt_3())
        .when(highlighted, |d| {
            d.bg(gpui::Rgba {
                a: 0.16,
                ..theme.brand
            })
        })
        .when(!ctx.suppress_hover, |d| d.hover(move |s| s.bg(hover_bg)))
        .when(has_reply, |d| {
            d.child(render_reply(&msg.references[0], ctx))
        })
        .child(body)
        .child(render_hover_actions(msg, theme, ctx.suppress_hover))
        .into_any_element()
}
