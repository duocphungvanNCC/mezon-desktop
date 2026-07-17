use crate::router::{build_tool_router, server_instructions};
use crate::tools::McpBackend;
use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{ServerCapabilities, ServerInfo},
    tool_handler,
};

#[derive(Clone)]
pub struct HttpMcpService {
    read_only: bool,
    router: ToolRouter<Self>,
}

impl HttpMcpService {
    pub fn new(backend: McpBackend) -> Self {
        let read_only = backend.read_only();
        let router = build_tool_router(read_only, {
            let backend = backend.clone();
            move |name, args| {
                let backend = backend.clone();
                async move { backend.call_tool(&name, args).await }
            }
        });
        Self { read_only, router }
    }
}

#[tool_handler(router = self.router)]
impl ServerHandler for HttpMcpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(server_instructions(self.read_only))
    }
}
