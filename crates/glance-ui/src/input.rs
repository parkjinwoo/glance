use std::io::stdout;

use anyhow::Result;
use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app_state::{
    AddressHitboxes, AddressMode, App, FocusPane, FooterHitboxes, ScrollAccumulator, SidebarAction,
    UiLayout,
};
use crate::draw::{
    control_confirmation_buttons, image_viewer_controls_rect, live_choice_options_rect,
    live_choice_window_start, live_text_editor_buttons, media_picker_layout, media_picker_rect,
    media_window_start, sidebar_action_at,
};
use crate::navigation::{
    clamp_content_scroll, clamp_cursor_to_terminal, content_action_at_click, cursor_target,
    execute_cursor_target, jump_focused, max_content_scroll, move_cursor_line, next_focus,
    page_move_cursor, set_cursor_from_screen_row,
};
use crate::{UiCommand, UiImageMode};

const MAX_LIVE_FIELD_BYTES: usize = 64 * 1024;

pub(crate) fn handle_key<F>(app: &mut App, key: KeyEvent, on_command: &mut F) -> Result<bool>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    if matches!(key.kind, KeyEventKind::Release) {
        return Ok(false);
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(true);
    }

    if app.control_confirmation.is_some() {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                confirm_live_control(app, on_command)?;
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N' | 'q') => {
                app.cancel_control_confirmation();
                app.set_click_feedback("live action cancelled");
            }
            _ => {}
        }
        return Ok(false);
    }

    if app.live_choice_editor.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.close_live_choice_editor();
                app.set_click_feedback("selection cancelled");
            }
            KeyCode::Down | KeyCode::Char('j') => app.move_live_choice(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_live_choice(-1),
            KeyCode::Home => app.move_live_choice_to_boundary(false),
            KeyCode::End => app.move_live_choice_to_boundary(true),
            KeyCode::Enter | KeyCode::Char(' ') => apply_live_choice(app, on_command)?,
            _ => {}
        }
        return Ok(false);
    }

    if app.help_open {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
        ) {
            app.close_help();
            app.set_click_feedback("command palette closed");
        }
        return Ok(false);
    }

    if app.media_picker_open {
        return handle_media_picker_key(app, key, on_command);
    }

    if app.image_viewer_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.close_image_viewer();
                app.set_click_feedback("image viewer closed");
                return Ok(false);
            }
            KeyCode::Left | KeyCode::Char('k') => {
                select_adjacent_image(app, -1, on_command)?;
                return Ok(false);
            }
            KeyCode::Right | KeyCode::Char('j') => {
                select_adjacent_image(app, 1, on_command)?;
                return Ok(false);
            }
            KeyCode::Char('y') => {
                copy_selected_image_url(app)?;
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    if app.address_mode != AddressMode::Nav {
        return handle_address_edit_mode(app, key, on_command);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Tab | KeyCode::BackTab => app.set_focus(next_focus(app.focus)),
        KeyCode::Char('o') => {
            app.enter_address_edit(app.page.rendered.source_url.clone());
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.enter_address_edit(app.page.rendered.source_url.clone());
        }
        KeyCode::Char(':') => app.enter_command_edit(),
        KeyCode::Char('/') => app.enter_find_edit(),
        KeyCode::Char('?') => app.open_help(),
        KeyCode::Esc if !app.find_query.is_empty() => {
            app.clear_find();
            app.set_click_feedback("find cleared");
        }
        KeyCode::Char('j') | KeyCode::Down => move_cursor_line(app, true)?,
        KeyCode::Char('k') | KeyCode::Up => move_cursor_line(app, false)?,
        KeyCode::Char('n') if !app.find_query.is_empty() => jump_find_match(app, true)?,
        KeyCode::Char('N') if !app.find_query.is_empty() => jump_find_match(app, false)?,
        KeyCode::Char('n') | KeyCode::Char(' ') | KeyCode::PageDown => page_move_cursor(app, true)?,
        KeyCode::Char('p') | KeyCode::PageUp => page_move_cursor(app, false)?,
        KeyCode::Enter => execute_cursor_target(app, on_command)?,
        KeyCode::Char('i') => {
            super::apply_command(app, UiCommand::ToggleImageMode, on_command)?;
            if app.page.image_mode == UiImageMode::Text {
                app.close_image_viewer();
            }
        }
        KeyCode::Char('m') => {
            if app.open_media_picker() {
                app.set_click_feedback(format!("{} media items", app.page.media.len()));
            }
        }
        KeyCode::Char('a') => open_auth_in_gui(app, on_command)?,
        KeyCode::Char('e') => open_current_in_gui(app, on_command)?,
        KeyCode::Char('y') => copy_cursor_target_url(app)?,
        KeyCode::Char('g') => jump_focused(app, false)?,
        KeyCode::Char('G') | KeyCode::End => jump_focused(app, true)?,
        KeyCode::Char('r') => super::apply_command(app, UiCommand::Reload, on_command)?,
        KeyCode::Char('d') => super::apply_command(app, UiCommand::ReloadDynamic, on_command)?,
        KeyCode::Char('b') => super::apply_command(app, UiCommand::Back, on_command)?,
        KeyCode::Char('f') => super::apply_command(app, UiCommand::Forward, on_command)?,
        _ => {}
    }

    Ok(false)
}

fn handle_address_edit_mode<F>(app: &mut App, key: KeyEvent, on_command: &mut F) -> Result<bool>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    if let AddressMode::LiveField(index) = app.address_mode
        && app
            .page
            .live_controls
            .get(index)
            .is_some_and(|control| control.multiline)
        && key.code == KeyCode::Enter
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::META)
    {
        insert_text(app, "\n");
        return Ok(false);
    }
    match key.code {
        KeyCode::Esc => {
            app.leave_address_edit();
            app.set_click_feedback("edit cancelled");
        }
        KeyCode::Enter => {
            let mode = app.address_mode;
            let cmd = app.command.clone();
            app.leave_address_edit();

            if mode == AddressMode::Find {
                update_find(app, cmd.trim())?;
                return Ok(false);
            }
            if let AddressMode::Form(index) = mode {
                super::apply_command(app, UiCommand::SubmitForm(index, cmd), on_command)?;
                return Ok(false);
            }
            if let AddressMode::LiveField(index) = mode {
                super::apply_command(app, UiCommand::FillLiveField(index, cmd), on_command)?;
                return Ok(false);
            }

            let cmd = cmd.trim().to_string();
            if cmd.is_empty() {
                return Ok(false);
            }
            if cmd == "quit" || cmd == "q" {
                return Ok(true);
            }
            if cmd == "reload" || cmd == "r" {
                super::apply_command(app, UiCommand::Reload, on_command)?;
                return Ok(false);
            }
            if cmd == "dynamic" || cmd == "dyn" {
                super::apply_command(app, UiCommand::ReloadDynamic, on_command)?;
                return Ok(false);
            }
            if cmd == "back" || cmd == "b" {
                super::apply_command(app, UiCommand::Back, on_command)?;
                return Ok(false);
            }
            if cmd == "forward" || cmd == "f" {
                super::apply_command(app, UiCommand::Forward, on_command)?;
                return Ok(false);
            }
            if cmd == "img" {
                super::apply_command(app, UiCommand::ToggleImageMode, on_command)?;
                if app.page.image_mode == UiImageMode::Text {
                    app.close_image_viewer();
                }
                return Ok(false);
            }
            if matches!(cmd.as_str(), "login" | "signin" | "auth") {
                open_auth_in_gui(app, on_command)?;
                return Ok(false);
            }
            if matches!(cmd.as_str(), "more" | "feed") {
                super::apply_command(app, UiCommand::LoadMore, on_command)?;
                return Ok(false);
            }
            if matches!(cmd.as_str(), "browser" | "gui" | "external") {
                open_current_in_gui(app, on_command)?;
                return Ok(false);
            }
            if let Some(idx) = super::parse_index_command(&cmd, "link") {
                super::apply_command(app, UiCommand::OpenLink(idx), on_command)?;
                return Ok(false);
            }
            if let Some(idx) = super::parse_index_command(&cmd, "media") {
                request_media_launch(app, idx, on_command)?;
                return Ok(false);
            }
            if let Some(idx) = super::parse_index_command(&cmd, "image") {
                super::apply_command(app, UiCommand::SelectImage(idx), on_command)?;
                app.open_image_viewer();
                return Ok(false);
            }
            if cmd == "help" {
                app.open_help();
                return Ok(false);
            }

            super::apply_command(app, UiCommand::OpenUrl(cmd), on_command)?;
        }
        KeyCode::Backspace => {
            delete_previous_char(app);
        }
        KeyCode::Delete => {
            delete_current_char(app);
        }
        KeyCode::Left => {
            app.command_cursor = app.command_cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            app.command_cursor = app
                .command_cursor
                .saturating_add(1)
                .min(app.command.chars().count());
        }
        KeyCode::Home => app.command_cursor = 0,
        KeyCode::End => app.command_cursor = app.command.chars().count(),
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.command_cursor = 0;
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.command_cursor = app.command.chars().count();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let end = byte_index_at_char(&app.command, app.command_cursor);
            app.command.replace_range(..end, "");
            app.command_cursor = 0;
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            delete_previous_word(app);
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            insert_text(app, &c.to_string());
        }
        _ => {}
    }

    Ok(false)
}

pub(crate) fn handle_paste(app: &mut App, text: &str) {
    if app.address_mode == AddressMode::Nav {
        return;
    }
    let multiline = if let AddressMode::LiveField(index) = app.address_mode {
        app.page
            .live_controls
            .get(index)
            .is_some_and(|control| control.multiline)
    } else {
        false
    };
    let normalized = if multiline {
        text.replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "  ")
    } else {
        text.replace(['\r', '\n', '\t'], " ")
    };
    insert_text(app, &normalized);
}

fn apply_live_choice<F>(app: &mut App, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let Some(editor) = app.live_choice_editor.clone() else {
        return Ok(());
    };
    let Some(control) = app.page.live_controls.get(editor.control_index) else {
        app.close_live_choice_editor();
        app.set_click_feedback("selection is no longer available");
        return Ok(());
    };
    let Some(option) = control.options.get(editor.selected_index) else {
        app.set_click_feedback("no selectable option");
        return Ok(());
    };
    if option.disabled {
        app.set_click_feedback("this option is disabled");
        return Ok(());
    }

    let index = editor.control_index;
    let label = option.label.clone();
    let value = option.value.clone();
    app.close_live_choice_editor();
    super::apply_command(app, UiCommand::FillLiveField(index, value), on_command)?;
    app.set_click_feedback(format!("selected {label}"));
    Ok(())
}

fn apply_live_text_edit<F>(app: &mut App, index: usize, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let value = app.command.clone();
    app.leave_address_edit();
    super::apply_command(app, UiCommand::FillLiveField(index, value), on_command)
}

fn insert_text(app: &mut App, text: &str) {
    let mut insertion = text;
    if matches!(app.address_mode, AddressMode::LiveField(_)) {
        let available = MAX_LIVE_FIELD_BYTES.saturating_sub(app.command.len());
        let mut end = text.len().min(available);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        insertion = &text[..end];
        if insertion.len() < text.len() {
            app.set_click_feedback("live field reached the 64 KiB editing limit");
        }
    }
    let byte_index = byte_index_at_char(&app.command, app.command_cursor);
    app.command.insert_str(byte_index, insertion);
    app.command_cursor = app
        .command_cursor
        .saturating_add(insertion.chars().count())
        .min(app.command.chars().count());
}

fn delete_previous_char(app: &mut App) {
    if app.command_cursor == 0 {
        return;
    }
    let start = byte_index_at_char(&app.command, app.command_cursor - 1);
    let end = byte_index_at_char(&app.command, app.command_cursor);
    app.command.replace_range(start..end, "");
    app.command_cursor = app.command_cursor.saturating_sub(1);
}

fn delete_current_char(app: &mut App) {
    let char_count = app.command.chars().count();
    if app.command_cursor >= char_count {
        return;
    }
    let start = byte_index_at_char(&app.command, app.command_cursor);
    let end = byte_index_at_char(&app.command, app.command_cursor + 1);
    app.command.replace_range(start..end, "");
}

fn delete_previous_word(app: &mut App) {
    let chars = app.command.chars().collect::<Vec<_>>();
    let mut start = app.command_cursor.min(chars.len());
    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let start_byte = byte_index_at_char(&app.command, start);
    let end_byte = byte_index_at_char(&app.command, app.command_cursor);
    app.command.replace_range(start_byte..end_byte, "");
    app.command_cursor = start;
}

fn byte_index_at_char(input: &str, char_index: usize) -> usize {
    input
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(input.len())
}

fn update_find(app: &mut App, query: &str) -> Result<()> {
    app.find_query = query.to_string();
    app.find_matches.clear();
    app.find_current = None;
    if query.is_empty() {
        app.set_click_feedback("find cleared");
        return Ok(());
    }
    let needle = query.to_lowercase();
    app.find_matches = app
        .page
        .rendered
        .lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.to_lowercase().contains(&needle).then_some(index))
        .collect();
    if app.find_matches.is_empty() {
        app.set_click_feedback(format!("no matches for /{query}"));
        return Ok(());
    }
    app.find_current = Some(0);
    jump_to_logical_line(app, app.find_matches[0])?;
    app.set_click_feedback(format!("match 1/{}", app.find_matches.len()));
    Ok(())
}

fn jump_find_match(app: &mut App, forward: bool) -> Result<()> {
    if app.find_matches.is_empty() {
        return Ok(());
    }
    let current = app.find_current.unwrap_or(0);
    let next = if forward {
        (current + 1) % app.find_matches.len()
    } else if current == 0 {
        app.find_matches.len() - 1
    } else {
        current - 1
    };
    app.find_current = Some(next);
    jump_to_logical_line(app, app.find_matches[next])?;
    app.set_click_feedback(format!("match {}/{}", next + 1, app.find_matches.len()));
    Ok(())
}

fn jump_to_logical_line(app: &mut App, logical_line: usize) -> Result<()> {
    let content_rect = crate::navigation::current_content_rect()?;
    let inner_width = content_rect.width.saturating_sub(2);
    let visual_lines =
        crate::navigation::build_visual_line_map(&app.page.rendered.lines, inner_width);
    if let Some(visual_row) = visual_lines
        .iter()
        .position(|line| line.logical_line_idx == logical_line)
    {
        app.cursor_visual_row = visual_row;
        crate::navigation::sync_scroll_to_cursor(app)?;
    }
    Ok(())
}

fn handle_media_picker_key<F>(app: &mut App, key: KeyEvent, on_command: &mut F) -> Result<bool>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    match key.code {
        KeyCode::Esc | KeyCode::Char('m') | KeyCode::Char('q') => {
            app.close_media_picker();
            app.set_click_feedback("media picker closed");
        }
        KeyCode::Down | KeyCode::Char('j') => app.move_media_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_media_selection(-1),
        KeyCode::PageDown => app.move_media_selection(8),
        KeyCode::PageUp => app.move_media_selection(-8),
        KeyCode::Home | KeyCode::Char('g') => app.selected_media_index = 0,
        KeyCode::End | KeyCode::Char('G') => {
            app.selected_media_index = app.page.media.len().saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Char(' ') => play_selected_media(app, on_command)?,
        KeyCode::Char('y') => copy_selected_media_url(app)?,
        _ => {}
    }
    Ok(false)
}

fn play_selected_media<F>(app: &mut App, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let index = app.selected_media_index;
    request_media_launch(app, index, on_command)?;
    Ok(())
}

pub(crate) fn request_media_launch<F>(app: &mut App, index: usize, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    if let Some(activity) = &app.page.media_activity {
        app.set_click_feedback(match activity.phase {
            crate::UiMediaPhase::Opening => format!("already opening {}", activity.label),
            crate::UiMediaPhase::Active => format!("player active {}", activity.label),
        });
        return Ok(());
    }
    let Some(media) = app.page.media.get(index) else {
        app.set_click_feedback("media unavailable");
        return Ok(());
    };
    app.set_click_feedback(format!("opening {}", media.label));
    super::apply_command(app, UiCommand::OpenMedia(index), on_command)
}

fn select_adjacent_image<F>(app: &mut App, delta: isize, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let len = app.page.images.len();
    if len == 0 {
        app.set_click_feedback("no images on this page");
        return Ok(());
    }
    let current = app.page.selected_image_index.unwrap_or(0).min(len - 1);
    let next = if delta < 0 {
        if current == 0 { len - 1 } else { current - 1 }
    } else {
        (current + 1) % len
    };
    super::apply_command(app, UiCommand::SelectImage(next), on_command)?;
    app.open_image_viewer();
    app.set_click_feedback(format!("image {}/{}", next + 1, len));
    Ok(())
}

fn action_url(app: &App, action: glance_render::InteractiveAction) -> Option<&str> {
    match action {
        glance_render::InteractiveAction::OpenLink(index) => {
            app.page.links.get(index).map(|item| item.url.as_str())
        }
        glance_render::InteractiveAction::OpenImage(index) => {
            app.page.images.get(index).map(|item| item.url.as_str())
        }
        glance_render::InteractiveAction::OpenMedia(index) => {
            app.page.media.get(index).map(|item| item.url.as_str())
        }
        glance_render::InteractiveAction::OpenForm(_) => None,
        glance_render::InteractiveAction::ActivateButton(_) => None,
        glance_render::InteractiveAction::EditLiveField(_) => None,
        glance_render::InteractiveAction::ActivateLiveControl(_) => None,
    }
}

fn action_hover_text(app: &App, action: glance_render::InteractiveAction) -> String {
    match action {
        glance_render::InteractiveAction::OpenLink(index) => app
            .page
            .links
            .get(index)
            .map(|item| format!("link: {} — {}", item.label, item.url)),
        glance_render::InteractiveAction::OpenImage(index) => app
            .page
            .images
            .get(index)
            .map(|item| format!("image: {} — {}", item.label, item.url)),
        glance_render::InteractiveAction::OpenMedia(index) => app
            .page
            .media
            .get(index)
            .map(|item| format!("{}: {} — {}", item.kind.label(), item.label, item.url)),
        glance_render::InteractiveAction::OpenForm(index) => {
            Some(format!("form {} — click to edit", index + 1))
        }
        glance_render::InteractiveAction::ActivateButton(index) => Some(format!(
            "continuation button {} — load more live content",
            index + 1
        )),
        glance_render::InteractiveAction::EditLiveField(index) => app
            .page
            .live_controls
            .get(index)
            .map(|control| format!("edit live field: {}", control.label)),
        glance_render::InteractiveAction::ActivateLiveControl(index) => {
            app.page.live_controls.get(index).map(|control| {
                if control.requires_confirmation {
                    format!("live action: {} — confirmation required", control.label)
                } else {
                    format!("open live control: {}", control.label)
                }
            })
        }
    }
    .unwrap_or_else(|| "action unavailable".to_string())
}

fn confirm_live_control<F>(app: &mut App, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let Some(confirmation) = app.control_confirmation.take() else {
        return Ok(());
    };
    let label = confirmation.label;
    super::apply_command(
        app,
        UiCommand::ActivateLiveControl(confirmation.index),
        on_command,
    )?;
    app.set_click_feedback(format!("confirmed {label}"));
    Ok(())
}

fn copy_url(app: &mut App, url: String) -> Result<()> {
    execute!(stdout(), CopyToClipboard::to_clipboard_from(url.as_bytes()))?;
    app.set_click_feedback(format!("copied URL: {}", super::one_line_label(&url)));
    Ok(())
}

fn copy_cursor_target_url(app: &mut App) -> Result<()> {
    let Some((_, span)) = cursor_target(app)? else {
        app.set_click_feedback("no URL on this line");
        return Ok(());
    };
    let Some(url) = action_url(app, span.action).map(ToString::to_string) else {
        app.set_click_feedback("this action has no copyable URL");
        return Ok(());
    };
    copy_url(app, url)
}

fn copy_selected_media_url(app: &mut App) -> Result<()> {
    let Some(url) = app
        .page
        .media
        .get(app.selected_media_index)
        .map(|media| media.url.clone())
    else {
        app.set_click_feedback("no media URL to copy");
        return Ok(());
    };
    copy_url(app, url)
}

fn copy_selected_image_url(app: &mut App) -> Result<()> {
    let Some(url) = app
        .page
        .selected_image_index
        .and_then(|index| app.page.images.get(index))
        .map(|image| image.url.clone())
    else {
        app.set_click_feedback("no image URL to copy");
        return Ok(());
    };
    copy_url(app, url)
}

pub(crate) fn handle_mouse<F>(app: &mut App, mouse: MouseEvent, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let (w, h) = crossterm::terminal::size()?;
    let screen = Rect::new(0, 0, w, h);
    let layout = super::compute_layout(screen);

    if app.control_confirmation.is_some() {
        if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
            let (cancel, confirm) = control_confirmation_buttons(screen);
            if super::point_in_rect(cancel, mouse.column, mouse.row) {
                app.cancel_control_confirmation();
                app.set_click_feedback("live action cancelled");
            } else if super::point_in_rect(confirm, mouse.column, mouse.row) {
                confirm_live_control(app, on_command)?;
            }
        }
        return Ok(());
    }

    if app.live_choice_editor.is_some() {
        if matches!(mouse.kind, MouseEventKind::ScrollDown) {
            app.move_live_choice(1);
        } else if matches!(mouse.kind, MouseEventKind::ScrollUp) {
            app.move_live_choice(-1);
        } else if matches!(mouse.kind, MouseEventKind::Moved) {
            let options = live_choice_options_rect(screen);
            if super::point_in_rect(options, mouse.column, mouse.row)
                && let Some(editor) = app.live_choice_editor.as_ref()
            {
                let control_index = editor.control_index;
                let window_start = app
                    .page
                    .live_controls
                    .get(control_index)
                    .map(|control| {
                        live_choice_window_start(
                            editor.selected_index,
                            control.options.len(),
                            options.height as usize,
                        )
                    })
                    .unwrap_or(0);
                let index = window_start + mouse.row.saturating_sub(options.y) as usize;
                let selectable = app
                    .page
                    .live_controls
                    .get(control_index)
                    .and_then(|control| control.options.get(index))
                    .is_some_and(|option| !option.disabled);
                if selectable && let Some(editor) = &mut app.live_choice_editor {
                    editor.selected_index = index;
                }
            }
        } else if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
            let options = live_choice_options_rect(screen);
            if super::point_in_rect(options, mouse.column, mouse.row) {
                let Some(editor) = app.live_choice_editor.as_ref() else {
                    return Ok(());
                };
                let control_index = editor.control_index;
                let window_start = app
                    .page
                    .live_controls
                    .get(control_index)
                    .map(|control| {
                        live_choice_window_start(
                            editor.selected_index,
                            control.options.len(),
                            options.height as usize,
                        )
                    })
                    .unwrap_or(0);
                let index = window_start + mouse.row.saturating_sub(options.y) as usize;
                let selectable = app
                    .page
                    .live_controls
                    .get(control_index)
                    .and_then(|control| control.options.get(index))
                    .is_some_and(|option| !option.disabled);
                if selectable {
                    if let Some(editor) = &mut app.live_choice_editor {
                        editor.selected_index = index;
                    }
                    apply_live_choice(app, on_command)?;
                } else {
                    app.set_click_feedback("this option is disabled");
                }
            } else {
                app.close_live_choice_editor();
                app.set_click_feedback("selection cancelled");
            }
        }
        return Ok(());
    }

    if let AddressMode::LiveField(index) = app.address_mode {
        if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
            let multiline = app
                .page
                .live_controls
                .get(index)
                .is_some_and(|control| control.multiline);
            let (cancel, apply) = live_text_editor_buttons(screen, multiline);
            if super::point_in_rect(cancel, mouse.column, mouse.row) {
                app.leave_address_edit();
                app.set_click_feedback("edit cancelled");
            } else if super::point_in_rect(apply, mouse.column, mouse.row) {
                apply_live_text_edit(app, index, on_command)?;
            }
        }
        return Ok(());
    }

    match mouse.kind {
        MouseEventKind::Moved => update_hover(app, screen, layout, mouse.column, mouse.row),
        MouseEventKind::Down(MouseButton::Left) => {
            app.last_left_down = Some((mouse.column, mouse.row));
            app.mouse_dragged = false;
            handle_left_press(app, screen, layout, mouse.column, mouse.row)?;
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            app.mouse_dragged = true;
            handle_left_drag(app, screen, layout, mouse.column, mouse.row)?;
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let activate = app.last_left_down == Some((mouse.column, mouse.row))
                && !app.mouse_dragged
                && !app.scrollbar_dragging;
            app.last_left_down = None;
            app.mouse_dragged = false;
            app.scrollbar_dragging = false;
            if activate {
                handle_left_click(app, screen, layout, mouse.column, mouse.row, on_command)?;
            }
        }
        MouseEventKind::Down(MouseButton::Middle) => {
            if app.help_open {
                app.set_click_feedback("press ? or Esc to close the command palette");
            } else if app.media_picker_open {
                select_media_at_pointer(app, screen, mouse.column, mouse.row);
                play_selected_media(app, on_command)?;
            } else if let Some(sidebar) = layout.sidebar
                && super::point_in_rect(sidebar, mouse.column, mouse.row)
                && let Some(action) = sidebar_action_at(app, sidebar, mouse.row)
            {
                activate_sidebar_action(app, action, true, on_command)?;
            } else if let Some(action) =
                content_action_at_click(app, layout.content, mouse.column, mouse.row)
            {
                if let Some(url) = action_url(app, action).map(ToString::to_string) {
                    app.set_click_feedback("middle-click: open in GUI browser");
                    super::apply_command(app, UiCommand::OpenExternal(url), on_command)?;
                } else {
                    super::execute_action(app, action, on_command)?;
                }
            }
        }
        MouseEventKind::Down(MouseButton::Right) => {
            handle_right_click(app, screen, layout, mouse.column, mouse.row)?;
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if app.image_viewer_open => {
            let delta = if matches!(mouse.kind, MouseEventKind::ScrollDown) {
                1
            } else {
                -1
            };
            select_adjacent_image(app, delta, on_command)?;
        }
        _ => {}
    }

    Ok(())
}

pub(crate) fn queue_scroll_delta(
    app: &App,
    mouse: MouseEvent,
    acc: &mut ScrollAccumulator,
) -> Result<bool> {
    if app.help_open
        || app.live_choice_editor.is_some()
        || matches!(app.address_mode, AddressMode::LiveField(_))
    {
        return Ok(false);
    }
    let (w, h) = crossterm::terminal::size()?;
    let screen = Rect::new(0, 0, w, h);
    let layout = super::compute_layout(screen);

    let step = if mouse.modifiers.contains(KeyModifiers::CONTROL) {
        layout.content.height.saturating_sub(3).max(1) as i16
    } else if mouse.modifiers.contains(KeyModifiers::SHIFT) {
        8
    } else {
        3
    };
    let delta = match mouse.kind {
        MouseEventKind::ScrollDown => step,
        MouseEventKind::ScrollUp => -step,
        _ => 0,
    };
    if delta == 0 {
        return Ok(false);
    }

    if app.media_picker_open {
        acc.media_delta = acc.media_delta.saturating_add(delta.signum());
        return Ok(false);
    } else if super::point_in_rect(layout.content, mouse.column, mouse.row) {
        acc.content_delta = acc.content_delta.saturating_add(delta);
        acc.set_content_focus = true;
        return Ok(delta > 0);
    }

    Ok(false)
}

pub(crate) fn flush_scroll_accumulator(app: &mut App, acc: &mut ScrollAccumulator) -> Result<()> {
    if acc.media_delta != 0 {
        app.move_media_selection(acc.media_delta as isize);
    }
    let content_rect = crate::navigation::current_content_rect()?;
    if acc.content_delta > 0 {
        app.content_scroll = app.content_scroll.saturating_add(acc.content_delta as u16);
    } else if acc.content_delta < 0 {
        app.content_scroll = app
            .content_scroll
            .saturating_sub((-acc.content_delta) as u16);
    }
    clamp_content_scroll(app, content_rect);

    if acc.set_content_focus {
        app.set_focus(FocusPane::Content);
    }

    *acc = ScrollAccumulator::default();
    Ok(())
}

fn handle_left_press(
    app: &mut App,
    screen: Rect,
    layout: UiLayout,
    column: u16,
    row: u16,
) -> Result<()> {
    if app.media_picker_open {
        select_media_at_pointer(app, screen, column, row);
        return Ok(());
    }
    if app.image_viewer_open || app.help_open {
        return Ok(());
    }
    if is_content_scrollbar(app, layout.content, column, row) {
        app.scrollbar_dragging = true;
        set_scroll_from_pointer(app, layout.content, row)?;
        return Ok(());
    }
    if super::point_in_rect(layout.content, column, row) {
        app.set_focus(FocusPane::Content);
        set_cursor_from_screen_row(app, layout.content, row);
        clamp_cursor_to_terminal(app)?;
    }
    Ok(())
}

fn handle_left_drag(
    app: &mut App,
    screen: Rect,
    layout: UiLayout,
    column: u16,
    row: u16,
) -> Result<()> {
    if app.media_picker_open {
        select_media_at_pointer(app, screen, column, row);
        return Ok(());
    }
    if app.image_viewer_open {
        return Ok(());
    }
    if app.scrollbar_dragging {
        set_scroll_from_pointer(app, layout.content, row)?;
        return Ok(());
    }
    if super::point_in_rect(layout.content, column, row) {
        let top = layout.content.y.saturating_add(1);
        let bottom = layout.content.bottom().saturating_sub(1);
        if bottom <= top {
            return Ok(());
        }
        if row <= top {
            app.content_scroll = app.content_scroll.saturating_sub(1);
        } else if row >= bottom.saturating_sub(1) {
            app.content_scroll = app.content_scroll.saturating_add(1);
        }
        clamp_content_scroll(app, layout.content);
        set_cursor_from_screen_row(
            app,
            layout.content,
            row.clamp(top, bottom.saturating_sub(1)),
        );
        clamp_cursor_to_terminal(app)?;
    }
    Ok(())
}

fn is_content_scrollbar(app: &App, content: Rect, column: u16, row: u16) -> bool {
    max_content_scroll(app, content) > 0
        && column == content.right().saturating_sub(1)
        && row > content.y
        && row < content.bottom().saturating_sub(1)
}

fn set_scroll_from_pointer(app: &mut App, content: Rect, row: u16) -> Result<()> {
    let top = content.y.saturating_add(1);
    let track_len = content.height.saturating_sub(2);
    let max_scroll = max_content_scroll(app, content);
    if track_len <= 1 || max_scroll == 0 {
        return Ok(());
    }
    let relative = row
        .clamp(top, content.bottom().saturating_sub(2))
        .saturating_sub(top);
    app.content_scroll =
        ((relative as u32 * max_scroll as u32) / track_len.saturating_sub(1) as u32) as u16;
    app.cursor_visual_row = app.content_scroll as usize;
    clamp_cursor_to_terminal(app)
}

fn select_media_at_pointer(app: &mut App, screen: Rect, column: u16, row: u16) {
    let picker = media_picker_rect(screen);
    let layout = media_picker_layout(picker);
    if !super::point_in_rect(layout.list, column, row) {
        return;
    }
    let start = media_window_start(
        app.selected_media_index,
        app.page.media.len(),
        layout.list.height as usize,
    );
    let index = start.saturating_add(row.saturating_sub(layout.list.y) as usize);
    if index < app.page.media.len() {
        app.selected_media_index = index;
    }
}

fn update_hover(app: &mut App, screen: Rect, layout: UiLayout, column: u16, row: u16) {
    app.clear_hover();
    if app.media_picker_open {
        let picker = media_picker_rect(screen);
        let picker_layout = media_picker_layout(picker);
        if super::point_in_rect(picker_layout.list, column, row) {
            let start = media_window_start(
                app.selected_media_index,
                app.page.media.len(),
                picker_layout.list.height as usize,
            );
            let index = start.saturating_add(row.saturating_sub(picker_layout.list.y) as usize);
            if let Some(media) = app.page.media.get(index) {
                app.hovered_media_index = Some(index);
                app.hover_feedback = Some(format!(
                    "{}: {} — {}",
                    media.kind.label(),
                    media.label,
                    media.url
                ));
            }
        }
        return;
    }
    if app.help_open {
        app.hover_feedback = Some("press ? or Esc to close the command palette".to_string());
        return;
    }
    if app.image_viewer_open {
        if let Some(image) = app
            .page
            .selected_image_index
            .and_then(|index| app.page.images.get(index))
        {
            app.hover_feedback = Some(format!("image: {} — {}", image.label, image.url));
        }
        return;
    }
    if let Some(sidebar) = layout.sidebar
        && super::point_in_rect(sidebar, column, row)
    {
        if let Some(action) = sidebar_action_at(app, sidebar, row) {
            app.hovered_sidebar_action = Some(action);
            app.hover_feedback = Some(sidebar_hover_text(app, action));
        } else {
            app.hover_feedback = Some("page overview and detected media".to_string());
        }
        return;
    }
    if let Some(action) = content_action_at_click(app, layout.content, column, row) {
        app.hovered_action = Some(action);
        app.hover_feedback = Some(action_hover_text(app, action));
        return;
    }
    if is_content_scrollbar(app, layout.content, column, row) {
        app.hover_feedback = Some("drag scrollbar to move through the page".to_string());
        return;
    }
    if super::point_in_rect(layout.address, column, row) {
        let hitboxes = address_hitboxes(layout.address);
        app.hover_feedback = Some(if hitboxes.external.contains(&column) {
            "open this page in the GUI browser".to_string()
        } else if hitboxes.dynamic.contains(&column) {
            "upgrade this page with a dynamic Chromium DOM".to_string()
        } else {
            "click to edit address or use navigation buttons".to_string()
        });
        return;
    }
    if super::point_in_rect(layout.footer, column, row) {
        let hitboxes = footer_hitboxes(app, layout.footer);
        if hitboxes.web.contains(&column) {
            app.hover_feedback = Some("upgrade this page with a dynamic Chromium DOM".to_string());
        } else if hitboxes.external.contains(&column) {
            app.hover_feedback = Some("open this page in the GUI browser".to_string());
        } else if hitboxes
            .auth
            .as_ref()
            .is_some_and(|range| range.contains(&column))
        {
            app.hover_feedback = Some("continue sign-in securely in the GUI browser".to_string());
        } else if hitboxes.img.contains(&column) {
            app.hover_feedback = Some("toggle inline image mode".to_string());
        } else if hitboxes.media.contains(&column) {
            app.hover_feedback = Some("open media picker".to_string());
        } else if hitboxes.help.contains(&column) {
            app.hover_feedback = Some("open shortcuts".to_string());
        }
    }
}

fn handle_right_click(
    app: &mut App,
    screen: Rect,
    layout: UiLayout,
    column: u16,
    row: u16,
) -> Result<()> {
    if app.media_picker_open {
        select_media_at_pointer(app, screen, column, row);
        return copy_selected_media_url(app);
    }
    if app.image_viewer_open {
        return copy_selected_image_url(app);
    }
    if app.help_open {
        app.set_click_feedback("press ? or Esc to close the command palette");
        return Ok(());
    }
    if let Some(sidebar) = layout.sidebar
        && super::point_in_rect(sidebar, column, row)
    {
        let Some(action) = sidebar_action_at(app, sidebar, row) else {
            app.set_click_feedback("right-click a media item to copy its URL");
            return Ok(());
        };
        return copy_sidebar_url(app, action);
    }
    let Some(action) = content_action_at_click(app, layout.content, column, row) else {
        app.set_click_feedback("right-click an actionable item to copy its URL");
        return Ok(());
    };
    let Some(url) = action_url(app, action).map(ToString::to_string) else {
        app.set_click_feedback("this action has no copyable URL");
        return Ok(());
    };
    copy_url(app, url)
}

fn handle_left_click<F>(
    app: &mut App,
    screen: Rect,
    layout: UiLayout,
    column: u16,
    row: u16,
    on_command: &mut F,
) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    if app.help_open {
        app.close_help();
        app.set_click_feedback("command palette closed");
        return Ok(());
    }

    if app.media_picker_open {
        handle_media_picker_click(app, screen, column, row, on_command)?;
        return Ok(());
    }

    if app.image_viewer_open {
        handle_image_viewer_click(app, screen, column, row, on_command)?;
        return Ok(());
    }

    if super::point_in_rect(layout.footer, column, row) {
        handle_footer_click(app, layout.footer, column, row, on_command)?;
        return Ok(());
    }

    if super::point_in_rect(layout.address, column, row) {
        handle_address_click(app, layout.address, column, row, on_command)?;
        return Ok(());
    }

    if let Some(sidebar) = layout.sidebar
        && super::point_in_rect(sidebar, column, row)
    {
        if let Some(action) = sidebar_action_at(app, sidebar, row) {
            activate_sidebar_action(app, action, false, on_command)?;
        } else {
            app.set_click_feedback("page dock");
        }
        return Ok(());
    }

    if super::point_in_rect(layout.content, column, row) {
        app.set_focus(FocusPane::Content);
        set_cursor_from_screen_row(app, layout.content, row);
        clamp_cursor_to_terminal(app)?;
        if let Some(action) = content_action_at_click(app, layout.content, column, row) {
            super::execute_action(app, action, on_command)?;
        } else {
            app.set_click_feedback("click: content pane");
        }
        return Ok(());
    }

    Ok(())
}

fn handle_media_picker_click<F>(
    app: &mut App,
    screen: Rect,
    column: u16,
    row: u16,
    on_command: &mut F,
) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let picker = media_picker_rect(screen);
    if !super::point_in_rect(picker, column, row) {
        app.close_media_picker();
        app.set_click_feedback("media picker closed");
        return Ok(());
    }
    let layout = media_picker_layout(picker);
    if super::point_in_rect(layout.list, column, row) {
        select_media_at_pointer(app, screen, column, row);
        if let Some(media) = app.page.media.get(app.selected_media_index) {
            app.set_click_feedback(format!("selected {} {}", media.kind.label(), media.label));
        }
        return Ok(());
    }
    if row != layout.controls.y {
        return Ok(());
    }

    let ranges = control_ranges(
        layout.controls.x,
        &[" ▶ Play ", " ← ", " → ", " Copy ", " × "],
    );
    if ranges[0].contains(&column) {
        play_selected_media(app, on_command)?;
    } else if ranges[1].contains(&column) {
        app.move_media_selection(-1);
    } else if ranges[2].contains(&column) {
        app.move_media_selection(1);
    } else if ranges[3].contains(&column) {
        copy_selected_media_url(app)?;
    } else if ranges[4].contains(&column) {
        app.close_media_picker();
        app.set_click_feedback("media picker closed");
    }
    Ok(())
}

fn handle_image_viewer_click<F>(
    app: &mut App,
    screen: Rect,
    column: u16,
    row: u16,
    on_command: &mut F,
) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let viewer = super::viewer_rect(screen);
    if !super::point_in_rect(viewer, column, row) {
        app.close_image_viewer();
        app.set_click_feedback("image viewer closed");
        return Ok(());
    }
    let controls = image_viewer_controls_rect(viewer);
    if row != controls.y {
        return Ok(());
    }
    let ranges = control_ranges(controls.x, &[" ← ", " → ", " Copy ", " × "]);
    if ranges[0].contains(&column) {
        select_adjacent_image(app, -1, on_command)?;
    } else if ranges[1].contains(&column) {
        select_adjacent_image(app, 1, on_command)?;
    } else if ranges[2].contains(&column) {
        copy_selected_image_url(app)?;
    } else if ranges[3].contains(&column) {
        app.close_image_viewer();
        app.set_click_feedback("image viewer closed");
    }
    Ok(())
}

fn control_ranges(start: u16, labels: &[&str]) -> Vec<std::ops::Range<u16>> {
    let mut cursor = start;
    labels
        .iter()
        .map(|label| {
            let end = cursor.saturating_add(width_u16(label));
            let range = cursor..end;
            cursor = end.saturating_add(2);
            range
        })
        .collect()
}

fn handle_footer_click<F>(
    app: &mut App,
    footer_rect: Rect,
    column: u16,
    row: u16,
    on_command: &mut F,
) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    if row != footer_rect.y.saturating_add(1) {
        return Ok(());
    }

    let hitboxes = footer_hitboxes(app, footer_rect);
    if hitboxes.web.contains(&column) {
        app.set_click_feedback("click: dynamic DOM upgrade");
        super::apply_command(app, UiCommand::ReloadDynamic, on_command)?;
    } else if hitboxes.external.contains(&column) {
        open_current_in_gui(app, on_command)?;
    } else if hitboxes
        .auth
        .as_ref()
        .is_some_and(|range| range.contains(&column))
    {
        open_auth_in_gui(app, on_command)?;
    } else if hitboxes.img.contains(&column) {
        app.set_click_feedback("click: image mode toggle");
        super::apply_command(app, UiCommand::ToggleImageMode, on_command)?;
        if app.page.image_mode == UiImageMode::Text {
            app.close_image_viewer();
        }
    } else if hitboxes.media.contains(&column) && app.open_media_picker() {
        app.set_click_feedback(format!("{} media items", app.page.media.len()));
    } else if hitboxes.help.contains(&column) {
        app.open_help();
    }
    Ok(())
}

fn handle_address_click<F>(
    app: &mut App,
    address_rect: Rect,
    column: u16,
    row: u16,
    on_command: &mut F,
) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    app.set_focus(FocusPane::Address);

    if app.address_mode != AddressMode::Nav {
        return Ok(());
    }

    if row == address_rect.y.saturating_add(1) {
        let hitboxes = address_hitboxes(address_rect);
        if hitboxes.back.contains(&column) {
            app.set_click_feedback("click: back");
            super::apply_command(app, UiCommand::Back, on_command)?;
            return Ok(());
        }
        if hitboxes.forward.contains(&column) {
            app.set_click_feedback("click: forward");
            super::apply_command(app, UiCommand::Forward, on_command)?;
            return Ok(());
        }
        if hitboxes.reload.contains(&column) {
            app.set_click_feedback("click: reload");
            super::apply_command(app, UiCommand::Reload, on_command)?;
            return Ok(());
        }
        if hitboxes.dynamic.contains(&column) {
            app.set_click_feedback("click: dynamic DOM upgrade");
            super::apply_command(app, UiCommand::ReloadDynamic, on_command)?;
            return Ok(());
        }
        if hitboxes.external.contains(&column) {
            open_current_in_gui(app, on_command)?;
            return Ok(());
        }
    }

    app.enter_address_edit(app.page.rendered.source_url.clone());
    app.set_click_feedback("click: edit url");
    Ok(())
}

fn address_hitboxes(address_rect: Rect) -> AddressHitboxes {
    let inner_start = address_rect.x.saturating_add(1);

    let back = " ← ";
    let forward = " → ";
    let reload = " ↻ ";
    let dynamic = " ⚡ ";
    let external = " ↗ ";

    let back_start = inner_start;
    let back_end = back_start.saturating_add(width_u16(back));

    let forward_start = back_end.saturating_add(1);
    let forward_end = forward_start.saturating_add(width_u16(forward));

    let reload_start = forward_end.saturating_add(1);
    let reload_end = reload_start.saturating_add(width_u16(reload));

    let dynamic_start = reload_end.saturating_add(1);
    let dynamic_end = dynamic_start.saturating_add(width_u16(dynamic));

    let external_start = dynamic_end.saturating_add(1);
    let external_end = external_start.saturating_add(width_u16(external));

    AddressHitboxes {
        back: back_start..back_end,
        forward: forward_start..forward_end,
        reload: reload_start..reload_end,
        dynamic: dynamic_start..dynamic_end,
        external: external_start..external_end,
    }
}

pub(crate) fn footer_hitboxes(app: &App, footer_rect: Rect) -> FooterHitboxes {
    let inner_start = footer_rect.x.saturating_add(1);

    let web_token = format!(" ●{} ", app.page.web_mode.label());
    let web_start = inner_start;
    let web_end = web_start.saturating_add(width_u16(&web_token));

    let external_token = " ↗ ";
    let external_start = web_end.saturating_add(1);
    let external_end = external_start.saturating_add(width_u16(external_token));

    let auth = app.page.auth.as_ref().map(|_| {
        let auth_token = " ♙ Sign in ";
        let auth_start = external_end.saturating_add(1);
        let auth_end = auth_start.saturating_add(width_u16(auth_token));
        auth_start..auth_end
    });

    let media_token = app
        .page
        .media_activity
        .as_ref()
        .map(|activity| match activity.phase {
            crate::UiMediaPhase::Opening => " ◐ ".to_string(),
            crate::UiMediaPhase::Active => " ■ ".to_string(),
        })
        .unwrap_or_else(|| format!(" ▶{} ", app.page.media.len()));
    let media_start = auth
        .as_ref()
        .map(|range| range.end.saturating_add(1))
        .unwrap_or_else(|| external_end.saturating_add(1));
    let media_end = media_start.saturating_add(width_u16(&media_token));

    let img_token = " ◇ ";

    let img_start = media_end.saturating_add(1);
    let img_end = img_start.saturating_add(width_u16(img_token));

    let help_token = " ? ";
    let help_start = img_end.saturating_add(1);
    let help_end = help_start.saturating_add(width_u16(help_token));

    FooterHitboxes {
        web: web_start..web_end,
        auth,
        external: external_start..external_end,
        img: img_start..img_end,
        media: media_start..media_end,
        help: help_start..help_end,
    }
}

fn sidebar_hover_text(app: &App, action: SidebarAction) -> String {
    match action {
        SidebarAction::Auth => app
            .page
            .auth
            .as_ref()
            .map(|item| format!("secure browser sign in: {} — {}", item.label, item.url)),
        SidebarAction::Image(index) => app
            .page
            .images
            .get(index)
            .map(|item| format!("image: {} — {}", item.label, item.url)),
        SidebarAction::Media(index) => app
            .page
            .media
            .get(index)
            .map(|item| format!("{}: {} — {}", item.kind.label(), item.label, item.url)),
    }
    .unwrap_or_else(|| "media unavailable".to_string())
}

fn copy_sidebar_url(app: &mut App, action: SidebarAction) -> Result<()> {
    let url = match action {
        SidebarAction::Auth => app.page.auth.as_ref().map(|item| item.url.clone()),
        SidebarAction::Image(index) => app.page.images.get(index).map(|item| item.url.clone()),
        SidebarAction::Media(index) => app.page.media.get(index).map(|item| item.url.clone()),
    };
    let Some(url) = url else {
        app.set_click_feedback("media URL unavailable");
        return Ok(());
    };
    copy_url(app, url)
}

fn activate_sidebar_action<F>(
    app: &mut App,
    action: SidebarAction,
    launch_media: bool,
    on_command: &mut F,
) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    match action {
        SidebarAction::Auth => {
            open_auth_in_gui(app, on_command)?;
        }
        SidebarAction::Image(index) => {
            super::apply_command(app, UiCommand::SelectImage(index), on_command)?;
            if launch_media {
                app.open_image_viewer();
                app.set_click_feedback(format!(
                    "image {}/{} · full view",
                    index + 1,
                    app.page.images.len()
                ));
            } else {
                app.set_click_feedback(format!(
                    "image {}/{} · preview in Page Dock",
                    index + 1,
                    app.page.images.len()
                ));
            }
        }
        SidebarAction::Media(index) if launch_media => {
            app.selected_media_index = index;
            play_selected_media(app, on_command)?;
        }
        SidebarAction::Media(index) => {
            app.selected_media_index = index;
            if app.open_media_picker() {
                app.set_click_feedback(format!("media {}/{}", index + 1, app.page.media.len()));
            }
        }
    }
    Ok(())
}

fn open_auth_in_gui<F>(app: &mut App, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let Some(url) = app.page.auth.as_ref().map(|auth| auth.url.clone()) else {
        app.set_click_feedback("no sign-in entry was detected on this page");
        return Ok(());
    };
    app.set_click_feedback("opening isolated secure sign-in browser");
    super::apply_command(app, UiCommand::OpenAuth(url), on_command)
}

fn open_current_in_gui<F>(app: &mut App, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let url = app.page.rendered.source_url.clone();
    if url.trim().is_empty() {
        app.set_click_feedback("no page to open in GUI browser");
        return Ok(());
    }
    app.set_click_feedback("opening page in GUI browser");
    super::apply_command(app, UiCommand::OpenExternal(url), on_command)
}

fn width_u16(s: &str) -> u16 {
    UnicodeWidthStr::width(s) as u16
}
