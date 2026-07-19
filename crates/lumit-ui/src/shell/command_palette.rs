//! The command palette (docs/07-UI-SPEC.md §12): a searchable launcher for
//! application commands, opened with Ctrl/Cmd+Shift+P.
//!
//! In plain terms: press one shortcut, start typing, and a short list of
//! matching commands appears — run one with Enter or a click, without hunting
//! through menus. It covers global actions (save, undo, new composition, add a
//! layer, switch the colour scheme, open Settings, export). It is deliberately
//! NOT the effects radial menu (that applies effects to a clip under the
//! cursor, a separate future surface) — this is the app-wide command list.
//!
//! It reuses `egui::Modal`, like the Settings dialog: a dimmed backdrop that
//! blocks the app behind it, drawn on top. All colours come from the theme.

use super::*;

/// One thing the palette can do. Variants that need a compiled-in feature
/// (export needs `media`) are gated so a headless build still compiles.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum PaletteAction {
    Undo,
    Redo,
    Save,
    NewProject,
    OpenProject,
    ImportFootage,
    NewComposition,
    AddSolidLayer,
    AddTextLayer,
    AddCameraLayer,
    AddAdjustmentLayer,
    AddSequenceLayer,
    DuplicateLayer,
    DeleteLayer,
    /// Key every selected property row at the playhead (note 2.6) — the
    /// multi-select "key selected" path.
    KeySelectedProps,
    ResetWorkspace,
    OpenSettings,
    SetScheme(crate::theme::ColorScheme),
    SetShape(crate::theme::ThemeShape),
    /// Apply a built-in effect (by `match_name`) to the selected layer — the
    /// palette's "effects" category (docs/07 §12). Only offered when a layer
    /// is selected.
    ApplyEffect(&'static str),
    #[cfg(feature = "media")]
    ExportComp,
}

/// A command with its display label and extra search keywords (words a user
/// might type that are not in the label — e.g. "prefs" for Settings).
pub(crate) struct PaletteCommand {
    pub action: PaletteAction,
    pub label: String,
    pub keywords: &'static str,
}

impl Shell {
    /// The (composition, layer) an effect would apply to, if a layer is
    /// selected in a real composition.
    fn selected_layer_target(&self) -> Option<(uuid::Uuid, uuid::Uuid)> {
        let comp = self.app.selected_comp?;
        let layer = self.app.selected_layer?;
        self.app
            .store
            .snapshot()
            .comp(comp)?
            .layers
            .iter()
            .any(|l| l.id == layer)
            .then_some((comp, layer))
    }

    /// Open the palette: cleared query, first row selected, search focused.
    // Only the non-macOS shortcut handler and the in-window Window menu call this;
    // on macOS the palette is not yet wired into the native menu, so the method is
    // (validly) unused there — annotate rather than fail the macOS clippy job.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn open_command_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_sel = 0;
        self.palette_focus = true;
    }

    /// The full command list. Order here is the fallback order shown for an
    /// empty query; a query re-ranks by match score.
    fn palette_commands(&self) -> Vec<PaletteCommand> {
        fn cmd(action: PaletteAction, label: &str, keywords: &'static str) -> PaletteCommand {
            PaletteCommand {
                action,
                label: label.to_owned(),
                keywords,
            }
        }
        let mut v = vec![
            cmd(PaletteAction::Save, "Save project", "write disk store"),
            cmd(PaletteAction::Undo, "Undo", "revert back"),
            cmd(PaletteAction::Redo, "Redo", "forward again"),
            cmd(
                PaletteAction::NewComposition,
                "New composition",
                "comp create",
            ),
            cmd(
                PaletteAction::AddSolidLayer,
                "Add solid layer",
                "colour fill",
            ),
            cmd(PaletteAction::AddTextLayer, "Add text layer", "type title"),
            cmd(PaletteAction::AddCameraLayer, "Add camera layer", "3d view"),
            cmd(
                PaletteAction::AddAdjustmentLayer,
                "Add adjustment layer",
                "effect stack",
            ),
            cmd(
                PaletteAction::AddSequenceLayer,
                "Add sequence layer",
                "clips row",
            ),
            cmd(
                PaletteAction::DuplicateLayer,
                "Duplicate layer",
                "copy clone selected",
            ),
            cmd(
                PaletteAction::DeleteLayer,
                "Delete layer",
                "remove selected",
            ),
            cmd(
                PaletteAction::KeySelectedProps,
                "Key selected properties",
                "keyframe add selection playhead multi",
            ),
            cmd(PaletteAction::NewProject, "New project", "blank"),
            cmd(PaletteAction::OpenProject, "Open project…", "load file"),
            cmd(
                PaletteAction::ImportFootage,
                "Import footage…",
                "media video clip",
            ),
            cmd(
                PaletteAction::OpenSettings,
                "Open settings",
                "preferences prefs performance appearance",
            ),
            cmd(
                PaletteAction::ResetWorkspace,
                "Reset workspace",
                "panels layout default",
            ),
        ];
        #[cfg(feature = "media")]
        v.push(cmd(
            PaletteAction::ExportComp,
            "Export composition…",
            "render output video mp4",
        ));
        for scheme in crate::theme::ColorScheme::ALL {
            v.push(cmd(
                PaletteAction::SetScheme(scheme),
                &format!("Theme: {}", scheme.label()),
                "colour scheme appearance dark light",
            ));
        }
        v.push(cmd(
            PaletteAction::SetShape(crate::theme::ThemeShape::Sharp),
            "Shape: sharp",
            "panels square edge",
        ));
        v.push(cmd(
            PaletteAction::SetShape(crate::theme::ThemeShape::Round),
            "Shape: round",
            "panels cards floating",
        ));
        // Effects apply to the selected layer, so only offer them when there
        // is one (docs/07 §12 effects category).
        if self.selected_layer_target().is_some() {
            for schema in lumit_core::fx::BUILTINS {
                v.push(cmd(
                    PaletteAction::ApplyEffect(schema.match_name),
                    &format!("Apply effect: {}", schema.label),
                    "effect fx add filter",
                ));
            }
        }
        v
    }

    /// Run one command, then close the palette.
    fn run_palette(&mut self, action: PaletteAction, ctx: &egui::Context) {
        match action {
            PaletteAction::Undo => self.app.undo(),
            PaletteAction::Redo => self.app.redo(),
            PaletteAction::Save => self.app.save(),
            PaletteAction::NewProject => self.app.new_project(),
            PaletteAction::OpenProject => self.app.open_dialog(),
            PaletteAction::ImportFootage => self.app.import_footage_dialog(),
            PaletteAction::NewComposition => self.app.new_composition(),
            PaletteAction::AddSolidLayer => self.app.add_solid_layer(),
            PaletteAction::AddTextLayer => self.app.add_text_layer(),
            PaletteAction::AddCameraLayer => self.app.add_camera_layer(),
            PaletteAction::AddAdjustmentLayer => self.app.add_adjustment_layer(),
            PaletteAction::AddSequenceLayer => self.app.add_sequence_layer(),
            PaletteAction::DuplicateLayer => self.app.duplicate_layer(),
            PaletteAction::DeleteLayer => self.app.delete_selected_layer(),
            PaletteAction::KeySelectedProps => self.app.key_selected_props(),
            PaletteAction::ResetWorkspace => self.dock = default_layout(),
            PaletteAction::OpenSettings => self.settings_open = true,
            PaletteAction::SetScheme(scheme) => {
                self.color_scheme = scheme;
                self.recompose(ctx);
            }
            PaletteAction::SetShape(shape) => {
                self.theme_shape = shape;
                self.recompose(ctx);
            }
            PaletteAction::ApplyEffect(name) => {
                if let Some((comp, layer_id)) = self.selected_layer_target() {
                    let doc = self.app.store.snapshot();
                    let current = doc
                        .comp(comp)
                        .and_then(|c| c.layers.iter().find(|l| l.id == layer_id))
                        .map(|l| l.effects.clone());
                    if let (Some(mut effects), Some(inst)) =
                        (current, lumit_core::fx::instantiate(name))
                    {
                        effects.push(inst);
                        self.app.commit(lumit_core::Op::SetLayerEffects {
                            comp,
                            layer: layer_id,
                            effects,
                        });
                        #[cfg(feature = "media")]
                        self.app.refresh_preview();
                    }
                }
            }
            #[cfg(feature = "media")]
            PaletteAction::ExportComp => {
                self.open_export_dialog(crate::export::ExportPreset::Custom)
            }
        }
        self.palette_open = false;
    }

    /// Draw the palette when open. Mirrors `settings_modal`: an `egui::Modal`
    /// for the dimmed, click-blocking, on-top behaviour, anchored near the top.
    pub(crate) fn command_palette_modal(&mut self, ctx: &egui::Context) {
        if !self.palette_open {
            return;
        }
        let theme = self.theme;
        // Rank the commands against the current query.
        let commands = self.palette_commands();
        let query = self.palette_query.clone();
        let mut ranked: Vec<(i32, usize)> = commands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| fuzzy_score(&query, &c.label, c.keywords).map(|score| (score, i)))
            .collect();
        // Highest score first; ties keep list order (stable by index).
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let visible: Vec<usize> = ranked.iter().map(|(_, i)| *i).collect();
        if self.palette_sel >= visible.len() {
            self.palette_sel = visible.len().saturating_sub(1);
        }

        // Keyboard: up/down move the selection, Enter runs it, Escape closes.
        let mut chosen: Option<PaletteAction> = None;
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) && !visible.is_empty() {
                self.palette_sel = (self.palette_sel + 1).min(visible.len() - 1);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                self.palette_sel = self.palette_sel.saturating_sub(1);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                if let Some(&idx) = visible.get(self.palette_sel) {
                    chosen = Some(commands[idx].action);
                }
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.palette_open = false;
            }
        });

        let area = egui::Modal::default_area(egui::Id::new("lumit-command-palette"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 96.0));
        let modal = egui::Modal::new(egui::Id::new("lumit-command-palette"))
            .area(area)
            .show(ctx, |ui| {
                ui.set_width(PALETTE_WIDTH);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .hint_text("Type a command…")
                        .desired_width(f32::INFINITY),
                );
                if self.palette_focus {
                    field.request_focus();
                    self.palette_focus = false;
                }
                // Typing changes the ranking; keep the top row selected.
                if field.changed() {
                    self.palette_sel = 0;
                }
                ui.add_space(4.0);
                if visible.is_empty() {
                    ui.label(
                        egui::RichText::new("No matching commands")
                            .small()
                            .color(theme.text_muted),
                    );
                }
                egui::ScrollArea::vertical()
                    .max_height(PALETTE_LIST_HEIGHT)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (row, &idx) in visible.iter().enumerate() {
                            let selected = row == self.palette_sel;
                            let label =
                                egui::RichText::new(&commands[idx].label).color(if selected {
                                    theme.text_primary
                                } else {
                                    theme.text_secondary
                                });
                            if ui
                                .add_sized(
                                    [ui.available_width(), 24.0],
                                    egui::SelectableLabel::new(selected, label),
                                )
                                .clicked()
                            {
                                chosen = Some(commands[idx].action);
                            }
                        }
                    });
            });

        if let Some(action) = chosen {
            self.run_palette(action, ctx);
        } else if modal.should_close() {
            self.palette_open = false;
        }
    }
}

/// The palette's fixed width.
const PALETTE_WIDTH: f32 = 460.0;
/// The command-list viewport height (it scrolls beyond this).
const PALETTE_LIST_HEIGHT: f32 = 320.0;

/// Score how well `query` matches a command, or `None` if it does not.
/// An empty query matches everything (score 0, so list order stands). A
/// non-empty query must appear as a case-insensitive subsequence of the label
/// or the keywords; a run in the label scores higher than one in the keywords,
/// and an earlier, more contiguous match scores higher.
pub(crate) fn fuzzy_score(query: &str, label: &str, keywords: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let label_score = subsequence_score(&q, &label.to_lowercase());
    let keyword_score = subsequence_score(&q, &keywords.to_lowercase()).map(|s| s - 50);
    match (label_score, keyword_score) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// `Some(score)` if every char of `q` appears in order within `text`. Rewards
/// matches that start early and stay contiguous.
fn subsequence_score(q: &str, text: &str) -> Option<i32> {
    let mut chars = text.char_indices();
    let mut score = 100;
    let mut last_pos: Option<usize> = None;
    for qc in q.chars() {
        let found = chars.by_ref().find(|(_, tc)| *tc == qc)?;
        if let Some(prev) = last_pos {
            // Penalise gaps between matched characters.
            score -= (found.0 - prev - 1) as i32;
        } else {
            // Penalise a match that starts late.
            score -= found.0 as i32;
        }
        last_pos = Some(found.0);
    }
    Some(score)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(fuzzy_score("", "Anything", "keywords"), Some(0));
        assert_eq!(fuzzy_score("   ", "Anything", ""), Some(0));
    }

    #[test]
    fn a_subsequence_matches_and_a_non_match_does_not() {
        assert!(fuzzy_score("save", "Save project", "").is_some());
        assert!(fuzzy_score("svp", "Save project", "").is_some()); // subsequence
        assert!(fuzzy_score("xyz", "Save project", "").is_none());
    }

    #[test]
    fn a_label_match_outranks_a_keywords_only_match() {
        let on_label = fuzzy_score("export", "Export composition…", "render output").unwrap();
        let on_keyword = fuzzy_score("render", "Export composition…", "render output").unwrap();
        assert!(
            on_label > on_keyword,
            "label {on_label} should beat keyword {on_keyword}"
        );
    }

    #[test]
    fn an_earlier_contiguous_match_scores_higher() {
        // "new" is contiguous at the start of "New composition" and scattered
        // in "Interview scene", so the former should score higher.
        let early = fuzzy_score("new", "New composition", "").unwrap();
        let late = fuzzy_score("new", "Interview scene", "").unwrap();
        assert!(early > late, "early {early} vs late {late}");
    }

    #[test]
    fn keyword_search_finds_a_command_whose_label_lacks_the_word() {
        // "prefs" is only in Open settings' keywords.
        assert!(fuzzy_score("prefs", "Open settings", "preferences prefs").is_some());
    }
}
