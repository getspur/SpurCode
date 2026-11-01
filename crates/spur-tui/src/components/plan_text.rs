use super::input_bar_wrap::{wrap_with_cell_width, WrapLayout};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

/// Match the grapheme units consumed by ratatui's paragraph renderer.
pub(crate) fn display_cells(value: &str) -> usize {
    value
        .graphemes(true)
        .map(|grapheme| Span::raw(grapheme).width())
        .sum()
}

pub(crate) fn fit_to_cells(value: &str, width: usize) -> String {
    if display_cells(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let cells = Span::raw(grapheme).width();
        if used + cells > width - 1 {
            break;
        }
        result.push_str(grapheme);
        used += cells;
    }
    result.push('…');
    result
}

pub(crate) fn wrap_lines(lines: &[Line<'static>], width: u16) -> Option<WrapLayout> {
    if width == 0 {
        return None;
    }
    let source: Vec<String> = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect();
    Some(wrap_with_cell_width(&source, width, |grapheme| {
        Span::raw(grapheme).width() as u8
    }))
}

pub(crate) fn visible_lines(
    lines: &[Line<'static>],
    layout: Option<&WrapLayout>,
    offset: usize,
    height: usize,
) -> Vec<Line<'static>> {
    layout
        .into_iter()
        .flat_map(|layout| layout.rows.iter())
        .skip(offset)
        .take(height)
        .map(|row| {
            let original = &lines[row.logical_row];
            let mut byte = 0;
            let spans = original
                .spans
                .iter()
                .filter_map(|span| {
                    let span_start = byte;
                    byte += span.content.len();
                    let start = row.byte_start.max(span_start);
                    let end = row.byte_end.min(byte);
                    (start < end).then(|| {
                        Span::styled(
                            span.content[start - span_start..end - span_start].to_string(),
                            original.style.patch(span.style),
                        )
                    })
                })
                .collect::<Vec<_>>();
            Line::from(spans).style(original.style)
        })
        .collect()
}
