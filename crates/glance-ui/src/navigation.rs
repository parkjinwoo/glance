use anyhow::Result;
use crossterm::terminal;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use glance_render::{InteractiveAction, InteractiveSpan};

use crate::UiCommand;
use crate::app_state::{App, FocusPane, VisualLine};

pub(crate) fn next_focus(focus: FocusPane) -> FocusPane {
    match focus {
        FocusPane::Address => FocusPane::Content,
        FocusPane::Content => FocusPane::Address,
    }
}

pub(crate) fn move_cursor_line(app: &mut App, down: bool) -> Result<()> {
    let max_row = max_visual_row_for_terminal(app)?;
    if down {
        app.cursor_visual_row = app.cursor_visual_row.saturating_add(1).min(max_row);
    } else {
        app.cursor_visual_row = app.cursor_visual_row.saturating_sub(1);
    }
    sync_scroll_to_cursor(app)
}

pub(crate) fn page_move_cursor(app: &mut App, down: bool) -> Result<()> {
    let content_rect = current_content_rect()?;
    let page = content_rect.height.saturating_sub(3) as usize;
    let delta = page.max(1);
    let max_row = max_visual_row(app, content_rect)?;
    if down {
        app.cursor_visual_row = app.cursor_visual_row.saturating_add(delta).min(max_row);
    } else {
        app.cursor_visual_row = app.cursor_visual_row.saturating_sub(delta);
    }
    sync_scroll_to_cursor(app)
}

pub(crate) fn jump_focused(app: &mut App, end: bool) -> Result<()> {
    let max_row = max_visual_row_for_terminal(app)?;
    app.cursor_visual_row = if end { max_row } else { 0 };
    sync_scroll_to_cursor(app)
}

pub(crate) fn clamp_cursor_to_terminal(app: &mut App) -> Result<()> {
    let max_row = max_visual_row_for_terminal(app)?;
    if app.cursor_visual_row > max_row {
        app.cursor_visual_row = max_row;
    }
    Ok(())
}

pub(crate) fn max_visual_row_for_terminal(app: &App) -> Result<usize> {
    let content_rect = current_content_rect()?;
    max_visual_row(app, content_rect)
}

pub(crate) fn max_visual_row(app: &App, content_rect: Rect) -> Result<usize> {
    let inner_width = content_rect.width.saturating_sub(2);
    if inner_width == 0 {
        return Ok(0);
    }
    let visual_lines = build_visual_line_map(&app.page.rendered.lines, inner_width);
    Ok(visual_lines.len().saturating_sub(1))
}

pub(crate) fn sync_scroll_to_cursor(app: &mut App) -> Result<()> {
    let content_rect = current_content_rect()?;
    let inner_height = content_rect.height.saturating_sub(2) as usize;
    if inner_height == 0 {
        return Ok(());
    }
    let top = app.content_scroll as usize;
    let row = app.cursor_visual_row;
    if row < top {
        app.content_scroll = row.min(u16::MAX as usize) as u16;
    } else if row >= top + inner_height {
        let new_top = row + 1 - inner_height;
        app.content_scroll = new_top.min(u16::MAX as usize) as u16;
    }
    clamp_content_scroll(app, content_rect);
    Ok(())
}

pub(crate) fn cursor_target(app: &App) -> Result<Option<(usize, InteractiveSpan)>> {
    let content_rect = current_content_rect()?;
    let inner_width = content_rect.width.saturating_sub(2);
    if inner_width == 0 {
        return Ok(None);
    }
    let visual_lines = build_visual_line_map(&app.page.rendered.lines, inner_width);
    Ok(cursor_target_from_visual(
        &app.page.rendered.interactive_spans,
        &visual_lines,
        app.cursor_visual_row,
    ))
}

pub(crate) fn cursor_target_from_visual(
    spans: &[InteractiveSpan],
    visual_lines: &[VisualLine],
    cursor_visual_row: usize,
) -> Option<(usize, InteractiveSpan)> {
    let visual = visual_lines.get(cursor_visual_row)?;
    let mut hits = spans
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, span)| {
            visual.logical_line_idx == span.line
                && visual.start_col < span.col_end
                && visual.end_col > span.col_start
        })
        .collect::<Vec<_>>();
    hits.sort_by_key(|(_, span)| span.col_start);
    hits.into_iter().next()
}

pub(crate) fn cursor_logical_line_from_visual(
    visual_lines: &[VisualLine],
    cursor_visual_row: usize,
) -> Option<usize> {
    visual_lines
        .get(cursor_visual_row)
        .map(|visual| visual.logical_line_idx)
}

pub(crate) fn execute_cursor_target<F>(app: &mut App, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let Some((_, span)) = cursor_target(app)? else {
        app.set_click_feedback("no action on this line");
        return Ok(());
    };
    super::execute_action(app, span.action, on_command)
}

pub(crate) fn set_cursor_from_screen_row(app: &mut App, content_rect: Rect, row: u16) {
    let top = content_rect.y.saturating_add(1);
    if row < top {
        return;
    }
    let relative = (row - top) as usize;
    app.cursor_visual_row = app.content_scroll as usize + relative;
}

pub(crate) fn content_action_at_click(
    app: &App,
    content_rect: Rect,
    column: u16,
    row: u16,
) -> Option<InteractiveAction> {
    let top = content_rect.y.saturating_add(1);
    let bottom = content_rect
        .y
        .saturating_add(content_rect.height.saturating_sub(1));
    let left = content_rect.x.saturating_add(1);
    let right = content_rect
        .x
        .saturating_add(content_rect.width.saturating_sub(1));

    if row < top || row >= bottom || column < left || column >= right {
        return None;
    }

    let inner_width = content_rect.width.saturating_sub(2);
    if inner_width == 0 {
        return None;
    }
    let visual_lines = build_visual_line_map(&app.page.rendered.lines, inner_width);
    let visual_row = app.content_scroll as usize + (row.saturating_sub(top)) as usize;
    let visual = visual_lines.get(visual_row)?;

    let click_col = column.saturating_sub(left);
    let segment_width = visual.end_col.saturating_sub(visual.start_col);
    if click_col >= segment_width {
        return None;
    }
    let line_idx = visual.logical_line_idx;
    let col_idx = visual.start_col.saturating_add(click_col);

    app.page
        .rendered
        .interactive_spans
        .iter()
        .find(|span| line_idx == span.line && col_idx >= span.col_start && col_idx < span.col_end)
        .map(|span| span.action)
}

pub(crate) fn current_content_rect() -> Result<Rect> {
    let (w, h) = terminal::size()?;
    let screen = Rect::new(0, 0, w, h);
    Ok(super::compute_layout(screen).content)
}

pub(crate) fn max_content_scroll(app: &App, content_rect: Rect) -> u16 {
    let inner_width = content_rect.width.saturating_sub(2);
    let inner_height = content_rect.height.saturating_sub(2) as usize;
    if inner_width == 0 || inner_height == 0 {
        return 0;
    }
    let visual_lines = build_visual_line_map(&app.page.rendered.lines, inner_width);
    visual_lines
        .len()
        .saturating_sub(inner_height)
        .min(u16::MAX as usize) as u16
}

pub(crate) fn clamp_content_scroll(app: &mut App, content_rect: Rect) {
    let max_scroll = max_content_scroll(app, content_rect);
    if app.content_scroll > max_scroll {
        app.content_scroll = max_scroll;
    }
}

pub(crate) fn clamp_content_scroll_to_terminal(app: &mut App) -> Result<()> {
    let content_rect = current_content_rect()?;
    clamp_content_scroll(app, content_rect);
    Ok(())
}

pub(crate) fn build_visual_line_map(lines: &[String], max_width: u16) -> Vec<VisualLine> {
    if max_width == 0 {
        return Vec::new();
    }
    let max_width = max_width as usize;
    let mut out = Vec::new();

    for (logical_line_idx, line) in lines.iter().enumerate() {
        let wrapped = wrap_line_map_word_boundary(line, max_width);
        if wrapped.is_empty() {
            out.push(VisualLine {
                logical_line_idx,
                start_col: 0,
                end_col: 0,
            });
            continue;
        }

        for (start_col, end_col) in wrapped {
            out.push(VisualLine {
                logical_line_idx,
                start_col: start_col as u16,
                end_col: end_col as u16,
            });
        }
    }

    out
}

#[derive(Debug, Clone, Copy)]
struct ColGrapheme {
    start_col: usize,
    end_col: usize,
    width: usize,
    is_whitespace: bool,
}

pub(crate) fn wrap_line_map_word_boundary(line: &str, max_width: usize) -> Vec<(usize, usize)> {
    use std::collections::VecDeque;
    use std::mem;

    if max_width == 0 {
        return Vec::new();
    }

    let mut wrapped_lines: Vec<Vec<ColGrapheme>> = Vec::new();
    let mut pending_line: Vec<ColGrapheme> = Vec::new();
    let mut pending_word: Vec<ColGrapheme> = Vec::new();
    let mut pending_whitespace: VecDeque<ColGrapheme> = VecDeque::new();

    let mut line_width = 0usize;
    let mut word_width = 0usize;
    let mut whitespace_width = 0usize;
    let mut non_whitespace_previous = false;
    let mut logical_col = 0usize;

    for ch in line.chars() {
        let symbol_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        let is_whitespace = ch.is_whitespace();
        let grapheme = ColGrapheme {
            start_col: logical_col,
            end_col: logical_col.saturating_add(symbol_width),
            width: symbol_width,
            is_whitespace,
        };
        logical_col = logical_col.saturating_add(symbol_width);

        if symbol_width > max_width {
            continue;
        }

        let word_found = non_whitespace_previous && is_whitespace;
        let untrimmed_overflow = pending_line.is_empty()
            && word_width
                .saturating_add(whitespace_width)
                .saturating_add(symbol_width)
                > max_width;

        if word_found || untrimmed_overflow {
            pending_line.extend(pending_whitespace.drain(..));
            line_width = line_width.saturating_add(whitespace_width);
            pending_line.append(&mut pending_word);
            line_width = line_width.saturating_add(word_width);
            whitespace_width = 0;
            word_width = 0;
        }

        let line_full = line_width >= max_width;
        let pending_word_overflow = symbol_width > 0
            && line_width
                .saturating_add(whitespace_width)
                .saturating_add(word_width)
                >= max_width;

        if line_full || pending_word_overflow {
            let mut remaining_width = max_width.saturating_sub(line_width);
            wrapped_lines.push(mem::take(&mut pending_line));
            line_width = 0;

            while let Some(front) = pending_whitespace.front() {
                if front.width > remaining_width {
                    break;
                }
                whitespace_width = whitespace_width.saturating_sub(front.width);
                remaining_width = remaining_width.saturating_sub(front.width);
                pending_whitespace.pop_front();
            }

            if grapheme.is_whitespace && pending_whitespace.is_empty() {
                continue;
            }
        }

        if grapheme.is_whitespace {
            whitespace_width = whitespace_width.saturating_add(grapheme.width);
            pending_whitespace.push_back(grapheme);
        } else {
            word_width = word_width.saturating_add(grapheme.width);
            pending_word.push(grapheme);
        }
        non_whitespace_previous = !grapheme.is_whitespace;
    }

    if pending_line.is_empty() && pending_word.is_empty() && !pending_whitespace.is_empty() {
        wrapped_lines.push(Vec::new());
    }
    pending_line.extend(pending_whitespace.drain(..));
    pending_line.append(&mut pending_word);

    if !pending_line.is_empty() {
        wrapped_lines.push(pending_line);
    } else if wrapped_lines.is_empty() {
        wrapped_lines.push(Vec::new());
    }

    wrapped_lines
        .into_iter()
        .map(|segment| {
            if segment.is_empty() {
                (0usize, 0usize)
            } else {
                let start_col = segment.first().map(|g| g.start_col).unwrap_or(0);
                let end_col = segment.last().map(|g| g.end_col).unwrap_or(start_col);
                (start_col, end_col)
            }
        })
        .collect()
}
