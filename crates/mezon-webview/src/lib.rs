mod tool_bars;

use anyhow::{Context, Result, bail};
use raw_window_handle::HasWindowHandle;
use wry::dpi::{LogicalPosition, LogicalSize, Position, Size};
use wry::{Rect, WebView, WebViewBuilder};

pub use tool_bars::{current_url, go_back, go_forward, reload};
pub use wry::WebView as ChannelAppWebView;

pub const WEBVIEW_TOP_OFFSET: f64 = 36.0;
pub const WEBVIEW_BOTTOM_OFFSET: f64 = 28.0;

pub fn validate_http_url(url: &str) -> Result<()> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        bail!("Only HTTP URLs are allowed")
    }
}

fn webview_bounds(width: f64, height: f64) -> Rect {
    let chrome_v = WEBVIEW_TOP_OFFSET + WEBVIEW_BOTTOM_OFFSET;
    let content_height = (height - chrome_v).max(1.0);
    Rect {
        position: Position::Logical(LogicalPosition::new(0.0, WEBVIEW_TOP_OFFSET)),
        size: Size::Logical(LogicalSize::new(width.max(1.0), content_height)),
    }
}

pub fn create_as_window(
    parent: &impl HasWindowHandle,
    url: &str,
    width: f64,
    height: f64,
) -> Result<WebView> {
    validate_http_url(url)?;
    let bounds = webview_bounds(width, height);
    WebViewBuilder::new()
        .with_url(url)
        .with_bounds(bounds)
        .build_as_child(parent)
        .context("Failed to create channel app webview")
}

pub fn resize_webview(webview: &WebView, width: f64, height: f64) -> Result<()> {
    webview
        .set_bounds(webview_bounds(width, height))
        .context("Failed to resize channel app webview")
}
