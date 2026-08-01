use scraper::{ElementRef, Html, Selector};
use std::collections::HashMap;
use url::Url;

#[derive(Debug, Clone)]
pub struct Document {
    pub title: String,
    pub source_url: Url,
    pub text_lines: Vec<String>,
    pub links: Vec<Link>,
    pub forms: Vec<Form>,
    pub continuation_buttons: Vec<ContinuationButton>,
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
    pub action_absolute: Option<Url>,
    pub fields: Vec<FormField>,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct FormField {
    pub name: String,
    pub value: String,
    pub label: String,
    pub kind: String,
    pub editable: bool,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct ContinuationButton {
    pub label: String,
    pub glance_id: usize,
}

#[derive(Debug, Clone)]
pub struct MediaRef {
    pub alt_or_label: String,
    pub src: String,
    pub absolute: Option<Url>,
    pub kind: MediaKind,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MediaKind {
    Image,
    Audio,
    Video,
    Stream,
    Media,
}

impl MediaKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Stream => "stream",
            Self::Media => "media",
        }
    }
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
    let forms = collect_forms(&doc, base_url);
    let continuation_buttons = collect_continuation_buttons(&doc);
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
        continuation_buttons,
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
        form.label = sanitize_for_display(&form.label);
        for field in &mut form.fields {
            field.name = sanitize_for_display(&field.name);
            field.value = sanitize_for_display(&field.value);
            field.label = sanitize_for_display(&field.label);
            field.kind = sanitize_for_display(&field.kind);
        }
    }
    for button in &mut doc.continuation_buttons {
        button.label = sanitize_for_display(&button.label);
    }
    for media in &mut doc.images {
        media.alt_or_label = sanitize_for_display(&media.alt_or_label);
        media.src = sanitize_for_display(&media.src);
        media.mime_type = media
            .mime_type
            .take()
            .map(|mime| sanitize_for_display(&mime));
    }
    for media in &mut doc.media_links {
        media.alt_or_label = sanitize_for_display(&media.alt_or_label);
        media.src = sanitize_for_display(&media.src);
        media.mime_type = media
            .mime_type
            .take()
            .map(|mime| sanitize_for_display(&mime));
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
                let text = visible_text(&node);
                let trimmed = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !trimmed.is_empty() {
                    lines.push(trimmed);
                }
            }
        }
    }

    if lines.is_empty() {
        let fallback = visible_text(&doc.root_element());
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
        let mut text = accessible_label(&node);
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

fn collect_forms(doc: &Html, base_url: &Url) -> Vec<Form> {
    let mut out = Vec::new();
    let Some(selector) = parse_selector("form") else {
        return out;
    };
    let Some(field_selector) = parse_selector("input[name], textarea[name], select[name]") else {
        return out;
    };

    for node in doc.select(&selector) {
        let method = node.value().attr("method").unwrap_or("GET").to_uppercase();
        let action = node.value().attr("action").unwrap_or("").to_string();
        let action_absolute = if action.trim().is_empty() {
            Some(base_url.clone())
        } else {
            base_url.join(&action).ok()
        };
        let mut fields = Vec::new();
        for field_node in node.select(&field_selector) {
            if field_node.value().attr("disabled").is_some() {
                continue;
            }
            let tag = field_node.value().name();
            let name = field_node
                .value()
                .attr("name")
                .unwrap_or_default()
                .trim()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let kind = if tag == "input" {
                field_node
                    .value()
                    .attr("type")
                    .unwrap_or("text")
                    .to_ascii_lowercase()
            } else {
                tag.to_string()
            };
            if matches!(
                kind.as_str(),
                "submit" | "button" | "reset" | "file" | "image"
            ) {
                continue;
            }
            if matches!(kind.as_str(), "checkbox" | "radio")
                && field_node.value().attr("checked").is_none()
            {
                continue;
            }

            let value = match kind.as_str() {
                "textarea" => field_node.text().collect::<String>(),
                "select" => selected_option_value(&field_node),
                "checkbox" | "radio" => {
                    field_node.value().attr("value").unwrap_or("on").to_string()
                }
                _ => field_node
                    .value()
                    .attr("value")
                    .unwrap_or_default()
                    .to_string(),
            };
            let label = field_node
                .value()
                .attr("aria-label")
                .or_else(|| field_node.value().attr("placeholder"))
                .or_else(|| field_node.value().attr("title"))
                .unwrap_or(&name)
                .trim()
                .to_string();
            let editable = matches!(
                kind.as_str(),
                "text" | "search" | "email" | "url" | "tel" | "number" | "textarea"
            );
            fields.push(FormField {
                name,
                value,
                label,
                kind,
                editable,
                required: field_node.value().attr("required").is_some(),
            });
        }

        let label = fields
            .iter()
            .find(|field| field.editable)
            .map(|field| field.label.clone())
            .unwrap_or_else(|| "submit form".to_string());
        out.push(Form {
            method,
            action,
            action_absolute,
            fields,
            label,
        });
    }

    out
}

fn collect_continuation_buttons(doc: &Html) -> Vec<ContinuationButton> {
    let Some(selector) = parse_selector(
        "button[data-glance-continuation], [role=\"button\"][data-glance-continuation], input[type=\"button\"][data-glance-continuation], input[type=\"submit\"][data-glance-continuation]",
    ) else {
        return Vec::new();
    };
    doc.select(&selector)
        .filter_map(|node| {
            if node.value().attr("disabled").is_some()
                || node.value().attr("aria-disabled") == Some("true")
            {
                return None;
            }
            let glance_id = node
                .value()
                .attr("data-glance-continuation")?
                .parse::<usize>()
                .ok()?;
            let label = node
                .value()
                .attr("aria-label")
                .or_else(|| node.value().attr("title"))
                .or_else(|| node.value().attr("value"))
                .map(str::to_string)
                .unwrap_or_else(|| node.text().collect::<Vec<_>>().join(" "));
            let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
            (!label.is_empty()).then_some(ContinuationButton { label, glance_id })
        })
        .collect()
}

fn selected_option_value(select: &ElementRef<'_>) -> String {
    let Some(option_selector) = parse_selector("option") else {
        return String::new();
    };
    let options = select.select(&option_selector).collect::<Vec<_>>();
    let selected = options
        .iter()
        .find(|option| option.value().attr("selected").is_some())
        .or_else(|| options.first());
    selected
        .map(|option| {
            option
                .value()
                .attr("value")
                .map(ToString::to_string)
                .unwrap_or_else(|| collapsed_text(option))
        })
        .unwrap_or_default()
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
            kind: MediaKind::Image,
            mime_type: None,
        });
    }

    out
}

fn collect_media_links(doc: &Html, base_url: &Url) -> Vec<MediaRef> {
    let mut out = Vec::new();
    let mut seen = HashMap::<String, usize>::new();
    let Some(selector) = parse_selector(
        "audio[src], video[src], audio source[src], video source[src], iframe[src], a[href]",
    ) else {
        return out;
    };

    for node in doc.select(&selector) {
        let tag = node.value().name();
        let raw = match tag {
            "a" => node.value().attr("href"),
            _ => node.value().attr("src"),
        }
        .unwrap_or_default()
        .trim()
        .to_string();
        if raw.is_empty() {
            continue;
        }

        let owner = if tag == "source" {
            node.parent().and_then(ElementRef::wrap)
        } else {
            None
        };
        let owner_tag = owner.as_ref().map(|element| element.value().name());
        let mime_type = node
            .value()
            .attr("type")
            .map(str::trim)
            .filter(|mime| !mime.is_empty())
            .map(ToString::to_string);
        let absolute = base_url.join(&raw).ok();
        let kind = match tag {
            "audio" => Some(MediaKind::Audio),
            "video" => Some(MediaKind::Video),
            "source" if owner_tag == Some("audio") => Some(MediaKind::Audio),
            "source" if owner_tag == Some("video") => Some(MediaKind::Video),
            "iframe" if is_known_media_provider(absolute.as_ref()) => Some(MediaKind::Stream),
            "a" => classify_linked_media(&raw, mime_type.as_deref(), absolute.as_ref()),
            _ => None,
        };
        let Some(kind) = kind else {
            continue;
        };

        let identity = absolute
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| raw.clone());
        if let Some(existing_index) = seen.get(&identity).copied() {
            if out[existing_index].mime_type.is_none() {
                out[existing_index].mime_type = mime_type;
            }
            continue;
        }

        let mut label = node
            .value()
            .attr("aria-label")
            .or_else(|| node.value().attr("title"))
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| collapsed_text(&node));
        if label.is_empty()
            && let Some(owner) = owner.as_ref()
        {
            label = owner
                .value()
                .attr("aria-label")
                .or_else(|| owner.value().attr("title"))
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| collapsed_text(owner));
        }
        if label.is_empty() {
            label = fallback_label(base_url, absolute.as_ref(), &raw, kind.label());
        }

        seen.insert(identity, out.len());
        out.push(MediaRef {
            alt_or_label: label,
            src: raw,
            absolute,
            kind,
            mime_type,
        });
    }

    out
}

fn classify_linked_media(
    raw: &str,
    mime_type: Option<&str>,
    absolute: Option<&Url>,
) -> Option<MediaKind> {
    let mime = mime_type.unwrap_or_default().to_ascii_lowercase();
    if mime.starts_with("audio/") {
        return Some(MediaKind::Audio);
    }
    if mime.starts_with("video/") {
        return Some(MediaKind::Video);
    }
    if mime.contains("mpegurl") || mime.contains("dash+xml") {
        return Some(MediaKind::Stream);
    }
    if is_known_playable_page(absolute) {
        return Some(MediaKind::Stream);
    }

    let lower = raw.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or(lower.as_str());
    if [".m3u8", ".mpd"].iter().any(|ext| path.ends_with(ext)) {
        return Some(MediaKind::Stream);
    }
    if [".mp3", ".m4a", ".aac", ".flac", ".wav", ".oga", ".opus"]
        .iter()
        .any(|ext| path.ends_with(ext))
    {
        return Some(MediaKind::Audio);
    }
    if [".mp4", ".webm", ".mov", ".mkv", ".m4v", ".ogv"]
        .iter()
        .any(|ext| path.ends_with(ext))
    {
        return Some(MediaKind::Video);
    }
    if matches!(mime.as_str(), "application/ogg" | "application/x-matroska") {
        return Some(MediaKind::Media);
    }
    None
}

fn is_known_media_provider(url: Option<&Url>) -> bool {
    let Some(host) = url.and_then(Url::host_str) else {
        return false;
    };
    [
        "youtube.com",
        "youtu.be",
        "vimeo.com",
        "soundcloud.com",
        "twitch.tv",
    ]
    .iter()
    .any(|provider| host == *provider || host.ends_with(&format!(".{provider}")))
}

fn is_known_playable_page(url: Option<&Url>) -> bool {
    let Some(url) = url else {
        return false;
    };
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let path = url.path().trim_end_matches('/');

    if host == "youtu.be" || host.ends_with(".youtu.be") {
        return !path.is_empty();
    }
    if host == "youtube.com" || host.ends_with(".youtube.com") {
        return (path == "/watch"
            && url
                .query_pairs()
                .any(|(key, value)| key == "v" && !value.is_empty()))
            || ["/shorts/", "/live/", "/embed/"].iter().any(|prefix| {
                path.strip_prefix(prefix)
                    .is_some_and(|id| !id.trim_matches('/').is_empty())
            });
    }
    if host == "vimeo.com" || host.ends_with(".vimeo.com") {
        return path
            .trim_start_matches('/')
            .split('/')
            .any(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()));
    }
    if host == "soundcloud.com" || host.ends_with(".soundcloud.com") {
        return path
            .trim_matches('/')
            .split('/')
            .filter(|part| !part.is_empty())
            .count()
            >= 2;
    }
    if host == "twitch.tv" || host.ends_with(".twitch.tv") {
        return path.starts_with("/videos/") || path.starts_with("/clip/");
    }
    false
}

fn visible_text(root: &ElementRef<'_>) -> String {
    root.descendants()
        .filter_map(|node| {
            let text = node.value().as_text()?;
            let hidden = node
                .ancestors()
                .filter_map(ElementRef::wrap)
                .any(|element| {
                    matches!(
                        element.value().name(),
                        "script" | "style" | "template" | "noscript" | "svg"
                    ) || element.value().attr("hidden").is_some()
                        || element.value().attr("aria-hidden") == Some("true")
                });
            (!hidden).then_some(text.as_ref())
        })
        .collect::<Vec<_>>()
        .join(" ")
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
                let mut label = accessible_label(&node);
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
    visible_text(node)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn accessible_label(node: &ElementRef<'_>) -> String {
    if let Some(label) = node
        .value()
        .attr("aria-label")
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        return label.to_string();
    }

    let text = collapsed_text(node);
    if !text.is_empty() {
        return text;
    }

    if let Some(label) = node
        .value()
        .attr("title")
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        return label.to_string();
    }

    let Some(image_selector) = parse_selector("img[alt]") else {
        return String::new();
    };
    node.select(&image_selector)
        .filter_map(|image| image.value().attr("alt"))
        .map(str::trim)
        .find(|label| !label.is_empty())
        .unwrap_or_default()
        .to_string()
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
            "expires" if is_immediate_expires(content) => {
                out.cache_hints.expires_immediate = true;
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
    use super::{MarkdownBlock, MediaKind, parse_document};
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
    fn fallback_text_excludes_script_style_and_hidden_payloads() {
        let html = r#"
            <html><body>
              <div>Readable home page</div>
              <script>window.__DATA__ = "very noisy script payload";</script>
              <style>.secret { display: none }</style>
              <div hidden>Hidden copy</div>
              <div aria-hidden="true">Decorative copy</div>
            </body></html>
        "#;
        let base = Url::parse("https://example.com").expect("url");
        let doc = parse_document(&base, html);

        assert_eq!(doc.text_lines, vec!["Readable home page"]);
    }

    #[test]
    fn semantic_text_nodes_exclude_nested_script_payloads() {
        let html = r#"
            <html><body>
              <p>Readable summary<script>window.__DATA__ = "noise";</script></p>
            </body></html>
        "#;
        let base = Url::parse("https://example.com").expect("url");
        let doc = parse_document(&base, html);

        assert_eq!(doc.text_lines, vec!["Readable summary"]);
        assert!(doc.markdown_blocks.iter().any(
            |block| matches!(block, MarkdownBlock::Paragraph { text } if text == "Readable summary")
        ));
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

    #[test]
    fn extracts_search_form_fields_and_defaults() {
        let html = r#"
            <form method="get" action="/search">
              <input type="hidden" name="lang" value="ko">
              <input type="search" name="q" placeholder="Search docs" required>
              <select name="sort">
                <option value="new">Newest</option>
                <option value="best" selected>Best</option>
              </select>
            </form>
        "#;
        let base = Url::parse("https://example.com/docs").expect("url");
        let doc = parse_document(&base, html);
        let form = &doc.forms[0];

        assert_eq!(form.method, "GET");
        assert_eq!(
            form.action_absolute.as_ref().map(Url::as_str),
            Some("https://example.com/search")
        );
        assert_eq!(form.label, "Search docs");
        assert_eq!(form.fields.len(), 3);
        assert_eq!(form.fields[2].value, "best");
        assert!(form.fields[1].editable);
        assert!(form.fields[1].required);
    }

    #[test]
    fn extracts_native_and_streaming_media_without_duplicates() {
        let html = r#"
            <video aria-label="Trailer" src="/trailer.mp4">
              <source src="/trailer.mp4" type="video/mp4">
              <source src="/trailer.webm" type="video/webm">
            </video>
            <audio title="Podcast"><source src="/episode.opus" type="audio/opus"></audio>
            <a href="/live/master.m3u8">Live channel</a>
            <a href="/asset" type="audio/flac">Lossless</a>
            <iframe title="Talk" src="https://www.youtube.com/embed/abc"></iframe>
        "#;
        let base = Url::parse("https://example.com/watch").expect("url");
        let doc = parse_document(&base, html);

        assert_eq!(doc.media_links.len(), 6);
        assert_eq!(doc.media_links[0].kind, MediaKind::Video);
        assert_eq!(doc.media_links[0].alt_or_label, "Trailer");
        assert_eq!(doc.media_links[0].mime_type.as_deref(), Some("video/mp4"));
        assert_eq!(doc.media_links[1].mime_type.as_deref(), Some("video/webm"));
        assert_eq!(doc.media_links[2].kind, MediaKind::Audio);
        assert_eq!(doc.media_links[2].alt_or_label, "Podcast");
        assert_eq!(doc.media_links[3].kind, MediaKind::Stream);
        assert_eq!(doc.media_links[4].kind, MediaKind::Audio);
        assert_eq!(doc.media_links[5].kind, MediaKind::Stream);
        assert_eq!(doc.media_links[5].alt_or_label, "Talk");
    }

    #[test]
    fn only_playable_provider_links_are_classified_as_media() {
        let html = r#"
            <a href="https://www.youtube.com/">YouTube home</a>
            <a href="https://www.youtube.com/about/">About YouTube</a>
            <a href="https://www.youtube.com/watch?v=abc">A video</a>
            <a href="https://youtu.be/xyz">A short URL</a>
            <a href="https://www.youtube.com/shorts/def">A Short</a>
        "#;
        let base = Url::parse("https://www.youtube.com/").expect("url");
        let doc = parse_document(&base, html);

        assert_eq!(doc.media_links.len(), 3);
        assert!(
            doc.media_links
                .iter()
                .all(|item| item.kind == MediaKind::Stream)
        );
        assert!(
            doc.media_links
                .iter()
                .any(|item| item.src.contains("watch?v=abc"))
        );
        assert!(
            doc.media_links
                .iter()
                .all(|item| item.alt_or_label != "YouTube home")
        );
        assert!(
            doc.media_links
                .iter()
                .all(|item| item.alt_or_label != "About YouTube")
        );
    }

    #[test]
    fn uses_accessible_name_for_icon_only_link() {
        let html = r#"<a href="/sign" aria-label="Log in"><svg></svg></a>"#;
        let doc = parse_document(&Url::parse("https://example.com/").expect("url"), html);
        assert_eq!(doc.links[0].text, "Log in");
        assert!(doc.markdown_blocks.iter().any(|block| matches!(
            block,
            MarkdownBlock::LinkLine { label, .. } if label == "Log in"
        )));
    }

    #[test]
    fn extracts_only_browser_tagged_continuation_controls() {
        let html = r#"
            <button data-glance-continuation="3">Load more posts</button>
            <button>Delete account</button>
            <a href="/delete" data-glance-continuation="0">Load more</a>
            <div role="button" data-glance-continuation="4" aria-label="더 보기"></div>
        "#;
        let doc = parse_document(&Url::parse("https://example.com/").expect("url"), html);
        assert_eq!(doc.continuation_buttons.len(), 2);
        assert_eq!(doc.continuation_buttons[0].glance_id, 3);
        assert_eq!(doc.continuation_buttons[0].label, "Load more posts");
        assert_eq!(doc.continuation_buttons[1].label, "더 보기");
    }
}
