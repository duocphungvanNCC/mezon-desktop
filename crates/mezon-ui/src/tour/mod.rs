mod anchor;
mod launcher;
mod overlay;
mod state;
mod tracks;

use gpui::{App, AppContext as _, KeyBinding};

pub use anchor::{TourAnchor, probe};
pub use launcher::TourLauncher;
pub use state::{TourState, TourStatus, auto_start_core, layer, pending_core_track};
pub use tracks::TRACKS;

pub fn mcp_start(track: Option<&str>, cx: &mut App) -> anyhow::Result<bool> {
    let handle = crate::app::main_window::handle(cx)
        .ok_or_else(|| anyhow::anyhow!("main window not found"))?;
    if let Some(id) = track
        && tracks::track(id).is_none()
    {
        anyhow::bail!("unknown tour track: {id}");
    }
    let track = track.map(str::to_string);
    cx.update_window(handle, |_, window, cx| match track.as_deref() {
        Some(id) => TourState::start_track(id, window, cx),
        None => auto_start_core(window, cx),
    })?;
    Ok(TourState::try_global(cx).is_some_and(|entity| entity.read(cx).is_active()))
}

pub fn mcp_advance(forward: bool, cx: &mut App) -> anyhow::Result<bool> {
    let handle = crate::app::main_window::handle(cx)
        .ok_or_else(|| anyhow::anyhow!("main window not found"))?;
    let Some(entity) = TourState::try_global(cx) else {
        return Ok(false);
    };
    if !entity.read(cx).is_active() {
        return Ok(false);
    }
    cx.update_window(handle, |_, window, cx| {
        entity.update(cx, |this, cx| this.advance(forward, window, cx));
    })?;
    Ok(true)
}

pub fn init(cx: &mut App) {
    TourState::init(cx);
    cx.bind_keys([
        KeyBinding::new("escape", ::menu::Cancel, Some("tour")),
        KeyBinding::new("right", state::TourNext, Some("tour")),
        KeyBinding::new("enter", state::TourNext, Some("tour")),
        KeyBinding::new("left", state::TourBack, Some("tour")),
    ]);
}
