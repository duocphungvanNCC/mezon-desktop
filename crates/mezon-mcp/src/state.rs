use crate::protocol::McpStatus;
use anyhow::Context as _;
use std::path::PathBuf;

const STATE_FILE: &str = "mcp-state.json";

pub fn state_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mezon")
        .join(STATE_FILE)
}

pub fn write_state(status: &McpStatus) -> anyhow::Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(status)?;
    std::fs::write(&path, data).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn read_state() -> Option<McpStatus> {
    let path = state_path();
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn clear_state() {
    let path = state_path();
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}
