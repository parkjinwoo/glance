use std::collections::HashMap;
use std::ops::Range;

use ratatui::layout::Rect;

use glance_media::InlineImagePlacement;

use crate::UiPage;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum FocusPane {
    Address,
    Content,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum AddressMode {
    Nav,
    Edit,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UiLayout {
    pub(crate) title: Rect,
    pub(crate) address: Rect,
    pub(crate) content: Rect,
    pub(crate) footer: Rect,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VisualLine {
    pub(crate) logical_line_idx: usize,
    pub(crate) start_col: u16,
    pub(crate) end_col: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct AddressHitboxes {
    pub(crate) back: Range<u16>,
    pub(crate) forward: Range<u16>,
    pub(crate) reload: Range<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct FooterHitboxes {
    pub(crate) js: Range<u16>,
    pub(crate) img: Range<u16>,
}

pub(crate) struct App {
    pub(crate) page: UiPage,
    pub(crate) content_scroll: u16,
    pub(crate) cursor_visual_row: usize,
    pub(crate) view_state_by_entry: HashMap<u64, ViewState>,
    pub(crate) focus: FocusPane,
    pub(crate) address_mode: AddressMode,
    pub(crate) command: String,
    pub(crate) click_feedback: Option<ClickFeedback>,
    pub(crate) image_viewer_open: bool,
    pub(crate) last_image_placement: Option<InlineImagePlacement>,
    pub(crate) needs_image_clear: bool,
    pub(crate) image_dirty: bool,
    pub(crate) last_left_down: Option<(u16, u16)>,
}

pub(crate) struct ClickFeedback {
    pub(crate) message: String,
    pub(crate) ticks_left: u8,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewState {
    pub(crate) content_scroll: u16,
    pub(crate) cursor_visual_row: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ScrollAccumulator {
    pub(crate) content_delta: i16,
    pub(crate) set_content_focus: bool,
}

impl App {
    pub(crate) fn new(page: UiPage) -> Self {
        Self {
            page,
            content_scroll: 0,
            cursor_visual_row: 0,
            view_state_by_entry: HashMap::new(),
            focus: FocusPane::Content,
            address_mode: AddressMode::Nav,
            command: String::new(),
            click_feedback: None,
            image_viewer_open: false,
            last_image_placement: None,
            needs_image_clear: false,
            image_dirty: true,
            last_left_down: None,
        }
    }

    pub(crate) fn reset_view(&mut self) {
        self.content_scroll = 0;
    }

    pub(crate) fn set_click_feedback(&mut self, message: impl Into<String>) {
        self.click_feedback = Some(ClickFeedback {
            message: message.into(),
            ticks_left: 10,
        });
    }

    pub(crate) fn tick_feedback(&mut self) {
        if let Some(fb) = &mut self.click_feedback {
            if fb.ticks_left == 0 {
                self.click_feedback = None;
            } else {
                fb.ticks_left = fb.ticks_left.saturating_sub(1);
            }
        }
    }

    pub(crate) fn effective_status(&self) -> &str {
        if let Some(fb) = &self.click_feedback {
            fb.message.as_str()
        } else {
            self.page.status.as_str()
        }
    }

    pub(crate) fn set_focus(&mut self, focus: FocusPane) {
        self.focus = focus;
        if focus != FocusPane::Address {
            self.leave_address_edit();
        }
    }

    pub(crate) fn enter_address_edit(&mut self, prefill: String) {
        self.focus = FocusPane::Address;
        self.address_mode = AddressMode::Edit;
        self.command = prefill;
    }

    pub(crate) fn leave_address_edit(&mut self) {
        self.address_mode = AddressMode::Nav;
        self.command.clear();
    }

    pub(crate) fn open_image_viewer(&mut self) {
        self.image_viewer_open = true;
        self.image_dirty = true;
    }

    pub(crate) fn close_image_viewer(&mut self) {
        self.image_viewer_open = false;
        self.needs_image_clear = true;
        self.image_dirty = false;
    }
}
