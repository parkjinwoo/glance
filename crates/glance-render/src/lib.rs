use glance_dom::{Document, MarkdownBlock};
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
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderIconMode {
    Auto,
    Ascii,
}

pub fn render_document(doc: &Document, icon_mode: RenderIconMode) -> RenderedPage {
    let (mut lines, interactive_spans) = if doc.markdown_blocks.is_empty() {
        (render_fallback_text_lines(doc), Vec::new())
    } else {
        render_markdown_blocks(doc, icon_mode)
    };

    if !doc.forms.is_empty() {
        lines.push(String::new());
        lines.push("Forms:".to_string());
        for (i, form) in doc.forms.iter().enumerate() {
            lines.push(format!("  [{}] {} {}", i + 1, form.method, form.action));
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
        },
        RenderIconMode::Ascii => match action {
            InteractiveAction::OpenLink(_) => format!("[L{n}]"),
            InteractiveAction::OpenImage(_) => format!("[I{n}]"),
            InteractiveAction::OpenMedia(_) => format!("[M{n}]"),
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
    }

    #[test]
    fn renders_nerd_font_tokens_in_auto_mode() {
        let html = r#"<html><body><a href="/a">A</a></body></html>"#;
        let base = Url::parse("https://example.com").expect("url");
        let doc = parse_document(&base, html);
        let rendered = render_document(&doc, RenderIconMode::Auto);
        assert!(rendered.lines.iter().any(|l| l.starts_with("󰌹1 ")));
    }
}
