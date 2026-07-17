mod app;
mod cli;
mod mcp;
mod stdio;

pub use cli::{Cli, Commands, is_cli_invocation, parse_from};

pub fn try_run<I, T>(args: I) -> anyhow::Result<Option<i32>>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = parse_from(args);
    let Some(command) = cli.command else {
        return Ok(None);
    };

    let result = match command {
        Commands::Logout => app::logout(),
        Commands::Refresh => app::refresh(),
        Commands::Quit => app::quit(),
        Commands::Status => app::status(),
        Commands::Mcp(mcp) => mcp::try_run_mcp(mcp),
    };

    match result {
        Ok(()) => Ok(Some(0)),
        Err(error) => {
            eprintln!("{error}");
            Ok(Some(1))
        }
    }
}
