use std::io::Read;

use anyhow::{Context, Result};

use crate::agent::parser::DisplayMode;
use crate::agent::{HookParser, OpenCodeHookParser};
use crate::ipc;

/// Run the OpenCode hook subcommand: read stdin JSON, parse via OpenCodeHookParser, send state via IPC.
pub async fn run() -> Result<()> {
    let pane_id = std::env::var("TMUX_PANE")
        .context("TMUX_PANE environment variable not set (not running inside tmux?)")?;

    let mut stdin_buf = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin_buf)
        .context("Failed to read stdin")?;

    eprintln!("[chikuwa opencode-hook] received: {}", stdin_buf.trim());

    let parser = OpenCodeHookParser;
    let result = parser.parse(pane_id, &stdin_buf)?;

    if result.display == DisplayMode::Suppress {
        return Ok(());
    }

    ipc::broadcast_state(&result.state).await?;

    // Persist to JSONL so TUI can restore state on restart
    if let Err(e) = crate::persist::append_agent_state(&result.state) {
        eprintln!("Warning: failed to persist agent state: {}", e);
    }

    Ok(())
}
