use gpui::{App, AppContext, Entity, Global, SharedString};
use std::sync::Arc;

pub fn download_url_with_dialog(url: SharedString, filename: SharedString, cx: &mut App) {
    let directory = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let suggested = filename.to_string();
    let receiver = cx.prompt_for_new_path(&directory, Some(suggested.as_str()));
    cx.spawn(async move |_cx| {
        if let Ok(Ok(Some(path))) = receiver.await
            && let Err(error) = mezon_client::transport_runtime::download_to(&url, path).await
        {
            tracing::error!("attachment download failed: {error}");
        }
    })
    .detach();
}

pub type OpenUrlFn = Arc<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>;
/// Download `url` and save it locally under the given suggested filename.
pub type SaveAttachmentFn = Arc<dyn Fn(&str, &str) -> anyhow::Result<()> + Send + Sync>;
pub type NotifyFn = Arc<dyn Fn(DesktopNotification) + Send + Sync>;
pub type CliInstallVisibleFn = Arc<dyn Fn() -> bool + Send + Sync>;
pub type CliInstallStateFn = Arc<dyn Fn() -> bool + Send + Sync>;
pub type CliInstallToggleFn = Arc<dyn Fn() -> anyhow::Result<bool> + Send + Sync>;
pub type McpStatusFn = Arc<dyn Fn() -> McpServerStatus + Send + Sync>;
pub type McpStartFn = Arc<dyn Fn(bool) -> anyhow::Result<McpServerStatus> + Send + Sync>;
pub type McpStopFn = Arc<dyn Fn() -> anyhow::Result<McpServerStatus> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpServerStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub read_only: bool,
    pub url: Option<String>,
}

pub struct McpServerHooks {
    pub status: McpStatusFn,
    pub start: McpStartFn,
    pub stop: McpStopFn,
}

pub struct CliInstallHooks {
    pub visible: CliInstallVisibleFn,
    pub installed: CliInstallStateFn,
    pub toggle: CliInstallToggleFn,
}

pub struct DesktopNotification {
    pub title: String,
    pub body: String,
    pub channel_id: Option<String>,
}

pub struct PlatformStore {
    open_url: Option<OpenUrlFn>,
    save_attachment: Option<SaveAttachmentFn>,
    notifier: Option<NotifyFn>,
    cli_install: Option<CliInstallHooks>,
    mcp_server: Option<McpServerHooks>,
}

impl PlatformStore {
    pub fn init(cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|_| Self {
            open_url: None,
            save_attachment: None,
            notifier: None,
            cli_install: None,
            mcp_server: None,
        });
        cx.set_global(GlobalPlatformStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalPlatformStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalPlatformStore>().map(|g| g.0.clone())
    }

    pub fn set_open_url(entity: &Entity<Self>, f: OpenUrlFn, cx: &mut App) {
        entity.update(cx, |store, cx| {
            store.open_url = Some(f);
            cx.notify();
        });
    }

    pub fn set_save_attachment(entity: &Entity<Self>, f: SaveAttachmentFn, cx: &mut App) {
        entity.update(cx, |store, cx| {
            store.save_attachment = Some(f);
            cx.notify();
        });
    }

    pub fn open_url_external(&self, url: &str) -> anyhow::Result<()> {
        match &self.open_url {
            Some(f) => f(url),
            None => Err(anyhow::anyhow!("open_url not registered")),
        }
    }

    pub fn save_attachment(&self, url: &str, filename: &str) -> anyhow::Result<()> {
        match &self.save_attachment {
            Some(f) => f(url, filename),
            None => Err(anyhow::anyhow!("save_attachment not registered")),
        }
    }

    pub fn set_notifier(entity: &Entity<Self>, f: NotifyFn, cx: &mut App) {
        entity.update(cx, |store, cx| {
            store.notifier = Some(f);
            cx.notify();
        });
    }

    pub fn show_notification(&self, notification: DesktopNotification) {
        if let Some(f) = &self.notifier {
            f(notification);
        }
    }

    pub fn set_cli_install(entity: &Entity<Self>, hooks: CliInstallHooks, cx: &mut App) {
        entity.update(cx, |store, cx| {
            store.cli_install = Some(hooks);
            cx.notify();
        });
    }

    pub fn cli_install_visible(&self) -> bool {
        self.cli_install
            .as_ref()
            .is_some_and(|hooks| (hooks.visible)())
    }

    pub fn cli_install_installed(&self) -> bool {
        self.cli_install
            .as_ref()
            .is_some_and(|hooks| (hooks.installed)())
    }

    pub fn cli_install_toggle(&self) -> anyhow::Result<bool> {
        match &self.cli_install {
            Some(hooks) => (hooks.toggle)(),
            None => Err(anyhow::anyhow!("cli install not available")),
        }
    }

    pub fn cli_install_toggle_fn(&self) -> Option<CliInstallToggleFn> {
        self.cli_install
            .as_ref()
            .map(|hooks| Arc::clone(&hooks.toggle))
    }

    pub fn set_mcp_server(entity: &Entity<Self>, hooks: McpServerHooks, cx: &mut App) {
        entity.update(cx, |store, cx| {
            store.mcp_server = Some(hooks);
            cx.notify();
        });
    }

    pub fn mcp_server_available(&self) -> bool {
        self.mcp_server.is_some()
    }

    pub fn mcp_server_status(&self) -> McpServerStatus {
        self.mcp_server
            .as_ref()
            .map(|hooks| (hooks.status)())
            .unwrap_or_default()
    }

    pub fn mcp_server_start_fn(&self) -> Option<McpStartFn> {
        self.mcp_server
            .as_ref()
            .map(|hooks| Arc::clone(&hooks.start))
    }

    pub fn mcp_server_stop_fn(&self) -> Option<McpStopFn> {
        self.mcp_server
            .as_ref()
            .map(|hooks| Arc::clone(&hooks.stop))
    }
}

struct GlobalPlatformStore(Entity<PlatformStore>);
impl Global for GlobalPlatformStore {}
