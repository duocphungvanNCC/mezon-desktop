use crate::http_server::HttpMcpService;
use crate::protocol::{McpStartResult, McpStatus, mcp_url};
use crate::state;
use crate::tools::McpBackend;
use anyhow::Context as _;
use futures::channel::mpsc::UnboundedSender;
use mezon_client::AppApi;
use rmcp::transport::{
    StreamableHttpServerConfig,
    streamable_http_server::{session::local::LocalSessionManager, tower::StreamableHttpService},
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

struct ControllerInner {
    api: Option<Arc<AppApi>>,
    ui_tx: Option<UnboundedSender<crate::command::McpCommand>>,
    status: McpStatus,
    cancel: Option<CancellationToken>,
    server_task: Option<tokio::task::JoinHandle<()>>,
}

pub struct McpController {
    inner: Arc<Mutex<ControllerInner>>,
}

impl McpController {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ControllerInner {
                api: None,
                ui_tx: None,
                status: McpStatus::stopped(),
                cancel: None,
                server_task: None,
            })),
        }
    }

    pub async fn status(&self) -> McpStatus {
        self.inner.lock().await.status.clone()
    }

    pub async fn set_backend(
        &self,
        api: Arc<AppApi>,
        ui_tx: UnboundedSender<crate::command::McpCommand>,
    ) {
        let mut inner = self.inner.lock().await;
        inner.api = Some(api);
        inner.ui_tx = Some(ui_tx);
    }

    pub async fn start(
        &self,
        read_only: bool,
        port: Option<u16>,
    ) -> anyhow::Result<McpStartResult> {
        let mut inner = self.inner.lock().await;
        if inner.status.running {
            anyhow::bail!("MCP server is already running");
        }
        let api = inner
            .api
            .clone()
            .ok_or_else(|| anyhow::anyhow!("MCP backend is not initialized"))?;
        let ui_tx = inner.ui_tx.clone();
        let backend = McpBackend::new(api, ui_tx, read_only);

        let cancel = CancellationToken::new();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port.unwrap_or(0)))
            .await
            .context("Binding MCP HTTP listener")?;
        let bound_port = listener
            .local_addr()
            .context("Reading MCP listener address")?
            .port();
        let url = mcp_url(bound_port);

        let cancel_for_task = cancel.clone();
        let service = StreamableHttpService::new(
            move || Ok(HttpMcpService::new(backend.clone())),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .with_cancellation_token(cancel.child_token()),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let server_task = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    cancel_for_task.cancelled_owned().await;
                })
                .await
            {
                tracing::error!("MCP HTTP server exited with error: {e}");
            }
        });

        let status = McpStatus {
            running: true,
            port: Some(bound_port),
            read_only,
            url: Some(url.clone()),
        };
        if let Err(e) = state::write_state(&status) {
            tracing::warn!("Failed to persist MCP state: {e}");
        }

        inner.status = status;
        inner.cancel = Some(cancel);
        inner.server_task = Some(server_task);

        Ok(McpStartResult {
            port: bound_port,
            url,
            read_only,
        })
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if !inner.status.running {
            return Ok(());
        }
        if let Some(cancel) = inner.cancel.take() {
            cancel.cancel();
        }
        if let Some(task) = inner.server_task.take() {
            task.abort();
            let _ = task.await;
        }
        inner.status = McpStatus::stopped();
        state::clear_state();
        Ok(())
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        let inner = self.inner.lock().await;
        let api = inner
            .api
            .clone()
            .ok_or_else(|| anyhow::anyhow!("MCP backend is not initialized"))?;
        let backend = McpBackend::new(api, inner.ui_tx.clone(), inner.status.read_only);
        backend.call_tool(name, arguments).await
    }
}

impl Default for McpController {
    fn default() -> Self {
        Self::new()
    }
}
