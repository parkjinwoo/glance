use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use chromiumoxide::cdp::browser_protocol::browser::{
    SetDownloadBehaviorBehavior, SetDownloadBehaviorParams,
};
use chromiumoxide::{Browser, BrowserConfig, Page};
use fs2::FileExt;
use futures_util::StreamExt;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio::sync::{mpsc as tokio_mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use url::Url;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

#[derive(Debug, Clone)]
pub(crate) enum CompanionProfile {
    Ephemeral,
    Persistent(PathBuf),
}

#[derive(Debug, Clone)]
pub(crate) struct CompanionConfig {
    pub(crate) executable: String,
    pub(crate) profile: CompanionProfile,
    pub(crate) settle_ms: u64,
    pub(crate) timeout_ms: u64,
    pub(crate) max_body_bytes: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum CompanionOperation {
    Authenticate,
    Focus,
    Navigate,
    Observe,
    Scroll,
    Activate,
}

#[derive(Debug)]
pub(crate) struct CompanionSnapshot {
    pub(crate) url: Url,
    pub(crate) html: String,
    pub(crate) scroll: Option<CompanionScroll>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompanionScroll {
    pub(crate) before: f64,
    pub(crate) after: f64,
    pub(crate) viewport: f64,
    pub(crate) extent: f64,
}

#[derive(Debug)]
pub(crate) enum CompanionEvent {
    AuthReady {
        request_id: u64,
        url: Url,
    },
    AuthFocused {
        request_id: u64,
        url: Url,
    },
    Snapshot {
        request_id: u64,
        operation: CompanionOperation,
        snapshot: CompanionSnapshot,
    },
    Failed {
        request_id: u64,
        operation: CompanionOperation,
        error: String,
    },
}

enum CompanionCommand {
    Authenticate { request_id: u64, url: Url },
    FocusAuth { request_id: u64 },
    Navigate { request_id: u64, url: Url },
    Observe { request_id: u64 },
    Scroll { request_id: u64 },
    Activate { request_id: u64, glance_id: usize },
    Shutdown { done: oneshot::Sender<()> },
}

pub(crate) struct CompanionClient {
    command_tx: tokio_mpsc::UnboundedSender<CompanionCommand>,
    event_rx: Receiver<CompanionEvent>,
    actor_task: Option<JoinHandle<()>>,
}

impl CompanionClient {
    pub(crate) fn spawn(runtime: &Runtime, config: CompanionConfig) -> Self {
        let (command_tx, command_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel();
        let actor_task = runtime.spawn(run_actor(config, command_rx, event_tx));
        Self {
            command_tx,
            event_rx,
            actor_task: Some(actor_task),
        }
    }

    pub(crate) fn authenticate(&self, request_id: u64, url: Url) -> Result<()> {
        self.command_tx
            .send(CompanionCommand::Authenticate { request_id, url })
            .map_err(|_| anyhow!("secure browser session is unavailable"))
    }

    pub(crate) fn navigate(&self, request_id: u64, url: Url) -> Result<()> {
        self.command_tx
            .send(CompanionCommand::Navigate { request_id, url })
            .map_err(|_| anyhow!("secure browser session is unavailable"))
    }

    pub(crate) fn focus_auth(&self, request_id: u64) -> Result<()> {
        self.command_tx
            .send(CompanionCommand::FocusAuth { request_id })
            .map_err(|_| anyhow!("secure browser session is unavailable"))
    }

    pub(crate) fn observe(&self, request_id: u64) -> Result<()> {
        self.command_tx
            .send(CompanionCommand::Observe { request_id })
            .map_err(|_| anyhow!("secure browser session is unavailable"))
    }

    pub(crate) fn scroll(&self, request_id: u64) -> Result<()> {
        self.command_tx
            .send(CompanionCommand::Scroll { request_id })
            .map_err(|_| anyhow!("secure browser session is unavailable"))
    }

    pub(crate) fn activate(&self, request_id: u64, glance_id: usize) -> Result<()> {
        self.command_tx
            .send(CompanionCommand::Activate {
                request_id,
                glance_id,
            })
            .map_err(|_| anyhow!("secure browser session is unavailable"))
    }

    pub(crate) fn try_recv(&self) -> std::result::Result<CompanionEvent, TryRecvError> {
        self.event_rx.try_recv()
    }

    pub(crate) fn shutdown(&mut self, runtime: &Runtime) -> Result<()> {
        let (done_tx, done_rx) = oneshot::channel();
        if self
            .command_tx
            .send(CompanionCommand::Shutdown { done: done_tx })
            .is_err()
        {
            self.join_actor(runtime);
            return Ok(());
        }
        let shutdown_result =
            match runtime.block_on(async { timeout(Duration::from_secs(5), done_rx).await }) {
                Ok(Ok(())) => Ok(()),
                Ok(Err(_)) => Err(anyhow!(
                    "secure browser session stopped before shutdown completed"
                )),
                Err(_) => Err(anyhow!(
                    "timed out while closing the secure browser session"
                )),
            };
        if shutdown_result.is_err() {
            self.abort_actor(runtime);
        } else {
            self.join_actor(runtime);
        }
        shutdown_result
    }

    fn join_actor(&mut self, runtime: &Runtime) {
        if let Some(task) = self.actor_task.take() {
            let _ = runtime.block_on(task);
        }
    }

    fn abort_actor(&mut self, runtime: &Runtime) {
        if let Some(task) = self.actor_task.take() {
            task.abort();
            let _ = runtime.block_on(task);
        }
    }
}

impl Drop for CompanionClient {
    fn drop(&mut self) {
        let (done, _ignored) = oneshot::channel();
        let _ = self.command_tx.send(CompanionCommand::Shutdown { done });
        if let Some(task) = self.actor_task.take() {
            task.abort();
        }
    }
}

async fn run_actor(
    config: CompanionConfig,
    mut command_rx: tokio_mpsc::UnboundedReceiver<CompanionCommand>,
    event_tx: mpsc::Sender<CompanionEvent>,
) {
    let mut session: Option<LiveSession> = None;

    while let Some(command) = command_rx.recv().await {
        match command {
            CompanionCommand::Shutdown { done } => {
                if let Some(live) = session.take() {
                    live.shutdown().await;
                }
                let _ = done.send(());
                break;
            }
            CompanionCommand::Authenticate { request_id, url } => {
                let result =
                    with_session(&config, &mut session, |live| Box::pin(live.open_auth(url))).await;
                match result {
                    Ok(url) => {
                        let _ = event_tx.send(CompanionEvent::AuthReady { request_id, url });
                    }
                    Err(error) => send_failure(
                        &event_tx,
                        request_id,
                        CompanionOperation::Authenticate,
                        error,
                    ),
                }
            }
            CompanionCommand::FocusAuth { request_id } => {
                let result =
                    with_session(&config, &mut session, |live| Box::pin(live.focus_auth())).await;
                match result {
                    Ok(url) => {
                        let _ = event_tx.send(CompanionEvent::AuthFocused { request_id, url });
                    }
                    Err(error) => {
                        send_failure(&event_tx, request_id, CompanionOperation::Focus, error)
                    }
                }
            }
            CompanionCommand::Navigate { request_id, url } => {
                let operation_config = config.clone();
                let result = with_session(&config, &mut session, move |live| {
                    Box::pin(
                        async move { live.navigate_and_snapshot(url, &operation_config).await },
                    )
                })
                .await;
                match result {
                    Ok(snapshot) => {
                        let _ = event_tx.send(CompanionEvent::Snapshot {
                            request_id,
                            operation: CompanionOperation::Navigate,
                            snapshot,
                        });
                    }
                    Err(error) => {
                        send_failure(&event_tx, request_id, CompanionOperation::Navigate, error)
                    }
                }
            }
            CompanionCommand::Observe { request_id } => {
                let operation_config = config.clone();
                let result = with_session(&config, &mut session, move |live| {
                    Box::pin(async move { live.snapshot(&operation_config).await })
                })
                .await;
                match result {
                    Ok(snapshot) => {
                        let _ = event_tx.send(CompanionEvent::Snapshot {
                            request_id,
                            operation: CompanionOperation::Observe,
                            snapshot,
                        });
                    }
                    Err(error) => {
                        send_failure(&event_tx, request_id, CompanionOperation::Observe, error)
                    }
                }
            }
            CompanionCommand::Scroll { request_id } => {
                let operation_config = config.clone();
                let result = with_session(&config, &mut session, move |live| {
                    Box::pin(async move { live.scroll_and_snapshot(&operation_config).await })
                })
                .await;
                match result {
                    Ok(snapshot) => {
                        let _ = event_tx.send(CompanionEvent::Snapshot {
                            request_id,
                            operation: CompanionOperation::Scroll,
                            snapshot,
                        });
                    }
                    Err(error) => {
                        send_failure(&event_tx, request_id, CompanionOperation::Scroll, error)
                    }
                }
            }
            CompanionCommand::Activate {
                request_id,
                glance_id,
            } => {
                let operation_config = config.clone();
                let result = with_session(&config, &mut session, move |live| {
                    Box::pin(async move {
                        live.activate_and_snapshot(glance_id, &operation_config)
                            .await
                    })
                })
                .await;
                match result {
                    Ok(snapshot) => {
                        let _ = event_tx.send(CompanionEvent::Snapshot {
                            request_id,
                            operation: CompanionOperation::Activate,
                            snapshot,
                        });
                    }
                    Err(error) => {
                        send_failure(&event_tx, request_id, CompanionOperation::Activate, error)
                    }
                }
            }
        }
    }

    if let Some(live) = session.take() {
        live.shutdown().await;
    }
}

fn send_failure(
    event_tx: &mpsc::Sender<CompanionEvent>,
    request_id: u64,
    operation: CompanionOperation,
    error: anyhow::Error,
) {
    let _ = event_tx.send(CompanionEvent::Failed {
        request_id,
        operation,
        error: format!("{error:#}"),
    });
}

async fn with_session<T>(
    config: &CompanionConfig,
    session: &mut Option<LiveSession>,
    action: impl for<'a> FnOnce(
        &'a mut LiveSession,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T>> + Send + 'a>,
    >,
) -> Result<T> {
    if session.is_none() {
        *session = Some(LiveSession::launch(config).await?);
    }
    let duration = Duration::from_millis(config.timeout_ms);
    timeout(
        duration,
        action(session.as_mut().expect("session initialized")),
    )
    .await
    .context("secure browser operation timed out")?
}

struct LiveSession {
    browser: Browser,
    page: Option<Page>,
    handler_task: JoinHandle<()>,
    _profile: ProfileLease,
}

impl LiveSession {
    async fn launch(config: &CompanionConfig) -> Result<Self> {
        let profile = ProfileLease::acquire(&config.profile)?;
        let browser_config = BrowserConfig::builder()
            .chrome_executable(&config.executable)
            .with_head()
            .respect_https_errors()
            .viewport(None)
            .window_size(1280, 900)
            .port(0)
            .user_data_dir(profile.path())
            .launch_timeout(Duration::from_millis(config.timeout_ms))
            .request_timeout(Duration::from_millis(config.timeout_ms))
            .disable_default_args()
            .arg(("remote-debugging-address", "127.0.0.1"))
            .arg("no-first-run")
            .arg("no-default-browser-check")
            .build()
            .map_err(anyhow::Error::msg)?;

        let (mut browser, mut handler) = Browser::launch(browser_config)
            .await
            .context("failed to start the secure Chromium companion")?;
        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });
        if let Err(error) = validate_loopback_debug_endpoint(browser.websocket_address()) {
            let _ = browser.close().await;
            handler_task.abort();
            return Err(error);
        }
        if let Err(error) = browser
            .execute(SetDownloadBehaviorParams::new(
                SetDownloadBehaviorBehavior::Deny,
            ))
            .await
        {
            let _ = browser.close().await;
            handler_task.abort();
            return Err(error).context("failed to disable browser downloads");
        }

        Ok(Self {
            browser,
            page: None,
            handler_task,
            _profile: profile,
        })
    }

    async fn open_auth(&mut self, url: Url) -> Result<Url> {
        let page = self.page_for(&url).await?;
        page.bring_to_front()
            .await
            .context("failed to focus the secure sign-in window")?;
        current_page_url(page, &url).await
    }

    async fn focus_auth(&mut self) -> Result<Url> {
        let page = self
            .page
            .as_ref()
            .context("secure browser has no sign-in page")?;
        page.bring_to_front()
            .await
            .context("failed to focus the secure sign-in window")?;
        let fallback = Url::parse("about:blank").expect("valid fallback URL");
        current_page_url(page, &fallback).await
    }

    async fn navigate_and_snapshot(
        &mut self,
        url: Url,
        config: &CompanionConfig,
    ) -> Result<CompanionSnapshot> {
        let page = self.page_for(&url).await?;
        sleep(Duration::from_millis(config.settle_ms)).await;
        snapshot_page(page, &url, config.max_body_bytes, None).await
    }

    async fn snapshot(&mut self, config: &CompanionConfig) -> Result<CompanionSnapshot> {
        let page = self
            .page
            .as_ref()
            .context("secure browser has no active page")?;
        let fallback = page
            .url()
            .await
            .context("failed to inspect secure browser URL")?
            .and_then(|raw| Url::parse(&raw).ok())
            .context("secure browser returned no valid page URL")?;
        snapshot_page(page, &fallback, config.max_body_bytes, None).await
    }

    async fn scroll_and_snapshot(&mut self, config: &CompanionConfig) -> Result<CompanionSnapshot> {
        let page = self
            .page
            .as_ref()
            .context("secure browser has no active page")?;
        let fallback = page
            .url()
            .await
            .context("failed to inspect secure browser URL")?
            .and_then(|raw| Url::parse(&raw).ok())
            .context("secure browser returned no valid page URL")?;
        let before = scroll_metrics(page).await?;
        let moved: [f64; 4] = page
            .evaluate(SCROLL_PAGE_SCRIPT)
            .await
            .context("failed to advance the secure browser viewport")?
            .into_value()
            .context("secure browser returned invalid scroll metrics")?;
        wait_for_feed_settle(page, config.settle_ms).await?;
        let after = scroll_metrics(page).await?;
        let scroll = CompanionScroll {
            before: before[0],
            after: after[0].max(moved[1]),
            viewport: after[1],
            extent: after[2],
        };
        snapshot_page(page, &fallback, config.max_body_bytes, Some(scroll)).await
    }

    async fn activate_and_snapshot(
        &mut self,
        glance_id: usize,
        config: &CompanionConfig,
    ) -> Result<CompanionSnapshot> {
        let page = self
            .page
            .as_ref()
            .context("secure browser has no active page")?;
        let fallback = page
            .url()
            .await
            .context("failed to inspect secure browser URL")?
            .and_then(|raw| Url::parse(&raw).ok())
            .context("secure browser returned no valid page URL")?;
        let script = format!(
            r#"() => {{
                const el = document.querySelector('[data-glance-continuation="{glance_id}"]');
                if (!el || el.disabled || el.getAttribute('aria-disabled') === 'true') return false;
                if (!el.matches('button, [role="button"], input[type="button"], input[type="submit"]')) return false;
                const style = getComputedStyle(el);
                const rect = el.getBoundingClientRect();
                if (style.visibility === 'hidden' || style.display === 'none' || rect.width <= 0 || rect.height <= 0) return false;
                const label = (el.getAttribute('aria-label') || el.innerText || el.value || el.title || '').replace(/\s+/g, ' ').trim();
                const patterns = [
                    /^(?:load|show|see) more(?:\s+(?:posts|results|items|stories|replies|comments|content|articles|updates|entries|messages|videos))?$/i,
                    /^more (?:posts|results|items|stories|replies|comments|content|articles|updates|entries|messages|videos)$/i,
                    /^(?:older posts|next page)$/i,
                    /^(?:더\s*(?:보기|불러오기|읽기)|더 많은\s*(?:게시물|글|댓글|답글|콘텐츠|결과)|이전 게시물|다음 페이지|계속 보기)$/,
                    /^(?:もっと見る|さらに表示|次へ)$/, /^(?:加载更多|顯示更多|显示更多|更多内容)$/
                ];
                if (!patterns.some(pattern => pattern.test(label))) return false;
                el.click();
                return true;
            }}"#
        );
        let activated: bool = page
            .evaluate(script)
            .await
            .context("failed to activate the continuation control")?
            .into_value()
            .context("secure browser returned an invalid activation result")?;
        ensure!(activated, "continuation control is no longer available");
        wait_for_feed_settle(page, config.settle_ms).await?;
        snapshot_page(page, &fallback, config.max_body_bytes, None).await
    }

    async fn page_for(&mut self, url: &Url) -> Result<&Page> {
        if self.page.is_none() {
            self.page = Some(
                self.browser
                    .new_page(url.as_str())
                    .await
                    .context("failed to create a secure browser page")?,
            );
        } else if let Some(page) = &self.page {
            page.goto(url.as_str())
                .await
                .context("secure browser navigation failed")?;
        }
        Ok(self.page.as_ref().expect("page initialized"))
    }

    async fn shutdown(mut self) {
        let _ = self.browser.close().await;
        let _ = timeout(Duration::from_secs(3), self.browser.wait()).await;
        self.handler_task.abort();
    }
}

fn validate_loopback_debug_endpoint(raw: &str) -> Result<()> {
    let endpoint = Url::parse(raw).context("Chromium returned an invalid debugging endpoint")?;
    ensure!(
        matches!(endpoint.scheme(), "ws" | "wss"),
        "Chromium debugging endpoint used an unexpected scheme"
    );
    ensure!(
        endpoint.username().is_empty() && endpoint.password().is_none(),
        "Chromium debugging endpoint unexpectedly contained credentials"
    );
    let host = endpoint
        .host_str()
        .context("Chromium debugging endpoint had no host")?;
    ensure!(
        matches!(host, "127.0.0.1" | "::1" | "localhost"),
        "refusing a non-loopback Chromium debugging endpoint"
    );
    ensure!(
        endpoint.port().is_some(),
        "Chromium debugging endpoint had no ephemeral port"
    );
    Ok(())
}

async fn current_page_url(page: &Page, fallback: &Url) -> Result<Url> {
    page.url()
        .await
        .context("failed to inspect secure browser URL")?
        .and_then(|raw| Url::parse(&raw).ok())
        .or_else(|| Some(fallback.clone()))
        .context("secure browser returned no valid page URL")
}

async fn snapshot_page(
    page: &Page,
    fallback: &Url,
    max_body_bytes: usize,
    scroll: Option<CompanionScroll>,
) -> Result<CompanionSnapshot> {
    let url = current_page_url(page, fallback).await?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "secure browser left the allowed HTTP(S) boundary"
    );
    let html_chars: usize = page
        .evaluate("() => document.documentElement ? document.documentElement.outerHTML.length : 0")
        .await
        .context("failed to measure the secure browser DOM")?
        .into_value()
        .context("secure browser returned an invalid DOM length")?;
    ensure!(
        html_chars <= max_body_bytes,
        "secure browser DOM exceeds configured network.max_body_bytes ({max_body_bytes} bytes)"
    );
    let _: usize = page
        .evaluate(TAG_CONTINUATION_BUTTONS_SCRIPT)
        .await
        .context("failed to identify safe continuation controls")?
        .into_value()
        .context("secure browser returned an invalid continuation count")?;
    let html = page
        .content()
        .await
        .context("failed to read the secure browser DOM")?;
    ensure!(
        html.len() <= max_body_bytes,
        "secure browser DOM exceeds configured network.max_body_bytes ({max_body_bytes} bytes)"
    );
    Ok(CompanionSnapshot { url, html, scroll })
}

const SCROLL_TARGET_SCRIPT: &str = r#"() => {
    const root = document.scrollingElement || document.documentElement;
    let best = root;
    let bestScore = Math.max(0, root.scrollHeight - root.clientHeight) * root.clientHeight;
    for (const el of document.querySelectorAll('main, section, [role="main"], [role="feed"], div')) {
        const style = getComputedStyle(el);
        if (style.overflowY !== 'auto' && style.overflowY !== 'scroll') continue;
        const rect = el.getBoundingClientRect();
        if (rect.height < 160 || rect.bottom <= 0 || rect.top >= innerHeight) continue;
        const overflow = el.scrollHeight - el.clientHeight;
        if (overflow <= 32) continue;
        const score = Math.min(overflow, 100000) * el.clientHeight;
        if (score > bestScore) {
            best = el;
            bestScore = score;
        }
    }
    return [best.scrollTop || scrollY || 0, best.clientHeight || innerHeight || 0, best.scrollHeight || 0];
}"#;

const TAG_CONTINUATION_BUTTONS_SCRIPT: &str = r#"() => {
    const patterns = [
        /^(?:load|show|see) more(?:\s+(?:posts|results|items|stories|replies|comments|content|articles|updates|entries|messages|videos))?$/i,
        /^more (?:posts|results|items|stories|replies|comments|content|articles|updates|entries|messages|videos)$/i,
        /^(?:older posts|next page)$/i,
        /^(?:더\s*(?:보기|불러오기|읽기)|더 많은\s*(?:게시물|글|댓글|답글|콘텐츠|결과)|이전 게시물|다음 페이지|계속 보기)$/,
        /^(?:もっと見る|さらに表示|次へ)$/, /^(?:加载更多|顯示更多|显示更多|更多内容)$/
    ];
    for (const node of document.querySelectorAll('[data-glance-continuation]')) {
        node.removeAttribute('data-glance-continuation');
    }
    const nodes = [...document.querySelectorAll('button, [role="button"], input[type="button"], input[type="submit"]')];
    let count = 0;
    for (const node of nodes) {
        if (node.disabled || node.getAttribute('aria-disabled') === 'true') continue;
        const style = getComputedStyle(node);
        const rect = node.getBoundingClientRect();
        if (style.visibility === 'hidden' || style.display === 'none' || rect.width <= 0 || rect.height <= 0) continue;
        const label = (node.getAttribute('aria-label') || node.innerText || node.value || node.title || '').replace(/\s+/g, ' ').trim();
        if (!label || !patterns.some(pattern => pattern.test(label))) continue;
        node.setAttribute('data-glance-continuation', String(count));
        count += 1;
    }
    return count;
}"#;

const SCROLL_PAGE_SCRIPT: &str = r#"() => {
    const root = document.scrollingElement || document.documentElement;
    let best = root;
    let bestScore = Math.max(0, root.scrollHeight - root.clientHeight) * root.clientHeight;
    for (const el of document.querySelectorAll('main, section, [role="main"], [role="feed"], div')) {
        const style = getComputedStyle(el);
        if (style.overflowY !== 'auto' && style.overflowY !== 'scroll') continue;
        const rect = el.getBoundingClientRect();
        if (rect.height < 160 || rect.bottom <= 0 || rect.top >= innerHeight) continue;
        const overflow = el.scrollHeight - el.clientHeight;
        if (overflow <= 32) continue;
        const score = Math.min(overflow, 100000) * el.clientHeight;
        if (score > bestScore) {
            best = el;
            bestScore = score;
        }
    }
    const before = best.scrollTop || scrollY || 0;
    const viewport = best.clientHeight || innerHeight || 0;
    const extent = best.scrollHeight || 0;
    best.scrollBy({top: Math.max(viewport * 0.9, 480), left: 0, behavior: 'auto'});
    return [before, best.scrollTop || scrollY || 0, viewport, extent];
}"#;

async fn scroll_metrics(page: &Page) -> Result<[f64; 3]> {
    page.evaluate(SCROLL_TARGET_SCRIPT)
        .await
        .context("failed to inspect the secure browser viewport")?
        .into_value()
        .context("secure browser returned invalid viewport metrics")
}

async fn wait_for_feed_settle(page: &Page, configured_wait_ms: u64) -> Result<()> {
    let maximum = Duration::from_millis(configured_wait_ms.clamp(1_200, 4_000));
    let minimum = Duration::from_millis(500);
    let stable_for = Duration::from_millis(450);
    let started = Instant::now();
    let mut stable_since = started;
    let mut previous = scroll_metrics(page).await?;

    loop {
        sleep(Duration::from_millis(150)).await;
        let current = scroll_metrics(page).await?;
        let changed = current
            .iter()
            .zip(previous)
            .any(|(current, previous)| (current - previous).abs() > 0.5);
        if changed {
            stable_since = Instant::now();
            previous = current;
        }
        let elapsed = Instant::now().saturating_duration_since(started);
        if elapsed >= minimum
            && Instant::now().saturating_duration_since(stable_since) >= stable_for
        {
            break;
        }
        if elapsed >= maximum {
            break;
        }
    }
    Ok(())
}

struct ProfileLease {
    path: PathBuf,
    _temporary: Option<TempDir>,
    lock: Option<File>,
}

impl ProfileLease {
    fn acquire(profile: &CompanionProfile) -> Result<Self> {
        match profile {
            CompanionProfile::Ephemeral => {
                let temporary = tempfile::Builder::new()
                    .prefix("glance-browser-")
                    .tempdir()
                    .context("failed to create a private browser profile")?;
                secure_directory(temporary.path())?;
                Ok(Self {
                    path: temporary.path().to_path_buf(),
                    _temporary: Some(temporary),
                    lock: None,
                })
            }
            CompanionProfile::Persistent(path) => {
                secure_directory(path)?;
                let lock_path = path.join(".glance-session.lock");
                let lock = secure_lock_file(&lock_path)?;
                FileExt::try_lock_exclusive(&lock).with_context(|| {
                    format!(
                        "browser profile is already in use by another Glance process: {}",
                        path.display()
                    )
                })?;
                Ok(Self {
                    path: path.clone(),
                    _temporary: None,
                    lock: Some(lock),
                })
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ProfileLease {
    fn drop(&mut self) {
        if let Some(lock) = &self.lock {
            let _ = FileExt::unlock(lock);
        }
    }
}

fn secure_directory(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            bail!(
                "refusing to use a symlink as a browser profile: {}",
                path.display()
            );
        }
        ensure!(
            metadata.is_dir(),
            "browser profile path is not a directory: {}",
            path.display()
        );
    } else {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(path)
            .with_context(|| format!("failed to create browser profile at {}", path.display()))?;
    }

    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).with_context(|| {
        format!(
            "failed to restrict browser profile permissions at {}",
            path.display()
        )
    })?;
    Ok(())
}

fn secure_lock_file(path: &Path) -> Result<File> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            !metadata.file_type().is_symlink(),
            "refusing a symlink as a browser profile lock: {}",
            path.display()
        );
        ensure!(
            metadata.is_file(),
            "browser profile lock is not a file: {}",
            path.display()
        );
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(path)
        .with_context(|| format!("failed to open browser profile lock at {}", path.display()))?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| {
            format!(
                "failed to restrict browser profile lock at {}",
                path.display()
            )
        })?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::{
        CompanionProfile, ProfileLease, secure_directory, secure_lock_file,
        validate_loopback_debug_endpoint,
    };

    #[test]
    fn only_loopback_debug_endpoints_are_accepted() {
        assert!(
            validate_loopback_debug_endpoint("ws://127.0.0.1:49152/devtools/browser/token").is_ok()
        );
        assert!(
            validate_loopback_debug_endpoint("ws://localhost:49152/devtools/browser/token").is_ok()
        );
        assert!(
            validate_loopback_debug_endpoint("ws://192.0.2.1:9222/devtools/browser/token").is_err()
        );
        assert!(
            validate_loopback_debug_endpoint("ws://user@127.0.0.1:9222/devtools/browser/token")
                .is_err()
        );
    }

    #[test]
    fn ephemeral_profiles_are_removed_on_drop() {
        let path = {
            let profile = ProfileLease::acquire(&CompanionProfile::Ephemeral).expect("profile");
            let path = profile.path().to_path_buf();
            assert!(path.is_dir());
            path
        };
        assert!(!path.exists());
    }

    #[test]
    fn persistent_profile_cannot_be_shared_between_process_sessions() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("profile");
        let profile = CompanionProfile::Persistent(path);
        let first = ProfileLease::acquire(&profile).expect("first lock");
        assert!(ProfileLease::acquire(&profile).is_err());
        drop(first);
        assert!(ProfileLease::acquire(&profile).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn profile_permissions_are_restricted() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("tempdir");
        let profile = root.path().join("profile");
        let lease = ProfileLease::acquire(&CompanionProfile::Persistent(profile.clone()))
            .expect("secure profile");
        let mode = profile.metadata().expect("metadata").permissions().mode() & 0o777;
        let lock_mode = profile
            .join(".glance-session.lock")
            .metadata()
            .expect("lock metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        assert_eq!(lock_mode, 0o600);
        drop(lease);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_profiles_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path().join("target");
        let link = root.path().join("link");
        std::fs::create_dir(&target).expect("target");
        symlink(&target, &link).expect("symlink");
        assert!(secure_directory(&link).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_profile_locks_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path().join("target");
        let link = root.path().join("lock");
        std::fs::write(&target, b"unrelated").expect("target");
        symlink(&target, &link).expect("symlink");
        assert!(secure_lock_file(&link).is_err());
    }
}
