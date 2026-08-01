use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};
use unicode_width::UnicodeWidthStr;

use glance_render::InteractiveAction;

use crate::app_state::{AddressMode, App, FocusPane, SidebarAction, UiLayout};
use crate::navigation::{
    build_visual_line_map, cursor_logical_line_from_visual, cursor_target_from_visual,
};
use crate::{UiImageMode, UiLiveFieldKind, UiPage, UiWebMode};

const SURFACE: Color = Color::Rgb(14, 18, 25);
const SURFACE_RAISED: Color = Color::Rgb(20, 25, 34);
const OUTLINE: Color = Color::Rgb(43, 50, 64);

pub(crate) fn draw_ui(frame: &mut Frame, app: &App) {
    let screen = frame.area();
    let layout = compute_layout(screen);
    let inner_width = layout.content.width.saturating_sub(2);
    let visual_lines = if inner_width == 0 {
        Vec::new()
    } else {
        build_visual_line_map(&app.page.rendered.lines, inner_width)
    };
    let cursor_line = cursor_logical_line_from_visual(&visual_lines, app.cursor_visual_row);
    let cursor_action = cursor_target_from_visual(
        &app.page.rendered.interactive_spans,
        &visual_lines,
        app.cursor_visual_row,
    )
    .map(|(_, span)| span.action);
    let max_row = visual_lines.len().saturating_sub(1);

    let progress = reading_progress(
        app.content_scroll as usize,
        visual_lines.len(),
        layout.content.height.saturating_sub(2) as usize,
    );

    draw_header(frame, app, layout.title);
    draw_address_bar(frame, app, layout.address);

    let content_title = if app.page.loading_more {
        format!(" {} LIVE FEED ", spinner(app.animation_tick))
    } else if app.page.can_load_more && progress == 100 {
        " 100% · SCROLL FOR MORE ".to_string()
    } else {
        format!(" {progress:>3}% ")
    };
    let content_block = chrome_block(content_title, app.focus == FocusPane::Content, Color::Cyan);
    let content = Paragraph::new(styled_content_lines(
        &app.page,
        cursor_line,
        &app.find_query,
        app.hovered_action,
    ))
    .style(Style::default().bg(SURFACE))
    .block(content_block)
    .scroll((app.content_scroll, 0))
    .wrap(Wrap { trim: false });
    frame.render_widget(content, layout.content);
    let viewport_len = layout.content.height.saturating_sub(2) as usize;
    if visual_lines.len() > viewport_len && viewport_len > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("┊"))
            .thumb_symbol("┃")
            .track_style(Style::default().fg(Color::DarkGray))
            .thumb_style(Style::default().fg(Color::Cyan));
        let mut scrollbar_state = ScrollbarState::new(visual_lines.len())
            .position(app.content_scroll as usize)
            .viewport_content_length(viewport_len);
        frame.render_stateful_widget(scrollbar, layout.content, &mut scrollbar_state);
    }

    if let Some(sidebar) = layout.sidebar {
        draw_sidebar(frame, app, sidebar, cursor_action, progress);
    }

    draw_footer(frame, app, layout.footer, cursor_action, max_row);
    draw_update_activity(frame, app, layout.content);

    if app.control_confirmation.is_some() {
        draw_control_confirmation(frame, app, screen);
    } else if app.live_choice_editor.is_some() {
        draw_live_choice_editor(frame, app, screen);
    } else if matches!(app.address_mode, AddressMode::LiveField(_)) {
        draw_live_text_editor(frame, app, screen);
    } else if app.help_open {
        draw_help_overlay(frame, screen);
    } else if app.media_picker_open {
        draw_media_picker(frame, app, screen);
    } else if app.image_viewer_open {
        draw_image_viewer_shell(frame, app, screen);
    }
    if app.control_confirmation.is_none()
        && !app.help_open
        && app
            .page
            .media_activity
            .as_ref()
            .is_some_and(|activity| activity.phase == crate::UiMediaPhase::Opening)
    {
        draw_media_activity(frame, app, screen);
    }
}

fn draw_update_activity(frame: &mut Frame, app: &App, content: Rect) {
    let running = if app.page.updating_live_controls {
        Some((
            format!("{}  Updating live controls", spinner(app.animation_tick)),
            Color::Yellow,
        ))
    } else if app.page.loading_more {
        Some((
            format!("{}  Loading live content", spinner(app.animation_tick)),
            Color::Cyan,
        ))
    } else if app.page.web_mode == UiWebMode::Upgrading {
        Some((
            format!("{}  Updating dynamic DOM", spinner(app.animation_tick)),
            Color::Magenta,
        ))
    } else {
        None
    };
    let completed = app.update_notice.as_ref().map(|notice| {
        (
            format!(
                "{}  {}",
                if notice.failed { "!" } else { "✓" },
                notice.message
            ),
            if notice.failed {
                Color::Red
            } else {
                Color::Green
            },
        )
    });
    let is_running = running.is_some();
    let Some((message, color)) = running.or(completed) else {
        return;
    };
    if content.width < 28 || content.height < 7 {
        return;
    }
    let width = (UnicodeWidthStr::width(message.as_str()) as u16 + 6)
        .min(content.width.saturating_sub(2))
        .max(28);
    let area = Rect::new(
        content.right().saturating_sub(width).saturating_sub(1),
        content.y.saturating_add(1),
        width,
        5,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                message,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                if is_running {
                    "━━━━━━━╺━━━━━━━━"
                } else {
                    "━━━━━━━━━━━━━━━━"
                },
                Style::default().fg(color),
            )),
            Line::from(Span::styled(
                if is_running {
                    "Interface remains responsive"
                } else {
                    "Ready"
                },
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .style(Style::default().bg(SURFACE_RAISED))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color)),
        ),
        area,
    );
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let mode_color = web_mode_color(app.page.web_mode);
    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled("●", Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(
            "GLANCE",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            app.page.rendered.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("●", Style::default().fg(mode_color)),
        Span::raw("  "),
        Span::styled(
            format!(
                "⌁ {}   ◆ {}   ▶ {}{}",
                app.page.links.len(),
                app.page.images.len(),
                app.page.media.len(),
                if app.page.auth.is_some() {
                    "   ♙"
                } else {
                    ""
                }
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line)
            .style(Style::default().bg(SURFACE_RAISED))
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(OUTLINE)),
            ),
        area,
    );
}

fn draw_address_bar(frame: &mut Frame, app: &App, area: Rect) {
    let accent = match app.address_mode {
        AddressMode::Nav => Color::Blue,
        AddressMode::Edit => Color::Cyan,
        AddressMode::Find => Color::Yellow,
        AddressMode::Form(_) | AddressMode::LiveField(_) => Color::Magenta,
    };
    let active = app.focus == FocusPane::Address || app.address_mode != AddressMode::Nav;
    let line = if app.address_mode != AddressMode::Nav {
        let (before, after) = split_at_char(&app.command, app.command_cursor);
        let prefix = match app.address_mode {
            AddressMode::Edit => "› ",
            AddressMode::Find => "/ ",
            AddressMode::Form(_) => "? ",
            AddressMode::LiveField(_) => "✎ ",
            AddressMode::Nav => "",
        };
        Line::from(vec![
            Span::styled(
                prefix,
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(before, Style::default().fg(Color::White)),
            Span::styled(
                "▏",
                Style::default()
                    .fg(Color::Black)
                    .bg(accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(after, Style::default().fg(Color::White)),
        ])
    } else {
        let mut spans = vec![
            nav_button("←", Color::Cyan),
            Span::raw(" "),
            nav_button("→", Color::Cyan),
            Span::raw(" "),
            nav_button("↻", Color::Green),
            Span::raw(" "),
            nav_button("⚡", Color::Magenta),
            Span::raw(" "),
            nav_button("↗", Color::Blue),
            Span::raw("  "),
        ];
        spans.extend(url_spans(&app.page.rendered.source_url));
        Line::from(spans)
    };
    frame.render_widget(
        Paragraph::new(line)
            .style(Style::default().bg(SURFACE_RAISED))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(if active { accent } else { OUTLINE })),
            ),
        area,
    );
}

fn draw_sidebar(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    cursor_action: Option<InteractiveAction>,
    progress: u16,
) {
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                page_origin(&app.page.rendered.source_url),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("●", Style::default().fg(web_mode_color(app.page.web_mode))),
        ]),
        Line::from(vec![Span::styled(
            format!(
                "⌁ {}   ◆ {}   ▶ {}",
                app.page.links.len(),
                app.page.images.len(),
                app.page.media.len()
            ),
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(vec![
            Span::styled(progress_bar(progress, 12), Style::default().fg(Color::Cyan)),
            Span::styled(format!(" {progress}%"), Style::default().fg(Color::Gray)),
            Span::raw("   "),
            Span::styled(
                cursor_action
                    .map(super::action_label)
                    .unwrap_or_else(|| "•".to_string()),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
    ];

    let preview = image_dock_preview_rect(app, area);
    let preview_space = preview.map_or(0, |rect| rect.height.saturating_add(1));
    let max_assets = area.height.saturating_sub(6).saturating_sub(preview_space) as usize;
    let assets = sidebar_assets(app, max_assets);
    if assets.is_empty() {
        lines.push(Line::from(Span::styled(
            "—",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (action, label) in assets {
            let hovered = app.hovered_sidebar_action == Some(action);
            let (glyph, color) = match action {
                SidebarAction::Auth => ("♙", Color::Cyan),
                SidebarAction::Image(_) => ("◇", Color::Magenta),
                SidebarAction::Media(index) => {
                    let glyph = app
                        .page
                        .media
                        .get(index)
                        .map(|media| media_glyph(media.kind))
                        .unwrap_or("▶");
                    (glyph, Color::Green)
                }
            };
            let style = if hovered {
                Style::default()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{glyph} "), Style::default().fg(color)),
                Span::styled(label, style),
            ]));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(SURFACE_RAISED))
            .block(chrome_block(" ◫ ".to_string(), false, Color::Cyan)),
        area,
    );
    if let Some(preview) = preview {
        let selected = app.page.selected_image_index.unwrap_or(0).saturating_add(1);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta))
                .title(Span::styled(
                    format!(" ◇ {selected}/{} ", app.page.images.len()),
                    Style::default().fg(Color::Magenta),
                )),
            preview,
        );
    }
}

pub(crate) fn image_dock_preview_rect(app: &App, sidebar: Rect) -> Option<Rect> {
    if app.page.selected_image_bytes.is_none()
        || app.page.image_mode == UiImageMode::Text
        || matches!(
            app.page.image_backend,
            glance_media::ImageBackend::TextFallback | glance_media::ImageBackend::Sixel
        )
        || sidebar.height < 14
        || sidebar.width < 12
    {
        return None;
    }
    let height = sidebar.height.saturating_sub(8).clamp(5, 9);
    Some(Rect::new(
        sidebar.x.saturating_add(1),
        sidebar.bottom().saturating_sub(height).saturating_sub(1),
        sidebar.width.saturating_sub(2),
        height,
    ))
}

fn web_mode_color(mode: UiWebMode) -> Color {
    match mode {
        UiWebMode::Static => Color::Yellow,
        UiWebMode::Upgrading => Color::Magenta,
        UiWebMode::Dynamic => Color::Green,
        UiWebMode::Session => Color::Cyan,
    }
}

fn media_glyph(kind: crate::UiMediaKind) -> &'static str {
    match kind {
        crate::UiMediaKind::Audio => "♪",
        crate::UiMediaKind::Video => "▶",
        crate::UiMediaKind::Stream => "◉",
        crate::UiMediaKind::Media => "◆",
    }
}

fn action_chip(label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

fn nav_button(label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(color)
            .bg(Color::Rgb(36, 42, 54))
            .add_modifier(Modifier::BOLD),
    )
}

fn section_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn chrome_block(title: String, active: bool, accent: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if active { accent } else { Color::DarkGray }))
        .style(Style::default().bg(if active { SURFACE_RAISED } else { SURFACE }))
        .title(Span::styled(
            title,
            Style::default()
                .fg(if active { accent } else { Color::DarkGray })
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ))
}

fn url_spans(raw: &str) -> Vec<Span<'static>> {
    let Ok(url) = url::Url::parse(raw) else {
        return vec![Span::styled(
            if raw.is_empty() { "about:blank" } else { raw }.to_string(),
            Style::default().fg(Color::Gray),
        )];
    };
    let color = match url.scheme() {
        "https" => Color::Green,
        "http" => Color::Yellow,
        _ => Color::Blue,
    };
    let host = url.host_str().unwrap_or(url.scheme()).to_string();
    let mut tail = url.path().to_string();
    if let Some(query) = url.query() {
        tail.push('?');
        tail.push_str(query);
    }
    if tail == "/" {
        tail.clear();
    }
    vec![
        Span::styled("●", Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            host,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(tail, Style::default().fg(Color::Gray)),
    ]
}

fn page_origin(raw: &str) -> String {
    url::Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(ToString::to_string))
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "local page".to_string())
}

pub(crate) fn reading_progress(scroll: usize, total: usize, viewport: usize) -> u16 {
    if total == 0 || total <= viewport {
        return 100;
    }
    scroll
        .saturating_add(viewport)
        .saturating_mul(100)
        .saturating_div(total)
        .min(100) as u16
}

fn progress_bar(progress: u16, width: usize) -> String {
    let filled = (progress as usize)
        .saturating_mul(width)
        .saturating_div(100)
        .min(width);
    format!("{}{}", "━".repeat(filled), "┄".repeat(width - filled))
}

fn sidebar_assets(app: &App, max_items: usize) -> Vec<(SidebarAction, String)> {
    app.page
        .auth
        .iter()
        .map(|auth| (SidebarAction::Auth, format!("{}  ◉ SECURE", auth.label)))
        .chain(
            app.page
                .media
                .iter()
                .enumerate()
                .map(|(index, media)| {
                    (
                        SidebarAction::Media(index),
                        format!("{:02}  {}", index + 1, media.label),
                    )
                })
                .chain(app.page.images.iter().enumerate().map(|(index, image)| {
                    (
                        SidebarAction::Image(index),
                        format!("{:02}  {}", index + 1, image.label),
                    )
                })),
        )
        .take(max_items)
        .collect()
}

pub(crate) fn sidebar_action_at(app: &App, area: Rect, row: u16) -> Option<SidebarAction> {
    let first_asset_row = area.y.saturating_add(1).saturating_add(4);
    let relative = row.checked_sub(first_asset_row)? as usize;
    let max_items = area.height.saturating_sub(6) as usize;
    sidebar_assets(app, max_items)
        .get(relative)
        .map(|(action, _)| *action)
}

fn help_section(label: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<10}"), section_style()),
        Span::styled(description.to_string(), Style::default().fg(Color::Gray)),
    ])
}

pub(crate) fn help_rect(area: Rect) -> Rect {
    let width = (area.width.saturating_mul(82) / 100)
        .clamp(36, 96)
        .min(area.width);
    let height = 15_u16.min(area.height).max(8.min(area.height));
    Rect::new(
        area.x
            .saturating_add(area.width.saturating_sub(width).saturating_div(2)),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_div(2)),
        width,
        height,
    )
}

fn draw_footer(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    cursor_action: Option<InteractiveAction>,
    max_row: usize,
) {
    let web_token = format!(" ●{} ", app.page.web_mode.label());
    let media_token = app
        .page
        .media_activity
        .as_ref()
        .map(|activity| match activity.phase {
            crate::UiMediaPhase::Opening => format!(" {} ", spinner(app.animation_tick)),
            crate::UiMediaPhase::Active => " ■ ".to_string(),
        })
        .unwrap_or_else(|| format!(" ▶{} ", app.page.media.len()));
    let mut controls = vec![
        flat_control(web_token, web_mode_color(app.page.web_mode)),
        Span::raw(" "),
        flat_control(" ↗ ".to_string(), Color::Blue),
        Span::raw(" "),
    ];
    if app.page.auth.is_some() {
        controls.push(flat_control(" ♙ Sign in ".to_string(), Color::Cyan));
        controls.push(Span::raw(" "));
    }
    controls.extend([
        flat_control(
            media_token,
            if app.page.media.is_empty() {
                Color::DarkGray
            } else {
                Color::Green
            },
        ),
        Span::raw(" "),
        flat_control(" ◇ ".to_string(), Color::Magenta),
        Span::raw(" "),
        flat_control(" ? ".to_string(), Color::Yellow),
        Span::styled("  │  ", Style::default().fg(OUTLINE)),
        Span::styled(
            app.effective_status(),
            Style::default().fg(if app.page.media_activity.is_some() {
                Color::Green
            } else if app.page.loading_more {
                Color::Cyan
            } else {
                Color::Gray
            }),
        ),
        Span::styled("  │  ", Style::default().fg(OUTLINE)),
        Span::styled(
            format!(
                "{}/{} {}",
                app.cursor_visual_row.saturating_add(1),
                max_row.saturating_add(1),
                cursor_action
                    .map(super::action_label)
                    .unwrap_or_else(|| "•".to_string())
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let status_line = Line::from(controls);
    frame.render_widget(
        Paragraph::new(status_line)
            .style(Style::default().bg(SURFACE_RAISED))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(OUTLINE)),
            ),
        area,
    );
}

fn flat_control(label: String, color: Color) -> Span<'static> {
    Span::styled(
        label,
        Style::default()
            .fg(color)
            .bg(SURFACE_RAISED)
            .add_modifier(Modifier::BOLD),
    )
}

fn draw_media_activity(frame: &mut Frame, app: &App, screen: Rect) {
    let Some(activity) = &app.page.media_activity else {
        return;
    };
    let width = 44_u16.min(screen.width.saturating_sub(2)).max(1);
    let height = 3_u16.min(screen.height).max(1);
    let area = Rect::new(
        screen.right().saturating_sub(width).saturating_sub(1),
        screen.bottom().saturating_sub(height).saturating_sub(2),
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let line = Line::from(vec![
        Span::styled(
            spinner(app.animation_tick),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            media_glyph(activity.kind),
            Style::default().fg(Color::Green),
        ),
        Span::raw("  "),
        Span::styled("Opening", Style::default().fg(Color::Gray)),
        Span::raw("  "),
        Span::styled(
            activity.label.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .style(Style::default().bg(SURFACE_RAISED))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green)),
        ),
        area,
    );
}

fn spinner(tick: u64) -> &'static str {
    const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];
    FRAMES[(tick as usize) % FRAMES.len()]
}

fn draw_image_viewer_shell(frame: &mut Frame, app: &App, screen: Rect) {
    let viewer = viewer_rect(screen);
    frame.render_widget(Clear, viewer);
    let selected = app
        .page
        .selected_image_index
        .and_then(|index| app.page.images.get(index));
    let title = app
        .page
        .selected_image_index
        .map(|index| format!(" ◇  {}/{} ", index + 1, app.page.images.len()))
        .unwrap_or_else(|| " ◇ ".to_string());
    frame.render_widget(
        Block::default()
            .style(Style::default().bg(SURFACE_RAISED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )),
        viewer,
    );
    let details = Rect::new(
        viewer.x.saturating_add(2),
        viewer.y.saturating_add(1),
        viewer.width.saturating_sub(4),
        2,
    );
    let mut detail_lines = Vec::new();
    if let Some(image) = selected {
        detail_lines.push(Line::from(Span::styled(
            image.label.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));
        let size = app
            .page
            .selected_image_size
            .map(|(w, h)| format!("{w}×{h}  ·  "))
            .unwrap_or_default();
        detail_lines.push(Line::from(vec![
            Span::styled(size, Style::default().fg(Color::DarkGray)),
            Span::styled(
                image.url.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ]));
    } else {
        detail_lines.push(Line::from(Span::styled(
            "No image selected",
            Style::default().fg(Color::DarkGray),
        )));
    }
    frame.render_widget(
        Paragraph::new(detail_lines).wrap(Wrap { trim: true }),
        details,
    );

    if matches!(
        app.page.image_backend,
        glance_media::ImageBackend::TextFallback
    ) {
        let message = Rect::new(
            viewer.x.saturating_add(3),
            viewer.y.saturating_add(viewer.height / 2),
            viewer.width.saturating_sub(6),
            2,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Inline graphics are unavailable in this terminal · URL remains copyable",
                Style::default().fg(Color::Yellow),
            ))),
            message,
        );
    }

    let controls = image_viewer_controls_rect(viewer);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            action_chip("←", Color::Cyan),
            Span::raw("  "),
            action_chip("→", Color::Cyan),
            Span::raw("  "),
            action_chip("Copy", Color::Yellow),
            Span::raw("  "),
            action_chip("×", Color::Red),
        ])),
        controls,
    );
}

fn draw_help_overlay(frame: &mut Frame, screen: Rect) {
    let area = help_rect(screen);
    frame.render_widget(Clear, area);
    let lines = vec![
        help_section(
            "NAVIGATE",
            "o / Ctrl-L  address or search     b / f  back / forward     r  reload",
        ),
        help_section(
            "DYNAMIC",
            "d  Chromium DOM upgrade     click ⚡ or the colored status dot",
        ),
        help_section(
            "READ",
            "j/k or arrows  move     Space/PgDn  page & extend feed     g/G  top/bottom",
        ),
        help_section(
            "ACT",
            "Enter  open / continue     a  secure sign-in     e  GUI     y  copy URL",
        ),
        help_section(
            "MEDIA",
            "m  library     i  image mode     Page Dock: click preview · middle-click full",
        ),
        help_section(
            "MOUSE",
            "hover previews · release activates · drag scroll · right-click copies",
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("TIP  ", section_style()),
            Span::styled(
                "Use :login, :more, :browser, :link N, :media N or :dynamic.",
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Press ? or Esc to close",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .style(Style::default().bg(SURFACE_RAISED))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        " ? ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub(crate) fn live_text_editor_rect(area: Rect, multiline: bool) -> Rect {
    let width = (area.width.saturating_mul(82) / 100)
        .clamp(38, 88)
        .min(area.width);
    let desired_height = if multiline { 16 } else { 10 };
    let height = desired_height.min(area.height).max(8.min(area.height));
    Rect::new(
        area.x
            .saturating_add(area.width.saturating_sub(width).saturating_div(2)),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_div(2)),
        width,
        height,
    )
}

pub(crate) fn live_text_editor_buttons(area: Rect, multiline: bool) -> (Rect, Rect) {
    let editor = live_text_editor_rect(area, multiline);
    let width = 19_u16.min(editor.width.saturating_sub(6).saturating_div(2));
    let row = editor.bottom().saturating_sub(3);
    (
        Rect::new(editor.x.saturating_add(3), row, width, 1),
        Rect::new(
            editor.right().saturating_sub(width).saturating_sub(3),
            row,
            width,
            1,
        ),
    )
}

fn draw_live_text_editor(frame: &mut Frame, app: &App, screen: Rect) {
    let AddressMode::LiveField(index) = app.address_mode else {
        return;
    };
    let Some(control) = app.page.live_controls.get(index) else {
        return;
    };
    let multiline = control.field_kind == Some(UiLiveFieldKind::TextArea);
    let area = live_text_editor_rect(screen, multiline);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .style(Style::default().bg(SURFACE_RAISED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                format!(" ✎  {} ", control.label),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )),
        area,
    );
    let input_area = Rect::new(
        area.x.saturating_add(3),
        area.y.saturating_add(2),
        area.width.saturating_sub(6),
        area.height.saturating_sub(7),
    );
    frame.render_widget(
        Block::default()
            .style(Style::default().bg(SURFACE))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(74, 52, 90))),
        input_area,
    );
    let body = input_area.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let content = if app.command.is_empty() {
        vec![Line::from(vec![
            Span::styled("▏", Style::default().fg(Color::Magenta)),
            Span::styled(
                control.placeholder.clone().unwrap_or_default(),
                Style::default().fg(Color::DarkGray),
            ),
        ])]
    } else {
        live_editor_lines(&app.command, app.command_cursor)
    };
    let editor_scroll = live_editor_scroll(
        &app.command,
        app.command_cursor,
        body.width as usize,
        body.height as usize,
    );
    frame.render_widget(
        Paragraph::new(content)
            .style(Style::default().bg(SURFACE))
            .scroll((editor_scroll, 0))
            .wrap(Wrap { trim: false }),
        body,
    );
    let hint = if multiline {
        "Enter newline  ·  Ctrl-Enter apply"
    } else {
        "Enter apply"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))),
        Rect::new(
            area.x.saturating_add(3),
            area.bottom().saturating_sub(5),
            area.width.saturating_sub(6),
            1,
        ),
    );
    let (cancel, apply) = live_text_editor_buttons(screen, multiline);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "  Cancel  Esc ",
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )),
        cancel,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            if multiline {
                " Apply  Ctrl-Enter "
            } else {
                "  Apply  Enter  "
            },
            Style::default()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )),
        apply,
    );
}

fn live_editor_lines(value: &str, cursor: usize) -> Vec<Line<'static>> {
    let (before, _) = split_at_char(value, cursor);
    let cursor_line = before.split('\n').count().saturating_sub(1);
    let cursor_column = before
        .rsplit('\n')
        .next()
        .map(|line| line.chars().count())
        .unwrap_or(0);
    value
        .split('\n')
        .enumerate()
        .map(|(line_index, line)| {
            if line_index != cursor_line {
                return Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::White),
                ));
            }
            let (left, right) = split_at_char(line, cursor_column);
            Line::from(vec![
                Span::styled(left, Style::default().fg(Color::White)),
                Span::styled(
                    "▏",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(right, Style::default().fg(Color::White)),
            ])
        })
        .collect()
}

pub(crate) fn live_editor_scroll(value: &str, cursor: usize, width: usize, height: usize) -> u16 {
    if width == 0 || height == 0 {
        return 0;
    }
    let (before, _) = split_at_char(value, cursor);
    let mut cursor_row = 0_usize;
    let mut lines = before.split('\n').peekable();
    while let Some(line) = lines.next() {
        let line_width = UnicodeWidthStr::width(line);
        if lines.peek().is_some() {
            cursor_row = cursor_row.saturating_add(line_width.max(1).div_ceil(width));
        } else {
            cursor_row = cursor_row.saturating_add(line_width / width);
        }
    }
    cursor_row
        .saturating_sub(height.saturating_sub(1))
        .min(u16::MAX as usize) as u16
}

pub(crate) fn live_choice_rect(area: Rect) -> Rect {
    let width = (area.width.saturating_mul(64) / 100)
        .clamp(34, 68)
        .min(area.width);
    let height = (area.height.saturating_mul(70) / 100)
        .clamp(10, 20)
        .min(area.height);
    Rect::new(
        area.x
            .saturating_add(area.width.saturating_sub(width).saturating_div(2)),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_div(2)),
        width,
        height,
    )
}

pub(crate) fn live_choice_options_rect(area: Rect) -> Rect {
    let editor = live_choice_rect(area);
    Rect::new(
        editor.x.saturating_add(2),
        editor.y.saturating_add(2),
        editor.width.saturating_sub(4),
        editor.height.saturating_sub(5),
    )
}

pub(crate) fn live_choice_window_start(selected: usize, len: usize, visible_rows: usize) -> usize {
    if visible_rows == 0 || len <= visible_rows {
        return 0;
    }
    selected
        .saturating_sub(visible_rows.saturating_sub(1) / 2)
        .min(len.saturating_sub(visible_rows))
}

fn draw_live_choice_editor(frame: &mut Frame, app: &App, screen: Rect) {
    let Some(editor) = &app.live_choice_editor else {
        return;
    };
    let Some(control) = app.page.live_controls.get(editor.control_index) else {
        return;
    };
    let area = live_choice_rect(screen);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .style(Style::default().bg(SURFACE_RAISED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                format!(" ▾  {} ", control.label),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
        area,
    );
    let options_area = live_choice_options_rect(screen);
    let visible_rows = options_area.height as usize;
    let window_start =
        live_choice_window_start(editor.selected_index, control.options.len(), visible_rows);
    let options = control
        .options
        .iter()
        .enumerate()
        .skip(window_start)
        .take(visible_rows)
        .map(|(index, option)| {
            let selected = index == editor.selected_index;
            let marker = if option.selected { "●" } else { "○" };
            let style = if option.disabled {
                Style::default().fg(Color::DarkGray)
            } else if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(
                format!(" {marker}  {}", option.label),
                style,
            )))
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(options), options_area);
    let position = if control.options.is_empty() {
        "0/0".to_string()
    } else {
        format!(
            "{}/{}",
            editor.selected_index.saturating_add(1),
            control.options.len()
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓ ", Style::default().fg(Color::Black).bg(Color::Cyan)),
            Span::styled(" choose   ", Style::default().fg(Color::Gray)),
            Span::styled(" Enter ", Style::default().fg(Color::Black).bg(Color::Cyan)),
            Span::styled(
                format!(" apply   Esc cancel   {position}"),
                Style::default().fg(Color::Gray),
            ),
        ])),
        Rect::new(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(2),
            area.width.saturating_sub(4),
            1,
        ),
    );
}

pub(crate) fn control_confirmation_rect(area: Rect) -> Rect {
    let width = 64_u16.min(area.width.saturating_sub(2)).max(1);
    let height = 10_u16.min(area.height).max(1);
    Rect::new(
        area.x
            .saturating_add(area.width.saturating_sub(width).saturating_div(2)),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_div(2)),
        width,
        height,
    )
}

pub(crate) fn control_confirmation_buttons(area: Rect) -> (Rect, Rect) {
    let dialog = control_confirmation_rect(area);
    let button_width = 17_u16.min(dialog.width.saturating_sub(4).saturating_div(2));
    let row = dialog.bottom().saturating_sub(3);
    let cancel = Rect::new(dialog.x.saturating_add(3), row, button_width, 1);
    let confirm = Rect::new(
        dialog
            .right()
            .saturating_sub(button_width)
            .saturating_sub(3),
        row,
        button_width,
        1,
    );
    (cancel, confirm)
}

fn draw_control_confirmation(frame: &mut Frame, app: &App, screen: Rect) {
    let Some(confirmation) = &app.control_confirmation else {
        return;
    };
    let area = control_confirmation_rect(screen);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .style(Style::default().bg(SURFACE_RAISED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow))
            .title(Span::styled(
                " Confirm live action ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
        area,
    );
    let context = confirmation
        .context
        .as_deref()
        .map(|value| format!("Context  {value}"))
        .unwrap_or_else(|| format!("Site  {}", page_origin(&app.page.rendered.source_url)));
    let body = Rect::new(
        area.x.saturating_add(3),
        area.y.saturating_add(2),
        area.width.saturating_sub(6),
        area.height.saturating_sub(6),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                confirmation.label.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(context, Style::default().fg(Color::Gray))),
            Line::from(""),
            Line::from(Span::styled(
                "This may publish content or change your account state.",
                Style::default().fg(Color::Yellow),
            )),
        ])
        .wrap(Wrap { trim: true }),
        body,
    );
    let (cancel, confirm) = control_confirmation_buttons(screen);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Cancel  Esc ",
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ))),
        cancel,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Confirm  Enter ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))),
        confirm,
    );
}

fn draw_media_picker(frame: &mut Frame, app: &App, screen: Rect) {
    let picker = media_picker_rect(screen);
    let layout = media_picker_layout(picker);
    frame.render_widget(Clear, picker);
    frame.render_widget(
        Block::default()
            .style(Style::default().bg(SURFACE_RAISED))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Green))
            .title(Span::styled(
                format!(" ▶  {} ", app.page.media.len()),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
        picker,
    );

    let start = media_window_start(
        app.selected_media_index,
        app.page.media.len(),
        layout.list.height as usize,
    );
    let items = app
        .page
        .media
        .iter()
        .enumerate()
        .skip(start)
        .take(layout.list.height as usize)
        .map(|(index, media)| {
            let hovered = app.hovered_media_index == Some(index);
            let selected = app.selected_media_index == index;
            let activity = app
                .page
                .media_activity
                .as_ref()
                .filter(|activity| activity.url == media.url);
            let color = match media.kind {
                crate::UiMediaKind::Audio => Color::Magenta,
                crate::UiMediaKind::Video => Color::Green,
                crate::UiMediaKind::Stream => Color::Yellow,
                crate::UiMediaKind::Media => Color::Blue,
            };
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if hovered {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    match activity.map(|activity| activity.phase) {
                        Some(crate::UiMediaPhase::Opening) => spinner(app.animation_tick),
                        Some(crate::UiMediaPhase::Active) => "■",
                        None => media_glyph(media.kind),
                    }
                    .to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{:02}", index + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("  "),
                Span::raw(media.label.clone()),
            ]))
            .style(style)
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(items), layout.list);

    if let Some(media) = app.page.media.get(app.selected_media_index) {
        let mime = media.mime_type.as_deref().unwrap_or("unknown MIME");
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!("{}  {mime}", media_glyph(media.kind)),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    media.url.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED),
                )),
            ])
            .wrap(Wrap { trim: true }),
            layout.details,
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            action_chip("▶ Play", Color::Green),
            Span::raw("  "),
            action_chip("←", Color::Cyan),
            Span::raw("  "),
            action_chip("→", Color::Cyan),
            Span::raw("  "),
            action_chip("Copy", Color::Yellow),
            Span::raw("  "),
            action_chip("×", Color::Red),
        ])),
        layout.controls,
    );
}

pub(crate) fn styled_content_lines(
    page: &UiPage,
    cursor_line: Option<usize>,
    find_query: &str,
    hovered_action: Option<InteractiveAction>,
) -> Vec<Line<'static>> {
    let lines = &page.rendered.lines;
    let mut styled = Vec::with_capacity(lines.len());
    let mut in_code = false;
    let mut actions_by_line: std::collections::HashMap<usize, Vec<InteractiveAction>> =
        std::collections::HashMap::new();
    for span in &page.rendered.interactive_spans {
        actions_by_line
            .entry(span.line)
            .or_default()
            .push(span.action);
    }

    for (line_idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if !find_query.is_empty() && line.to_lowercase().contains(&find_query.to_lowercase()) {
            styled.push(Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        if trimmed == "```" {
            in_code = !in_code;
            styled.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        if in_code {
            styled.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            )));
            continue;
        }

        if is_hr_line(trimmed) {
            styled.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        if trimmed.starts_with("╭─") || trimmed == "╰─" {
            styled.push(Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(if trimmed.contains("ACTIVE") {
                        Color::Magenta
                    } else {
                        Color::Cyan
                    })
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        if trimmed.starts_with('#') {
            let heading = line.trim_start();
            let indent_len = line.len().saturating_sub(heading.len());
            let marker_len = heading.chars().take_while(|c| *c == '#').count();
            let (markers, title) = heading.split_at(marker_len);
            styled.push(Line::from(vec![
                Span::raw(line[..indent_len].to_string()),
                Span::styled(
                    markers.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    title.to_string(),
                    Style::default()
                        .fg(if marker_len == 1 {
                            Color::Yellow
                        } else {
                            Color::White
                        })
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }

        if let Some((label, value)) = metadata_parts(line) {
            let color = if label == "Dynamic DOM:" {
                Color::Green
            } else {
                Color::DarkGray
            };
            styled.push(Line::from(vec![
                Span::styled(
                    label.to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(value.to_string(), Style::default().fg(Color::Gray)),
            ]));
            continue;
        }

        if let Some(actions) = actions_by_line.get(&line_idx) {
            if actions.len() == 1 {
                if let Some((token, label)) = line.split_once(' ') {
                    let (token_style, label_style) = interactive_styles(
                        actions[0],
                        cursor_line == Some(line_idx),
                        hovered_action == Some(actions[0]),
                        live_action_accent(page, actions[0]),
                    );
                    styled.push(Line::from(vec![
                        Span::styled(token.to_string(), token_style),
                        Span::raw(" "),
                        Span::styled(label.to_string(), label_style),
                    ]));
                    continue;
                }
            } else {
                let hovered = actions.iter().any(|action| Some(*action) == hovered_action);
                let style = if cursor_line == Some(line_idx) || hovered {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED)
                };
                styled.push(Line::from(Span::styled(line.to_string(), style)));
                continue;
            }
        }

        if let Some(rest) = trimmed.strip_prefix("│ ") {
            styled.push(Line::from(vec![
                Span::styled(
                    "│ ".to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(rest.to_string(), Style::default().fg(Color::White)),
            ]));
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("• ") {
            styled.push(Line::from(vec![
                Span::styled(
                    "• ".to_string(),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(rest.to_string(), Style::default().fg(Color::White)),
            ]));
            continue;
        }

        if let Some((marker, rest)) = split_numbered_marker(trimmed) {
            styled.push(Line::from(vec![
                Span::styled(
                    marker.to_string(),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(rest.to_string(), Style::default().fg(Color::White)),
            ]));
            continue;
        }

        if trimmed.starts_with('|') {
            styled.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Green),
            )));
            continue;
        }

        if trimmed.starts_with("↗ ") {
            if let Some(url_idx) = trimmed.find("http://").or_else(|| trimmed.find("https://")) {
                let (left, right) = trimmed.split_at(url_idx);
                styled.push(Line::from(vec![
                    Span::styled(
                        left.to_string(),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        right.to_string(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::UNDERLINED),
                    ),
                ]));
            } else {
                styled.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            continue;
        }

        let mut style = Style::default().fg(Color::White);
        if cursor_line == Some(line_idx) {
            style = style.bg(Color::DarkGray);
        }
        styled.push(Line::from(Span::styled(line.to_string(), style)));
    }

    styled
}

pub(crate) fn interactive_styles(
    action: InteractiveAction,
    selected: bool,
    hovered: bool,
    live_accent: Option<Color>,
) -> (Style, Style) {
    let (token_style, label_style) = match action {
        InteractiveAction::OpenLink(_) => (
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED),
        ),
        InteractiveAction::OpenImage(_) => (
            Style::default()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        InteractiveAction::OpenMedia(_) => (
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::Green),
        ),
        InteractiveAction::OpenForm(_) => (
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::UNDERLINED),
        ),
        InteractiveAction::ActivateButton(_) => (
            Style::default()
                .fg(Color::Black)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        InteractiveAction::EditLiveField(_) => (
            Style::default()
                .fg(Color::Black)
                .bg(live_accent.unwrap_or(Color::Magenta))
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(live_accent.unwrap_or(Color::Magenta))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        InteractiveAction::ActivateLiveControl(_) => (
            Style::default()
                .fg(Color::Black)
                .bg(live_accent.unwrap_or(Color::Yellow))
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(live_accent.unwrap_or(Color::Yellow))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    };

    if selected {
        (
            token_style.add_modifier(Modifier::REVERSED),
            label_style.bg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )
    } else if hovered {
        (
            token_style.add_modifier(Modifier::REVERSED),
            label_style
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
    } else {
        (token_style, label_style)
    }
}

fn live_action_accent(page: &UiPage, action: InteractiveAction) -> Option<Color> {
    let (index, field) = match action {
        InteractiveAction::EditLiveField(index) => (index, true),
        InteractiveAction::ActivateLiveControl(index) => (index, false),
        _ => return None,
    };
    let control = page.live_controls.get(index)?;
    if field {
        return Some(match control.field_kind {
            Some(UiLiveFieldKind::Checkbox | UiLiveFieldKind::Radio) => Color::Green,
            Some(UiLiveFieldKind::Select) => Color::Cyan,
            _ => Color::Magenta,
        });
    }
    Some(match control.kind {
        crate::UiLiveControlKind::Disclosure => Color::Cyan,
        crate::UiLiveControlKind::Submit => Color::Green,
        crate::UiLiveControlKind::Action => Color::Yellow,
        crate::UiLiveControlKind::Field => Color::Magenta,
    })
}

fn split_numbered_marker(input: &str) -> Option<(&str, &str)> {
    let dot = input.find('.')?;
    if dot == 0 {
        return None;
    }
    let (left, right) = input.split_at(dot + 1);
    if !left[..left.len().saturating_sub(1)]
        .chars()
        .all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let rest = right.trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((left, rest))
}

fn is_hr_line(input: &str) -> bool {
    input.len() >= 10 && input.chars().all(|c| c == '─')
}

fn metadata_parts(line: &str) -> Option<(&str, &str)> {
    const LABELS: [&str; 6] = [
        "Requested URL:",
        "Final URL:",
        "HTTP status:",
        "Content-Type:",
        "Redirects followed:",
        "Dynamic DOM:",
    ];
    LABELS
        .iter()
        .find_map(|label| line.strip_prefix(label).map(|value| (*label, value)))
}

fn split_at_char(input: &str, char_index: usize) -> (String, String) {
    let byte_index = input
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(input.len());
    (
        input[..byte_index].to_string(),
        input[byte_index..].to_string(),
    )
}

pub(crate) fn compute_layout(area: Rect) -> UiLayout {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(area);

    let (content, sidebar) = if area.width >= 112 && root[2].height >= 10 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(56), Constraint::Length(36)])
            .split(root[2]);
        (columns[0], Some(columns[1]))
    } else {
        (root[2], None)
    };

    UiLayout {
        title: root[0],
        address: root[1],
        content,
        sidebar,
        footer: root[3],
    }
}

pub(crate) fn viewer_rect(area: Rect) -> Rect {
    let width = (area.width.saturating_mul(80)).saturating_div(100).max(20);
    let height = (area.height.saturating_mul(75)).saturating_div(100).max(10);
    let x = area
        .x
        .saturating_add(area.width.saturating_sub(width).saturating_div(2));
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(height).saturating_div(2));
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

pub(crate) fn image_viewer_controls_rect(viewer: Rect) -> Rect {
    Rect::new(
        viewer.x.saturating_add(2),
        viewer.bottom().saturating_sub(2),
        viewer.width.saturating_sub(4),
        1,
    )
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MediaPickerLayout {
    pub(crate) list: Rect,
    pub(crate) details: Rect,
    pub(crate) controls: Rect,
}

pub(crate) fn media_picker_rect(area: Rect) -> Rect {
    let width = (area.width.saturating_mul(85)).saturating_div(100).max(30);
    let height = (area.height.saturating_mul(75)).saturating_div(100).max(12);
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x
            .saturating_add(area.width.saturating_sub(width).saturating_div(2)),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_div(2)),
        width,
        height,
    )
}

pub(crate) fn media_picker_layout(picker: Rect) -> MediaPickerLayout {
    let inner = picker.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);
    if rows[0].width >= 64 && rows[0].height >= 6 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(58),
                Constraint::Length(2),
                Constraint::Min(24),
            ])
            .split(rows[0]);
        return MediaPickerLayout {
            list: columns[0],
            details: columns[2],
            controls: rows[1],
        };
    }

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(rows[0]);
    MediaPickerLayout {
        list: body[0],
        details: body[1],
        controls: rows[1],
    }
}

pub(crate) fn media_window_start(selected: usize, len: usize, visible_rows: usize) -> usize {
    if len <= visible_rows || visible_rows == 0 {
        return 0;
    }
    selected
        .saturating_sub(visible_rows / 2)
        .min(len.saturating_sub(visible_rows))
}

pub(crate) fn one_line_label(raw: &str) -> String {
    let compact = raw.replace(['\n', '\r', '\t'], " ");
    let trimmed = compact.split_whitespace().collect::<Vec<_>>().join(" ");
    let max_chars = 120usize;
    if trimmed.chars().count() <= max_chars {
        trimmed
    } else {
        let mut out = trimmed
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        out.push('…');
        out
    }
}

pub(crate) fn point_in_rect(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}
