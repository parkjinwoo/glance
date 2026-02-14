use anyhow::{Context, Result};
use tokio::runtime::Runtime;
use url::Url;

use glance_config::Config;
use glance_dom::parse_document_with_options;
use glance_net::{NetClient, normalize_url};
use glance_ui::UiPage;

use crate::browser::Browser;

#[derive(Debug, Clone)]
pub(crate) enum InitialTarget {
    Blank,
    Url(Url),
}

pub(crate) fn build_runtime_and_net(config: &Config) -> Result<(Runtime, NetClient)> {
    let runtime = Runtime::new().context("failed to create tokio runtime")?;
    let net = NetClient::new(
        &config.network.user_agent,
        config.network.timeout_ms,
        config.network.max_redirects,
    )?;
    Ok((runtime, net))
}

pub(crate) fn run_dump_dom(
    runtime: &Runtime,
    net: &NetClient,
    dump_url: String,
    max_body_bytes: usize,
    sanitize_control_chars: bool,
) -> Result<()> {
    let url = normalize_url(&dump_url)?;
    let fetch = runtime.block_on(net.fetch_limited(&url, max_body_bytes))?;
    let doc = parse_document_with_options(&fetch.final_url, &fetch.body, sanitize_control_chars);
    println!("title: {}", doc.title);
    println!("url: {}", doc.source_url);
    println!("scripts: {}", doc.script_count);
    println!("--- text ---");
    for line in doc.text_lines.iter().take(100) {
        println!("{line}");
    }
    println!("--- links ---");
    for (i, link) in doc.links.iter().enumerate() {
        println!("{}: {} -> {}", i + 1, link.text, link.href);
    }
    Ok(())
}

pub(crate) fn resolve_initial_target(
    cli_url: Option<String>,
    startup_home: &str,
) -> Result<InitialTarget> {
    let raw = cli_url.unwrap_or_else(|| startup_home.to_string());
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("about:blank") {
        return Ok(InitialTarget::Blank);
    }
    normalize_url(trimmed).map(InitialTarget::Url)
}

pub(crate) fn build_initial_page(
    browser: &mut Browser,
    target: InitialTarget,
    private: bool,
) -> Result<UiPage> {
    let private_msg = if private {
        "private mode requested (cookie persistence is disabled in MVP by default)"
    } else {
        "session mode"
    };

    let mut first = match target {
        InitialTarget::Blank => browser.to_ui_page(),
        InitialTarget::Url(url) => browser.load_initial(url)?,
    };
    first.status = private_msg.to_string();
    Ok(first)
}
