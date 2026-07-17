use crate::mcp::tool_call;
use mezon_mcp::{build_tool_router, list_tools_json};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::router::tool::ToolRouter,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool_handler,
    transport::stdio,
};

struct StdioProxy {
    router: ToolRouter<StdioProxy>,
}

impl StdioProxy {
    fn new(read_only: bool) -> Self {
        Self {
            router: build_tool_router(
                read_only,
                |name, args| async move { tool_call(&name, args) },
            ),
        }
    }
}

#[tool_handler(router = self.router)]
impl ServerHandler for StdioProxy {
    fn get_info(&self) -> ServerInfo {
        let count = self.router.list_all().len();
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new("mezon", env!("CARGO_PKG_VERSION")))
        .with_instructions(format!("Mezon desktop MCP stdio proxy ({count} tools)"))
    }
}

pub async fn run_stdio_proxy(read_only: bool) -> anyhow::Result<()> {
    let _ = list_tools_json(read_only);
    let service = StdioProxy::new(read_only).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mezon_mcp::{TOOL_SPECS, tool_count};

    #[test]
    fn registers_full_tool_catalog() {
        assert_eq!(tool_count(false), TOOL_SPECS.len());
        let proxy = StdioProxy::new(false);
        assert_eq!(proxy.router.list_all().len(), TOOL_SPECS.len());
    }

    #[test]
    fn read_only_excludes_write_tools() {
        let write_tools = TOOL_SPECS.iter().filter(|spec| spec.write).count();
        assert_eq!(tool_count(true), TOOL_SPECS.len() - write_tools);
        let proxy = StdioProxy::new(true);
        assert_eq!(proxy.router.list_all().len(), tool_count(true));
    }
}
