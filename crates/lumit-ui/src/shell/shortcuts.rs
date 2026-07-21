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
                    // Honours the 0–100 sensitivity slider (docs/09 §5; the
                    // slider itself lives in the timeline's context menu —
                    // native menus can't host one).
                    #[cfg(feature = "media")]
                    if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                        let delta =
                            lumit_audio::beat::delta_from_sensitivity(self.app.beat_sensitivity);
                        self.app.detect_beats(id, delta);
                    }
                }
                MenuAction::DetectBeatsMore => {
                    // "More markers": the slider's setting nudged 20 points up.
                    #[cfg(feature = "media")]
                    if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                        let delta = lumit_audio::beat::delta_from_sensitivity(
                            self.app.beat_sensitivity.saturating_add(20).min(100),
                        );
                        self.app.detect_beats(id, delta);
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
                MenuAction::OpenSettings => self.settings_open = true,
            }
        }
    }

    /// Ctrl/Cmd+C copies the selected lane keyframes; Ctrl/Cmd+V pastes them at
    /// the playhead (note 2.2). Handled on both platforms. Skipped while a text
    /// field is focused, so its own copy/paste keeps working; otherwise there is
    /// no other copy/paste consumer, so it is safe to take the gesture. A clicked
    /// lane-key glyph does not hold keyboard focus (see
    /// `clicking_a_lane_key_glyph_does_not_hold_focus`), so having a keyframe
    /// selection never trips the focus guard.
    pub(super) fn keyframe_clipboard_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused()).is_some() {
            return;
        }
        let (copy, paste) = keyframe_clipboard_intent(ctx);
        if copy {
            self.app.copy_selected_keyframes();
        }
        if paste {
            self.app.paste_keyframes();
        }
    }

    /// Cross-platform global shortcuts (both platforms, unlike the non-macOS
    /// menu accelerators in [`Self::shortcuts`] / the macOS native menu):
    /// Shift+F3 graph editor (§5), Cmd/Ctrl+D duplicate (§4.7), `=`/`-`/`\`
    /// timeline zoom (§4.6), and `[`/`]`/Alt+`[`/`]` layer span edits (§4.7).
    /// Skipped while a text field holds focus so typing is never stolen.
    pub(super) fn global_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        if ctx.memory(|m| m.focused()).is_some() {
            return;
        }
        const GRAPH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
        if ctx.input_mut(|i| i.consume_shortcut(&GRAPH)) {
            self.app.timeline_graph_mode = !self.app.timeline_graph_mode;
        }
        // Cmd/Ctrl+D duplicates the selected layer (docs/07-UI-SPEC §4.7, the AE
        // convention). Only consumed when a layer is selected, so it is a clean
        // no-op otherwise rather than flashing an error. Razor is Ctrl+Shift+D,
        // a different chord.
        const DUPLICATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::D);
        if self.app.selected_layer.is_some() && ctx.input_mut(|i| i.consume_shortcut(&DUPLICATE)) {
            self.app.duplicate_layer();
        }
        // Timeline zoom (docs/07-UI-SPEC §4.6): `=`/`Shift+=` zoom in, `-` zooms
        // out, `\` fits. Same 1.4× steps and 1..400% clamp as the bottom bar's
        // zoom buttons. Read (not consumed) — no other reader claims these keys.
        let (zoom_in, zoom_out, zoom_fit) = ctx.input(|i| {
            (
                i.key_pressed(Key::Equals) || i.key_pressed(Key::Plus),
                i.key_pressed(Key::Minus),
                i.key_pressed(Key::Backslash),
            )
        });
        if zoom_in {
            self.app.timeline_zoom = (self.app.timeline_zoom * 1.4).min(400.0);
        }
        if zoom_out {
            self.app.timeline_zoom = (self.app.timeline_zoom / 1.4).max(1.0);
        }
        if zoom_fit {
            self.app.timeline_zoom = 1.0;
        }
        // Layer span edits (docs/07-UI-SPEC §4.7): `[`/`]` move the selected
        // layer's in/out to the playhead; Alt+`[`/`]` trim that edge. Only when a
        // layer is selected. The Alt (trim) chord is checked before the plain
        // (move) one so the more-specific binding wins.
        if self.app.selected_layer.is_some() {
            use lumit_core::ops::SpanEdit;
            const MOVE_IN: KeyboardShortcut =
                KeyboardShortcut::new(Modifiers::NONE, Key::OpenBracket);
            const MOVE_OUT: KeyboardShortcut =
                KeyboardShortcut::new(Modifiers::NONE, Key::CloseBracket);
            const TRIM_IN: KeyboardShortcut =
                KeyboardShortcut::new(Modifiers::ALT, Key::OpenBracket);
            const TRIM_OUT: KeyboardShortcut =
                KeyboardShortcut::new(Modifiers::ALT, Key::CloseBracket);
            if ctx.input_mut(|i| i.consume_shortcut(&TRIM_IN)) {
                self.app.edit_selected_layer_span(SpanEdit::TrimIn);
            } else if ctx.input_mut(|i| i.consume_shortcut(&MOVE_IN)) {
                self.app.edit_selected_layer_span(SpanEdit::MoveIn);
            }
            if ctx.input_mut(|i| i.consume_shortcut(&TRIM_OUT)) {
                self.app.edit_selected_layer_span(SpanEdit::TrimOut);
            } else if ctx.input_mut(|i| i.consume_shortcut(&MOVE_OUT)) {
                self.app.edit_selected_layer_span(SpanEdit::MoveOut);
            }
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

/// The lane keyframe copy/paste gesture this frame, taken from the input events.
///
/// UI-7: egui-winit translates Ctrl/Cmd+C and Ctrl/Cmd+V (and the dedicated
/// Copy/Paste keys) into [`egui::Event::Copy`] / [`egui::Event::Paste`] and never
/// emits a `Key::C` / `Key::V` press for them — so the previous wiring, which
/// read `consume_key(COMMAND, Key::C/V)`, could never fire and paste did nothing.
/// This reads the actual clipboard events and consumes them, so no other reader
/// acts on the same gesture. Returns `(copy, paste)`.
pub(crate) fn keyframe_clipboard_intent(ctx: &egui::Context) -> (bool, bool) {
    ctx.input_mut(|i| {
        let mut copy = false;
        let mut paste = false;
        i.events.retain(|e| match e {
            egui::Event::Copy => {
                copy = true;
                false
            }
            egui::Event::Paste(_) => {
                paste = true;
                false
            }
            _ => true,
        });
        (copy, paste)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Drive one headless frame with `events` and return the copy/paste gesture
    /// `keyframe_clipboard_intent` reads from it.
    fn intent_from(events: Vec<egui::Event>) -> (bool, bool) {
        let ctx = egui::Context::default();
        let out = std::cell::Cell::new((false, false));
        let ri = egui::RawInput {
            events,
            ..Default::default()
        };
        let _ = ctx.run(ri, |ctx| {
            out.set(keyframe_clipboard_intent(ctx));
        });
        out.get()
    }

    /// UI-7 regression: Ctrl/Cmd+C reaches egui as `Event::Copy`, not a `Key::C`
    /// press, so the gesture must be read from the clipboard event. Before the
    /// fix the wiring watched `Key::C`, so this arrived as nothing and copy never
    /// ran.
    #[test]
    fn a_copy_event_is_read_as_the_copy_gesture() {
        assert_eq!(intent_from(vec![egui::Event::Copy]), (true, false));
    }

    /// Likewise Ctrl/Cmd+V arrives as `Event::Paste`, never a `Key::V` press.
    #[test]
    fn a_paste_event_is_read_as_the_paste_gesture() {
        assert_eq!(
            intent_from(vec![egui::Event::Paste(String::new())]),
            (false, true)
        );
    }

    /// A bare `Key::C` press (no clipboard translation) is NOT a copy gesture —
    /// this is exactly what the old `consume_key(Key::C)` wiring keyed on, and
    /// why it never fired for a real Ctrl+C.
    #[test]
    fn a_plain_key_c_press_is_not_a_copy_gesture() {
        let key_c = egui::Event::Key {
            key: egui::Key::C,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        assert_eq!(intent_from(vec![key_c]), (false, false));
    }

    /// The gesture is consumed, so a text field drawn later in the frame does not
    /// also act on the same Copy/Paste.
    #[test]
    fn reading_the_gesture_consumes_the_events() {
        let ctx = egui::Context::default();
        let remaining = std::cell::Cell::new(usize::MAX);
        let ri = egui::RawInput {
            events: vec![egui::Event::Copy, egui::Event::Paste(String::new())],
            ..Default::default()
        };
        let _ = ctx.run(ri, |ctx| {
            let _ = keyframe_clipboard_intent(ctx);
            remaining.set(ctx.input(|i| i.events.len()));
        });
        assert_eq!(remaining.get(), 0, "the clipboard events must be consumed");
    }
}
