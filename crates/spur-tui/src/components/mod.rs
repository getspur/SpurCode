pub mod activity_log;
pub mod agents_tree;
pub mod brain_query_source;
pub mod collision_modal;
pub mod command_input_query_source;
pub mod completion_popup;
pub mod completion_trigger;
pub mod config_option_query_source;
pub mod detail_pane;
pub mod diff_viewer;
pub mod execute_modal;
pub mod help_overlay;
#[cfg(feature = "markdown")]
pub mod image_cache;
pub mod inline_executor_card;
pub mod input_bar;
pub mod input_bar_mention_hint;
pub mod input_bar_wrap;
pub mod input_completion;
pub mod issue_comments_pane;
pub mod issue_detail_pane;
pub mod issue_graph_pane;
pub mod issue_utils;
pub mod issues_panel;
pub mod keyhint;
pub mod line_wrap;
#[cfg(feature = "markdown")]
pub mod markdown_stream;
#[cfg(feature = "markdown")]
pub mod mermaid;
pub mod mini_input;
pub mod palette;
pub mod palette_overlay;
pub mod palette_sources;
pub(crate) mod paste_burst;
pub mod picker_popover;
pub mod picker_shell;
pub mod plan_pulse;
pub mod plan_stage_board;
pub mod plan_task_detail;
pub(crate) mod plan_text;
pub mod query_source;
pub mod quit_confirm;
pub mod react_trace;
pub mod resume_banner;
pub mod review_card;
pub mod session_preview;
pub mod snippet;
pub mod spinner;
pub mod status_bar;
pub mod stream_pane;
pub mod theme_query_source;
pub mod tombstone;
pub(crate) mod trace_format;
pub mod upgrade_modal;
pub mod workers_panel;

use ratatui::style::{Color, Style};
use std::time::Instant;

// ─── Shared constants ───────────────────────────────────────────────

// Re-export for backward compatibility with existing consumers.
// Prefer `spinner::BRAILLE` for new code.
pub use spinner::BRAILLE as SPINNER_FRAMES;

// ─── Shared utility functions ───────────────────────────────────────

/// Current local time formatted as HH:MM:SS.
pub fn now_stamp() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// Format an elapsed duration from a start time as `Xm YYs`.
pub fn format_elapsed(started_at: Instant) -> String {
    let secs = started_at.elapsed().as_secs();
    format!("{}m {:02}s", secs / 60, secs % 60)
}

/// Border style for focused (cyan) vs unfocused (dark gray) panels.
pub fn focused_border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// A single entry in the activity log.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub prefix: String,
    pub message: String,
    pub kind: LogEntryKind,
}

/// What kind of log entry this is (for styling and filtering).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEntryKind {
    Think,
    Act,
    Observe,
    Delegate,
    Complete,
    Error,
    UserMessage,
    Permission,
    Info,
}

/// Maximum log entries before oldest are evicted.
pub const MAX_LOG_ENTRIES: usize = 5_000;
