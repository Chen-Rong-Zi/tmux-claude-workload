use clap::{Parser, Subcommand};

use chikuwa::{app, codex_hook, hook, ipc, opencode};

#[derive(Parser)]
#[command(name = "chikuwa", about = "tmux AI Agent monitor TUI", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Hook mode: update agent state from Claude Code hooks (reads event from stdin JSON)
    Hook,
    /// OpenCode hook mode: update agent state from OpenCode hooks (reads event from stdin JSON)
    OpencodeHook,
    /// Codex hook mode: update agent state from Codex CLI hooks (reads event from stdin JSON)
    CodexHook,
    /// Notify the TUI of a tmux change (used by tmux hooks)
    Notify,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Hook) => {
            hook::run().await?;
        }
        Some(Commands::OpencodeHook) => {
            opencode::run().await?;
        }
        Some(Commands::CodexHook) => {
            codex_hook::run().await?;
        }
        Some(Commands::Notify) => {
            ipc::broadcast_notify().await?;
        }
        None => {
            app::run().await?;
        }
    }

    Ok(())
}
