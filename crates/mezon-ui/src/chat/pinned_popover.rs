use std::rc::Rc;

use gpui::{
    App, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    ScrollHandle, SharedString, Window, div, point, prelude::*, px,
};
use mezon_store::{MessagesStore, PinnedMessage, PinnedMessagesStore, Settings};
use ui::prelude::*;
use ui::{Label, PopoverMenuHandle, ScrollAxes, Scrollbars, WithScrollbar};

use crate::components::primitives::{Avatar, Button, ButtonVariants, Icon, IconName, Sizable, Size, h_flex, v_flex};
use crate::theme::{ActiveTheme, Theme};

const POPOVER_WIDTH: f32 = 420.;
const HEADER_HEIGHT: f32 = 48.;
const MIN_BODY_HEIGHT: f32 = 144.;
const MAX_VH: f32 = 0.8;

pub struct PinnedPopoverPanel {
    settings: Entity<Settings>,
    popover_handle: PopoverMenuHandle<PinnedPopoverPanel>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
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

        cx.observe(&PinnedMessagesStore::global(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        Self {
            settings,
            popover_handle,
            scroll: ScrollHandle::new(),
            focus_handle,
        }
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
        let pinned = store.read(cx).pinned().to_vec();
        let loading = store.read(cx).is_loading();
        let handle = self.popover_handle.clone();
        let scroll = self.scroll.clone();

        v_flex()
            .key_context("menu")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &::menu::Cancel, _window, cx| {
                cx.emit(DismissEvent);
            }))
            .w(px(POPOVER_WIDTH))
            .overflow_hidden()
            .elevation_2(cx)
            .child(render_header(&theme, &locale))
            .child(render_body(
                &pinned,
                loading,
                &theme,
                &locale,
                handle,
                &scroll,
                window,
                cx,
            ))
    }
}

fn render_header(theme: &Theme, locale: &str) -> impl IntoElement {
    let tokens = &theme.tokens;
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .px(px(16.))
        .h(px(48.))
        .border_b_1()
        .border_color(tokens.border_primary)
        .bg(tokens.theme_setting_nav)
        .child(
            Icon::new(IconName::PinRight)
                .size_4()
                .text_color(tokens.text_theme_primary),
        )
        .child(
            Label::new(mezon_i18n::t(
                locale,
                "channelTopbar.modals.pinnedMessages.title",
            ))
            .size(LabelSize::Default),
        )
}

#[allow(clippy::too_many_arguments)]
fn render_body(
    pinned: &[PinnedMessage],
    loading: bool,
    theme: &Theme,
    locale: &str,
    popover_handle: PopoverMenuHandle<PinnedPopoverPanel>,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut Context<PinnedPopoverPanel>,
) -> impl IntoElement {
    let tokens = &theme.tokens;

    let content = if pinned.is_empty() {
        if loading {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .min_h(px(MIN_BODY_HEIGHT))
                .child(crate::components::primitives::Spinner::new().with_size(Size::Small))
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .min_h(px(MIN_BODY_HEIGHT))
                .text_sm()
                .text_color(tokens.text_theme_primary)
                .child(mezon_i18n::t(
                    locale,
                    "channelTopbar.pinnedMessages.emptyTitle",
                ))
                .into_any_element()
        }
    } else {
        v_flex()
            .w_full()
            .gap_2()
            .py(px(8.))
            .children(
                pinned
                    .iter()
                    .enumerate()
                    .map(|(index, msg)| {
                        pin_card(index, msg, theme, locale, popover_handle.clone())
                    }),
            )
            .into_any_element()
    };

    let viewport_h = f32::from(window.viewport_size().height);
    let max_body = px((viewport_h * MAX_VH - HEADER_HEIGHT).max(MIN_BODY_HEIGHT));

    let scroll_body = div()
        .id("pin-body")
        .w_full()
        .flex_1()
        .min_h_0()
        .pr(px(6.))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .child(content)
        .custom_scrollbars(
            Scrollbars::new(ScrollAxes::Vertical).tracked_scroll_handle(scroll),
            window,
            cx,
        );

    let scrollable = f32::from(scroll.max_offset().y) > 0.5;
    let mut body_children: Vec<gpui::AnyElement> = Vec::new();
    if scrollable {
        body_children.push(scroll_arrow("pin-scroll-up", "⌃", true, scroll, theme));
    }
    body_children.push(scroll_body.into_any_element());
    if scrollable {
        body_children.push(scroll_arrow("pin-scroll-down", "⌄", false, scroll, theme));
    }

    v_flex()
        .w_full()
        .min_h(px(MIN_BODY_HEIGHT))
        .max_h(max_body)
        .children(body_children)
}

fn scroll_arrow(
    id: &'static str,
    glyph: &'static str,
    up: bool,
    scroll: &ScrollHandle,
    theme: &Theme,
) -> gpui::AnyElement {
    let tokens = &theme.tokens;
    let handle = scroll.clone();
    h_flex()
        .w_full()
        .h(px(13.))
        .flex_shrink_0()
        .justify_end()
        .pr(px(2.))
        .child(
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .w(px(13.))
                .h(px(13.))
                .text_size(px(8.))
                .text_color(tokens.text_theme_primary)
                .cursor_pointer()
                .hover(|s| s.text_color(tokens.text_theme_primary_hover))
                .child(glyph)
                .on_click(move |_: &ClickEvent, _window, _cx| {
                    let off = handle.offset();
                    let y = f32::from(off.y);
                    let max_y = f32::from(handle.max_offset().y);
                    const STEP: f32 = 64.;
                    let new_y = if up {
                        (y + STEP).min(0.)
                    } else {
                        (y - STEP).max(-max_y)
                    };
                    handle.set_offset(point(off.x, px(new_y)));
                }),
        )
        .into_any_element()
}

fn pin_card(
    index: usize,
    msg: &PinnedMessage,
    theme: &Theme,
    locale: &str,
    popover_handle: PopoverMenuHandle<PinnedPopoverPanel>,
) -> gpui::AnyElement {
    let tokens = &theme.tokens;
    let group_name = SharedString::from(format!("pin-card-{index}"));

    let mut avatar = Avatar::new().name(&msg.sender_name).with_size(Size::Small);
    if !msg.avatar_proxied.is_empty() {
        avatar = avatar.src(msg.avatar_proxied.clone());
    } else if !msg.avatar_url.is_empty() {
        avatar = avatar.src(msg.avatar_url.as_str());
    }

    let name_row = h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(tokens.text_theme_primary)
                .child(msg.sender_name.clone()),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(tokens.text_theme_primary)
                .child(format_pin_time(msg.create_time, locale)),
        );

    let content = div()
        .text_sm()
        .text_color(tokens.text_theme_message)
        .child(msg.content.clone());

    let jump_message_id = msg.message_id.clone();
    let jump_handle = popover_handle.clone();
    let jump = Button::new(("pin-jump", index))
        .label(mezon_i18n::t(locale, "channelTopbar.tooltips.jump"))
        .ghost()
        .with_size(Size::XSmall)
        .on_click(move |_: &ClickEvent, _window, cx| {
            MessagesStore::global(cx).update(cx, |store, cx| {
                store.jump_to_message(jump_message_id.clone(), cx);
            });
            jump_handle.hide(cx);
        });

    let pin_id = msg.id.clone();
    let message_id = msg.message_id.clone();
    let delete = Button::new(("pin-del", index))
        .label("✕")
        .ghost()
        .with_size(Size::XSmall)
        .on_click(move |_: &ClickEvent, _window, cx| {
            let pin_id = pin_id.clone();
            let message_id = message_id.clone();
            PinnedMessagesStore::global(cx).update(cx, move |store, cx| {
                store.unpin(&pin_id, &message_id, cx);
            });
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
        .mx(px(8.))
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
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(name_row)
                .child(content),
        )
        .child(actions)
        .into_any_element()
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
        PinnedMessagesStore::global(cx).update(cx, |store, cx| store.ensure_loaded(cx));
    })
}
