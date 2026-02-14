use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use anyhow::{Context, Result};
use tokio::runtime::Runtime;
use url::Url;

use glance_config::UiIconMode as ConfigUiIconMode;
use glance_dom::{Document, parse_document_with_options};
use glance_js::{JsPolicy, evaluate_document_scripts};
use glance_media::{
    ImageBackend, detect_image_backend, image_dimensions, launch_external_player_with_options,
};
use glance_net::{FetchResult, NetClient, normalize_url};
use glance_render::{RenderIconMode, RenderedPage, render_document};
use glance_ui::{UiCommand, UiIconMode, UiImageItem, UiImageMode, UiPage};

use crate::policy::{MediaPolicy, NetworkPolicy, is_about_blank, validate_url_for_network};

#[derive(Debug, Clone)]
struct LoadedPage {
    url: Url,
    rendered: RenderedPage,
    document: Document,
    js_note: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SessionImageMode {
    Auto,
    TextForced,
}

#[derive(Debug, Clone, Copy)]
enum HistoryPlan {
    Push,
    SetCursor(usize),
    None,
}

#[derive(Debug, Clone, Copy)]
struct PendingNavigation {
    request_id: u64,
    plan: HistoryPlan,
}

#[derive(Debug, Clone, Copy)]
struct PendingImage {
    request_id: u64,
    index: usize,
}

enum AsyncResult {
    Navigation {
        request_id: u64,
        result: Box<std::result::Result<NavigationOutcome, String>>,
    },
    Image {
        request_id: u64,
        index: usize,
        result: std::result::Result<Vec<u8>, String>,
    },
}

struct NavigationOutcome {
    loaded: LoadedPage,
    status: String,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    id: u64,
    url: Url,
}

pub(crate) struct Browser {
    runtime: Runtime,
    net: NetClient,
    js_policy: JsPolicy,
    media: MediaPolicy,
    image_backend: ImageBackend,
    image_mode: SessionImageMode,
    icon_mode: UiIconMode,
    render_icon_mode: RenderIconMode,
    network_policy: NetworkPolicy,
    selected_image_index: Option<usize>,
    selected_image_bytes: Option<Vec<u8>>,
    selected_image_size: Option<(u32, u32)>,
    history: Vec<HistoryEntry>,
    cursor: usize,
    next_history_entry_id: u64,
    current: Option<LoadedPage>,
    last_status: String,
    next_request_id: u64,
    results_tx: Sender<AsyncResult>,
    results_rx: Receiver<AsyncResult>,
    inflight_nav: Option<PendingNavigation>,
    inflight_image: Option<PendingImage>,
}

const META_REFRESH_HOP_LIMIT: usize = 3;

impl Browser {
    pub(crate) fn new(
        runtime: Runtime,
        net: NetClient,
        js_policy: JsPolicy,
        icon_mode: ConfigUiIconMode,
        media: MediaPolicy,
        network_policy: NetworkPolicy,
    ) -> Self {
        let image_backend = detect_image_backend();
        let (results_tx, results_rx) = mpsc::channel();
        let icon_mode_ui = map_ui_icon_mode(icon_mode);
        let render_icon_mode = map_render_icon_mode(icon_mode);

        Self {
            runtime,
            net,
            js_policy,
            media,
            image_backend,
            image_mode: SessionImageMode::Auto,
            icon_mode: icon_mode_ui,
            render_icon_mode,
            network_policy,
            selected_image_index: None,
            selected_image_bytes: None,
            selected_image_size: None,
            history: Vec::new(),
            cursor: 0,
            next_history_entry_id: 1,
            current: None,
            last_status: "ready".to_string(),
            next_request_id: 1,
            results_tx,
            results_rx,
            inflight_nav: None,
            inflight_image: None,
        }
    }

    pub(crate) fn load_initial(&mut self, url: Url) -> Result<UiPage> {
        let outcome = self.runtime.block_on(fetch_and_build_async(
            self.net.clone(),
            self.js_policy.clone(),
            self.render_icon_mode,
            url,
            self.network_policy.clone(),
        ))?;

        self.history = vec![HistoryEntry {
            id: self.next_history_entry_id(),
            url: outcome.loaded.url.clone(),
        }];
        self.cursor = 0;
        self.apply_loaded_page(outcome.loaded, outcome.status);

        Ok(self.to_ui_page())
    }

    pub(crate) fn handle_command(&mut self, cmd: UiCommand) -> UiPage {
        self.drain_async_results();

        let result = match cmd {
            UiCommand::Poll => Ok(()),
            UiCommand::OpenUrl(raw) => self.open_url_string(&raw),
            UiCommand::OpenLink(idx) => self.open_link(idx),
            UiCommand::OpenMedia(idx) => self.open_media(idx),
            UiCommand::SelectImage(idx) => self.select_image(idx),
            UiCommand::Reload => self.reload(),
            UiCommand::Back => self.back(),
            UiCommand::Forward => self.forward(),
            UiCommand::ToggleJsDefault => self.toggle_js_default(),
            UiCommand::ToggleImageMode => self.toggle_image_mode(),
        };

        if let Err(err) = result {
            self.last_status = format!("error: {err:#}");
        }

        self.to_ui_page()
    }

    pub(crate) fn to_ui_page(&self) -> UiPage {
        let status = self.composed_status();

        if let Some(current) = &self.current {
            let images = build_image_items(&current.document);
            let media = build_media_items(&current.document);
            let selected_image_index = self
                .selected_image_index
                .and_then(|i| if i < images.len() { Some(i) } else { None });

            let (selected_image_bytes, selected_image_size) =
                if self.image_mode == SessionImageMode::Auto {
                    (self.selected_image_bytes.clone(), self.selected_image_size)
                } else {
                    (None, None)
                };

            UiPage {
                history_entry_id: self.current_history_entry_id().unwrap_or(0),
                rendered: current.rendered.clone(),
                status,
                js_note: current.js_note.clone(),
                icon_mode: self.icon_mode,
                image_backend: self.image_backend,
                image_mode: match self.image_mode {
                    SessionImageMode::Auto => UiImageMode::Auto,
                    SessionImageMode::TextForced => UiImageMode::Text,
                },
                js_default_enabled: self.js_policy.default_enabled(),
                images,
                media,
                selected_image_index,
                selected_image_bytes,
                selected_image_size,
            }
        } else {
            UiPage {
                history_entry_id: self.current_history_entry_id().unwrap_or(0),
                rendered: RenderedPage {
                    title: "glance".to_string(),
                    source_url: "".to_string(),
                    lines: vec!["No page loaded".to_string()],
                    links: vec![],
                    media: vec![],
                    interactive_spans: vec![],
                },
                status,
                js_note: "js: off".to_string(),
                icon_mode: self.icon_mode,
                image_backend: self.image_backend,
                image_mode: UiImageMode::Auto,
                js_default_enabled: self.js_policy.default_enabled(),
                images: vec![],
                media: vec![],
                selected_image_index: None,
                selected_image_bytes: None,
                selected_image_size: None,
            }
        }
    }

    fn open_url_string(&mut self, raw: &str) -> Result<()> {
        let url = normalize_url(raw)?;
        self.open_url(url, HistoryPlan::Push)
    }

    fn open_url(&mut self, url: Url, plan: HistoryPlan) -> Result<()> {
        if is_about_blank(&url) {
            self.navigate_blank(plan, url);
            return Ok(());
        }
        self.validate_url_for_network(&url)?;

        let request_id = self.next_request_id();
        self.inflight_nav = Some(PendingNavigation { request_id, plan });
        self.clear_selected_image_data();

        let tx = self.results_tx.clone();
        let net = self.net.clone();
        let js_policy = self.js_policy.clone();
        let render_icon_mode = self.render_icon_mode;
        let network_policy = self.network_policy.clone();
        let status_url = url.to_string();
        self.runtime.spawn(async move {
            let result =
                fetch_and_build_async(net, js_policy, render_icon_mode, url, network_policy)
                    .await
                    .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncResult::Navigation {
                request_id,
                result: Box::new(result),
            });
        });

        self.last_status = format!("loading {status_url}");
        Ok(())
    }

    fn open_link(&mut self, idx: usize) -> Result<()> {
        let current = self.current.as_ref().context("no current page")?;
        let link = current
            .document
            .links
            .get(idx)
            .with_context(|| format!("invalid link index: {}", idx + 1))?;
        let url =
            resolve_optional_url(&link.absolute, &link.href).context("link has no valid URL")?;
        self.open_url(url, HistoryPlan::Push)
    }

    fn open_media(&mut self, idx: usize) -> Result<()> {
        let current = self.current.as_ref().context("no current page")?;
        let media = current
            .document
            .media_links
            .get(idx)
            .with_context(|| format!("invalid media index: {}", idx + 1))?;
        let media_url = resolve_optional_url(&media.absolute, &media.src)
            .context("media link has no valid URL")?;
        self.validate_url_for_network(&media_url)?;

        launch_external_player_with_options(
            &self.media.player,
            &media_url,
            self.media.mode,
            &self.media.allowed_players,
        )
        .map_err(|e| anyhow::anyhow!(e))?;
        self.last_status = format!("opened media {}", media_url);
        Ok(())
    }

    fn select_image(&mut self, idx: usize) -> Result<()> {
        let current = self.current.as_ref().context("no current page")?;
        let image = current
            .document
            .images
            .get(idx)
            .with_context(|| format!("invalid image index: {}", idx + 1))?;
        let label = image.alt_or_label.clone();
        self.selected_image_index = Some(idx);
        self.clear_selected_image_data();
        self.queue_selected_image_fetch();
        self.last_status = format!("selected image {} ({})", idx + 1, label);
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        let current_url = self
            .current
            .as_ref()
            .map(|p| p.url.clone())
            .context("nothing to reload")?;
        self.open_url(current_url, HistoryPlan::None)
    }

    fn back(&mut self) -> Result<()> {
        if self.cursor == 0 || self.history.is_empty() {
            self.last_status = "already at oldest history".to_string();
            return Ok(());
        }
        let target_cursor = self.cursor - 1;
        let target = self
            .history
            .get(target_cursor)
            .map(|e| e.url.clone())
            .context("history out of bounds")?;
        self.open_url(target, HistoryPlan::SetCursor(target_cursor))
    }

    fn forward(&mut self) -> Result<()> {
        if self.history.is_empty() || self.cursor + 1 >= self.history.len() {
            self.last_status = "already at newest history".to_string();
            return Ok(());
        }
        let target_cursor = self.cursor + 1;
        let target = self
            .history
            .get(target_cursor)
            .map(|e| e.url.clone())
            .context("history out of bounds")?;
        self.open_url(target, HistoryPlan::SetCursor(target_cursor))
    }

    fn toggle_js_default(&mut self) -> Result<()> {
        self.js_policy.toggle_default();
        let enabled = self.js_policy.default_enabled();
        self.last_status = format!("js default {}", if enabled { "on" } else { "off" });

        if let Some(current_url) = self.current.as_ref().map(|p| p.url.clone()) {
            self.open_url(current_url, HistoryPlan::None)?;
        }

        Ok(())
    }

    fn toggle_image_mode(&mut self) -> Result<()> {
        self.image_mode = match self.image_mode {
            SessionImageMode::Auto => SessionImageMode::TextForced,
            SessionImageMode::TextForced => SessionImageMode::Auto,
        };

        match self.image_mode {
            SessionImageMode::TextForced => {
                self.inflight_image = None;
                self.clear_selected_image_data();
            }
            SessionImageMode::Auto => {
                if self.selected_image_index.is_none() {
                    self.selected_image_index = self.default_selected_image_index_for_current();
                }
                self.queue_selected_image_fetch();
            }
        }

        self.last_status = format!(
            "image mode {}",
            match self.image_mode {
                SessionImageMode::Auto => "auto",
                SessionImageMode::TextForced => "text",
            }
        );
        Ok(())
    }

    fn drain_async_results(&mut self) {
        loop {
            match self.results_rx.try_recv() {
                Ok(AsyncResult::Navigation { request_id, result }) => {
                    if self
                        .inflight_nav
                        .as_ref()
                        .is_none_or(|pending| pending.request_id != request_id)
                    {
                        continue;
                    }

                    let plan = self
                        .inflight_nav
                        .take()
                        .map(|p| p.plan)
                        .unwrap_or(HistoryPlan::None);
                    match *result {
                        Ok(outcome) => {
                            self.apply_history_plan(plan, outcome.loaded.url.clone());
                            self.apply_loaded_page(outcome.loaded, outcome.status);
                        }
                        Err(err) => {
                            self.last_status = format!("error: {err}");
                        }
                    }
                }
                Ok(AsyncResult::Image {
                    request_id,
                    index,
                    result,
                }) => {
                    let Some(pending) = self.inflight_image.as_ref() else {
                        continue;
                    };
                    if pending.request_id != request_id || pending.index != index {
                        continue;
                    }
                    self.inflight_image = None;

                    if self.selected_image_index != Some(index)
                        || self.image_mode != SessionImageMode::Auto
                    {
                        continue;
                    }

                    match result {
                        Ok(bytes) => {
                            self.selected_image_size = image_dimensions(&bytes);
                            self.selected_image_bytes = Some(bytes);
                        }
                        Err(err) => {
                            self.clear_selected_image_data();
                            self.last_status = format!("image fetch error: {err}");
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn queue_selected_image_fetch(&mut self) {
        if self.image_mode != SessionImageMode::Auto {
            return;
        }

        let Some(current) = &self.current else {
            return;
        };
        let Some(index) = self.selected_image_index else {
            return;
        };
        let Some(image) = current.document.images.get(index) else {
            return;
        };

        let Some(url) = resolve_optional_url(&image.absolute, &image.src) else {
            return;
        };
        if let Err(err) = self.validate_url_for_network(&url) {
            self.last_status = format!("image blocked: {err}");
            return;
        }

        let request_id = self.next_request_id();
        self.inflight_image = Some(PendingImage { request_id, index });
        self.clear_selected_image_data();

        let tx = self.results_tx.clone();
        let net = self.net.clone();
        let max_image_bytes = self.network_policy.max_image_bytes;
        self.runtime.spawn(async move {
            let result = net
                .fetch_bytes_limited(&url, max_image_bytes)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncResult::Image {
                request_id,
                index,
                result,
            });
        });
    }

    fn apply_history_plan(&mut self, plan: HistoryPlan, final_url: Url) {
        match plan {
            HistoryPlan::Push => {
                if self.cursor + 1 < self.history.len() {
                    self.history.truncate(self.cursor + 1);
                }
                let entry_id = self.next_history_entry_id();
                self.history.push(HistoryEntry {
                    id: entry_id,
                    url: final_url,
                });
                self.cursor = self.history.len().saturating_sub(1);
            }
            HistoryPlan::SetCursor(idx) => {
                if idx < self.history.len() {
                    self.cursor = idx;
                    self.history[idx].url = final_url;
                } else if self.history.is_empty() {
                    let entry_id = self.next_history_entry_id();
                    self.history.push(HistoryEntry {
                        id: entry_id,
                        url: final_url,
                    });
                    self.cursor = 0;
                }
            }
            HistoryPlan::None => {
                if self.history.is_empty() {
                    let entry_id = self.next_history_entry_id();
                    self.history.push(HistoryEntry {
                        id: entry_id,
                        url: final_url,
                    });
                    self.cursor = 0;
                }
            }
        }
    }

    fn current_history_entry_id(&self) -> Option<u64> {
        self.history.get(self.cursor).map(|e| e.id)
    }

    fn default_selected_image_index_for_current(&self) -> Option<usize> {
        self.current.as_ref().and_then(|p| {
            if p.document.images.is_empty() {
                None
            } else {
                Some(0)
            }
        })
    }

    fn composed_status(&self) -> String {
        let mut parts = Vec::new();
        if self.inflight_nav.is_some() {
            parts.push("LOADING".to_string());
        }
        if self.inflight_image.is_some() {
            parts.push("IMG-FETCH".to_string());
        }
        parts.push(self.last_status.clone());
        parts.join(" | ")
    }

    fn next_request_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        id
    }

    fn next_history_entry_id(&mut self) -> u64 {
        let id = self.next_history_entry_id;
        self.next_history_entry_id = self.next_history_entry_id.saturating_add(1);
        id
    }

    fn navigate_blank(&mut self, plan: HistoryPlan, url: Url) {
        self.apply_history_plan(plan, url.clone());
        self.apply_loaded_page(blank_loaded_page(url), "loaded blank page".to_string());
        self.inflight_image = None;
        self.inflight_nav = None;
    }

    fn validate_url_for_network(&self, url: &Url) -> Result<()> {
        validate_url_for_network(
            url,
            &self.network_policy.allowed_schemes,
            self.network_policy.allow_private_ip,
        )
    }

    fn apply_loaded_page(&mut self, loaded: LoadedPage, status: String) {
        self.current = Some(loaded);
        self.selected_image_index = self.default_selected_image_index_for_current();
        self.clear_selected_image_data();
        self.last_status = status;
        self.queue_selected_image_fetch();
    }

    fn clear_selected_image_data(&mut self) {
        self.selected_image_bytes = None;
        self.selected_image_size = None;
    }
}

fn build_image_items(doc: &Document) -> Vec<UiImageItem> {
    doc.images
        .iter()
        .map(|img| UiImageItem {
            label: img.alt_or_label.clone(),
            url: img
                .absolute
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| img.src.clone()),
        })
        .collect()
}

fn build_media_items(doc: &Document) -> Vec<UiImageItem> {
    doc.media_links
        .iter()
        .map(|m| UiImageItem {
            label: m.alt_or_label.clone(),
            url: m
                .absolute
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| m.src.clone()),
        })
        .collect()
}

async fn fetch_and_build_async(
    net: NetClient,
    js_policy: JsPolicy,
    render_icon_mode: RenderIconMode,
    url: Url,
    network_policy: NetworkPolicy,
) -> Result<NavigationOutcome> {
    let mut current_url = url;
    let mut meta_refresh_hops = 0usize;

    loop {
        validate_url_for_network(
            &current_url,
            &network_policy.allowed_schemes,
            network_policy.allow_private_ip,
        )?;
        let fetch = net
            .fetch_limited(&current_url, network_policy.max_body_bytes)
            .await?;
        let parse_as_html = should_parse_as_html(fetch.status, fetch.content_type.as_deref());

        if parse_as_html {
            let document = parse_document_with_options(
                &fetch.final_url,
                &fetch.body,
                network_policy.sanitize_control_chars,
            );
            if let Some(refresh) = document.meta_http_equiv.refresh.as_ref()
                && refresh.delay_secs == 0
                && meta_refresh_hops < META_REFRESH_HOP_LIMIT
                && let Some(target) = refresh.target_absolute.clone()
                && target != fetch.final_url
            {
                current_url = target;
                meta_refresh_hops = meta_refresh_hops.saturating_add(1);
                continue;
            }

            let js_enabled = js_policy.is_enabled_for(&fetch.final_url);
            let report = evaluate_document_scripts(js_enabled, document.script_count);
            let mut rendered = render_document(&document, render_icon_mode);
            prepend_fetch_details(&mut rendered, &fetch, &document);

            let loaded = LoadedPage {
                url: fetch.final_url.clone(),
                rendered,
                document,
                js_note: format!(
                    "{} scripts: {}, policy: {}",
                    report.note,
                    report.script_count,
                    if report.enabled { "on" } else { "off" }
                ),
            };

            return Ok(NavigationOutcome {
                loaded,
                status: build_loaded_status(&fetch, meta_refresh_hops, "loaded"),
            });
        }

        let rendered = render_non_html_page(&fetch);
        let document = empty_document(fetch.final_url.clone(), "Non-HTML response");
        let loaded = LoadedPage {
            url: fetch.final_url.clone(),
            rendered,
            document,
            js_note: "js skipped for non-html response".to_string(),
        };
        return Ok(NavigationOutcome {
            loaded,
            status: build_loaded_status(&fetch, meta_refresh_hops, "loaded (non-html)"),
        });
    }
}

fn should_parse_as_html(status: u16, content_type: Option<&str>) -> bool {
    if status == 204 || status == 304 {
        return false;
    }
    let Some(ct) = content_type else {
        return true;
    };
    let ct = ct.to_ascii_lowercase();
    ct.contains("html") || ct.starts_with("text/")
}

fn render_non_html_page(fetch: &FetchResult) -> RenderedPage {
    let mut lines = Vec::new();
    if fetch.status == 204 {
        lines.push("No Content (204): server returned an empty response body.".to_string());
    } else if fetch.status == 304 {
        lines.push("Not Modified (304): no local cache available in this session.".to_string());
    } else {
        lines.push("This response is not rendered as HTML.".to_string());
    }
    lines.push(format!("Requested URL: {}", fetch.requested_url));
    lines.push(format!("Final URL: {}", fetch.final_url));
    lines.push(format!("HTTP status: {}", fetch.status));
    if let Some(ct) = &fetch.content_type {
        lines.push(format!("Content-Type: {ct}"));
    }
    if let Some(location) = &fetch.location {
        lines.push(format!("Location: {location}"));
    }
    if !fetch.redirect_chain.is_empty() {
        lines.push(format!(
            "Redirects followed: {}",
            fetch.redirect_chain.len()
        ));
    }
    lines.push(String::new());
    lines.push("Tip: open this URL in an external browser if needed.".to_string());

    RenderedPage {
        title: format!("HTTP {}", fetch.status),
        source_url: fetch.final_url.to_string(),
        lines,
        links: vec![],
        media: vec![],
        interactive_spans: vec![],
    }
}

fn empty_document(url: Url, title: &str) -> Document {
    Document {
        title: title.to_string(),
        source_url: url,
        text_lines: vec![],
        links: vec![],
        forms: vec![],
        images: vec![],
        media_links: vec![],
        script_count: 0,
        meta_http_equiv: Default::default(),
        markdown_blocks: vec![],
    }
}

fn blank_loaded_page(url: Url) -> LoadedPage {
    LoadedPage {
        url: url.clone(),
        rendered: RenderedPage {
            title: "blank".to_string(),
            source_url: url.to_string(),
            lines: vec!["(blank page)".to_string()],
            links: vec![],
            media: vec![],
            interactive_spans: vec![],
        },
        document: empty_document(url, "Blank Page"),
        js_note: "js skipped on blank page".to_string(),
    }
}

fn prepend_fetch_details(rendered: &mut RenderedPage, fetch: &FetchResult, document: &Document) {
    let mut prefix = vec![
        format!("Requested URL: {}", fetch.requested_url),
        format!("Final URL: {}", fetch.final_url),
        format!("HTTP status: {}", fetch.status),
    ];
    if let Some(ct) = &fetch.content_type {
        prefix.push(format!("Content-Type: {ct}"));
    }
    if !fetch.redirect_chain.is_empty() {
        prefix.push(format!(
            "Redirects followed: {}",
            fetch.redirect_chain.len()
        ));
    }

    if let Some(location) = &fetch.location {
        prefix.push(format!("Location: {location}"));
    }

    let hints = &document.meta_http_equiv.cache_hints;
    if hints.no_cache || hints.no_store || hints.max_age_zero || hints.expires_immediate {
        prefix.push(format!(
            "Meta cache hints: no-cache={}, no-store={}, max-age=0={}, expires-immediate={}",
            hints.no_cache, hints.no_store, hints.max_age_zero, hints.expires_immediate
        ));
    }

    if let Some(refresh) = document.meta_http_equiv.refresh.as_ref() {
        let target = refresh
            .target_absolute
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| refresh.target_raw.clone())
            .unwrap_or_else(|| "(current URL)".to_string());
        prefix.push(format!(
            "Meta refresh: delay={}s target={}",
            refresh.delay_secs, target
        ));
    }

    prefix.push(String::new());
    let offset = prefix.len();

    for span in &mut rendered.interactive_spans {
        span.line = span.line.saturating_add(offset);
    }

    rendered.lines.splice(0..0, prefix);
}

fn build_loaded_status(fetch: &FetchResult, meta_refresh_hops: usize, base: &str) -> String {
    let mut parts = vec![base.to_string(), format!("http {}", fetch.status)];
    if !fetch.redirect_chain.is_empty() {
        parts.push(format!("redir {}", fetch.redirect_chain.len()));
    }
    if meta_refresh_hops > 0 {
        parts.push(format!("meta-refresh {}", meta_refresh_hops));
    }
    parts.join(" | ")
}

fn map_ui_icon_mode(mode: ConfigUiIconMode) -> UiIconMode {
    match mode {
        ConfigUiIconMode::Auto => UiIconMode::Auto,
        ConfigUiIconMode::Ascii => UiIconMode::Ascii,
    }
}

fn map_render_icon_mode(mode: ConfigUiIconMode) -> RenderIconMode {
    match mode {
        ConfigUiIconMode::Auto => RenderIconMode::Auto,
        ConfigUiIconMode::Ascii => RenderIconMode::Ascii,
    }
}

fn resolve_optional_url(absolute: &Option<Url>, raw: &str) -> Option<Url> {
    absolute.clone().or_else(|| normalize_url(raw).ok())
}
