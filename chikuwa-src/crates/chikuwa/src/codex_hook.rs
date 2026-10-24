use std::io::Read;

use anyhow::{Context, Result};

use crate::agent::parser::DisplayMode;
use crate::agent::state::AgentView;
use crate::agent::{CodexHookParser, HookParser};
use crate::ipc;

/// Run the Codex hook subcommand: read stdin JSON, parse via CodexHookParser, send state via IPC.
pub async fn run() -> Result<()> {
    let pane_id = std::env::var("TMUX_PANE")
        .context("TMUX_PANE environment variable not set (not running inside tmux?)")?;

    let mut stdin_buf = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin_buf)
        .context("Failed to read stdin")?;

    let parser = CodexHookParser;
    let result = parser.parse(pane_id, &stdin_buf)?;

    if result.display == DisplayMode::Suppress {
        return Ok(());
    }

    eprintln!(
        "[chikuwa codex-hook] event: {} agent_id: {:?}",
        result.state.event_label(),
        result.state.agent_id()
    );

    ipc::broadcast_state(&result.state).await?;

    if let Err(e) = crate::persist::append_agent_state(&result.state) {
        eprintln!("Warning: failed to persist agent state: {}", e);
    }

    Ok(())
}
