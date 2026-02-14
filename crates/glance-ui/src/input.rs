use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app_state::{
    AddressHitboxes, AddressMode, App, FocusPane, FooterHitboxes, ScrollAccumulator, UiLayout,
};
use crate::navigation::{
    clamp_content_scroll, content_action_at_click, execute_cursor_target, jump_focused,
    move_cursor_line, next_focus, page_move_cursor, set_cursor_from_screen_row,
};
use crate::{UiCommand, UiImageMode};

pub(crate) fn handle_key<F>(app: &mut App, key: KeyEvent, on_command: &mut F) -> Result<bool>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    if app.image_viewer_open {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char(' ') => {
                app.close_image_viewer();
                app.set_click_feedback("image viewer closed");
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    if app.address_mode == AddressMode::Edit {
        return handle_address_edit_mode(app, key, on_command);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Tab | KeyCode::BackTab => app.set_focus(next_focus(app.focus)),
        KeyCode::Char('o') | KeyCode::Char(':') | KeyCode::Char('/') => {
            app.enter_address_edit(app.page.rendered.source_url.clone());
        }
        KeyCode::Char('j') | KeyCode::Down => move_cursor_line(app, true)?,
        KeyCode::Char('k') | KeyCode::Up => move_cursor_line(app, false)?,
        KeyCode::Char('n') | KeyCode::PageDown => page_move_cursor(app, true)?,
        KeyCode::Char('p') | KeyCode::PageUp => page_move_cursor(app, false)?,
        KeyCode::Enter => execute_cursor_target(app, on_command)?,
        KeyCode::Char('z') => super::apply_command(app, UiCommand::ToggleJsDefault, on_command)?,
        KeyCode::Char('i') => {
            super::apply_command(app, UiCommand::ToggleImageMode, on_command)?;
            if app.page.image_mode == UiImageMode::Text {
                app.close_image_viewer();
            }
        }
        KeyCode::Char('g') => jump_focused(app, false)?,
        KeyCode::Char('G') => jump_focused(app, true)?,
        KeyCode::Char('r') => super::apply_command(app, UiCommand::Reload, on_command)?,
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
    match key.code {
        KeyCode::Esc => {
            app.leave_address_edit();
            app.set_click_feedback("edit cancelled");
        }
        KeyCode::Enter => {
            let cmd = app.command.trim().to_string();
            app.leave_address_edit();

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
            if cmd == "back" || cmd == "b" {
                super::apply_command(app, UiCommand::Back, on_command)?;
                return Ok(false);
            }
            if cmd == "forward" || cmd == "f" {
                super::apply_command(app, UiCommand::Forward, on_command)?;
                return Ok(false);
            }
            if cmd == "js" {
                super::apply_command(app, UiCommand::ToggleJsDefault, on_command)?;
                return Ok(false);
            }
            if cmd == "img" {
                super::apply_command(app, UiCommand::ToggleImageMode, on_command)?;
                if app.page.image_mode == UiImageMode::Text {
                    app.close_image_viewer();
                }
                return Ok(false);
            }
            if let Some(idx) = super::parse_index_command(&cmd, "link") {
                super::apply_command(app, UiCommand::OpenLink(idx), on_command)?;
                return Ok(false);
            }
            if let Some(idx) = super::parse_index_command(&cmd, "media") {
                super::apply_command(app, UiCommand::OpenMedia(idx), on_command)?;
                return Ok(false);
            }
            if let Some(idx) = super::parse_index_command(&cmd, "image") {
                super::apply_command(app, UiCommand::SelectImage(idx), on_command)?;
                app.open_image_viewer();
                return Ok(false);
            }
            if cmd == "help" {
                app.set_click_feedback(
                    "keys: j/k line, n/p page, Enter open, :link/:image/:media N",
                );
                return Ok(false);
            }

            super::apply_command(app, UiCommand::OpenUrl(cmd), on_command)?;
        }
        KeyCode::Backspace => {
            app.command.pop();
        }
        KeyCode::Char(c) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.command.push(c);
            }
        }
        _ => {}
    }

    Ok(false)
}

pub(crate) fn handle_mouse<F>(app: &mut App, mouse: MouseEvent, on_command: &mut F) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    let (w, h) = crossterm::terminal::size()?;
    let screen = Rect::new(0, 0, w, h);
    let layout = super::compute_layout(screen);

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            app.last_left_down = Some((mouse.column, mouse.row));
            handle_left_click(app, layout, mouse.column, mouse.row, on_command)?;
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let same_cell_up = app.last_left_down == Some((mouse.column, mouse.row));
            app.last_left_down = None;
            if !same_cell_up {
                handle_left_click(app, layout, mouse.column, mouse.row, on_command)?;
            }
        }
        MouseEventKind::ScrollDown => {
            if super::point_in_rect(layout.content, mouse.column, mouse.row) {
                app.set_focus(FocusPane::Content);
                app.content_scroll = app.content_scroll.saturating_add(2);
                clamp_content_scroll(app, layout.content);
            }
        }
        MouseEventKind::ScrollUp => {
            if super::point_in_rect(layout.content, mouse.column, mouse.row) {
                app.set_focus(FocusPane::Content);
                app.content_scroll = app.content_scroll.saturating_sub(2);
                clamp_content_scroll(app, layout.content);
            }
        }
        _ => {}
    }

    Ok(())
}

pub(crate) fn queue_scroll_delta(mouse: MouseEvent, acc: &mut ScrollAccumulator) -> Result<()> {
    let (w, h) = crossterm::terminal::size()?;
    let screen = Rect::new(0, 0, w, h);
    let layout = super::compute_layout(screen);

    let delta = match mouse.kind {
        MouseEventKind::ScrollDown => 2,
        MouseEventKind::ScrollUp => -2,
        _ => 0,
    };
    if delta == 0 {
        return Ok(());
    }

    if super::point_in_rect(layout.content, mouse.column, mouse.row) {
        acc.content_delta = acc.content_delta.saturating_add(delta);
        acc.set_content_focus = true;
    }

    Ok(())
}

pub(crate) fn flush_scroll_accumulator(app: &mut App, acc: &mut ScrollAccumulator) -> Result<()> {
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

fn handle_left_click<F>(
    app: &mut App,
    layout: UiLayout,
    column: u16,
    row: u16,
    on_command: &mut F,
) -> Result<()>
where
    F: FnMut(UiCommand) -> Result<crate::UiPage>,
{
    if app.image_viewer_open {
        app.close_image_viewer();
        app.set_click_feedback("image viewer closed");
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

    if super::point_in_rect(layout.content, column, row) {
        app.set_focus(FocusPane::Content);
        set_cursor_from_screen_row(app, layout.content, row);
        if let Some(action) = content_action_at_click(app, layout.content, column, row) {
            super::execute_action(app, action, on_command)?;
        } else {
            app.set_click_feedback("click: content pane");
        }
        return Ok(());
    }

    Ok(())
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
    if hitboxes.js.contains(&column) {
        app.set_click_feedback("click: js toggle");
        super::apply_command(app, UiCommand::ToggleJsDefault, on_command)?;
    } else if hitboxes.img.contains(&column) {
        app.set_click_feedback("click: image mode toggle");
        super::apply_command(app, UiCommand::ToggleImageMode, on_command)?;
        if app.page.image_mode == UiImageMode::Text {
            app.close_image_viewer();
        }
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

    if row == address_rect.y.saturating_add(1) && app.address_mode == AddressMode::Nav {
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
    }

    app.enter_address_edit(app.page.rendered.source_url.clone());
    app.set_click_feedback("click: edit url");
    Ok(())
}

fn address_hitboxes(address_rect: Rect) -> AddressHitboxes {
    let inner_start = address_rect.x.saturating_add(1);

    let back = "[]";
    let forward = "[]";
    let reload = "[]";

    let back_start = inner_start;
    let back_end = back_start.saturating_add(width_u16(back));

    let forward_start = back_end.saturating_add(1);
    let forward_end = forward_start.saturating_add(width_u16(forward));

    let reload_start = forward_end.saturating_add(1);
    let reload_end = reload_start.saturating_add(width_u16(reload));

    AddressHitboxes {
        back: back_start..back_end,
        forward: forward_start..forward_end,
        reload: reload_start..reload_end,
    }
}

fn footer_hitboxes(app: &App, footer_rect: Rect) -> FooterHitboxes {
    let inner_start = footer_rect.x.saturating_add(1);

    let js_token = format!("JS[{}]", super::bool_label(app.page.js_default_enabled));
    let img_token = format!(
        "IMG[{}:{}]",
        super::image_mode_label(app.page.image_mode),
        glance_media::image_backend_label(app.page.image_backend)
    );

    let js_start = inner_start;
    let js_end = js_start.saturating_add(width_u16(&js_token));

    let img_start = js_end.saturating_add(2);
    let img_end = img_start.saturating_add(width_u16(&img_token));

    FooterHitboxes {
        js: js_start..js_end,
        img: img_start..img_end,
    }
}

fn width_u16(s: &str) -> u16 {
    UnicodeWidthStr::width(s) as u16
}
