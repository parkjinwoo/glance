use glance_media::ImageBackend;
use glance_render::RenderedPage;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UiImageMode {
    Auto,
    Text,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UiIconMode {
    Auto,
    Ascii,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UiWebMode {
    Static,
    Upgrading,
    Dynamic,
    Session,
}

impl UiWebMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Static => "STATIC",
            Self::Upgrading => "SYNCING",
            Self::Dynamic => "DYNAMIC",
            Self::Session => "SESSION",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UiResourceItem {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct UiAuthItem {
    pub link_index: usize,
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UiLiveControlKind {
    Disclosure,
    Action,
    Submit,
    Field,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UiLiveFieldKind {
    Text,
    TextArea,
    Checkbox,
    Radio,
    Select,
}

#[derive(Debug, Clone)]
pub struct UiLiveOption {
    pub label: String,
    pub value: String,
    pub selected: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub struct UiLiveControlItem {
    pub label: String,
    pub context: Option<String>,
    pub value: String,
    pub placeholder: Option<String>,
    pub kind: UiLiveControlKind,
    pub field_kind: Option<UiLiveFieldKind>,
    pub checked: bool,
    pub options: Vec<UiLiveOption>,
    pub requires_confirmation: bool,
    pub multiline: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UiMediaKind {
    Audio,
    Video,
    Stream,
    Media,
}

impl UiMediaKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "AUDIO",
            Self::Video => "VIDEO",
            Self::Stream => "STREAM",
            Self::Media => "MEDIA",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UiMediaItem {
    pub label: String,
    pub url: String,
    pub kind: UiMediaKind,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UiMediaActivity {
    pub index: usize,
    pub label: String,
    pub url: String,
    pub kind: UiMediaKind,
    pub phase: UiMediaPhase,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UiMediaPhase {
    Opening,
    Active,
}

#[derive(Debug, Clone)]
pub struct UiPage {
    pub history_entry_id: u64,
    pub rendered: RenderedPage,
    pub status: String,
    pub icon_mode: UiIconMode,
    pub web_mode: UiWebMode,
    pub image_backend: ImageBackend,
    pub image_mode: UiImageMode,
    pub links: Vec<UiResourceItem>,
    pub auth: Option<UiAuthItem>,
    pub live_controls: Vec<UiLiveControlItem>,
    pub images: Vec<UiResourceItem>,
    pub media: Vec<UiMediaItem>,
    pub media_activity: Option<UiMediaActivity>,
    pub can_load_more: bool,
    pub loading_more: bool,
    pub updating_live_controls: bool,
    pub selected_image_index: Option<usize>,
    pub selected_image_bytes: Option<Vec<u8>>,
    pub selected_image_size: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    Poll,
    OpenUrl(String),
    OpenExternal(String),
    OpenAuth(String),
    LoadMore,
    OpenLink(usize),
    SubmitForm(usize, String),
    ActivateButton(usize),
    FillLiveField(usize, String),
    ToggleLiveField(usize),
    ActivateLiveControl(usize),
    OpenMedia(usize),
    SelectImage(usize),
    Reload,
    ReloadDynamic,
    Back,
    Forward,
    ToggleImageMode,
}
