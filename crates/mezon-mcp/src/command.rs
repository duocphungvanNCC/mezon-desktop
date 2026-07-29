use serde_json::Value;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy)]
pub enum CaptureTarget {
    Window,
    Chat,
}

#[derive(Debug)]
pub enum McpCommand {
    GetContext {
        reply: oneshot::Sender<Value>,
    },
    Navigate {
        path: String,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Logout {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Refresh {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Quit {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    ShowWindow {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Capture {
        target: CaptureTarget,
        reply: oneshot::Sender<anyhow::Result<Value>>,
    },
    GoBack {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    GoForward {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    GetSettings {
        reply: oneshot::Sender<Value>,
    },
    SetSetting {
        key: String,
        value: Value,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    SetCliEnabled {
        enabled: bool,
        reply: oneshot::Sender<anyhow::Result<bool>>,
    },
}
