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

#[derive(Debug, Clone)]
pub struct UiImageItem {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct UiPage {
    pub history_entry_id: u64,
    pub rendered: RenderedPage,
    pub status: String,
    pub js_note: String,
    pub icon_mode: UiIconMode,
    pub image_backend: ImageBackend,
    pub image_mode: UiImageMode,
    pub js_default_enabled: bool,
    pub images: Vec<UiImageItem>,
    pub media: Vec<UiImageItem>,
    pub selected_image_index: Option<usize>,
    pub selected_image_bytes: Option<Vec<u8>>,
    pub selected_image_size: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    Poll,
    OpenUrl(String),
    OpenLink(usize),
    OpenMedia(usize),
    SelectImage(usize),
    Reload,
    Back,
    Forward,
    ToggleJsDefault,
    ToggleImageMode,
}
