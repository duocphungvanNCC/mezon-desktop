use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    pub show_member_list: bool,
    pub show_member_list_dm: bool,
    pub show_profile_dm: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            show_member_list: true,
            show_member_list_dm: true,
            show_profile_dm: true,
        }
    }
}

impl UiState {
    pub fn close_dm_profile_for_clan(&mut self) -> bool {
        std::mem::replace(&mut self.show_profile_dm, false)
    }

    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mezon")
            .join("ui_state.json")
    }

    pub fn load_sync() -> Self {
        let path = Self::path();
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse ui_state.json, using defaults: {e}");
                Self::default()
            }),
            Err(e) => {
                tracing::warn!("Failed to read ui_state.json, using defaults: {e}");
                Self::default()
            }
        }
    }

    pub fn save_sync(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = match serde_json::to_string_pretty(self) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to serialize ui_state: {e}");
                return;
            }
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &data) {
            tracing::warn!("Failed to write ui_state tmp file: {e}");
            return;
        }
        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UiState;

    #[test]
    fn ui_state_defaults_open() {
        let state = UiState::default();
        assert!(state.show_member_list);
        assert!(state.show_member_list_dm);
        assert!(state.show_profile_dm);
    }

    #[test]
    fn ui_state_json_roundtrip() {
        let state = UiState {
            show_member_list: false,
            show_member_list_dm: true,
            show_profile_dm: false,
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: UiState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, state);
    }

    #[test]
    fn entering_clan_closes_profile_without_closing_member_lists() {
        let mut state = UiState::default();
        assert!(state.close_dm_profile_for_clan());
        assert!(!state.show_profile_dm);
        assert!(state.show_member_list_dm);
        assert!(state.show_member_list);
        assert!(!state.close_dm_profile_for_clan());
        let restored: UiState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(!restored.show_profile_dm);
    }

    #[test]
    fn legacy_ui_state_keeps_group_visibility_and_defaults_profile_open() {
        let state: UiState =
            serde_json::from_str(r#"{"show_member_list":true,"show_member_list_dm":false}"#)
                .unwrap();
        assert!(!state.show_member_list_dm);
        assert!(state.show_profile_dm);
    }
}
