use gpui::{
    AnyElement, Context, Entity, FollowMode, ListAlignment, ListState, SharedString, Window, div,
    list, prelude::*, px,
};

use mezon_store::{ChannelList, Message, MessagesEvent, MessagesStore, Settings};

use crate::chat::message_row::{MessageAttachmentView, MessageRow};
use crate::image_cache::{LruImageCache, MESSAGE_IMAGE_CACHE_CAPACITY};
use crate::theme::{ActiveTheme, Theme};

const LOAD_MORE_ITEM_THRESHOLD: usize = 6;

pub struct MessageTimeline {
    pub(crate) list_state: ListState,
    settings: Entity<Settings>,
    image_cache: Entity<LruImageCache>,
    cached_for_channel: Option<String>,
}

impl MessageTimeline {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let store = MessagesStore::global(cx);
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&store, |this, store, event, cx| match event {
            MessagesEvent::Reset { count } => {
                this.list_state.reset(*count);
                this.list_state.set_follow_mode(FollowMode::Tail);
            }
            MessagesEvent::OlderPrepended { count } => this.list_state.splice(0..0, *count),
            MessagesEvent::Appended => {
                let new_len = store.read(cx).messages().len();
                let old_len = this.list_state.item_count();
                if new_len >= old_len {
                    this.list_state.splice(old_len..old_len, new_len - old_len);
                } else {
                    this.list_state.reset(new_len);
                }
            }
        })
        .detach();

        let list_state = ListState::new(0, ListAlignment::Bottom, px(200.));
        list_state.set_follow_mode(FollowMode::Tail);
        list_state.set_scroll_handler(move |event, _window, cx| {
            if event.visible_range.start < LOAD_MORE_ITEM_THRESHOLD {
                MessagesStore::global(cx).update(cx, |store, cx| store.load_more(cx));
            }
        });
        let image_cache = cx.new(|cx| LruImageCache::new(MESSAGE_IMAGE_CACHE_CAPACITY, cx));
        Self {
            list_state,
            settings,
            image_cache,
            cached_for_channel: None,
        }
    }

    fn clear_image_cache_if_channel_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channel_id = ChannelList::global(cx).read(cx).active_channel_id.clone();
        if self.cached_for_channel.as_ref() == channel_id.as_ref() {
            return;
        }
        self.cached_for_channel = channel_id;
        self.image_cache
            .update(cx, |cache, cx| cache.clear(window, cx));
    }
}

impl Render for MessageTimeline {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::trace_render!("MessageTimeline");
        self.clear_image_cache_if_channel_changed(window, cx);

        let store = MessagesStore::global(cx);
        let count = store.read(cx).messages().len();
        if self.list_state.item_count() != count {
            self.list_state.reset(count);
            self.list_state.set_follow_mode(FollowMode::Tail);
        }

        let locale = self.settings.read(cx).language.clone();
        let reply_label: SharedString = mezon_i18n::t(&locale, "chat.replyingToSomeone").into();
        let list_state = self.list_state.clone();
        let list_element = list(list_state, move |ix, _window, cx| {
            render_row(store.read(cx).messages(), ix, cx, "", &reply_label)
        })
        .size_full();

        div()
            .size_full()
            .image_cache(self.image_cache.clone())
            .child(list_element)
    }
}

fn render_row(
    messages: &[Message],
    ix: usize,
    cx: &gpui::App,
    current_user_id: &str,
    reply_label: &SharedString,
) -> AnyElement {
    let theme = cx.theme();
    let Some(msg) = messages.get(ix) else {
        return div().into_any_element();
    };
    let prev = ix.checked_sub(1).and_then(|p| messages.get(p));

    let day_label = msg.day_label.as_str();
    let show_separator = prev.map(|p| p.day_label.as_str()) != Some(day_label);
    let combined = !show_separator && msg.combined_with_prev;

    let attachment_views = attachment_views(msg);
    let message_row = MessageRow::new(msg, theme, current_user_id, reply_label.clone())
        .combined(combined)
        .avatar_src(msg.avatar_proxied.clone())
        .attachments(attachment_views);

    let mut column = div().flex().flex_col().w_full();
    if show_separator {
        column = column.child(date_separator(theme, day_label));
    }
    column.child(message_row.render()).into_any_element()
}

fn attachment_views(msg: &Message) -> Vec<MessageAttachmentView> {
    msg.attachments
        .iter()
        .map(|att| {
            if att.is_image() {
                let label = if att.filename.is_empty() {
                    "image".to_string()
                } else {
                    att.filename.clone()
                };
                MessageAttachmentView::Image {
                    src: att.proxied_src.clone(),
                    width: px(att.display_width),
                    height: px(att.display_height),
                    label,
                }
            } else {
                let label = if att.filename.is_empty() {
                    "Attachment".to_string()
                } else {
                    att.filename.clone()
                };
                MessageAttachmentView::File { label }
            }
        })
        .collect()
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
        .child(div().flex_1().h(px(1.)).bg(theme.border))
        .child(
            div()
                .text_xs()
                .text_color(theme.text_muted)
                .child(label.to_string()),
        )
        .child(div().flex_1().h(px(1.)).bg(theme.border))
}
