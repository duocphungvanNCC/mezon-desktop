use std::sync::atomic::{AtomicBool, Ordering};

static WAYLAND_SESSION: AtomicBool = AtomicBool::new(false);

pub fn record_wayland_session() {
    WAYLAND_SESSION.store(true, Ordering::Relaxed);
}

pub(crate) fn is_wayland_session() -> bool {
    WAYLAND_SESSION.load(Ordering::Relaxed)
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty())
}
