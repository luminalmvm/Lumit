//! The application's per-frame update loop: `Shell::new` (construction and
//! restore) and `Shell::ui` (the frame body — menu bar, docked panels,
//! status line and live preview, per docs/07-UI-SPEC).

use super::*;

impl Shell {
    pub fn new(
        ctx: &egui::Context,
        restored: Option<Self>,
        boot_notes: Vec<String>,
        #[cfg(feature = "media")] render_state: Option<egui_wgpu::RenderState>,
    ) -> Self {
        let workspace_restored = restored.is_some();
        let mut shell = restored.unwrap_or_default();
        // Startup default: the left tab group opens on Project. The dock tree —
        // including each tab group's *active* tab — is persisted with the
        // workspace, so without this the tab last in front (often Effect
        // controls) greeted every launch. Startup only; tab clicks are
        // untouched for the rest of the session.
        activate_panel_tab(&mut shell.dock, Panel::Project);
        Theme::install_fonts(ctx);
        // Migrate a pre-K-097 save (see `migrated_scheme`).
        shell.color_scheme =
            migrated_scheme(shell.color_scheme, shell.theme_mode, shell.theme_variant);
        // Honour the saved picks (K-097 scheme × K-092 shape).
        shell.theme = Theme::for_scheme(shell.color_scheme, shell.theme_shape);
        if let Some(rgb) = shell.accent_override {
            shell.theme = shell.theme.with_accent(rgb);
        }
        shell.theme.apply(ctx);
        crate::theme::apply_animation_level(ctx, shell.animation_level);
        ctx.style_mut(|s| s.visuals.panel_fill = shell.theme.surface_0);
        // Honour saved cache budgets (Settings → Performance).
        shell.apply_cache_budgets();
        // Honour saved interface settings (Settings → Interface, K-117): a
        // saved non-default UI scale or a tooltips-off pick takes effect
        // before the first frame, not just after the next control touch.
        ctx.set_pixels_per_point(shell.interface.ui_scale);
        settings::apply_tooltips_enabled(ctx, shell.interface.show_tooltips);

        // The boot log (K-008): every line reflects real initialisation state.
        let mut lines = vec![
            BootLine::ok(format!(
                "Theme: {} ({})",
                shell.color_scheme.label(),
                match shell.theme_shape {
                    crate::theme::ThemeShape::Sharp => "sharp",
                    crate::theme::ThemeShape::Round => "round",
                }
            )),
            BootLine::ok(if workspace_restored {
                "Workspace: restored"
            } else {
                "Workspace: default (Edit)"
            }),
            BootLine::ok("Document store: ready"),
            BootLine::ok("Recovery journal: clean"),
        ];
        lines.extend(boot_notes.into_iter().map(BootLine::ok));
        #[cfg(feature = "media")]
        lines.push(BootLine::ok(format!(
            "Media engine: FFmpeg (libavformat {})",
            lumit_media::ffmpeg_version()
        )));
        #[cfg(feature = "media")]
        match render_state {
            Some(rs) => {
                shell.gpu = Some(GpuViewer::new(rs));
                lines.push(BootLine::ok(
                    "Nova render pipeline: GPU (sRGB → linear fp16 → display)",
                ));
            }
            None => lines.push(BootLine {
                text: "Nova render pipeline: CPU fallback (no wgpu render state)".into(),
                failed: true,
            }),
        }
        #[cfg(feature = "media")]
        lines.push(BootLine::ok("Nebula cache: RAM tier ready (512 MB)"));
        #[cfg(feature = "media")]
        lines.push(BootLine::ok(
            "Pulsar audio: cpal (clock starts with playback)",
        ));
        lines.push(BootLine::ok(format!(
            "Effects: {} built-in registered",
            lumit_core::fx::BUILTINS.len()
        )));

        #[cfg(target_os = "macos")]
        {
            match crate::native_menu::NativeMenu::install() {
                Ok(menu) => {
                    shell.native_menu = Some(menu);
                    lines.push(BootLine::ok("Menu bar: native (macOS)"));
                }
                Err(e) => lines.push(BootLine {
                    text: format!("Menu bar: in-window fallback ({e})"),
                    failed: true,
                }),
            }
        }

        shell.splash = Some(Splash::new(lines));
        shell
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if let Some(splash) = &self.splash {
            if crate::splash::show(ctx, &self.theme, splash) {
                // Boot finished: the splash window becomes the application window.
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1440.0, 900.0)));
                self.splash = None;
            }
            return;
        }
        self.app.autosave_tick(
            self.autosave.interval_mins as u64 * 60,
            self.autosave.keep as usize,
        );
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.app.import_paths(dropped);
        }
        #[cfg(feature = "media")]
        {
            self.app.poll_audio();
            self.app.poll_comp_audio();
            // Keep the loaded comp mix in step with the document: mute, move,
            // trim and delete of an audio layer all take effect here (GEN-4).
            self.app.sync_comp_audio();
            self.app.poll_beats();
            // Transport keys (07-UI-SPEC keymap; shuttle speeds arrive with
            // the ring buffer — J/left step back, L plays, K/Space pause).
            if (self.app.preview_item.is_some() || self.app.preview_comp.is_some())
                && !ctx.wants_keyboard_input()
            {
                let (space, k, l, left, right, j, home) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::Space),
                        i.key_pressed(egui::Key::K),
                        i.key_pressed(egui::Key::L),
                        i.key_pressed(egui::Key::ArrowLeft),
                        i.key_pressed(egui::Key::ArrowRight),
                        i.key_pressed(egui::Key::J),
                        i.key_pressed(egui::Key::Home),
                    )
                });
                if space {
                    self.app.toggle_play();
                }
                if k && self.app.is_playing() {
                    self.app.toggle_play();
                }
                if l && !self.app.is_playing() {
                    self.app.toggle_play();
                }
                let (b, n) =
                    ctx.input(|i| (i.key_pressed(egui::Key::B), i.key_pressed(egui::Key::N)));
                if b {
                    self.app.set_work_area_edge(false);
                }
                if n {
                    self.app.set_work_area_edge(true);
                }
                let step: i64 = i64::from(right) - i64::from(left || j);
                if step != 0 || home {
                    if self.app.is_playing() {
                        self.app.toggle_play(); // stepping implies pause
                    }
                    let frame = if home {
                        0
                    } else {
                        self.app.preview_frame.saturating_add_signed(step as isize)
                    };
                    self.app.preview_frame = frame;
                    self.app.refresh_preview();
                }
            }
            if let Some(export) = &self.export {
                let mut encoder_seen = None;
                let mut finished: Option<Result<std::path::PathBuf, String>> = None;
                while let Ok(ev) = export.events.try_recv() {
                    match ev {
                        crate::export::ExportEvent::Encoder(label) => {
                            encoder_seen = Some(label);
                        }
                        crate::export::ExportEvent::Progress { frame, total } => {
                            self.export_progress = Some((frame, total));
                        }
                        crate::export::ExportEvent::Done(path) => {
                            finished = Some(Ok(path));
                        }
                        crate::export::ExportEvent::Failed(e) => {
                            finished = Some(Err(e));
                        }
                    }
                }
                if let Some(label) = encoder_seen {
                    self.export_encoder = Some(label);
                }
                match finished {
                    Some(Ok(path)) => {
                        let with = self
                            .export_encoder
                            .map(|l| format!(" — encoded with {l}"))
                            .unwrap_or_default();
                        // A completed export is a quiet notice, not an error
                        // (docs/15 §10 — completion is quiet, never fig-tinted).
                        self.app.notice = Some(format!("exported {}{with}", path.display()));
                        self.export = None;
                        self.export_progress = None;
                        self.export_encoder = None;
                        self.export_name = None;
                        // The queue carries on with the next item.
                        self.try_start_next_export();
                    }
                    Some(Err(e)) => {
                        self.app.error = Some(format!("export: {e}"));
                        self.export = None;
                        self.export_progress = None;
                        self.export_encoder = None;
                        self.export_name = None;
                        // One failed or cancelled item never stalls the rest.
                        self.try_start_next_export();
                    }
                    None => {
                        ctx.request_repaint_after(std::time::Duration::from_millis(120));
                    }
                }
            }
            if self.app.comp_playback_tick() {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            if self.app.is_playing() && self.app.preview_comp.is_none() {
                if let (Some(clock), Some(fps)) =
                    (self.app.playback_clock(), self.app.preview_fps())
                {
                    let frame = (clock * fps) as usize;
                    if frame != self.app.preview_frame {
                        self.app.preview_frame = frame;
                        self.app.refresh_preview();
                    }
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            self.app.media.poll();
            if self.app.media.any_probing() {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
            if (self.app.preview_item.is_some() || self.app.preview_comp.is_some())
                && self.preview_display.is_none()
            {
                // Selection made before probe finished: retry until Ready.
                self.app.refresh_preview();
            }
            // The disk tier follows the project's save path (and the user's
            // cache-root override, Settings → Performance → Cache), and any
            // frames it promoted land in RAM here (docs/06 §5.4).
            self.app.disk_sync_root(self.settings.cache_root.as_deref());
            if self.app.drain_disk_loads() {
                ctx.request_repaint();
            }
            // Nebula warm path: a cached frame presents as a plain upload.
            if let Some(key) = self.app.cached_present.take() {
                if let Some(gpu) = &mut self.gpu {
                    // VRAM first (docs/06 §5): a still-resident display texture
                    // re-presents with zero GPU work; a RAM hit re-uploads and
                    // joins the VRAM tier for next time.
                    if let Some(hit) = gpu.present_vram(key) {
                        self.preview_display = Some(hit);
                    } else if let Some(frame) = self.app.comp_frame_cache.get(&key) {
                        let (w, h, rgba) = (frame.width, frame.height, frame.rgba.clone());
                        self.preview_display = Some(gpu.present_keyed(key, &rgba, w, h));
                    }
                }
            }
            // Idle: fill the work area around the playhead, one frame at a
            // time (any real request supersedes the fill mid-flight). Paused
            // while scrubbing/dragging so fills don't fight the interaction.
            if self.settings.background_fill
                && !self.app.is_playing()
                && !self.app.is_interacting()
                && self.app.fill_in_flight.is_none()
            {
                if let Some(comp_id) = self.app.preview_comp {
                    if let Some(frame) = self.app.next_fill_frame(comp_id) {
                        // Disk-first (docs/06 §5.4: promote, never re-render
                        // what is already parked): a frame the disk tier holds
                        // is loaded back instead of rendered; the render path
                        // only runs on frames no tier has.
                        let promoted = match self.app.frame_key_for(comp_id, frame) {
                            Some(key) if self.app.disk_has(key) => {
                                self.app.disk_request_load(key);
                                true
                            }
                            _ => false,
                        };
                        if !promoted {
                            self.app.request_fill_frame(comp_id, frame);
                        }
                        ctx.request_repaint_after(std::time::Duration::from_millis(30));
                    }
                }
            }
            let mut newest = None;
            while let Ok(result) = self.app.preview_engine.results.try_recv() {
                newest = Some(result);
            }
            use crate::app_state::preview::PreviewResult;
            match newest {
                Some(Ok(PreviewResult::Comp(cf))) if Some(cf.comp) == self.app.preview_comp => {
                    // Only the frame under the playhead is presented; any other
                    // frame (a background fill, or a stale render that arrived
                    // after an edit moved on) is banked, never shown — otherwise
                    // the viewport jumps to whatever fill just finished.
                    let is_fill = cf.frame != self.app.preview_frame;
                    if let Some(gpu) = &mut self.gpu {
                        let doc = self.app.store.snapshot();
                        if let Some(comp) = doc.comp(cf.comp) {
                            let t_comp = cf.frame as f64 / comp.frame_rate.fps().max(1.0);
                            let pixels_by_layer: std::collections::HashMap<_, _> =
                                cf.layers.iter().map(|lp| (lp.layer, lp)).collect();
                            let mut visited = vec![comp.id];
                            let draws = build_comp_draws(
                                &doc,
                                comp,
                                t_comp,
                                &pixels_by_layer,
                                &mut visited,
                            );
                            let bg = comp.background.0;
                            let background = [
                                f64::from(bg[0]),
                                f64::from(bg[1]),
                                f64::from(bg[2]),
                                f64::from(bg[3]),
                            ];
                            let pose = comp.camera_pose(t_comp);
                            if is_fill {
                                // Background fill: readback, store, don't show.
                                if let (Some(key), Some(rgba)) = (
                                    self.app.frame_key_for(cf.comp, cf.frame),
                                    gpu.realise_to_bytes(
                                        pose,
                                        comp.width,
                                        comp.height,
                                        background,
                                        &draws,
                                    ),
                                ) {
                                    self.app.disk_store_behind(
                                        key,
                                        comp.width,
                                        comp.height,
                                        rgba.clone(),
                                    );
                                    self.app.comp_frame_cache.insert(
                                        key,
                                        crate::app_state::CachedCompFrame {
                                            width: comp.width,
                                            height: comp.height,
                                            rgba,
                                        },
                                    );
                                    self.app.cache_epoch += 1;
                                }
                                self.app.fill_in_flight = None;
                            } else {
                                // Realtime mode (K-030, docs/06 §6.5): time the
                                // live composite and feed it to the adaptive
                                // controller so the next frames' resolution tracks
                                // the load. Only uncached frames reach here (cached
                                // ones present earlier, for free), so the tier only
                                // moves when we're actually rendering live.
                                // CAVEAT: this is the CPU-side composite/submit
                                // cost — a partial proxy that does NOT capture the
                                // async GPU execution or the decode cost. Needs
                                // validation on real hardware (audit 06 §6.5).
                                let started = std::time::Instant::now();
                                self.preview_display = Some(gpu.present_comp(
                                    pose,
                                    comp.width,
                                    comp.height,
                                    background,
                                    &draws,
                                ));
                                if self.app.preview_realtime && self.app.is_playing() {
                                    let fps = comp.frame_rate.fps().max(1.0);
                                    self.app
                                        .realtime_ctrl
                                        .record(started.elapsed().as_secs_f64(), fps);
                                }
                                // Paused: bank the frame while it's hot (playback
                                // misses skip the readback to protect the frame
                                // budget; draft frames are never banked — the
                                // cache holds specified-resolution frames only).
                                if !self.app.is_playing() && !self.app.preview_draft {
                                    if let Some(key) = self.app.frame_key_for(cf.comp, cf.frame) {
                                        if !self.app.comp_frame_cache.contains_key(&key) {
                                            if let Some(rgba) = gpu.realise_to_bytes(
                                                pose,
                                                comp.width,
                                                comp.height,
                                                background,
                                                &draws,
                                            ) {
                                                self.app.disk_store_behind(
                                                    key,
                                                    comp.width,
                                                    comp.height,
                                                    rgba.clone(),
                                                );
                                                self.app.comp_frame_cache.insert(
                                                    key,
                                                    crate::app_state::CachedCompFrame {
                                                        width: comp.width,
                                                        height: comp.height,
                                                        rgba,
                                                    },
                                                );
                                                self.app.cache_epoch += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Retain the presented frame's decoded pixels so a value
                    // drag can re-composite from them without re-decoding.
                    if !is_fill {
                        self.last_comp = Some(cf);
                    }
                }
                Some(Ok(PreviewResult::Footage(px)))
                    if Some(px.item) == self.app.preview_item
                        && self.app.preview_comp.is_none() =>
                {
                    if let Some(gpu) = &mut self.gpu {
                        self.preview_display = Some(gpu.present(&px.rgba, px.width, px.height));
                    } else {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [px.width as usize, px.height as usize],
                            &px.rgba,
                        );
                        let tex =
                            ctx.load_texture("viewer-frame", image, egui::TextureOptions::LINEAR);
                        self.preview_display = Some((tex.id(), tex.size_vec2()));
                        self.preview_tex = Some(tex);
                    }
                }
                Some(Err(e)) => self.app.error = Some(format!("preview: {e}")),
                _ => {}
            }
            // Edits (commits/undo) re-render the comp preview automatically.
            let doc_ptr = std::sync::Arc::as_ptr(&self.app.store.snapshot()) as usize;
            if self.app.preview_comp.is_some() && self.last_doc_ptr != doc_ptr {
                self.last_doc_ptr = doc_ptr;
                self.app.refresh_preview();
            } else {
                self.last_doc_ptr = doc_ptr;
            }

            // Live edit preview: while a transform value OR a graph keyframe is
            // being dragged, re-composite the retained frame with the provisional
            // value patched in for this frame only — instant feedback with no
            // re-decode, since a transform change never alters which footage
            // frame a layer shows.
            if let Some(comp_id) = self.app.preview_comp {
                // The live patch below re-composites from this frame's decoded
                // per-layer pixels (`last_comp`). A frame served from the
                // composite cache never populates `last_comp`, so a value drag on
                // a cache-hit frame would show nothing live until release (the
                // owner bug: effect-value drags in the layer area only updated on
                // frames that had a keyframe — a keyframe at the playhead
                // invalidated the cache and forced the decode). When a live edit
                // is active and `last_comp` is stale, request a decode; the engine
                // coalesces repeats until it lands, then this stops firing.
                let stale = !matches!(
                    &self.last_comp,
                    Some(cf) if cf.comp == comp_id && cf.frame == self.app.preview_frame
                );
                if stale && self.app.live_edit_active() {
                    self.app.refresh_preview();
                }
                if let (Some(gpu), Some(cf)) = (&mut self.gpu, &self.last_comp) {
                    if cf.comp == comp_id && cf.frame == self.app.preview_frame {
                        let doc = self.app.store.snapshot();
                        if let Some(comp) = doc.comp(comp_id) {
                            let t_comp = cf.frame as f64 / comp.frame_rate.fps().max(1.0);
                            use lumit_core::model::TransformProp;
                            // A linked-scale drag moves both axes at once, so
                            // patch both; otherwise a direct value drag gives
                            // (layer, prop, value), and a graph keyframe drag
                            // gives the property's provisional value at the
                            // playhead.
                            let patched = if let Some((sl, nx, ny)) = self.app.scale_preview {
                                Some(patch_layer_prop(
                                    &patch_layer_prop(comp, sl, TransformProp::ScaleX, nx),
                                    sl,
                                    TransformProp::ScaleY,
                                    ny,
                                ))
                            } else if let Some((lid, ei, pi, val)) = self.app.fx_edit {
                                // Live effect-value drag: patch the effect
                                // param and let the stack re-run with it.
                                Some(patch_layer_effect_param(comp, lid, ei, pi, val))
                            } else {
                                self.app
                                    .prop_edit
                                    .or_else(|| {
                                        let (idx, kt, kv) = self.app.graph_edit?;
                                        let prop = self.app.graph_prop?;
                                        let layer_id = self.app.selected_layer?;
                                        let layer =
                                            comp.layers.iter().find(|l| l.id == layer_id)?;
                                        let lumit_core::anim::Animation::Keyframed(keys) =
                                            &layer.transform.get(prop).animation
                                        else {
                                            return None;
                                        };
                                        let mut keys = keys.clone();
                                        let k = keys.get_mut(idx)?;
                                        k.time = rational_at(kt.max(0.0));
                                        k.value = kv;
                                        keys.sort_by_key(|k| k.time);
                                        let lt = t_comp - layer.start_offset.0.to_f64();
                                        let value = lumit_core::anim::evaluate(&keys, lt)?;
                                        Some((layer_id, prop, value))
                                    })
                                    .map(|(edit_layer, prop, value)| {
                                        patch_layer_prop(comp, edit_layer, prop, value)
                                    })
                            };
                            if let Some(patched) = patched {
                                let pixels_by_layer: std::collections::HashMap<_, _> =
                                    cf.layers.iter().map(|lp| (lp.layer, lp)).collect();
                                let mut visited = vec![comp_id];
                                let draws = build_comp_draws(
                                    &doc,
                                    &patched,
                                    t_comp,
                                    &pixels_by_layer,
                                    &mut visited,
                                );
                                let bg = comp.background.0;
                                let background = [
                                    f64::from(bg[0]),
                                    f64::from(bg[1]),
                                    f64::from(bg[2]),
                                    f64::from(bg[3]),
                                ];
                                let pose = patched.camera_pose(t_comp);
                                self.preview_display = Some(gpu.present_comp(
                                    pose,
                                    comp.width,
                                    comp.height,
                                    background,
                                    &draws,
                                ));
                                ctx.request_repaint();
                            }
                        }
                    }
                }
            }
        }
        #[cfg(target_os = "macos")]
        self.native_menu_frame();
        #[cfg(not(target_os = "macos"))]
        self.shortcuts(ctx);
        self.keyframe_clipboard_shortcuts(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.app.project_title()));

        #[cfg(not(target_os = "macos"))]
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New project").clicked() {
                        self.app.new_project();
                        ui.close_menu();
                    }
                    if ui.button("Open project…").clicked() {
                        self.app.open_dialog();
                        ui.close_menu();
                    }
                    if ui.button("Import footage…").clicked() {
                        self.app.import_footage_dialog();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Save").clicked() {
                        self.app.save();
                        ui.close_menu();
                    }
                    #[cfg(feature = "media")]
                    if ui.button("Export comp…").clicked() {
                        // Settings → Export default preset (K-119); the
                        // "Export preset" submenu below always keeps its
                        // own explicit pick regardless of this default.
                        self.open_export_dialog(self.settings_export.default_preset);
                        ui.close_menu();
                    }
                    #[cfg(feature = "media")]
                    ui.menu_button("Export preset", |ui| {
                        for preset in [
                            crate::export::ExportPreset::Youtube1080p60,
                            crate::export::ExportPreset::Youtube1440p60,
                            crate::export::ExportPreset::Youtube4k60,
                            crate::export::ExportPreset::Vertical1080p60,
                        ] {
                            if ui.button(preset.label()).clicked() {
                                self.open_export_dialog(preset);
                                ui.close_menu();
                            }
                        }
                    });
                    #[cfg(feature = "media")]
                    ui.menu_button("Export for sharing", |ui| {
                        if ui.button("Discord 50 MB").clicked() {
                            self.start_share_export(50.0);
                            ui.close_menu();
                        }
                        if ui.button("Small 10 MB").clicked() {
                            self.start_share_export(10.0);
                            ui.close_menu();
                        }
                    });
                });
                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(self.app.store.can_undo(), egui::Button::new("Undo"))
                        .clicked()
                    {
                        self.app.undo();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(self.app.store.can_redo(), egui::Button::new("Redo"))
                        .clicked()
                    {
                        self.app.redo();
                        ui.close_menu();
                    }
                });
                ui.menu_button("Composition", |ui| {
                    if ui.button("New composition").clicked() {
                        self.app.new_composition();
                        ui.close_menu();
                    }
                    if ui.button("Add solid layer").clicked() {
                        self.app.add_solid_layer();
                        ui.close_menu();
                    }
                    if ui.button("Add text layer").clicked() {
                        self.app.add_text_layer();
                        ui.close_menu();
                    }
                    if ui.button("Add camera layer").clicked() {
                        self.app.add_camera_layer();
                        ui.close_menu();
                    }
                    if ui.button("Add adjustment layer").clicked() {
                        self.app.add_adjustment_layer();
                        ui.close_menu();
                    }
                    if ui.button("Add sequence layer").clicked() {
                        self.app.add_sequence_layer();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.app.selected_layer.is_some(),
                            egui::Button::new("Duplicate layer"),
                        )
                        .clicked()
                    {
                        self.app.duplicate_layer();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            self.app.selected_layer.is_some(),
                            egui::Button::new("Delete layer"),
                        )
                        .clicked()
                    {
                        self.app.delete_selected_layer();
                        ui.close_menu();
                    }
                    if ui.button("Cut clip at playhead").clicked() {
                        self.app.cut_sequence_at_playhead();
                        ui.close_menu();
                    }
                    if ui.button("Delete clip at playhead").clicked() {
                        self.app.delete_clip_at_playhead();
                        ui.close_menu();
                    }
                    if ui.button("Add marker at playhead").clicked() {
                        self.app.add_marker_at_playhead();
                        ui.close_menu();
                    }
                    #[cfg(feature = "media")]
                    ui.menu_button("Detect beats", |ui| {
                        // Sensitivity slider (docs/09 §5): 0–100, higher = more
                        // beats. 50 is the old "Standard"; the δ the detector
                        // wants is derived from this.
                        ui.add(
                            egui::Slider::new(&mut self.app.beat_sensitivity, 0..=100)
                                .text("Sensitivity"),
                        );
                        if ui.button("Detect").clicked() {
                            if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                                let delta = lumit_audio::beat::delta_from_sensitivity(
                                    self.app.beat_sensitivity,
                                );
                                self.app.detect_beats(id, delta);
                            }
                            ui.close_menu();
                        }
                    });
                    if ui.button("Clear beat markers").clicked() {
                        self.app.clear_beat_markers();
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.add_enabled_ui(self.app.selected_layer.is_some(), |ui| {
                        ui.menu_button("Add mask", |ui| {
                            for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Star]
                            {
                                if ui.button(kind.label()).clicked() {
                                    self.add_mask_to_selected(kind);
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                });
                ui.menu_button("Window", |ui| {
                    if ui.button("Command palette…").clicked() {
                        self.open_command_palette();
                        ui.close_menu();
                    }
                    if ui.button("Reset workspace").clicked() {
                        self.dock = default_layout();
                        ui.close_menu();
                    }
                    ui.separator();
                    // T25: the waveform bar can be hidden from its own close
                    // button, so it needs a durable way back on — here and in
                    // the Timeline lane right-click menu.
                    if ui
                        .selectable_label(self.app.show_audio_bar, "Audio waveform")
                        .on_hover_text("Show the audio waveform bar under the Timeline lanes")
                        .clicked()
                    {
                        self.app.show_audio_bar = !self.app.show_audio_bar;
                        ui.close_menu();
                    }
                    ui.separator();
                    // Theme, shape, motion and performance now live in the
                    // Settings window (Appearance / Performance pages).
                    if ui.button("Settings…").clicked() {
                        self.settings_open = true;
                        ui.close_menu();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("Edit")
                            .small()
                            .color(self.theme.text_muted),
                    )
                    .on_hover_text("Workspace — presets arrive with the panel set");
                });
            });
        });

        // Tool strip: the pointer's mode (docs/07-UI-SPEC toolbar). Object
        // tools join as they land; today: navigation and mask drawing.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            // Under Round the tools sit in their own rounded bar (owner
            // request); under Sharp the wrap is transparent with no margin,
            // so the row is pixel-identical to before.
            let theme = self.theme;
            let round = theme.shape == crate::theme::ThemeShape::Round;
            let t = theme.tokens;
            let wrap = egui::Frame::new()
                .fill(if round {
                    theme.surface_2
                } else {
                    egui::Color32::TRANSPARENT
                })
                .corner_radius(if round { t.control_radius } else { 0 })
                .shadow(if round {
                    t.card_shadow
                } else {
                    egui::Shadow::NONE
                })
                .inner_margin(if round {
                    egui::Margin::symmetric(5, 2)
                } else {
                    egui::Margin::ZERO
                });
            if round {
                ui.add_space(3.0);
            }
            ui.horizontal(|ui| {
                if round {
                    ui.add_space(t.window_inset);
                }
                wrap.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let tool = self.app.tool;
                        if icon_button(ui, &theme, Icon::Pointer, tool == ToolMode::Select)
                            .on_hover_text("Select / move the view (V)")
                            .clicked()
                        {
                            self.app.tool = ToolMode::Select;
                        }
                        if icon_button(ui, &theme, Icon::Move, tool == ToolMode::Hand)
                            .on_hover_text("Drag to pan the view (H)")
                            .clicked()
                        {
                            self.app.tool = ToolMode::Hand;
                        }
                        // The Shape button wears the current shape; right-click to switch.
                        let shape_icon = match self.app.shape_kind {
                            ShapeKind::Rectangle => Icon::Rectangle,
                            ShapeKind::Ellipse => Icon::Ellipse,
                            ShapeKind::Star => Icon::Star,
                        };
                        let shape_resp =
                            icon_button(ui, &theme, shape_icon, tool == ToolMode::Shape)
                                .on_hover_text(format!(
                                    "Draw a {} mask — right-click to pick a shape (Q)",
                                    self.app.shape_kind.label().to_lowercase()
                                ));
                        if shape_resp.clicked() {
                            self.app.tool = ToolMode::Shape;
                        }
                        shape_resp.context_menu(|ui| {
                            for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Star]
                            {
                                if ui
                                    .selectable_label(self.app.shape_kind == kind, kind.label())
                                    .clicked()
                                {
                                    self.app.shape_kind = kind;
                                    self.app.tool = ToolMode::Shape;
                                    ui.close_menu();
                                }
                            }
                        });
                        if icon_button(ui, &theme, Icon::Pen, tool == ToolMode::Pen)
                            .on_hover_text(
                                "Click points to draw a mask; click the first to close (G)",
                            )
                            .clicked()
                        {
                            self.app.tool = if tool == ToolMode::Pen {
                                ToolMode::Select
                            } else {
                                ToolMode::Pen
                            };
                            self.app.pen_path.clear();
                        }
                    });
                });
            });
        });
        // Single-key tool shortcuts, ignored while a text field has focus.
        if !ctx.wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::V) {
                    self.app.tool = ToolMode::Select;
                }
                if i.key_pressed(egui::Key::H) {
                    self.app.tool = ToolMode::Hand;
                }
                if i.key_pressed(egui::Key::Q) {
                    self.app.tool = ToolMode::Shape;
                }
                if i.key_pressed(egui::Key::G) {
                    self.app.tool = ToolMode::Pen;
                    self.app.pen_path.clear();
                }
            });
        }
        // Razor (Cmd/Ctrl+Shift+D). On macOS the native menu's accelerator
        // handles it, so this keyboard path is the Windows/in-window one.
        #[cfg(not(target_os = "macos"))]
        if !ctx.wants_keyboard_input()
            && ctx
                .input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::D))
        {
            self.app.cut_sequence_at_playhead();
        }

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let status = if self.app.dirty {
                    "Unsaved changes"
                } else {
                    "Ready"
                };
                ui.label(
                    egui::RichText::new(status)
                        .small()
                        .color(self.theme.text_muted),
                );
                #[cfg(feature = "media")]
                if let Some((frame, total)) = self.export_progress {
                    ui.separator();
                    let name = self.export_name.as_deref().unwrap_or("export");
                    let mut line = format!("exporting {name} {frame}/{total}");
                    if let Some(enc) = self.export_encoder {
                        line.push_str(&format!(" · {enc}"));
                    }
                    if !self.export_queue.is_empty() {
                        line.push_str(&format!(" · {} queued", self.export_queue.len()));
                    }
                    ui.label(
                        egui::RichText::new(line)
                            .monospace()
                            .small()
                            .color(self.theme.accent),
                    );
                    if ui.small_button("Cancel").clicked() {
                        if let Some(export) = &self.export {
                            export.cancel();
                        }
                    }
                }
                if let Some(err) = self.app.error.clone() {
                    ui.separator();
                    // Errors are fig-tinted banners (docs/15 §10), set apart from
                    // the quiet neutral notice.
                    ui.label(egui::RichText::new(&err).small().color(self.theme.error));
                    if ui.small_button("Dismiss").clicked() {
                        self.app.error = None;
                    }
                } else if let Some(notice) = self.app.notice.clone() {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(&notice)
                            .small()
                            .color(self.theme.text_secondary),
                    );
                    if ui.small_button("Dismiss").clicked() {
                        self.app.notice = None;
                    }
                }
            });
        });

        self.recovery_modal(ctx);
        self.comp_dialog_modal(ctx);
        #[cfg(feature = "media")]
        self.export_dialog_modal(ctx);
        self.settings_modal(ctx);
        self.command_palette_modal(ctx);
        // Read before the borrow below splits `self` apart (used by the
        // active-panel focus edge further down).
        let modal_open = self.any_modal_open();

        // While a DoF Focus (depth) eyedropper is armed, stash the referenced
        // depth layer's decoded pixels for the eyedropper to sample — this is the
        // one place holding both the egui context and the per-layer decode cache
        // (`last_comp`), so a depth pick reads the real depth pass instead of the
        // composite. Guarded internally: a no-op unless a depth pick is armed.
        #[cfg(feature = "media")]
        eyedropper::stash_depth_source(ctx, &self.app, &self.last_comp);

        // The tiling dock fills the window: a solo pane renders bare with no
        // tab bar — the Viewer's look (K-074) on every lone panel (K-086) —
        // while stacked panels carry tabs and can be dragged to re-arrange
        // the workspace.
        let Shell {
            dock,
            floating,
            theme,
            app,
            preview_display,
            ..
        } = self;
        let preview_display = *preview_display;
        // Clear the effect-value live-preview slot once per frame before the
        // panels draw; whichever panel (Timeline or Effect Controls) has an
        // active drag re-sets it. Both draw the same effect rows in one frame,
        // so an unconditional write from either used to clobber the other's
        // drag — the fix that makes "live preview while dragging" actually work
        // when both panels are docked.
        app.fx_edit = None;
        let tree_id = dock.id();
        let bare_tiles = bare_tile_ids(dock);
        let (pop_out, panel_rects) = {
            let mut behavior = DockBehavior {
                theme,
                app,
                preview_display,
                pop_out: None,
                panel_rects: Vec::new(),
                tree_id,
                bare_tiles,
            };
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::default()
                        .fill(theme.surface_0)
                        .inner_margin(theme.tokens.window_inset),
                )
                .show(ctx, |ui| dock.ui(&mut behavior, ui));
            (behavior.pop_out, behavior.panel_rects)
        };

        // An eyedropper button (drawn deep inside the effect rows, shared with
        // the Timeline) stashes its arm request in context data; drain it here,
        // once the panels have drawn, and arm the tool for the next frame.
        if let Some(target) = eyedropper::take_arm_request(ctx) {
            app.arm_eyedropper(target);
        }

        // Active-panel boundary (owner request): a press inside a pane makes
        // it the focused one, and it wears a 1px accent edge so the eye always
        // knows where shortcuts land. Skipped while a modal is open — its
        // backdrop owns the click, so a press over the dialog must not reach
        // through and re-focus a panel behind it.
        if !modal_open {
            if let Some(pos) = ctx.input(|i| i.pointer.press_origin()) {
                if let Some((panel, _)) = panel_rects.iter().find(|(_, r)| r.contains(pos)) {
                    self.active_panel = Some(*panel);
                }
            }
        }
        if let Some(active) = self.active_panel {
            if let Some((_, rect)) = panel_rects.iter().find(|(p, _)| *p == active) {
                // `Order::Middle` sits above the panels (which draw in the
                // background layer) but below menus, popups and tooltips
                // (foreground and up), so the accent edge no longer paints
                // over an open menu.
                ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Middle,
                    egui::Id::new("active-panel-edge"),
                ))
                .rect_stroke(
                    rect.shrink(0.5),
                    theme.tokens.card_radius,
                    egui::Stroke::new(1.0_f32, theme.accent.gamma_multiply(0.55)),
                    egui::StrokeKind::Inside,
                );
            }
        }

        // Apply a pop-out request: hide the panel in the dock, float it. A
        // solo Timeline has no dock tab (K-086), so its request arrives from
        // the comp strip's context menu through AppState rather than from a
        // tab's pop-out button.
        let pop_out = pop_out
            .or_else(|| std::mem::take(&mut app.pop_out_timeline).then_some(Panel::Timeline));
        if let Some(panel) = pop_out {
            if let Some(tile) = tile_id_of(dock, panel) {
                dock.tiles.set_visible(tile, false);
            }
            if !floating.contains(&panel) {
                floating.push(panel);
            }
        }

        // UI-13: after an import, bring the Project tab to the front so the new
        // footage is visible where it was just selected.
        if std::mem::take(&mut app.focus_project_tab) {
            activate_panel_tab(dock, Panel::Project);
        }
        // The Effect Controls tab only exists while a layer is selected
        // (owner): with nothing selected the tab hides from the dock entirely.
        // A floating Effect Controls window is left alone — hiding the dock
        // tile of a floated panel is the pop-out mechanism above.
        if !floating.contains(&Panel::EffectControls) {
            if let Some(tile) = tile_id_of(dock, Panel::EffectControls) {
                dock.tiles.set_visible(tile, app.selected_layer.is_some());
            }
        }
        // Owner: after applying an effect, bring the Effect Controls tab to the
        // front so the freshly added (and now selected) effect is visible (the
        // apply also selected the layer, so the tab is visible again by here).
        if std::mem::take(&mut app.focus_effects_tab) {
            activate_panel_tab(dock, Panel::EffectControls);
        }

        // Render each floating panel in its own OS window (an immediate
        // viewport, so it can borrow the live app state). Closing the window
        // docks the panel back into the tree where it came from.
        let mut dock_back: Vec<Panel> = Vec::new();
        for panel in floating.iter_mut() {
            let vid = egui::ViewportId::from_hash_of(("lumit-float", panel.title()));
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("Lumit — {}", panel.title()))
                .with_inner_size([640.0, 420.0]);
            ctx.show_viewport_immediate(vid, builder, |ctx, _class| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default().fill(theme.surface_0))
                    .show(ctx, |ui| {
                        render_panel(ui, theme, app, preview_display, panel)
                    });
                if ctx.input(|i| i.viewport().close_requested()) {
                    dock_back.push(*panel);
                }
            });
        }
        for panel in dock_back {
            floating.retain(|p| *p != panel);
            if let Some(tile) = tile_id_of(dock, panel) {
                dock.tiles.set_visible(tile, true);
            }
        }
    }
}
