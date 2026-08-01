use std::collections::HashMap;
use std::ops::Range;

use ratatui::layout::Rect;

use glance_media::InlineImagePlacement;
use glance_render::InteractiveAction;

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
    Find,
    Form(usize),
    LiveField(usize),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UiLayout {
    pub(crate) title: Rect,
    pub(crate) address: Rect,
    pub(crate) content: Rect,
    pub(crate) sidebar: Option<Rect>,
    pub(crate) footer: Rect,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SidebarAction {
    Auth,
    Image(usize),
    Media(usize),
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
    pub(crate) dynamic: Range<u16>,
    pub(crate) external: Range<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct FooterHitboxes {
    pub(crate) web: Range<u16>,
    pub(crate) auth: Option<Range<u16>>,
    pub(crate) external: Range<u16>,
    pub(crate) img: Range<u16>,
    pub(crate) media: Range<u16>,
    pub(crate) help: Range<u16>,
}

pub(crate) struct App {
    pub(crate) page: UiPage,
    pub(crate) content_scroll: u16,
    pub(crate) cursor_visual_row: usize,
    pub(crate) view_state_by_entry: HashMap<u64, ViewState>,
    pub(crate) focus: FocusPane,
    pub(crate) address_mode: AddressMode,
    pub(crate) command: String,
    pub(crate) command_cursor: usize,
    pub(crate) find_query: String,
    pub(crate) find_matches: Vec<usize>,
    pub(crate) find_current: Option<usize>,
    pub(crate) click_feedback: Option<ClickFeedback>,
    pub(crate) hover_feedback: Option<String>,
    pub(crate) hovered_action: Option<InteractiveAction>,
    pub(crate) hovered_sidebar_action: Option<SidebarAction>,
    pub(crate) help_open: bool,
    pub(crate) image_viewer_open: bool,
    pub(crate) media_picker_open: bool,
    pub(crate) selected_media_index: usize,
    pub(crate) hovered_media_index: Option<usize>,
    pub(crate) last_image_placement: Option<InlineImagePlacement>,
    pub(crate) needs_image_clear: bool,
    pub(crate) image_dirty: bool,
    pub(crate) last_left_down: Option<(u16, u16)>,
    pub(crate) mouse_dragged: bool,
    pub(crate) scrollbar_dragging: bool,
    pub(crate) animation_tick: u64,
    pub(crate) update_notice: Option<UpdateNotice>,
    pub(crate) control_confirmation: Option<ControlConfirmation>,
    pub(crate) live_choice_editor: Option<LiveChoiceEditor>,
}

pub(crate) struct ClickFeedback {
    pub(crate) message: String,
    pub(crate) ticks_left: u8,
}

pub(crate) struct UpdateNotice {
    pub(crate) message: String,
    pub(crate) failed: bool,
    pub(crate) ticks_left: u8,
}

#[derive(Debug, Clone)]
pub(crate) struct ControlConfirmation {
    pub(crate) index: usize,
    pub(crate) label: String,
    pub(crate) context: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LiveChoiceEditor {
    pub(crate) control_index: usize,
    pub(crate) selected_index: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewState {
    pub(crate) content_scroll: u16,
    pub(crate) cursor_visual_row: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ScrollAccumulator {
    pub(crate) content_delta: i16,
    pub(crate) media_delta: i16,
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
            command_cursor: 0,
            find_query: String::new(),
            find_matches: Vec::new(),
            find_current: None,
            click_feedback: None,
            hover_feedback: None,
            hovered_action: None,
            hovered_sidebar_action: None,
            help_open: false,
            image_viewer_open: false,
            media_picker_open: false,
            selected_media_index: 0,
            hovered_media_index: None,
            last_image_placement: None,
            needs_image_clear: false,
            image_dirty: true,
            last_left_down: None,
            mouse_dragged: false,
            scrollbar_dragging: false,
            animation_tick: 0,
            update_notice: None,
            control_confirmation: None,
            live_choice_editor: None,
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
        self.animation_tick = self.animation_tick.wrapping_add(1);
        if let Some(fb) = &mut self.click_feedback {
            if fb.ticks_left == 0 {
                self.click_feedback = None;
            } else {
                fb.ticks_left = fb.ticks_left.saturating_sub(1);
            }
        }
        if let Some(notice) = &mut self.update_notice {
            if notice.ticks_left == 0 {
                self.update_notice = None;
            } else {
                notice.ticks_left = notice.ticks_left.saturating_sub(1);
            }
        }
    }

    pub(crate) fn set_update_notice(&mut self, message: impl Into<String>, failed: bool) {
        self.update_notice = Some(UpdateNotice {
            message: message.into(),
            failed,
            ticks_left: 28,
        });
    }

    pub(crate) fn effective_status(&self) -> &str {
        if let Some(fb) = &self.click_feedback {
            fb.message.as_str()
        } else if let Some(hover) = &self.hover_feedback {
            hover.as_str()
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
        self.command_cursor = self.command.chars().count();
    }

    pub(crate) fn enter_command_edit(&mut self) {
        self.enter_address_edit(String::new());
    }

    pub(crate) fn enter_find_edit(&mut self) {
        self.focus = FocusPane::Address;
        self.address_mode = AddressMode::Find;
        self.command = self.find_query.clone();
        self.command_cursor = self.command.chars().count();
    }

    pub(crate) fn enter_form_edit(&mut self, index: usize) {
        self.focus = FocusPane::Address;
        self.address_mode = AddressMode::Form(index);
        self.command.clear();
        self.command_cursor = 0;
    }

    pub(crate) fn enter_live_field_edit(&mut self, index: usize, value: String) {
        self.close_live_choice_editor();
        self.focus = FocusPane::Address;
        self.address_mode = AddressMode::LiveField(index);
        self.command = value;
        self.command_cursor = self.command.chars().count();
    }

    pub(crate) fn request_control_confirmation(
        &mut self,
        index: usize,
        label: String,
        context: Option<String>,
    ) {
        self.help_open = false;
        self.close_live_choice_editor();
        self.close_media_picker();
        if self.image_viewer_open {
            self.close_image_viewer();
        }
        self.control_confirmation = Some(ControlConfirmation {
            index,
            label,
            context,
        });
    }

    pub(crate) fn open_live_choice_editor(&mut self, control_index: usize) {
        let selected_index = self
            .page
            .live_controls
            .get(control_index)
            .and_then(|control| control.options.iter().position(|option| option.selected))
            .unwrap_or(0);
        self.help_open = false;
        self.close_media_picker();
        if self.image_viewer_open {
            self.close_image_viewer();
        }
        self.live_choice_editor = Some(LiveChoiceEditor {
            control_index,
            selected_index,
        });
    }

    pub(crate) fn close_live_choice_editor(&mut self) {
        self.live_choice_editor = None;
    }

    pub(crate) fn move_live_choice(&mut self, delta: isize) {
        let Some(editor) = self.live_choice_editor.as_mut() else {
            return;
        };
        let Some(control) = self.page.live_controls.get(editor.control_index) else {
            return;
        };
        if control.options.is_empty() {
            editor.selected_index = 0;
            return;
        }
        let len = control.options.len();
        let mut candidate = editor.selected_index;
        for _ in 0..len {
            candidate = if delta < 0 {
                if candidate == 0 {
                    len - 1
                } else {
                    candidate - 1
                }
            } else {
                (candidate + 1) % len
            };
            if !control.options[candidate].disabled {
                editor.selected_index = candidate;
                break;
            }
        }
    }

    pub(crate) fn move_live_choice_to_boundary(&mut self, end: bool) {
        let Some(editor) = self.live_choice_editor.as_mut() else {
            return;
        };
        let Some(control) = self.page.live_controls.get(editor.control_index) else {
            return;
        };
        let candidate = if end {
            control.options.iter().rposition(|option| !option.disabled)
        } else {
            control.options.iter().position(|option| !option.disabled)
        };
        if let Some(index) = candidate {
            editor.selected_index = index;
        }
    }

    pub(crate) fn cancel_control_confirmation(&mut self) {
        self.control_confirmation = None;
    }

    pub(crate) fn leave_address_edit(&mut self) {
        self.address_mode = AddressMode::Nav;
        self.command.clear();
        self.command_cursor = 0;
    }

    pub(crate) fn clear_find(&mut self) {
        self.find_query.clear();
        self.find_matches.clear();
        self.find_current = None;
    }

    pub(crate) fn open_image_viewer(&mut self) {
        self.help_open = false;
        self.close_live_choice_editor();
        self.close_media_picker();
        self.image_viewer_open = true;
        self.image_dirty = true;
    }

    pub(crate) fn close_image_viewer(&mut self) {
        self.image_viewer_open = false;
        self.needs_image_clear = true;
        self.image_dirty = false;
    }

    pub(crate) fn open_media_picker(&mut self) -> bool {
        if self.page.media.is_empty() {
            self.set_click_feedback("no playable media on this page");
            return false;
        }
        if self.image_viewer_open {
            self.close_image_viewer();
        }
        self.help_open = false;
        self.close_live_choice_editor();
        self.selected_media_index = self
            .selected_media_index
            .min(self.page.media.len().saturating_sub(1));
        self.media_picker_open = true;
        self.hovered_media_index = None;
        true
    }

    pub(crate) fn open_help(&mut self) {
        self.close_media_picker();
        self.close_live_choice_editor();
        if self.image_viewer_open {
            self.close_image_viewer();
        }
        self.help_open = true;
    }

    pub(crate) fn close_help(&mut self) {
        self.help_open = false;
    }

    pub(crate) fn close_media_picker(&mut self) {
        self.media_picker_open = false;
        self.hovered_media_index = None;
    }

    pub(crate) fn move_media_selection(&mut self, delta: isize) {
        let len = self.page.media.len();
        if len == 0 {
            self.selected_media_index = 0;
            return;
        }
        self.selected_media_index = self
            .selected_media_index
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
    }

    pub(crate) fn clear_hover(&mut self) {
        self.hover_feedback = None;
        self.hovered_action = None;
        self.hovered_sidebar_action = None;
        self.hovered_media_index = None;
    }
}
