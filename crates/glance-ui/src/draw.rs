use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use glance_media::image_backend_label;
use glance_render::InteractiveAction;

use crate::app_state::{AddressMode, App, FocusPane, UiLayout};
use crate::navigation::{
    build_visual_line_map, cursor_logical_line_from_visual, cursor_target_from_visual,
};
use crate::{UiImageMode, UiPage};

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

    let page_block = pane_block("Page", false, Color::DarkGray);
    let header = Paragraph::new(app.page.rendered.title.clone()).block(page_block);
    frame.render_widget(header, layout.title);

    let address_block = pane_block("Address", app.focus == FocusPane::Address, Color::Blue);
    let address_line = if app.address_mode == AddressMode::Edit {
        Line::from(vec![
            Span::styled(
                ":",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(app.command.as_str(), Style::default().fg(Color::White)),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                "[]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "[]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "[]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                app.page.rendered.source_url.as_str(),
                Style::default().fg(Color::White),
            ),
        ])
    };
    frame.render_widget(
        Paragraph::new(address_line).block(address_block),
        layout.address,
    );

    let content_block = pane_block("Content", app.focus == FocusPane::Content, Color::Green);
    let content = Paragraph::new(styled_content_lines(&app.page, cursor_line))
        .block(content_block)
        .scroll((app.content_scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(content, layout.content);

    let status_line = Line::from(vec![
        Span::styled(
            format!("JS[{}]", bool_label(app.page.js_default_enabled)),
            Style::default()
                .fg(if app.page.js_default_enabled {
                    Color::Green
                } else {
                    Color::Red
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "IMG[{}:{}]",
                image_mode_label(app.page.image_mode),
                image_backend_label(app.page.image_backend)
            ),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            if app.image_viewer_open {
                "VIEWER[ON]"
            } else {
                "VIEWER[OFF]"
            },
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "CUR[{}/{}]",
                app.cursor_visual_row.saturating_add(1),
                max_row.saturating_add(1)
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "ACT[{}]",
                cursor_action
                    .map(super::action_label)
                    .unwrap_or_else(|| "-".to_string())
            ),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  |  "),
        Span::styled(app.effective_status(), Style::default().fg(Color::White)),
    ]);

    let hint_line = contextual_hints(app);

    let footer = Paragraph::new(vec![status_line, hint_line]).block(
        Block::default().borders(Borders::ALL).title(Span::styled(
            "Status",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    );
    frame.render_widget(footer, layout.footer);

    if app.image_viewer_open {
        let viewer = viewer_rect(screen);
        frame.render_widget(Clear, viewer);
        let viewer_title = match app.page.selected_image_index {
            Some(i) => format!("Image Viewer [{}]", i + 1),
            None => "Image Viewer".to_string(),
        };
        let viewer_widget = Paragraph::new("").block(
            Block::default().borders(Borders::ALL).title(Span::styled(
                viewer_title,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
        );
        frame.render_widget(viewer_widget, viewer);
    }
}

pub(crate) fn pane_block<'a>(title: &'a str, active: bool, active_color: Color) -> Block<'a> {
    if active {
        Block::default().borders(Borders::ALL).title(Span::styled(
            format!("● {title}"),
            Style::default()
                .fg(active_color)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(title, Style::default().fg(Color::DarkGray)))
    }
}

pub(crate) fn styled_content_lines(
    page: &UiPage,
    cursor_line: Option<usize>,
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

        if trimmed.starts_with('#') {
            styled.push(Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        if let Some(actions) = actions_by_line.get(&line_idx) {
            if actions.len() == 1 {
                if let Some((token, label)) = line.split_once(' ') {
                    let (token_style, label_style) =
                        interactive_styles(actions[0], cursor_line == Some(line_idx));
                    styled.push(Line::from(vec![
                        Span::styled(token.to_string(), token_style),
                        Span::raw(" "),
                        Span::styled(label.to_string(), label_style),
                    ]));
                    continue;
                }
            } else {
                let style = if cursor_line == Some(line_idx) {
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

pub(crate) fn interactive_styles(action: InteractiveAction, selected: bool) -> (Style, Style) {
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
    };

    if selected {
        (
            token_style.add_modifier(Modifier::REVERSED),
            label_style.bg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )
    } else {
        (token_style, label_style)
    }
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

pub(crate) fn contextual_hints(app: &App) -> Line<'static> {
    if app.image_viewer_open {
        return Line::from(vec![
            hotkey("Esc/Enter/Space"),
            Span::raw(": close viewer  "),
            hotkey("click"),
            Span::raw(": close viewer"),
        ]);
    }

    if app.address_mode == AddressMode::Edit {
        return Line::from(vec![
            hotkey("Enter"),
            Span::raw(" open/run  "),
            hotkey("Esc"),
            Span::raw(" cancel  "),
            hotkey("Backspace"),
            Span::raw(" delete  click other pane: close edit  "),
            hotkey("q"),
            Span::raw(" quit"),
        ]);
    }

    match app.focus {
        FocusPane::Address => Line::from(vec![
            hotkey("click [][][]"),
            Span::raw(" nav  "),
            hotkey("o/:"),
            Span::raw(" edit url  "),
            hotkey("Tab/Shift+Tab"),
            Span::raw(" pane  "),
            hotkey("q"),
            Span::raw(" quit"),
        ]),
        FocusPane::Content => Line::from(vec![
            hotkey("j/k or ↑/↓"),
            Span::raw(": cursor line  "),
            hotkey("n/p or PgUp/PgDn"),
            Span::raw(": page move  "),
            hotkey("Enter"),
            Span::raw(": open if actionable  "),
            hotkey("wheel"),
            Span::raw(": scroll  "),
            hotkey("r"),
            Span::raw(": reload  "),
            hotkey("b"),
            Span::raw(": back  "),
            hotkey("f"),
            Span::raw(": forward  "),
            hotkey("i"),
            Span::raw(": img mode  "),
            hotkey(":link/:image/:media N"),
            Span::raw(" run"),
        ]),
    }
}

fn hotkey(text: &'static str) -> Span<'static> {
    Span::styled(
        text,
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn compute_layout(area: Rect) -> UiLayout {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(4),
        ])
        .split(area);

    UiLayout {
        title: root[0],
        address: root[1],
        content: root[2],
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

pub(crate) fn bool_label(value: bool) -> &'static str {
    if value { "ON" } else { "OFF" }
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

pub(crate) fn image_mode_label(mode: UiImageMode) -> &'static str {
    match mode {
        UiImageMode::Auto => "auto",
        UiImageMode::Text => "text",
    }
}

pub(crate) fn point_in_rect(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}
