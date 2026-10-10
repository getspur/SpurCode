use super::*;
use crate::components::stream_pane::{render_stream, StreamViewState};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Widget},
    Terminal,
};

fn seed_rows(trace: &mut ReactTrace, width: u16, rows: Vec<Line<'static>>) {
    trace.body_cache = Some(render::BodyCacheEntry {
        lines: rows,
        width,
        generation: trace.generation,
    });
}

fn draw(
    trace: Option<&mut ReactTrace>,
    width: u16,
    height: u16,
    state: &mut StreamViewState,
) -> (Buffer, usize) {
    let mut terminal = Terminal::new(TestBackend::new(width, height + 2)).unwrap();
    let mut total = 0;
    terminal
        .draw(|frame| {
            total = render_stream(frame, frame.area(), "test", None, None, trace, state).total_rows;
        })
        .unwrap();
    (terminal.backend().buffer().clone(), total)
}

#[test]
fn warm_body_cache_reuses_its_line_storage() {
    let mut trace = ReactTrace::new();
    trace.append_message("cached text", "codex", "10:00:00".into());
    let first = trace.build_body_lines(80).as_ptr();
    assert_eq!(
        first,
        trace.body_cache.as_ref().unwrap().lines.as_ptr(),
        "cold build must return the stored cache, not a full copy"
    );
    let second = trace.build_body_lines(80).as_ptr();
    assert_eq!(first, second, "warm frame must reuse cached row storage");
}

#[test]
fn stream_scroll_preserves_solver_witness_above_u16() {
    // Solve PRE sol_342b9223981340c2: total=65537, height=1, offset=65536.
    let mut trace = ReactTrace::new();
    seed_rows(
        &mut trace,
        24,
        (0..65537)
            .map(|i| Line::from(format!("row {i:06}")))
            .collect(),
    );
    let mut state = StreamViewState {
        scroll_offset: 65536,
        is_following: false,
    };
    let (buffer, total) = draw(Some(&mut trace), 24, 1, &mut state);
    let text: String = (0..10).map(|x| buffer[(x, 1)].symbol()).collect();
    assert_eq!(text, "row 065536");
    assert_eq!(total, 65537);
    assert_eq!(state.scroll_offset, 65536);
    assert!(state.is_following);
}

#[test]
fn cached_stream_preserves_visible_cells_and_follow_state() {
    for width in [1, 7, 24, 80] {
        for height in [0, 1, 4, 24] {
            for n in [0, 1, 50] {
                let rows: Vec<_> = (0..n)
                    .flat_map(|i| {
                        crate::components::line_wrap::wrap_line_to_width(
                            &Line::from(format!("row {i}: unicode 界 e\u{301} words")),
                            width,
                        )
                    })
                    .collect();
                for (requested, following) in
                    [(0, false), (3, false), (usize::MAX, false), (0, true)]
                {
                    let total = rows.len();
                    let max = total.saturating_sub(height as usize);
                    let start = if following { max } else { requested.min(max) };
                    let mut trace = ReactTrace::new();
                    seed_rows(&mut trace, width, rows.clone());
                    let mut state = StreamViewState {
                        scroll_offset: requested,
                        is_following: following,
                    };
                    let (actual, actual_total) = draw(Some(&mut trace), width, height, &mut state);
                    let area = Rect::new(0, 1, width, height);
                    let mut expected = Buffer::empty(area);
                    Paragraph::new(rows.clone())
                        .scroll((start as u16, 0))
                        .render(area, &mut expected);
                    for y in 1..1 + height {
                        for x in 0..width {
                            assert_eq!(actual[(x, y)], expected[(x, y)]);
                        }
                    }
                    assert_eq!(actual_total, total);
                    assert_eq!(state.scroll_offset, start);
                    assert_eq!(state.is_following, following || (start >= max && max > 0));
                }
            }
        }
    }
}

#[test]
fn missing_stream_placeholder_is_wrapped_and_scrolled() {
    for width in [1, 7, 24, 80] {
        for height in [0, 1, 4, 24] {
            let line = Line::from(ratatui::text::Span::styled(
                "(no stream yet)",
                ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray),
            ));
            let rows = crate::components::line_wrap::wrap_line_to_width(&line, width);
            let mut state = StreamViewState::default();
            let (actual, total) = draw(None, width, height, &mut state);
            assert_eq!(total, rows.len());
            let area = Rect::new(0, 1, width, height);
            let mut expected = Buffer::empty(area);
            Paragraph::new(rows)
                .scroll((state.scroll_offset as u16, 0))
                .render(area, &mut expected);
            for y in 1..1 + height {
                for x in 0..width {
                    assert_eq!(actual[(x, y)], expected[(x, y)]);
                }
            }
        }
    }
}

#[test]
fn body_cache_rebuilds_after_append_resize_tick_and_clear() {
    let mut trace = ReactTrace::new();
    trace.append_message(
        "first text with enough words to wrap at narrow widths",
        "codex",
        "10:00:00".into(),
    );
    let before = trace.build_body_lines(80).to_vec();
    trace.append_message(" appended suffix", "codex", "10:00:00".into());
    let appended = trace.build_body_lines(80).to_vec();
    assert_ne!(before, appended);
    let narrow = trace.build_body_lines(24).to_vec();
    assert_ne!(appended, narrow);
    assert_eq!(trace.body_cache.as_ref().unwrap().width, 24);
    trace.body_cache = None;
    assert_eq!(narrow, trace.build_body_lines(24));
    trace.push(TraceEntry {
        kind: TraceKind::Act {
            tool: "read_file".into(),
            family: spur_acp::adapter::ToolFamily::Unknown,
            input: ToolInputDisplay::Empty,
            tool_call_id: None,
            status: ActStatus::Pending,
        },
        text: "read_file".into(),
        timestamp: "10:00:01".into(),
        #[cfg(feature = "markdown")]
        markdown: None,
    });
    trace.build_body_lines(24);
    let cached_generation = trace.body_cache.as_ref().unwrap().generation;
    trace.tick();
    assert_ne!(cached_generation, trace.generation);
    let ticked = trace.build_body_lines(24).to_vec();
    assert_eq!(
        trace.body_cache.as_ref().unwrap().generation,
        trace.generation
    );
    trace.body_cache = None;
    assert_eq!(ticked, trace.build_body_lines(24));
    trace.clear();
    assert!(trace.body_cache.is_none());
    let cleared = trace.build_body_lines(24).to_vec();
    let mut fresh = ReactTrace::new();
    assert_eq!(cleared, fresh.build_body_lines(24));
}
