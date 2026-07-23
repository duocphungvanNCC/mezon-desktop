use std::cell::RefCell;
use std::ffi::c_ulong;
use std::mem;
use std::sync::Once;

use anyhow::{Context, Result, bail};
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, WindowHandle, XcbWindowHandle, XlibWindowHandle,
};
use webkit2gtk::WebViewExt;
use wry::{WebContext, WebViewBuilder, WebViewExtUnix};

use crate::webview::ChannelAppWebView;

static GTK_INIT: Once = Once::new();

thread_local! {
    static SHARED_WEB_CONTEXT: RefCell<Option<WebContext>> = RefCell::new(None);
}

pub fn with_shared_web_context<R>(f: impl FnOnce(&mut WebContext) -> Result<R>) -> Result<R> {
    init_gtk()?;
    SHARED_WEB_CONTEXT.with(|cell| {
        let mut context = cell.borrow_mut();
        if context.is_none() {
            *context = Some(WebContext::new(None));
        }
        f(context.as_mut().expect("shared web context initialized"))
    })
}

pub fn init_gtk() -> Result<()> {
    GTK_INIT.call_once(|| unsafe {
        std::env::set_var("GDK_BACKEND", "x11");
    });
    if gtk::is_initialized() {
        return Ok(());
    }
    gtk::init().context("gtk init failed")?;
    Ok(())
}

pub fn pump_gtk_events() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}

pub fn destroy_webview(webview: ChannelAppWebView) {
    use gtk::prelude::WidgetExtManual;

    let gtk_webview = webview.webview();
    gtk_webview.try_close();
    if let Err(error) = webview.set_visible(false) {
        tracing::warn!("channel app webview hide failed: {error:#}");
    }
    pump_gtk_events();

    let gtk_window = wry_gtk_window(&gtk_webview);
    unsafe {
        gtk_webview.destroy();
    }
    pump_gtk_events();

    if let Some(gtk_window) = gtk_window {
        unsafe {
            gtk_window.destroy();
        }
        pump_gtk_events();
    }

    sync_x11_display();
    mem::forget(webview);
}

fn wry_gtk_window(gtk_webview: &webkit2gtk::WebView) -> Option<gtk::Window> {
    use gtk::prelude::{Cast, WidgetExt};

    let mut widget = gtk_webview.clone().upcast::<gtk::Widget>();
    while let Some(parent) = widget.parent() {
        if let Ok(window) = parent.clone().downcast::<gtk::Window>() {
            return Some(window);
        }
        widget = parent;
    }
    None
}

fn sync_x11_display() {
    use gdkx11::X11Display;
    use gtk::glib::object::{Cast, ObjectType};

    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let Some(x11_display) = display.downcast_ref::<X11Display>() else {
        return;
    };
    unsafe {
        let xdisplay = gdkx11::ffi::gdk_x11_display_get_xdisplay(x11_display.as_ptr());
        if xdisplay.is_null() {
            return;
        }
        if let Ok(xlib) = x11_dl::xlib::Xlib::open() {
            (xlib.XSync)(xdisplay as _, x11_dl::xlib::False);
        }
    }
}

pub fn create(
    parent: &impl HasWindowHandle,
    builder: WebViewBuilder<'_>,
) -> Result<ChannelAppWebView> {
    init_gtk()?;
    pump_gtk_events();

    let handle = parent.window_handle()?.as_raw();
    match handle {
        RawWindowHandle::Xcb(xcb) => create_x11_child(&xcb, builder),
        RawWindowHandle::Xlib(_) => builder
            .build_as_child(parent)
            .map(ChannelAppWebView::new)
            .context("Failed to create channel app webview"),
        RawWindowHandle::Wayland(_) => bail!(
            "Channel app webviews require the X11 backend on Linux. \
             Restart with DISPLAY set and MEZON_LINUX_BACKEND=x11, or unset WAYLAND_DISPLAY."
        ),
        _ => bail!("Failed to create channel app webview: the window handle kind is not supported"),
    }
}

fn create_x11_child(
    xcb: &XcbWindowHandle,
    builder: WebViewBuilder<'_>,
) -> Result<ChannelAppWebView> {
    let xlib_parent = XlibParentWindow::from_xcb(xcb);
    builder
        .build_as_child(&xlib_parent)
        .map(ChannelAppWebView::new)
        .context("Failed to create channel app webview")
}

struct XlibParentWindow {
    handle: XlibWindowHandle,
}

impl XlibParentWindow {
    fn from_xcb(xcb: &XcbWindowHandle) -> Self {
        let mut handle = XlibWindowHandle::new(xcb.window.get() as c_ulong);
        if let Some(visual_id) = xcb.visual_id {
            handle.visual_id = visual_id.get() as c_ulong;
        }
        Self { handle }
    }
}

impl HasWindowHandle for XlibParentWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Xlib(self.handle)) })
    }
}
