use std::time::{Duration, Instant};

use gpui::{
    Context, Entity, ListAlignment, ListState, SharedString, Task, Window, div, list, prelude::*,
    px,
};
use ui::{ScrollAxes, Scrollbars, WithScrollbar};

use mezon_store::{ChannelId, ChannelList, MessagesEvent, MessagesStore, Settings};

use super::context::RowCtx;
use super::dispatch::render_message_item;
use super::skeleton::message_skeleton;
use crate::image_cache::{
    AVATAR_ENTRY_MAX_BYTES, AVATAR_IMAGE_CACHE_BYTES, AVATAR_IMAGE_CACHE_CAPACITY, LruImageCache,
    MESSAGE_ENTRY_MAX_BYTES, MESSAGE_IMAGE_CACHE_BYTES, MESSAGE_IMAGE_CACHE_CAPACITY,
};
use crate::theme::ActiveTheme;

/// How close (in rows) to an edge of the rendered window before we reveal/fetch
/// more. Approximates React's pixel sensitive area (`MESSAGE_LIST_SENSITIVE_AREA
/// = 1500px`) so loading starts well before the user reaches the very edge,
/// rather than only at the top/bottom row.
const LOAD_MORE_ITEM_THRESHOLD: usize = 12;
/// Pixels of off-screen content rendered/measured above and below the viewport.
/// Chat rows are short and numerous, so a screenful of overdraw keeps fast
/// scrolling smooth without laying out a huge number of rows every frame (Zed's
/// 2048 suits its tall, sparse agent messages, not a dense chat list).
const LIST_OVERDRAW: f32 = 1024.;
const SCROLL_HOVER_RELEASE_MS: u64 = 150;
/// Minimum gap between scroll-triggered pagination attempts. The list emits
/// scroll events at ~120fps while the user holds the scrollbar at an edge;
/// without this, every event would re-trigger reveal/load (flooding the guard
/// and racing through cached history). Leading-edge throttle, like React's
/// debounced `loadMore`.
const PAGINATE_THROTTLE: Duration = Duration::from_millis(250);

/// The chat message timeline. It renders the store's bounded viewport window
/// (≤ `VIEWPORT_LIMIT` rows — the React `ChannelMessages` viewport slice) via a
/// bottom-aligned `gpui::list`, which provides correct scroll anchoring and
/// tail-follow. The `ListState` is only mutated from `MessagesEvent`s so renders
/// stay flicker-free.
pub struct ChannelMessages {
    pub(crate) list_state: ListState,
    settings: Entity<Settings>,
    image_cache: Entity<LruImageCache>,
    avatar_image_cache: Entity<LruImageCache>,
    cached_for_channel: Option<ChannelId>,
    skeleton_armed: bool,
    skeleton_channel: Option<ChannelId>,
    _skeleton_timer: Option<Task<()>>,
    suppress_hover: bool,
    _hover_release_task: Option<Task<()>>,
    /// Last time a scroll gesture triggered a reveal/load (throttle).
    last_paginate: Option<Instant>,
    /// Last scroll event time, used to release hover suppression after idle.
    last_scroll_at: Option<Instant>,
    /// Whether the list is auto-following the tail. Only enabled when the window
    /// reaches the true newest message; disabled while `has_more_bottom` so
    /// scrolling down loads newer pages instead of snapping to the bottom.
    /// Whether the newest message is currently in view, so appended messages
    /// should keep the bottom pinned (React: scroll to bottom when not viewing
    /// older). Maintained from the scroll handler.
    at_bottom: bool,
    /// Absolute list index of the first visible row, captured from the latest
    /// scroll event. Used to re-anchor the viewport after a forward-page append
    /// (the `ListAlignment::Bottom` anchor is `None` at the edge, so `splice`
    /// can't keep the position and the list would re-pin to the new bottom).
    last_visible_start: usize,
    /// Whether the persistent top loading skeleton occupies list index 0
    /// (React `LoadingSkeletonMessages`, shown while there is more history).
    header_shown: bool,
    /// A message to scroll to on the next render (set from `MessagesEvent::
    /// JumpTo`). Deferred to render so the header reconcile has settled and the
    /// row index is correct.
    pending_jump: Option<SharedString>,
    /// Message id to briefly highlight after a jump (React jump highlight).
    highlight_id: Option<SharedString>,
    _highlight_timer: Option<Task<()>>,
}

impl ChannelMessages {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let store = MessagesStore::global(cx);
        cx.subscribe(&store, |this, _store, event, cx| {
            match event {
                MessagesEvent::Reset { count } => {
                    this.list_state.reset(*count);
                    // Open pinned to the newest message (React opens a channel at
                    // the bottom). We never use gpui's auto-tail; instead we scroll
                    // explicitly so loading older/newer pages never yanks the view
                    // to an edge.
                    this.list_state.scroll_to_end();
                    this.at_bottom = true;
                    // `reset` drops every row including the header; render re-adds
                    // it if more history exists.
                    this.header_shown = false;
                }
                MessagesEvent::Shifted {
                    added_top,
                    removed_top,
                    added_bottom,
                    removed_bottom,
                } => {
                    // The header (if any) sits at index 0; messages follow it.
                    let h = usize::from(this.header_shown);
                    if *removed_top > 0 {
                        this.list_state.splice(h..h + *removed_top, 0);
                    }
                    if *added_top > 0 {
                        this.list_state.splice(h..h, *added_top);
                    }
                    if *removed_bottom > 0 {
                        let n = this.list_state.item_count();
                        this.list_state
                            .splice(n.saturating_sub(*removed_bottom)..n, 0);
                    }
                    if *added_bottom > 0 {
                        let n = this.list_state.item_count();
                        this.list_state.splice(n..n, *added_bottom);
                    }
                    // Re-anchor the viewport after the splice. With
                    // `ListAlignment::Bottom`, the scroll anchor is `None` while
                    // the user is parked at an edge, so `splice` cannot shift it —
                    // the list would re-pin to the new bottom (forward paging) or
                    // stick to the skeleton header (back paging), both of which
                    // re-trigger load-more in a cascade. Anchor explicitly.
                    let following_new =
                        *added_bottom > 0 && this.at_bottom && !_store.read(cx).has_more_bottom();
                    if following_new {
                        // Parked at the true newest: follow the appended message
                        // (React `scrollToBottom` when not viewing older).
                        this.list_state.scroll_to_end();
                    } else if *added_top > 0 {
                        // Prepend (older page): anchor to the first real message so
                        // the skeleton header at index 0 doesn't keep the view at
                        // the top and re-trigger load-more.
                        let first_real = h + *added_top;
                        if this.list_state.logical_scroll_top().item_ix < first_real {
                            this.list_state.scroll_to(gpui::ListOffset {
                                item_ix: first_real,
                                offset_in_item: px(0.),
                            });
                        }
                    } else if *added_bottom > 0 || *removed_top > 0 {
                        // Forward pagination (newer page) or a non-followed append:
                        // keep the row that was at the top of the viewport in place
                        // (it shifts up by `removed_top` after the front trim), so
                        // the user stays on the same messages and the new rows sit
                        // below them.
                        let new_top = this.last_visible_start.saturating_sub(*removed_top);
                        this.list_state.scroll_to(gpui::ListOffset {
                            item_ix: new_top,
                            offset_in_item: px(0.),
                        });
                    }
                }
                MessagesEvent::Updated => {}
                MessagesEvent::JumpTo { message_id } => {
                    // Defer the actual scroll to render (header reconcile + any
                    // preceding `Reset` splice must settle first), then highlight
                    // the row for ~1s (React jump highlight).
                    this.pending_jump = Some(message_id.clone());
                    this.highlight_id = Some(message_id.clone());
                    this._highlight_timer = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(1500))
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            this.highlight_id = None;
                            cx.notify();
                        });
                    }));
                }
            }
            cx.notify();
        })
        .detach();

        // Generous overdraw (matching Zed's agent chat) so rows just outside the
        // viewport are measured ahead of time — keeps variable-height rows (with
        // images/attachments) from popping in or jumping the scrollbar.
        let list_state = ListState::new(0, ListAlignment::Bottom, px(LIST_OVERDRAW));
        let timeline = cx.weak_entity();
        list_state.set_scroll_handler(move |event, _window, cx| {
            // NB: do not call `list_state.item_count()` here — the list holds its
            // state borrowed while invoking this handler. Use `event.count`.
            // Load older only when the user has actually scrolled up — the top
            // is near AND there are rows below the viewport (`end < count`). In
            // normal flow the newest rows are always in the window, so the
            // bottom edge does nothing; after a jump-to-message (where the newest
            // is not loaded) reaching the bottom fetches the next newer page.
            let near_top = event.visible_range.start < LOAD_MORE_ITEM_THRESHOLD
                && event.visible_range.end < event.count;
            let near_bottom = event.visible_range.end + LOAD_MORE_ITEM_THRESHOLD >= event.count
                && event.visible_range.start > 0;
            // Pinned to the newest row when the bottom edge is in view. Drives the
            // explicit follow-on-append (React `scrollToBottom` when not viewing
            // older). `start == 0` means the whole window fits, which still counts
            // as being at the bottom.
            let at_bottom = event.visible_range.end + LOAD_MORE_ITEM_THRESHOLD >= event.count;
            let visible_start = event.visible_range.start;
            let _ = timeline.update(cx, |this, cx| {
                this.at_bottom = at_bottom;
                this.last_visible_start = visible_start;
                // Suppress hover affordances while scrolling. Record the time on
                // every event (cheap) but spawn the release watcher only once per
                // scroll session — the list fires ~120 events/sec, so spawning a
                // task per event would churn the executor.
                this.last_scroll_at = Some(Instant::now());
                if !this.suppress_hover {
                    this.suppress_hover = true;
                    cx.notify();
                    this._hover_release_task = Some(cx.spawn(async move |this, cx| {
                        let idle = Duration::from_millis(SCROLL_HOVER_RELEASE_MS);
                        loop {
                            cx.background_executor().timer(idle).await;
                            let still_scrolling = this
                                .update(cx, |this, _| {
                                    this.last_scroll_at.is_some_and(|t| t.elapsed() < idle)
                                })
                                .unwrap_or(false);
                            if still_scrolling {
                                continue;
                            }
                            let _ = this.update(cx, |this, cx| {
                                this.suppress_hover = false;
                                cx.notify();
                            });
                            break;
                        }
                    }));
                }

                // Throttle pagination: the list fires scroll events ~120fps
                // while the user holds the scrollbar at an edge. Only act once
                // per `PAGINATE_THROTTLE`.
                if !(near_top || near_bottom) {
                    return;
                }
                let now = Instant::now();
                if this
                    .last_paginate
                    .is_some_and(|t| now.duration_since(t) < PAGINATE_THROTTLE)
                {
                    return;
                }
                let store = MessagesStore::global(cx);
                tracing::debug!(
                    near_top,
                    near_bottom,
                    start = event.visible_range.start,
                    end = event.visible_range.end,
                    count = event.count,
                    has_more_bottom = store.read(cx).has_more_bottom(),
                    "timeline pagination trigger"
                );
                if near_top {
                    this.last_paginate = Some(now);
                    store.update(cx, |store, cx| store.scroll_reached_top(cx));
                } else if store.read(cx).has_more_bottom() {
                    this.last_paginate = Some(now);
                    store.update(cx, |store, cx| store.scroll_reached_bottom(cx));
                }
            });
        });

        let image_cache = cx.new(|cx| {
            LruImageCache::labeled(
                "msg-image",
                MESSAGE_IMAGE_CACHE_CAPACITY,
                MESSAGE_IMAGE_CACHE_BYTES,
                MESSAGE_ENTRY_MAX_BYTES,
                cx,
            )
        });
        let avatar_image_cache = cx.new(|cx| {
            LruImageCache::avatar_thumbnail(
                "msg-avatar",
                AVATAR_IMAGE_CACHE_CAPACITY,
                AVATAR_IMAGE_CACHE_BYTES,
                AVATAR_ENTRY_MAX_BYTES,
                cx,
            )
        });
        Self {
            list_state,
            settings,
            image_cache,
            avatar_image_cache,
            cached_for_channel: None,
            skeleton_armed: false,
            skeleton_channel: None,
            _skeleton_timer: None,
            suppress_hover: false,
            _hover_release_task: None,
            last_paginate: None,
            last_scroll_at: None,
            at_bottom: true,
            last_visible_start: 0,
            header_shown: false,
            pending_jump: None,
            highlight_id: None,
            _highlight_timer: None,
        }
    }

    fn clear_image_cache_if_channel_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channel_id = ChannelList::global(cx).read(cx).active_channel_id;
        if self.cached_for_channel == channel_id {
            return;
        }
        self.cached_for_channel = channel_id;
        self.image_cache
            .update(cx, |cache, cx| cache.clear(window, cx));
        self.avatar_image_cache
            .update(cx, |cache, cx| cache.clear(window, cx));
    }
}

impl Render for ChannelMessages {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::trace_render!("ChannelMessages");
        self.clear_image_cache_if_channel_changed(window, cx);
        self.image_cache
            .update(cx, |cache, cx| cache.sweep(window, cx));

        let store = MessagesStore::global(cx);
        let channel_id = ChannelList::global(cx).read(cx).active_channel_id;
        let is_empty = store.read(cx).viewport_messages().is_empty();
        let loading = store.read(cx).is_loading() && is_empty;
        if loading {
            if self.skeleton_channel != channel_id {
                self.skeleton_channel = channel_id;
                self.skeleton_armed = false;
                self._skeleton_timer = Some(cx.spawn(async move |this, cx| {
                    // Match React `StickyLoadingIndicator`: only show the
                    // first-load skeleton after 1s, so it never flashes for the
                    // common fast (<100ms) fetch.
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(1000))
                        .await;
                    this.update(cx, |this, cx| {
                        this.skeleton_armed = true;
                        cx.notify();
                    })
                    .ok();
                }));
            }
        } else {
            self.skeleton_armed = false;
            self.skeleton_channel = None;
            self._skeleton_timer = None;
        }
        let show_skeleton = loading && self.skeleton_armed;

        if show_skeleton {
            // First load of an empty channel: full skeleton, bottom-aligned and
            // left-aligned like React's `StickyLoadingIndicator`.
            return div()
                .size_full()
                .flex()
                .flex_col()
                .justify_end()
                .image_cache(self.image_cache.clone())
                .child(message_skeleton(cx.theme(), 5))
                .into_any_element();
        }

        // Reconcile the persistent top loading skeleton at list index 0 (React
        // `LoadingSkeletonMessages`, shown while `hasMoreTop` AND there is at
        // least one message — React guards on `messageIds?.[0]`). It scrolls
        // with the content and is only visible at the top, where load-more
        // triggers.
        let want_header = !is_empty && store.read(cx).has_more_top();
        if want_header && !self.header_shown {
            self.list_state.splice(0..0, 1);
            self.header_shown = true;
        } else if !want_header && self.header_shown {
            self.list_state.splice(0..1, 0);
            self.header_shown = false;
        }
        let header_shown = self.header_shown;

        // Deferred jump-to-message: now that the row count + header are settled,
        // scroll the target into view (React `scrollIntoView`).
        if let Some(target) = self.pending_jump.take() {
            if let Some(pos) = store
                .read(cx)
                .viewport_messages()
                .iter()
                .position(|m| m.id.as_str() == target.as_ref())
            {
                self.list_state
                    .scroll_to_reveal_item(usize::from(header_shown) + pos);
            }
        }

        let locale = self.settings.read(cx).language.clone();
        let list_state = self.list_state.clone();
        let suppress_hover = self.suppress_hover;
        let avatar_image_cache = self.avatar_image_cache.clone();
        let unread_boundary_id = unread_boundary(&store, cx);
        let highlight_id = self.highlight_id.clone();

        div()
            .size_full()
            .overflow_hidden()
            .image_cache(self.image_cache.clone())
            .child(
                list(list_state, move |ix, _window, cx| {
                    if header_shown && ix == 0 {
                        return div()
                            .id("msg-loading-top")
                            .py_2()
                            .child(message_skeleton(cx.theme(), 5))
                            .into_any_element();
                    }
                    let msg_ix = ix - usize::from(header_shown);
                    let store = MessagesStore::global(cx);
                    let ctx = RowCtx {
                        theme: cx.theme(),
                        locale: &locale,
                        current_user_id: "",
                        suppress_hover,
                        avatar_cache: avatar_image_cache.clone(),
                        unread_boundary_id: unread_boundary_id.clone(),
                        highlight_id: highlight_id.clone(),
                    };
                    render_message_item(store.read(cx).viewport_messages(), msg_ix, &ctx)
                })
                .flex_1()
                .size_full(),
            )
            .custom_scrollbars(
                Scrollbars::new(ScrollAxes::Vertical).tracked_scroll_handle(&self.list_state),
                window,
                cx,
            )
            .into_any_element()
    }
}

/// Find the id of the first message newer than the channel's last-seen
/// timestamp — the row above which the "New messages" break is shown. Returns
/// `None` when the channel has never been seen or is fully caught up.
fn unread_boundary(
    store: &Entity<MessagesStore>,
    cx: &Context<ChannelMessages>,
) -> Option<SharedString> {
    let channel_list = ChannelList::global(cx);
    let cl = channel_list.read(cx);
    let last_seen = cl
        .active_channel_id
        .and_then(|id| cl.find_channel(id))
        .map(|c| c.last_seen_timestamp)
        .unwrap_or(0);
    if last_seen <= 0 {
        return None;
    }
    store
        .read(cx)
        .viewport_messages()
        .iter()
        .find(|m| m.create_time > last_seen)
        .map(|m| SharedString::from(m.id.clone()))
}
