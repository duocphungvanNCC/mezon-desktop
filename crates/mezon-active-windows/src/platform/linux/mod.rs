mod proc_scan;
mod x11;

use crate::info::ActiveWindowInfo;

pub fn get_active_window() -> anyhow::Result<ActiveWindowInfo> {
    match x11::get_active_window_x11() {
        Ok(info) => Ok(info),
        Err(x11_error) => {
            let Some(mut info) = proc_scan::scan_tracked_process() else {
                tracing::debug!(
                    "X11 active window unavailable ({x11_error}); proc scan also found no tracked process"
                );
                return Err(x11_error);
            };
            enrich_title_from_x11(&mut info);
            Ok(info)
        }
    }
}

fn enrich_title_from_x11(info: &mut ActiveWindowInfo) {
    if !info.window_name.is_empty() {
        return;
    }
    let Some(pid) = info.window_pid.parse::<u32>().ok().filter(|pid| *pid > 0) else {
        return;
    };
    let Some(title) = x11::window_title_for_pid(pid) else {
        tracing::debug!(
            pid,
            "proc scan matched process but no X11 window title was found (native Wayland app?)"
        );
        return;
    };
    tracing::debug!(
        pid,
        title,
        "resolved window title from X11 window owned by pid"
    );
    info.window_name = title;
}
