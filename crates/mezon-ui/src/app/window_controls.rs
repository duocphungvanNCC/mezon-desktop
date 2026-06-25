use gpui::{Div, TitlebarOptions, Window, WindowDecorations, div, prelude::*};

use crate::theme::Theme;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

/// Whether the app paints its own title bar with custom window controls.
/// macOS and Windows use native title bars.
/// Linux uses client-side decorations.
pub const HAS_CUSTOM_TITLE_BAR: bool = cfg!(target_os = "linux");
pub const APP_NAME: &str = "Mezon";

pub const MACOS_TRAFFIC_LIGHT_X: f32 = 12.0;
pub const MACOS_TRAFFIC_LIGHT_Y: f32 = 11.0;
pub const MACOS_TRAFFIC_LIGHT_CLEARANCE: f32 = 34.0;
pub const NAV_ARROW_ICON_SIZE: f32 = 20.0;
pub const NAV_ARROW_BUTTON_PADDING: f32 = 4.0;
pub const APP_HEADER_HEIGHT: f32 = 50.0;

#[cfg(target_os = "macos")]
pub const NAV_TOP_INSET: f32 = MACOS_TRAFFIC_LIGHT_CLEARANCE;
#[cfg(not(target_os = "macos"))]
pub const NAV_TOP_INSET: f32 = 0.0;

/// Render the platform window controls (minimize / maximize / close).
/// Returns an empty element on macOS, where the native traffic lights are used.
pub fn render_controls(theme: &Theme, window: &Window) -> impl IntoElement {
    platform::render_controls(theme, window)
}

pub fn window_title_options() -> TitlebarOptions {
    #[cfg(target_os = "macos")]
    {
        use gpui::{point, px};

        TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(
                px(MACOS_TRAFFIC_LIGHT_X),
                px(MACOS_TRAFFIC_LIGHT_Y),
            )),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        TitlebarOptions {
            title: Some(APP_NAME.into()),
            appears_transparent: cfg!(not(target_os = "windows")),
            ..Default::default()
        }
    }
}

pub fn main_window_decorations() -> Option<WindowDecorations> {
    #[cfg(target_os = "linux")]
    {
        Some(WindowDecorations::Client)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Invisible full-width strip at the top of the window for macOS window dragging
pub fn render_app_drag_header() -> impl IntoElement {
    #[cfg(target_os = "macos")]
    {
        use gpui::px;

        window_drag_handle(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(APP_HEADER_HEIGHT)),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        div().hidden()
    }
}

pub fn window_drag_handle(header: Div) -> Div {
    #[cfg(target_os = "macos")]
    {
        use gpui::MouseButton;
        header.on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count >= 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        header
    }
}
