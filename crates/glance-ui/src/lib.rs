use std::io::stdout;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, MouseEventKind};
use crossterm::execute;
use glance_media::{
    ImageBackend, InlineImagePlacement, clear_inline_image, compute_aspect_fit,
    render_inline_image_at,
};
use glance_render::InteractiveAction;
use ratatui::DefaultTerminal;
use ratatui::layout::Rect;

mod app_state;
mod draw;
mod input;
mod model;
mod navigation;

use app_state::{App, ScrollAccumulator, ViewState};
use draw::{
    bool_label, compute_layout, draw_ui, image_mode_label, one_line_label, point_in_rect,
    viewer_rect,
};
use input::{flush_scroll_accumulator, handle_key, handle_mouse, queue_scroll_delta};
pub use model::{UiCommand, UiIconMode, UiImageItem, UiImageMode, UiPage};
use navigation::{
    clamp_content_scroll_to_terminal, clamp_cursor_to_terminal, sync_scroll_to_cursor,
};

pub fn run<F>(initial_page: UiPage, mut on_command: F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<UiPage>,
{
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;

    let app_res = run_loop(&mut terminal, &mut App::new(initial_page), &mut on_command);

    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    app_res
}

fn run_loop<F>(terminal: &mut DefaultTerminal, app: &mut App, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<UiPage>,
{
    loop {
        terminal.draw(|f| draw_ui(f, app))?;
        draw_image_viewer(app)?;

        if !event::poll(std::time::Duration::from_millis(16))? {
            apply_command(app, UiCommand::Poll, on_command)?;
            app.tick_feedback();
            continue;
        }

        let mut scroll_acc = ScrollAccumulator::default();
        loop {
            match event::read()? {
                Event::Key(key) => {
                    flush_scroll_accumulator(app, &mut scroll_acc)?;
                    if handle_key(app, key, on_command)? {
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => {
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                    ) {
                        queue_scroll_delta(mouse, &mut scroll_acc)?;
                    } else {
                        flush_scroll_accumulator(app, &mut scroll_acc)?;
                        handle_mouse(app, mouse, on_command)?;
                    }
                }
                _ => {}
            }

            if !event::poll(std::time::Duration::from_millis(0))? {
                break;
            }
        }
        flush_scroll_accumulator(app, &mut scroll_acc)?;

        app.tick_feedback();
    }
}

fn draw_image_viewer(app: &mut App) -> Result<()> {
    if app.needs_image_clear {
        if let Some(old) = app.last_image_placement.take() {
            let _ = clear_inline_image(old);
        }
        app.needs_image_clear = false;
    }

    if !app.image_viewer_open {
        return Ok(());
    }

    if app.page.image_mode == UiImageMode::Text
        || matches!(
            app.page.image_backend,
            ImageBackend::TextFallback | ImageBackend::Sixel
        )
    {
        if let Some(old) = app.last_image_placement.take() {
            let _ = clear_inline_image(old);
        }
        return Ok(());
    }
    let Some(image_bytes) = app.page.selected_image_bytes.as_ref() else {
        if let Some(old) = app.last_image_placement.take() {
            let _ = clear_inline_image(old);
        }
        return Ok(());
    };

    let (w, h) = crossterm::terminal::size()?;
    let viewer = viewer_rect(Rect::new(0, 0, w, h));
    if viewer.width <= 8 || viewer.height <= 8 {
        return Ok(());
    }

    let inner_left = viewer.x.saturating_add(2);
    let inner_top = viewer.y.saturating_add(3);
    let max_width_cells = viewer.width.saturating_sub(4);
    let max_height_cells = viewer.height.saturating_sub(5);

    let (width_cells, height_cells) = if let Some((orig_w, orig_h)) = app.page.selected_image_size {
        compute_aspect_fit(orig_w, orig_h, max_width_cells, max_height_cells, 8, 16)
    } else {
        (max_width_cells, max_height_cells)
    };

    let col_offset = max_width_cells.saturating_sub(width_cells) / 2;
    let row_offset = max_height_cells.saturating_sub(height_cells) / 2;
    let col = inner_left.saturating_add(col_offset);
    let row = inner_top.saturating_add(row_offset);

    let placement = InlineImagePlacement {
        backend: app.page.image_backend,
        image_id: 42,
        col,
        row,
        width_cells,
        height_cells,
    };

    if !app.image_dirty
        && app
            .last_image_placement
            .as_ref()
            .is_some_and(|prev| *prev == placement)
    {
        return Ok(());
    }

    if let Some(old) = app.last_image_placement.take() {
        let _ = clear_inline_image(old);
    }

    match render_inline_image_at(image_bytes, placement) {
        Ok(()) => {
            app.last_image_placement = Some(placement);
            app.image_dirty = false;
        }
        Err(err) => {
            app.set_click_feedback(format!("image render error: {}", one_line_label(&err)));
            app.image_dirty = false;
        }
    }
    Ok(())
}

fn execute_action<F>(app: &mut App, action: InteractiveAction, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<UiPage>,
{
    match action {
        InteractiveAction::OpenLink(idx) => {
            app.set_click_feedback(format!("open link {}", idx + 1));
            apply_command(app, UiCommand::OpenLink(idx), on_command)?;
        }
        InteractiveAction::OpenImage(idx) => {
            apply_command(app, UiCommand::SelectImage(idx), on_command)?;
            app.set_click_feedback(format!("open image {}", idx + 1));
            app.open_image_viewer();
        }
        InteractiveAction::OpenMedia(idx) => {
            app.set_click_feedback(format!("open media {}", idx + 1));
            apply_command(app, UiCommand::OpenMedia(idx), on_command)?;
        }
    }
    Ok(())
}

fn apply_command<F>(app: &mut App, command: UiCommand, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<UiPage>,
{
    let is_poll = matches!(command, UiCommand::Poll);
    let is_reload_or_js_toggle = matches!(command, UiCommand::Reload | UiCommand::ToggleJsDefault);
    let prev_cursor = app.cursor_visual_row;
    let prev_scroll = app.content_scroll;
    let prev_url = app.page.rendered.source_url.clone();
    let prev_entry_id = app.page.history_entry_id;
    if should_snapshot_view_state(&command) && prev_entry_id != 0 {
        app.view_state_by_entry.insert(
            prev_entry_id,
            ViewState {
                content_scroll: prev_scroll,
                cursor_visual_row: prev_cursor,
            },
        );
    }
    let new_page = on_command(command)?;
    app.page = new_page;
    let current_entry_id = app.page.history_entry_id;
    let url_changed = app.page.rendered.source_url != prev_url;
    let should_reset_view = url_changed || is_reload_or_js_toggle;

    if !is_poll {
        if should_reset_view {
            app.reset_view();
            app.cursor_visual_row = 0;
        } else {
            app.content_scroll = prev_scroll;
            app.cursor_visual_row = prev_cursor;
        }
        app.image_dirty = true;
    } else if !url_changed {
        app.cursor_visual_row = prev_cursor;
        app.content_scroll = prev_scroll;
    }

    if url_changed || current_entry_id != prev_entry_id {
        if let Some(saved) = app.view_state_by_entry.get(&current_entry_id).copied() {
            app.content_scroll = saved.content_scroll;
            app.cursor_visual_row = saved.cursor_visual_row;
        } else {
            app.content_scroll = 0;
            app.cursor_visual_row = 0;
        }
    }

    clamp_content_scroll_to_terminal(app)?;
    clamp_cursor_to_terminal(app)?;
    if !is_poll || url_changed {
        sync_scroll_to_cursor(app)?;
    }
    Ok(())
}

fn should_snapshot_view_state(command: &UiCommand) -> bool {
    matches!(
        command,
        UiCommand::OpenUrl(_) | UiCommand::OpenLink(_) | UiCommand::Back | UiCommand::Forward
    )
}

fn parse_index_command(cmd: &str, name: &str) -> Option<usize> {
    let mut parts = cmd.split_whitespace();
    if parts.next()? != name {
        return None;
    }
    let raw = parts.next()?;
    let idx = raw.parse::<usize>().ok()?;
    idx.checked_sub(1)
}

fn action_label(action: InteractiveAction) -> String {
    match action {
        InteractiveAction::OpenLink(i) => format!("L{}", i + 1),
        InteractiveAction::OpenImage(i) => format!("I{}", i + 1),
        InteractiveAction::OpenMedia(i) => format!("M{}", i + 1),
    }
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::{App, UiIconMode, UiImageItem, UiImageMode, UiPage};
    use crate::app_state::{AddressMode, FocusPane};
    use crate::navigation::{
        clamp_content_scroll, max_content_scroll, wrap_line_map_word_boundary,
    };
    use glance_media::ImageBackend;
    use glance_render::RenderedPage;
    use ratatui::layout::Rect;

    #[test]
    fn wrap_map_prefers_word_boundaries() {
        let wrapped = wrap_line_map_word_boundary("aaaa bbbb cccc", 6);
        assert_eq!(wrapped, vec![(0, 4), (5, 9), (10, 14)]);
    }

    #[test]
    fn wrap_map_breaks_long_word_when_needed() {
        let wrapped = wrap_line_map_word_boundary("abcdefghij", 4);
        assert_eq!(wrapped, vec![(0, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn max_content_scroll_is_zero_when_content_fits() {
        let app = test_app(vec!["one".to_string(), "two".to_string()]);
        let content_rect = Rect::new(0, 0, 30, 6); // inner height = 4
        assert_eq!(max_content_scroll(&app, content_rect), 0);
    }

    #[test]
    fn max_content_scroll_matches_overflow_lines() {
        let app = test_app(vec![
            "line 1".to_string(),
            "line 2".to_string(),
            "line 3".to_string(),
            "line 4".to_string(),
            "line 5".to_string(),
            "line 6".to_string(),
        ]);
        let content_rect = Rect::new(0, 0, 30, 6); // inner height = 4
        assert_eq!(max_content_scroll(&app, content_rect), 2);
    }

    #[test]
    fn clamp_content_scroll_caps_at_max() {
        let mut app = test_app(vec![
            "line 1".to_string(),
            "line 2".to_string(),
            "line 3".to_string(),
            "line 4".to_string(),
            "line 5".to_string(),
            "line 6".to_string(),
        ]);
        let content_rect = Rect::new(0, 0, 30, 6); // inner height = 4, max = 2
        app.content_scroll = 100;
        clamp_content_scroll(&mut app, content_rect);
        assert_eq!(app.content_scroll, 2);
    }

    fn test_app(lines: Vec<String>) -> App {
        App {
            page: UiPage {
                history_entry_id: 1,
                rendered: RenderedPage {
                    title: "t".to_string(),
                    source_url: "https://example.com".to_string(),
                    lines,
                    links: vec![],
                    media: vec![],
                    interactive_spans: vec![],
                },
                status: "ok".to_string(),
                js_note: "".to_string(),
                icon_mode: UiIconMode::Auto,
                image_backend: ImageBackend::TextFallback,
                image_mode: UiImageMode::Auto,
                js_default_enabled: true,
                images: Vec::<UiImageItem>::new(),
                media: Vec::<UiImageItem>::new(),
                selected_image_index: None,
                selected_image_bytes: None,
                selected_image_size: None,
            },
            content_scroll: 0,
            cursor_visual_row: 0,
            view_state_by_entry: std::collections::HashMap::new(),
            focus: FocusPane::Content,
            address_mode: AddressMode::Nav,
            command: String::new(),
            click_feedback: None,
            image_viewer_open: false,
            last_image_placement: None,
            needs_image_clear: false,
            image_dirty: false,
            last_left_down: None,
        }
    }
}
