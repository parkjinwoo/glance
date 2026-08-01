use glance_dom::{Document, LiveControl, LiveControlKind, LiveFieldKind, MarkdownBlock};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub title: String,
    pub source_url: String,
    pub lines: Vec<String>,
    pub links: Vec<String>,
    pub media: Vec<String>,
    pub interactive_spans: Vec<InteractiveSpan>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct InteractiveSpan {
    pub line: usize,
    pub col_start: u16,
    pub col_end: u16,
    pub action: InteractiveAction,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InteractiveAction {
    OpenLink(usize),
    OpenImage(usize),
    OpenMedia(usize),
    OpenForm(usize),
    ActivateButton(usize),
    EditLiveField(usize),
    ActivateLiveControl(usize),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderIconMode {
    Auto,
    Ascii,
}

pub fn render_document(doc: &Document, icon_mode: RenderIconMode) -> RenderedPage {
    let (body_lines, mut body_spans) = if doc.markdown_blocks.is_empty() {
        (render_fallback_text_lines(doc), Vec::new())
    } else {
        render_markdown_blocks(doc, icon_mode)
    };

    let mut lines = Vec::new();
    let mut interactive_spans = Vec::new();

    if !doc.live_controls.is_empty() {
        let has_layer = doc.live_controls.iter().any(|control| control.in_layer);
        let context = doc
            .live_controls
            .iter()
            .find(|control| !has_layer || control.in_layer)
            .and_then(|control| control.context.as_deref())
            .filter(|context| !context.is_empty());
        lines.push(match (has_layer, context) {
            (true, Some(context)) => format!("╭─ ACTIVE PANEL · {context}"),
            (true, None) => "╭─ ACTIVE PANEL".to_string(),
            (false, Some(context)) => format!("╭─ PAGE CONTROLS · {context}"),
            (false, None) => "╭─ PAGE CONTROLS".to_string(),
        });
        for (i, control) in doc
            .live_controls
            .iter()
            .enumerate()
            .filter(|(_, control)| !has_layer || control.in_layer)
        {
            let action = if control.kind == LiveControlKind::Field {
                InteractiveAction::EditLiveField(i)
            } else {
                InteractiveAction::ActivateLiveControl(i)
            };
            let token = live_control_token(control, i, icon_mode);
            let detail = live_control_detail(control);
            let line = format!("{token}  {}{detail}", control.label);
            let line_idx = lines.len();
            let line_width = UnicodeWidthStr::width(line.as_str()) as u16;
            lines.push(line);
            interactive_spans.push(InteractiveSpan {
                line: line_idx,
                col_start: 0,
                col_end: line_width,
                action,
            });
        }
        lines.push("╰─".to_string());
        lines.push(String::new());
    }

    if !doc.forms.is_empty() {
        for (i, form) in doc.forms.iter().enumerate() {
            let token = interactive_token(InteractiveAction::OpenForm(i), i, icon_mode);
            let required = form
                .fields
                .iter()
                .find(|field| field.editable)
                .is_some_and(|field| field.required);
            let line = format!(
                "{token} {}{}",
                form.label,
                if required { "  • required" } else { "" }
            );
            let line_idx = lines.len();
            let line_width = UnicodeWidthStr::width(line.as_str()) as u16;
            lines.push(line);
            interactive_spans.push(InteractiveSpan {
                line: line_idx,
                col_start: 0,
                col_end: line_width,
                action: InteractiveAction::OpenForm(i),
            });
        }
        lines.push(String::new());
    }

    let body_offset = lines.len();
    for span in &mut body_spans {
        span.line = span.line.saturating_add(body_offset);
    }
    lines.extend(body_lines);
    interactive_spans.extend(body_spans);

    if !doc.continuation_buttons.is_empty() {
        if lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(String::new());
        }
        for (i, button) in doc.continuation_buttons.iter().enumerate() {
            let token = interactive_token(InteractiveAction::ActivateButton(i), i, icon_mode);
            let line = format!("{token}  {}", button.label);
            let line_idx = lines.len();
            let line_width = UnicodeWidthStr::width(line.as_str()) as u16;
            lines.push(line);
            interactive_spans.push(InteractiveSpan {
                line: line_idx,
                col_start: 0,
                col_end: line_width,
                action: InteractiveAction::ActivateButton(i),
            });
        }
    }

    let links = doc
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let label = if l.text.is_empty() {
                "(untitled link)"
            } else {
                l.text.as_str()
            };
            let url = l
                .absolute
                .as_ref()
                .map(|u| u.as_str())
                .unwrap_or_else(|| l.href.as_str());
            format!("[{}] {} -> {}", i + 1, label, url)
        })
        .collect();

    let media = doc
        .images
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let src = m
                .absolute
                .as_ref()
                .map(|u| u.as_str())
                .unwrap_or_else(|| m.src.as_str());
            format!("[img:{}] {} -> {}", i + 1, m.alt_or_label, src)
        })
        .chain(doc.media_links.iter().enumerate().map(|(i, m)| {
            let src = m
                .absolute
                .as_ref()
                .map(|u| u.as_str())
                .unwrap_or_else(|| m.src.as_str());
            format!("[media:{}] {} -> {}", i + 1, m.alt_or_label, src)
        }))
        .collect();

    RenderedPage {
        title: doc.title.clone(),
        source_url: doc.source_url.to_string(),
        lines,
        links,
        media,
        interactive_spans,
    }
}

fn live_control_token(control: &LiveControl, index: usize, icon_mode: RenderIconMode) -> String {
    if icon_mode == RenderIconMode::Ascii {
        return match control.field_kind {
            Some(LiveFieldKind::Checkbox) => {
                if control.checked { "[x]" } else { "[ ]" }.to_string()
            }
            Some(LiveFieldKind::Radio) => if control.checked { "(*)" } else { "( )" }.to_string(),
            Some(LiveFieldKind::Select) => "[v]".to_string(),
            Some(LiveFieldKind::Text | LiveFieldKind::TextArea) => "[>]".to_string(),
            None => interactive_token(
                if control.kind == LiveControlKind::Field {
                    InteractiveAction::EditLiveField(index)
                } else {
                    InteractiveAction::ActivateLiveControl(index)
                },
                index,
                icon_mode,
            ),
        };
    }
    match control.field_kind {
        Some(LiveFieldKind::Text) => "✎".to_string(),
        Some(LiveFieldKind::TextArea) => "▤".to_string(),
        Some(LiveFieldKind::Checkbox) => if control.checked { "☑" } else { "☐" }.to_string(),
        Some(LiveFieldKind::Radio) => if control.checked { "◉" } else { "○" }.to_string(),
        Some(LiveFieldKind::Select) => "▾".to_string(),
        None => match control.kind {
            LiveControlKind::Disclosure => "›".to_string(),
            LiveControlKind::Action => "●".to_string(),
            LiveControlKind::Submit => "✓".to_string(),
            LiveControlKind::Field => "✎".to_string(),
        },
    }
}

fn live_control_detail(control: &LiveControl) -> String {
    match control.field_kind {
        Some(LiveFieldKind::Text | LiveFieldKind::TextArea) => {
            let value = control.value.lines().next().unwrap_or_default().trim();
            let detail = if value.is_empty() {
                control.placeholder.as_deref().unwrap_or_default()
            } else {
                value
            };
            if detail.is_empty() {
                String::new()
            } else {
                format!("  ·  {}", compact_label(detail, 72))
            }
        }
        Some(LiveFieldKind::Select) => {
            let label = control
                .options
                .iter()
                .find(|option| option.selected)
                .map(|option| option.label.as_str())
                .unwrap_or(control.value.as_str());
            if label.is_empty() {
                String::new()
            } else {
                format!("  ·  {}", compact_label(label, 48))
            }
        }
        Some(LiveFieldKind::Checkbox | LiveFieldKind::Radio) => String::new(),
        None if control.kind.requires_confirmation() => "  [confirm]".to_string(),
        None => String::new(),
    }
}

fn compact_label(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let compact = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{compact}…")
    } else {
        compact
    }
}

fn render_fallback_text_lines(doc: &Document) -> Vec<String> {
    if doc.text_lines.is_empty() {
        vec!["(No readable text extracted from page)".to_string()]
    } else {
        doc.text_lines.clone()
    }
}

fn render_markdown_blocks(
    doc: &Document,
    icon_mode: RenderIconMode,
) -> (Vec<String>, Vec<InteractiveSpan>) {
    let blocks = &doc.markdown_blocks;
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    let mut ordered_counter: usize = 1;

    for block in blocks {
        match block {
            MarkdownBlock::Heading { level, text } => {
                ordered_counter = 1;
                let marks = "#".repeat((*level).clamp(1, 6) as usize);
                lines.push(format!("{marks} {text}"));
                lines.push(String::new());
            }
            MarkdownBlock::Paragraph { text } => {
                ordered_counter = 1;
                lines.push(text.clone());
                lines.push(String::new());
            }
            MarkdownBlock::ListItem { ordered, text } => {
                if *ordered {
                    lines.push(format!("{ordered_counter}. {text}"));
                    ordered_counter = ordered_counter.saturating_add(1);
                } else {
                    ordered_counter = 1;
                    lines.push(format!("• {text}"));
                }
            }
            MarkdownBlock::Quote { text } => {
                ordered_counter = 1;
                lines.push(format!("│ {text}"));
                lines.push(String::new());
            }
            MarkdownBlock::CodeBlock { code } => {
                ordered_counter = 1;
                lines.push("```".to_string());
                for line in code.lines() {
                    lines.push(line.to_string());
                }
                lines.push("```".to_string());
                lines.push(String::new());
            }
            MarkdownBlock::LinkLine { label, url } => {
                ordered_counter = 1;
                if let Some(idx) = find_media_index(doc, url) {
                    let token =
                        interactive_token(InteractiveAction::OpenMedia(idx), idx, icon_mode);
                    let line = format!("{token} {label}");
                    let line_idx = lines.len();
                    let line_width = UnicodeWidthStr::width(line.as_str()) as u16;
                    lines.push(line);
                    spans.push(InteractiveSpan {
                        line: line_idx,
                        col_start: 0,
                        col_end: line_width,
                        action: InteractiveAction::OpenMedia(idx),
                    });
                } else if let Some(idx) = find_link_index(doc, url) {
                    let token = interactive_token(InteractiveAction::OpenLink(idx), idx, icon_mode);
                    let line = format!("{token} {label}");
                    let line_idx = lines.len();
                    let line_width = UnicodeWidthStr::width(line.as_str()) as u16;
                    lines.push(line);
                    spans.push(InteractiveSpan {
                        line: line_idx,
                        col_start: 0,
                        col_end: line_width,
                        action: InteractiveAction::OpenLink(idx),
                    });
                } else {
                    lines.push(format!("↗ {label}: {url}"));
                }
            }
            MarkdownBlock::Image { label, url } => {
                ordered_counter = 1;
                if let Some(idx) = find_image_index(doc, url) {
                    let token =
                        interactive_token(InteractiveAction::OpenImage(idx), idx, icon_mode);
                    let line = format!("{token} {label}");
                    let line_idx = lines.len();
                    let line_width = UnicodeWidthStr::width(line.as_str()) as u16;
                    lines.push(line);
                    spans.push(InteractiveSpan {
                        line: line_idx,
                        col_start: 0,
                        col_end: line_width,
                        action: InteractiveAction::OpenImage(idx),
                    });
                } else {
                    lines.push(format!("[IMG] {label}"));
                }
            }
            MarkdownBlock::HorizontalRule => {
                ordered_counter = 1;
                lines.push("────────────────────────────────".to_string());
            }
            MarkdownBlock::Table { headers, rows } => {
                ordered_counter = 1;
                lines.extend(format_table(headers, rows));
                lines.push(String::new());
            }
        }
    }

    if lines.is_empty() {
        lines.push("(No readable markdown blocks extracted)".to_string());
    }

    (lines, spans)
}

fn interactive_token(action: InteractiveAction, index: usize, icon_mode: RenderIconMode) -> String {
    let n = index + 1;
    match icon_mode {
        RenderIconMode::Auto => match action {
            InteractiveAction::OpenLink(_) => format!("󰌹{n}"),
            InteractiveAction::OpenImage(_) => format!("󰋩{n}"),
            InteractiveAction::OpenMedia(_) => format!("󰕧{n}"),
            InteractiveAction::OpenForm(_) => format!("󰘎{n}"),
            InteractiveAction::ActivateButton(_) => format!("󰐕{n}"),
            InteractiveAction::EditLiveField(_) => format!("󰘎{n}"),
            InteractiveAction::ActivateLiveControl(_) => format!("󰐕{n}"),
        },
        RenderIconMode::Ascii => match action {
            InteractiveAction::OpenLink(_) => format!("[L{n}]"),
            InteractiveAction::OpenImage(_) => format!("[I{n}]"),
            InteractiveAction::OpenMedia(_) => format!("[M{n}]"),
            InteractiveAction::OpenForm(_) => format!("[F{n}]"),
            InteractiveAction::ActivateButton(_) => format!("[+{n}]"),
            InteractiveAction::EditLiveField(_) => format!("[E{n}]"),
            InteractiveAction::ActivateLiveControl(_) => format!("[A{n}]"),
        },
    }
}

fn format_table(headers: &[String], rows: &[Vec<String>]) -> Vec<String> {
    let cols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if cols == 0 {
        return Vec::new();
    }

    let mut widths = vec![3usize; cols];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = widths[i].max(UnicodeWidthStr::width(h.as_str()));
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(UnicodeWidthStr::width(cell.as_str()));
        }
    }

    let mut out = Vec::new();
    if !headers.is_empty() {
        let padded = headers
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
            .collect::<Vec<_>>();
        out.push(format!("| {} |", padded.join(" | ")));
        let sep = widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("-|-");
        out.push(format!("|-{sep}-|"));
    }

    for row in rows {
        let mut cells = Vec::with_capacity(cols);
        for (i, w) in widths.iter().enumerate() {
            let value = row.get(i).cloned().unwrap_or_default();
            cells.push(format!("{:<width$}", value, width = *w));
        }
        out.push(format!("| {} |", cells.join(" | ")));
    }

    out
}

fn find_link_index(doc: &Document, url: &str) -> Option<usize> {
    doc.links.iter().position(|link| {
        link.absolute
            .as_ref()
            .map(|u| u.as_str() == url)
            .unwrap_or(false)
            || link.href == url
    })
}

fn find_image_index(doc: &Document, url: &str) -> Option<usize> {
    doc.images.iter().position(|img| {
        img.absolute
            .as_ref()
            .map(|u| u.as_str() == url)
            .unwrap_or(false)
            || img.src == url
    })
}

fn find_media_index(doc: &Document, url: &str) -> Option<usize> {
    doc.media_links.iter().position(|media| {
        media
            .absolute
            .as_ref()
            .map(|u| u.as_str() == url)
            .unwrap_or(false)
            || media.src == url
    })
}

#[cfg(test)]
mod tests {
    use super::{InteractiveAction, RenderIconMode, render_document};
    use glance_dom::parse_document;
    use url::Url;

    #[test]
    fn builds_interactive_spans_for_targets() {
        let html = r#"
            <html><body>
                <a href="/a">A</a>
                <img src="/i.png" alt="img-a" />
                <a href="/m.mp4">video</a>
                <form action="/search"><input type="search" name="q"></form>
                <button data-glance-continuation="7">Load more posts</button>
            </body></html>
        "#;
        let base = Url::parse("https://example.com").expect("url");
        let doc = parse_document(&base, html);
        let rendered = render_document(&doc, RenderIconMode::Ascii);

        assert!(
            rendered
                .interactive_spans
                .iter()
                .any(|s| matches!(s.action, InteractiveAction::OpenLink(0)))
        );
        assert!(
            rendered
                .interactive_spans
                .iter()
                .any(|s| matches!(s.action, InteractiveAction::OpenImage(0)))
        );
        assert!(
            rendered
                .interactive_spans
                .iter()
                .any(|s| matches!(s.action, InteractiveAction::OpenMedia(0)))
        );
        assert!(
            rendered
                .interactive_spans
                .iter()
                .any(|s| matches!(s.action, InteractiveAction::OpenForm(0)))
        );
        assert!(
            rendered
                .interactive_spans
                .iter()
                .any(|s| matches!(s.action, InteractiveAction::ActivateButton(0)))
        );
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line == "[+1]  Load more posts")
        );
    }

    #[test]
    fn renders_nerd_font_tokens_in_auto_mode() {
        let html = r#"<html><body><a href="/a">A</a></body></html>"#;
        let base = Url::parse("https://example.com").expect("url");
        let doc = parse_document(&base, html);
        let rendered = render_document(&doc, RenderIconMode::Auto);
        assert!(rendered.lines.iter().any(|l| l.starts_with("󰌹1 ")));
    }

    #[test]
    fn renders_live_fields_and_confirmable_actions_first() {
        let html = r#"
            <main><p>Post body</p></main>
            <textarea data-glance-live-control="0" data-glance-live-kind="field"
                      data-glance-live-field-kind="textarea"
                      data-glance-live-label="내용" data-glance-live-value="draft"
                      data-glance-live-layer="true" placeholder="댓글을 입력하세요…"></textarea>
            <button data-glance-live-control="1" data-glance-live-kind="action"
                    data-glance-live-label="공유" data-glance-live-layer="true">공유</button>
        "#;
        let base = Url::parse("https://example.com/post").expect("url");
        let doc = parse_document(&base, html);
        let rendered = render_document(&doc, RenderIconMode::Ascii);

        assert_eq!(rendered.lines[0], "╭─ ACTIVE PANEL");
        assert!(rendered.lines[1].starts_with("[>]  내용"));
        assert!(rendered.lines[2].contains("[A2]  공유  [confirm]"));
        assert!(
            rendered
                .interactive_spans
                .iter()
                .any(|span| { matches!(span.action, InteractiveAction::EditLiveField(0)) })
        );
        assert!(
            rendered
                .interactive_spans
                .iter()
                .any(|span| { matches!(span.action, InteractiveAction::ActivateLiveControl(1)) })
        );
    }
}
