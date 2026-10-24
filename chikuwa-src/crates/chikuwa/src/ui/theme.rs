use ratatui::style::{Color, Style};

use crate::agent::state::AgentStatus;

// 3-color palette
pub const COLOR_WHITE: Color = Color::Rgb(0xff, 0xff, 0xff);
pub const COLOR_PURPLE: Color = Color::Rgb(0x92, 0x93, 0xfe);
pub const COLOR_LIGHT_PURPLE: Color = Color::Rgb(0xb6, 0xb9, 0xff);

// NerdFont icons
pub const ICON_CARET_RIGHT: &str = "\u{f0da}"; //
pub const ICON_SESSION: &str = "\u{ebc8}";
pub const ICON_GITHUB: &str = "\u{f09b}";
pub const ICON_GIT_BRANCH: &str = "\u{e725}"; //
pub const ICON_PR: &str = "\u{f407}"; //
pub const SPINNER_FRAMES: &[&str] = &["·", "✢", "*", "✶", "✻", "✽"];
pub const ICON_WAITING: &str = "\u{f28b}"; //
pub const ICON_PERMISSION: &str = "\u{f071}"; //
pub const ICON_STARTED: &str = "\u{f04b}"; //
pub const ICON_CLAUDE: &str = "\u{f06a9}"; // 󰚩
pub const ICON_OPENCODE: &str = "\u{2600}"; // ☀
pub const ICON_NEOVIM: &str = "\u{e7c5}"; //
pub const ICON_TERMINAL: &str = "\u{f489}"; //
pub const ICON_WINDOW: &str = "\u{f10aa}"; // 󱂪
pub const ICON_BOLT: &str = "\u{f0e7}"; //
pub const ICON_TOOL: &str = "\u{f0ad}"; //
pub const ICON_FAILURE: &str = "\u{f00d}"; //  cross
pub const COLOR_YELLOW: Color = Color::Rgb(0xff, 0xd7, 0x00);
pub const COLOR_PERMISSION_BG: Color = Color::Rgb(0x3a, 0x1a, 0x3a);

// UI accent colors (exception: not in the 3-color palette, used for dim/selection)
pub const COLOR_DIM: Color = Color::Rgb(0x7a, 0x7a, 0x7a);
pub const COLOR_SELECTED_BG: Color = Color::DarkGray;
pub const COLOR_FAILURE: Color = Color::Rgb(0xff, 0x44, 0x44);
/// Return a color for a usage utilization value (0.0–1.0).
pub fn usage_color(utilization: f64) -> Color {
    if utilization < 0.8 {
        COLOR_LIGHT_PURPLE
    } else {
        COLOR_WHITE
    }
}

pub fn status_icon(status: &AgentStatus, anim_frame: usize) -> &'static str {
    match status {
        AgentStatus::Running => SPINNER_FRAMES[anim_frame % SPINNER_FRAMES.len()],
        AgentStatus::Waiting => ICON_WAITING,
        AgentStatus::Permission => ICON_PERMISSION,
        AgentStatus::Started => ICON_STARTED,
        AgentStatus::Ended => ICON_STARTED,
    }
}

pub fn status_color(status: &AgentStatus, session_attached: bool) -> Color {
    match status {
        AgentStatus::Permission | AgentStatus::Waiting => {
            if session_attached {
                COLOR_LIGHT_PURPLE
            } else {
                COLOR_PURPLE
            }
        }
        _ => COLOR_DIM,
    }
}

/// Face emoji for agent status, animated only when working or needing attention.
/// Running and Permission animate; all other states are static.
/// Time-escalated: Permission and Waiting frames intensify with elapsed time.
pub fn agent_face_emoji(
    status: &AgentStatus,
    has_failure: bool,
    anim_frame: usize,
    elapsed_secs: u64,
) -> &'static str {
    match status {
        AgentStatus::Started => "🟢",
        AgentStatus::Running => {
            let frames = ["⚙️", "🔧"];
            frames[anim_frame % frames.len()]
        }
        AgentStatus::Permission => {
            let frames = if elapsed_secs < 30 {
                ["🥺✋", "🙁🤚"]
            } else if elapsed_secs < 60 {
                ["😯🙋", "😫🙋"]
            } else {
                ["😫🙋", "😫🙋‍♂️"]
            };
            frames[anim_frame % frames.len()]
        }
        AgentStatus::Waiting => {
            let frames = if elapsed_secs < 30 {
                ["😴", "😪"]
            } else if elapsed_secs < 90 {
                ["😪", "🥱"]
            } else {
                ["🥱", "😵‍💫"]
            };
            frames[anim_frame % frames.len()]
        }
        AgentStatus::Ended => {
            if has_failure {
                "❌"
            } else {
                "✅"
            }
        }
    }
}

/// Permission warning text, escalating with wait time.
pub fn permission_warning_text(elapsed_secs: u64) -> &'static str {
    if elapsed_secs < 30 {
        "🟡 NEED USER INPUT ⚠️"
    } else if elapsed_secs < 60 {
        "🟠 AWAITING YOUR RESPONSE ⚠️"
    } else {
        "🔴 PLEASE INPUT ASAP ⚠️"
    }
}

/// Number of 💤 emojis for idle state, based on elapsed time.
pub fn idle_zzz_count(elapsed_secs: u64) -> usize {
    if elapsed_secs < 30 {
        1
    } else if elapsed_secs < 90 {
        2
    } else {
        3
    }
}

/// Duration label prefix based on agent status.
pub fn duration_label(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "Running",
        AgentStatus::Permission => "Waited",
        AgentStatus::Waiting => "Idle",
        AgentStatus::Started => "",
        AgentStatus::Ended => "Done",
    }
}

pub const TOOL_SPINNER_FRAMES: &[&str] = &["◐", "◓", "◑", "◒"];

pub fn tool_spinner(anim_frame: usize) -> &'static str {
    TOOL_SPINNER_FRAMES[anim_frame % TOOL_SPINNER_FRAMES.len()]
}

/// Static emoji for hook event names, used in title bar right side.
pub fn event_emoji(hook_event_name: &str) -> &'static str {
    match hook_event_name {
        "PreToolUse" => "🪝",
        "PostToolUse" => "🟩",
        "PostToolUseFailure" => "🟥",
        "UserPromptSubmit" | "UserPromptExpansion" => "✍️",
        "SubagentStart" => "👶",
        "SubagentStop" => "🔀",
        "Stop" => "✅",
        "StopFailure" => "❌",
        "PermissionRequest" | "PermissionDenied" => "🔐",
        "PreCompact" | "PostCompact" => "🗜️",
        "SessionStart" => "🟢",
        "SessionEnd" => "🏁",
        _ => "🔧",
    }
}

pub fn status_style(status: &AgentStatus, session_attached: bool) -> Style {
    Style::default().fg(status_color(status, session_attached))
}

pub fn branch_style() -> Style {
    Style::default().fg(COLOR_LIGHT_PURPLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_spinner_cycles() {
        assert_eq!(tool_spinner(0), "◐");
        assert_eq!(tool_spinner(1), "◓");
        assert_eq!(tool_spinner(2), "◑");
        assert_eq!(tool_spinner(3), "◒");
        assert_eq!(tool_spinner(4), "◐"); // wraps
    }

    #[test]
    fn test_agent_face_emoji_started_static() {
        assert_eq!(agent_face_emoji(&AgentStatus::Started, false, 0, 0), "🟢");
        assert_eq!(agent_face_emoji(&AgentStatus::Started, false, 1, 100), "🟢");
    }

    #[test]
    fn test_agent_face_emoji_running_animated() {
        assert_eq!(agent_face_emoji(&AgentStatus::Running, false, 0, 0), "⚙️");
        assert_eq!(agent_face_emoji(&AgentStatus::Running, false, 1, 0), "🔧");
    }

    #[test]
    fn test_agent_face_emoji_permission_escalation() {
        // 0-30s: mild
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 0, 0),
            "🥺✋"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 1, 29),
            "🙁🤚"
        );
        // 30-60s: urgent
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 0, 30),
            "😯🙋"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 1, 59),
            "😫🙋"
        );
        // 60s+: critical
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 0, 60),
            "😫🙋"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 1, 120),
            "😫🙋‍♂️"
        );
    }

    #[test]
    fn test_agent_face_emoji_waiting_escalation() {
        // 0-30s: light drowsiness
        assert_eq!(agent_face_emoji(&AgentStatus::Waiting, false, 0, 0), "😴");
        assert_eq!(agent_face_emoji(&AgentStatus::Waiting, false, 1, 29), "😪");
        // 30-90s: yawning
        assert_eq!(agent_face_emoji(&AgentStatus::Waiting, false, 0, 30), "😪");
        assert_eq!(agent_face_emoji(&AgentStatus::Waiting, false, 1, 89), "🥱");
        // 90s+: deep sleep
        assert_eq!(agent_face_emoji(&AgentStatus::Waiting, false, 0, 90), "🥱");
        assert_eq!(agent_face_emoji(&AgentStatus::Waiting, false, 1, 200), "😵‍💫");
    }

    #[test]
    fn test_agent_face_emoji_ended() {
        assert_eq!(agent_face_emoji(&AgentStatus::Ended, false, 0, 0), "✅");
        assert_eq!(agent_face_emoji(&AgentStatus::Ended, true, 0, 0), "❌");
    }

    #[test]
    fn test_permission_warning_text() {
        assert_eq!(permission_warning_text(0), "🟡 NEED USER INPUT ⚠️");
        assert_eq!(permission_warning_text(29), "🟡 NEED USER INPUT ⚠️");
        assert_eq!(permission_warning_text(30), "🟠 AWAITING YOUR RESPONSE ⚠️");
        assert_eq!(permission_warning_text(59), "🟠 AWAITING YOUR RESPONSE ⚠️");
        assert_eq!(permission_warning_text(60), "🔴 PLEASE INPUT ASAP ⚠️");
        assert_eq!(permission_warning_text(300), "🔴 PLEASE INPUT ASAP ⚠️");
    }

    #[test]
    fn test_idle_zzz_count() {
        assert_eq!(idle_zzz_count(0), 1);
        assert_eq!(idle_zzz_count(29), 1);
        assert_eq!(idle_zzz_count(30), 2);
        assert_eq!(idle_zzz_count(89), 2);
        assert_eq!(idle_zzz_count(90), 3);
        assert_eq!(idle_zzz_count(300), 3);
    }

    #[test]
    fn test_duration_label() {
        assert_eq!(duration_label(&AgentStatus::Running), "Running");
        assert_eq!(duration_label(&AgentStatus::Permission), "Waited");
        assert_eq!(duration_label(&AgentStatus::Waiting), "Idle");
        assert_eq!(duration_label(&AgentStatus::Started), "");
        assert_eq!(duration_label(&AgentStatus::Ended), "Done");
    }
}
