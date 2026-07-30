use crate::cli::McpSubcommand;
use anyhow::Context as _;
use mezon_mcp::{McpStartParams, McpStatus, ToolCallParams, read_state};
use mezon_native::control::ControlClient;
use serde_json::Value;

pub fn try_run_mcp(mcp: crate::cli::McpCommand) -> anyhow::Result<()> {
    match mcp.command {
        McpSubcommand::Start { read_only, port } => mcp_start(read_only, port),
        McpSubcommand::Status => mcp_status(),
        McpSubcommand::Stop => mcp_stop(),
        McpSubcommand::Tools { read_only } => mcp_tools(read_only),
        McpSubcommand::Call { name, args } => mcp_call(&name, &args),
        McpSubcommand::Stdio { read_only } => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()?;
            runtime.block_on(mcp_stdio(read_only))
        }
    }
}

fn mcp_start(read_only: bool, port: Option<u16>) -> anyhow::Result<()> {
    let params = serde_json::to_value(McpStartParams { read_only, port })?;
    let result = ControlClient::request("mcp.start", params)?;
    let port = result
        .get("port")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("missing port in response"))?;
    let url = result
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing url in response"))?;
    println!("MCP server started on port {port}");
    println!("{url}");
    Ok(())
}

fn mcp_status() -> anyhow::Result<()> {
    let status = match ControlClient::request("mcp.status", Value::Null) {
        Ok(result) => serde_json::from_value::<McpStatus>(result)?,
        Err(_) => read_state().unwrap_or_else(McpStatus::stopped),
    };
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

fn mcp_stop() -> anyhow::Result<()> {
    ControlClient::request("mcp.stop", Value::Null)?;
    println!("MCP server stopped");
    Ok(())
}

fn mcp_tools(read_only: bool) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&mezon_mcp::list_tools_json(read_only))?
    );
    Ok(())
}

fn mcp_call(name: &str, args: &str) -> anyhow::Result<()> {
    let arguments: Value =
        serde_json::from_str(args).with_context(|| format!("invalid JSON for --args: {args}"))?;
    let result = tool_call(name, arguments)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

async fn mcp_stdio(read_only: bool) -> anyhow::Result<()> {
    crate::stdio::run_stdio_proxy(read_only).await
}

pub(crate) fn tool_call(name: &str, arguments: Value) -> anyhow::Result<Value> {
    let params = ToolCallParams {
        name: name.to_string(),
        arguments,
    };
    ControlClient::request("tool.call", serde_json::to_value(params)?).map_err(|error| {
        if error.to_string().contains("not running") {
            anyhow::anyhow!("{error}. Open the Mezon desktop app first, then retry the tool.")
        } else {
            error
        }
    })
}
