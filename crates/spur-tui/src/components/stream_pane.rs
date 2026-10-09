use std::borrow::Cow;

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::components::react_trace::ReactTrace;

/// Scroll + follow state for a Stream view. Owned independently by every
/// caller of [`render_stream`] so two stream views can scroll without
/// interfering with each other.
#[derive(Debug, Clone)]
pub struct StreamViewState {
    pub scroll_offset: usize,
    pub is_following: bool,
}

impl Default for StreamViewState {
    fn default() -> Self {
        Self {
            scroll_offset: 0,
            is_following: true,
        }
    }
}

impl StreamViewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scroll_up_by(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        self.is_following = false;
    }

    pub fn scroll_down_by(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(n);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.is_following = false;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.is_following = true;
    }

    pub fn toggle_follow(&mut self) {
        self.is_following = !self.is_following;
    }
}

/// Pure label helper, identical to `detail_pane::scroll_label` for the
/// Stream tab. Kept here to make the renderer self-contained.
pub(crate) fn stream_scroll_label(
    total: usize,
    visible_h: usize,
    scroll_offset: usize,
    is_following: bool,
) -> Cow<'static, str> {
    if total == 0 {
        return Cow::Borrowed("");
    }
    let max_offset = total.saturating_sub(visible_h);
    if max_offset == 0 {
        return Cow::Borrowed(" ▼ ");
    }
    if is_following {
        return Cow::Borrowed(" ▼ following ");
    }
    if scroll_offset == 0 {
        return Cow::Borrowed(" top ");
    }
    if scroll_offset >= max_offset {
        return Cow::Borrowed(" end ");
    }
    Cow::Owned(format!(" ▲ {} ↑ ", scroll_offset))
}

/// Outcome of a Stream render, useful for callers that want to compose
/// additional bottom-border information.
#[derive(Debug, Clone, Copy)]
pub struct StreamRenderInfo {
    pub total_rows: usize,
    pub visible_height: usize,
}

/// Render a Stream pane into `area`. The caller owns `state`; this function
/// mutates `state.scroll_offset` / `state.is_following` to apply the same
/// clamp + re-engage-following invariants used by `DetailPane`.
///
/// `title_left`: left side of the top border (e.g. `"codex"`). Wrapped
///     with surrounding spaces internally.
/// `title_right`: right side of the top border, e.g. badge or attempt label.
/// `bottom_right_hint`: optional right-side bottom-border hint (e.g.
///     `"[esc] close"` for the peek; `Some("[I]ssue detail")` for
///     `DetailPane` when an issue badge is present).
pub fn render_stream(
    frame: &mut Frame,
    area: Rect,
    title_left: &str,
    title_right: Option<&str>,
    bottom_right_hint: Option<&str>,
    trace: Option<&mut ReactTrace>,
    state: &mut StreamViewState,
) -> StreamRenderInfo {
    // 1. Skeleton: shape-equivalent to the final block so inner() is stable.
    let mut skeleton = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .title(" ")
        .title_bottom(" ");
    if title_right.is_some() {
        skeleton = skeleton.title_top(Line::from(" ").alignment(Alignment::Right));
    }
    if bottom_right_hint.is_some() {
        skeleton = skeleton.title_bottom(Line::from(" ").alignment(Alignment::Right));
    }
    let inner = skeleton.inner(area);
    let chunks = Layout::vertical([Constraint::Min(1)]).split(inner);
    let body_area = chunks[0];
    let visible_h = body_area.height as usize;

    // 2. Borrow the already-wrapped cache; wrap the placeholder only when needed.
    let placeholder_rows;
    let wrapped = match trace {
        Some(t) => t.build_body_lines(body_area.width),
        None => {
            let placeholder = Line::from(Span::styled(
                "(no stream yet)",
                Style::default().fg(Color::DarkGray),
            ));
            placeholder_rows =
                crate::components::line_wrap::wrap_line_to_width(&placeholder, body_area.width);
            &placeholder_rows
        }
    };
    let total = wrapped.len();

    // 3. Clamp + re-engage-following.
    let max_offset = total.saturating_sub(visible_h);
    if state.is_following {
        state.scroll_offset = max_offset;
    } else {
        state.scroll_offset = state.scroll_offset.min(max_offset);
        if state.scroll_offset >= max_offset && max_offset > 0 {
            state.is_following = true;
        }
    }

    // 4. Real block with titles.
    let label = stream_scroll_label(total, visible_h, state.scroll_offset, state.is_following);
    let mut block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .title(format!(" {} ", title_left))
        .title_bottom(label.as_ref().to_string());
    if let Some(tr) = title_right {
        block = block.title_top(Line::from(format!(" {} ", tr)).alignment(Alignment::Right));
    }
    if let Some(br) = bottom_right_hint {
        block = block.title_bottom(Line::from(format!(" {} ", br)).alignment(Alignment::Right));
    }
    frame.render_widget(block, area);

    // 5. Copy only visible rows. Slice with usize so long streams never truncate
    // the scroll offset to Paragraph's u16 range.
    let start = state.scroll_offset;
    let count = visible_h.min(total - start);
    let p = Paragraph::new(wrapped[start..start + count].to_vec());
    frame.render_widget(p, body_area);

    StreamRenderInfo {
        total_rows: total,
        visible_height: visible_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_up_disengages_follow() {
        let mut s = StreamViewState::new();
        assert!(s.is_following);
        s.scroll_up_by(1);
        assert!(!s.is_following);
        assert_eq!(s.scroll_offset, 0);
    }

    #[test]
    fn scroll_to_bottom_reengages_follow() {
        let mut s = StreamViewState {
            scroll_offset: 42,
            is_following: false,
        };
        s.scroll_to_bottom();
        assert!(s.is_following);
    }

    #[test]
    fn label_following_shows_following_marker() {
        let l = stream_scroll_label(100, 20, 80, true);
        assert_eq!(l, Cow::Borrowed(" ▼ following "));
    }

    #[test]
    fn label_empty_total_is_blank() {
        let l = stream_scroll_label(0, 20, 0, false);
        assert_eq!(l, Cow::Borrowed(""));
    }
}
