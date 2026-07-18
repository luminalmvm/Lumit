//! Export actions driven from the shell: the size-targeted share export
//! (K-037), the export dialogue, and the one-at-a-time export queue
//! (docs/06 §7.1, K-119). Gated on the `media` feature, like every other
//! export concept.

use super::*;

impl Shell {
    /// Size-targeted share export (K-037): bitrate from the byte budget,
    /// with the audio track's share subtracted first.
    #[cfg(feature = "media")]
    pub(super) fn start_share_export(&mut self, target_mb: f64) {
        let Some(comp_id) = self.app.preview_comp.or(self.app.selected_comp) else {
            self.app.error = Some("select a composition to export".into());
            return;
        };
        let doc = self.app.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let duration = match comp.work_area {
            Some((a, b)) => (b.0.to_f64() - a.0.to_f64()).max(0.1),
            None => comp.duration.0.to_f64().max(0.1),
        };
        // 8% container/overhead headroom; a leaner AAC rate than the
        // delivery presets because every audio bit comes out of the budget.
        let audio_bit_rate: i64 = 192_000;
        let has_audio = !self.app.comp_audio_jobs(&doc, comp).is_empty();
        let mut bits = target_mb * 1_000_000.0 * 8.0 * 0.92;
        if has_audio {
            bits -= audio_bit_rate as f64 * duration;
        }
        let bit_rate = ((bits / duration) as i64).max(100_000);
        let spec = crate::export::ExportSpec {
            codec: lumit_media::encode::VideoCodec::H264,
            target: (comp.width, comp.height),
            bit_rate: Some(bit_rate),
            max_rate: None,
            include_audio: true,
            audio_bit_rate,
        };
        self.enqueue_export(comp_id, spec, &format!("share-{}mb.mp4", target_mb as u64));
    }

    /// Open the export dialogue for the current comp, with `preset` applied
    /// (Custom = the comp's own size and the encoder's default quality).
    #[cfg(feature = "media")]
    pub(super) fn open_export_dialog(&mut self, preset: crate::export::ExportPreset) {
        let Some(comp_id) = self.app.preview_comp.or(self.app.selected_comp) else {
            self.app.error = Some("select a composition to export".into());
            return;
        };
        let doc = self.app.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let mut dialog = ExportDialogState {
            comp_id,
            comp_size: (comp.width, comp.height),
            comp_name: comp.name.clone(),
            filename_template: self.settings_export.filename_template.clone(),
            preset,
            stamped: None,
            codec: lumit_media::encode::VideoCodec::H264,
            size: None,
            bitrate_mbps: String::new(),
            include_audio: true,
            default_name: "export.mp4".to_string(),
        };
        dialog.apply(preset);
        self.export_dialog = Some(dialog);
    }

    /// Ask where to save, then queue one export. Queue items snapshot the
    /// document now (docs/06 §7.1); the queue runs them one after another.
    #[cfg(feature = "media")]
    fn enqueue_export(
        &mut self,
        comp_id: uuid::Uuid,
        spec: crate::export::ExportSpec,
        default_name: &str,
    ) {
        let doc = self.app.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let picked = rfd::FileDialog::new()
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(default_name)
            .save_file();
        let Some(path) = picked else { return };
        let items = crate::export::item_infos(&doc, &self.app.media);
        let audio = self.app.comp_audio_jobs(&doc, comp);
        self.export_queue.push_back(crate::export::QueuedExport {
            doc,
            comp_id,
            items,
            audio,
            out_path: path,
            spec,
        });
        self.try_start_next_export();
    }

    /// Start the next queued export if none is running.
    #[cfg(feature = "media")]
    pub(super) fn try_start_next_export(&mut self) {
        if self.export.is_some() {
            return;
        }
        let Some(next) = self.export_queue.pop_front() else {
            return;
        };
        let Some(gpu) = &self.gpu else {
            self.app.error = Some("export needs the GPU pipeline".into());
            self.export_queue.clear();
            return;
        };
        self.export_name = Some(
            next.out_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "export".into()),
        );
        self.export_encoder = None;
        self.export_progress = Some((0, 0));
        self.export = Some(crate::export::start(
            next.doc,
            next.comp_id,
            next.items,
            next.audio,
            gpu.export_context(),
            next.out_path,
            next.spec,
        ));
    }

    /// The export dialogue: preset, codec, frame, bitrate, audio — the
    /// stamped preset numbers stay editable (the custom path). Confirming
    /// asks where to save and queues the export.
    #[cfg(feature = "media")]
    pub(super) fn export_dialog_modal(&mut self, ctx: &egui::Context) {
        use crate::export::ExportPreset;
        use lumit_media::encode::VideoCodec;
        let Some(dialog) = &mut self.export_dialog else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Export")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                let theme = &self.theme;
                egui::Grid::new("export-dialog")
                    .num_columns(2)
                    .spacing(egui::vec2(12.0, 8.0))
                    .show(ui, |ui| {
                        ui.label("Preset");
                        bare_dropdown(ui, dialog.preset.label(), |ui| {
                            for preset in ExportPreset::ALL {
                                if ui.button(preset.label()).clicked() {
                                    dialog.apply(preset);
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("Codec");
                        bare_dropdown(ui, dialog.codec.label(), |ui| {
                            for codec in [VideoCodec::H264, VideoCodec::Hevc] {
                                if ui.button(codec.label()).clicked() {
                                    dialog.codec = codec;
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("Frame");
                        ui.horizontal(|ui| {
                            let (w, h) = dialog.size.unwrap_or(dialog.comp_size);
                            let suffix = if dialog.size.is_none() {
                                " (comp size)"
                            } else {
                                ""
                            };
                            ui.label(
                                egui::RichText::new(format!("{w}×{h}{suffix}"))
                                    .monospace()
                                    .color(theme.text_muted),
                            );
                            if dialog.size.is_some() && ui.small_button("Use comp size").clicked() {
                                dialog.size = None;
                            }
                        });
                        ui.end_row();

                        ui.label("Bitrate");
                        ui.horizontal(|ui| {
                            let (resp, buf) = text_field(
                                ui,
                                egui::Id::new("export-bitrate"),
                                &dialog.bitrate_mbps,
                                72.0,
                            );
                            if resp.changed() {
                                dialog.bitrate_mbps = buf;
                            }
                            ui.label(
                                egui::RichText::new("Mbps — empty for default quality")
                                    .small()
                                    .color(theme.text_muted),
                            );
                        });
                        ui.end_row();

                        ui.label("Audio");
                        ui.checkbox(&mut dialog.include_audio, "Include audio (AAC 320 kbps)");
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Frame rate follows the composition. Exports queue and run in order.",
                    )
                    .small()
                    .color(self.theme.text_disabled),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Export…").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        cancel = true;
                    }
                });
            });
        if confirm {
            if let Some(dialog) = self.export_dialog.take() {
                let spec = dialog.spec();
                self.enqueue_export(dialog.comp_id, spec, &dialog.default_name);
            }
        } else if cancel {
            self.export_dialog = None;
        }
    }
}
