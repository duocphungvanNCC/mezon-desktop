use gpui::{App, Window, WindowHandle, div, prelude::*};
use objc::runtime::Object;
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::theme::Theme;

pub fn render_controls(_theme: &Theme) -> impl IntoElement {
    div()
}

pub fn configure_window<V: 'static>(cx: &mut App, handle: WindowHandle<V>) {
    if let Err(error) = cx.update_window(handle.into(), |_, window, cx| {
        window.on_window_should_close(cx, |window, _| {
            if let Some(view) = appkit_view(window) {
                order_out(view);
            }
            false
        });

        if let Some(view) = appkit_view(window) {
            disable_fullscreen(view);
        }
    }) {
        tracing::warn!("Failed to configure window: {error}");
    }
}

fn appkit_view(window: &Window) -> Option<*mut std::ffi::c_void> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    Some(appkit.ns_view.as_ptr())
}

fn with_ns_window(native_view: *mut std::ffi::c_void, f: impl FnOnce(*mut Object)) {
    if native_view.is_null() {
        return;
    }

    unsafe {
        let native_view = native_view.cast::<Object>();
        let ns_window: *mut Object = msg_send![native_view, window];
        if ns_window.is_null() {
            return;
        }
        f(ns_window);
    }
}

fn order_out(native_view: *mut std::ffi::c_void) {
    with_ns_window(native_view, |window| unsafe {
        let _: () = msg_send![window, orderOut: std::ptr::null::<Object>()];
    });
}

fn disable_fullscreen(native_view: *mut std::ffi::c_void) {
    const FULLSCREEN_PRIMARY: u64 = 1 << 7;
    const FULLSCREEN_AUXILIARY: u64 = 1 << 8;
    const FULLSCREEN_NONE: u64 = 1 << 9;
    const FULLSCREEN_STYLE_MASK: u64 = 1 << 14;
    
    with_ns_window(native_view, |window| unsafe {
        let style_mask: u64 = msg_send![window, styleMask];
        if style_mask & FULLSCREEN_STYLE_MASK != 0 {
            let _: () = msg_send![window, toggleFullScreen: std::ptr::null::<Object>()];
        }

        let current: u64 = msg_send![window, collectionBehavior];
        let behavior = (current & !(FULLSCREEN_PRIMARY | FULLSCREEN_AUXILIARY)) | FULLSCREEN_NONE;
        let _: () = msg_send![window, setCollectionBehavior: behavior];
    });
}
