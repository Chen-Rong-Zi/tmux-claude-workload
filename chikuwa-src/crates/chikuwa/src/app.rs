use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::agent::state::push_recent_tool;
use crate::agent::state::{AgentState, AgentView};
use crate::agent::{SubagentInfo, SubagentStatus};
use crate::event::{self, Action, AppEvent};
use crate::git::GitInfoCache;
use crate::ipc;
use crate::persist;
use crate::tmux::{client as tmux_client, types::TmuxSession};
use crate::ui::{status_bar, theme, tree};
use crate::usage::Usage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Tree,
    Office,
}

/// Strip a leading NerdFont icon (Private Use Area character) and whitespace from a title.
fn strip_leading_icon(title: &str) -> &str {
    let mut chars = title.chars();
    if let Some(c) = chars.next() {
        let cp = c as u32;
        if matches!(cp, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD) {
            return chars.as_str().trim_start();
        }
    }
    title
}

/// Extract the filename and optional directory from an nvim pane_title.
/// Supports two formats:
/// - New: "<icon> relative/path" (NerdFont icon + relative path from repo root)
/// - Legacy: "filename (dir) - Nvim" or "filename - Nvim"
///
/// Plugin UIs like NeoTree produce titles like "neo-tree filesystem [1] - Nvim".
/// Returns `Some((filename, Option<dir>))` for valid file titles, `None` for plugin UIs.
fn extract_nvim_file_info(title: &str) -> Option<(&str, Option<&str>)> {
    // Strip leading NerdFont icon if present
    let title = strip_leading_icon(title);

    // Nvim standard format: "filename (dir) - Nvim" or "filename - Nvim"
    if let Some(rest) = title.strip_suffix(" - Nvim") {
        // Try to extract "filename (dir)"
        if let Some(paren_start) = rest.find(" (") {
            let name = &rest[..paren_start];
            if !name.is_empty() && !name.contains(' ') {
                let dir = &rest[paren_start + 2..];
                let dir = dir.strip_suffix(')').unwrap_or(dir);
                return Some((name, Some(dir)));
            }
            return None;
        }
        // "filename - Nvim" without directory
        if !rest.is_empty() && !rest.contains(' ') {
            return Some((rest, None));
        }
        return None;
    }
    // Path or bare filename without " - Nvim" suffix
    if !title.is_empty() && !title.starts_with("term://") {
        // If it contains '/', it's a relative path — return as-is
        if title.contains('/') {
            return Some((title, None));
        }
        // Bare filename (no spaces allowed)
        if !title.contains(' ') {
            return Some((title, None));
        }
    }
    None
}

/// Compute relative path from git toplevel, abbreviating directories
/// progressively from left if the result exceeds max_len.
fn relative_nvim_path(filename: &str, dir: Option<&str>, toplevel: Option<&str>) -> String {
    let Some(dir) = dir else {
        // New format: filename is already a relative path from repo root.
        // Prepend repo dir name and shorten.
        if filename.contains('/') {
            if let Some(toplevel) = toplevel {
                let repo_dir = std::path::Path::new(toplevel)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let full = format!("{}/{}", repo_dir, filename);
                return tree::shorten_relative_path(&full, 30);
            }
        }
        return filename.to_string();
    };
    let Some(toplevel) = toplevel else {
        return filename.to_string();
    };

    // Expand ~ in dir
    let home = std::env::var("HOME").unwrap_or_default();
    let expanded_dir = if dir.starts_with("~/") {
        format!("{}{}", home, &dir[1..])
    } else if dir == "~" {
        home.clone()
    } else {
        dir.to_string()
    };

    // Extract repo dir name from toplevel
    let repo_dir = std::path::Path::new(toplevel)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // Compute relative path from toplevel
    let full_path = format!("{}/{}", expanded_dir, filename);
    let full = if let Some(rest) = full_path.strip_prefix(toplevel) {
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        if rest.is_empty() {
            return filename.to_string();
        }
        format!("{}/{}", repo_dir, rest)
    } else {
        return filename.to_string();
    };

    tree::shorten_relative_path(&full, 30)
}

pub struct App {
    sessions: Vec<TmuxSession>,
    tree_items: Vec<tree::TreeItem>,
    selected: usize,
    scroll_offset: usize,
    collapsed: HashSet<String>,
    should_quit: bool,
    agent_states: HashMap<String, AgentState>,
    /// Subagent states keyed by (tmux_pane, agent_id)
    subagent_states: HashMap<(String, String), SubagentInfo>,
    /// Count of completed subagents per pane
    completed_subagent_counts: HashMap<String, u32>,
    git_cache: GitInfoCache,
    anim_frame: usize,
    /// Cache of last valid nvim file title per pane_id.
    nvim_title_cache: HashMap<String, String>,
    /// Last known terminal width for visual row calculations.
    last_width: u16,
    /// The tree area rect from the last draw, for mouse hit-testing.
    tree_area: ratatui::layout::Rect,
    /// True when the user has navigated manually (Up/Down/Top/Bottom).
    /// Prevents auto-follow of the active tmux pane until the user selects an item.
    user_navigated: bool,
    /// True until the first selection is made (used to select TMUX_PANE on startup).
    first_selection: bool,
    /// Cached visible height for scrolloff calculations outside draw.
    visible_height: usize,
    /// Claude API usage data (fetched periodically).
    usage: Option<Result<Usage, String>>,
    /// When the next usage fetch is scheduled.
    usage_next_fetch: Option<std::time::Instant>,
    view_mode: ViewMode,
    /// Last time Tick event triggered a tmux refresh (throttled to ~2s).
    last_tick_refresh: Instant,
    /// Pending git info from background fetches, keyed by pane path.
    pending_git_info: HashMap<String, crate::git::GitInfo>,
    /// True when a FlushGitInfo timer is armed.
    git_debounce_active: bool,
    /// Minimum visible rows above/below the selected item (scrolloff).
    scrolloff: usize,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let mut git_cache = GitInfoCache::new();
        if let Some(entries) = persist::load_git_cache() {
            git_cache.populate_from_entries(entries);
        }

        Self {
            sessions: Vec::new(),
            tree_items: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            collapsed: HashSet::new(),
            should_quit: false,
            agent_states: persist::load_agent_states(),
            subagent_states: persist::load_subagent_states(),
            completed_subagent_counts: persist::load_completed_counts(),
            git_cache,
            anim_frame: 0,
            nvim_title_cache: HashMap::new(),
            last_width: 80,
            tree_area: ratatui::layout::Rect::default(),
            user_navigated: false,
            first_selection: true,
            visible_height: 0,
            usage: persist::load_usage().map(Ok),
            usage_next_fetch: None,
            view_mode: ViewMode::Tree,
            last_tick_refresh: Instant::now() - Duration::from_secs(3),
            pending_git_info: HashMap::new(),
            git_debounce_active: false,
            scrolloff: 7,
        }
    }

    /// Refresh tmux data and rebuild the tree.
    async fn refresh(&mut self) -> Result<()> {
        match tmux_client::fetch_tree(&self.agent_states).await {
            Ok(sessions) => {
                self.sessions = sessions;
                self.merge_git_info().await;
                self.fixup_nvim_titles();
                self.rebuild_tree();
            }
            Err(_) => {
                // tmux not running or error - show empty state
                self.sessions.clear();
                self.tree_items.clear();
            }
        }
        Ok(())
    }

    /// Refresh tmux tree without fetching git info.
    async fn refresh_tree_only(&mut self) -> Result<()> {
        match tmux_client::fetch_tree(&self.agent_states).await {
            Ok(sessions) => {
                self.sessions = sessions;
                // Skip merge_git_info — git info arrives later via GitInfoReady
                // Skip fixup_nvim_titles — needs toplevel from git info
                self.rebuild_tree();
            }
            Err(_) => {
                self.sessions.clear();
                self.tree_items.clear();
            }
        }
        Ok(())
    }

    /// Apply all pending git info to matching panes and redraw.
    fn apply_pending_git_info(&mut self) {
        for (path, info) in self.pending_git_info.drain() {
            for session in &mut self.sessions {
                for window in &mut session.windows {
                    for pane in &mut window.panes {
                        if pane.pane_current_path == path {
                            pane.git_info = Some(info.clone());
                        }
                    }
                }
            }
        }
        // Re-derive session-level repo_name/toplevel/worktree_name
        for session in &mut self.sessions {
            session.repo_name = session
                .windows
                .iter()
                .flat_map(|w| w.panes.iter())
                .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.repo_name.clone()));
            session.toplevel = session
                .windows
                .iter()
                .flat_map(|w| w.panes.iter())
                .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.toplevel.clone()));
            session.worktree_name = session
                .windows
                .iter()
                .flat_map(|w| w.panes.iter())
                .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.worktree_name.clone()));
        }
        self.fixup_nvim_titles();
        self.rebuild_tree();
    }

    /// Fetch git info for all unique pane paths and merge into panes.
    async fn merge_git_info(&mut self) {
        // Collect unique paths
        let mut active_paths = HashSet::new();
        for session in &self.sessions {
            for window in &session.windows {
                for pane in &window.panes {
                    active_paths.insert(PathBuf::from(&pane.pane_current_path));
                }
            }
        }

        // GC stale cache entries
        self.git_cache.retain_paths(&active_paths);

        // Clone paths for async closure (we need owned values for parallel execution)
        let paths: Vec<PathBuf> = active_paths.into_iter().collect();

        // Fetch git info for all paths in parallel
        // Note: We can't easily parallelize the cache access due to &mut self,
        // but we can at least parallelize the internal git fetches within each get()
        let mut path_info: HashMap<String, crate::git::GitInfo> = HashMap::new();
        for path in paths {
            if let Some(path_str) = path.to_str() {
                if let Some(info) = self.git_cache.get(path_str).await {
                    path_info.insert(path_str.to_string(), info);
                }
            }
        }

        // Merge into panes and derive session repo_name
        for session in &mut self.sessions {
            for window in &mut session.windows {
                for pane in &mut window.panes {
                    pane.git_info = path_info.get(&pane.pane_current_path).cloned();
                }
            }
            // Derive repo_name and toplevel from the first pane that has them
            session.repo_name = session
                .windows
                .iter()
                .flat_map(|w| w.panes.iter())
                .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.repo_name.clone()));
            session.toplevel = session
                .windows
                .iter()
                .flat_map(|w| w.panes.iter())
                .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.toplevel.clone()));
            session.worktree_name = session
                .windows
                .iter()
                .flat_map(|w| w.panes.iter())
                .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.worktree_name.clone()));
        }
    }

    /// For nvim panes, extract the filename from the title and compute
    /// relative path from git toplevel. Plugin UI titles are replaced with
    /// the last known path from cache.
    fn fixup_nvim_titles(&mut self) {
        for session in &mut self.sessions {
            let session_toplevel = session.toplevel.clone();
            for window in &mut session.windows {
                for pane in &mut window.panes {
                    if pane.pane_current_command != "nvim" {
                        continue;
                    }
                    if let Some((filename, dir)) = extract_nvim_file_info(&pane.pane_title) {
                        let pane_toplevel =
                            pane.git_info.as_ref().and_then(|gi| gi.toplevel.as_deref());
                        // Only use relative path when pane's repo matches session's repo
                        let toplevel = if pane_toplevel == session_toplevel.as_deref() {
                            pane_toplevel
                        } else {
                            None
                        };
                        let label = relative_nvim_path(filename, dir, toplevel);
                        self.nvim_title_cache
                            .insert(pane.pane_id.clone(), label.clone());
                        pane.pane_title = label;
                    } else if let Some(cached) = self.nvim_title_cache.get(&pane.pane_id) {
                        pane.pane_title = cached.clone();
                    }
                }
            }
        }
    }

    fn rebuild_tree(&mut self) {
        self.tree_items = tree::flatten(&self.sessions, &self.collapsed);

        // On first selection, try to select the pane where chikuwa is running
        if self.first_selection {
            if let Ok(pane_id) = std::env::var("TMUX_PANE") {
                if let Some(idx) = tree::find_pane_index(&self.tree_items, &pane_id) {
                    self.selected = idx;
                    self.first_selection = false;
                }
            }
            // Fall back to active pane if TMUX_PANE not found
            if self.first_selection && !self.user_navigated {
                if let Some(active_idx) = tree::find_active_index(&self.sessions, &self.tree_items)
                {
                    self.selected = active_idx;
                }
            }
            self.first_selection = false;
        } else if !self.user_navigated {
            // Follow the active (focused) pane/window when user hasn't navigated manually
            if let Some(active_idx) = tree::find_active_index(&self.sessions, &self.tree_items) {
                self.selected = active_idx;
            }
        }

        // Clamp selected index
        if !self.tree_items.is_empty() && self.selected >= self.tree_items.len() {
            self.selected = self.tree_items.len() - 1;
        }
        // Ensure selected is not a Session item
        self.snap_to_selectable();
        // Clamp scroll offset to valid visual row range
        let total_visual = tree::total_visual_rows(&self.tree_items, self.last_width);
        if total_visual > 0 && self.scroll_offset >= total_visual {
            self.scroll_offset = total_visual - 1;
        }
    }

    /// Merge agent states into the existing session tree without re-fetching tmux.
    /// Only overwrites pane state when hook data exists for that pane — preserves
    /// detected "Waiting" state for panes that haven't sent hook events yet.
    fn merge_agent_states(&mut self) {
        let mut updated = 0u32;
        for session in &mut self.sessions {
            for window in &mut session.windows {
                for pane in &mut window.panes {
                    if let Some(state) = self.agent_states.get(&pane.pane_id) {
                        pane.agent_state = Some(state.clone());
                        updated += 1;
                    }
                }
            }
        }
        crate::ipc::debug_log(format!(
            "merge_agent_states: {} panes updated, sessions={}",
            updated,
            self.sessions.len(),
        ));
        self.rebuild_tree();
    }

    fn move_up(&mut self) {
        self.user_navigated = true;
        let mut idx = self.selected;
        while idx > 0 {
            idx -= 1;
            if self.tree_items[idx].is_selectable() {
                self.selected = idx;
                self.ensure_visible();
                return;
            }
        }
    }

    fn move_down(&mut self) {
        self.user_navigated = true;
        let mut idx = self.selected;
        while idx < self.tree_items.len().saturating_sub(1) {
            idx += 1;
            if self.tree_items[idx].is_selectable() {
                self.selected = idx;
                self.ensure_visible();
                return;
            }
        }
    }

    fn move_top(&mut self) {
        self.user_navigated = true;
        if let Some(idx) = self.tree_items.iter().position(|item| item.is_selectable()) {
            self.selected = idx;
        }
    }

    fn move_bottom(&mut self) {
        self.user_navigated = true;
        if let Some(idx) = self
            .tree_items
            .iter()
            .rposition(|item| item.is_selectable())
        {
            self.selected = idx;
        }
    }

    /// Snap selected to a selectable item if it currently points to a non-selectable one.
    fn snap_to_selectable(&mut self) {
        if self.tree_items.is_empty() {
            return;
        }
        if self.tree_items[self.selected].is_selectable() {
            return;
        }
        // Try forward first, then backward
        if let Some(offset) = self.tree_items[self.selected..]
            .iter()
            .position(|item| item.is_selectable())
        {
            self.selected += offset;
        } else if let Some(idx) = self.tree_items.iter().position(|item| item.is_selectable()) {
            self.selected = idx;
        }
    }

    fn ensure_visible(&mut self) {
        let visual = tree::item_to_visual_row(&self.tree_items, self.selected, self.last_width);
        let vh = self.visible_height;
        if vh == 0 {
            return;
        }
        let soff = self.scrolloff;
        // Bottom margin: selected must be at most (visible_height - soff - 1) from top
        if visual + soff >= self.scroll_offset + vh {
            self.scroll_offset = visual.saturating_sub(vh.saturating_sub(soff + 1));
        }
        // Top margin: selected must be at least soff from top
        if visual < self.scroll_offset + soff {
            self.scroll_offset = visual.saturating_sub(soff);
        }
    }

    /// Get all active subagents for a given tmux pane, sorted by update time (newest first).
    #[allow(dead_code)]
    fn get_subagents_for_pane(&self, pane_id: &str) -> Vec<&SubagentInfo> {
        let mut subagents: Vec<&SubagentInfo> = self
            .subagent_states
            .iter()
            .filter(|((pane, _), _)| pane == pane_id)
            .map(|(_, info)| info)
            .filter(|info| info.state != SubagentStatus::Ended)
            .collect();
        subagents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        subagents
    }

    /// Get completed subagent count for a pane.
    #[allow(dead_code)]
    fn get_completed_count(&self, pane_id: &str) -> u32 {
        *self.completed_subagent_counts.get(pane_id).unwrap_or(&0)
    }

    /// Build a map of pane_id -> (subagents, completed_count) for UI rendering.
    fn build_subagent_data(&self) -> HashMap<String, (Vec<SubagentInfo>, u32)> {
        let mut data: HashMap<String, (Vec<SubagentInfo>, u32)> = HashMap::new();

        for ((pane_id, _), info) in &self.subagent_states {
            data.entry(pane_id.clone())
                .or_insert_with(|| (Vec::new(), 0))
                .0
                .push(info.clone());
        }

        for (pane_id, count) in &self.completed_subagent_counts {
            data.entry(pane_id.clone())
                .or_insert_with(|| (Vec::new(), 0))
                .1 = *count;
        }

        data
    }

    /// Merge a subagent state update into the app state.
    fn merge_subagent_state(&mut self, pane_id: String, agent_id: String, state: AgentState) {
        use crate::agent::state::AgentStatus;

        // Handle subagent ended
        if state.status() == AgentStatus::Ended {
            // Only increment completed count if subagent actually existed
            let existed = self
                .subagent_states
                .remove(&(pane_id.clone(), agent_id.clone()))
                .is_some();
            if existed {
                *self
                    .completed_subagent_counts
                    .entry(pane_id.clone())
                    .or_insert(0) += 1;
            }

            // Persist ended state
            let ended_info = SubagentInfo {
                id: agent_id,
                short_id: String::new(),
                description: None,
                state: SubagentStatus::Ended,
                tools: Vec::new(),
                recent_tools: Vec::new(),
                updated_at: state.updated_at,
            };
            if let Err(e) = persist::append_subagent_state(&pane_id, &ended_info) {
                eprintln!("Warning: failed to persist subagent state: {}", e);
            }
            return;
        }

        // Get or create subagent info
        let entry = self
            .subagent_states
            .entry((pane_id.clone(), agent_id.clone()));

        match entry {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                let info = e.get_mut();
                let event = state.event_label();
                // PostToolUse is Silent: only update tools, preserve visual state
                if event != "PostToolUse" {
                    info.state = SubagentStatus::from(state.status());
                }
                info.updated_at = state.updated_at;

                // Merge tools based on event
                match event {
                    "PreToolUse" => {
                        if let Some(tool) = state.active_tools().first() {
                            if !info.tools.iter().any(|t| t.key == tool.key) {
                                info.tools.push(tool.clone());
                            }
                        }
                    }
                    "PostToolUse" | "PostToolUseFailure" => {
                        if let Some(removing) = state.active_tools().first() {
                            // Try exact key match first, fall back to name-only
                            if let Some(pos) = info
                                .tools
                                .iter()
                                .position(|t| t.key == removing.key)
                                .or_else(|| info.tools.iter().position(|t| t.name == removing.name))
                            {
                                let completed = info.tools.remove(pos);
                                push_recent_tool(&mut info.recent_tools, completed);
                            }
                        }
                    }
                    _ => {}
                }

                // Persist the updated state
                if let Err(e) = persist::append_subagent_state(&pane_id, info) {
                    eprintln!("Warning: failed to persist subagent state: {}", e);
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                // New subagent - extract description from Task tool if available
                let description = if state.current_tool_name() == Some("Task") {
                    state.active_tools().first().and_then(|t| t.detail.clone())
                } else {
                    None
                };

                let mut info = SubagentInfo::new(agent_id, description);
                info.state = SubagentStatus::from(state.status());
                info.updated_at = state.updated_at;
                if let Some(tool) = state.active_tools().first() {
                    info.tools.push(tool.clone());
                }

                // Persist the new state
                if let Err(e) = persist::append_subagent_state(&pane_id, &info) {
                    eprintln!("Warning: failed to persist subagent state: {}", e);
                }

                e.insert(info);
            }
        }
    }

    async fn handle_select(&mut self) -> Result<()> {
        if self.tree_items.is_empty() {
            return Ok(());
        }

        let item = &self.tree_items[self.selected];

        // Toggle collapse for sessions
        if let tree::TreeItem::Session { name, .. } = item {
            let name = name.clone();
            if self.collapsed.contains(&name) {
                self.collapsed.remove(&name);
            } else {
                self.collapsed.insert(name);
            }
            self.rebuild_tree();
            return Ok(());
        }

        // Switch tmux for windows/panes
        self.user_navigated = false;
        let target = item.tmux_target();
        if let Err(e) = tmux_client::switch_to(&target).await {
            eprintln!("Warning: failed to switch tmux: {}", e);
        }
        self.refresh().await?;

        Ok(())
    }

    /// Toggle collapse/expand for the session that the currently selected item belongs to.
    fn toggle_current_session(&mut self) {
        if self.tree_items.is_empty() {
            return;
        }

        // Find the session name for the current selected item by scanning backward
        let mut session_name: Option<String> = None;
        for i in (0..=self.selected).rev() {
            if let tree::TreeItem::Session { name, .. } = &self.tree_items[i] {
                session_name = Some(name.clone());
                break;
            }
        }

        if let Some(name) = session_name {
            if self.collapsed.contains(&name) {
                self.collapsed.remove(&name);
            } else {
                self.collapsed.insert(name);
            }
            self.rebuild_tree();
        }
    }
}

fn is_subagent_state(state: &AgentState) -> bool {
    state.agent_id().is_some()
        && matches!(
            state.source(),
            crate::agent::state::AgentSource::Claude | crate::agent::state::AgentSource::Codex
        )
}

/// Collect unique pane paths from sessions.
fn collect_pane_paths(sessions: &[TmuxSession]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for session in sessions {
        for window in &session.windows {
            for pane in &window.panes {
                if seen.insert(pane.pane_current_path.clone()) {
                    paths.push(pane.pane_current_path.clone());
                }
            }
        }
    }
    paths
}

fn apply_key_action(app: &mut App, action: Action) -> bool {
    match action {
        Action::Quit => {
            app.should_quit = true;
            return true;
        }
        Action::Up => app.move_up(),
        Action::Down => app.move_down(),
        Action::Top => app.move_top(),
        Action::Bottom => app.move_bottom(),
        Action::ToggleCollapse => app.toggle_current_session(),
        Action::Select | Action::ToggleView | Action::None => {}
    }

    false
}

/// Render the title bar into the given area.
fn render_title(f: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let has_running = app
        .agent_states
        .values()
        .any(|s| s.status() == crate::agent::state::AgentStatus::Running);
    let title_spans = if has_running {
        // Shine effect: white highlight sweeps across purple base
        let wave_palette = {
            let white: (f32, f32, f32) = (0xff as f32, 0xff as f32, 0xff as f32);
            let purple: (f32, f32, f32) = (0x92 as f32, 0x93 as f32, 0xfe as f32);
            // Sharp peak: 3 steps rise, 3 steps fall, long purple rest
            let total = 40;
            let peak = 3;
            let mut palette = Vec::with_capacity(total);
            for i in 0..total {
                let t = if i < peak {
                    // rise to white
                    i as f32 / peak as f32
                } else if i < peak * 2 {
                    // fall back to purple
                    1.0 - (i - peak) as f32 / peak as f32
                } else {
                    // stay purple
                    0.0
                };
                // ease-in-out for smoother peak
                let t = t * t * (3.0 - 2.0 * t);
                let r = purple.0 + (white.0 - purple.0) * t;
                let g = purple.1 + (white.1 - purple.1) * t;
                let b = purple.2 + (white.2 - purple.2) * t;
                palette.push(Color::Rgb(r as u8, g as u8, b as u8));
            }
            palette
        };
        let plen = wave_palette.len();
        let chikuwa_spans: Vec<Span> = "chikuwa"
            .chars()
            .enumerate()
            .map(|(i, c)| {
                let idx = (plen + i * 2 - app.anim_frame * 3 % plen) % plen;
                Span::styled(
                    c.to_string(),
                    Style::default()
                        .fg(wave_palette[idx])
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();
        let bolt_style = Style::default()
            .fg(theme::COLOR_YELLOW)
            .add_modifier(Modifier::BOLD);
        let white_style = Style::default()
            .fg(theme::COLOR_WHITE)
            .add_modifier(Modifier::BOLD);
        let mut spans = vec![
            Span::styled("🐧 ", white_style),
            Span::styled(theme::ICON_BOLT, bolt_style),
            Span::styled("  ", white_style),
        ];
        spans.extend(chikuwa_spans);
        spans.push(Span::styled(
            match app.view_mode {
                ViewMode::Tree => "  ",
                ViewMode::Office => ":pixtuoid  ",
            },
            white_style,
        ));
        spans.push(Span::styled(theme::ICON_BOLT, bolt_style));
        spans.push(Span::styled(" 🐧", white_style));
        spans
    } else {
        let bolt_style = Style::default()
            .fg(theme::COLOR_YELLOW)
            .add_modifier(Modifier::BOLD);
        let white_style = Style::default()
            .fg(theme::COLOR_WHITE)
            .add_modifier(Modifier::BOLD);
        vec![
            Span::styled("🐧 ", white_style),
            Span::styled(theme::ICON_BOLT, bolt_style),
            Span::styled(
                match app.view_mode {
                    ViewMode::Tree => "  chikuwa  ",
                    ViewMode::Office => "  chikuwa:pixtuoid  ",
                },
                white_style,
            ),
            Span::styled(theme::ICON_BOLT, bolt_style),
            Span::styled(" 🐧", white_style),
        ]
    };
    let title = vec![Line::from(""), Line::from(title_spans)];
    f.render_widget(Paragraph::new(title).alignment(Alignment::Center), area);
}

/// Render the status bar into the given area.
fn render_status_bar(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    sessions: &[TmuxSession],
    usage: Option<std::result::Result<&Usage, &String>>,
    usage_remaining: Option<u64>,
) {
    status_bar::render(f, area, sessions, usage, usage_remaining);
}

/// Run the TUI application.
pub async fn run() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        crossterm::terminal::SetTitle("🐧⚡️chikuwa ⚡️🐧")
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();

    // Event channel — create early so Stage 3 background tasks can use it
    let (tx, mut rx) = mpsc::channel(256);
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // ── Stage 1: Shell frame (before any I/O) ──
    terminal.draw(|f| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(f.area());
        render_title(f, chunks[0], &app);
        render_status_bar(f, chunks[2], &app.sessions, None, None);
    })?;

    // ── Stage 2: Tmux structure only (no git info) ──
    app.refresh_tree_only().await?;

    // Draw with tmux tree visible
    terminal.draw(|f| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(f.area());
        render_title(f, chunks[0], &app);
        let visible_height = chunks[1].height as usize;
        app.last_width = chunks[1].width;
        app.visible_height = visible_height;
        app.tree_area = chunks[1];
        app.ensure_visible();
        tree::render(
            f,
            chunks[1],
            &app.tree_items,
            app.selected,
            app.scroll_offset,
            app.anim_frame,
            &HashMap::new(),
        );
        render_status_bar(f, chunks[2], &app.sessions, None, None);
    })?;

    // ── Stage 3: Background git info fetch + hook registration ──
    let paths = collect_pane_paths(&app.sessions);
    let git_tx = tx.clone();
    for path in paths {
        let tx = git_tx.clone();
        tokio::spawn(async move {
            if let Some(info) = crate::git::fetch_git_info(&path).await {
                let _ = tx.send(AppEvent::GitInfoReady { path, info }).await;
            }
        });
    }
    // Hook registration (background, non-blocking)
    tokio::spawn(async move {
        if let Err(e) = tmux_client::register_hooks().await {
            eprintln!("Warning: failed to register tmux hooks: {}", e);
        }
    });

    // Spawn event loop in a blocking thread (crossterm events are blocking)
    // Use std::sync::mpsc to avoid nested block_on anti-pattern
    // Wrap in catch_unwind to prevent crossterm parsing panics from crashing the TUI
    let s = shutdown.clone();
    let (blocking_tx, blocking_rx) = std::sync::mpsc::channel::<AppEvent>();
    handles.push(tokio::task::spawn_blocking(move || loop {
        if s.load(Ordering::Relaxed) {
            break;
        }

        // Use catch_unwind to handle potential panics in crossterm event parsing
        // (e.g., integer overflow in parse_csi_sgr_mouse with malformed sequences)
        let poll_result =
            std::panic::catch_unwind(|| crossterm::event::poll(Duration::from_millis(100)));

        match poll_result {
            Ok(Ok(true)) => {
                // Successfully polled, event available
                let read_result = std::panic::catch_unwind(crossterm::event::read);
                if let Ok(Ok(evt)) = read_result {
                    #[allow(clippy::collapsible_match)]
                    match evt {
                        Event::Key(key) => {
                            if blocking_tx.send(AppEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                        Event::Mouse(mouse) => {
                            if blocking_tx.send(AppEvent::Mouse(mouse)).is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Ok(false)) => {
                // No event available, send tick
                if blocking_tx.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
            Ok(Err(e)) => {
                // crossterm error (not panic), log and continue
                eprintln!("[chikuwa] crossterm error: {:?}", e);
                // Send tick to keep the event loop running
                let _ = blocking_tx.send(AppEvent::Tick);
            }
            Err(panic_info) => {
                // Panic occurred during poll/read, log and continue
                eprintln!("[chikuwa] recovered from crossterm panic: {:?}", panic_info);

                // Try to consume the problematic event from the input buffer
                // by repeatedly calling read() until it succeeds or there's no more data
                // This prevents infinite panic loop from the same malformed event
                for _ in 0..10 {
                    match std::panic::catch_unwind(crossterm::event::read) {
                        Ok(Ok(_)) => break,  // Successfully consumed an event
                        Ok(Err(_)) => break, // No more events
                        Err(_) => continue,  // Another panic, try again
                    }
                }

                // Send tick to keep the event loop running (allows user to quit)
                let _ = blocking_tx.send(AppEvent::Tick);
            }
        }
    }));

    // Bridge blocking channel to async channel
    let event_tx = tx.clone();
    handles.push(tokio::spawn(async move {
        while let Ok(evt) = blocking_rx.recv() {
            if event_tx.send(evt).await.is_err() {
                break;
            }
        }
    }));

    // Start IPC socket listener
    let pid = std::process::id();
    let socket_path = ipc::instance_socket_path(pid);
    let _ = std::fs::create_dir_all(ipc::socket_dir());
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).ok();
    }
    let ipc_tx = tx.clone();
    let ipc_path = socket_path.clone();
    handles.push(tokio::spawn(async move {
        if let Err(e) = ipc::start_listener(&ipc_path, ipc_tx).await {
            eprintln!("IPC listener error: {}", e);
        }
    }));

    // Animation tick (100ms for smooth spinner)
    let anim_tx = tx.clone();
    handles.push(tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(150));
        loop {
            interval.tick().await;
            if anim_tx.send(AppEvent::AnimationTick).await.is_err() {
                break;
            }
        }
    }));

    // Usage polling (10 min base, exponential backoff on 429)
    let usage_tx = tx.clone();
    handles.push(tokio::spawn(async move {
        const BASE_INTERVAL: u64 = 600; // 10 minutes
        const MAX_INTERVAL: u64 = 3600; // 1 hour cap
        let mut current_interval = BASE_INTERVAL;
        let mut first = true;
        loop {
            if first {
                first = false;
            } else {
                tokio::time::sleep(Duration::from_secs(current_interval)).await;
            }
            match crate::usage::fetch_usage().await {
                crate::usage::FetchResult::Success(usage) => {
                    current_interval = BASE_INTERVAL;
                    if usage_tx
                        .send(AppEvent::UsageUpdate(usage, current_interval))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                crate::usage::FetchResult::RateLimited(msg) => {
                    current_interval = (current_interval * 2).min(MAX_INTERVAL);
                    if usage_tx
                        .send(AppEvent::UsageError(msg, current_interval))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                crate::usage::FetchResult::Error(msg) => {
                    if usage_tx
                        .send(AppEvent::UsageError(msg, current_interval))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    }));

    loop {
        // Draw
        terminal.draw(|f| {
            let size = f.area();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(3),
                    Constraint::Length(3),
                ])
                .split(size);

            render_title(f, chunks[0], &app);

            // Adjust scroll for visible area (visual rows, no outer border)
            app.tree_area = chunks[1];
            let visible_height = chunks[1].height as usize;
            app.last_width = chunks[1].width;
            app.visible_height = visible_height;

            let subagent_data = app.build_subagent_data();

            match app.view_mode {
                ViewMode::Tree => {
                    // Enforce scrolloff margin (handles both top and bottom)
                    app.ensure_visible();

                    tree::render(
                        f,
                        chunks[1],
                        &app.tree_items,
                        app.selected,
                        app.scroll_offset,
                        app.anim_frame,
                        &subagent_data,
                    );
                }
                ViewMode::Office => {
                    // Pixtuoid mode — no TUI rendering; handled in ToggleView action
                }
            }

            let usage_remaining = app.usage_next_fetch.map(|t| {
                t.saturating_duration_since(std::time::Instant::now())
                    .as_secs()
            });
            render_status_bar(
                f,
                chunks[2],
                &app.sessions,
                app.usage.as_ref().map(|r| r.as_ref()),
                usage_remaining,
            );
        })?;

        if app.should_quit {
            break;
        }

        // Handle events
        if let Some(evt) = rx.recv().await {
            match evt {
                AppEvent::Key(key) => {
                    let action = event::handle_key(key);
                    if action == Action::Select {
                        app.handle_select().await?;
                    } else if action == Action::ToggleView {
                        if app.view_mode == ViewMode::Tree {
                            disable_raw_mode()?;
                            execute!(
                                terminal.backend_mut(),
                                LeaveAlternateScreen,
                                DisableMouseCapture
                            )?;
                            terminal.show_cursor()?;

                            let status = tokio::process::Command::new("pixtuoid")
                                .arg("run")
                                .status()
                                .await;

                            enable_raw_mode()?;
                            execute!(
                                terminal.backend_mut(),
                                EnterAlternateScreen,
                                EnableMouseCapture
                            )?;
                            terminal.hide_cursor()?;

                            if let Err(e) = status {
                                eprintln!("Warning: pixtuoid failed: {}", e);
                            }

                            app.view_mode = ViewMode::Tree;
                            app.refresh().await?;
                        }
                    } else if apply_key_action(&mut app, action) {
                        break;
                    }
                }
                AppEvent::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            let area = app.tree_area;
                            if mouse.column >= area.x
                                && mouse.column < area.x + area.width
                                && mouse.row >= area.y
                                && mouse.row < area.y + area.height
                            {
                                let click_visual_row =
                                    app.scroll_offset + (mouse.row - area.y) as usize;
                                if let Some(item_idx) = tree::visual_row_to_item(
                                    &app.tree_items,
                                    click_visual_row,
                                    app.last_width,
                                ) {
                                    if let tree::TreeItem::Session { name, .. } =
                                        &app.tree_items[item_idx]
                                    {
                                        // Toggle collapse on session click
                                        let name = name.clone();
                                        if app.collapsed.contains(&name) {
                                            app.collapsed.remove(&name);
                                        } else {
                                            app.collapsed.insert(name);
                                        }
                                        app.rebuild_tree();
                                    } else if app.tree_items[item_idx].is_selectable() {
                                        app.selected = item_idx;
                                        app.handle_select().await?;
                                    }
                                }
                            }
                        }
                        MouseEventKind::ScrollUp => app.move_up(),
                        MouseEventKind::ScrollDown => app.move_down(),
                        _ => {}
                    }
                }
                AppEvent::Tick => {
                    if app.last_tick_refresh.elapsed() >= Duration::from_secs(2) {
                        app.refresh().await?;
                        app.last_tick_refresh = Instant::now();
                    }
                }
                AppEvent::TmuxChanged => {
                    app.refresh().await?;
                }
                AppEvent::AnimationTick => {
                    app.anim_frame = app.anim_frame.wrapping_add(1);
                }
                AppEvent::UsageUpdate(usage, next_secs) => {
                    app.usage = Some(Ok(usage));
                    app.usage_next_fetch =
                        Some(std::time::Instant::now() + Duration::from_secs(next_secs));
                }
                AppEvent::UsageError(msg, next_secs) => {
                    app.usage = Some(Err(msg));
                    app.usage_next_fetch =
                        Some(std::time::Instant::now() + Duration::from_secs(next_secs));
                }
                AppEvent::GitInfoReady { path, info } => {
                    app.pending_git_info.insert(path, info);
                    if !app.git_debounce_active {
                        app.git_debounce_active = true;
                        let flush_tx = tx.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            let _ = flush_tx.send(AppEvent::FlushGitInfo).await;
                        });
                    }
                }
                AppEvent::FlushGitInfo => {
                    app.git_debounce_active = false;
                    app.apply_pending_git_info();
                }
                AppEvent::AgentStateUpdate(state) => {
                    let state = *state;
                    crate::ipc::debug_log(format!(
                        "AgentStateUpdate: pane={} status={:?} source={:?} tools={} event={} session_id={:?}",
                        state.tmux_pane,
                        state.status(),
                        state.source(),
                        state.active_tools().len(),
                        state.event_label(),
                        state.session_id(),
                    ));
                    // Determine if this is a subagent event (Claude with agent_id)
                    if is_subagent_state(&state) {
                        crate::ipc::debug_log("  -> routing to subagent");
                        let pane_id = state.tmux_pane.clone();
                        let agent_id = state.agent_id().unwrap().to_string();
                        app.merge_subagent_state(pane_id, agent_id, state);
                    } else {
                        use crate::agent::state::AgentStatus;
                        if state.status() == AgentStatus::Ended {
                            crate::ipc::debug_log("  -> removing ended state");
                            app.agent_states.remove(&state.tmux_pane);
                        } else if let Some(existing) = app.agent_states.get(&state.tmux_pane) {
                            crate::ipc::debug_log(format!(
                                "  -> merging with existing (existing tools={})",
                                existing.active_tools().len(),
                            ));
                            let merged = state.merge_with(existing);
                            crate::ipc::debug_log(format!(
                                "  -> merged result: status={:?} tools={}",
                                merged.status(),
                                merged.active_tools().len(),
                            ));
                            app.agent_states.insert(merged.tmux_pane.clone(), merged);
                        } else {
                            crate::ipc::debug_log("  -> new agent state");
                            app.agent_states.insert(state.tmux_pane.clone(), state);
                        }
                    }
                    let before_tools = app
                        .sessions
                        .iter()
                        .flat_map(|s| s.windows.iter())
                        .flat_map(|w| w.panes.iter())
                        .filter(|p| p.pane_id == "%305")
                        .flat_map(|p| p.agent_state.as_ref().map(|a| a.active_tools().len()))
                        .next()
                        .unwrap_or(999);
                    app.merge_agent_states();
                    let after_tools = app
                        .sessions
                        .iter()
                        .flat_map(|s| s.windows.iter())
                        .flat_map(|w| w.panes.iter())
                        .filter(|p| p.pane_id == "%305")
                        .flat_map(|p| p.agent_state.as_ref().map(|a| a.active_tools().len()))
                        .next()
                        .unwrap_or(999);
                    crate::ipc::debug_log(format!(
                        "  -> after merge_agent_states: pane %305 tools before={} after={} sessions_in_tree={}",
                        before_tools, after_tools, app.sessions.len(),
                    ));
                }
            }
        }
    }

    // === SHUTDOWN ===
    shutdown.store(true, Ordering::Relaxed);
    for h in &handles {
        h.abort();
    }
    drop(tx);
    for h in handles {
        let _ = h.await;
    }
    tmux_client::unregister_hooks().await;
    ipc::cleanup_instance_socket(std::process::id());

    // Persist git cache on shutdown
    let git_entries = app.git_cache.to_cache_entries();
    if let Err(e) = persist::save_git_cache(&git_entries) {
        eprintln!("Warning: failed to save git cache: {}", e);
    }

    // Persist usage data on shutdown
    if let Some(Ok(ref usage)) = app.usage {
        if let Err(e) = persist::save_usage(usage) {
            eprintln!("Warning: failed to save usage data: {}", e);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::claude::ClaudeState;
    use crate::agent::state::AgentData;
    use crate::tmux::types::{TmuxPane, TmuxSession, TmuxWindow};

    fn make_nvim_pane(pane_id: &str, title: &str) -> TmuxPane {
        TmuxPane {
            pane_id: pane_id.to_string(),
            pane_index: 0,
            pane_current_command: "nvim".to_string(),
            pane_current_path: "/home/user".to_string(),
            pane_title: title.to_string(),
            pane_active: true,
            agent_state: None,
            git_info: None,
        }
    }

    fn make_session(panes: Vec<TmuxPane>) -> TmuxSession {
        TmuxSession {
            session_name: "test".to_string(),
            session_attached: true,
            windows: vec![TmuxWindow {
                window_index: 0,
                window_name: "nvim".to_string(),
                window_active: true,
                panes,
            }],
            repo_name: None,
            toplevel: None,
            worktree_name: None,
        }
    }

    #[test]
    fn test_codex_agent_id_routes_as_subagent() {
        let state = AgentState::new(
            "%1".to_string(),
            AgentData::Codex(crate::agent::codex_state::CodexState {
                session_id: Some("sess-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                status: crate::agent::state::AgentStatus::Running,
                hook_event_name: "SubagentStart".to_string(),
                event_emoji: "🤖".to_string(),
                tool_name: None,
                tool_detail: None,
                active_tools: Vec::new(),
                recent_tools: Vec::new(),
                failure_detail: None,
                turn_id: Some("turn-1".to_string()),
                permission_mode: None,
                model: None,
                cwd: None,
                agent_type: Some("code-review".to_string()),
                transcript_path: None,
            }),
        );

        assert!(is_subagent_state(&state));
    }

    #[test]
    fn quit_action_requests_main_loop_exit() {
        let mut app = App::new();

        assert!(apply_key_action(&mut app, Action::Quit));
        assert!(app.should_quit);
    }

    #[test]
    fn test_extract_nvim_file_info_standard_format() {
        assert_eq!(
            extract_nvim_file_info("theme.rs (~/src/project/src/ui) - Nvim"),
            Some(("theme.rs", Some("~/src/project/src/ui")))
        );
        assert_eq!(
            extract_nvim_file_info("CLAUDE.md (~/src/project/.claude) - Nvim"),
            Some(("CLAUDE.md", Some("~/src/project/.claude")))
        );
    }

    #[test]
    fn test_extract_nvim_file_info_no_dir() {
        assert_eq!(
            extract_nvim_file_info("app.rs - Nvim"),
            Some(("app.rs", None))
        );
    }

    #[test]
    fn test_extract_nvim_file_info_bare() {
        assert_eq!(extract_nvim_file_info("app.rs"), Some(("app.rs", None)));
    }

    #[test]
    fn test_extract_nvim_file_info_nerdfont_path() {
        // New format: NerdFont icon + relative path
        assert_eq!(
            extract_nvim_file_info("\u{e7c5} programs/claude/settings.json"),
            Some(("programs/claude/settings.json", None))
        );
        assert_eq!(
            extract_nvim_file_info("\u{e7c5} src/main.rs"),
            Some(("src/main.rs", None))
        );
    }

    #[test]
    fn test_extract_nvim_file_info_nerdfont_bare() {
        assert_eq!(
            extract_nvim_file_info("\u{e7c5} main.rs"),
            Some(("main.rs", None))
        );
    }

    #[test]
    fn test_extract_nvim_file_info_invalid() {
        assert_eq!(extract_nvim_file_info(""), None);
        assert_eq!(extract_nvim_file_info("neo-tree filesystem [1]"), None);
        assert_eq!(
            extract_nvim_file_info("neo-tree filesystem [1] - Nvim"),
            None
        );
        assert_eq!(extract_nvim_file_info("[No Name] - Nvim"), None);
        assert_eq!(extract_nvim_file_info("term://something"), None);
    }

    #[test]
    fn test_strip_leading_icon() {
        assert_eq!(strip_leading_icon("\u{e7c5} src/main.rs"), "src/main.rs");
        assert_eq!(strip_leading_icon("\u{f489} terminal"), "terminal");
        assert_eq!(strip_leading_icon("no icon"), "no icon");
        assert_eq!(strip_leading_icon(""), "");
    }

    #[test]
    fn test_relative_nvim_path_with_toplevel() {
        std::env::set_var("HOME", "/home/user");
        assert_eq!(
            relative_nvim_path(
                "theme.rs",
                Some("~/src/project/src/ui"),
                Some("/home/user/src/project")
            ),
            "project/src/ui/theme.rs"
        );
    }

    #[test]
    fn test_relative_nvim_path_no_dir() {
        assert_eq!(
            relative_nvim_path("app.rs", None, Some("/project")),
            "app.rs"
        );
    }

    #[test]
    fn test_relative_nvim_path_no_toplevel() {
        assert_eq!(
            relative_nvim_path("app.rs", Some("~/project/src"), None),
            "app.rs"
        );
    }

    #[test]
    fn test_relative_nvim_path_new_format() {
        // New format: relative path from repo root, dir is None
        assert_eq!(
            relative_nvim_path("src/main.rs", None, Some("/home/user/project")),
            "project/src/main.rs"
        );
    }

    #[test]
    fn test_relative_nvim_path_new_format_long() {
        // Long relative path should be abbreviated
        let result = relative_nvim_path(
            "tmp/long/deep/path/from/repo/root/to/test/filename",
            None,
            Some("/home/user/project"),
        );
        assert!(result.len() <= 30 || !result.contains("long"));
        assert!(result.ends_with("filename"));
    }

    #[test]
    fn test_relative_nvim_path_abbreviation() {
        std::env::set_var("HOME", "/home/user");
        // A long relative path should be abbreviated
        let result = relative_nvim_path(
            "very_long_filename.rs",
            Some("~/project/src/deeply/nested/directory"),
            Some("/home/user/project"),
        );
        // "src/deeply/nested/directory/very_long_filename.rs" is > 30 chars
        // Should abbreviate to something like "s/d/n/directory/very_long_filename.rs"
        assert!(result.len() <= 30 || !result.contains("deeply"));
        assert!(result.ends_with("very_long_filename.rs"));
    }

    #[test]
    fn test_fixup_computes_relative_path() {
        std::env::set_var("HOME", "/home/user");
        let mut app = App::new();
        let mut pane = make_nvim_pane("%0", "theme.rs (~/project/src/ui) - Nvim");
        pane.git_info = Some(crate::git::GitInfo {
            branch: None,
            pr: None,
            repo_name: None,
            toplevel: Some("/home/user/project".to_string()),
            worktree_name: None,
        });
        let mut session = make_session(vec![pane]);
        session.toplevel = Some("/home/user/project".to_string());
        app.sessions = vec![session];

        app.fixup_nvim_titles();

        assert_eq!(
            app.sessions[0].windows[0].panes[0].pane_title,
            "project/src/ui/theme.rs"
        );
    }

    #[test]
    fn test_fixup_restores_cached_title_for_plugin_ui() {
        std::env::set_var("HOME", "/home/user");
        let mut app = App::new();
        let mut pane = make_nvim_pane("%0", "app.rs (~/project/src) - Nvim");
        pane.git_info = Some(crate::git::GitInfo {
            branch: None,
            pr: None,
            repo_name: None,
            toplevel: Some("/home/user/project".to_string()),
            worktree_name: None,
        });
        let mut session = make_session(vec![pane]);
        session.toplevel = Some("/home/user/project".to_string());
        app.sessions = vec![session];
        app.fixup_nvim_titles();

        // Second refresh: plugin UI title → restored from cache
        app.sessions = vec![make_session(vec![make_nvim_pane(
            "%0",
            "neo-tree filesystem [1]",
        )])];
        app.fixup_nvim_titles();

        assert_eq!(
            app.sessions[0].windows[0].panes[0].pane_title,
            "project/src/app.rs"
        );
    }

    #[test]
    fn test_fixup_no_cache_leaves_invalid_title() {
        let mut app = App::new();
        app.sessions = vec![make_session(vec![make_nvim_pane(
            "%0",
            "neo-tree filesystem [1]",
        )])];
        app.fixup_nvim_titles();

        assert_eq!(
            app.sessions[0].windows[0].panes[0].pane_title,
            "neo-tree filesystem [1]"
        );
    }

    #[test]
    fn test_fixup_skips_non_nvim_panes() {
        let mut app = App::new();
        let mut pane = make_nvim_pane("%0", "some title with spaces");
        pane.pane_current_command = "zsh".to_string();
        app.sessions = vec![make_session(vec![pane])];

        app.fixup_nvim_titles();

        assert!(app.nvim_title_cache.is_empty());
    }

    #[test]
    fn test_fixup_updates_cache_on_file_change() {
        std::env::set_var("HOME", "/home/user");
        let mut app = App::new();
        let mut pane = make_nvim_pane("%0", "app.rs (~/project/src) - Nvim");
        pane.git_info = Some(crate::git::GitInfo {
            branch: None,
            pr: None,
            repo_name: None,
            toplevel: Some("/home/user/project".to_string()),
            worktree_name: None,
        });
        let mut session = make_session(vec![pane]);
        session.toplevel = Some("/home/user/project".to_string());
        app.sessions = vec![session];
        app.fixup_nvim_titles();
        assert_eq!(
            app.nvim_title_cache.get("%0").unwrap(),
            "project/src/app.rs"
        );

        let mut pane2 = make_nvim_pane("%0", "main.rs (~/project/src) - Nvim");
        pane2.git_info = Some(crate::git::GitInfo {
            branch: None,
            pr: None,
            repo_name: None,
            toplevel: Some("/home/user/project".to_string()),
            worktree_name: None,
        });
        let mut session2 = make_session(vec![pane2]);
        session2.toplevel = Some("/home/user/project".to_string());
        app.sessions = vec![session2];
        app.fixup_nvim_titles();
        assert_eq!(
            app.nvim_title_cache.get("%0").unwrap(),
            "project/src/main.rs"
        );
    }

    #[test]
    fn test_fixup_mismatched_session_toplevel_falls_back_to_filename() {
        std::env::set_var("HOME", "/home/user");
        let mut app = App::new();
        let mut pane = make_nvim_pane("%0", "theme.rs (~/chikuwa/src/ui) - Nvim");
        pane.git_info = Some(crate::git::GitInfo {
            branch: None,
            pr: None,
            repo_name: None,
            toplevel: Some("/home/user/chikuwa".to_string()),
            worktree_name: None,
        });
        // Session belongs to a different repo
        let mut session = make_session(vec![pane]);
        session.toplevel = Some("/home/user/other-project".to_string());
        app.sessions = vec![session];

        app.fixup_nvim_titles();

        // Should fall back to just the filename since repos don't match
        assert_eq!(app.sessions[0].windows[0].panes[0].pane_title, "theme.rs");
    }

    #[test]
    fn test_merge_subagent_state_new() {
        // Clear persisted state to isolate test
        let _ = std::fs::remove_file(crate::persist::subagent_states_path());
        let _ = std::fs::remove_file(crate::persist::agent_states_path());

        let mut app = App::new();
        let state = AgentState::new(
            "%test_new".to_string(),
            AgentData::Claude(ClaudeState {
                session_id: None,
                agent_id: Some("sub_new_123".to_string()),
                status: crate::agent::state::AgentStatus::Running,
                hook_event_name: "SubagentStart".to_string(),
                event_emoji: "🤖".to_string(),
                tool_name: Some("Task".to_string()),
                tool_detail: None,
                active_tools: vec![crate::agent::state::ActiveTool {
                    key: crate::agent::state::ToolKey::Claude {
                        tool_use_id: "toolu_sub_new".to_string(),
                    },
                    name: "Task".to_string(),
                    detail: Some("Search codebase".to_string()),
                    failure_detail: None,
                }],
                recent_tools: Vec::new(),
                failure_detail: None,
            }),
        );

        app.merge_subagent_state("%test_new".to_string(), "sub_new_123".to_string(), state);

        let subagents = app.get_subagents_for_pane("%test_new");
        assert_eq!(subagents.len(), 1);
        assert_eq!(
            subagents[0].description,
            Some("Search codebase".to_string())
        );
    }

    #[test]
    fn test_merge_subagent_state_ended() {
        // Clear persisted state to isolate test
        let _ = std::fs::remove_file(crate::persist::subagent_states_path());
        let _ = std::fs::remove_file(crate::persist::agent_states_path());

        let mut app = App::new();

        // First add a running subagent
        let running_state = AgentState::new(
            "%test_end".to_string(),
            AgentData::Claude(ClaudeState {
                session_id: None,
                agent_id: Some("sub_end_456".to_string()),
                status: crate::agent::state::AgentStatus::Running,
                hook_event_name: "SubagentStart".to_string(),
                event_emoji: "🤖".to_string(),
                tool_name: None,
                tool_detail: None,
                active_tools: vec![],
                recent_tools: Vec::new(),
                failure_detail: None,
            }),
        );
        app.merge_subagent_state(
            "%test_end".to_string(),
            "sub_end_456".to_string(),
            running_state,
        );

        // Then end it
        let ended_state = AgentState::new(
            "%test_end".to_string(),
            AgentData::Claude(ClaudeState {
                session_id: None,
                agent_id: Some("sub_end_456".to_string()),
                status: crate::agent::state::AgentStatus::Ended,
                hook_event_name: "SubagentStop".to_string(),
                event_emoji: "🏁".to_string(),
                tool_name: None,
                tool_detail: None,
                active_tools: vec![],
                recent_tools: Vec::new(),
                failure_detail: None,
            }),
        );
        app.merge_subagent_state(
            "%test_end".to_string(),
            "sub_end_456".to_string(),
            ended_state,
        );

        assert_eq!(app.get_subagents_for_pane("%test_end").len(), 0);
        assert_eq!(app.get_completed_count("%test_end"), 1);
    }
}
