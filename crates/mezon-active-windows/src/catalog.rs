use crate::info::normalize_process_name;

pub const ACTIVITY_TYPE_WORK: i32 = 1;
pub const ACTIVITY_TYPE_LIVE: i32 = 2;
pub const ACTIVITY_TYPE_PLAY: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Work,
    Live,
    Play,
}

impl ActivityKind {
    pub fn as_type(self) -> i32 {
        match self {
            Self::Work => ACTIVITY_TYPE_WORK,
            Self::Live => ACTIVITY_TYPE_LIVE,
            Self::Play => ACTIVITY_TYPE_PLAY,
        }
    }
}

struct CatalogEntry {
    aliases: &'static [&'static str],
    kind: ActivityKind,
}

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        aliases: &[
            "Code",
            "Visual Studio Code",
            "Cursor",
            "Xcode",
            "Sublime Text",
            "Atom",
            "Notepad",
            "CoffeeCup HTML Editor",
            "TextMate",
            "Bluefish",
            "Vim",
            "NetBeans",
            "Codeshare.io",
            "GNU Emacs",
            "Spacemacs",
            "BBEdit",
            "WebStorm",
            "UltraEdit",
            "Espresso",
            "Nova",
            "Unity",
            "Figma",
        ],
        kind: ActivityKind::Work,
    },
    CatalogEntry {
        aliases: &["Spotify"],
        kind: ActivityKind::Live,
    },
    CatalogEntry {
        aliases: &["LeagueClientUx", "League Of Legends"],
        kind: ActivityKind::Play,
    },
];

pub fn classify_process_name(raw: &str) -> Option<ActivityKind> {
    match_process_name(raw).map(|(_, kind)| kind)
}

pub fn match_process_name(raw: &str) -> Option<(String, ActivityKind)> {
    let normalized = normalize_process_name(raw);
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    for entry in CATALOG {
        for alias in entry.aliases {
            if alias.to_ascii_lowercase() == lower {
                return Some((alias.to_string(), entry.kind));
            }
        }
    }
    None
}

pub fn match_linux_process(
    comm: &str,
    cmdline: &str,
    exe: Option<&str>,
) -> Option<(String, ActivityKind)> {
    if comm.eq_ignore_ascii_case("cursorsandbox") {
        return None;
    }
    if let Some(matched) = match_process_name(comm) {
        return Some(matched);
    }
    for line in [cmdline, exe.unwrap_or_default()] {
        if line.is_empty() {
            continue;
        }
        if let Some(matched) = match_linux_cmdline(line) {
            return Some(matched);
        }
    }
    None
}

fn match_linux_cmdline(line: &str) -> Option<(String, ActivityKind)> {
    let lower = line.to_ascii_lowercase();
    if lower.contains("cursorsandbox") {
        return None;
    }
    const PATH_HINTS: [(&str, &str); 5] = [
        ("/cursor/cursor", "Cursor"),
        ("/cursor.appimage", "Cursor"),
        ("/spotify/", "Spotify"),
        ("/code/code", "Code"),
        ("/visual studio code/", "Visual Studio Code"),
    ];
    for (needle, alias) in PATH_HINTS {
        if lower.contains(needle) {
            return match_process_name(alias);
        }
    }
    if let Some(file_name) = line.rsplit(['/', '\\']).next() {
        return match_process_name(file_name);
    }
    None
}

pub fn pick_highest_priority_match(
    matches: impl IntoIterator<Item = (String, ActivityKind)>,
) -> Option<(String, ActivityKind)> {
    let mut best: Option<(String, ActivityKind, u8)> = None;
    for (name, kind) in matches {
        let priority = kind_priority(kind);
        let replace = best
            .as_ref()
            .map(|(_, _, current)| priority > *current)
            .unwrap_or(true);
        if replace {
            best = Some((name, kind, priority));
        }
    }
    best.map(|(name, kind, _)| (name, kind))
}

fn kind_priority(kind: ActivityKind) -> u8 {
    match kind {
        ActivityKind::Play => 3,
        ActivityKind::Work => 2,
        ActivityKind::Live => 1,
    }
}

pub fn matching_aliases(raw: &str) -> Option<&'static [&'static str]> {
    let normalized = normalize_process_name(raw);
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    CATALOG
        .iter()
        .find(|entry| {
            entry
                .aliases
                .iter()
                .any(|alias| alias.to_ascii_lowercase() == lower)
        })
        .map(|entry| entry.aliases)
}

pub fn classify_wm_class(raw: &str) -> Option<ActivityKind> {
    let (instance, class) = crate::info::parse_wm_class(raw);
    classify_process_name(&instance).or_else(|| classify_process_name(&class))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_code_editors() {
        assert_eq!(classify_process_name("Code.exe"), Some(ActivityKind::Work));
        assert_eq!(
            classify_process_name("Visual Studio Code"),
            Some(ActivityKind::Work)
        );
        assert_eq!(classify_process_name("Cursor"), Some(ActivityKind::Work));
        assert_eq!(classify_process_name("cursor"), Some(ActivityKind::Work));
    }

    #[test]
    fn classifies_spotify_and_lol() {
        assert_eq!(classify_process_name("Spotify"), Some(ActivityKind::Live));
        assert_eq!(
            classify_process_name("LeagueClientUx"),
            Some(ActivityKind::Play)
        );
        assert_eq!(
            classify_process_name("League Of Legends"),
            Some(ActivityKind::Play)
        );
    }

    #[test]
    fn rejects_unlisted_apps() {
        assert_eq!(classify_process_name("Google Chrome"), None);
        assert_eq!(classify_process_name("Safari"), None);
        assert_eq!(classify_process_name(""), None);
    }

    #[test]
    fn match_linux_process_uses_cmdline_path_hints() {
        assert_eq!(
            match_linux_process("electron", "/opt/Cursor/cursor", None),
            Some(("Cursor".to_string(), ActivityKind::Work))
        );
        assert_eq!(
            match_linux_process("spotify", "/usr/share/spotify/spotify", None),
            Some(("Spotify".to_string(), ActivityKind::Live))
        );
    }

    #[test]
    fn pick_highest_priority_match_prefers_play_over_work_and_live() {
        let picked = pick_highest_priority_match([
            ("Spotify".to_string(), ActivityKind::Live),
            ("Code".to_string(), ActivityKind::Work),
            ("LeagueClientUx".to_string(), ActivityKind::Play),
        ]);
        assert_eq!(
            picked,
            Some(("LeagueClientUx".to_string(), ActivityKind::Play))
        );

        let picked = pick_highest_priority_match([
            ("Spotify".to_string(), ActivityKind::Live),
            ("Cursor".to_string(), ActivityKind::Work),
        ]);
        assert_eq!(picked, Some(("Cursor".to_string(), ActivityKind::Work)));
    }
}
