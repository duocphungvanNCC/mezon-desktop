use crate::catalog::{ActivityKind, match_process_name, matching_aliases};
use crate::info::ActiveWindowInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedActivity {
    pub app_name: String,
    pub description: String,
    pub kind: ActivityKind,
}

pub fn tracked_activity_from_window(info: &ActiveWindowInfo) -> Option<TrackedActivity> {
    let (app_name, kind) = match_process_name(&info.app_name())?;
    let description = activity_description(info, kind);
    Some(TrackedActivity {
        app_name,
        description,
        kind,
    })
}

pub fn activity_description(info: &ActiveWindowInfo, kind: ActivityKind) -> String {
    let title = info.window_name.trim();
    if title.is_empty() {
        return String::new();
    }
    let Some((app_name, _)) = match_process_name(&info.app_name()) else {
        return String::new();
    };
    let normalized = normalize_title(title, matching_aliases(&info.app_name()), kind);
    if normalized.is_empty() {
        return String::new();
    }
    if normalized.eq_ignore_ascii_case(&app_name) {
        return String::new();
    }
    normalized
}

fn normalize_title(title: &str, aliases: Option<&[&str]>, kind: ActivityKind) -> String {
    let stripped = strip_app_suffix(title, aliases);
    match kind {
        ActivityKind::Live => strip_spotify_suffix(&stripped),
        ActivityKind::Work | ActivityKind::Play => stripped,
    }
}

fn strip_app_suffix(title: &str, aliases: Option<&[&str]>) -> String {
    if let Some(aliases) = aliases {
        for alias in aliases {
            for separator in [" - ", " — ", " – "] {
                let suffix = format!("{separator}{alias}");
                if let Some(base) = title.strip_suffix(&suffix) {
                    return base.trim().to_string();
                }
            }
        }
    }
    title.trim().to_string()
}

fn strip_spotify_suffix(title: &str) -> String {
    for suffix in [" - Spotify", " — Spotify", " – Spotify"] {
        if let Some(base) = title.strip_suffix(suffix) {
            return base.trim().to_string();
        }
    }
    title.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::ActiveWindowInfo;

    fn info(class: &str, title: &str) -> ActiveWindowInfo {
        ActiveWindowInfo {
            os: "linux".into(),
            window_class: class.into(),
            window_name: title.into(),
            window_desktop: "0".into(),
            window_type: "0".into(),
            window_pid: "0".into(),
            idle_time: "0".into(),
        }
    }

    #[test]
    fn tracked_activity_uses_editor_title() {
        let tracked = tracked_activity_from_window(&info(
            "Code",
            "main.rs - mezon-desktop - Visual Studio Code",
        ))
        .expect("code activity");
        assert_eq!(tracked.app_name, "Code");
        assert_eq!(tracked.description, "main.rs - mezon-desktop");
        assert_eq!(tracked.kind, ActivityKind::Work);
    }

    #[test]
    fn tracked_activity_strips_spotify_suffix() {
        let tracked =
            tracked_activity_from_window(&info("Spotify", "Song Name - Artist - Spotify"))
                .expect("spotify activity");
        assert_eq!(tracked.description, "Song Name - Artist");
        assert_eq!(tracked.kind, ActivityKind::Live);
    }

    #[test]
    fn empty_title_yields_empty_description() {
        let tracked = tracked_activity_from_window(&info("Cursor", "")).expect("cursor activity");
        assert!(tracked.description.is_empty());
    }

    #[test]
    fn title_equal_to_app_name_yields_empty_description() {
        let description = activity_description(&info("Spotify", "Spotify"), ActivityKind::Live);
        assert!(description.is_empty());
    }

    #[test]
    fn rejects_untracked_apps() {
        assert!(tracked_activity_from_window(&info("Google Chrome", "GitHub")).is_none());
    }
}
