use gpui::{AnyWindowHandle, App, AppContext, Bounds, DisplayId, Global, Pixels, WindowBounds};

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
