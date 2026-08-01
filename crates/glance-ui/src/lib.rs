use std::io::stdout;

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, MouseEventKind,
};
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

use app_state::{AddressMode, App, ScrollAccumulator, ViewState};
use draw::{
    compute_layout, draw_ui, image_dock_preview_rect, one_line_label, point_in_rect, viewer_rect,
};
use input::{
    flush_scroll_accumulator, handle_key, handle_mouse, handle_paste, queue_scroll_delta,
    request_media_launch,
};
pub use model::{
    UiAuthItem, UiCommand, UiIconMode, UiImageMode, UiMediaActivity, UiMediaItem, UiMediaKind,
    UiMediaPhase, UiPage, UiResourceItem, UiWebMode,
};
use navigation::{
    clamp_content_scroll_to_terminal, clamp_cursor_to_terminal, sync_scroll_to_cursor,
};

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

pub fn run<F>(initial_page: UiPage, mut on_command: F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<UiPage>,
{
    let mut terminal = ratatui::init();
    if let Err(err) = execute!(stdout(), EnableMouseCapture, EnableBracketedPaste) {
        ratatui::restore();
        return Err(err.into());
    }

    let app_res = run_loop(&mut terminal, &mut App::new(initial_page), &mut on_command);

    let _ = execute!(stdout(), DisableBracketedPaste, DisableMouseCapture);
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

        if !event::poll(POLL_INTERVAL)? {
            apply_command(app, UiCommand::Poll, on_command)?;
            app.tick_feedback();
            continue;
        }

        let mut scroll_acc = ScrollAccumulator::default();
        let mut feed_advance_intent = false;
        loop {
            match event::read()? {
                Event::Key(key) => {
                    flush_scroll_accumulator(app, &mut scroll_acc)?;
                    feed_advance_intent |= app.address_mode == AddressMode::Nav
                        && !app.help_open
                        && !app.media_picker_open
                        && !app.image_viewer_open
                        && matches!(
                            key.code,
                            KeyCode::Down
                                | KeyCode::PageDown
                                | KeyCode::End
                                | KeyCode::Char('j' | 'n' | 'G' | ' ')
                        );
                    if handle_key(app, key, on_command)? {
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => {
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                    ) && !app.image_viewer_open
                    {
                        feed_advance_intent |= queue_scroll_delta(app, mouse, &mut scroll_acc)?;
                    } else {
                        flush_scroll_accumulator(app, &mut scroll_acc)?;
                        handle_mouse(app, mouse, on_command)?;
                    }
                }
                Event::Paste(text) => {
                    flush_scroll_accumulator(app, &mut scroll_acc)?;
                    handle_paste(app, &text);
                }
                _ => {}
            }

            if !event::poll(std::time::Duration::from_millis(0))? {
                break;
            }
        }
        flush_scroll_accumulator(app, &mut scroll_acc)?;
        if feed_advance_intent {
            request_more_if_near_edge(app, on_command)?;
        }

        app.tick_feedback();
    }
}

fn request_more_if_near_edge<F>(app: &mut App, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<UiPage>,
{
    if !app.page.can_load_more || app.page.loading_more {
        return Ok(());
    }
    let content = crate::navigation::current_content_rect()?;
    if is_near_feed_edge(app, content) {
        apply_command(app, UiCommand::LoadMore, on_command)?;
    }
    Ok(())
}

fn is_near_feed_edge(app: &App, content: Rect) -> bool {
    let max_scroll = crate::navigation::max_content_scroll(app, content);
    let threshold = content.height.saturating_sub(2).saturating_div(2).max(3);
    max_scroll.saturating_sub(app.content_scroll) <= threshold
}

fn draw_image_viewer(app: &mut App) -> Result<()> {
    if app.needs_image_clear {
        if let Some(old) = app.last_image_placement.take() {
            let _ = clear_inline_image(old);
        }
        app.needs_image_clear = false;
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
    let screen = Rect::new(0, 0, w, h);
    let (inner_left, inner_top, max_width_cells, max_height_cells, image_id) =
        if app.image_viewer_open {
            let viewer = viewer_rect(screen);
            if viewer.width <= 8 || viewer.height <= 8 {
                return Ok(());
            }
            (
                viewer.x.saturating_add(2),
                viewer.y.saturating_add(3),
                viewer.width.saturating_sub(4),
                viewer.height.saturating_sub(5),
                42,
            )
        } else {
            let Some(sidebar) = compute_layout(screen).sidebar else {
                if let Some(old) = app.last_image_placement.take() {
                    let _ = clear_inline_image(old);
                }
                return Ok(());
            };
            let Some(preview) = image_dock_preview_rect(app, sidebar) else {
                if let Some(old) = app.last_image_placement.take() {
                    let _ = clear_inline_image(old);
                }
                return Ok(());
            };
            (
                preview.x.saturating_add(1),
                preview.y.saturating_add(1),
                preview.width.saturating_sub(2),
                preview.height.saturating_sub(2),
                41,
            )
        };

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
        image_id,
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
            request_media_launch(app, idx, on_command)?;
        }
        InteractiveAction::OpenForm(idx) => {
            app.enter_form_edit(idx);
            app.set_click_feedback(format!("form {}: enter a value", idx + 1));
        }
        InteractiveAction::ActivateButton(idx) => {
            app.set_click_feedback("requesting more content");
            apply_command(app, UiCommand::ActivateButton(idx), on_command)?;
        }
    }
    Ok(())
}

fn apply_command<F>(app: &mut App, command: UiCommand, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<UiPage>,
{
    let is_poll = matches!(command, UiCommand::Poll);
    let is_reload = matches!(command, UiCommand::Reload | UiCommand::ReloadDynamic);
    let prev_cursor = app.cursor_visual_row;
    let prev_scroll = app.content_scroll;
    let prev_url = app.page.rendered.source_url.clone();
    let prev_entry_id = app.page.history_entry_id;
    let prev_web_mode = app.page.web_mode;
    let prev_loading_more = app.page.loading_more;
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
    let update_failed = status_is_failure(&app.page.status);
    if prev_loading_more && !app.page.loading_more {
        app.set_update_notice(
            if update_failed {
                "Live content update failed"
            } else {
                "Live content updated"
            },
            update_failed,
        );
    } else if prev_web_mode == UiWebMode::Upgrading && app.page.web_mode != UiWebMode::Upgrading {
        app.set_update_notice(
            if update_failed {
                "Dynamic DOM update failed"
            } else {
                "Dynamic DOM updated"
            },
            update_failed,
        );
    }
    let current_entry_id = app.page.history_entry_id;
    let url_changed = app.page.rendered.source_url != prev_url;
    let should_reset_view = url_changed || is_reload;

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
        app.clear_find();
        app.clear_hover();
        app.close_media_picker();
        app.selected_media_index = 0;
        if app.image_viewer_open {
            app.close_image_viewer();
        }
        if let Some(saved) = app.view_state_by_entry.get(&current_entry_id).copied() {
            app.content_scroll = saved.content_scroll;
            app.cursor_visual_row = saved.cursor_visual_row;
        } else {
            app.content_scroll = 0;
            app.cursor_visual_row = 0;
        }
    }

    app.selected_media_index = app
        .selected_media_index
        .min(app.page.media.len().saturating_sub(1));

    clamp_content_scroll_to_terminal(app)?;
    clamp_cursor_to_terminal(app)?;
    if !is_poll || url_changed {
        sync_scroll_to_cursor(app)?;
    }
    Ok(())
}

fn status_is_failure(status: &str) -> bool {
    let normalized = status.to_ascii_lowercase();
    ["error", "failed", "could not", "unavailable"]
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn should_snapshot_view_state(command: &UiCommand) -> bool {
    matches!(
        command,
        UiCommand::OpenUrl(_)
            | UiCommand::OpenLink(_)
            | UiCommand::Back
            | UiCommand::Forward
            | UiCommand::SubmitForm(_, _)
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
        InteractiveAction::OpenForm(i) => format!("F{}", i + 1),
        InteractiveAction::ActivateButton(i) => format!("B{}", i + 1),
    }
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::{
        App, UiAuthItem, UiIconMode, UiImageMode, UiMediaActivity, UiMediaItem, UiMediaKind,
        UiMediaPhase, UiPage, UiResourceItem, UiWebMode, is_near_feed_edge,
    };
    use crate::app_state::{AddressMode, FocusPane};
    use crate::draw::{
        compute_layout, draw_ui, image_dock_preview_rect, media_picker_layout, media_window_start,
        reading_progress,
    };
    use crate::input::{footer_hitboxes, request_media_launch};
    use crate::navigation::{
        clamp_content_scroll, max_content_scroll, wrap_line_map_word_boundary,
    };
    use glance_media::ImageBackend;
    use glance_render::RenderedPage;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
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

    #[test]
    fn live_feed_prefetch_starts_only_near_the_local_edge() {
        let mut app = test_app((0..12).map(|i| format!("line {i}")).collect());
        let content_rect = Rect::new(0, 0, 30, 6);
        app.page.can_load_more = true;
        assert!(!is_near_feed_edge(&app, content_rect));

        app.content_scroll = 6;
        assert!(is_near_feed_edge(&app, content_rect));
    }

    #[test]
    fn media_picker_window_keeps_selection_visible() {
        assert_eq!(media_window_start(0, 20, 5), 0);
        assert_eq!(media_window_start(10, 20, 5), 8);
        assert_eq!(media_window_start(19, 20, 5), 15);
        assert_eq!(media_window_start(3, 4, 5), 0);
    }

    #[test]
    fn footer_hitboxes_follow_rendered_token_order() {
        let app = test_app(Vec::new());
        let hitboxes = footer_hitboxes(&app, Rect::new(0, 20, 80, 4));
        assert_eq!(hitboxes.web, 1..10);
        assert_eq!(hitboxes.external.start, hitboxes.web.end + 1);
        assert!(hitboxes.auth.is_none());
        assert_eq!(hitboxes.media.start, hitboxes.external.end + 1);
        assert_eq!(hitboxes.img.start, hitboxes.media.end + 1);
        assert!(hitboxes.img.end > hitboxes.img.start);
        assert_eq!(hitboxes.help.start, hitboxes.img.end + 1);

        let mut signed_out = test_app(Vec::new());
        signed_out.page.auth = Some(UiAuthItem {
            link_index: 0,
            label: "Sign in".to_string(),
            url: "https://example.com/login".to_string(),
        });
        let hitboxes = footer_hitboxes(&signed_out, Rect::new(0, 20, 100, 4));
        let auth = hitboxes.auth.expect("sign-in control");
        assert_eq!(auth.start, hitboxes.external.end + 1);
        assert_eq!(hitboxes.media.start, auth.end + 1);
    }

    #[test]
    fn wide_layout_adds_page_dock_and_narrow_layout_keeps_reader_full_width() {
        let wide = compute_layout(Rect::new(0, 0, 140, 36));
        assert_eq!(wide.sidebar.map(|area| area.width), Some(36));
        assert_eq!(wide.content.width, 104);

        let narrow = compute_layout(Rect::new(0, 0, 100, 36));
        assert!(narrow.sidebar.is_none());
        assert_eq!(narrow.content.width, 100);
    }

    #[test]
    fn media_library_uses_split_view_when_space_allows() {
        let wide = media_picker_layout(Rect::new(10, 4, 100, 24));
        assert_eq!(wide.list.y, wide.details.y);
        assert!(wide.list.right() < wide.details.x);

        let narrow = media_picker_layout(Rect::new(0, 0, 50, 18));
        assert!(narrow.list.bottom() <= narrow.details.y);
    }

    #[test]
    fn reading_progress_accounts_for_visible_lines() {
        assert_eq!(reading_progress(0, 100, 20), 20);
        assert_eq!(reading_progress(80, 100, 20), 100);
        assert_eq!(reading_progress(0, 5, 10), 100);
    }

    #[test]
    fn wide_render_contains_modern_shell_and_page_dock() {
        let app = test_app(vec!["# Welcome".to_string(), "A reader page".to_string()]);
        let backend = TestBackend::new(140, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| draw_ui(frame, &app))
            .expect("draw modern shell");

        let text = terminal_text(&terminal);
        assert!(text.contains("GLANCE"));
        assert!(text.contains("example.com"));
        assert!(text.contains("# Welcome"));
        assert!(text.contains("●STATIC"));
        assert!(text.contains("↗"));
    }

    #[test]
    fn detected_sign_in_is_visible_in_compact_chrome() {
        let mut app = test_app(vec!["A reader page".to_string()]);
        app.page.auth = Some(UiAuthItem {
            link_index: 2,
            label: "로그인".to_string(),
            url: "https://example.com/sign".to_string(),
        });
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| draw_ui(frame, &app))
            .expect("draw sign-in action");

        let text = terminal_text(&terminal);
        assert!(text.contains("♙ Sign in"));
    }

    #[test]
    fn help_palette_renders_as_a_modal() {
        let mut app = test_app(vec!["A reader page".to_string()]);
        app.help_open = true;
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| draw_ui(frame, &app))
            .expect("draw command palette");

        let text = terminal_text(&terminal);
        assert!(text.contains("NAVIGATE"));
        assert!(text.contains("Press ? or Esc to close"));
    }

    #[test]
    fn repeated_media_launch_is_suppressed_while_player_is_opening() {
        let mut app = test_app(Vec::new());
        app.page.media.push(UiMediaItem {
            label: "Demo".to_string(),
            url: "https://example.com/demo.mp4".to_string(),
            kind: UiMediaKind::Video,
            mime_type: Some("video/mp4".to_string()),
        });
        app.page.media_activity = Some(UiMediaActivity {
            index: 0,
            label: "Demo".to_string(),
            url: "https://example.com/demo.mp4".to_string(),
            kind: UiMediaKind::Video,
            phase: UiMediaPhase::Opening,
        });
        let mut calls = 0;
        request_media_launch(&mut app, 0, &mut |_| {
            calls += 1;
            unreachable!("duplicate launch must not reach the browser")
        })
        .expect("suppress duplicate media launch");
        assert_eq!(calls, 0);
        assert!(app.effective_status().contains("already opening Demo"));
    }

    #[test]
    fn media_launch_activity_renders_immediately() {
        let mut app = test_app(vec!["Watching soon".to_string()]);
        app.page.media.push(UiMediaItem {
            label: "Demo".to_string(),
            url: "https://example.com/demo.mp4".to_string(),
            kind: UiMediaKind::Video,
            mime_type: Some("video/mp4".to_string()),
        });
        app.page.media_activity = Some(UiMediaActivity {
            index: 0,
            label: "Demo".to_string(),
            url: "https://example.com/demo.mp4".to_string(),
            kind: UiMediaKind::Video,
            phase: UiMediaPhase::Opening,
        });
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| draw_ui(frame, &app))
            .expect("draw launch activity");

        let text = terminal_text(&terminal);
        assert!(text.contains("◐"));
        assert!(text.contains("Opening  Demo"));
    }

    #[test]
    fn page_dock_image_preview_requires_a_capable_inline_backend() {
        let mut app = test_app(Vec::new());
        let sidebar = Rect::new(104, 2, 36, 30);
        app.page.selected_image_bytes = Some(vec![1, 2, 3]);
        app.page.image_backend = ImageBackend::Kitty;

        let preview = image_dock_preview_rect(&app, sidebar).expect("inline Page Dock preview");
        assert!(preview.x > sidebar.x);
        assert!(preview.bottom() < sidebar.bottom());

        app.page.image_backend = ImageBackend::TextFallback;
        assert!(image_dock_preview_rect(&app, sidebar).is_none());
    }

    fn terminal_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut text = String::new();
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
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
                icon_mode: UiIconMode::Auto,
                web_mode: UiWebMode::Static,
                image_backend: ImageBackend::TextFallback,
                image_mode: UiImageMode::Auto,
                links: Vec::<UiResourceItem>::new(),
                auth: None,
                images: Vec::<UiResourceItem>::new(),
                media: Vec::new(),
                media_activity: None,
                can_load_more: false,
                loading_more: false,
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
            command_cursor: 0,
            find_query: String::new(),
            find_matches: Vec::new(),
            find_current: None,
            click_feedback: None,
            hover_feedback: None,
            hovered_action: None,
            hovered_sidebar_action: None,
            help_open: false,
            image_viewer_open: false,
            media_picker_open: false,
            selected_media_index: 0,
            hovered_media_index: None,
            last_image_placement: None,
            needs_image_clear: false,
            image_dirty: false,
            last_left_down: None,
            mouse_dragged: false,
            scrollbar_dragging: false,
            animation_tick: 0,
            update_notice: None,
        }
    }
}
