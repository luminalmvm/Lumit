//! Modal dialogues driven from the shell: adding a mask to the selected
//! layer, the composition settings dialogue, the recovery prompt, and the
//! shared "is any modal up" check.

use super::*;

impl Shell {
    /// The composition settings dialogue (create + edit — K-068).
    /// Add a mask of `kind` to the selected layer, centred (the menu path;
    /// the toolbar's shape tool is the draw-a-box path).
    pub(super) fn add_mask_to_selected(&mut self, kind: ShapeKind) {
        let doc = self.app.store.snapshot();
        let Some(comp_id) = self.app.selected_comp else {
            self.app.error = Some("select a composition first".into());
            return;
        };
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer_id) = self.app.selected_layer else {
            self.app.error = Some("select a layer first".into());
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let (w, h) = mask_space(layer, &self.app, comp);
        let mask = match kind {
            ShapeKind::Rectangle => {
                lumit_core::mask::Mask::rectangle(w * 0.25, h * 0.25, w * 0.5, h * 0.5)
            }
            ShapeKind::Ellipse => {
                lumit_core::mask::Mask::ellipse(w * 0.5, h * 0.5, w * 0.3, h * 0.3)
            }
            ShapeKind::Star => {
                lumit_core::mask::Mask::star(w * 0.5, h * 0.5, w * 0.32, w * 0.14, 5)
            }
        };
        let mut masks = layer.masks.clone();
        masks.push(mask);
        self.app.commit(lumit_core::Op::SetLayerMasks {
            comp: comp_id,
            layer: layer_id,
            masks,
        });
        #[cfg(feature = "media")]
        self.app.refresh_preview();
    }

    pub(super) fn comp_dialog_modal(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.app.comp_dialog else {
            return;
        };
        let creating = dialog.editing.is_none();
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(if creating {
            "New composition"
        } else {
            "Composition settings"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            let theme = &self.theme;
            egui::Grid::new("comp-dialog")
                .num_columns(2)
                .spacing(egui::vec2(12.0, 8.0))
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(220.0));
                    ui.end_row();

                    // Width × Height on one line, with a ratio lock.
                    ui.label("Size");
                    ui.horizontal(|ui| {
                        let (w_resp, w_buf) = text_field(
                            ui,
                            egui::Id::new("comp-w"),
                            &dialog.width.to_string(),
                            60.0,
                        );
                        ui.label(egui::RichText::new("×").color(theme.text_muted));
                        let (h_resp, h_buf) = text_field(
                            ui,
                            egui::Id::new("comp-h"),
                            &dialog.height.to_string(),
                            60.0,
                        );
                        if w_resp.lost_focus() {
                            if let Ok(w) = w_buf.trim().parse::<u32>() {
                                dialog.width = w.clamp(16, 16384);
                                if dialog.lock_ratio {
                                    dialog.height =
                                        ((f64::from(dialog.width) / dialog.aspect).round() as u32)
                                            .clamp(16, 16384);
                                } else {
                                    dialog.aspect =
                                        f64::from(dialog.width) / f64::from(dialog.height).max(1.0);
                                }
                            }
                        }
                        if h_resp.lost_focus() {
                            if let Ok(h) = h_buf.trim().parse::<u32>() {
                                dialog.height = h.clamp(16, 16384);
                                if dialog.lock_ratio {
                                    dialog.width =
                                        ((f64::from(dialog.height) * dialog.aspect).round() as u32)
                                            .clamp(16, 16384);
                                } else {
                                    dialog.aspect =
                                        f64::from(dialog.width) / f64::from(dialog.height).max(1.0);
                                }
                            }
                        }
                        let lock = dialog.lock_ratio;
                        if icon_button(
                            ui,
                            theme,
                            if lock { Icon::Lock } else { Icon::Unlock },
                            lock,
                        )
                        .on_hover_text("Lock aspect ratio")
                        .clicked()
                        {
                            dialog.lock_ratio = !lock;
                            dialog.aspect =
                                f64::from(dialog.width) / f64::from(dialog.height).max(1.0);
                        }
                        ui.label(
                            egui::RichText::new(aspect_ratio_label(dialog.width, dialog.height))
                                .small()
                                .monospace()
                                .color(theme.text_muted),
                        );
                    });
                    ui.end_row();

                    // Frame rate: free text, plus a preset dropdown (arbitrary
                    // values such as 29.9997 are accepted).
                    ui.label("Frame rate");
                    ui.horizontal(|ui| {
                        let shown = format!("{:.4}", dialog.fps);
                        let shown = shown.trim_end_matches('0').trim_end_matches('.');
                        let (resp, buf) = text_field(ui, egui::Id::new("comp-fps"), shown, 72.0);
                        if resp.lost_focus() {
                            if let Ok(f) = buf.trim().parse::<f64>() {
                                dialog.fps = f.clamp(1.0, 1000.0);
                            }
                        }
                        ui.label(egui::RichText::new("fps").small().color(theme.text_muted));
                        bare_dropdown(ui, "Presets", |ui| {
                            for preset in
                                [23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0, 120.0]
                            {
                                if ui.button(format!("{preset}")).clicked() {
                                    dialog.fps = preset;
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                    ui.end_row();

                    // Duration as HH:MM:SS:mmm.
                    ui.label("Duration");
                    let (d_resp, d_buf) = text_field(
                        ui,
                        egui::Id::new("comp-dur"),
                        &fmt_duration(dialog.duration_s),
                        110.0,
                    );
                    if d_resp.lost_focus() {
                        if let Some(secs) = parse_duration(&d_buf) {
                            dialog.duration_s = secs.clamp(0.04, 86_400.0);
                        }
                    }
                    ui.end_row();
                });
            // Motion blur (K-120): master enable + shutter, shown when editing
            // an existing comp (a fresh comp starts with it off).
            if !creating {
                ui.add_space(6.0);
                ui.separator();
                ui.checkbox(&mut dialog.motion_blur.enabled, "Motion blur")
                    .on_hover_text("Blur layers whose own motion-blur switch is set");
                if dialog.motion_blur.enabled {
                    egui::Grid::new("comp-mb-grid")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Shutter angle");
                            ui.add(
                                egui::DragValue::new(&mut dialog.motion_blur.shutter_angle)
                                    .range(0.0..=720.0)
                                    .suffix("\u{00b0}"),
                            );
                            ui.end_row();
                            ui.label("Shutter phase");
                            ui.add(
                                egui::DragValue::new(&mut dialog.motion_blur.shutter_phase)
                                    .range(-360.0..=360.0)
                                    .suffix("\u{00b0}"),
                            );
                            ui.end_row();
                            ui.label("Samples");
                            ui.add(
                                egui::DragValue::new(&mut dialog.motion_blur.samples).range(2..=64),
                            );
                            ui.end_row();
                        });
                }
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Duration is HH:MM:SS:mmm.")
                    .small()
                    .color(self.theme.text_disabled),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(if creating { "Create" } else { "Apply" })
                    .clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    confirm = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if confirm {
            self.app.confirm_comp_dialog();
        } else if cancel {
            self.app.comp_dialog = None;
        }
    }

    /// True while any modal dialog is up (Settings, the composition or export
    /// dialogs, or the recovery prompt). Callers that read the raw pointer —
    /// the active-panel focus edge — skip themselves so a click meant for the
    /// dialog never reaches a panel behind it.
    pub(super) fn any_modal_open(&self) -> bool {
        if self.settings_open
            || self.palette_open
            || self.app.comp_dialog.is_some()
            || self.app.pending_recovery.is_some()
        {
            return true;
        }
        #[cfg(feature = "media")]
        if self.export_dialog.is_some() {
            return true;
        }
        false
    }

    pub(super) fn recovery_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.app.pending_recovery else {
            return;
        };
        let n = pending.ops.len();
        // The doc's third recovery option (10 §4): open an autosave, offered
        // only when one exists beside the project.
        let autosave_path = lumit_project::latest_autosave(&pending.path);
        enum Pick {
            Restore,
            LastSave,
            Autosave(std::path::PathBuf),
        }
        let mut pick: Option<Pick> = None;
        egui::Window::new("Recover changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "The last session ended without saving. {n} change{} can be restored.",
                    if n == 1 { "" } else { "s" }
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(format!(
                            "Restore {n} change{}",
                            if n == 1 { "" } else { "s" }
                        ))
                        .clicked()
                    {
                        pick = Some(Pick::Restore);
                    }
                    if ui.button("Open last save").clicked() {
                        pick = Some(Pick::LastSave);
                    }
                    if let Some(path) = &autosave_path {
                        if ui.button("Open autosave").clicked() {
                            pick = Some(Pick::Autosave(path.clone()));
                        }
                    }
                });
            });
        match pick {
            Some(Pick::Restore) => self.app.resolve_recovery(true),
            Some(Pick::LastSave) => self.app.resolve_recovery(false),
            Some(Pick::Autosave(path)) => self.app.recover_from_autosave(path),
            None => {}
        }
    }
}
