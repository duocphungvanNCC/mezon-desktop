//! Process-wide rustls crypto provider (ring) for TLS used by TCP and WebSocket adapters.

use std::sync::OnceLock;

static INSTALLED: OnceLock<()> = OnceLock::new();

/// Install the ring crypto provider once per process. Safe to call from any adapter.
pub fn ensure_crypto_provider() {
    INSTALLED.get_or_init(|| {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    });
}
