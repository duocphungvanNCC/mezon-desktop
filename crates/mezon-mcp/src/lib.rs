pub mod catalog;
pub mod command;
pub mod http_server;
pub mod protocol;
pub mod router;
pub mod schemas;
pub mod server;
pub mod state;
pub mod tools;

pub use catalog::{TOOL_SPECS, is_write_tool, list_tools_json};
pub use router::{build_tool_router, server_instructions, tool_call_result, tool_count};

pub use command::{CaptureTarget, McpCommand};
pub use protocol::{McpStartParams, McpStartResult, McpStatus, ToolCallParams, mcp_url};
pub use server::McpController;
pub use state::{clear_state, read_state, state_path, write_state};
pub use tools::McpBackend;
