use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::timeout;
use url::Url;

use glance_dom::{Document, MarkdownBlock, parse_document_with_options};
use glance_render::{RenderIconMode, RenderedPage, render_document};

use crate::policy::DynamicPolicy;

pub(crate) struct DynamicPage {
    pub(crate) document: Document,
    pub(crate) rendered: RenderedPage,
}

pub(crate) fn resolve_browser_executable(configured: Option<&str>) -> Option<String> {
    if let Some(configured) = configured.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(configured.to_string());
    }

    absolute_browser_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(|| {
            path_browser_candidates()
                .into_iter()
                .find(|candidate| executable_on_path(candidate))
                .map(ToString::to_string)
        })
}

pub(crate) fn looks_like_client_rendered_shell(document: &Document, html: &str) -> bool {
    if document.script_count == 0 {
        return false;
    }

    let visible_chars = document
        .markdown_blocks
        .iter()
        .map(|block| match block {
            MarkdownBlock::Heading { text, .. }
            | MarkdownBlock::Paragraph { text }
            | MarkdownBlock::ListItem { text, .. }
            | MarkdownBlock::Quote { text } => text.chars().count(),
            MarkdownBlock::CodeBlock { code } => code.chars().count(),
            MarkdownBlock::LinkLine { label, .. } | MarkdownBlock::Image { label, .. } => {
                label.chars().count()
            }
            MarkdownBlock::Table { headers, rows } => headers
                .iter()
                .chain(rows.iter().flatten())
                .map(|cell| cell.chars().count())
                .sum(),
            MarkdownBlock::HorizontalRule => 0,
        })
        .sum::<usize>();
    let framework_marker = [
        "data-hk=",
        "__NEXT_DATA__",
        "__NUXT__",
        "data-reactroot",
        "id=\"__next\"",
        "id=\"root\"",
        "id=\"app\"",
        "ng-version=",
        "ytInitialData",
        "<ytd-app",
    ]
    .iter()
    .any(|marker| html.contains(marker));
    let known_partial_shell = ["window[\"EAGER-DATA\"]", "ytInitialData", "<ytd-app"]
        .iter()
        .any(|marker| html.contains(marker));

    known_partial_shell
        || (framework_marker && visible_chars < 4_000 && document.links.len() < 100)
        || (visible_chars < 160 && html.len() > 8_000)
}

pub(crate) async fn render_with_browser(
    executable: String,
    url: Url,
    policy: DynamicPolicy,
    render_icon_mode: RenderIconMode,
    max_body_bytes: usize,
    sanitize_control_chars: bool,
    request_id: u64,
) -> Result<DynamicPage> {
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "dynamic rendering only supports HTTP(S) URLs"
    );

    let profile_prefix = format!("glance-chromium-{}-{request_id}-", std::process::id());
    let profile_dir = tempfile::Builder::new()
        .prefix(&profile_prefix)
        .tempdir()
        .context("failed to create an isolated browser profile")?;

    run_browser(
        &executable,
        &url,
        profile_dir.path(),
        &policy,
        max_body_bytes,
    )
    .await
    .map(|html| {
        let mut document = parse_document_with_options(&url, &html, sanitize_control_chars);
        // Interactive continuation tags are trusted only when our connected companion
        // has cleared and regenerated them immediately before the snapshot.
        document.continuation_buttons.clear();
        document.live_controls.clear();
        let rendered = render_document(&document, render_icon_mode);
        DynamicPage { document, rendered }
    })
}

async fn run_browser(
    executable: &str,
    url: &Url,
    profile_dir: &Path,
    policy: &DynamicPolicy,
    max_body_bytes: usize,
) -> Result<String> {
    let mut command = Command::new(executable);
    command
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-sync")
        .arg("--disable-extensions")
        .arg("--disable-component-update")
        .arg("--disable-default-apps")
        .arg("--mute-audio")
        .arg("--window-size=1440,1000")
        .arg("--force-device-scale-factor=1")
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg(format!("--virtual-time-budget={}", policy.wait_ms))
        .arg("--dump-dom")
        .arg(url.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start browser executable '{executable}'"))?;
    let mut stdout = child
        .stdout
        .take()
        .context("browser stdout was unavailable")?;
    let read_result = match timeout(
        Duration::from_millis(policy.timeout_ms),
        read_html_snapshot(&mut stdout, max_body_bytes),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("dynamic rendering timed out")),
    };

    // Chrome helpers on some platforms inherit the output pipe and outlive the main
    // headless process. The serialized DOM is complete at </html>, so stop the isolated
    // process tree promptly instead of waiting for unrelated updater helpers.
    let _ = child.kill().await;
    let _ = child.wait().await;

    let output = read_result?;
    let stdout = String::from_utf8_lossy(&output);
    extract_dumped_html(&stdout)
        .map(ToString::to_string)
        .context("browser returned no HTML document")
}

async fn read_html_snapshot(
    reader: &mut (impl AsyncRead + Unpin),
    max_body_bytes: usize,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        ensure!(
            output.len().saturating_add(read) <= max_body_bytes,
            "hydrated DOM exceeds configured network.max_body_bytes ({max_body_bytes} bytes)"
        );
        output.extend_from_slice(&chunk[..read]);
        if output
            .windows(b"</html>".len())
            .any(|window| window.eq_ignore_ascii_case(b"</html>"))
        {
            break;
        }
    }
    Ok(output)
}

fn extract_dumped_html(output: &str) -> Option<&str> {
    let start = output
        .find("<!DOCTYPE")
        .or_else(|| output.find("<!doctype"))
        .or_else(|| output.find("<html"))?;
    let html = output[start..].trim();
    (!html.is_empty()).then_some(html)
}

fn executable_on_path(candidate: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let base = dir.join(candidate);
        base.is_file() || cfg!(windows) && base.with_extension("exe").is_file()
    })
}

fn absolute_browser_candidates() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
    ];

    for variable in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(variable) {
            let base = PathBuf::from(base);
            paths.push(base.join("Google/Chrome/Application/chrome.exe"));
            paths.push(base.join("Microsoft/Edge/Application/msedge.exe"));
        }
    }
    paths
}

fn path_browser_candidates() -> [&'static str; 7] {
    [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "msedge",
        "microsoft-edge",
    ]
}

#[cfg(test)]
mod tests {
    use super::{extract_dumped_html, looks_like_client_rendered_shell};
    use glance_dom::parse_document;
    use url::Url;

    #[test]
    fn detects_sparse_hydration_shell() {
        let url = Url::parse("https://example.com").expect("url");
        let html = format!(
            r#"<html><body><div id="root" data-hk="0"><p>Loading</p></div><script>{}</script></body></html>"#,
            "x".repeat(9_000)
        );
        let document = parse_document(&url, &html);
        assert!(looks_like_client_rendered_shell(&document, &html));
    }

    #[test]
    fn leaves_content_rich_server_page_static() {
        let url = Url::parse("https://example.com").expect("url");
        let paragraphs = "<p>Substantial readable server content.</p>".repeat(200);
        let html = format!(
            r#"<html><body><main id="root">{paragraphs}</main><script>hydrate()</script></body></html>"#
        );
        let document = parse_document(&url, &html);
        assert!(!looks_like_client_rendered_shell(&document, &html));
    }

    #[test]
    fn detects_partially_serialized_youtube_shell() {
        let url = Url::parse("https://www.youtube.com/").expect("url");
        let html = format!(
            r#"<html><body><ytd-app><a href="/">YouTube</a></ytd-app><script>var ytInitialData = {{}};{}</script></body></html>"#,
            "x".repeat(9_000)
        );
        let document = parse_document(&url, &html);

        assert!(looks_like_client_rendered_shell(&document, &html));
    }

    #[test]
    fn detects_naver_eager_data_shell_even_with_many_server_links() {
        let url = Url::parse("https://www.naver.com/").expect("url");
        let links = (0..150)
            .map(|index| format!(r#"<a href="/item/{index}">Item {index}</a>"#))
            .collect::<String>();
        let html = format!(
            r#"<html><body>{links}<script>window["EAGER-DATA"] = {{}};</script></body></html>"#
        );
        let document = parse_document(&url, &html);

        assert!(looks_like_client_rendered_shell(&document, &html));
    }

    #[test]
    fn extracts_html_after_non_html_noise() {
        assert_eq!(
            extract_dumped_html("warning\n<!DOCTYPE html><html></html>"),
            Some("<!DOCTYPE html><html></html>")
        );
        assert_eq!(extract_dumped_html("warning only"), None);
    }
}
