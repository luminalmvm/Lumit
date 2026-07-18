//! Keyboard shortcuts and the native macOS menu bar (docs/07-UI-SPEC).
//! These helpers are driven from `Shell::ui` each frame.

use super::*;

impl Shell {
    #[cfg(target_os = "macos")]
    pub(super) fn native_menu_frame(&mut self) {
        use crate::native_menu::MenuAction;
        let Some(menu) = &self.native_menu else {
            return;
        };
        let actions = menu.poll();
        let (can_undo, can_redo) = (self.app.store.can_undo(), self.app.store.can_redo());
        menu.sync(can_undo, can_redo);
        for action in actions {
            match action {
                MenuAction::NewProject => self.app.new_project(),
                MenuAction::OpenProject => self.app.open_dialog(),
                MenuAction::ImportFootage => self.app.import_footage_dialog(),
                MenuAction::Save => self.app.save(),
                MenuAction::ExportComp => {
                    // A generic "Export…" action stamps the Settings →
                    // Export default preset (K-119); an explicit preset
                    // pick below always keeps its own preset regardless.
                    #[cfg(feature = "media")]
                    self.open_export_dialog(self.settings_export.default_preset);
                }
                MenuAction::ExportYouTube1080 => {
                    #[cfg(feature = "media")]
                    self.open_export_dialog(crate::export::ExportPreset::Youtube1080p60);
                }
                MenuAction::ExportVertical => {
                    #[cfg(feature = "media")]
                    self.open_export_dialog(crate::export::ExportPreset::Vertical1080p60);
                }
                MenuAction::ShareExport50 => {
                    #[cfg(feature = "media")]
                    self.start_share_export(50.0);
                }
                MenuAction::ShareExport10 => {
                    #[cfg(feature = "media")]
                    self.start_share_export(10.0);
                }
                MenuAction::Undo => self.app.undo(),
                MenuAction::Redo => self.app.redo(),
                MenuAction::NewComposition => self.app.new_composition(),
                MenuAction::AddSolidLayer => self.app.add_solid_layer(),
                MenuAction::AddTextLayer => self.app.add_text_layer(),
                MenuAction::AddCameraLayer => self.app.add_camera_layer(),
                MenuAction::AddAdjustmentLayer => self.app.add_adjustment_layer(),
                MenuAction::AddSequenceLayer => self.app.add_sequence_layer(),
                MenuAction::CutClip => self.app.cut_sequence_at_playhead(),
                MenuAction::DeleteClip => self.app.delete_clip_at_playhead(),
                MenuAction::AddMarker => self.app.add_marker_at_playhead(),
                MenuAction::ClearBeatMarkers => self.app.clear_beat_markers(),
                MenuAction::DetectBeats => {
                    #[cfg(feature = "media")]
                    if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                        self.app.detect_beats(id, 1.5);
                    }
                }
                MenuAction::DetectBeatsMore => {
                    #[cfg(feature = "media")]
                    if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                        self.app.detect_beats(id, 1.1);
                    }
                }
                MenuAction::AddMaskRectangle => self.add_mask_to_selected(ShapeKind::Rectangle),
                MenuAction::AddMaskEllipse => self.add_mask_to_selected(ShapeKind::Ellipse),
                MenuAction::AddMaskStar => self.add_mask_to_selected(ShapeKind::Star),
                MenuAction::CompSettings => {
                    if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                        self.app.open_comp_settings(id);
                    }
                }
                MenuAction::ResetWorkspace => self.dock = default_layout(),
            }
        }
    }

    /// Ctrl/Cmd+C copies the selected lane keyframes; Ctrl/Cmd+V pastes them at
    /// the playhead (note 2.2). Handled on both platforms. Skipped while a text
    /// field is focused, so its own copy/paste keeps working; otherwise there is
    /// no other C/V binding, so it is safe to consume.
    pub(super) fn keyframe_clipboard_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused()).is_some() {
            return;
        }
        let (copy, paste) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::C),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::V),
            )
        });
        if copy {
            self.app.copy_selected_keyframes();
        }
        if paste {
            self.app.paste_keyframes();
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        const REDO: KeyboardShortcut =
            KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
        const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        // The macOS-standard Settings shortcut (Cmd/Ctrl+comma).
        const SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
        // Command palette (Cmd/Ctrl+Shift+P, per docs/07-UI-SPEC §12/§15).
        const PALETTE: KeyboardShortcut =
            KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::P);
        // Order matters: consume the more-modified shortcut first.
        if ctx.input_mut(|i| i.consume_shortcut(&REDO)) {
            self.app.redo();
        } else if ctx.input_mut(|i| i.consume_shortcut(&UNDO)) {
            self.app.undo();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.app.save();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SETTINGS)) {
            self.settings_open = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&PALETTE)) {
            self.open_command_palette();
        }
    }
}
