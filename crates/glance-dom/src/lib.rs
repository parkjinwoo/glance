use scraper::{ElementRef, Html, Selector};
use url::Url;

#[derive(Debug, Clone)]
pub struct Document {
    pub title: String,
    pub source_url: Url,
    pub text_lines: Vec<String>,
    pub links: Vec<Link>,
    pub forms: Vec<Form>,
    pub images: Vec<MediaRef>,
    pub media_links: Vec<MediaRef>,
    pub script_count: usize,
    pub meta_http_equiv: MetaHttpEquiv,
    pub markdown_blocks: Vec<MarkdownBlock>,
}

#[derive(Debug, Clone)]
pub struct Link {
    pub text: String,
    pub href: String,
    pub absolute: Option<Url>,
}

#[derive(Debug, Clone)]
pub struct Form {
    pub method: String,
    pub action: String,
}

#[derive(Debug, Clone)]
pub struct MediaRef {
    pub alt_or_label: String,
    pub src: String,
    pub absolute: Option<Url>,
}

#[derive(Debug, Clone, Default)]
pub struct MetaHttpEquiv {
    pub refresh: Option<MetaRefresh>,
    pub cache_hints: CacheHints,
}

#[derive(Debug, Clone)]
pub struct MetaRefresh {
    pub delay_secs: u64,
    pub target_raw: Option<String>,
    pub target_absolute: Option<Url>,
}

#[derive(Debug, Clone, Default)]
pub struct CacheHints {
    pub no_store: bool,
    pub no_cache: bool,
    pub max_age_zero: bool,
    pub expires_immediate: bool,
}

#[derive(Debug, Clone)]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
    },
    ListItem {
        ordered: bool,
        text: String,
    },
    Quote {
        text: String,
    },
    CodeBlock {
        code: String,
    },
    LinkLine {
        label: String,
        url: String,
    },
    Image {
        label: String,
        url: String,
    },
    HorizontalRule,
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

pub fn parse_document(base_url: &Url, html: &str) -> Document {
    parse_document_with_options(base_url, html, true)
}

pub fn parse_document_with_options(
    base_url: &Url,
    html: &str,
    sanitize_control_chars: bool,
) -> Document {
    let doc = Html::parse_document(html);

    let title = select_text(&doc, "title").unwrap_or_else(|| base_url.to_string());
    let text_lines = collect_text_lines(&doc);
    let links = collect_links(&doc, base_url);
    let forms = collect_forms(&doc);
    let images = collect_images(&doc, base_url);
    let media_links = collect_media_links(&doc, base_url);
    let script_count = collect_count(&doc, "script");
    let meta_http_equiv = collect_meta_http_equiv(&doc, base_url);
    let markdown_blocks = collect_markdown_blocks(&doc, base_url);

    let mut out = Document {
        title,
        source_url: base_url.clone(),
        text_lines,
        links,
        forms,
        images,
        media_links,
        script_count,
        meta_http_equiv,
        markdown_blocks,
    };

    if sanitize_control_chars {
        sanitize_document(&mut out);
    }

    out
}

fn sanitize_document(doc: &mut Document) {
    doc.title = sanitize_for_display(&doc.title);
    for line in &mut doc.text_lines {
        *line = sanitize_for_display(line);
    }
    for link in &mut doc.links {
        link.text = sanitize_for_display(&link.text);
        link.href = sanitize_for_display(&link.href);
    }
    for form in &mut doc.forms {
        form.method = sanitize_for_display(&form.method);
        form.action = sanitize_for_display(&form.action);
    }
    for media in &mut doc.images {
        media.alt_or_label = sanitize_for_display(&media.alt_or_label);
        media.src = sanitize_for_display(&media.src);
    }
    for media in &mut doc.media_links {
        media.alt_or_label = sanitize_for_display(&media.alt_or_label);
        media.src = sanitize_for_display(&media.src);
    }
    for block in &mut doc.markdown_blocks {
        sanitize_markdown_block(block);
    }
}

fn sanitize_markdown_block(block: &mut MarkdownBlock) {
    match block {
        MarkdownBlock::Heading { text, .. } => *text = sanitize_for_display(text),
        MarkdownBlock::Paragraph { text } => *text = sanitize_for_display(text),
        MarkdownBlock::ListItem { text, .. } => *text = sanitize_for_display(text),
        MarkdownBlock::Quote { text } => *text = sanitize_for_display(text),
        MarkdownBlock::CodeBlock { code } => *code = sanitize_multiline(code),
        MarkdownBlock::LinkLine { label, url } => {
            *label = sanitize_for_display(label);
            *url = sanitize_for_display(url);
        }
        MarkdownBlock::Image { label, url } => {
            *label = sanitize_for_display(label);
            *url = sanitize_for_display(url);
        }
        MarkdownBlock::HorizontalRule => {}
        MarkdownBlock::Table { headers, rows } => {
            for header in headers {
                *header = sanitize_for_display(header);
            }
            for row in rows {
                for cell in row {
                    *cell = sanitize_for_display(cell);
                }
            }
        }
    }
}

fn sanitize_for_display(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_control() {
            if matches!(ch, '\n' | '\r' | '\t') {
                out.push(' ');
            }
            continue;
        }
        out.push(ch);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitize_multiline(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_control() {
            match ch {
                '\n' => out.push('\n'),
                '\r' => {}
                '\t' => out.push(' '),
                _ => {}
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn parse_selector(s: &str) -> Option<Selector> {
    Selector::parse(s).ok()
}

fn select_text(doc: &Html, selector: &str) -> Option<String> {
    let selector = parse_selector(selector)?;
    let node = doc.select(&selector).next()?;
    let text = node.text().collect::<Vec<_>>().join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() { None } else { Some(text) }
}

fn collect_text_lines(doc: &Html) -> Vec<String> {
    let mut lines = Vec::new();
    for sel in ["h1", "h2", "h3", "p", "li", "pre", "blockquote"] {
        if let Some(selector) = parse_selector(sel) {
            for node in doc.select(&selector) {
                let text = node.text().collect::<Vec<_>>().join(" ");
                let trimmed = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !trimmed.is_empty() {
                    lines.push(trimmed);
                }
            }
        }
    }

    if lines.is_empty() {
        let fallback = doc.root_element().text().collect::<Vec<_>>().join(" ");
        let collapsed = fallback.split_whitespace().collect::<Vec<_>>().join(" ");
        if !collapsed.is_empty() {
            lines.push(collapsed);
        }
    }

    lines
}

fn collect_links(doc: &Html, base_url: &Url) -> Vec<Link> {
    let mut out = Vec::new();
    let Some(selector) = parse_selector("a[href]") else {
        return out;
    };

    for node in doc.select(&selector) {
        let href = node.value().attr("href").unwrap_or_default().to_string();
        let mut text = node
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if href.is_empty() {
            continue;
        }

        let absolute = base_url.join(&href).ok();
        if text.is_empty() {
            text = fallback_label(base_url, absolute.as_ref(), &href, "link");
        }
        out.push(Link {
            text,
            href,
            absolute,
        });
    }

    out
}

fn collect_forms(doc: &Html) -> Vec<Form> {
    let mut out = Vec::new();
    let Some(selector) = parse_selector("form") else {
        return out;
    };

    for node in doc.select(&selector) {
        let method = node.value().attr("method").unwrap_or("GET").to_uppercase();
        let action = node.value().attr("action").unwrap_or("").to_string();
        out.push(Form { method, action });
    }

    out
}

fn collect_images(doc: &Html, base_url: &Url) -> Vec<MediaRef> {
    let mut out = Vec::new();
    let Some(selector) = parse_selector("img[src]") else {
        return out;
    };

    for node in doc.select(&selector) {
        let src = node.value().attr("src").unwrap_or_default().to_string();
        if src.is_empty() {
            continue;
        }
        let absolute = base_url.join(&src).ok();
        let alt = node.value().attr("alt").unwrap_or_default().trim();
        let alt_or_label = if alt.is_empty() {
            fallback_label(base_url, absolute.as_ref(), &src, "image")
        } else {
            alt.to_string()
        };
        out.push(MediaRef {
            alt_or_label,
            src,
            absolute,
        });
    }

    out
}

fn collect_media_links(doc: &Html, base_url: &Url) -> Vec<MediaRef> {
    let mut out = Vec::new();
    let Some(selector) = parse_selector("a[href]") else {
        return out;
    };

    for node in doc.select(&selector) {
        let href = node.value().attr("href").unwrap_or_default().to_string();
        if href.is_empty() {
            continue;
        }
        let lower = href.to_ascii_lowercase();
        let media_probe = lower.split(['?', '#']).next().unwrap_or(lower.as_str());
        let is_media = [".mp4", ".webm", ".mp3", ".m4a", ".ogg", ".mov"]
            .iter()
            .any(|ext| media_probe.ends_with(ext));
        if !is_media {
            continue;
        }

        let mut label = node
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let absolute = base_url.join(&href).ok();
        if label.is_empty() {
            label = fallback_label(base_url, absolute.as_ref(), &href, "media");
        }

        out.push(MediaRef {
            alt_or_label: label,
            src: href.clone(),
            absolute,
        });
    }

    out
}

fn collect_count(doc: &Html, selector: &str) -> usize {
    let Some(selector) = parse_selector(selector) else {
        return 0;
    };
    doc.select(&selector).count()
}

fn collect_markdown_blocks(doc: &Html, base_url: &Url) -> Vec<MarkdownBlock> {
    let Some(selector) = parse_selector(
        "h1, h2, h3, h4, h5, h6, p, li, pre, blockquote, hr, table, a[href], img[src]",
    ) else {
        return Vec::new();
    };

    let mut blocks = Vec::new();
    for node in doc.select(&selector) {
        let tag = node.value().name();
        match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let text = collapsed_text(&node);
                if text.is_empty() {
                    continue;
                }
                let level = tag[1..].parse::<u8>().unwrap_or(1);
                blocks.push(MarkdownBlock::Heading { level, text });
            }
            "p" => {
                let text = collapsed_text(&node);
                if !text.is_empty() {
                    blocks.push(MarkdownBlock::Paragraph { text });
                }
            }
            "li" => {
                let text = collapsed_text(&node);
                if text.is_empty() {
                    continue;
                }
                let ordered = node
                    .parent()
                    .and_then(ElementRef::wrap)
                    .is_some_and(|p| p.value().name() == "ol");
                blocks.push(MarkdownBlock::ListItem { ordered, text });
            }
            "pre" => {
                let code = node
                    .text()
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string();
                if !code.is_empty() {
                    blocks.push(MarkdownBlock::CodeBlock { code });
                }
            }
            "blockquote" => {
                let text = collapsed_text(&node);
                if !text.is_empty() {
                    blocks.push(MarkdownBlock::Quote { text });
                }
            }
            "a" => {
                let href = node.value().attr("href").unwrap_or_default();
                if href.is_empty() {
                    continue;
                }
                let absolute = base_url.join(href).ok();
                let url = absolute
                    .as_ref()
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| href.to_string());
                let mut label = collapsed_text(&node);
                if label.is_empty() {
                    label = fallback_label(base_url, absolute.as_ref(), href, "link");
                }
                blocks.push(MarkdownBlock::LinkLine { label, url });
            }
            "img" => {
                let src = node.value().attr("src").unwrap_or_default();
                if src.is_empty() {
                    continue;
                }
                let absolute = base_url.join(src).ok();
                let url = absolute
                    .as_ref()
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| src.to_string());
                let alt = node.value().attr("alt").unwrap_or_default().trim();
                let label = if alt.is_empty() {
                    fallback_label(base_url, absolute.as_ref(), src, "image")
                } else {
                    alt.to_string()
                };
                blocks.push(MarkdownBlock::Image { label, url });
            }
            "hr" => blocks.push(MarkdownBlock::HorizontalRule),
            "table" => {
                if let Some((headers, rows)) = parse_table(&node) {
                    blocks.push(MarkdownBlock::Table { headers, rows });
                }
            }
            _ => {}
        }
    }

    blocks
}

fn collapsed_text(node: &ElementRef<'_>) -> String {
    node.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_table(node: &ElementRef<'_>) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let tr_selector = parse_selector("tr")?;
    let th_selector = parse_selector("th")?;
    let td_selector = parse_selector("td")?;

    let mut headers = Vec::new();
    let mut rows = Vec::new();

    for tr in node.select(&tr_selector) {
        let th_cells = tr
            .select(&th_selector)
            .map(|n| collapsed_text(&n))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !th_cells.is_empty() && headers.is_empty() {
            headers = th_cells;
            continue;
        }

        let row = tr
            .select(&td_selector)
            .map(|n| collapsed_text(&n))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !row.is_empty() {
            rows.push(row);
        }
    }

    if headers.is_empty() && rows.is_empty() {
        None
    } else {
        Some((headers, rows))
    }
}

fn collect_meta_http_equiv(doc: &Html, base_url: &Url) -> MetaHttpEquiv {
    let Some(selector) = parse_selector("meta[http-equiv][content]") else {
        return MetaHttpEquiv::default();
    };

    let mut out = MetaHttpEquiv::default();
    for node in doc.select(&selector) {
        let http_equiv = node
            .value()
            .attr("http-equiv")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let content = node.value().attr("content").unwrap_or_default().trim();
        if content.is_empty() {
            continue;
        }

        match http_equiv.as_str() {
            "refresh" => {
                if out.refresh.is_none() {
                    out.refresh = parse_meta_refresh(content, base_url);
                }
            }
            "cache-control" => {
                for token in content.split(',') {
                    let token = token.trim().to_ascii_lowercase();
                    if token == "no-store" {
                        out.cache_hints.no_store = true;
                    } else if token == "no-cache" {
                        out.cache_hints.no_cache = true;
                    } else if token == "max-age=0" {
                        out.cache_hints.max_age_zero = true;
                    }
                }
            }
            "pragma" => {
                if content.to_ascii_lowercase().contains("no-cache") {
                    out.cache_hints.no_cache = true;
                }
            }
            "expires" => {
                if is_immediate_expires(content) {
                    out.cache_hints.expires_immediate = true;
                }
            }
            _ => {}
        }
    }

    out
}

fn parse_meta_refresh(content: &str, base_url: &Url) -> Option<MetaRefresh> {
    let mut parts = content.splitn(2, ';');
    let delay_raw = parts.next()?.trim();
    let delay_secs = delay_raw.parse::<u64>().ok()?;

    let target_raw = parts
        .next()
        .and_then(extract_refresh_url)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let target_absolute = match target_raw.as_deref() {
        Some(raw) => base_url.join(raw).ok().or_else(|| Url::parse(raw).ok()),
        None => Some(base_url.clone()),
    };

    Some(MetaRefresh {
        delay_secs,
        target_raw,
        target_absolute,
    })
}

fn extract_refresh_url(raw: &str) -> Option<&str> {
    let lower = raw.to_ascii_lowercase();
    let idx = lower.find("url=")?;
    let value = raw.get(idx + 4..)?.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(value);
    let value = value
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(value);
    Some(value)
}

fn is_immediate_expires(content: &str) -> bool {
    let raw = content.trim().to_ascii_lowercase();
    raw == "0" || raw == "-1" || raw.contains("1970")
}

fn fallback_label(base_url: &Url, absolute: Option<&Url>, raw: &str, kind: &str) -> String {
    let target = absolute
        .cloned()
        .or_else(|| base_url.join(raw).ok())
        .or_else(|| Url::parse(raw).ok());
    let Some(target) = target else {
        return format!("({kind})");
    };

    let base_host = base_url.host_str().unwrap_or_default();
    let target_host = target.host_str().unwrap_or_default();
    let same_host = !base_host.is_empty() && base_host.eq_ignore_ascii_case(target_host);

    let mut label = if same_host {
        same_host_label(base_url, &target)
    } else {
        cross_host_label(&target)
    };
    if label.is_empty() {
        label = format!("({kind})");
    }
    truncate_label(&label, 80)
}

fn same_host_label(base_url: &Url, target: &Url) -> String {
    let mut out = if target.path().is_empty() {
        "/".to_string()
    } else {
        target.path().to_string()
    };
    if let Some(query) = target.query() {
        out.push('?');
        out.push_str(query);
    }
    let path_query_same = target.path() == base_url.path() && target.query() == base_url.query();
    if let Some(fragment) = target.fragment()
        && (path_query_same || !fragment.is_empty())
    {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

fn cross_host_label(target: &Url) -> String {
    let host = target.host_str().unwrap_or_default();
    let path = if target.path().is_empty() {
        "/"
    } else {
        target.path()
    };
    let mut out = if host.is_empty() {
        path.to_string()
    } else {
        format!("{host}{path}")
    };
    if let Some(query) = target.query() {
        out.push('?');
        out.push_str(query);
    }
    if let Some(fragment) = target.fragment() {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

fn truncate_label(label: &str, max_chars: usize) -> String {
    let count = label.chars().count();
    if count <= max_chars {
        return label.to_string();
    }

    if let Some(hash_idx) = label.find('#') {
        let frag = label.chars().skip(hash_idx).collect::<String>();
        let prefix_budget = max_chars.saturating_sub(frag.chars().count() + 1);
        if prefix_budget > 8 {
            let prefix = label.chars().take(prefix_budget).collect::<String>();
            return format!("{prefix}…{frag}");
        }
    }

    let mut out = label
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::{MarkdownBlock, parse_document};
    use url::Url;

    #[test]
    fn extracts_text_and_links() {
        let html = r#"
            <html>
              <head><title>Test</title></head>
              <body>
                <h1>Hello</h1>
                <p>World</p>
                <a href="/next">Next</a>
                <script>console.log('x')</script>
              </body>
            </html>
        "#;
        let base = Url::parse("https://example.com").expect("url");
        let doc = parse_document(&base, html);
        assert_eq!(doc.title, "Test");
        assert_eq!(doc.script_count, 1);
        assert!(!doc.text_lines.is_empty());
        assert_eq!(doc.links.len(), 1);
        assert_eq!(
            doc.links[0].absolute.as_ref().expect("absolute").as_str(),
            "https://example.com/next"
        );
    }

    #[test]
    fn parses_meta_http_equiv_refresh_and_cache_hints() {
        let html = r#"
            <html>
              <head>
                <meta http-equiv="refresh" content="0; url=/moved" />
                <meta http-equiv="cache-control" content="no-cache, no-store, max-age=0" />
                <meta http-equiv="pragma" content="no-cache" />
                <meta http-equiv="expires" content="0" />
              </head>
              <body></body>
            </html>
        "#;
        let base = Url::parse("https://example.com/path").expect("url");
        let doc = parse_document(&base, html);
        let refresh = doc.meta_http_equiv.refresh.expect("refresh");
        assert_eq!(refresh.delay_secs, 0);
        assert_eq!(refresh.target_raw.as_deref(), Some("/moved"));
        assert_eq!(
            refresh.target_absolute.expect("absolute").as_str(),
            "https://example.com/moved"
        );
        assert!(doc.meta_http_equiv.cache_hints.no_store);
        assert!(doc.meta_http_equiv.cache_hints.no_cache);
        assert!(doc.meta_http_equiv.cache_hints.max_age_zero);
        assert!(doc.meta_http_equiv.cache_hints.expires_immediate);
    }

    #[test]
    fn extracts_markdown_blocks() {
        let html = r#"
            <html>
              <body>
                <h1>Hello</h1>
                <p>Paragraph</p>
                <ul><li>Item</li></ul>
                <blockquote>Quote</blockquote>
                <pre>code</pre>
              </body>
            </html>
        "#;
        let base = Url::parse("https://example.com").expect("url");
        let doc = parse_document(&base, html);
        assert!(
            doc.markdown_blocks
                .iter()
                .any(|b| matches!(b, MarkdownBlock::Heading { level: 1, text } if text == "Hello"))
        );
        assert!(
            doc.markdown_blocks
                .iter()
                .any(|b| matches!(b, MarkdownBlock::Paragraph { text } if text == "Paragraph"))
        );
        assert!(
            doc.markdown_blocks
                .iter()
                .any(|b| matches!(b, MarkdownBlock::ListItem { text, .. } if text == "Item"))
        );
    }

    #[test]
    fn fallback_uses_path_and_fragment_for_same_host() {
        let html = r##"
            <html><body>
                <a href="#intro"></a>
                <a href="/docs#usage"></a>
            </body></html>
        "##;
        let base = Url::parse("https://example.com/docs").expect("url");
        let doc = parse_document(&base, html);
        assert_eq!(doc.links[0].text, "/docs#intro");
        assert_eq!(doc.links[1].text, "/docs#usage");
    }

    #[test]
    fn image_alt_fallback_uses_url_when_alt_missing() {
        let html = r#"<html><body><img src="/assets/logo.png" /></body></html>"#;
        let base = Url::parse("https://example.com/x").expect("url");
        let doc = parse_document(&base, html);
        assert_eq!(doc.images[0].alt_or_label, "/assets/logo.png");
    }

    #[test]
    fn strips_terminal_control_chars() {
        let html =
            "<html><head><title>Te\x1bst</title></head><body><p>Hi\x1b[31m</p></body></html>";
        let base = Url::parse("https://example.com").expect("url");
        let doc = parse_document(&base, html);
        assert!(!doc.title.contains('\x1b'));
        assert!(doc.text_lines.iter().all(|line| !line.contains('\x1b')));
    }
}
