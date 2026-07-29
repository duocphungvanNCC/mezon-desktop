use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mezon",
    about = "Mezon CLI",
    version,
    after_help = "Run `mezon` to open the Mezon app.\nUse `mezon mcp stdio` for Standard MCP integration."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Sign out of the current Mezon session
    Logout,
    /// Refresh clans, DMs, and message lists
    Refresh,
    /// Quit the running Mezon desktop app
    Quit,
    /// Show Mezon app status
    Status,
    /// Manage the local Mezon MCP server
    Mcp(McpCommand),
}

#[derive(Parser)]
#[command(about = "Manage the local Mezon MCP server")]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

#[derive(Subcommand)]
pub enum McpSubcommand {
    /// Start the MCP HTTP server in the running Mezon app
    Start {
        #[arg(long, help = "Expose only read tools")]
        read_only: bool,
        #[arg(long, help = "Preferred listen port (default: ephemeral)")]
        port: Option<u16>,
    },
    /// Show MCP server status
    Status,
    /// Stop the MCP HTTP server
    Stop,
    /// List available MCP tools
    Tools {
        #[arg(long, help = "List only read tools")]
        read_only: bool,
    },
    /// Call an MCP tool through the running Mezon app
    Call {
        /// Tool name
        name: String,
        #[arg(long, default_value = "{}", help = "JSON object of tool arguments")]
        args: String,
    },
    /// Run an MCP stdio proxy for Standard MCP clients
    Stdio {
        #[arg(long, help = "Expose only read tools")]
        read_only: bool,
    },
}

pub fn parse_from<I, T>(args: I) -> Cli
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    Cli::parse_from(args)
}

pub fn is_cli_invocation(args: &[String]) -> bool {
    if args.len() <= 1 {
        return false;
    }
    !args
        .get(1)
        .is_some_and(|arg| arg.starts_with("mezonapp://"))
}
