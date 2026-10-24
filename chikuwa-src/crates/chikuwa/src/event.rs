use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::agent::state::AgentState;
use crate::git::GitInfo;
use crate::usage::Usage;

/// Application events.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Tick,
    AnimationTick,
    AgentStateUpdate(Box<AgentState>),
    TmuxChanged,
    /// Usage data fetched successfully. Second field is seconds until next fetch.
    UsageUpdate(Usage, u64),
    /// Usage fetch failed. Second field is seconds until next fetch.
    UsageError(String, u64),
    /// Git info fetched for a pane path (ready for incremental update).
    GitInfoReady {
        path: String,
        info: GitInfo,
    },
    /// Debounce timer expired — apply all pending git info and redraw.
    FlushGitInfo,
}

/// Process a key event and return an action.
pub fn handle_key(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Action::ToggleCollapse
        }
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::Char('h') | KeyCode::Left => Action::ToggleView,
        KeyCode::Char('l') | KeyCode::Right => Action::ToggleView,
        KeyCode::Enter | KeyCode::Char(' ') => Action::Select,
        KeyCode::Char('g') => Action::Top,
        KeyCode::Char('G') => Action::Bottom,
        _ => Action::None,
    }
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Quit,
    Up,
    Down,
    Select,
    Top,
    Bottom,
    ToggleCollapse,
    ToggleView,
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn key_with_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn test_quit_q() {
        assert_eq!(handle_key(key(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn test_quit_ctrl_c() {
        assert_eq!(
            handle_key(key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn test_navigation_jk() {
        assert_eq!(handle_key(key(KeyCode::Char('j'))), Action::Down);
        assert_eq!(handle_key(key(KeyCode::Char('k'))), Action::Up);
    }

    #[test]
    fn test_navigation_arrows() {
        assert_eq!(handle_key(key(KeyCode::Down)), Action::Down);
        assert_eq!(handle_key(key(KeyCode::Up)), Action::Up);
    }

    #[test]
    fn test_select_enter() {
        assert_eq!(handle_key(key(KeyCode::Enter)), Action::Select);
    }

    #[test]
    fn test_select_space() {
        assert_eq!(handle_key(key(KeyCode::Char(' '))), Action::Select);
    }

    #[test]
    fn test_top_bottom() {
        assert_eq!(handle_key(key(KeyCode::Char('g'))), Action::Top);
        assert_eq!(handle_key(key(KeyCode::Char('G'))), Action::Bottom);
    }

    #[test]
    fn test_unknown_key() {
        assert_eq!(handle_key(key(KeyCode::Char('x'))), Action::None);
        assert_eq!(handle_key(key(KeyCode::F(1))), Action::None);
    }

    #[test]
    fn test_toggle_collapse_ctrl_k() {
        assert_eq!(
            handle_key(key_with_mod(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            Action::ToggleCollapse
        );
        // Plain 'k' is still Up
        assert_eq!(handle_key(key(KeyCode::Char('k'))), Action::Up);
    }

    #[test]
    fn test_toggle_view_hl() {
        assert_eq!(handle_key(key(KeyCode::Char('h'))), Action::ToggleView);
        assert_eq!(handle_key(key(KeyCode::Char('l'))), Action::ToggleView);
        assert_eq!(handle_key(key(KeyCode::Left)), Action::ToggleView);
        assert_eq!(handle_key(key(KeyCode::Right)), Action::ToggleView);
    }
}
