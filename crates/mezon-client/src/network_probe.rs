use std::time::{Duration, Instant};

use http_client::{AsyncBody, HttpClient, http};

use crate::transport_runtime;

pub const RECONNECT_NETWORK_PROBE_TIMEOUT: Duration = Duration::from_millis(4000);
pub const ENDPOINT_QUALITY_PROBE_TIMEOUT: Duration = Duration::from_millis(3000);

const FALLBACK_PROBE_ORIGIN: &str = "https://mezon.ai";
const PROBE_PATH: &str = "/assets/favicon.ico";
const ENDPOINT_PROBE_PATH: &str = "/probe";

pub fn favicon_probe_url(origin: &str) -> String {
    let trimmed = origin.trim_end_matches('/');
    let base = if trimmed.is_empty() {
        FALLBACK_PROBE_ORIGIN
    } else {
        trimmed
    };
    format!("{base}{PROBE_PATH}")
}

pub fn endpoint_probe_url(origin: &str) -> String {
    format!("{}{ENDPOINT_PROBE_PATH}", origin.trim_end_matches('/'))
}

pub async fn probe_network_reachability(probe_url: &str, timeout: Duration) -> bool {
    let url = probe_url.to_string();

    let probe = transport_runtime::handle().spawn(async move {
        let request = match http::Request::builder()
            .method(http::Method::HEAD)
            .uri(&url)
            .body(AsyncBody::empty())
        {
            Ok(request) => request,
            Err(_) => return false,
        };
        matches!(
            tokio::time::timeout(timeout, transport_runtime::http_client().send(request)).await,
            Ok(Ok(_))
        )
    });

    probe.await.unwrap_or(false)
}

pub async fn probe_endpoint_latency(probe_url: &str, timeout: Duration) -> Option<Duration> {
    let url = probe_url.to_string();

    let probe = transport_runtime::handle().spawn(async move {
        let request = http::Request::builder()
            .method(http::Method::HEAD)
            .uri(&url)
            .body(AsyncBody::empty())
            .ok()?;
        let started = Instant::now();
        match tokio::time::timeout(timeout, transport_runtime::http_client().send(request)).await {
            Ok(Ok(_)) => Some(started.elapsed()),
            _ => None,
        }
    });

    probe.await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favicon_probe_url_appends_path() {
        assert_eq!(
            favicon_probe_url("https://mezon.ai"),
            "https://mezon.ai/assets/favicon.ico"
        );
    }

    #[test]
    fn favicon_probe_url_trims_trailing_slash() {
        assert_eq!(
            favicon_probe_url("https://dev-mezon.nccsoft.vn/"),
            "https://dev-mezon.nccsoft.vn/assets/favicon.ico"
        );
    }

    #[test]
    fn favicon_probe_url_falls_back_on_empty_origin() {
        assert_eq!(favicon_probe_url(""), "https://mezon.ai/assets/favicon.ico");
    }

    #[test]
    fn probe_url_stays_cacheable() {
        assert!(!favicon_probe_url("https://mezon.ai").contains('?'));
    }

    #[test]
    fn endpoint_probe_uses_the_deployment_probe_path() {
        assert_eq!(
            endpoint_probe_url("https://api.mezon.ai/"),
            "https://api.mezon.ai/probe"
        );
    }
}
