use gpui::{
    AnyWindowHandle, App, AppContext, Bounds, DisplayId, Global, Pixels, Window, WindowBounds,
    WindowHandle, WindowKind, px, size,
};

struct MainWindowHandle(AnyWindowHandle);
impl Global for MainWindowHandle {}

pub fn register_main_window(handle: AnyWindowHandle, cx: &mut App) {
    if cx.try_global::<MainWindowHandle>().is_some() {
        tracing::warn!("register_main_window called more than once");
    }
    cx.set_global(MainWindowHandle(handle));
}

pub fn handle(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<MainWindowHandle>().map(|g| g.0)
}

pub fn main_window_bounds(cx: &mut App) -> Option<Bounds<Pixels>> {
    main_window_placement(cx).map(|(bounds, _)| bounds.get_bounds())
}

pub fn window_placement_for(
    handle: AnyWindowHandle,
    cx: &mut App,
) -> Option<(WindowBounds, Option<DisplayId>)> {
    cx.update_window(handle, |_, window, cx| {
        let display_id = window.display(cx).map(|d| d.id());
        let bounds = window.bounds();
        let window_bounds = if window.is_fullscreen() {
            WindowBounds::Fullscreen(bounds)
        } else if window.is_maximized() {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        };
        (window_bounds, display_id)
    })
    .ok()
}

pub fn main_window_placement(cx: &mut App) -> Option<(WindowBounds, Option<DisplayId>)> {
    let handle = handle(cx)?;
    window_placement_for(handle, cx)
}

pub fn activate_main_window(cx: &mut App) {
    let Some(handle) = handle(cx) else {
        return;
    };
    cx.activate(true);
    let _ = cx.update_window(handle, |_, window, _| window.activate_window());
}

#[cfg(target_os = "linux")]
pub fn uses_wayland_overlay_parent() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        && std::env::var("XDG_SESSION_TYPE")
            .map(|session| session.eq_ignore_ascii_case("wayland"))
            .unwrap_or(true)
}

#[cfg(not(target_os = "linux"))]
pub fn uses_wayland_overlay_parent() -> bool {
    false
}

pub fn overlay_fallback_placement(cx: &mut App) -> (WindowBounds, Option<DisplayId>) {
    (
        WindowBounds::Windowed(Bounds::centered(None, size(px(1100.0), px(740.0)), cx)),
        None,
    )
}

pub fn overlay_placement(
    main_app: AnyWindowHandle,
    cx: &mut App,
) -> (WindowBounds, Option<DisplayId>) {
    window_placement_for(main_app, cx).unwrap_or_else(|| overlay_fallback_placement(cx))
}

pub fn overlay_window_kind_and_parent(
    main_app: AnyWindowHandle,
) -> (WindowKind, Option<AnyWindowHandle>) {
    if uses_wayland_overlay_parent() {
        (WindowKind::Floating, Some(main_app))
    } else {
        (WindowKind::Normal, None)
    }
}

pub fn apply_overlay_bounds(window: &mut Window, bounds: WindowBounds) {
    match bounds {
        WindowBounds::Windowed(bounds) => window.set_bounds(bounds),
        WindowBounds::Maximized(_) => {
            if !window.is_maximized() {
                window.zoom_window();
            }
        }
        WindowBounds::Fullscreen(_) => {
            if !window.is_fullscreen() {
                window.toggle_fullscreen();
            }
        }
    }
}

pub fn sync_overlay_to_main<W: 'static>(
    overlay: WindowHandle<W>,
    main_app: AnyWindowHandle,
    cx: &mut App,
) {
    let Some((main_bounds, _)) = window_placement_for(main_app, cx) else {
        return;
    };
    let _ = overlay.update(cx, |_, window, _| {
        apply_overlay_bounds(window, main_bounds);
    });
}
