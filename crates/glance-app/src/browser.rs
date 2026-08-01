use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::runtime::Runtime;
use url::Url;

use glance_config::UiIconMode as ConfigUiIconMode;
use glance_dom::{
    Document, Form, LiveControlKind, LiveFieldKind, MediaKind, parse_document_with_options,
};
use glance_media::{
    ExternalPlayerOutcome, ImageBackend, detect_image_backend, image_dimensions,
    launch_external_player_with_options,
};
use glance_net::{FetchResult, NetClient, normalize_url, resolve_user_input};
use glance_render::{RenderIconMode, RenderedPage, render_document};
use glance_ui::{
    UiAuthItem, UiCommand, UiIconMode, UiImageMode, UiLiveControlItem, UiLiveControlKind,
    UiLiveFieldKind, UiLiveOption, UiMediaActivity, UiMediaItem, UiMediaKind, UiMediaPhase, UiPage,
    UiResourceItem, UiWebMode,
};

use crate::companion::{
    CompanionClient, CompanionConfig, CompanionEvent, CompanionOperation, CompanionProfile,
    CompanionSnapshot,
};
use crate::dynamic::{
    DynamicPage, looks_like_client_rendered_shell, render_with_browser, resolve_browser_executable,
};
use crate::external::launch_system_browser;
use crate::policy::{
    DynamicMode, DynamicPolicy, MediaPolicy, NetworkPolicy, is_about_blank,
    validate_url_for_network,
};

#[derive(Debug, Clone)]
struct LoadedPage {
    url: Url,
    rendered: RenderedPage,
    document: Document,
    can_scroll: bool,
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

#[derive(Debug, Clone)]
struct PendingDynamic {
    request_id: u64,
    url: Url,
}

#[derive(Debug, Clone, Copy)]
struct PendingCompanionNavigation {
    request_id: u64,
    plan: HistoryPlan,
}

#[derive(Debug, Clone, Copy)]
struct PendingCompanionScroll {
    request_id: u64,
}

#[derive(Debug, Clone)]
struct PendingCompanionAction {
    request_id: u64,
    label: String,
}

#[derive(Debug, Clone)]
struct PendingLiveControlAction {
    request_id: u64,
    label: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AuthPhase {
    Opening,
    Waiting,
}

#[derive(Debug, Clone)]
struct PendingAuth {
    request_id: u64,
    observe_request_id: Option<u64>,
    focus_request_id: Option<u64>,
    auth_url: Url,
    return_url: Url,
    phase: AuthPhase,
    last_observe_at: Instant,
    completion_url: Option<Url>,
    completion_observations: u8,
}

#[derive(Debug, Clone)]
struct PendingMedia {
    request_id: u64,
    index: usize,
    label: String,
    url: Url,
    kind: UiMediaKind,
    phase: UiMediaPhase,
    release_at: Instant,
    spawned_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SessionWebMode {
    Static,
    Dynamic,
    Companion,
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
    Dynamic {
        request_id: u64,
        url: Url,
        result: Box<std::result::Result<DynamicPage, String>>,
    },
    Media {
        request_id: u64,
        index: usize,
        result: std::result::Result<ExternalPlayerOutcome, String>,
    },
}

struct NavigationOutcome {
    loaded: LoadedPage,
    status: String,
    suggest_dynamic: bool,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    id: u64,
    url: Url,
}

pub(crate) struct Browser {
    runtime: Runtime,
    net: NetClient,
    media: MediaPolicy,
    dynamic: DynamicPolicy,
    dynamic_executable: Option<String>,
    web_mode: SessionWebMode,
    image_backend: ImageBackend,
    image_mode: SessionImageMode,
    icon_mode: UiIconMode,
    render_icon_mode: RenderIconMode,
    network_policy: NetworkPolicy,
    search_template: String,
    private: bool,
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
    inflight_dynamic: Option<PendingDynamic>,
    inflight_companion_nav: Option<PendingCompanionNavigation>,
    inflight_companion_scroll: Option<PendingCompanionScroll>,
    inflight_companion_action: Option<PendingCompanionAction>,
    inflight_live_control_action: Option<PendingLiveControlAction>,
    inflight_media: Option<PendingMedia>,
    last_external_launch: Option<(Url, Instant)>,
    companion: Option<CompanionClient>,
    companion_authenticated: bool,
    pending_auth: Option<PendingAuth>,
    companion_feed_capped: bool,
    last_companion_scroll_at: Option<Instant>,
}

pub(crate) struct BrowserOptions {
    pub(crate) icon_mode: ConfigUiIconMode,
    pub(crate) media: MediaPolicy,
    pub(crate) dynamic: DynamicPolicy,
    pub(crate) network: NetworkPolicy,
    pub(crate) search_template: String,
    pub(crate) private: bool,
    pub(crate) companion_profile: CompanionProfile,
}

const META_REFRESH_HOP_LIMIT: usize = 3;
const MEDIA_LAUNCH_GUARD: Duration = Duration::from_secs(4);
const EXTERNAL_LAUNCH_GUARD: Duration = Duration::from_secs(2);
const AUTH_OBSERVE_INTERVAL: Duration = Duration::from_millis(750);
const COMPANION_SCROLL_GUARD: Duration = Duration::from_millis(450);
const MAX_FEED_BLOCKS: usize = 4_000;
const MAX_FEED_TEXT_LINES: usize = 10_000;
const MAX_FEED_RESOURCES: usize = 2_000;

impl Browser {
    pub(crate) fn new(runtime: Runtime, net: NetClient, options: BrowserOptions) -> Self {
        let BrowserOptions {
            icon_mode,
            media,
            dynamic,
            network: network_policy,
            search_template,
            private,
            companion_profile,
        } = options;
        let image_backend = detect_image_backend();
        let dynamic_executable = resolve_browser_executable(dynamic.executable.as_deref());
        let (results_tx, results_rx) = mpsc::channel();
        let icon_mode_ui = map_ui_icon_mode(icon_mode);
        let render_icon_mode = map_render_icon_mode(icon_mode);
        let image_mode = if media.images_enabled {
            SessionImageMode::Auto
        } else {
            SessionImageMode::TextForced
        };
        let companion = if dynamic.security_compatible {
            dynamic_executable.as_ref().map(|executable| {
                CompanionClient::spawn(
                    &runtime,
                    CompanionConfig {
                        executable: executable.clone(),
                        profile: companion_profile,
                        settle_ms: dynamic.wait_ms,
                        timeout_ms: dynamic.timeout_ms,
                        max_body_bytes: network_policy.max_body_bytes,
                    },
                )
            })
        } else {
            None
        };

        Self {
            runtime,
            net,
            media,
            dynamic,
            dynamic_executable,
            web_mode: SessionWebMode::Static,
            image_backend,
            image_mode,
            icon_mode: icon_mode_ui,
            render_icon_mode,
            network_policy,
            search_template,
            private,
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
            inflight_dynamic: None,
            inflight_companion_nav: None,
            inflight_companion_scroll: None,
            inflight_companion_action: None,
            inflight_live_control_action: None,
            inflight_media: None,
            last_external_launch: None,
            companion,
            companion_authenticated: false,
            pending_auth: None,
            companion_feed_capped: false,
            last_companion_scroll_at: None,
        }
    }

    pub(crate) fn load_initial(&mut self, url: Url) -> Result<UiPage> {
        self.open_url(url, HistoryPlan::Push)?;
        Ok(self.to_ui_page())
    }

    pub(crate) fn handle_command(&mut self, cmd: UiCommand) -> UiPage {
        self.drain_async_results();

        let is_poll = matches!(cmd, UiCommand::Poll);
        let result = match cmd {
            UiCommand::Poll => Ok(()),
            UiCommand::OpenUrl(raw) => self.open_url_string(&raw),
            UiCommand::OpenExternal(raw) => self.open_external(&raw),
            UiCommand::OpenAuth(raw) => self.open_auth(&raw),
            UiCommand::LoadMore => self.load_more(),
            UiCommand::ActivateButton(index) => self.activate_button(index),
            UiCommand::FillLiveField(index, value) => self.fill_live_field(index, &value),
            UiCommand::ToggleLiveField(index) => self.toggle_live_field(index),
            UiCommand::ActivateLiveControl(index) => self.activate_live_control(index),
            UiCommand::OpenLink(idx) => self.open_link(idx),
            UiCommand::SubmitForm(idx, value) => self.submit_form(idx, &value),
            UiCommand::OpenMedia(idx) => self.open_media(idx),
            UiCommand::SelectImage(idx) => self.select_image(idx),
            UiCommand::Reload => self.reload(),
            UiCommand::ReloadDynamic => self.reload_dynamic(),
            UiCommand::Back => self.back(),
            UiCommand::Forward => self.forward(),
            UiCommand::ToggleImageMode => self.toggle_image_mode(),
        };

        if is_poll {
            self.poll_pending_auth();
        }

        if let Err(err) = result {
            self.last_status = format!("error: {err:#}");
        }

        self.to_ui_page()
    }

    pub(crate) fn save_session(&self) -> Result<()> {
        self.net.save_cookies()
    }

    pub(crate) fn shutdown(&mut self) -> Result<()> {
        if let Some(companion) = &mut self.companion {
            companion.shutdown(&self.runtime)?;
        }
        Ok(())
    }

    pub(crate) fn to_ui_page(&self) -> UiPage {
        let status = self.composed_status();

        if let Some(current) = &self.current {
            let links = build_link_items(&current.document);
            let auth = find_auth_link_index(&current.document).and_then(|link_index| {
                links.get(link_index).map(|link| UiAuthItem {
                    link_index,
                    label: normalized_auth_label(&link.label),
                    url: link.url.clone(),
                })
            });
            let images = build_image_items(&current.document);
            let media = build_media_items(&current.document);
            let live_controls = build_live_control_items(&current.document);
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
                icon_mode: self.icon_mode,
                web_mode: self.ui_web_mode(),
                image_backend: self.image_backend,
                image_mode: match self.image_mode {
                    SessionImageMode::Auto => UiImageMode::Auto,
                    SessionImageMode::TextForced => UiImageMode::Text,
                },
                links,
                auth,
                live_controls,
                images,
                media,
                media_activity: self.ui_media_activity(),
                can_load_more: self.companion_authenticated
                    && self.web_mode == SessionWebMode::Companion
                    && current.can_scroll
                    && !self.companion_feed_capped,
                loading_more: self.inflight_companion_scroll.is_some()
                    || self.inflight_companion_action.is_some(),
                updating_live_controls: self.inflight_live_control_action.is_some(),
                selected_image_index,
                selected_image_bytes,
                selected_image_size,
            }
        } else {
            UiPage {
                history_entry_id: self.current_history_entry_id().unwrap_or(0),
                rendered: RenderedPage {
                    title: "New tab".to_string(),
                    source_url: "".to_string(),
                    lines: vec![
                        "# New tab".to_string(),
                        String::new(),
                        "Open an address or search  ·  o / Ctrl-L".to_string(),
                        "Shortcuts  ·  ?".to_string(),
                    ],
                    links: vec![],
                    media: vec![],
                    interactive_spans: vec![],
                },
                status,
                icon_mode: self.icon_mode,
                web_mode: self.ui_web_mode(),
                image_backend: self.image_backend,
                image_mode: match self.image_mode {
                    SessionImageMode::Auto => UiImageMode::Auto,
                    SessionImageMode::TextForced => UiImageMode::Text,
                },
                links: vec![],
                auth: None,
                live_controls: vec![],
                images: vec![],
                media: vec![],
                media_activity: self.ui_media_activity(),
                can_load_more: false,
                loading_more: false,
                updating_live_controls: false,
                selected_image_index: None,
                selected_image_bytes: None,
                selected_image_size: None,
            }
        }
    }

    fn open_url_string(&mut self, raw: &str) -> Result<()> {
        let url = resolve_user_input(raw, &self.search_template)?;
        self.open_url(url, HistoryPlan::Push)
    }

    fn open_external(&mut self, raw: &str) -> Result<()> {
        let url = Url::parse(raw).context("GUI browser handoff requires a complete URL")?;
        self.validate_url_for_network(&url)?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https"),
            "GUI browser handoff only supports HTTP(S) URLs"
        );

        if self
            .last_external_launch
            .as_ref()
            .is_some_and(|(last, at)| {
                last == &url
                    && Instant::now().saturating_duration_since(*at) < EXTERNAL_LAUNCH_GUARD
            })
        {
            self.last_status = "already opened in GUI browser".to_string();
            return Ok(());
        }

        let pid = launch_system_browser(&url)?;
        self.last_external_launch = Some((url, Instant::now()));
        self.last_status = format!("opened in GUI browser (pid {pid})");
        Ok(())
    }

    fn open_auth(&mut self, raw: &str) -> Result<()> {
        let url = Url::parse(raw).context("sign-in requires a complete URL")?;
        self.validate_url_for_network(&url)?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https"),
            "secure sign-in only supports HTTP(S) URLs"
        );

        if self.pending_auth.is_some() {
            if self
                .pending_auth
                .as_ref()
                .is_some_and(|pending| pending.focus_request_id.is_some())
            {
                self.last_status = "secure sign-in browser is already being focused".to_string();
                return Ok(());
            }
            let request_id = self.next_request_id();
            self.companion
                .as_ref()
                .context("secure browser session is unavailable")?
                .focus_auth(request_id)?;
            if let Some(pending) = &mut self.pending_auth {
                pending.focus_request_id = Some(request_id);
            }
            self.last_status = "sign-in is already active; focusing its secure window".to_string();
            return Ok(());
        }

        if self.companion_authenticated && self.inflight_companion_nav.is_some() {
            self.last_status =
                "sign-in already completed; returning to the authenticated page".to_string();
            return Ok(());
        }

        if self.companion.is_none() {
            self.open_external(raw)?;
            self.last_status
                .push_str(" | TUI session sharing unavailable");
            return Ok(());
        }
        let return_url = self
            .current
            .as_ref()
            .map(|page| page.url.clone())
            .unwrap_or_else(|| url.clone());
        let request_id = self.next_request_id();
        self.companion
            .as_ref()
            .context("secure browser session is unavailable")?
            .authenticate(request_id, url.clone())?;
        self.pending_auth = Some(PendingAuth {
            request_id,
            observe_request_id: None,
            focus_request_id: None,
            auth_url: url,
            return_url,
            phase: AuthPhase::Opening,
            last_observe_at: Instant::now(),
            completion_url: None,
            completion_observations: 0,
        });
        self.last_status = "opening isolated secure sign-in browser".to_string();
        Ok(())
    }

    fn open_url(&mut self, url: Url, plan: HistoryPlan) -> Result<()> {
        if self.companion_authenticated && !is_about_blank(&url) {
            return self.queue_companion_navigation(url, plan);
        }
        self.inflight_dynamic = None;
        self.inflight_companion_scroll = None;
        self.inflight_companion_action = None;
        self.inflight_live_control_action = None;
        self.companion_feed_capped = false;
        self.web_mode = SessionWebMode::Static;
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
        let render_icon_mode = self.render_icon_mode;
        let network_policy = self.network_policy.clone();
        let status_url = url.to_string();
        self.runtime.spawn(async move {
            let result = fetch_and_build_async(net, render_icon_mode, url, network_policy)
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

    fn queue_companion_navigation(&mut self, url: Url, plan: HistoryPlan) -> Result<()> {
        self.validate_url_for_network(&url)?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https"),
            "secure browser navigation only supports HTTP(S) URLs"
        );
        anyhow::ensure!(
            self.companion.is_some(),
            "secure browser session is unavailable"
        );
        if self.inflight_companion_nav.is_some() {
            self.last_status = "secure session navigation already in progress".to_string();
            return Ok(());
        }
        let request_id = self.next_request_id();
        self.companion
            .as_ref()
            .context("secure browser session is unavailable")?
            .navigate(request_id, url.clone())?;
        self.inflight_nav = None;
        self.inflight_dynamic = None;
        self.inflight_companion_scroll = None;
        self.inflight_companion_action = None;
        self.inflight_live_control_action = None;
        self.companion_feed_capped = false;
        self.inflight_companion_nav = Some(PendingCompanionNavigation { request_id, plan });
        self.clear_selected_image_data();
        self.last_status = format!("loading secure session {url}");
        Ok(())
    }

    fn load_more(&mut self) -> Result<()> {
        if !self.companion_authenticated || self.web_mode != SessionWebMode::Companion {
            self.last_status = "continuous feed requires an authenticated browser session".into();
            return Ok(());
        }
        if self.companion_feed_capped {
            self.last_status =
                "feed memory limit reached; open a link or reload to start a fresh view".into();
            return Ok(());
        }
        if self.live_page_update_inflight() {
            self.last_status = "fetching more feed content".into();
            return Ok(());
        }
        if self.inflight_companion_nav.is_some() {
            self.last_status = "secure session navigation is still in progress".into();
            return Ok(());
        }
        if self.last_companion_scroll_at.is_some_and(|last| {
            Instant::now().saturating_duration_since(last) < COMPANION_SCROLL_GUARD
        }) {
            return Ok(());
        }

        let request_id = self.next_request_id();
        self.companion
            .as_ref()
            .context("secure browser session is unavailable")?
            .scroll(request_id)?;
        self.inflight_companion_scroll = Some(PendingCompanionScroll { request_id });
        self.last_companion_scroll_at = Some(Instant::now());
        self.last_status = "scrolling the live page and waiting for more content".into();
        Ok(())
    }

    fn activate_button(&mut self, index: usize) -> Result<()> {
        let button = self
            .current
            .as_ref()
            .and_then(|page| page.document.continuation_buttons.get(index))
            .cloned()
            .with_context(|| format!("continuation button {} is unavailable", index + 1))?;
        anyhow::ensure!(
            self.companion_authenticated && self.web_mode == SessionWebMode::Companion,
            "continuation controls require a connected secure browser session"
        );
        if self.live_page_update_inflight() {
            self.last_status = "a live page update is already in progress".to_string();
            return Ok(());
        }
        let request_id = self.next_request_id();
        let expected_url = self
            .current
            .as_ref()
            .context("no current page")?
            .url
            .clone();
        self.companion
            .as_ref()
            .context("secure browser session is unavailable")?
            .activate(
                request_id,
                expected_url,
                button.glance_id,
                button.label.clone(),
            )?;
        self.inflight_companion_action = Some(PendingCompanionAction {
            request_id,
            label: button.label.clone(),
        });
        self.last_status = format!("activating {}", button.label);
        Ok(())
    }

    fn activate_live_control(&mut self, index: usize) -> Result<()> {
        let control = self
            .current
            .as_ref()
            .and_then(|page| page.document.live_controls.get(index))
            .cloned()
            .with_context(|| format!("live control {} is unavailable", index + 1))?;
        anyhow::ensure!(
            control.kind != LiveControlKind::Field,
            "live field must be edited instead of activated"
        );
        self.ensure_live_control_session()?;
        if self.live_page_update_inflight() {
            self.last_status = "a live page update is already in progress".to_string();
            return Ok(());
        }
        let request_id = self.next_request_id();
        let expected_url = self
            .current
            .as_ref()
            .context("no current page")?
            .url
            .clone();
        self.companion
            .as_ref()
            .context("secure browser session is unavailable")?
            .activate_live(
                request_id,
                expected_url,
                control.glance_id,
                control.kind,
                control.label.clone(),
            )?;
        self.inflight_live_control_action = Some(PendingLiveControlAction {
            request_id,
            label: control.label.clone(),
        });
        self.last_status = format!("updating live control {}", control.label);
        Ok(())
    }

    fn fill_live_field(&mut self, index: usize, value: &str) -> Result<()> {
        let control = self
            .current
            .as_ref()
            .and_then(|page| page.document.live_controls.get(index))
            .cloned()
            .with_context(|| format!("live field {} is unavailable", index + 1))?;
        anyhow::ensure!(
            control.kind == LiveControlKind::Field,
            "selected live control is not an editable field"
        );
        let field_kind = control
            .field_kind
            .context("live field type is unavailable")?;
        anyhow::ensure!(
            matches!(
                field_kind,
                LiveFieldKind::Text | LiveFieldKind::TextArea | LiveFieldKind::Select
            ),
            "selected live field does not accept a text value"
        );
        self.ensure_live_control_session()?;
        if self.live_page_update_inflight() {
            self.last_status = "a live page update is already in progress".to_string();
            return Ok(());
        }
        let request_id = self.next_request_id();
        let expected_url = self
            .current
            .as_ref()
            .context("no current page")?
            .url
            .clone();
        self.companion
            .as_ref()
            .context("secure browser session is unavailable")?
            .fill_live(
                request_id,
                expected_url,
                control.glance_id,
                field_kind,
                control.label.clone(),
                value.to_string(),
            )?;
        self.inflight_live_control_action = Some(PendingLiveControlAction {
            request_id,
            label: control.label.clone(),
        });
        self.last_status = format!("editing live field {}", control.label);
        Ok(())
    }

    fn toggle_live_field(&mut self, index: usize) -> Result<()> {
        let control = self
            .current
            .as_ref()
            .and_then(|page| page.document.live_controls.get(index))
            .cloned()
            .with_context(|| format!("live field {} is unavailable", index + 1))?;
        anyhow::ensure!(
            control.kind == LiveControlKind::Field,
            "selected live control is not an editable field"
        );
        let field_kind = control
            .field_kind
            .context("live field type is unavailable")?;
        anyhow::ensure!(
            matches!(field_kind, LiveFieldKind::Checkbox | LiveFieldKind::Radio),
            "selected live field is not a toggle"
        );
        self.ensure_live_control_session()?;
        if self.live_page_update_inflight() {
            self.last_status = "a live page update is already in progress".to_string();
            return Ok(());
        }
        let request_id = self.next_request_id();
        let expected_url = self
            .current
            .as_ref()
            .context("no current page")?
            .url
            .clone();
        self.companion
            .as_ref()
            .context("secure browser session is unavailable")?
            .toggle_live(
                request_id,
                expected_url,
                control.glance_id,
                field_kind,
                control.label.clone(),
            )?;
        self.inflight_live_control_action = Some(PendingLiveControlAction {
            request_id,
            label: control.label.clone(),
        });
        self.last_status = format!("updating {}", control.label);
        Ok(())
    }

    fn ensure_live_control_session(&self) -> Result<()> {
        anyhow::ensure!(
            self.companion_authenticated && self.web_mode == SessionWebMode::Companion,
            "live controls require a connected secure browser session"
        );
        Ok(())
    }

    fn live_page_update_inflight(&self) -> bool {
        self.inflight_companion_action.is_some()
            || self.inflight_companion_scroll.is_some()
            || self.inflight_live_control_action.is_some()
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

    fn submit_form(&mut self, idx: usize, value: &str) -> Result<()> {
        let current = self.current.as_ref().context("no current page")?;
        let form = current
            .document
            .forms
            .get(idx)
            .with_context(|| format!("invalid form index: {}", idx + 1))?;
        let target = build_get_form_url(form, value)?;
        self.open_url(target, HistoryPlan::Push)
    }

    fn open_media(&mut self, idx: usize) -> Result<()> {
        if let Some(pending) = &self.inflight_media {
            self.last_status = if pending.phase == UiMediaPhase::Active {
                format!("player active {}", pending.label)
            } else if pending.index == idx {
                format!("already opening {}", pending.label)
            } else {
                format!("player is opening {}", pending.label)
            };
            return Ok(());
        }

        let (media_url, label, kind) = {
            let current = self.current.as_ref().context("no current page")?;
            let media = current
                .document
                .media_links
                .get(idx)
                .with_context(|| format!("invalid media index: {}", idx + 1))?;
            let media_url = resolve_optional_url(&media.absolute, &media.src)
                .context("media link has no valid URL")?;
            (
                media_url,
                media.alt_or_label.clone(),
                map_media_kind(media.kind),
            )
        };
        self.validate_url_for_network(&media_url)?;

        let request_id = self.next_request_id();
        self.inflight_media = Some(PendingMedia {
            request_id,
            index: idx,
            label: label.clone(),
            url: media_url.clone(),
            kind,
            phase: UiMediaPhase::Opening,
            release_at: Instant::now() + MEDIA_LAUNCH_GUARD,
            spawned_pid: None,
        });
        self.last_status = format!("opening {} {}", kind.label(), label);

        let tx = self.results_tx.clone();
        let policy = self.media.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("glance-media-launch".to_string())
            .spawn(move || {
                let result = launch_external_player_with_options(
                    &policy.player,
                    &media_url,
                    policy.mode,
                    &policy.allowed_players,
                    &policy.player_args,
                );
                let _ = tx.send(AsyncResult::Media {
                    request_id,
                    index: idx,
                    result,
                });
            })
        {
            self.inflight_media = None;
            anyhow::bail!("failed to start media worker: {err}");
        }
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

    fn reload_dynamic(&mut self) -> Result<()> {
        self.queue_dynamic_render(true)
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
                            let NavigationOutcome {
                                loaded,
                                status,
                                suggest_dynamic,
                            } = outcome;
                            self.apply_history_plan(plan, loaded.url.clone());
                            self.apply_loaded_page(loaded, status);
                            if suggest_dynamic
                                && self.dynamic.mode == DynamicMode::Auto
                                && let Err(err) = self.queue_dynamic_render(false)
                            {
                                self.last_status =
                                    format!("{} | dynamic skipped: {err:#}", self.last_status);
                            }
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
                Ok(AsyncResult::Dynamic {
                    request_id,
                    url,
                    result,
                }) => {
                    let Some(pending) = self.inflight_dynamic.as_ref() else {
                        continue;
                    };
                    if pending.request_id != request_id || pending.url != url {
                        continue;
                    }
                    self.inflight_dynamic = None;
                    if self
                        .current
                        .as_ref()
                        .is_none_or(|current| current.url != url)
                    {
                        continue;
                    }

                    match *result {
                        Ok(page) => {
                            self.web_mode = SessionWebMode::Dynamic;
                            self.apply_loaded_page(
                                LoadedPage {
                                    url,
                                    rendered: page.rendered,
                                    document: page.document,
                                    can_scroll: false,
                                },
                                "loaded dynamic DOM".to_string(),
                            );
                        }
                        Err(err) => {
                            self.web_mode = SessionWebMode::Static;
                            self.last_status = format!("dynamic render error: {err}");
                        }
                    }
                }
                Ok(AsyncResult::Media {
                    request_id,
                    index,
                    result,
                }) => {
                    let Some(pending) = self.inflight_media.as_ref() else {
                        continue;
                    };
                    if pending.request_id != request_id || pending.index != index {
                        continue;
                    }

                    match result {
                        Ok(ExternalPlayerOutcome::Spawned { pid }) => {
                            if let Some(pending) = &mut self.inflight_media {
                                pending.spawned_pid = Some(pid);
                            }
                        }
                        Ok(ExternalPlayerOutcome::Completed) => {
                            let completed = self.inflight_media.take();
                            if self.inflight_nav.is_none()
                                && let Some(completed) = completed
                            {
                                self.last_status = format!("finished {}", completed.label);
                            }
                        }
                        Err(err) => {
                            self.inflight_media = None;
                            if self.inflight_nav.is_none() {
                                self.last_status = format!("player error: {err}");
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        self.drain_companion_events();
        self.finish_media_launch_guard();
    }

    fn drain_companion_events(&mut self) {
        let mut events = Vec::new();
        if let Some(companion) = &self.companion {
            while let Ok(event) = companion.try_recv() {
                events.push(event);
            }
        }

        for event in events {
            match event {
                CompanionEvent::AuthReady { request_id, url } => {
                    let Some(pending) = &mut self.pending_auth else {
                        continue;
                    };
                    if pending.request_id != request_id {
                        continue;
                    }
                    pending.phase = AuthPhase::Waiting;
                    pending.observe_request_id = None;
                    pending.last_observe_at = Instant::now()
                        .checked_sub(AUTH_OBSERVE_INTERVAL)
                        .unwrap_or_else(Instant::now);
                    self.last_status = format!(
                        "approve the passkey or account prompt in the secure browser ({})",
                        display_origin(&url)
                    );
                }
                CompanionEvent::AuthFocused { request_id, url } => {
                    let Some(pending) = &mut self.pending_auth else {
                        continue;
                    };
                    if pending.focus_request_id != Some(request_id) {
                        continue;
                    }
                    pending.focus_request_id = None;
                    self.last_status = match pending.phase {
                        AuthPhase::Opening => {
                            "secure sign-in window focused; waiting for it to open".to_string()
                        }
                        AuthPhase::Waiting => format!(
                            "secure sign-in window focused; continue there ({})",
                            display_origin(&url)
                        ),
                    };
                }
                CompanionEvent::Snapshot {
                    request_id,
                    operation: CompanionOperation::Navigate,
                    snapshot,
                } => {
                    let Some(pending) = self.inflight_companion_nav.as_ref() else {
                        continue;
                    };
                    if pending.request_id != request_id {
                        continue;
                    }
                    let plan = self
                        .inflight_companion_nav
                        .take()
                        .map(|pending| pending.plan)
                        .unwrap_or(HistoryPlan::None);
                    match loaded_from_companion(
                        snapshot,
                        self.render_icon_mode,
                        self.network_policy.sanitize_control_chars,
                    ) {
                        Ok(loaded) => {
                            self.apply_history_plan(plan, loaded.url.clone());
                            self.web_mode = SessionWebMode::Companion;
                            self.apply_loaded_page(
                                loaded,
                                "loaded authenticated browser session".to_string(),
                            );
                        }
                        Err(error) => {
                            self.last_status = format!("secure session render error: {error:#}");
                        }
                    }
                }
                CompanionEvent::Snapshot {
                    request_id,
                    operation: CompanionOperation::Observe,
                    snapshot,
                } => self.handle_auth_observation(request_id, snapshot),
                CompanionEvent::Snapshot {
                    request_id,
                    operation: CompanionOperation::Scroll,
                    snapshot,
                } => self.handle_companion_scroll(request_id, snapshot),
                CompanionEvent::Snapshot {
                    request_id,
                    operation: CompanionOperation::Activate,
                    snapshot,
                } => self.handle_companion_activation(request_id, snapshot),
                CompanionEvent::Snapshot {
                    request_id,
                    operation: CompanionOperation::ActivateLive | CompanionOperation::FillLive,
                    snapshot,
                } => self.handle_live_control_update(request_id, snapshot),
                CompanionEvent::Snapshot {
                    operation: CompanionOperation::Authenticate | CompanionOperation::Focus,
                    ..
                } => {}
                CompanionEvent::Failed {
                    request_id,
                    operation,
                    error,
                } => match operation {
                    CompanionOperation::Authenticate | CompanionOperation::Observe => {
                        let matches_pending = self.pending_auth.as_ref().is_some_and(|pending| {
                            pending.request_id == request_id
                                || pending.observe_request_id == Some(request_id)
                        });
                        if matches_pending {
                            self.pending_auth = None;
                            self.last_status = format!("secure sign-in error: {error}");
                        }
                    }
                    CompanionOperation::Focus => {
                        let matches_pending = self
                            .pending_auth
                            .as_ref()
                            .is_some_and(|pending| pending.focus_request_id == Some(request_id));
                        if matches_pending {
                            if let Some(pending) = &mut self.pending_auth {
                                pending.focus_request_id = None;
                            }
                            self.last_status = format!(
                                "sign-in continues, but its window could not be focused: {error}"
                            );
                        }
                    }
                    CompanionOperation::Navigate => {
                        if self
                            .inflight_companion_nav
                            .as_ref()
                            .is_some_and(|pending| pending.request_id == request_id)
                        {
                            self.inflight_companion_nav = None;
                            self.last_status = format!("secure session navigation error: {error}");
                        }
                    }
                    CompanionOperation::Scroll => {
                        if self
                            .inflight_companion_scroll
                            .as_ref()
                            .is_some_and(|pending| pending.request_id == request_id)
                        {
                            self.inflight_companion_scroll = None;
                            self.last_status = format!("could not extend the live feed: {error}");
                        }
                    }
                    CompanionOperation::Activate => {
                        if self
                            .inflight_companion_action
                            .as_ref()
                            .is_some_and(|pending| pending.request_id == request_id)
                        {
                            self.inflight_companion_action = None;
                            self.last_status = format!("continuation control failed: {error}");
                        }
                    }
                    CompanionOperation::ActivateLive | CompanionOperation::FillLive => {
                        if self
                            .inflight_live_control_action
                            .as_ref()
                            .is_some_and(|pending| pending.request_id == request_id)
                        {
                            self.inflight_live_control_action = None;
                            self.last_status = format!("live control update failed: {error}");
                        }
                    }
                },
            }
        }
    }

    fn handle_companion_scroll(&mut self, request_id: u64, snapshot: CompanionSnapshot) {
        let Some(pending) = self.inflight_companion_scroll.as_ref() else {
            return;
        };
        if pending.request_id != request_id {
            return;
        }
        self.inflight_companion_scroll = None;

        let scroll = snapshot.scroll;
        let loaded = match loaded_from_companion(
            snapshot,
            self.render_icon_mode,
            self.network_policy.sanitize_control_chars,
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.last_status = format!("live feed render error: {error:#}");
                return;
            }
        };
        if let Err(error) = self.validate_url_for_network(&loaded.url) {
            self.last_status = format!("live feed left the allowed network boundary: {error:#}");
            return;
        }

        let same_page = self.current.as_ref().is_some_and(|current| {
            same_origin(&current.url, &loaded.url) && current.url.path() == loaded.url.path()
        });
        if !same_page {
            self.apply_history_plan(HistoryPlan::SetCursor(self.cursor), loaded.url.clone());
            self.companion_feed_capped = false;
            self.apply_loaded_page(loaded, "live page changed while scrolling".to_string());
            return;
        }

        let Some(current) = &mut self.current else {
            return;
        };
        current.can_scroll = loaded.can_scroll;
        let merge = merge_feed_document(&mut current.document, loaded.document);
        current.url = loaded.url;
        current.rendered = render_document(&current.document, self.render_icon_mode);
        self.companion_feed_capped = merge.capped;

        let moved = scroll.is_some_and(|metrics| metrics.after > metrics.before + 0.5);
        let at_edge =
            scroll.is_some_and(|metrics| metrics.after + metrics.viewport >= metrics.extent - 2.0);
        self.last_status = if merge.capped {
            "feed memory limit reached; navigate or reload for a fresh view".to_string()
        } else if merge.new_sections > 0 {
            format!(
                "live feed extended · {} new section{}",
                merge.new_sections,
                if merge.new_sections == 1 { "" } else { "s" }
            )
        } else if at_edge {
            "at the live edge; scroll again to check for newer content".to_string()
        } else if moved {
            "live page advanced; no new readable content appeared yet".to_string()
        } else {
            "live page did not move; it may not expose a scrollable feed".to_string()
        };
    }

    fn handle_companion_activation(&mut self, request_id: u64, snapshot: CompanionSnapshot) {
        let Some(pending) = self.inflight_companion_action.as_ref() else {
            return;
        };
        if pending.request_id != request_id {
            return;
        }
        let label = self
            .inflight_companion_action
            .take()
            .map(|pending| pending.label)
            .unwrap_or_else(|| "continuation control".to_string());
        let loaded = match loaded_from_companion(
            snapshot,
            self.render_icon_mode,
            self.network_policy.sanitize_control_chars,
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.last_status = format!("continuation render error: {error:#}");
                return;
            }
        };
        if let Err(error) = self.validate_url_for_network(&loaded.url) {
            self.last_status = format!("continuation left the network boundary: {error:#}");
            return;
        }

        let same_page = self.current.as_ref().is_some_and(|current| {
            same_origin(&current.url, &loaded.url) && current.url.path() == loaded.url.path()
        });
        if !same_page {
            self.apply_history_plan(HistoryPlan::SetCursor(self.cursor), loaded.url.clone());
            self.companion_feed_capped = false;
            self.apply_loaded_page(loaded, format!("{label} opened a new live page"));
            return;
        }

        let Some(current) = &mut self.current else {
            return;
        };
        current.can_scroll = loaded.can_scroll;
        let merge = merge_feed_document(&mut current.document, loaded.document);
        current.url = loaded.url;
        current.rendered = render_document(&current.document, self.render_icon_mode);
        self.companion_feed_capped = merge.capped;
        self.last_status = if merge.capped {
            "feed memory limit reached; navigate or reload for a fresh view".to_string()
        } else if merge.new_sections > 0 {
            format!("{label} · {} new sections", merge.new_sections)
        } else {
            format!("{label} activated; waiting for readable content")
        };
    }

    fn handle_live_control_update(&mut self, request_id: u64, snapshot: CompanionSnapshot) {
        let Some(pending) = self.inflight_live_control_action.as_ref() else {
            return;
        };
        if pending.request_id != request_id {
            return;
        }
        let label = self
            .inflight_live_control_action
            .take()
            .map(|pending| pending.label)
            .unwrap_or_else(|| "live control".to_string());
        let loaded = match loaded_from_companion(
            snapshot,
            self.render_icon_mode,
            self.network_policy.sanitize_control_chars,
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.last_status = format!("live control render error: {error:#}");
                return;
            }
        };
        if let Err(error) = self.validate_url_for_network(&loaded.url) {
            self.last_status = format!("live control left the network boundary: {error:#}");
            return;
        }

        let same_page = self.current.as_ref().is_some_and(|current| {
            same_origin(&current.url, &loaded.url) && current.url.path() == loaded.url.path()
        });
        if !same_page {
            self.apply_history_plan(HistoryPlan::SetCursor(self.cursor), loaded.url.clone());
            self.companion_feed_capped = false;
            self.apply_loaded_page(loaded, format!("{label} opened a new live page"));
            return;
        }

        self.apply_loaded_page(loaded, format!("{label} · live interface updated"));
    }

    fn handle_auth_observation(&mut self, request_id: u64, snapshot: CompanionSnapshot) {
        let Some(pending) = self.pending_auth.as_ref() else {
            return;
        };
        if pending.observe_request_id != Some(request_id) {
            return;
        }
        let auth_url = pending.auth_url.clone();
        let return_url = pending.return_url.clone();
        if let Some(pending) = &mut self.pending_auth {
            pending.observe_request_id = None;
        }

        let loaded = match loaded_from_companion(
            snapshot,
            self.render_icon_mode,
            self.network_policy.sanitize_control_chars,
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.last_status = format!("secure sign-in status error: {error:#}");
                return;
            }
        };
        if !auth_session_completed(&loaded, &auth_url, &return_url) {
            if let Some(pending) = &mut self.pending_auth {
                pending.completion_url = None;
                pending.completion_observations = 0;
            }
            self.last_status =
                "waiting for passkey or account approval in the secure browser".to_string();
            return;
        }

        if let Some(pending) = &mut self.pending_auth
            && !confirm_auth_completion_candidate(pending, &loaded.url)
        {
            self.last_status =
                "sign-in returned; confirming the session before reconnecting".to_string();
            return;
        }

        self.pending_auth = None;
        self.companion_authenticated = true;
        self.web_mode = SessionWebMode::Companion;
        if is_auth_flow_url(&loaded.url, &auth_url) {
            let plan = if self.history.is_empty() {
                HistoryPlan::None
            } else {
                HistoryPlan::SetCursor(self.cursor)
            };
            if let Err(error) = self.queue_companion_navigation(return_url, plan) {
                self.last_status = format!("signed in, but return navigation failed: {error:#}");
            } else {
                self.last_status = "signed in; returning to the authenticated page".to_string();
            }
        } else {
            let plan = if self.history.is_empty() {
                HistoryPlan::None
            } else {
                HistoryPlan::SetCursor(self.cursor)
            };
            self.apply_history_plan(plan, loaded.url.clone());
            self.apply_loaded_page(loaded, "passkey session connected securely".to_string());
        }
    }

    fn poll_pending_auth(&mut self) {
        let should_observe = self.pending_auth.as_ref().is_some_and(|pending| {
            pending.phase == AuthPhase::Waiting
                && pending.observe_request_id.is_none()
                && Instant::now().saturating_duration_since(pending.last_observe_at)
                    >= AUTH_OBSERVE_INTERVAL
        });
        if !should_observe {
            return;
        }
        let request_id = self.next_request_id();
        let Some(companion) = &self.companion else {
            return;
        };
        if let Err(error) = companion.observe(request_id) {
            self.pending_auth = None;
            self.last_status = format!("secure sign-in status error: {error:#}");
            return;
        }
        if let Some(pending) = &mut self.pending_auth {
            pending.observe_request_id = Some(request_id);
            pending.last_observe_at = Instant::now();
        }
    }

    fn finish_media_launch_guard(&mut self) {
        let expired = self
            .inflight_media
            .as_ref()
            .is_some_and(|pending| Instant::now() >= pending.release_at);
        if !expired {
            return;
        }
        let active_label = self.inflight_media.as_mut().and_then(|pending| {
            if pending.spawned_pid.is_none() && pending.phase == UiMediaPhase::Opening {
                pending.phase = UiMediaPhase::Active;
                Some(pending.label.clone())
            } else {
                None
            }
        });
        if self
            .inflight_media
            .as_ref()
            .is_some_and(|pending| pending.spawned_pid.is_none())
        {
            if self.inflight_nav.is_none()
                && let Some(label) = active_label
            {
                self.last_status = format!("player active {label}");
            }
            return;
        }
        let Some(launched) = self.inflight_media.take() else {
            return;
        };
        if self.inflight_nav.is_none() {
            self.last_status = format!("playing {}", launched.label);
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
        let access_policy = self.network_policy.access.clone();
        self.runtime.spawn(async move {
            let result = net
                .fetch_bytes_limited_with_policy(&url, max_image_bytes, &access_policy)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncResult::Image {
                request_id,
                index,
                result,
            });
        });
    }

    fn queue_dynamic_render(&mut self, forced: bool) -> Result<()> {
        if self.dynamic.mode == DynamicMode::Off {
            anyhow::bail!("dynamic rendering is disabled by config");
        }
        if !forced && self.dynamic.mode != DynamicMode::Auto {
            return Ok(());
        }
        if !self.dynamic.security_compatible {
            anyhow::bail!(
                "dynamic rendering is unavailable while security.allow_private_ip is false"
            );
        }

        if self.companion_authenticated {
            let url = self
                .current
                .as_ref()
                .map(|page| page.url.clone())
                .context("nothing to render dynamically")?;
            return self.queue_companion_navigation(url, HistoryPlan::None);
        }

        let current = self
            .current
            .as_ref()
            .context("nothing to render dynamically")?;
        let url = current.url.clone();
        self.validate_url_for_network(&url)?;
        if !matches!(url.scheme(), "http" | "https") {
            anyhow::bail!("dynamic rendering only supports HTTP(S) pages");
        }
        if self
            .inflight_dynamic
            .as_ref()
            .is_some_and(|pending| pending.url == url)
        {
            self.last_status = "dynamic rendering already in progress".to_string();
            return Ok(());
        }

        let executable = self
            .dynamic_executable
            .clone()
            .context("no supported Chromium browser was found; set dynamic.executable")?;
        let request_id = self.next_request_id();
        self.inflight_dynamic = Some(PendingDynamic {
            request_id,
            url: url.clone(),
        });
        self.web_mode = SessionWebMode::Static;

        let tx = self.results_tx.clone();
        let policy = self.dynamic.clone();
        let render_icon_mode = self.render_icon_mode;
        let max_body_bytes = self.network_policy.max_body_bytes;
        let sanitize_control_chars = self.network_policy.sanitize_control_chars;
        self.runtime.spawn(async move {
            let result = render_with_browser(
                executable,
                url.clone(),
                policy,
                render_icon_mode,
                max_body_bytes,
                sanitize_control_chars,
                request_id,
            )
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(AsyncResult::Dynamic {
                request_id,
                url,
                result: Box::new(result),
            });
        });
        self.last_status = "upgrading with dynamic DOM".to_string();
        Ok(())
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
        if self.private {
            parts.push("PRIVATE".to_string());
        }
        if self.inflight_nav.is_some() {
            parts.push("LOADING".to_string());
        }
        if self.inflight_image.is_some() {
            parts.push("IMG-FETCH".to_string());
        }
        if self.inflight_dynamic.is_some() {
            parts.push("DYNAMIC".to_string());
        }
        if self.inflight_companion_nav.is_some() {
            parts.push("SESSION".to_string());
        }
        if self.inflight_companion_scroll.is_some() {
            parts.push("FEED".to_string());
        }
        if self.inflight_companion_action.is_some() {
            parts.push("ACTION".to_string());
        }
        if self.inflight_live_control_action.is_some() {
            parts.push("CONTROL".to_string());
        }
        if self.pending_auth.is_some() {
            parts.push("AUTH".to_string());
        }
        if self.inflight_media.is_some() {
            parts.push("PLAYER".to_string());
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
        self.inflight_dynamic = None;
        self.web_mode = SessionWebMode::Static;
    }

    fn validate_url_for_network(&self, url: &Url) -> Result<()> {
        validate_url_for_network(url, &self.network_policy.access)
    }

    fn apply_loaded_page(&mut self, loaded: LoadedPage, status: String) {
        self.current = Some(loaded);
        self.selected_image_index = self.default_selected_image_index_for_current();
        self.clear_selected_image_data();
        self.last_status = status;
        self.queue_selected_image_fetch();
    }

    fn ui_web_mode(&self) -> UiWebMode {
        if self.inflight_dynamic.is_some() || self.inflight_companion_nav.is_some() {
            UiWebMode::Upgrading
        } else {
            match self.web_mode {
                SessionWebMode::Static => UiWebMode::Static,
                SessionWebMode::Dynamic => UiWebMode::Dynamic,
                SessionWebMode::Companion => UiWebMode::Session,
            }
        }
    }

    fn ui_media_activity(&self) -> Option<UiMediaActivity> {
        self.inflight_media.as_ref().map(|pending| UiMediaActivity {
            index: pending.index,
            label: pending.label.clone(),
            url: pending.url.to_string(),
            kind: pending.kind,
            phase: pending.phase,
        })
    }

    fn clear_selected_image_data(&mut self) {
        self.selected_image_bytes = None;
        self.selected_image_size = None;
    }
}

fn loaded_from_companion(
    snapshot: CompanionSnapshot,
    render_icon_mode: RenderIconMode,
    sanitize_control_chars: bool,
) -> Result<LoadedPage> {
    anyhow::ensure!(
        matches!(snapshot.url.scheme(), "http" | "https"),
        "secure browser returned a non-HTTP(S) page"
    );
    let document =
        parse_document_with_options(&snapshot.url, &snapshot.html, sanitize_control_chars);
    let rendered = render_document(&document, render_icon_mode);
    Ok(LoadedPage {
        url: snapshot.url,
        rendered,
        document,
        can_scroll: snapshot.can_scroll,
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct FeedMerge {
    new_sections: usize,
    capped: bool,
}

fn merge_feed_document(current: &mut Document, incoming: Document) -> FeedMerge {
    let Document {
        title,
        source_url,
        text_lines,
        links,
        forms,
        continuation_buttons,
        live_controls,
        images,
        media_links,
        script_count,
        meta_http_equiv,
        markdown_blocks,
    } = incoming;

    if !title.trim().is_empty() {
        current.title = title;
    }
    current.source_url = source_url;
    current.script_count = current.script_count.max(script_count);
    current.meta_http_equiv = meta_http_equiv;
    current.continuation_buttons = continuation_buttons;
    current.live_controls = live_controls;

    let (new_lines, lines_capped) =
        append_unique_debug(&mut current.text_lines, text_lines, MAX_FEED_TEXT_LINES);
    let (new_blocks, blocks_capped) = append_unique_debug(
        &mut current.markdown_blocks,
        markdown_blocks,
        MAX_FEED_BLOCKS,
    );
    let (_, links_capped) = append_unique_debug(&mut current.links, links, MAX_FEED_RESOURCES);
    let (_, forms_capped) = append_unique_debug(&mut current.forms, forms, MAX_FEED_RESOURCES);
    let (_, images_capped) = append_unique_debug(&mut current.images, images, MAX_FEED_RESOURCES);
    let (_, media_capped) =
        append_unique_debug(&mut current.media_links, media_links, MAX_FEED_RESOURCES);

    FeedMerge {
        new_sections: if current.markdown_blocks.is_empty() {
            new_lines
        } else {
            new_blocks
        },
        capped: lines_capped
            || blocks_capped
            || links_capped
            || forms_capped
            || images_capped
            || media_capped,
    }
}

fn append_unique_debug<T: std::fmt::Debug>(
    target: &mut Vec<T>,
    incoming: Vec<T>,
    limit: usize,
) -> (usize, bool) {
    let mut known = target
        .iter()
        .map(|item| format!("{item:?}"))
        .collect::<HashSet<_>>();
    let mut added = 0;
    let mut capped = false;
    for item in incoming {
        let key = format!("{item:?}");
        if !known.insert(key) {
            continue;
        }
        if target.len() >= limit {
            capped = true;
            break;
        }
        target.push(item);
        added += 1;
    }
    (added, capped)
}

fn auth_session_completed(loaded: &LoadedPage, auth_url: &Url, return_url: &Url) -> bool {
    if same_origin(&loaded.url, return_url) && document_has_sign_out(&loaded.document) {
        return true;
    }
    same_origin(&loaded.url, return_url)
        && !is_auth_flow_url(&loaded.url, auth_url)
        && find_auth_link_index(&loaded.document).is_none()
}

fn confirm_auth_completion_candidate(pending: &mut PendingAuth, url: &Url) -> bool {
    let same_candidate = pending
        .completion_url
        .as_ref()
        .is_some_and(|candidate| same_origin(candidate, url) && candidate.path() == url.path());
    if same_candidate {
        pending.completion_observations = pending.completion_observations.saturating_add(1);
    } else {
        pending.completion_url = Some(url.clone());
        pending.completion_observations = 1;
    }
    pending.completion_observations >= 2
}

fn document_has_sign_out(document: &Document) -> bool {
    const MARKERS: [&str; 8] = [
        "logout",
        "log out",
        "signout",
        "sign out",
        "로그아웃",
        "ログアウト",
        "退出登录",
        "登出",
    ];
    document.links.iter().any(|link| {
        let label = link.text.to_lowercase();
        let href = link.href.to_ascii_lowercase();
        MARKERS
            .iter()
            .any(|marker| label.contains(marker) || href.contains(marker))
    }) || document.text_lines.iter().any(|line| {
        let line = line.to_lowercase();
        MARKERS.iter().any(|marker| line.contains(marker))
    })
}

fn is_auth_flow_url(url: &Url, auth_url: &Url) -> bool {
    if same_origin(url, auth_url) && url.path() == auth_url.path() {
        return true;
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    host == "accounts.google.com"
        || host == "nid.naver.com"
        || path.contains("/login")
        || path.contains("/signin")
        || path.contains("/sign-in")
        || path == "/sign"
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn display_origin(url: &Url) -> String {
    url.host_str().unwrap_or(url.as_str()).to_string()
}

fn build_link_items(doc: &Document) -> Vec<UiResourceItem> {
    doc.links
        .iter()
        .map(|link| UiResourceItem {
            label: link.text.clone(),
            url: link
                .absolute
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| link.href.clone()),
        })
        .collect()
}

fn build_live_control_items(doc: &Document) -> Vec<UiLiveControlItem> {
    doc.live_controls
        .iter()
        .map(|control| UiLiveControlItem {
            label: control.label.clone(),
            context: control.context.clone(),
            value: control.value.clone(),
            placeholder: control.placeholder.clone(),
            kind: match control.kind {
                LiveControlKind::Disclosure => UiLiveControlKind::Disclosure,
                LiveControlKind::Action => UiLiveControlKind::Action,
                LiveControlKind::Submit => UiLiveControlKind::Submit,
                LiveControlKind::Field => UiLiveControlKind::Field,
            },
            field_kind: control.field_kind.map(|kind| match kind {
                LiveFieldKind::Text => UiLiveFieldKind::Text,
                LiveFieldKind::TextArea => UiLiveFieldKind::TextArea,
                LiveFieldKind::Checkbox => UiLiveFieldKind::Checkbox,
                LiveFieldKind::Radio => UiLiveFieldKind::Radio,
                LiveFieldKind::Select => UiLiveFieldKind::Select,
            }),
            checked: control.checked,
            options: control
                .options
                .iter()
                .map(|option| UiLiveOption {
                    label: option.label.clone(),
                    value: option.value.clone(),
                    selected: option.selected,
                    disabled: option.disabled,
                })
                .collect(),
            requires_confirmation: control.kind.requires_confirmation(),
            multiline: control.multiline,
        })
        .collect()
}

fn find_auth_link_index(doc: &Document) -> Option<usize> {
    doc.links
        .iter()
        .enumerate()
        .filter_map(|(index, link)| {
            auth_link_score(&link.text, link.absolute.as_ref(), &link.href)
                .map(|score| (score, index))
        })
        .max_by_key(|(score, index)| (*score, std::cmp::Reverse(*index)))
        .map(|(_, index)| index)
}

fn auth_link_score(label: &str, absolute: Option<&Url>, raw_href: &str) -> Option<u16> {
    let label = label.trim().to_lowercase();
    let href = absolute
        .map(Url::as_str)
        .unwrap_or(raw_href)
        .to_ascii_lowercase();
    let negative = [
        "logout",
        "log out",
        "signout",
        "sign out",
        "로그아웃",
        "signup",
        "sign up",
        "회원가입",
        "register",
    ];
    if negative
        .iter()
        .any(|needle| label.contains(needle) || href.contains(needle))
    {
        return None;
    }

    let exact_labels = [
        "login",
        "log in",
        "signin",
        "sign in",
        "로그인",
        "ログイン",
        "登录",
        "登入",
    ];
    if exact_labels.iter().any(|candidate| label == *candidate) {
        return Some(120);
    }
    if exact_labels
        .iter()
        .any(|candidate| label.contains(candidate))
    {
        return Some(100);
    }

    let auth_paths = [
        "/login",
        "/log-in",
        "/signin",
        "/sign-in",
        "/sign?",
        "servicelogin",
        "nidlogin",
        "accounts.google.",
        "/oauth/authorize",
    ];
    auth_paths
        .iter()
        .any(|needle| href.contains(needle))
        .then_some(70)
}

fn normalized_auth_label(label: &str) -> String {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        "Sign in".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_image_items(doc: &Document) -> Vec<UiResourceItem> {
    doc.images
        .iter()
        .map(|img| UiResourceItem {
            label: img.alt_or_label.clone(),
            url: img
                .absolute
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| img.src.clone()),
        })
        .collect()
}

fn build_media_items(doc: &Document) -> Vec<UiMediaItem> {
    doc.media_links
        .iter()
        .map(|m| UiMediaItem {
            label: m.alt_or_label.clone(),
            url: m
                .absolute
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| m.src.clone()),
            kind: map_media_kind(m.kind),
            mime_type: m.mime_type.clone(),
        })
        .collect()
}

fn map_media_kind(kind: MediaKind) -> UiMediaKind {
    match kind {
        MediaKind::Audio => UiMediaKind::Audio,
        MediaKind::Video => UiMediaKind::Video,
        MediaKind::Stream => UiMediaKind::Stream,
        MediaKind::Image | MediaKind::Media => UiMediaKind::Media,
    }
}

async fn fetch_and_build_async(
    net: NetClient,
    render_icon_mode: RenderIconMode,
    url: Url,
    network_policy: NetworkPolicy,
) -> Result<NavigationOutcome> {
    let mut current_url = url;
    let mut meta_refresh_hops = 0usize;

    loop {
        validate_url_for_network(&current_url, &network_policy.access)?;
        let fetch = net
            .fetch_limited_with_policy(
                &current_url,
                network_policy.max_body_bytes,
                &network_policy.access,
            )
            .await?;
        let parse_as_html = should_parse_as_html(fetch.status, fetch.content_type.as_deref());

        if parse_as_html {
            let mut document = parse_document_with_options(
                &fetch.final_url,
                &fetch.body,
                network_policy.sanitize_control_chars,
            );
            // Server HTML cannot opt itself into companion-only button execution.
            document.continuation_buttons.clear();
            document.live_controls.clear();
            let suggest_dynamic = looks_like_client_rendered_shell(&document, &fetch.body);
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

            let rendered = render_document(&document, render_icon_mode);

            let loaded = LoadedPage {
                url: fetch.final_url.clone(),
                rendered,
                document,
                can_scroll: false,
            };

            return Ok(NavigationOutcome {
                loaded,
                status: build_loaded_status(&fetch, meta_refresh_hops, "loaded"),
                suggest_dynamic,
            });
        }

        let rendered = render_non_html_page(&fetch);
        let document = empty_document(fetch.final_url.clone(), "Non-HTML response");
        let loaded = LoadedPage {
            url: fetch.final_url.clone(),
            rendered,
            document,
            can_scroll: false,
        };
        return Ok(NavigationOutcome {
            loaded,
            status: build_loaded_status(&fetch, meta_refresh_hops, "loaded (non-html)"),
            suggest_dynamic: false,
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
        continuation_buttons: vec![],
        live_controls: vec![],
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
            title: "New tab".to_string(),
            source_url: url.to_string(),
            lines: vec![
                "# New tab".to_string(),
                String::new(),
                "Open an address or search  ·  o / Ctrl-L".to_string(),
                "Shortcuts  ·  ?".to_string(),
            ],
            links: vec![],
            media: vec![],
            interactive_spans: vec![],
        },
        document: empty_document(url, "Blank Page"),
        can_scroll: false,
    }
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

fn build_get_form_url(form: &Form, value: &str) -> Result<Url> {
    if !form.method.eq_ignore_ascii_case("GET") {
        anyhow::bail!(
            "{} forms are not supported yet; only safe GET submissions are enabled",
            form.method
        );
    }
    let editable_index = form
        .fields
        .iter()
        .position(|field| field.editable)
        .context("form has no editable text field")?;
    if form.fields[editable_index].required && value.trim().is_empty() {
        anyhow::bail!("{} is required", form.fields[editable_index].label);
    }
    let mut target = form
        .action_absolute
        .clone()
        .context("form action is not a valid URL")?;
    target.set_query(None);
    {
        let mut query = target.query_pairs_mut();
        for (field_index, field) in form.fields.iter().enumerate() {
            let field_value = if field_index == editable_index {
                value
            } else {
                field.value.as_str()
            };
            query.append_pair(&field.name, field_value);
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::{
        AuthPhase, Browser, BrowserOptions, LoadedPage, PendingAuth, auth_session_completed,
        build_get_form_url, confirm_auth_completion_candidate, find_auth_link_index,
        merge_feed_document,
    };
    use crate::companion::CompanionProfile;
    use crate::policy::{DynamicMode, DynamicPolicy, MediaPolicy, NetworkPolicy};
    use glance_config::UiIconMode as ConfigUiIconMode;
    use glance_dom::{Form, FormField, parse_document};
    use glance_media::ExternalPlayerLaunchMode;
    use glance_net::{NetClient, NetworkAccessPolicy};
    use glance_render::{RenderIconMode, render_document};
    use glance_ui::UiCommand;
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;
    use url::Url;

    #[test]
    fn builds_get_form_query_with_hidden_defaults() {
        let form = Form {
            method: "GET".to_string(),
            action: "/search".to_string(),
            action_absolute: Some(Url::parse("https://example.com/search?old=1").expect("url")),
            fields: vec![
                FormField {
                    name: "lang".to_string(),
                    value: "ko".to_string(),
                    label: "lang".to_string(),
                    kind: "hidden".to_string(),
                    editable: false,
                    required: false,
                },
                FormField {
                    name: "q".to_string(),
                    value: String::new(),
                    label: "Search".to_string(),
                    kind: "search".to_string(),
                    editable: true,
                    required: true,
                },
            ],
            label: "Search".to_string(),
        };

        let target = build_get_form_url(&form, "rust tui").expect("target");
        assert_eq!(
            target.as_str(),
            "https://example.com/search?lang=ko&q=rust+tui"
        );
    }

    #[test]
    fn finds_multilingual_login_links_without_confusing_registration() {
        let url = Url::parse("https://example.com/").expect("url");
        let doc = parse_document(
            &url,
            r#"
                <a href="/register">회원가입</a>
                <a href="/help">도움말</a>
                <a href="/sign?next=%2F">로그인</a>
            "#,
        );

        assert_eq!(find_auth_link_index(&doc), Some(2));
    }

    #[test]
    fn finds_provider_login_from_accessible_label() {
        let url = Url::parse("https://www.naver.com/").expect("url");
        let doc = parse_document(
            &url,
            r#"<a aria-label="NAVER 로그인" href="https://nid.naver.com/nidlogin.login"></a>"#,
        );

        assert_eq!(find_auth_link_index(&doc), Some(0));
    }

    #[test]
    fn feed_snapshots_append_only_new_readable_content_and_resources() {
        let url = Url::parse("https://social.example/home").expect("url");
        let mut first = parse_document(
            &url,
            r#"<main><article><h2>Alice</h2><p>First post</p><a href="/a">Open A</a></article></main>"#,
        );
        let second = parse_document(
            &url,
            r#"<main>
                <article><h2>Alice</h2><p>First post</p><a href="/a">Open A</a></article>
                <article><h2>Bob</h2><p>Second post</p><a href="/b">Open B</a></article>
            </main>"#,
        );

        let merged = merge_feed_document(&mut first, second);
        let rendered = render_document(&first, RenderIconMode::Ascii);
        assert!(merged.new_sections > 0);
        assert!(!merged.capped);
        assert_eq!(first.links.len(), 2);
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.contains("First post"))
        );
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.contains("Second post"))
        );
    }

    #[test]
    fn authenticated_return_requires_the_expected_origin() {
        let auth_url = Url::parse("https://hackers.pub/sign?next=%2Flocal").expect("auth url");
        let return_url = Url::parse("https://hackers.pub/local").expect("return url");
        let signed_in_url = Url::parse("https://hackers.pub/local").expect("signed in url");
        let document = parse_document(
            &signed_in_url,
            r#"<a href="/@me">My account</a><a href="/logout">로그아웃</a>"#,
        );
        let loaded = LoadedPage {
            url: signed_in_url,
            rendered: render_document(&document, RenderIconMode::Ascii),
            document,
            can_scroll: false,
        };
        assert!(auth_session_completed(&loaded, &auth_url, &return_url));

        let attacker_url = Url::parse("https://example.net/local").expect("attacker url");
        let document = parse_document(&attacker_url, "<p>Welcome</p>");
        let loaded = LoadedPage {
            url: attacker_url,
            rendered: render_document(&document, RenderIconMode::Ascii),
            document,
            can_scroll: false,
        };
        assert!(!auth_session_completed(&loaded, &auth_url, &return_url));
    }

    #[test]
    fn authentication_requires_two_stable_completion_observations() {
        let auth_url = Url::parse("https://example.com/sign").expect("auth url");
        let return_url = Url::parse("https://example.com/feed").expect("return url");
        let mut pending = PendingAuth {
            request_id: 1,
            observe_request_id: None,
            focus_request_id: None,
            auth_url,
            return_url,
            phase: AuthPhase::Waiting,
            last_observe_at: Instant::now(),
            completion_url: None,
            completion_observations: 0,
        };
        let candidate = Url::parse("https://example.com/feed").expect("candidate");
        assert!(!confirm_auth_completion_candidate(&mut pending, &candidate));
        assert!(confirm_auth_completion_candidate(&mut pending, &candidate));

        let changed = Url::parse("https://example.com/account").expect("changed");
        assert!(!confirm_auth_completion_candidate(&mut pending, &changed));
    }

    #[test]
    fn sign_in_page_without_session_does_not_complete_authentication() {
        let auth_url = Url::parse("https://hackers.pub/sign?next=%2Flocal").expect("auth url");
        let return_url = Url::parse("https://hackers.pub/local").expect("return url");
        let document = parse_document(
            &auth_url,
            r#"<button>패스키로 로그인</button><a href="/sign">로그인</a>"#,
        );
        let loaded = LoadedPage {
            url: auth_url.clone(),
            rendered: render_document(&document, RenderIconMode::Ascii),
            document,
            can_scroll: false,
        };
        assert!(!auth_session_completed(&loaded, &auth_url, &return_url));
    }

    #[cfg(unix)]
    #[test]
    fn synchronous_player_launch_does_not_block_and_repeated_requests_are_coalesced() {
        let runtime = Runtime::new().expect("runtime");
        let net = NetClient::new("glance-test", 2_000, 3, None).expect("network client");
        let options = BrowserOptions {
            icon_mode: ConfigUiIconMode::Ascii,
            media: MediaPolicy {
                player: "/bin/sh".to_string(),
                mode: ExternalPlayerLaunchMode::Sync,
                player_args: vec![
                    "-c".to_string(),
                    "sleep 0.3".to_string(),
                    "glance-player-fixture".to_string(),
                    "{url}".to_string(),
                ],
                allowed_players: vec!["/bin/sh".to_string()],
                images_enabled: false,
            },
            dynamic: DynamicPolicy {
                mode: DynamicMode::Off,
                executable: None,
                wait_ms: 0,
                timeout_ms: 1_000,
                security_compatible: true,
            },
            network: NetworkPolicy {
                access: NetworkAccessPolicy::new(
                    vec!["http".to_string(), "https".to_string()],
                    true,
                )
                .expect("access policy"),
                max_body_bytes: 1024,
                max_image_bytes: 1024,
                sanitize_control_chars: true,
            },
            search_template: "https://example.com/?q={query}".to_string(),
            private: false,
            companion_profile: CompanionProfile::Ephemeral,
        };
        let mut browser = Browser::new(runtime, net, options);
        let url = Url::parse("https://example.com/").expect("page url");
        let document = parse_document(
            &url,
            r#"<video aria-label="Demo video" src="https://example.com/demo.mp4"></video>"#,
        );
        let rendered = render_document(&document, RenderIconMode::Ascii);
        browser.current = Some(LoadedPage {
            url,
            rendered,
            document,
            can_scroll: false,
        });

        let started = Instant::now();
        let page = browser.handle_command(UiCommand::OpenMedia(0));
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(page.media_activity.is_some());
        let first_request_id = browser
            .inflight_media
            .as_ref()
            .expect("pending launch")
            .request_id;

        let page = browser.handle_command(UiCommand::OpenMedia(0));
        assert_eq!(
            browser
                .inflight_media
                .as_ref()
                .expect("same pending launch")
                .request_id,
            first_request_id
        );
        assert!(page.status.contains("already opening"));

        browser
            .inflight_media
            .as_mut()
            .expect("pending sync player")
            .release_at = Instant::now() - Duration::from_millis(1);
        browser.finish_media_launch_guard();
        assert_eq!(
            browser
                .to_ui_page()
                .media_activity
                .expect("active sync player")
                .phase,
            glance_ui::UiMediaPhase::Active
        );

        std::thread::sleep(Duration::from_millis(400));
        let page = browser.handle_command(UiCommand::Poll);
        assert!(page.media_activity.is_none());
    }
}
