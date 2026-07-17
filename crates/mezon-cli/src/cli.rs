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
    Logout,
    Refresh,
    Quit,
    Status,
    Mcp(McpCommand),
}

#[derive(Parser)]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

#[derive(Subcommand)]
pub enum McpSubcommand {
    Start {
        #[arg(long)]
        read_only: bool,
        #[arg(long)]
        port: Option<u16>,
    },
    Status,
    Stop,
    Tools {
        #[arg(long)]
        read_only: bool,
    },
    Call {
        name: String,
        #[arg(long, default_value = "{}")]
        args: String,
    },
    Stdio {
        #[arg(long)]
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
    if args
        .iter()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return true;
    }
    matches!(
        args.get(1).map(String::as_str),
        Some("mcp" | "logout" | "refresh" | "quit" | "status")
    )
}
