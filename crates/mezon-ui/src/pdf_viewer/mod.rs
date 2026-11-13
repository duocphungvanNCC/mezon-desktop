mod layout;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use gpui::http_client::HttpClient;
use gpui::{
    App, AppContext, Context, DisplayId, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton,
    MouseDownEvent, Render, RenderImage, ScrollHandle, ScrollWheelEvent, SharedString,
    Subscription, Task, Window, WindowBounds, WindowHandle, WindowOptions, div, img, point,
    prelude::*, px, size,
};
use mezon_pdf::{PdfDocument, fit_page_pixels};
use mezon_store::{Settings, open_media_url_external};

use crate::app::main_window::{
    activate_main_window, apply_overlay_bounds, handle as main_window_handle,
    overlay_placement_from_window, overlay_window_kind, sync_overlay_to_main,
};
use crate::app::title_bar::TitleBar;
use crate::app::window_controls;
use crate::components::primitives::{Icon, IconName, Spinner};
use crate::theme::{ActiveTheme, Theme};
use layout::{PageLayout, page_aspect};

const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 3.0;
const ZOOM_STEP: f32 = 0.2;
const BASE_PAGE_WIDTH: f32 = 800.0;
const PAGE_PADDING: f32 = 16.0;
const PAGE_GAP: f32 = 16.0;
const PREFETCH_PAGES: usize = 1;
const KEY_SCROLL_LINES: f32 = 3.0;
const PDF_FETCH_MAX_BYTES: usize = 64 * 1024 * 1024;

pub struct OpenPdfRequest {
    pub url: SharedString,
    pub filename: SharedString,
    pub settings: Entity<Settings>,
}

struct GlobalPdfViewer(WindowHandle<PdfViewer>);
impl gpui::Global for GlobalPdfViewer {}

fn clear_pdf_viewer_global(cx: &mut App) {
    if cx.try_global::<GlobalPdfViewer>().is_some() {
        cx.remove_global::<GlobalPdfViewer>();
    }
}

pub fn close_pdf_viewer(cx: &mut App) {
    let Some(handle) = cx.try_global::<GlobalPdfViewer>().map(|g| g.0) else {
        return;
    };
    let _ = handle.update(cx, |viewer, window, cx| {
        viewer.release_resources(Some(window), cx);
        window.remove_window();
    });
    clear_pdf_viewer_global(cx);
}

pub fn open_pdf_viewer(request: OpenPdfRequest, window: &Window, cx: &mut App) {
    if request.url.is_empty() {
        return;
    }
    // A machine with no renderer has nothing to show, and a window that opens only to
    // say so is worse than no window: the click hands the document to whatever the OS
    // opens pdfs with - the browser, on every platform we ship - and stays out of the way.
    if !mezon_pdf::is_supported() {
        if let Some(reason) = mezon_pdf::unavailable_reason() {
            tracing::warn!("pdf viewer unavailable, opening externally: {reason}");
        }
        open_externally(&request.url, cx);
        return;
    }
    let placement = overlay_placement_from_window(window, cx);
    let mut pending = Some(request);
    if let Some(handle) = cx.try_global::<GlobalPdfViewer>().map(|g| g.0) {
        let _ = handle.update(cx, |viewer, window, cx| {
            if let Some(request) = pending.take() {
                apply_overlay_bounds(window, placement.0);
                window.activate_window();
                window.focus(&viewer.focus_handle, cx);
                viewer.set_request(request, window, cx);
            }
        });
        if pending.is_none() {
            return;
        }
        clear_pdf_viewer_global(cx);
    }
    let Some(request) = pending else {
        return;
    };
    spawn_pdf_viewer_window(request, placement, cx);
}

fn spawn_pdf_viewer_window(
    request: OpenPdfRequest,
    placement: (WindowBounds, Option<DisplayId>),
    cx: &mut App,
) {
    let main_app = main_window_handle(cx);
    let (window_bounds, display_id) = placement;
    let options = WindowOptions {
        window_bounds: Some(window_bounds),
        window_min_size: Some(size(px(600.0), px(480.0))),
        kind: overlay_window_kind(),
        focus: true,
        show: true,
        is_movable: true,
        display_id,
        titlebar: Some(window_controls::window_title_options()),
        window_decorations: window_controls::main_window_decorations(),
        app_id: window_controls::linux_app_id(),
        ..Default::default()
    };

    match cx.open_window(options, |window, cx| {
        cx.new(|cx| PdfViewer::new(request, window, cx))
    }) {
        Ok(handle) => {
            #[cfg(target_os = "macos")]
            if let Err(error) = handle.update(cx, |_, window, _| {
                window_controls::macos::disable_window_fullscreen(window);
            }) {
                tracing::warn!("Failed to configure pdf viewer window: {error}");
            }
            cx.set_global(GlobalPdfViewer(handle));
            let _ = handle.update(cx, |viewer, window, cx| {
                window.activate_window();
                window.focus(&viewer.focus_handle, cx);
            });
            if let Some(main_app) = main_app {
                cx.defer(move |cx| {
                    sync_overlay_to_main(handle, main_app, cx);
                });
            }
        }
        Err(error) => tracing::error!("failed to open pdf viewer window: {error}"),
    }
}

enum LoadState {
    Loading,
    Ready,
    Failed {
        message: SharedString,
        detail: Option<SharedString>,
    },
}

struct RenderedPage {
    image: Arc<RenderImage>,
    device_width: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyCommand {
    Close,
    PreviousPage,
    NextPage,
    FirstPage,
    LastPage,
    LineUp,
    LineDown,
    ScreenUp,
    ScreenDown,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

fn key_command(key: &str, shift: bool) -> Option<KeyCommand> {
    let command = match key {
        "escape" => KeyCommand::Close,
        "left" => KeyCommand::PreviousPage,
        "right" => KeyCommand::NextPage,
        "home" => KeyCommand::FirstPage,
        "end" => KeyCommand::LastPage,
        "up" => KeyCommand::LineUp,
        "down" => KeyCommand::LineDown,
        "pageup" => KeyCommand::ScreenUp,
        "pagedown" => KeyCommand::ScreenDown,
        "space" if shift => KeyCommand::ScreenUp,
        "space" => KeyCommand::ScreenDown,
        "+" | "=" => KeyCommand::ZoomIn,
        "-" => KeyCommand::ZoomOut,
        "0" => KeyCommand::ZoomReset,
        _ => return None,
    };
    Some(command)
}

fn retain_range(visible: &Range<usize>, page_count: usize) -> Range<usize> {
    let end = visible.end.saturating_add(PREFETCH_PAGES).min(page_count);
    visible.start.saturating_sub(PREFETCH_PAGES).min(end)..end
}

fn render_order(visible: &Range<usize>, retain: &Range<usize>) -> impl Iterator<Item = usize> {
    visible
        .clone()
        .chain(visible.end..retain.end)
        .chain((retain.start..visible.start).rev())
}

pub struct PdfViewer {
    focus_handle: FocusHandle,
    title_bar: Entity<TitleBar>,
    settings: Entity<Settings>,
    url: SharedString,
    filename: SharedString,
    document: Option<Arc<PdfDocument>>,
    layout: Option<PageLayout>,
    current_page: usize,
    zoom: f32,
    state: LoadState,
    pages: HashMap<usize, RenderedPage>,
    failed_pages: HashSet<usize>,
    render_in_flight: Option<usize>,
    scroll: ScrollHandle,
    sync_scheduled: bool,
    follow_scroll_pending: bool,
    closing: bool,
    _load_task: Option<Task<()>>,
    _render_task: Option<Task<()>>,
    _release: Subscription,
    _window_bounds: Subscription,
}

impl PdfViewer {
    fn new(request: OpenPdfRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, app| {
            let _ = weak.update(app, |viewer, cx| {
                viewer.release_resources(Some(window), cx);
            });
            clear_pdf_viewer_global(app);
            activate_main_window(app);
            true
        });
        let release = cx.on_release(|viewer, cx| {
            viewer.clear_pages(None, cx);
        });
        let window_bounds = cx.observe_window_bounds(window, |this, window, cx| {
            this.schedule_sync(false, window, cx);
        });
        let title_bar = cx.new(|cx| TitleBar::new(request.settings.clone(), cx));
        let mut this = Self {
            focus_handle,
            title_bar,
            settings: request.settings.clone(),
            url: request.url.clone(),
            filename: request.filename.clone(),
            document: None,
            layout: None,
            current_page: 0,
            zoom: 1.0,
            state: LoadState::Loading,
            pages: HashMap::new(),
            failed_pages: HashSet::new(),
            render_in_flight: None,
            scroll: ScrollHandle::new(),
            sync_scheduled: false,
            follow_scroll_pending: false,
            closing: false,
            _load_task: None,
            _render_task: None,
            _release: release,
            _window_bounds: window_bounds,
        };
        this.start_load(window, cx);
        this
    }

    fn set_request(
        &mut self,
        request: OpenPdfRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.url == request.url {
            return;
        }
        self.url = request.url;
        self.filename = request.filename;
        self.settings = request.settings;
        self.document = None;
        self.layout = None;
        self.current_page = 0;
        self.zoom = 1.0;
        self.render_in_flight = None;
        self._render_task = None;
        self.failed_pages.clear();
        self.follow_scroll_pending = false;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.clear_pages(Some(window), cx);
        self.start_load(window, cx);
    }

    fn locale(&self, cx: &App) -> String {
        self.settings.read(cx).language.clone()
    }

    fn start_load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state = LoadState::Loading;
        let url = self.url.to_string();
        let client = cx.http_client();
        self._load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let bytes = match fetch_pdf(client, url).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    let reason = error.to_string();
                    tracing::warn!("pdf download failed: {reason}");
                    let _ = this.update(cx, move |this, cx| this.fail(reason, cx));
                    return;
                }
            };
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let document = PdfDocument::from_bytes(bytes)?;
                    let aspects = (0..document.page_count())
                        .map(|index| document.page_size(index).map(|page| page_aspect(&page)))
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    anyhow::Ok((document, aspects))
                })
                .await;
            let (document, aspects) = match opened {
                Ok((document, aspects)) => (Arc::new(document), aspects),
                Err(error) => {
                    let reason = error.to_string();
                    tracing::warn!("pdf open failed: {reason}");
                    let _ = this.update(cx, move |this, cx| this.fail(reason, cx));
                    return;
                }
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.layout = Some(PageLayout::new(
                    aspects,
                    BASE_PAGE_WIDTH * this.zoom,
                    PAGE_PADDING,
                    PAGE_GAP,
                ));
                this.document = Some(document);
                this.state = LoadState::Ready;
                this.sync_pages(window, cx);
                this.schedule_sync(false, window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The reason rides along under the message. Without it a failed document looks
    /// the same whether the download 404'd, the bytes were not a pdf, or the platform
    /// renderer refused the page — and a report from another OS cannot be acted on.
    fn fail(&mut self, reason: String, cx: &mut Context<Self>) {
        self.state = LoadState::Failed {
            message: SharedString::from(mezon_i18n::t(&self.locale(cx), "media.pdf.loadFailed")),
            detail: Some(SharedString::from(reason)),
        };
        cx.notify();
    }

    fn viewport_height(&self, window: &Window) -> f32 {
        let height = f32::from(self.scroll.bounds().size.height);
        if height > 0.0 {
            height
        } else {
            f32::from(window.viewport_size().height)
        }
    }

    fn viewport_width(&self, window: &Window) -> f32 {
        let width = f32::from(self.scroll.bounds().size.width);
        if width > 0.0 {
            width
        } else {
            f32::from(window.viewport_size().width)
        }
    }

    fn scroll_top(&self) -> f32 {
        (-f32::from(self.scroll.offset().y)).max(0.0)
    }

    fn scroll_left(&self) -> f32 {
        (-f32::from(self.scroll.offset().x)).max(0.0)
    }

    fn set_scroll(&self, left: f32, top: f32) {
        self.scroll.set_offset(point(px(-left), px(-top)));
    }

    fn device_page_width(&self, window: &Window) -> u32 {
        let scale_factor = window.scale_factor().max(1.0);
        let logical_width = (BASE_PAGE_WIDTH * self.zoom).max(1.0);
        (logical_width * scale_factor).round().max(1.0) as u32
    }

    fn schedule_sync(&mut self, follow_scroll: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_scroll_pending |= follow_scroll;
        if self.sync_scheduled {
            return;
        }
        self.sync_scheduled = true;
        cx.on_next_frame(window, |this, window, cx| {
            this.sync_scheduled = false;
            if std::mem::take(&mut this.follow_scroll_pending) {
                this.follow_scroll(window, cx);
            }
            this.sync_pages(window, cx);
        });
    }

    fn follow_scroll(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let page = layout.current_page(self.scroll_top(), self.viewport_height(window));
        if page != self.current_page {
            self.current_page = page;
            cx.notify();
        }
    }

    fn sync_pages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        let (Some(document), Some(layout)) = (self.document.clone(), self.layout.as_ref()) else {
            return;
        };
        let page_count = layout.page_count();
        let visible = layout.visible_range(self.scroll_top(), self.viewport_height(window));
        let retain = retain_range(&visible, page_count);
        let evicted: Vec<usize> = self
            .pages
            .keys()
            .copied()
            .filter(|index| !retain.contains(index))
            .collect();
        for index in evicted {
            if let Some(page) = self.pages.remove(&index) {
                cx.drop_image(page.image, Some(window));
            }
        }
        if self.render_in_flight.is_some() {
            return;
        }
        let device_width = self.device_page_width(window);
        let next = render_order(&visible, &retain).find(|index| {
            !self.failed_pages.contains(index)
                && self
                    .pages
                    .get(index)
                    .is_none_or(|page| page.device_width != device_width)
        });
        if let Some(index) = next {
            self.rasterize_page(document, index, device_width, window, cx);
        }
    }

    fn rasterize_page(
        &mut self,
        document: Arc<PdfDocument>,
        index: usize,
        device_width: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.render_in_flight = Some(index);
        self._render_task = Some(cx.spawn_in(window, async move |this, cx| {
            let rendered = cx
                .background_executor()
                .spawn(async move {
                    let page = document.page_size(index)?;
                    let (width, height) = fit_page_pixels(&page, device_width as f32);
                    let bitmap = document.render_page(index, width, height)?;
                    let (width, height) = (bitmap.width, bitmap.height);
                    image::RgbaImage::from_raw(width, height, bitmap.bgra).ok_or_else(|| {
                        anyhow::anyhow!("page bitmap does not fill {width}x{height}")
                    })
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.render_in_flight = None;
                match rendered {
                    Ok(buffer) => {
                        let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
                        this.store_page(index, device_width, image, window, cx);
                    }
                    Err(error) => {
                        tracing::warn!("pdf page {index} render failed: {error}");
                        this.failed_pages.insert(index);
                        cx.notify();
                    }
                }
                this.sync_pages(window, cx);
            });
        }));
    }

    fn store_page(
        &mut self,
        index: usize,
        device_width: u32,
        image: Arc<RenderImage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let visible = layout.visible_range(self.scroll_top(), self.viewport_height(window));
        if !retain_range(&visible, layout.page_count()).contains(&index) {
            return;
        }
        let page = RenderedPage {
            image,
            device_width,
        };
        if let Some(previous) = self.pages.insert(index, page) {
            cx.drop_image(previous.image, Some(window));
        }
        cx.notify();
    }

    fn clear_pages(&mut self, mut window: Option<&mut Window>, cx: &mut App) {
        for (_, page) in self.pages.drain() {
            match window.as_deref_mut() {
                Some(window) => cx.drop_image(page.image, Some(window)),
                None => crate::image_cache::queue_atlas_drop(cx, page.image),
            }
        }
    }

    fn release_resources(&mut self, window: Option<&mut Window>, cx: &mut App) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.render_in_flight = None;
        self._load_task = None;
        self._render_task = None;
        self.document = None;
        self.clear_pages(window, cx);
        crate::image_cache::release_freed_memory_to_os(cx);
    }

    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release_resources(Some(window), cx);
        clear_pdf_viewer_global(cx);
        activate_main_window(cx);
        window.remove_window();
    }

    fn page_count(&self) -> usize {
        self.layout.as_ref().map_or(0, PageLayout::page_count)
    }

    fn go_to_page(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        if layout.page_count() == 0 {
            return;
        }
        let index = index.min(layout.page_count() - 1);
        let top = layout.clamp_scroll_top(
            layout.scroll_top_for_page(index),
            self.viewport_height(window),
        );
        self.current_page = index;
        self.set_scroll(self.scroll_left(), top);
        self.sync_pages(window, cx);
        cx.notify();
    }

    fn prev_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_page > 0 {
            self.go_to_page(self.current_page - 1, window, cx);
        }
    }

    fn next_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_page + 1 < self.page_count() {
            self.go_to_page(self.current_page + 1, window, cx);
        }
    }

    fn scroll_by(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let viewport_height = self.viewport_height(window);
        let top = layout.clamp_scroll_top(self.scroll_top() + delta, viewport_height);
        self.current_page = layout.current_page(top, viewport_height);
        self.set_scroll(self.scroll_left(), top);
        self.sync_pages(window, cx);
        cx.notify();
    }

    fn line_scroll_step(window: &Window) -> f32 {
        f32::from(window.line_height()) * KEY_SCROLL_LINES
    }

    fn screen_scroll_step(&self, window: &Window) -> f32 {
        let line = f32::from(window.line_height());
        (self.viewport_height(window) - line).max(line)
    }

    fn set_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (zoom - self.zoom).abs() < f32::EPSILON {
            return;
        }
        self.zoom = zoom;
        let viewport_width = self.viewport_width(window);
        let viewport_height = self.viewport_height(window);
        let (scroll_left, scroll_top) = (self.scroll_left(), self.scroll_top());
        if let Some(layout) = self.layout.as_mut() {
            let anchor = layout.anchor_at(scroll_top);
            let previous_width = layout.content_width();
            layout.set_page_width(BASE_PAGE_WIDTH * zoom);
            let top =
                layout.clamp_scroll_top(layout.scroll_top_for_anchor(anchor), viewport_height);
            let center = (scroll_left + viewport_width / 2.0) * layout.content_width()
                / previous_width.max(1.0);
            let max_left = (layout.content_width() - viewport_width).max(0.0);
            let left = (center - viewport_width / 2.0).clamp(0.0, max_left);
            self.set_scroll(left, top);
        }
        self.sync_pages(window, cx);
        cx.notify();
    }

    fn download(&self, cx: &mut App) {
        crate::util::download::save_with_progress_toast(
            self.url.clone(),
            self.filename.clone(),
            cx,
        );
    }

    fn open_externally(&self, cx: &mut App) {
        open_externally(&self.url, cx);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let Some(command) = key_command(keystroke.key.as_str(), keystroke.modifiers.shift) else {
            return;
        };
        match command {
            KeyCommand::Close => self.close_window(window, cx),
            KeyCommand::PreviousPage => self.prev_page(window, cx),
            KeyCommand::NextPage => self.next_page(window, cx),
            KeyCommand::FirstPage => self.go_to_page(0, window, cx),
            KeyCommand::LastPage => {
                self.go_to_page(self.page_count().saturating_sub(1), window, cx)
            }
            KeyCommand::LineUp => self.scroll_by(-Self::line_scroll_step(window), window, cx),
            KeyCommand::LineDown => self.scroll_by(Self::line_scroll_step(window), window, cx),
            KeyCommand::ScreenUp => self.scroll_by(-self.screen_scroll_step(window), window, cx),
            KeyCommand::ScreenDown => self.scroll_by(self.screen_scroll_step(window), window, cx),
            KeyCommand::ZoomIn => self.set_zoom(self.zoom + ZOOM_STEP, window, cx),
            KeyCommand::ZoomOut => self.set_zoom(self.zoom - ZOOM_STEP, window, cx),
            KeyCommand::ZoomReset => self.set_zoom(1.0, window, cx),
        }
    }
}

fn open_externally(url: &str, cx: &mut App) {
    open_media_url_external(url.to_string(), cx);
}

async fn fetch_pdf(client: Arc<dyn HttpClient>, url: String) -> anyhow::Result<Vec<u8>> {
    if !url.starts_with("https://") {
        anyhow::bail!("pdf fetch rejected: only https scheme is allowed");
    }
    let mut response = client.get(&url, ().into(), true).await?;
    if !response.status().is_success() {
        anyhow::bail!("pdf fetch failed with status {}", response.status());
    }
    let bytes = crate::image_cache::read_body_limited(&mut response, PDF_FETCH_MAX_BYTES).await?;
    Ok(bytes)
}

impl Focusable for PdfViewer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PdfViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.locale(cx);

        div()
            .relative()
            .track_focus(&self.focus_handle)
            .key_context("PdfViewer")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg_tertiary)
            .text_color(theme.text_primary)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    if !this.focus_handle.is_focused(window) {
                        window.focus(&this.focus_handle, cx);
                    }
                }),
            )
            .when(window_controls::HAS_CUSTOM_TITLE_BAR, |el| {
                el.child(self.title_bar.clone())
            })
            .when(!window_controls::HAS_CUSTOM_TITLE_BAR, |el| {
                el.child(window_controls::window_drag_handle(
                    div().flex_shrink_0().h_8().w_full().bg(theme.title_bar_bg),
                ))
            })
            .child(self.render_header(&theme, cx))
            .child(self.render_controls(&theme, cx))
            .child(self.render_page(&theme, &locale, window, cx))
            .child(self.render_footer(&theme, &locale))
            .child(window_controls::render_app_drag_header())
            .when(window_controls::is_edge_resizable(), |el| {
                el.child(window_controls::render_resize_edges(window))
            })
    }
}

impl PdfViewer {
    fn render_header(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .h(px(40.))
            .w_full()
            .px_4()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .bg(theme.bg_secondary)
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.))
                    .child(self.filename.clone()),
            )
            .child(div().flex().flex_row().items_center().gap_1().child(
                tool_button("pdf-download", IconName::Download, theme).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| this.download(cx)),
                ),
            ))
    }

    fn render_controls(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let page_count = self.page_count().max(1);
        let counter = SharedString::from(format!("{} / {}", self.current_page + 1, page_count));
        let zoom_label = SharedString::from(format!("{}%", (self.zoom * 100.0).round() as i32));
        let at_first = self.current_page == 0;
        let at_last = self.current_page + 1 >= page_count;

        div()
            .flex_shrink_0()
            .h(px(44.))
            .w_full()
            .px_4()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .bg(theme.bg_secondary)
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        tool_button("pdf-prev", IconName::ChevronLeft, theme)
                            .when(at_first, |el| el.opacity(0.5))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                    this.prev_page(window, cx)
                                }),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme.text_secondary)
                            .child(counter),
                    )
                    .child(
                        tool_button("pdf-next", IconName::ChevronRight, theme)
                            .when(at_last, |el| el.opacity(0.5))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                    this.next_page(window, cx)
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        tool_button("pdf-zoom-out", IconName::MinusCircle, theme).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.set_zoom(this.zoom - ZOOM_STEP, window, cx)
                            }),
                        ),
                    )
                    .child(
                        div()
                            .id("pdf-zoom-reset")
                            .min_w(px(56.))
                            .text_size(px(13.))
                            .text_color(theme.text_secondary)
                            .cursor_pointer()
                            .flex()
                            .justify_center()
                            .child(zoom_label)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                    this.set_zoom(1.0, window, cx)
                                }),
                            ),
                    )
                    .child(
                        tool_button("pdf-zoom-in", IconName::Plus, theme).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.set_zoom(this.zoom + ZOOM_STEP, window, cx)
                            }),
                        ),
                    ),
            )
    }

    fn render_page(
        &self,
        theme: &Theme,
        locale: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let body = match &self.state {
            LoadState::Failed { message, detail } => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    Icon::new(IconName::Warning)
                        .size(px(32.))
                        .text_color(theme.danger_text),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(theme.danger_text)
                        .child(message.clone()),
                )
                .when_some(detail.clone(), |el, detail| {
                    el.child(
                        div()
                            .max_w(px(520.))
                            .text_size(px(12.))
                            .text_color(theme.text_muted)
                            .child(detail),
                    )
                })
                .child(
                    div()
                        .id("pdf-open-external")
                        .px_3()
                        .py_1()
                        .rounded(px(6.))
                        .bg(theme.bg_hover)
                        .cursor_pointer()
                        .text_size(px(13.))
                        .child(SharedString::from(mezon_i18n::t(
                            locale,
                            "media.pdf.openExternally",
                        )))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _, cx| this.open_externally(cx)),
                        ),
                )
                .into_any_element(),
            _ => match self.layout.as_ref() {
                Some(layout) => self.render_pages(layout, theme, window).into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Spinner::new())
                    .into_any_element(),
            },
        };

        div()
            .id("pdf-page-scroll")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(cx.listener(|this, _: &ScrollWheelEvent, window, cx| {
                this.schedule_sync(true, window, cx);
            }))
            .bg(theme.bg_tertiary)
            .child(body)
    }

    fn render_pages(
        &self,
        layout: &PageLayout,
        theme: &Theme,
        window: &Window,
    ) -> impl IntoElement {
        let page_count = layout.page_count();
        let window_height = f32::from(window.viewport_size().height);
        let visible = layout.visible_range(self.scroll_top(), window_height);
        let drawn = retain_range(&visible, page_count);
        let leading_space = layout.page_top(drawn.start) - layout.padding();
        let trailing_space = if drawn.end < page_count {
            layout.content_height() - layout.padding() - layout.page_top(drawn.end)
        } else {
            0.0
        };

        div()
            .min_w_full()
            .w(px(layout.content_width()))
            .flex()
            .flex_col()
            .items_center()
            .p(px(layout.padding()))
            .when(leading_space > 0.0, |el| {
                el.child(div().flex_shrink_0().h(px(leading_space)))
            })
            .children(drawn.map(|index| {
                self.render_page_slot(index, layout, theme)
                    .when(index + 1 < page_count, |el| el.mb(px(layout.gap())))
            }))
            .when(trailing_space > 0.0, |el| {
                el.child(div().flex_shrink_0().h(px(trailing_space)))
            })
    }

    fn render_page_slot(&self, index: usize, layout: &PageLayout, theme: &Theme) -> gpui::Div {
        let width = px(layout.page_width());
        let height = px(layout.page_height(index));
        let slot = div()
            .flex_shrink_0()
            .w(width)
            .h(height)
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.bg_secondary);
        match self.pages.get(&index) {
            Some(page) => slot.child(img(page.image.clone()).w(width).h(height)),
            None if self.failed_pages.contains(&index) => slot.child(
                Icon::new(IconName::Warning)
                    .size(px(32.))
                    .text_color(theme.danger_text),
            ),
            None => slot.child(Spinner::new()),
        }
    }

    fn render_footer(&self, theme: &Theme, locale: &str) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .h(px(28.))
            .w_full()
            .px_4()
            .flex()
            .items_center()
            .bg(theme.bg_secondary)
            .border_t_1()
            .border_color(theme.border)
            .text_size(px(11.))
            .text_color(theme.text_muted)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(mezon_i18n::t(
                        locale,
                        "media.pdf.shortcutHint",
                    ))),
            )
    }
}

fn tool_button(id: &'static str, icon: IconName, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|el| el.bg(theme.bg_hover))
        .child(
            Icon::new(icon)
                .size(px(18.))
                .text_color(theme.text_secondary),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_shortcuts_keep_their_meaning() {
        assert_eq!(key_command("escape", false), Some(KeyCommand::Close));
        assert_eq!(key_command("left", false), Some(KeyCommand::PreviousPage));
        assert_eq!(key_command("right", false), Some(KeyCommand::NextPage));
        assert_eq!(key_command("+", false), Some(KeyCommand::ZoomIn));
        assert_eq!(key_command("=", false), Some(KeyCommand::ZoomIn));
        assert_eq!(key_command("-", false), Some(KeyCommand::ZoomOut));
        assert_eq!(key_command("0", false), Some(KeyCommand::ZoomReset));
    }

    #[test]
    fn scroll_and_jump_shortcuts() {
        assert_eq!(key_command("up", false), Some(KeyCommand::LineUp));
        assert_eq!(key_command("down", false), Some(KeyCommand::LineDown));
        assert_eq!(key_command("pageup", false), Some(KeyCommand::ScreenUp));
        assert_eq!(key_command("pagedown", false), Some(KeyCommand::ScreenDown));
        assert_eq!(key_command("space", false), Some(KeyCommand::ScreenDown));
        assert_eq!(key_command("space", true), Some(KeyCommand::ScreenUp));
        assert_eq!(key_command("home", false), Some(KeyCommand::FirstPage));
        assert_eq!(key_command("end", false), Some(KeyCommand::LastPage));
        assert_eq!(key_command("a", false), None);
    }

    #[test]
    fn retain_range_adds_prefetch_within_bounds() {
        assert_eq!(retain_range(&(0..1), 5), 0..1 + PREFETCH_PAGES);
        assert_eq!(
            retain_range(&(2..3), 5),
            2 - PREFETCH_PAGES..3 + PREFETCH_PAGES
        );
        assert_eq!(retain_range(&(4..5), 5), 4 - PREFETCH_PAGES..5);
        assert_eq!(retain_range(&(0..1), 1), 0..1);
        assert_eq!(retain_range(&(0..0), 0), 0..0);
    }

    #[test]
    fn render_order_puts_visible_pages_first() {
        let visible = 3..5;
        let retain = 2..6;
        let order: Vec<usize> = render_order(&visible, &retain).collect();
        assert_eq!(order, vec![3, 4, 5, 2]);
    }

    #[test]
    fn rendered_pages_stay_bounded_while_scrolling_through_document() {
        let layout = PageLayout::new(vec![1.25; 200], BASE_PAGE_WIDTH, PAGE_PADDING, PAGE_GAP);
        let viewport = 900.0;
        let max_top = layout.max_scroll_top(viewport);
        let mut top = 0.0;
        while top <= max_top {
            let visible = layout.visible_range(top, viewport);
            let retain = retain_range(&visible, layout.page_count());
            assert!(retain.len() <= visible.len() + 2 * PREFETCH_PAGES);
            assert!(retain.contains(&layout.current_page(top, viewport)));
            top += viewport / 3.0;
        }
    }
}
