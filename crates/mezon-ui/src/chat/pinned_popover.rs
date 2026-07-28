use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, HighlightStyle, Hsla, ListAlignment, ListState, MouseDownEvent, ObjectFit,
    SharedString, StyledText, Window, div, img, list, prelude::*, px, relative, rems,
};
use mezon_store::{
    AccountStore, ChannelId, ClanMembersStore, DirectMessageStore, Embed, MessageAttachment,
    MessageId, MessageSpan, MessagesStore, PinnedMessage, PinnedMessagesStore, RichLayout,
    RichRunKind, Settings, UsersByUserStore,
};
use ui::{PopoverMenuHandle, ScrollAxes, Scrollbars, WithScrollbar};

use crate::chat::file_type_icon::file_type_icon_for;
use crate::chat::message::parts::{
    effective_clan_id, resolve_pin_avatar_url, resolve_pin_sender_label_with_message,
};
use crate::chat::message::{
    ConfirmUnpinMessageModal, DEFAULT_DISPLAY_NAME_COLOR, render_ogp_preview,
};
use crate::components::primitives::{
    Avatar, Button, ButtonVariants, Icon, IconName, Sizable, Size, Spinner, h_flex, v_flex,
};
use crate::image_cache::LruImageCache;
use crate::theme::{ActiveTheme, Theme};
use crate::util::download::save_with_progress_toast;

const POPOVER_WIDTH: f32 = 420.;
const HEADER_HEIGHT: f32 = 48.;
const PANEL_MIN_HEIGHT: f32 = 200.;
const LIST_BODY_HEIGHT: f32 = 500.;
const PANEL_MAX_VIEWPORT_OFFSET: f32 = 180.;
const LIST_OVERDRAW: f32 = 200.;
const LIST_PAD_X: f32 = 16.;
const LIST_PAD_Y: f32 = 8.;
const EMPTY_BODY_HEIGHT: f32 = 144.;
const FILE_NAME_COLOR: u32 = 0x3b_82_f6;

#[derive(Clone)]
struct PinCardVm {
    pin_id: SharedString,
    message_id: SharedString,
    create_time: i64,
}

impl PinCardVm {
    fn resolve(msg: &PinnedMessage) -> Self {
        Self {
            pin_id: msg.id.clone().into(),
            message_id: msg.message_id.clone().into(),
            create_time: msg.create_time,
        }
    }
}

pub struct PinnedPopoverPanel {
    settings: Entity<Settings>,
    popover_handle: PopoverMenuHandle<PinnedPopoverPanel>,
    list_state: ListState,
    focus_handle: FocusHandle,
    avatar_image_cache: Entity<LruImageCache>,
    pin_cards: Vec<PinCardVm>,
    _subs: Vec<gpui::Subscription>,
}

impl PinnedPopoverPanel {
    pub fn new(
        settings: Entity<Settings>,
        popover_handle: PopoverMenuHandle<PinnedPopoverPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        cx.on_blur(&focus_handle, window, |_, _, cx| cx.emit(DismissEvent))
            .detach();

        let subs = vec![
            cx.observe(&PinnedMessagesStore::global(cx), |this, _, cx| {
                this.pin_cards = Self::compute_pin_cards(cx);
                cx.notify();
            }),
            cx.observe(&ClanMembersStore::global(cx), |this, _, cx| {
                this.refresh_name_rows(cx);
            }),
            cx.observe(&DirectMessageStore::global(cx), |this, _, cx| {
                this.refresh_name_rows(cx);
            }),
            cx.observe(&UsersByUserStore::global(cx), |this, _, cx| {
                this.refresh_name_rows(cx);
            }),
            cx.observe(&AccountStore::global(cx), |this, _, cx| {
                this.refresh_name_rows(cx);
            }),
            cx.observe(&settings, |_, _, cx| cx.notify()),
        ];

        let avatar_image_cache = crate::image_cache::shared_avatar_cache(cx);

        let pin_cards = Self::compute_pin_cards(cx);
        let list_state = ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)).measure_all();

        Self {
            settings,
            popover_handle,
            list_state,
            focus_handle,
            avatar_image_cache,
            pin_cards,
            _subs: subs,
        }
    }

    fn refresh_name_rows(&mut self, cx: &mut Context<Self>) {
        let count = self.list_state.item_count();
        if count > 0 {
            self.list_state.reset(count);
        }
        cx.notify();
    }

    fn compute_pin_cards(cx: &App) -> Vec<PinCardVm> {
        PinnedMessagesStore::global(cx)
            .read(cx)
            .pinned()
            .iter()
            .map(PinCardVm::resolve)
            .collect()
    }
}

impl Focusable for PinnedPopoverPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for PinnedPopoverPanel {}

impl Render for PinnedPopoverPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let store = PinnedMessagesStore::global(cx);
        let cards = Rc::new(self.pin_cards.clone());
        let loading = store.read(cx).is_loading();
        let clan_id = store.read(cx).clan_id();
        let handle = self.popover_handle.clone();
        let avatar_cache = self.avatar_image_cache.clone();
        let tokens = &theme.tokens;

        if let Some(clan_id) = clan_id {
            ClanMembersStore::global(cx).update(cx, |members, cx| {
                members.ensure_loaded(clan_id, cx);
            });
        }

        let current = self.list_state.item_count();
        if cards.len() > current {
            self.list_state.splice(0..0, cards.len() - current);
        } else if cards.len() < current {
            self.list_state.reset(cards.len());
        }
        let list_state = self.list_state.clone();
        let viewport_h = f32::from(window.viewport_size().height);
        let panel_max_h = (viewport_h - PANEL_MAX_VIEWPORT_OFFSET).max(PANEL_MIN_HEIGHT);

        v_flex()
            .key_context("menu")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &::menu::Cancel, _window, cx| {
                cx.emit(DismissEvent);
            }))
            .on_mouse_down_out(cx.listener(|_, _: &MouseDownEvent, _window, cx| {
                cx.emit(DismissEvent);
            }))
            .w(px(POPOVER_WIDTH))
            .min_h(px(PANEL_MIN_HEIGHT.min(HEADER_HEIGHT + EMPTY_BODY_HEIGHT)))
            .max_h(px(panel_max_h))
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(tokens.border_primary)
            .bg(tokens.theme_setting_primary)
            .text_color(tokens.text_theme_message)
            .child(render_header(&theme, &locale))
            .child(render_body(
                cards,
                loading,
                theme.clone(),
                locale,
                handle,
                list_state,
                avatar_cache,
                panel_max_h,
                window,
                cx,
            ))
    }
}

fn render_header(theme: &Theme, locale: &str) -> impl IntoElement {
    let tokens = &theme.tokens;
    h_flex()
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap_3()
        .px(px(16.))
        .h(px(HEADER_HEIGHT))
        .border_b_1()
        .border_color(tokens.border_primary)
        .bg(tokens.theme_setting_nav)
        .child(
            Icon::new(IconName::PinRight)
                .size_4()
                .text_color(tokens.bg_icon_theme),
        )
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tokens.text_theme_message)
                .child(mezon_i18n::t(
                    locale,
                    "channelTopbar.modals.pinnedMessages.title",
                )),
        )
}

#[allow(clippy::too_many_arguments)]
fn render_body(
    cards: Rc<Vec<PinCardVm>>,
    loading: bool,
    theme: Arc<Theme>,
    locale: String,
    popover_handle: PopoverMenuHandle<PinnedPopoverPanel>,
    list_state: ListState,
    avatar_cache: Entity<LruImageCache>,
    panel_max_h: f32,
    window: &mut Window,
    cx: &mut Context<PinnedPopoverPanel>,
) -> impl IntoElement {
    let tokens = &theme.tokens;
    let max_body = (panel_max_h - HEADER_HEIGHT).max(EMPTY_BODY_HEIGHT);
    let body_h = if cards.is_empty() {
        EMPTY_BODY_HEIGHT.min(max_body)
    } else {
        LIST_BODY_HEIGHT.min(max_body)
    };

    let body: gpui::AnyElement = if cards.is_empty() {
        let inner = if loading {
            Spinner::new().with_size(Size::Small).into_any_element()
        } else {
            div()
                .text_sm()
                .text_color(tokens.text_secondary)
                .child(mezon_i18n::t(
                    &locale,
                    "channelTopbar.pinnedMessages.emptyTitle",
                ))
                .into_any_element()
        };
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .child(inner)
            .into_any_element()
    } else {
        let cards_for_list = cards.clone();
        let theme_for_list = theme.clone();
        let locale_for_list = locale.clone();
        let handle_for_list = popover_handle.clone();
        let avatar_for_list = avatar_cache.clone();
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .pl(px(LIST_PAD_X))
            .pr(px(LIST_PAD_X))
            .py(px(LIST_PAD_Y))
            .child(
                list(list_state.clone(), move |ix, _window, cx| {
                    let store = PinnedMessagesStore::global(cx).read(cx);
                    let Some(pin) = store.pinned().get(ix) else {
                        return div().into_any_element();
                    };
                    let Some(vm) = cards_for_list.get(ix) else {
                        return div().into_any_element();
                    };
                    let clan_id = effective_clan_id(store.clan_id(), cx);
                    let channel_id = store.channel_id();
                    div()
                        .w_full()
                        .pb(px(8.))
                        .child(pin_card(
                            ix,
                            pin,
                            vm,
                            clan_id,
                            channel_id,
                            &theme_for_list,
                            &locale_for_list,
                            handle_for_list.clone(),
                            avatar_for_list.clone(),
                            cx,
                        ))
                        .into_any_element()
                })
                .size_full(),
            )
            .custom_scrollbars(
                Scrollbars::always_visible(ScrollAxes::Vertical).tracked_scroll_handle(&list_state),
                window,
                cx,
            )
            .into_any_element()
    };

    div()
        .w_full()
        .h(px(body_h))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(tokens.theme_setting_primary)
        .child(body)
}

fn resolve_pin_avatar_urls(
    pin: &PinnedMessage,
    clan_id: Option<mezon_store::ClanId>,
    channel_id: Option<ChannelId>,
    cx: &App,
) -> (Option<SharedString>, Option<SharedString>) {
    let mut avatar_raw =
        resolve_pin_avatar_url(&pin.sender_id, &pin.avatar_url, clan_id, channel_id, cx);
    if avatar_raw.is_empty()
        && let Ok(message_id) = pin.message_id.parse::<MessageId>()
    {
        let store = MessagesStore::global(cx).read(cx);
        if let Some(message) = store
            .messages()
            .iter()
            .find(|m| m.id == message_id)
            .or_else(|| {
                channel_id.and_then(|channel_id| store.message_in_channel(channel_id, message_id))
            })
            && !message.avatar_url.is_empty()
        {
            avatar_raw = message.avatar_url.to_string();
        }
    }

    if !avatar_raw.is_empty() {
        let proxied = crate::util::imgproxy::avatar_url(cx, &avatar_raw);
        (Some(proxied.into()), Some(avatar_raw.into()))
    } else if !pin.avatar_proxied.is_empty() {
        let fallback = (!pin.avatar_url.is_empty()
            && pin.avatar_url != pin.avatar_proxied.as_ref())
        .then(|| SharedString::from(pin.avatar_url.clone()));
        (Some(pin.avatar_proxied.clone()), fallback)
    } else {
        (None, None)
    }
}

fn pin_card(
    index: usize,
    pin: &PinnedMessage,
    vm: &PinCardVm,
    clan_id: Option<mezon_store::ClanId>,
    channel_id: Option<ChannelId>,
    theme: &Theme,
    locale: &str,
    popover_handle: PopoverMenuHandle<PinnedPopoverPanel>,
    avatar_cache: Entity<LruImageCache>,
    cx: &App,
) -> gpui::AnyElement {
    let tokens = &theme.tokens;
    let group_name = SharedString::from(format!("pin-card-{index}"));
    let sender_color = Hsla::from(gpui::rgb(DEFAULT_DISPLAY_NAME_COLOR));
    let sender_label = resolve_pin_sender_label_with_message(
        &pin.sender_id,
        &pin.sender_name,
        Some(pin.message_id.as_str()),
        clan_id,
        channel_id,
        cx,
    );
    let (avatar_src, avatar_fallback) = resolve_pin_avatar_urls(pin, clan_id, channel_id, cx);

    let mut avatar = Avatar::new()
        .name(&sender_label)
        .with_size(Size::Small)
        .image_cache(avatar_cache.clone());
    if let Some(src) = &avatar_src {
        avatar = avatar.src(src.clone());
    }
    if let Some(fallback) = &avatar_fallback {
        avatar = avatar.fallback_src(fallback.clone());
    }

    let name_row = h_flex()
        .items_center()
        .gap_4()
        .min_w_0()
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(200.))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(sender_color)
                .overflow_hidden()
                .text_ellipsis()
                .child(sender_label.clone()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(tokens.text_secondary)
                .child(format_pin_time(vm.create_time, locale)),
        );

    let content = render_pin_body(pin, theme, avatar_cache, cx);

    let jump_message_id = vm.message_id.clone();
    let jump_handle = popover_handle.clone();
    let jump = Button::new(("pin-jump", index))
        .label(mezon_i18n::t(locale, "channelTopbar.tooltips.jump"))
        .ghost()
        .with_size(Size::XSmall)
        .on_click(move |_: &ClickEvent, _window, cx| {
            if let Ok(jump_target) = jump_message_id.parse::<MessageId>() {
                MessagesStore::global(cx).update(cx, |store, cx| {
                    store.jump_to_message(jump_target, cx);
                });
            }
            jump_handle.hide(cx);
        });

    let pin_id = vm.pin_id.clone();
    let message_id = vm.message_id.clone();
    let sender_label_for_modal = sender_label.clone();
    let avatar_src_for_modal = avatar_src.clone();
    let avatar_fallback_for_modal = avatar_fallback.clone();
    let preview_content = SharedString::from(pin.content.clone());
    let locale_owned: SharedString = locale.to_string().into();
    let delete = Button::new(("pin-del", index))
        .label("✕")
        .ghost()
        .with_size(Size::XSmall)
        .on_click(move |_: &ClickEvent, window, cx| {
            ConfirmUnpinMessageModal::open(
                pin_id.clone(),
                message_id.clone(),
                sender_label_for_modal.clone(),
                avatar_src_for_modal.clone(),
                avatar_fallback_for_modal.clone(),
                preview_content.clone(),
                locale_owned.clone(),
                window,
                cx,
            );
        });

    let actions = h_flex()
        .absolute()
        .top(px(8.))
        .right(px(8.))
        .items_center()
        .gap_1()
        .invisible()
        .group_hover(group_name.clone(), |s| s.visible())
        .child(jump)
        .child(delete);

    h_flex()
        .id(("pin-item", index))
        .group(group_name)
        .relative()
        .w_full()
        .items_start()
        .gap_2()
        .px(px(12.))
        .py(px(12.))
        .rounded(px(4.))
        .border_1()
        .border_color(tokens.border_primary)
        .bg(tokens.bg_active_member_channel)
        .child(avatar)
        .child(
            v_flex()
                .w(relative(0.85))
                .min_w_0()
                .gap_1()
                .child(name_row)
                .child(content),
        )
        .child(actions)
        .into_any_element()
}

fn render_pin_body(
    pin: &PinnedMessage,
    theme: &Theme,
    image_cache: Entity<LruImageCache>,
    cx: &App,
) -> gpui::AnyElement {
    let message_id = pin.message_id.parse::<MessageId>().unwrap_or(MessageId(0));
    let text_body = render_pin_text_body(pin, theme);
    let image_preview = pin
        .attachments
        .iter()
        .find(|att| att.is_image() && !att.proxied_src.is_empty())
        .map(|att| render_pin_image_attachment(att, image_cache.clone()));
    let file_preview = pin
        .attachments
        .iter()
        .find(|att| !att.is_image())
        .map(|att| render_pin_file_attachment(att, theme));
    let ogp = pin
        .ogp
        .as_ref()
        .and_then(|ogp| render_ogp_preview(ogp, message_id, theme, cx));
    let embeds = render_pin_embeds(&pin.embeds, theme, image_cache);

    v_flex()
        .w_full()
        .min_w_0()
        .max_w_full()
        .overflow_hidden()
        .gap_1()
        .child(text_body)
        .children(ogp)
        .children(image_preview)
        .children(file_preview)
        .children(embeds)
        .into_any_element()
}

fn render_pin_text_body(pin: &PinnedMessage, theme: &Theme) -> gpui::AnyElement {
    let expanded = expand_pin_spans(&pin.spans);
    let spans = if expanded.is_empty() {
        pin.spans.as_ref()
    } else {
        expanded.as_slice()
    };
    if !spans.is_empty() {
        return render_pin_spans(spans, theme);
    }
    if let Some(layout) = pin.rich_layout.as_ref()
        && !layout.text.is_empty()
        && !layout.text.contains("```")
    {
        return render_pin_rich_layout(layout, theme);
    }
    if pin.content.is_empty() {
        return div().into_any_element();
    }
    if pin.content.contains('`') || text_has_block_markup(&pin.content) {
        let fallback = expand_pin_spans(&[MessageSpan::Text(pin.content.clone().into())]);
        if !fallback.is_empty() {
            return render_pin_spans(&fallback, theme);
        }
    }
    div()
        .w_full()
        .min_w_0()
        .max_w_full()
        .overflow_hidden()
        .child(pin_plain_line(
            &pin.content,
            theme.tokens.text_theme_message,
        ))
        .into_any_element()
}

fn text_has_block_markup(text: &str) -> bool {
    text.contains("```")
        || text
            .split('\n')
            .any(|line| parse_pin_heading_line(line).is_some())
}

fn expand_pin_spans(spans: &[MessageSpan]) -> Vec<MessageSpan> {
    let mut out = Vec::with_capacity(spans.len());
    let mut changed = false;
    for span in spans {
        match span {
            MessageSpan::Text(text) if text.contains('`') || text_has_block_markup(text) => {
                let before = out.len();
                split_pin_plain_text(text, &mut out);
                if out.len() != before + 1
                    || !matches!(out.last(), Some(MessageSpan::Text(t)) if t.as_ref() == text.as_ref())
                {
                    changed = true;
                }
            }
            MessageSpan::CodeBlock { language, text } => {
                let cleaned = normalize_pin_code_block(text);
                if cleaned.as_ref() != text.as_ref() || language.is_some() {
                    changed = true;
                }
                out.push(MessageSpan::CodeBlock {
                    language: language.clone(),
                    text: cleaned,
                });
            }
            other => out.push(other.clone()),
        }
    }
    if changed { out } else { Vec::new() }
}

fn split_pin_plain_text(text: &str, out: &mut Vec<MessageSpan>) {
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        if start > 0 {
            split_pin_text_lines(&rest[..start], out);
        }
        let after_open = &rest[start + 3..];
        if let Some(end) = after_open.find("```") {
            let fence_body = &after_open[..end];
            let cleaned = normalize_pin_code_block(fence_body);
            out.push(MessageSpan::CodeBlock {
                language: None,
                text: cleaned,
            });
            rest = &after_open[end + 3..];
        } else {
            split_pin_text_lines(&rest[start..], out);
            return;
        }
    }
    if !rest.is_empty() {
        split_pin_text_lines(rest, out);
    }
}

fn split_pin_text_lines(text: &str, out: &mut Vec<MessageSpan>) {
    if text.is_empty() {
        return;
    }
    let mut buf = String::new();
    for line in text.split('\n') {
        if let Some((level, body)) = parse_pin_heading_line(line) {
            flush_pin_text_buf(&mut buf, out);
            out.push(MessageSpan::Heading {
                level,
                text: body.to_string().into(),
            });
        } else {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
        }
    }
    flush_pin_text_buf(&mut buf, out);
}

fn flush_pin_text_buf(buf: &mut String, out: &mut Vec<MessageSpan>) {
    if buf.is_empty() {
        return;
    }
    out.push(MessageSpan::Text(std::mem::take(buf).into()));
}

fn normalize_pin_code_block(raw: &str) -> SharedString {
    let mut body = raw.trim();
    if let Some(rest) = body.strip_prefix("```") {
        body = rest;
        if let Some(rest) = body.strip_suffix("```") {
            body = rest;
        }
    }
    body = body.trim_matches('`');
    body = body.strip_prefix('\n').unwrap_or(body);
    body = body.strip_suffix('\n').unwrap_or(body);
    let mut lines = body.splitn(2, '\n');
    let first = lines.next().unwrap_or("");
    let rest = lines.next();
    if let Some(rest) = rest
        && !first.is_empty()
        && first.len() <= 32
        && first
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'_' || b == b'#')
    {
        return rest.to_string().into();
    }
    body.to_string().into()
}

fn parse_pin_heading_line(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    let body = rest.trim_start_matches([' ', '\t']);
    if body.len() == rest.len() || body.is_empty() {
        return None;
    }
    Some((hashes as u8, body))
}

fn soft_break_hints(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for ch in text.chars() {
        out.push(ch);
        if matches!(ch, '/' | '?' | '&' | '=' | '_' | '-') {
            out.push('\u{200B}');
        }
    }
    out
}

fn render_pin_rich_layout(layout: &RichLayout, theme: &Theme) -> gpui::AnyElement {
    let mention_color: Hsla = theme.tokens.mention_color.into();
    let mention_bg: Hsla = theme.tokens.mention_primary.into();
    let code_bg: Hsla = theme.tokens.bg_markdown_code.into();
    let link_color: Hsla = theme.tokens.mention_color.into();
    let text = layout.text.clone();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    for run in layout.runs.iter() {
        let style = match run.kind {
            RichRunKind::Bold => HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
            RichRunKind::Code => HighlightStyle {
                background_color: Some(code_bg),
                ..Default::default()
            },
            RichRunKind::Link => HighlightStyle {
                color: Some(link_color),
                underline: Some(gpui::UnderlineStyle {
                    thickness: px(1.),
                    color: Some(link_color),
                    wavy: false,
                }),
                ..Default::default()
            },
            RichRunKind::Mention | RichRunKind::Hashtag => HighlightStyle {
                color: Some(mention_color),
                background_color: Some(mention_bg),
                ..Default::default()
            },
        };
        highlights.push((run.range.clone(), style));
    }
    div()
        .w_full()
        .min_w_0()
        .max_w_full()
        .overflow_hidden()
        .text_sm()
        .line_height(rems(1.25))
        .text_color(theme.tokens.text_theme_message)
        .child(StyledText::new(text).with_highlights(highlights))
        .into_any_element()
}

fn pin_inline_row() -> gpui::Div {
    div()
        .w_full()
        .min_w_0()
        .max_w_full()
        .overflow_hidden()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_baseline()
}

fn pin_plain_line(text: &str, color: impl Into<Hsla>) -> gpui::AnyElement {
    div()
        .w_full()
        .min_w_0()
        .max_w_full()
        .overflow_hidden()
        .text_sm()
        .line_height(rems(1.25))
        .text_color(color)
        .child(soft_break_hints(text))
        .into_any_element()
}

fn pin_heading_size(level: u8) -> gpui::Pixels {
    match level {
        1 => px(36.),
        2 => px(30.),
        3 => px(24.),
        4 => px(20.),
        5 => px(18.),
        _ => px(16.),
    }
}

fn pin_heading_line_height(level: u8) -> impl Into<gpui::DefiniteLength> {
    match level {
        1 => rems(2.5),
        2 => rems(2.25),
        3 => rems(2.),
        4 => rems(1.75),
        5 => rems(1.75),
        _ => rems(1.5),
    }
}

fn render_pin_spans(spans: &[MessageSpan], theme: &Theme) -> gpui::AnyElement {
    let link_color = theme.tokens.mention_color;
    let mention_bg = theme.tokens.mention_primary;
    let mention_color = theme.tokens.mention_color;
    let code_bg = theme.tokens.bg_markdown_code;
    let body_color = theme.tokens.text_theme_message;
    let mut col = v_flex().w_full().min_w_0().max_w_full().overflow_hidden();
    let mut row = pin_inline_row();
    let mut has_inline = false;

    for span in spans {
        match span {
            MessageSpan::Text(text) => {
                for (line_index, line) in text.split('\n').enumerate() {
                    if line_index > 0 {
                        if has_inline {
                            col = col.child(row);
                            row = pin_inline_row();
                            has_inline = false;
                        } else if line.is_empty() {
                            col = col.child(div().w_full().h(px(8.)));
                            continue;
                        }
                    }
                    if line.is_empty() {
                        continue;
                    }
                    if has_inline {
                        row = row.child(
                            div()
                                .text_sm()
                                .line_height(rems(1.25))
                                .text_color(body_color)
                                .child(soft_break_hints(line)),
                        );
                    } else {
                        col = col.child(pin_plain_line(line, body_color));
                    }
                }
            }
            MessageSpan::Bold(text) => {
                has_inline = true;
                row = row.child(
                    div()
                        .text_sm()
                        .line_height(rems(1.25))
                        .font_weight(FontWeight::BOLD)
                        .text_color(body_color)
                        .child(soft_break_hints(text)),
                );
            }
            MessageSpan::Code(text) => {
                has_inline = true;
                row = row.child(
                    div()
                        .text_sm()
                        .line_height(rems(1.25))
                        .px_1()
                        .rounded_sm()
                        .bg(code_bg)
                        .text_color(body_color)
                        .child(soft_break_hints(text)),
                );
            }
            MessageSpan::Link { text, .. } => {
                has_inline = true;
                row = row.child(
                    div()
                        .text_sm()
                        .line_height(rems(1.25))
                        .text_color(link_color)
                        .underline()
                        .child(soft_break_hints(text)),
                );
            }
            MessageSpan::Mention { display, .. } | MessageSpan::Hashtag { display, .. } => {
                has_inline = true;
                row = row.child(
                    div()
                        .text_sm()
                        .line_height(rems(1.25))
                        .px_1()
                        .rounded_sm()
                        .bg(mention_bg)
                        .text_color(mention_color)
                        .child(display.to_string()),
                );
            }
            MessageSpan::Emoji { name, .. } => {
                has_inline = true;
                row = row.child(
                    div()
                        .text_sm()
                        .line_height(rems(1.25))
                        .text_color(body_color)
                        .child(name.to_string()),
                );
            }
            MessageSpan::Canvas { title, .. } => {
                has_inline = true;
                row = row.child(
                    div()
                        .text_sm()
                        .line_height(rems(1.25))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(link_color)
                        .child(title.to_string()),
                );
            }
            MessageSpan::Heading { level, text } => {
                if has_inline {
                    col = col.child(row);
                    row = pin_inline_row();
                    has_inline = false;
                }
                col = col.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .max_w_full()
                        .overflow_hidden()
                        .my(px(2.))
                        .text_size(pin_heading_size(*level))
                        .line_height(pin_heading_line_height(*level))
                        .font_weight(FontWeight::BOLD)
                        .text_color(body_color)
                        .child(soft_break_hints(text)),
                );
            }
            MessageSpan::CodeBlock { text, .. } => {
                if has_inline {
                    col = col.child(row);
                    row = pin_inline_row();
                    has_inline = false;
                }
                let code_text = normalize_pin_code_block(text);
                let mut code_col = v_flex().w_full().min_w_0().overflow_hidden();
                for (i, line) in code_text.split('\n').enumerate() {
                    if i > 0 && line.is_empty() {
                        code_col = code_col.child(div().w_full().h(px(8.)));
                        continue;
                    }
                    code_col = code_col.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(14.))
                            .line_height(rems(1.25))
                            .text_color(body_color)
                            .child(soft_break_hints(line)),
                    );
                }
                col = col.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .max_w_full()
                        .overflow_hidden()
                        .mt(px(4.))
                        .p_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.tokens.border_primary)
                        .bg(code_bg)
                        .child(code_col),
                );
            }
        }
    }
    if has_inline {
        col = col.child(row);
    }
    col.into_any_element()
}

fn render_pin_image_attachment(
    att: &MessageAttachment,
    image_cache: Entity<LruImageCache>,
) -> gpui::AnyElement {
    let width = att.display_width.clamp(1., 280.);
    let height = att.display_height.clamp(1., 200.);
    div()
        .mt_1()
        .w(px(width))
        .h(px(height))
        .max_w_full()
        .flex_shrink_0()
        .overflow_hidden()
        .rounded_md()
        .image_cache(image_cache)
        .child(
            img(att.proxied_src.clone())
                .size_full()
                .object_fit(ObjectFit::Cover),
        )
        .into_any_element()
}

fn render_pin_file_attachment(att: &MessageAttachment, theme: &Theme) -> gpui::AnyElement {
    let filename = if att.filename.is_empty() {
        SharedString::from("Attachment")
    } else {
        SharedString::from(att.filename.clone())
    };
    let size_line = if att.size_label.is_empty() {
        SharedString::from(format!("size: {}", mezon_store::format_file_size(att.size)))
    } else {
        SharedString::from(format!("size: {}", att.size_label))
    };
    let icon = file_type_icon_for(&att.filetype, &att.filename);
    let file_id = SharedString::from(att.url.clone());
    let download_url = file_id.clone();
    let download_name = filename.clone();
    let open_url = download_url.clone();
    let open_name = download_name.clone();
    let group_name = SharedString::from(format!("pin-file-{}", att.url));
    let body_id = group_name.clone();
    let dl_id = SharedString::from(format!("pin-file-dl-{}", att.url));

    div()
        .id(group_name.clone())
        .group(group_name.clone())
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .w_full()
        .max_w_full()
        .min_w_0()
        .mt(px(10.))
        .p_3()
        .rounded_lg()
        .bg(theme.tokens.bg_item_theme_hover)
        .border_1()
        .border_color(theme.tokens.border_primary)
        .overflow_hidden()
        .child(
            div()
                .relative()
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .w(px(32.))
                .h(px(40.))
                .child(img(icon.path()).w(px(32.)).h(px(40.)).flex_none()),
        )
        .child(
            div()
                .id(body_id)
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .cursor_pointer()
                .on_click(move |_: &ClickEvent, _window, cx| {
                    save_with_progress_toast(open_url.clone(), open_name.clone(), cx);
                })
                .child(
                    div()
                        .truncate()
                        .text_size(px(16.))
                        .text_color(gpui::rgb(FILE_NAME_COLOR))
                        .hover(|s| s.underline())
                        .child(filename),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(theme.tokens.text_theme_primary)
                        .child(size_line),
                ),
        )
        .child(
            div()
                .absolute()
                .right(px(12.))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .opacity(0.)
                .group_hover(group_name, |s| s.opacity(1.))
                .child(
                    div()
                        .id(dl_id)
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(32.))
                        .rounded_md()
                        .bg(theme.tokens.bg_theme_contexify)
                        .border_1()
                        .border_color(theme.tokens.border_primary)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .on_click(move |_: &ClickEvent, _window, cx| {
                            cx.stop_propagation();
                            save_with_progress_toast(
                                download_url.clone(),
                                download_name.clone(),
                                cx,
                            );
                        })
                        .child(
                            Icon::new(IconName::Download)
                                .size(px(16.))
                                .text_color(theme.tokens.text_theme_primary),
                        ),
                ),
        )
        .into_any_element()
}

fn render_pin_embeds(
    embeds: &[Embed],
    theme: &Theme,
    image_cache: Entity<LruImageCache>,
) -> Vec<gpui::AnyElement> {
    embeds
        .iter()
        .filter(|embed| {
            !embed.title.is_empty()
                || !embed.description_spans.is_empty()
                || !embed.thumbnail_proxied.is_empty()
        })
        .map(|embed| {
            let description = embed
                .description_spans
                .iter()
                .filter_map(|span| match span {
                    MessageSpan::Text(text) | MessageSpan::Bold(text) | MessageSpan::Code(text) => {
                        Some(text.as_ref())
                    }
                    MessageSpan::Link { text, .. }
                    | MessageSpan::Mention { display: text, .. }
                    | MessageSpan::Hashtag { display: text, .. } => Some(text.as_ref()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            let mut card = v_flex()
                .w_full()
                .min_w_0()
                .mt_1()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(theme.tokens.border_primary)
                .bg(theme.tokens.theme_setting_primary);
            if !embed.title.is_empty() {
                card = card.child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.tokens.text_theme_message)
                        .child(embed.title.clone()),
                );
            }
            if !description.is_empty() {
                card = card.child(
                    div()
                        .text_sm()
                        .text_color(theme.tokens.text_theme_primary)
                        .child(description),
                );
            }
            if !embed.thumbnail_proxied.is_empty() {
                card = card.child(
                    div()
                        .max_w(px(220.))
                        .max_h(px(120.))
                        .overflow_hidden()
                        .rounded_md()
                        .image_cache(image_cache.clone())
                        .child(
                            img(embed.thumbnail_proxied.clone())
                                .w_full()
                                .h_full()
                                .object_fit(ObjectFit::Cover),
                        ),
                );
            }
            card.into_any_element()
        })
        .collect()
}

/// Format a pin's create time (unix seconds): Today at HH:MM, Yesterday at HH:MM, otherwise dd/MM/yyyy, HH:MM (in the local timezone).
fn format_pin_time(create_time: i64, locale: &str) -> String {
    let Some(utc) = chrono::DateTime::from_timestamp(create_time, 0) else {
        return String::new();
    };
    let local = utc.with_timezone(&chrono::Local);
    let date = local.date_naive();
    let today = chrono::Local::now().date_naive();
    let time = local.format("%H:%M").to_string();

    if date == today {
        format!("{} {}", mezon_i18n::t(locale, "common.todayAt"), time)
    } else if Some(date) == today.pred_opt() {
        format!("{} {}", mezon_i18n::t(locale, "common.yesterdayAt"), time)
    } else {
        local.format("%d/%m/%Y, %H:%M").to_string()
    }
}

pub fn pin_popover_on_open() -> Rc<dyn Fn(&mut Window, &mut App)> {
    Rc::new(|_window, cx| {
        PinnedMessagesStore::global(cx).update(cx, |store, cx| {
            store.clear_active_pin_badge(cx);
            store.ensure_loaded(cx);
            if let Some(clan_id) = store.clan_id() {
                ClanMembersStore::global(cx).update(cx, |members, cx| {
                    members.ensure_loaded(clan_id, cx);
                });
            }
        });
    })
}
