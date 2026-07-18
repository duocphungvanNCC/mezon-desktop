use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, FocusHandle, MouseButton, Pixels, Render,
    SharedString, Subscription, Task, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    div, prelude::*, px, size,
};
use std::collections::HashMap;
use std::time::Duration;
use mezon_webview::ChannelAppWebView;

use crate::app::main_window::{activate_main_window, main_window_bounds};
use crate::app::window_controls;
use crate::components::primitives::{Icon, IconName};
use crate::theme::{ActiveTheme, Theme};

const MIN_WIDTH: f32 = 400.0;
const MIN_HEIGHT: f32 = 300.0;
const FOOTER_HEIGHT: f32 = 29.0;
const NAV_BUTTON_SIZE: f32 = 28.0;
const NAV_ICON_SIZE: f32 = 16.0;
const TOOL_BAR_EDGE_SPACE: f32 = 4.0;
const ACTION_SUCCESS_MS: u64 = 1200;
const TITLE_BAR_ACTIONS: [TitleBarAction; 4] = [
    TitleBarAction::Back,
    TitleBarAction::Forward,
    TitleBarAction::Reload,
    TitleBarAction::CopyUrl,
];

#[derive(Copy, Clone, PartialEq, Eq)]
enum TitleBarAction {
    Back,
    Forward,
    Reload,
    CopyUrl,
}

impl TitleBarAction {
    fn id(self) -> &'static str {
        match self {
            Self::Back => "channel-app-back",
            Self::Forward => "channel-app-forward",
            Self::Reload => "channel-app-reload",
            Self::CopyUrl => "channel-app-copy-url",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Back => IconName::ArrowLeft,
            Self::Forward => IconName::ArrowRight,
            Self::Reload => IconName::ReloadIcon,
            Self::CopyUrl => IconName::CopyIcon,
        }
    }
}

pub struct OpenChannelAppRequest {
    pub app_id: i64,
    pub url: String,
    pub title: SharedString,
}

#[derive(Clone)]
struct ChannelAppStoreRequest {
    app_id: i64,
    app_url: String,
    app_name: String,
    clan_id: mezon_store::ClanId,
    clan_name: String,
    channel_list: gpui::Entity<mezon_store::ChannelList>,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum PresentMode {
    Launch,
    Reset,
}

struct GlobalChannelAppWindows(HashMap<i64, WindowHandle<ChannelAppWindow>>);
impl gpui::Global for GlobalChannelAppWindows {}

fn channel_app_title(app_name: &str) -> SharedString {
    if app_name.is_empty() {
        SharedString::from("Channel App")
    } else {
        SharedString::from(app_name.to_string())
    }
}

fn ensure_channel_app_windows(cx: &mut App) {
    if !cx.has_global::<GlobalChannelAppWindows>() {
        cx.set_global(GlobalChannelAppWindows(HashMap::new()));
    }
}

fn channel_app_handle(app_id: i64, cx: &App) -> Option<WindowHandle<ChannelAppWindow>> {
    cx.try_global::<GlobalChannelAppWindows>()
        .and_then(|windows| windows.0.get(&app_id).cloned())
}

fn update_channel_app(
    app_id: i64,
    cx: &mut App,
    update: impl FnOnce(&mut ChannelAppWindow, &mut Window, &mut Context<ChannelAppWindow>),
) -> bool {
    let Some(handle) = channel_app_handle(app_id, cx) else {
        return false;
    };
    handle.update(cx, update).is_ok()
}

fn register_channel_app_window(
    app_id: i64,
    handle: WindowHandle<ChannelAppWindow>,
    cx: &mut App,
) {
    ensure_channel_app_windows(cx);
    cx.global_mut::<GlobalChannelAppWindows>()
        .0
        .insert(app_id, handle);
    cx.refresh_windows();
}

fn unregister_channel_app_window(app_id: i64, cx: &mut App) {
    if cx.try_global::<GlobalChannelAppWindows>().is_none() {
        return;
    }
    let empty = {
        let windows = cx.global_mut::<GlobalChannelAppWindows>();
        windows.0.remove(&app_id);
        windows.0.is_empty()
    };
    if empty {
        cx.remove_global::<GlobalChannelAppWindows>();
    }
    cx.refresh_windows();
}

pub fn close_channel_app_window(cx: &mut App) {
    let handles: Vec<WindowHandle<ChannelAppWindow>> = cx
        .try_global::<GlobalChannelAppWindows>()
        .map(|windows| windows.0.values().cloned().collect())
        .unwrap_or_default();
    if handles.is_empty() {
        return;
    }
    cx.remove_global::<GlobalChannelAppWindows>();
    for handle in handles {
        let _ = handle.update(cx, |viewer, window, cx| {
            viewer.release_webview(window, cx);
            window.remove_window();
        });
    }
    cx.refresh_windows();
}

/// Returns true when at least one channel app popup is open.
pub fn is_channel_app_window_open(cx: &App) -> bool {
    cx.try_global::<GlobalChannelAppWindows>()
        .is_some_and(|windows| !windows.0.is_empty())
}

/// Returns true when the channel app popup is open for the given app id string.
pub fn is_channel_app_open(app_id: &str, cx: &App) -> bool {
    let Ok(app_id) = app_id.parse::<i64>() else {
        return false;
    };
    is_channel_app_open_id(app_id, cx)
}

/// Returns true when the channel app popup is open for the given app id.
pub fn is_channel_app_open_id(app_id: i64, cx: &App) -> bool {
    channel_app_handle(app_id, cx).is_some()
}

/// Brings an already-open channel app window to the front. Returns false if none is open.
pub fn focus_channel_app_window(app_id: i64, cx: &mut App) -> bool {
    update_channel_app(app_id, cx, |_, window, _| {
        window.activate_window();
    })
}

fn request_channel_app_from_store(request: ChannelAppStoreRequest, mode: PresentMode, cx: &mut App) {
    let ChannelAppStoreRequest {
        app_id,
        app_url,
        app_name,
        clan_id,
        clan_name,
        channel_list,
    } = request;
    let task = channel_list.update(cx, |store, cx| {
        store.fetch_channel_app_url(app_id, app_url, clan_id, clan_name, cx)
    });
    cx.spawn(async move |cx| {
        let Some(url) = task.await else {
            return;
        };
        let _ = cx.update(|cx| {
            present_channel_app_window(
                OpenChannelAppRequest {
                    app_id,
                    url,
                    title: channel_app_title(&app_name),
                },
                mode,
                cx,
            );
        });
    })
    .detach();
}

/// Fetch a signed launch URL and open the app, or focus the window if it is already open.
pub fn launch_channel_app_from_store(
    app_id: i64,
    app_url: String,
    app_name: String,
    clan_id: mezon_store::ClanId,
    clan_name: String,
    channel_list: gpui::Entity<mezon_store::ChannelList>,
    cx: &mut App,
) {
    if focus_channel_app_window(app_id, cx) {
        return;
    }
    request_channel_app_from_store(
        ChannelAppStoreRequest {
            app_id,
            app_url,
            app_name,
            clan_id,
            clan_name,
            channel_list,
        },
        PresentMode::Launch,
        cx,
    );
}

/// Fetch a fresh signed URL and reload the app window (Reset App), creating one if needed.
pub fn reset_channel_app_from_store(
    app_id: i64,
    app_url: String,
    app_name: String,
    clan_id: mezon_store::ClanId,
    clan_name: String,
    channel_list: gpui::Entity<mezon_store::ChannelList>,
    cx: &mut App,
) {
    request_channel_app_from_store(
        ChannelAppStoreRequest {
            app_id,
            app_url,
            app_name,
            clan_id,
            clan_name,
            channel_list,
        },
        PresentMode::Reset,
        cx,
    );
}

pub fn open_channel_app_window(request: OpenChannelAppRequest, cx: &mut App) {
    cx.defer(move |cx| present_channel_app_window(request, PresentMode::Launch, cx));
}

fn present_channel_app_window(request: OpenChannelAppRequest, mode: PresentMode, cx: &mut App) {
    let app_id = request.app_id;
    if channel_app_handle(app_id, cx).is_some() {
        let _ = update_channel_app(app_id, cx, |viewer, window, cx| {
            if mode == PresentMode::Reset {
                viewer.set_request(request, window, cx);
            }
            window.activate_window();
        });
        return;
    }
    spawn_channel_app_window(request, cx);
}

fn default_window_bounds(cx: &mut App) -> Bounds<Pixels> {
    if let Some(main) = main_window_bounds(cx) {
        let w = (f32::from(main.size.width) * 0.8).max(MIN_WIDTH);
        let h = (f32::from(main.size.height) * 0.8).max(MIN_HEIGHT);
        return Bounds::centered(None, size(px(w), px(h)), cx);
    }
    Bounds::centered(None, size(px(900.0), px(700.0)), cx)
}

fn spawn_channel_app_window(request: OpenChannelAppRequest, cx: &mut App) {
    let app_id = request.app_id;
    let bounds = default_window_bounds(cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
        kind: WindowKind::Normal,
        focus: true,
        show: true,
        titlebar: Some(window_controls::window_title_options()),
        window_decorations: window_controls::main_window_decorations(),
        ..Default::default()
    };

    match cx.open_window(options, |window, cx| {
        cx.new(|cx| ChannelAppWindow::new(request, window, cx))
    }) {
        Ok(handle) => {
            #[cfg(target_os = "macos")]
            {
                let handle = handle.clone();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| {
                        window_controls::macos::disable_window_fullscreen(window);
                    });
                });
            }
            register_channel_app_window(app_id, handle, cx);
        }
        Err(error) => tracing::error!("failed to open channel app window: {error}"),
    }
}

pub struct ChannelAppWindow {
    focus_handle: FocusHandle,
    title: SharedString,
    url: SharedString,
    webview: Option<ChannelAppWebView>,
    webview_init_scheduled: bool,
    last_webview_size: Option<(f64, f64)>,
    action_success: Option<TitleBarAction>,
    _action_success_reset: Option<Task<()>>,
    _bounds_observer: Option<Subscription>,
}

impl ChannelAppWindow {
    fn new(request: OpenChannelAppRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let app_id = request.app_id;
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |closing_window, app| {
            let _ = weak.update(app, |viewer, cx| {
                viewer.release_webview(closing_window, cx);
            });
            unregister_channel_app_window(app_id, app);
            if !is_channel_app_window_open(app) {
                activate_main_window(app);
            }
            true
        });

        let mut this = Self {
            focus_handle,
            title: request.title,
            url: SharedString::from(request.url),
            webview: None,
            webview_init_scheduled: false,
            last_webview_size: None,
            action_success: None,
            _action_success_reset: None,
            _bounds_observer: None,
        };
        this.validate_bounds_observer(window, cx);
        this.schedule_webview_init(window, cx);
        this
    }

    fn set_request(
        &mut self,
        request: OpenChannelAppRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.release_webview(window, cx);
        self.action_success = None;
        self._action_success_reset = None;
        self.title = request.title;
        self.url = SharedString::from(request.url);
        self.schedule_webview_init(window, cx);
        cx.notify();
    }

    fn validate_bounds_observer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self._bounds_observer.is_some() {
            return;
        }
        self._bounds_observer = Some(cx.observe_window_bounds(window, |_, _, cx| {
            cx.notify();
        }));
    }

    fn schedule_webview_init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.webview.is_some() || self.webview_init_scheduled {
            return;
        }
        self.webview_init_scheduled = true;
        cx.defer_in(window, |this, window, cx| {
            this.webview_init_scheduled = false;
            if this.webview.is_some() {
                return;
            }
            let (width, height) = Self::window_size(window);
            match mezon_webview::create_as_window(window, this.url.as_ref(), width, height) {
                Ok(webview) => {
                    this.webview = Some(webview);
                    this.last_webview_size = Some((width, height));
                }
                Err(error) => tracing::error!("channel app webview failed: {error:#}"),
            }
            cx.notify();
        });
    }

    fn window_size(window: &Window) -> (f64, f64) {
        let size = window.bounds().size;
        (f64::from(size.width), f64::from(size.height))
    }

    fn sync_webview_bounds(&mut self, window: &Window) {
        let Some(webview) = self.webview.as_ref() else {
            return;
        };
        let size = Self::window_size(window);
        if self.last_webview_size == Some(size) {
            return;
        }
        if mezon_webview::resize_webview(webview, size.0, size.1).is_ok() {
            self.last_webview_size = Some(size);
        } else {
            tracing::warn!("Channel app webview resize failed; recreating on next frame");
            self.webview = None;
            self.last_webview_size = None;
        }
    }

    fn with_webview(&self, action: impl FnOnce(&ChannelAppWebView)) {
        if let Some(webview) = self.webview.as_ref() {
            action(webview);
        }
    }

    fn reload(&mut self) {
        self.with_webview(mezon_webview::reload);
    }

    fn go_back(&mut self) {
        self.with_webview(mezon_webview::go_back);
    }

    fn go_forward(&mut self) {
        self.with_webview(mezon_webview::go_forward);
    }

    fn current_url(&self) -> String {
        mezon_webview::current_url(self.webview.as_ref(), self.url.as_ref())
    }

    fn copy_url(&mut self, cx: &mut Context<Self>) {
        let url = self.current_url();
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.mark_action_success(TitleBarAction::CopyUrl, cx);
    }

    fn mark_action_success(&mut self, action: TitleBarAction, cx: &mut Context<Self>) {
        self.action_success = Some(action);
        cx.notify();
        self._action_success_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(ACTION_SUCCESS_MS))
                .await;
            this.update(cx, |this, cx| {
                this.action_success = None;
                cx.notify();
            })
            .ok();
        }));
    }

    fn invoke_action(
        &mut self,
        action: TitleBarAction,
        _window: &Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            TitleBarAction::Back => self.go_back(),
            TitleBarAction::Forward => self.go_forward(),
            TitleBarAction::Reload => self.reload(),
            TitleBarAction::CopyUrl => self.copy_url(cx),
        }
    }

    fn release_webview(&mut self, _window: &Window, _cx: &mut Context<Self>) {
        self.webview = None;
        self.webview_init_scheduled = false;
        self.last_webview_size = None;
    }

    fn render_title_button(
        &self,
        action: TitleBarAction,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let succeeded = self.action_success == Some(action);
        let icon = if succeeded {
            IconName::Check
        } else {
            action.icon()
        };
        let icon_color = if succeeded {
            theme.status_online
        } else {
            theme.text_secondary
        };

        div()
            .id(action.id())
            .w(px(NAV_BUTTON_SIZE))
            .h(px(NAV_BUTTON_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .cursor_pointer()
            .rounded(px(6.))
            .hover(|s| s.bg(theme.bg_hover))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _: &gpui::MouseUpEvent, window, cx| {
                    this.invoke_action(action, window, cx);
                    cx.notify();
                }),
            )
            .child(
                Icon::new(icon)
                    .size(px(NAV_ICON_SIZE))
                    .text_color(icon_color),
            )
    }

    fn render_title_controls(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .h_full()
            .flex_shrink_0()
            .when(window_controls::HAS_CUSTOM_TITLE_BAR, |row| {
                row.pl(px(TOOL_BAR_EDGE_SPACE))
            })
            .when(!window_controls::HAS_CUSTOM_TITLE_BAR, |row| {
                row.pr(px(TOOL_BAR_EDGE_SPACE))
            });
        for action in TITLE_BAR_ACTIONS {
            row = row.child(self.render_title_button(action, theme, cx));
        }
        row
    }

    fn render_title_bar(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(mezon_webview::WEBVIEW_TOP_OFFSET as f32))
            .bg(theme.title_bar_bg)
            .child(self.render_title_controls(theme, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .h_full()
                    .when(cfg!(target_os = "windows"), |bar| {
                        bar.window_control_area(gpui::WindowControlArea::Drag)
                    })
                    .when(cfg!(target_os = "linux"), |bar| {
                        bar.on_mouse_down(MouseButton::Left, |event, window, _| {
                            if event.click_count >= 2 {
                                window.zoom_window();
                            } else {
                                window.start_window_move();
                            }
                        })
                    }),
            )
            .when(window_controls::HAS_CUSTOM_TITLE_BAR, |bar| {
                bar.child(window_controls::render_controls(theme, window))
            })
    }

    fn render_app_title_strip(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        window_controls::window_drag_handle(
            div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .h(px(mezon_webview::WEBVIEW_TOP_OFFSET as f32))
                .bg(theme.title_bar_bg)
                .child(div().flex().flex_1().h_full())
                .child(self.render_title_controls(theme, cx)),
        )
    }

    fn render_footer(&self, theme: &Theme) -> impl IntoElement {
        let label = format!("@{}", self.title.to_lowercase());
        div()
            .flex_shrink_0()
            .w_full()
            .h(px(FOOTER_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.title_bar_bg)
            .text_sm()
            .text_color(theme.text_secondary)
            .child(label)
    }
}

impl Render for ChannelAppWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.schedule_webview_init(window, cx);
        self.sync_webview_bounds(window);
        let theme = cx.theme().clone();

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg_tertiary)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseDownEvent, window, cx| {
                    if !this.focus_handle.is_focused(window) {
                        window.focus(&this.focus_handle, cx);
                    }
                }),
            )
            .when(window_controls::HAS_CUSTOM_TITLE_BAR, |el| {
                el.child(self.render_title_bar(&theme, window, cx))
            })
            .when(!window_controls::HAS_CUSTOM_TITLE_BAR, |el| {
                el.child(self.render_app_title_strip(&theme, cx))
            })
            .child(
                div()
                    .id("channel-app-webview-host")
                    .flex_1()
                    .min_h_0()
                    .w_full(),
            )
            .child(self.render_footer(&theme))
            .child(window_controls::render_app_drag_header())
            .when(window_controls::is_edge_resizable(), |el| {
                el.child(window_controls::render_resize_edges(window))
            })
    }
}
