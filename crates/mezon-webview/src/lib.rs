mod tool_bars;

use anyhow::{Context, Result, bail};
use raw_window_handle::HasWindowHandle;
use url::Url;
use wry::dpi::{LogicalPosition, LogicalSize, Position, Size};
use wry::{Rect, WebView, WebViewBuilder};

pub use tool_bars::{current_url, go_back, go_forward, reload};
pub use wry::WebView as ChannelAppWebView;

pub const WEBVIEW_TOP_OFFSET: f64 = 36.0;
pub const WEBVIEW_BOTTOM_OFFSET: f64 = 29.0;

const MEDIA_DENY_SCRIPT: &str = r#"
(function () {
  if (!navigator.mediaDevices) {
    return;
  }
  const denied = function () {
    return Promise.reject(new DOMException("Permission denied", "NotAllowedError"));
  };
  try {
    Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
      configurable: false,
      value: denied,
    });
  } catch (_) {
    navigator.mediaDevices.getUserMedia = denied;
  }
})();
"#;

pub fn validate_http_url(url: &str) -> Result<()> {
    if url.starts_with("https://") {
        return Ok(());
    }
    #[cfg(debug_assertions)]
    if is_debug_local_http(url) {
        return Ok(());
    }
    if url.starts_with("http://") {
        bail!("HTTP URLs are not allowed; use HTTPS");
    }
    bail!("Only HTTPS URLs are allowed")
}

fn is_debug_local_http(url: &str) -> bool {
    Url::parse(url).is_ok_and(|parsed| {
        parsed.scheme() == "http"
            && matches!(
                parsed.host_str(),
                Some("localhost") | Some("127.0.0.1") | Some("[::1]")
            )
    })
}

fn allowed_navigation_origin(url: &str) -> Result<String> {
    validate_http_url(url)?;
    let parsed = Url::parse(url).context("Invalid channel app URL")?;
    let origin = parsed.origin().ascii_serialization();
    if origin.is_empty() || origin == "null" {
        bail!("Channel app URL must have an origin");
    }
    Ok(origin)
}

fn is_allowed_navigation(navigation_url: &str, allowed_origin: &str) -> bool {
    Url::parse(navigation_url)
        .ok()
        .is_some_and(|url| url.origin().ascii_serialization() == allowed_origin)
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
    let allowed_origin = allowed_navigation_origin(url)?;
    let bounds = webview_bounds(width, height);
    WebViewBuilder::new()
        .with_url(url)
        .with_bounds(bounds)
        .with_initialization_script(MEDIA_DENY_SCRIPT)
        .with_navigation_handler(move |navigation_url| {
            is_allowed_navigation(&navigation_url, &allowed_origin)
        })
        .with_new_window_req_handler(|_| false)
        .build_as_child(parent)
        .context("Failed to create channel app webview")
}

pub fn resize_webview(webview: &WebView, width: f64, height: f64) -> Result<()> {
    webview
        .set_bounds(webview_bounds(width, height))
        .context("Failed to resize channel app webview")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_urls() {
        validate_http_url("https://app.example.com/path").unwrap();
    }

    #[test]
    fn rejects_plain_http() {
        assert!(validate_http_url("http://app.example.com/path").is_err());
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(validate_http_url("file:///etc/passwd").is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn allows_localhost_http_in_debug() {
        validate_http_url("http://localhost:3000/app").unwrap();
        validate_http_url("http://127.0.0.1:8080/app").unwrap();
    }

    #[test]
    fn navigation_allows_same_origin() {
        let origin = allowed_navigation_origin("https://app.example.com/launch").unwrap();
        assert!(is_allowed_navigation(
            "https://app.example.com/other",
            &origin
        ));
    }

    #[test]
    fn navigation_blocks_other_origins() {
        let origin = allowed_navigation_origin("https://app.example.com/launch").unwrap();
        assert!(!is_allowed_navigation("https://evil.example.com/", &origin));
    }
}
