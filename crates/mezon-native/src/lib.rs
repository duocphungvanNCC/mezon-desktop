pub mod audio;
pub mod autostart;
pub mod badge;
pub mod deep_link;
pub mod instance;
pub mod notifications;
pub mod power;
pub mod tray;

/// Opens a URL in the system default browser.
///
/// Only `https://` URLs are accepted; all other schemes are rejected to prevent
/// `file://`, `javascript:`, or `data:` URIs from being opened.
pub fn open_url(url: &str) -> anyhow::Result<()> {
    if !url.starts_with("https://") {
        return Err(anyhow::anyhow!(
            "open_url rejected: only https scheme is allowed"
        ));
    }
    open::that(url).map_err(|e| anyhow::anyhow!("Failed to open URL: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_url_rejects_http() {
        assert!(open_url("http://mezon.ai/").is_err());
    }

    #[test]
    fn open_url_rejects_file_scheme() {
        assert!(open_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn open_url_rejects_javascript_scheme() {
        assert!(open_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn open_url_rejects_data_uri() {
        assert!(open_url("data:text/html,<h1>hi</h1>").is_err());
    }

    #[test]
    fn open_url_rejects_empty() {
        assert!(open_url("").is_err());
    }
}
