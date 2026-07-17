//! `shell::timeline` — split out of the monolithic shell.rs (mechanical,
//! no logic changes). Shared names resolve through the parent module
//! via `use super::*` and the glob re-exports in `shell/mod.rs`.

use super::*;

/// Whether a layer of this kind accepts an effect dragged from the Effects &
/// Presets browser onto its Timeline row (K-101). Every `Layer` carries an
/// effect stack regardless of kind (`model::Layer::effects`), but v1 narrows
/// the drop target to footage and adjustment layers — an effect stack's two
/// ordinary homes (an adjustment layer exists only to host one). Every other
/// kind still gains effects the existing way: the "Add effect" row in its
/// own Effects group, untouched by this change.
fn accepts_effect_drop(kind: &lumit_core::model::LayerKind) -> bool {
    matches!(
        kind,
        lumit_core::model::LayerKind::Footage { .. } | lumit_core::model::LayerKind::Adjustment
    )
}

pub(crate) fn timeline_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    // Comp tab strip first: it owns a drop zone (drop a comp here to open it as
    // a separate tab). The body below then accepts drops that file into the
    // open comp (footage → layer, comp → nested precomp). Splitting the two
    // regions is what lets a comp be opened *beside* the tabs versus added
    // *into* the open comp.
    comp_tab_strip(ui, theme, app);
    let body_rect = ui.available_rect_before_wrap();
    accept_item_drop(ui, theme, app, body_rect);
    let doc = app.store.snapshot();
    let comp = app.selected_comp.and_then(|id| doc.comp(id));
    let Some(comp) = comp else {
        empty_hint(
            ui,
            theme,
            "No composition open",
            "Create one with Composition → New, or drag footage here from the Project panel.",
        );
        return;
    };
    let comp_id = comp.id;
    // The comp tab strip above already names the open comp; no redundant
    // in-panel title, resolution or frame-rate line here (Mack).
    if comp.layers.is_empty() {
        ui.label(
            egui::RichText::new("Drag footage here to create the first layer.")
                .small()
                .color(theme.text_muted),
        );
        return;
    }
    use lumit_core::anim::Animation;
    let mut pending: Option<lumit_core::Op> = None;
    // A per-clip speed-ramp edit (start %, end %, ease), applied after the loop.
    let mut clip_ramp_edit: Option<(f64, f64, lumit_core::retime::Ease)> = None;
    // A per-clip frame-interpolation edit, applied after the layer loop.
    let mut clip_interp_edit: Option<lumit_core::retime::Interpolation> = None;

    // ---- ruler + time geometry (07-UI-SPEC Timeline) --------------------
    let panel_left = ui.max_rect().left();
    let panel_right = ui.max_rect().right();
    // Draggable left-column width (Mack): clamp so it can't swallow the track.
    let name_w = app
        .timeline_name_w
        .clamp(96.0, (panel_right - panel_left - 120.0).max(96.0));
    let duration = comp.duration.0.to_f64().max(1e-6);
    let frames = app.comp_frame_count(comp).max(1);
    let track_left = panel_left + name_w;
    let track_w = (panel_right - track_left - 8.0).max(40.0);
    // Horizontal view model (07-UI-SPEC §4): zoom + scrolled left edge. At zoom 1
    // this is exactly the old whole-comp-across-the-track mapping.
    let (px_per_sec, view_start) = lane_view(
        track_w,
        duration,
        app.timeline_zoom,
        app.timeline_view_start,
    );
    app.timeline_view_start = view_start; // persist the clamp
    let x_of = |seconds: f64| track_left + ((seconds - view_start) * px_per_sec) as f32;
    let seconds_of = |x: f32| view_start + (x - track_left) as f64 / px_per_sec.max(1e-6);

    // AE wheel shortcuts over the lane area: Alt = zoom (around the cursor),
    // Shift = scroll through time. Plain wheel falls through to the vertical
    // ScrollArea. Alt/Shift consume the scroll so it does not also scroll lanes.
    let lane_area = egui::Rect::from_min_max(
        egui::pos2(track_left, ui.max_rect().top()),
        egui::pos2(panel_right, ui.max_rect().bottom()),
    );
    let (scroll, mods, hover) =
        ui.input(|i| (i.raw_scroll_delta, i.modifiers, i.pointer.hover_pos()));
    if hover.is_some_and(|p| lane_area.contains(p)) {
        let cursor_x = hover.map(|p| p.x).unwrap_or(track_left);
        let consume = |ui: &mut egui::Ui| {
            ui.input_mut(|i| {
                i.raw_scroll_delta = egui::Vec2::ZERO;
                i.smooth_scroll_delta = egui::Vec2::ZERO;
            });
        };
        if mods.alt && scroll.y.abs() > 0.01 {
            // Alt-wheel: zoom the time axis around the cursor.
            let cursor_t = view_start + (cursor_x - track_left) as f64 / px_per_sec.max(1e-6);
            let factor = (scroll.y as f64 * 0.004).exp();
            app.timeline_zoom = (app.timeline_zoom * factor).clamp(1.0, 400.0);
            let (new_ppx, _) = lane_view(track_w, duration, app.timeline_zoom, view_start);
            app.timeline_view_start = cursor_t - (cursor_x - track_left) as f64 / new_ppx.max(1e-6);
            consume(ui);
        } else if app.timeline_graph_mode
            && !app.graph_speed_view
            && !mods.shift
            && scroll.y.abs() > 0.01
            && scroll.x.abs() <= 0.01
        {
            // Graph mode, value lens: the plain (or Ctrl-) wheel scrolls/zooms
            // the *curve* vertically, not the layer list (K-079). The outline's
            // ScrollArea reads `smooth_scroll_delta`, so zeroing only that frees
            // the wheel for graph_plot (which reads `raw_scroll_delta`) — the
            // graph and the layer list therefore scroll independently. A wheel
            // over the outline column (left of the lane area) isn't in
            // `lane_area`, so it still scrolls the list as before.
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        } else {
            // Horizontal scroll: a horizontal wheel (Shift-wheel arrives as
            // scroll.x on most platforms) or Shift + a vertical wheel. Plain
            // vertical wheel falls through to the ScrollArea.
            let h = if scroll.x.abs() > 0.01 {
                scroll.x
            } else if mods.shift {
                scroll.y
            } else {
                0.0
            };
            if h.abs() > 0.01 {
                app.timeline_view_start = view_start - h as f64 / px_per_sec.max(1e-6);
                consume(ui);
            }
        }
    }

    let (ruler_rect, ruler_resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 20.0),
        egui::Sense::click_and_drag(),
    );
    ui.painter().rect_filled(ruler_rect, 0.0, theme.surface_2);
    // Clip everything time-positioned (ruler ticks, bars, keyframes, markers,
    // playhead) to the lane area so a zoomed/scrolled view never bleeds left over
    // the layer outline. The outline columns replace this clip with their own
    // (via `place`), so they draw unaffected.
    let saved_clip = ui.clip_rect();
    ui.set_clip_rect(saved_clip.intersect(lane_area));
    if let Some((a, b)) = comp.work_area {
        let band = egui::Rect::from_min_max(
            egui::pos2(x_of(a.0.to_f64()), ruler_rect.top()),
            egui::pos2(x_of(b.0.to_f64()), ruler_rect.top() + 4.0),
        );
        ui.painter().rect_filled(band, 0.0, theme.success);
    }
    // Nebula cache bar (docs/06 §5.6): mint runs where frames are in RAM and
    // play right now; blue where they are parked on disk, promotable. Never a
    // warning colour — an empty bar is normal, not a fault.
    #[cfg(feature = "media")]
    if let Some(bars) = app.cache_bar(comp) {
        use crate::app_state::CacheTier;
        let fps = comp.frame_rate.fps().max(1.0);
        for (tier, colour) in [
            (CacheTier::Ram, theme.success),
            (CacheTier::Disk, theme.cache_disk),
        ] {
            let mut run_start: Option<usize> = None;
            for f in 0..=bars.len() {
                let on = f < bars.len() && bars[f] == tier;
                match (on, run_start) {
                    (true, None) => run_start = Some(f),
                    (false, Some(s)) => {
                        let band = egui::Rect::from_min_max(
                            egui::pos2(x_of(s as f64 / fps), ruler_rect.bottom() - 2.0),
                            egui::pos2(x_of(f as f64 / fps), ruler_rect.bottom()),
                        );
                        ui.painter().rect_filled(band, 0.0, colour);
                        run_start = None;
                    }
                    _ => {}
                }
            }
        }
    }

    let label_every = (duration / 10.0).ceil().max(1.0) as usize;
    for s in 0..=duration.floor() as usize {
        let x = x_of(s as f64);
        ui.painter().line_segment(
            [
                egui::pos2(x, ruler_rect.bottom() - 6.0),
                egui::pos2(x, ruler_rect.bottom()),
            ],
            egui::Stroke::new(1.0_f32, theme.hairline_strong),
        );
        if s % label_every == 0 {
            ui.painter().text(
                egui::pos2(x + 3.0, ruler_rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{s}s"),
                egui::FontId::monospace(9.0),
                theme.text_muted,
            );
        }
    }
    // Detected tempo readout, right-aligned in the ruler.
    #[cfg(feature = "media")]
    if let Some((bc, bpm)) = app.detected_bpm {
        if bc == comp_id && bpm > 0.0 {
            ui.painter().text(
                egui::pos2(track_left + track_w - 2.0, ruler_rect.top() + 2.0),
                egui::Align2::RIGHT_TOP,
                format!("♪ {bpm:.0} BPM"),
                egui::FontId::monospace(9.0),
                theme.accent,
            );
        }
    }
    // Markers (docs 03 §11): beats are faint clay ticks fading by confidence;
    // user/chapter markers are full-height and solid.
    for m in &comp.markers {
        let x = x_of(m.time.0.to_f64());
        if x < track_left - 1.0 || x > track_left + track_w + 1.0 {
            continue;
        }
        let (col, top) = match m.kind {
            lumit_core::markers::MarkerKind::Beat { confidence } => (
                theme
                    .accent
                    .gamma_multiply(0.25 + 0.55 * confidence.clamp(0.0, 1.0)),
                ruler_rect.top() + 9.0,
            ),
            _ => (theme.accent, ruler_rect.top()),
        };
        ui.painter().line_segment(
            [egui::pos2(x, top), egui::pos2(x, ruler_rect.bottom())],
            egui::Stroke::new(1.0_f32, col),
        );
    }
    if ruler_resp.clicked() || ruler_resp.dragged() {
        if let Some(pos) = ruler_resp.interact_pointer_pos() {
            // Map the cursor through the displayed (zoomed, scrolled) time axis,
            // so the playhead lands under the pointer at any zoom.
            let raw = seconds_of(pos.x).clamp(0.0, duration);
            // Snap the scrub to a nearby marker (within ~6 px) so the playhead
            // lands on the beat (docs/impl/beat-detection.md grid assist).
            let threshold = drag_secs(6.0, px_per_sec);
            let secs = lumit_core::markers::snap_time(
                rational_at(raw),
                &comp.markers,
                rational_at(threshold),
            )
            .to_f64();
            app.preview_comp = Some(comp_id);
            app.comp_playback = None; // scrubbing pauses
            app.preview_frame =
                ((secs / duration * frames as f64) as usize).min(frames.saturating_sub(1));
            // Dragging is scrubbing: decode a coarse draft for instant feedback.
            // A plain click jumps once and wants the specified resolution.
            app.preview_draft = ruler_resp.dragged();
            #[cfg(feature = "media")]
            app.refresh_preview();
        }
    }
    if ruler_resp.drag_stopped() {
        // Scrub finished: reload the frame at the specified resolution.
        app.preview_draft = false;
        #[cfg(feature = "media")]
        app.refresh_preview();
    }
    // Audio waveform strip (mono peaks) beneath the ruler, aligned to the same
    // time axis so beats and transients line up.
    #[cfg(feature = "media")]
    if let Some((wc, wf)) = &app.comp_waveform {
        if *wc == comp_id && !wf.is_empty() {
            let (wave_rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(track_left, wave_rect.top()),
                    egui::pos2(track_left + track_w, wave_rect.bottom()),
                ),
                0.0,
                theme.surface_0,
            );
            let cy = wave_rect.center().y;
            let half = wave_rect.height() * 0.45;
            let n = wf.len().max(1) as f32;
            let col = theme.text_muted.gamma_multiply(0.7);
            for (i, (lo, hi)) in wf.iter().enumerate() {
                let x = track_left + (i as f32 / n) * track_w;
                ui.painter().line_segment(
                    [egui::pos2(x, cy - hi * half), egui::pos2(x, cy - lo * half)],
                    egui::Stroke::new(1.0_f32, col),
                );
            }
        }
    }
    let rows_top = ui.cursor().top();

    // Graph mode (K-070) keeps everything around the lanes — the ruler, the
    // layer outline, the scrollbars and the bottom bar — and swaps only the
    // lane content for the curve editor (the AE shape). The rows below still
    // render their outline columns in both modes; only the drawing to the
    // right of `track_left` is suppressed, and the curve fills that area
    // after the ScrollArea.

    // Lanes scroll vertically when there are more layers than fit. Full-width clip
    // inside so `place` (which re-clips each outline column) sees the viewport.
    ui.set_clip_rect(saved_clip);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height((ui.available_height() - 38.0).max(48.0)) // leave the bottom bar + hscrollbar
        .id_salt(("timeline-lanes", comp_id))
        .show(ui, |ui| {
            // The scroll viewport (full width). Lane content clips to lane_area ∩ it;
            // outline columns clip to their own x but this viewport's y (never bleeding
            // above the ruler when a row is half-scrolled).
            let viewport = ui.clip_rect();
            ui.set_clip_rect(viewport.intersect(lane_area));
            for layer in &comp.layers {
                // The row response only hit-tests over the lane area (the clip
                // above), so in graph mode it goes inert — clicks there belong
                // to the curve, and the outline keeps its own controls.
                let row_sense = if app.timeline_graph_mode {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click()
                };
                let (row_rect, row_resp) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), row_sense);
                if row_resp.clicked() {
                    app.selected_layer = Some(layer.id);
                }
                // Effects & Presets browser drop target (K-101): footage and
                // adjustment layers accept an effect dragged from the
                // browser, appended through the same `SetLayerEffects` op
                // the "Add effect" row commits, so a drop is one ordinary
                // undo step. `row_resp` only hit-tests the lane area (see
                // the `row_sense` comment above — the outline column has its
                // own, separately-interacted controls), so the hover cue and
                // the drop both land within that same region; dropping over
                // the name column does not yet register.
                if accepts_effect_drop(&layer.kind) {
                    if let Some(payload) = row_resp.dnd_release_payload::<EffectDragPayload>() {
                        if let Some(inst) = lumit_core::fx::instantiate(payload.0) {
                            let mut effects = layer.effects.clone();
                            effects.push(inst);
                            app.commit(lumit_core::Op::SetLayerEffects {
                                comp: comp_id,
                                layer: layer.id,
                                effects,
                            });
                            app.selected_layer = Some(layer.id);
                            #[cfg(feature = "media")]
                            app.refresh_preview();
                        }
                    } else if row_resp.dnd_hover_payload::<EffectDragPayload>().is_some() {
                        ui.painter().rect_stroke(
                            row_rect,
                            2.0,
                            egui::Stroke::new(1.0_f32, theme.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                // Right-click a layer to add things (the house pattern: right-click or
                // menu, never scattered buttons).
                let mut ctx_op: Option<lumit_core::Op> = None;
                let mut convert_layer = false;
                let mut trim_to_source = false;
                row_resp.context_menu(|ui| {
                    ui.menu_button("Add mask", |ui| {
                        let (w, h) = mask_space(layer, app, comp);
                        let mut new_mask = None;
                        if ui.button("Rectangle").clicked() {
                            new_mask = Some(lumit_core::mask::Mask::rectangle(
                                w * 0.25,
                                h * 0.25,
                                w * 0.5,
                                h * 0.5,
                            ));
                            ui.close_menu();
                        }
                        if ui.button("Ellipse").clicked() {
                            new_mask = Some(lumit_core::mask::Mask::ellipse(
                                w * 0.5,
                                h * 0.5,
                                w * 0.3,
                                h * 0.3,
                            ));
                            ui.close_menu();
                        }
                        if ui.button("Star").clicked() {
                            new_mask = Some(lumit_core::mask::Mask::star(
                                w * 0.5,
                                h * 0.5,
                                w * 0.32,
                                w * 0.14,
                                5,
                            ));
                            ui.close_menu();
                        }
                        if let Some(m) = new_mask {
                            let mut masks = layer.masks.clone();
                            masks.push(m);
                            ctx_op = Some(lumit_core::Op::SetLayerMasks {
                                comp: comp_id,
                                layer: layer.id,
                                masks,
                            });
                        }
                    });
                    // Footage → sequenced layer (K-071).
                    if matches!(layer.kind, lumit_core::model::LayerKind::Footage { .. })
                        && ui.button("Convert to sequenced layer").clicked()
                    {
                        convert_layer = true;
                        ui.close_menu();
                    }
                    // Trim to source end (K-022) — only offered for a retimed clip.
                    if matches!(
                        layer.kind,
                        lumit_core::model::LayerKind::Footage {
                            retime: Some(_),
                            ..
                        }
                    ) && ui.button("Trim to source end").clicked()
                    {
                        trim_to_source = true;
                        ui.close_menu();
                    }
                });
                if ctx_op.is_some() {
                    pending = ctx_op;
                    app.selected_layer = Some(layer.id);
                }
                if trim_to_source {
                    app.selected_layer = Some(layer.id);
                    #[cfg(feature = "media")]
                    app.trim_selected_to_source_end();
                }
                if convert_layer {
                    app.selected_layer = Some(layer.id);
                    app.convert_to_sequenced_layer();
                }
                // Outline glyphs draw left of the lanes, so they paint through a
                // viewport-clipped painter, not the lane clip (which would hide
                // them). Child-UI columns (place/row_frame) already clip this way.
                // NB: with_clip_rect INTERSECTS with the ui's current clip, which
                // was narrowed to the lanes above — it would erase everything left
                // of the lanes (the twirl and every outline glyph). set_clip_rect
                // REPLACES the clip, so the full-width outline draws.
                let mut outline_painter = ui.painter().clone();
                outline_painter.set_clip_rect(viewport);
                // Selection highlight is the background — drawn first so the twirl
                // and glyphs on top of it stay visible.
                if app.selected_layer == Some(layer.id) {
                    outline_painter.rect_filled(
                        egui::Rect::from_min_max(
                            row_rect.min,
                            egui::pos2(row_rect.left() + name_w - 4.0, row_rect.bottom()),
                        ),
                        3.0,
                        theme.surface_2,
                    );
                }
                // Disclosure twirl: layer options hide until opened (AE behaviour).
                let twirl_id = ui.id().with(("twirl", layer.id));
                let mut expanded = ui.data(|d| d.get_temp::<bool>(twirl_id).unwrap_or(false));
                let tri = egui::Rect::from_min_size(
                    egui::pos2(row_rect.left() + 2.0, row_rect.top()),
                    egui::vec2(16.0, row_rect.height()),
                );
                // The ui's clip is the lanes, and egui hit-tests against rect ∩ clip
                // — so an outline interaction needs the clip widened, or the click
                // never registers (this is why the twirl looked dead).
                let tri_resp = {
                    let saved = ui.clip_rect();
                    ui.set_clip_rect(viewport);
                    let r = ui.interact(tri, twirl_id.with("hit"), egui::Sense::click());
                    ui.set_clip_rect(saved);
                    r
                };
                if tri_resp.clicked() {
                    expanded = !expanded;
                    ui.data_mut(|d| d.insert_temp(twirl_id, expanded));
                }
                // Secondary (not muted) so the twirl reads as a control against
                // the dark outline; brightens under the cursor like other glyphs.
                let twirl_col = if tri_resp.hovered() {
                    theme.text_primary
                } else {
                    theme.text_secondary
                };
                crate::icons::disclosure(&outline_painter, tri, expanded, twirl_col);
                // Left-column subcolumns (Mack): [visibility][title…][matte][blend][3D]
                // [mute]. Switches are right-anchored so they align across every row;
                // the title flexes and truncates. Each is clipped to its slot, so a
                // narrow column just crops controls off the edge.
                let top = row_rect.top();
                let bot = row_rect.bottom();
                let edge = track_left - 6.0;
                let slot = |x0: f32, x1: f32| {
                    egui::Rect::from_min_max(
                        egui::pos2(x0.max(row_rect.left() + 18.0), top),
                        egui::pos2(x1.max(x0 + 1.0), bot),
                    )
                };
                let is_footage = matches!(layer.kind, lumit_core::model::LayerKind::Footage { .. });
                let eye_r = slot(row_rect.left() + 18.0, row_rect.left() + 36.0);
                let mute_r = slot(edge - 34.0, edge);
                let td_r = slot(edge - 60.0, edge - 38.0);
                // Flow option toggle (K-088), footage layers only.
                let flow_r = slot(edge - 86.0, edge - 64.0);
                let blend_r = slot(edge - 150.0, edge - 90.0);
                let matte_r = slot(edge - 204.0, edge - 154.0);
                // Layer type is encoded by the lane bar's colour (Mack) — no glyph
                // or colour tab in the outline.
                let title_r = slot(row_rect.left() + 58.0, edge - 208.0);
                let place =
                    |ui: &mut egui::Ui, r: egui::Rect, add: &mut dyn FnMut(&mut egui::Ui)| {
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(r)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        // Clip to the column's own x but the scroll viewport's y, so an
                        // outline column in a half-scrolled row doesn't bleed past the ruler.
                        child.set_clip_rect(r.intersect(viewport));
                        add(&mut child);
                    };
                let mut select_this = false;
                place(ui, eye_r, &mut |ui| {
                    visible_control(ui, theme, comp_id, layer, &mut pending)
                });
                place(ui, title_r, &mut |ui| {
                    if ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(trim_title(&layer.name))
                                    .small()
                                    .color(theme.text_secondary),
                            )
                            .truncate()
                            .sense(egui::Sense::click()),
                        )
                        .clicked()
                    {
                        select_this = true;
                    }
                });
                place(ui, matte_r, &mut |ui| {
                    matte_control(ui, theme, comp, comp_id, layer, &mut pending)
                });
                place(ui, blend_r, &mut |ui| {
                    blend_control(ui, comp_id, layer, &mut pending)
                });
                place(ui, td_r, &mut |ui| {
                    three_d_control(ui, comp_id, layer, &mut pending)
                });
                if is_footage {
                    place(ui, mute_r, &mut |ui| {
                        mute_control(ui, theme, comp_id, layer, &mut pending)
                    });
                    place(ui, flow_r, &mut |ui| {
                        flow_control(ui, theme, comp_id, layer, &mut pending)
                    });
                } else if matches!(layer.kind, lumit_core::model::LayerKind::Precomp { .. }) {
                    // Precomp layers have no audio; their slot carries the
                    // collapse switch (docs/06 §1.4) instead.
                    let clt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
                        - layer.start_offset.0.to_f64();
                    place(ui, mute_r, &mut |ui| {
                        collapse_control(ui, theme, &doc, comp, comp_id, layer, clt, &mut pending)
                    });
                }
                if select_this {
                    app.selected_layer = Some(layer.id);
                }
                // Lane content — the bar, its labels and its drag handles — only
                // draws in the layers view; in graph mode the lane area belongs
                // to the curve (drawn after the ScrollArea).
                if !app.timeline_graph_mode {
                    // A whole-layer move (dragging the bar body) slides the bar for preview;
                    // the snapped landing is what draws.
                    let move_dx = match app.move_edit {
                        Some((id, raw_in)) if id == layer.id => {
                            let thr = drag_secs(6.0, px_per_sec);
                            let snapped = lumit_core::markers::snap_time(
                                rational_at(raw_in.max(0.0)),
                                &comp.markers,
                                rational_at(thr),
                            )
                            .to_f64();
                            snapped - layer.in_point.0.to_f64()
                        }
                        _ => 0.0,
                    };
                    let bar = egui::Rect::from_min_max(
                        egui::pos2(
                            x_of(layer.in_point.0.to_f64() + move_dx),
                            row_rect.top() + 2.0,
                        ),
                        egui::pos2(
                            x_of(layer.out_point.0.to_f64() + move_dx),
                            row_rect.bottom() - 2.0,
                        ),
                    );
                    // The bar wears its layer's identity (15-DESIGN §6.1, Mack):
                    // a quiet tonal wash of the type colour over the neutral fill
                    // and a 3px type tab on the bar's left edge. Muted siblings by
                    // design — selection and hover (accent) beat every one of them.
                    let type_colour = layer_type_style(&layer.kind, theme).1;
                    ui.painter().rect_filled(bar, 3.0, theme.surface_3);
                    ui.painter()
                        .rect_filled(bar, 3.0, type_colour.gamma_multiply(0.13));
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(
                            bar.left_top(),
                            egui::pos2(bar.left() + 3.0, bar.bottom()),
                        ),
                        egui::CornerRadius {
                            nw: 3,
                            sw: 3,
                            ne: 0,
                            se: 0,
                        },
                        type_colour,
                    );
                    ui.painter().rect_stroke(
                        bar,
                        3.0,
                        egui::Stroke::new(1.0_f32, theme.hairline_strong),
                        egui::StrokeKind::Inside,
                    );
                    // Overrun (K-022): a retimed clip that outruns its source holds
                    // the boundary frame — wash and hatch the held span in warning
                    // kraft (15-DESIGN §6.4: calm, never a red alarm) with a hairline
                    // tick at the exact exhaustion point and a HOLD tag when there is
                    // room. Indication only: boundaries never move on their own.
                    #[cfg(feature = "media")]
                    let mut overrun_span: Option<egui::Rect> = None;
                    #[cfg(feature = "media")]
                    if let lumit_core::model::LayerKind::Footage {
                        item,
                        retime: Some(rt),
                    } = &layer.kind
                    {
                        use crate::app_state::media::MediaStatus;
                        if let Some(MediaStatus::Ready { probe, frames, .. }) =
                            app.media.map.get(item)
                        {
                            if let Some(v) = probe.video.as_ref() {
                                let src_dur = *frames as f64 / v.fps().max(1.0);
                                if let Some((span_in, span_out)) = overrun_span_secs(
                                    rt,
                                    src_dur,
                                    layer.start_offset.0.to_f64(),
                                    layer.in_point.0.to_f64(),
                                    layer.out_point.0.to_f64(),
                                ) {
                                    let sx = x_of(span_in + move_dx).max(bar.left());
                                    let ex = x_of(span_out + move_dx).min(bar.right());
                                    if ex - sx > 0.5 {
                                        let span = egui::Rect::from_min_max(
                                            egui::pos2(sx, bar.top()),
                                            egui::pos2(ex, bar.bottom()),
                                        );
                                        // Low-alpha wash so the held region reads as
                                        // one piece even where the hatch gets sparse.
                                        ui.painter().rect_filled(
                                            span,
                                            egui::CornerRadius {
                                                nw: 0,
                                                sw: 0,
                                                ne: 3,
                                                se: 3,
                                            },
                                            theme.warning.gamma_multiply(0.14),
                                        );
                                        // 45° hatching, 1px lines on a 4px
                                        // perpendicular pitch (§6.4) — so the
                                        // horizontal step is 4·√2. Clipped to the
                                        // span, so diagonals end cleanly at its edges.
                                        let hatch = ui.painter().with_clip_rect(span);
                                        let stroke = egui::Stroke::new(
                                            1.0_f32,
                                            theme.warning.gamma_multiply(0.6),
                                        );
                                        let rise = span.height();
                                        let step = 4.0 * std::f32::consts::SQRT_2;
                                        let mut hx = span.left() - rise;
                                        while hx < span.right() {
                                            hatch.line_segment(
                                                [
                                                    egui::pos2(hx, span.bottom()),
                                                    egui::pos2(hx + rise, span.top()),
                                                ],
                                                stroke,
                                            );
                                            hx += step;
                                        }
                                        // The exhaustion tick — only when the crossing
                                        // itself is on the bar (a fully-held bar has
                                        // its crossing left of the in point).
                                        if sx > bar.left() + 0.5 {
                                            ui.painter().line_segment(
                                                [
                                                    egui::pos2(sx, bar.top()),
                                                    egui::pos2(sx, bar.bottom()),
                                                ],
                                                egui::Stroke::new(1.0_f32, theme.warning),
                                            );
                                        }
                                        if span.width() > 40.0 {
                                            ui.painter().text(
                                                span.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "HOLD",
                                                egui::FontId::monospace(8.0),
                                                theme.warning,
                                            );
                                        }
                                        overrun_span = Some(span);
                                    }
                                }
                            }
                        }
                    }
                    if layer.matte.is_some() {
                        ui.painter().text(
                            egui::pos2(bar.right() - 4.0, bar.center().y),
                            egui::Align2::RIGHT_CENTER,
                            "matte",
                            egui::FontId::monospace(8.0),
                            theme.text_muted,
                        );
                    }
                    // Keyframe glyphs: a clay diamond on the bar at each keyframed time
                    // (across the layer's animated properties). Only when collapsed — when
                    // expanded, each property shows its own keys on its own row (K-072).
                    // Times are layer-local, so comp time = start_offset + keyframe time.
                    if !expanded {
                        let off = layer.start_offset.0.to_f64();
                        let cy = bar.center().y;
                        for kt in layer_keyframe_times(layer) {
                            let x = x_of(off + kt);
                            if x >= bar.left() - 1.0 && x <= bar.right() + 1.0 {
                                let d = 3.5;
                                ui.painter().add(egui::Shape::convex_polygon(
                                    vec![
                                        egui::pos2(x, cy - d),
                                        egui::pos2(x + d, cy),
                                        egui::pos2(x, cy + d),
                                        egui::pos2(x - d, cy),
                                    ],
                                    theme.accent,
                                    egui::Stroke::new(1.0_f32, theme.surface_0),
                                ));
                            }
                        }
                    }
                    // Sequence layers show their clips as sub-bars; gaps show the darker
                    // base bar; each edit point gets a clay tick.
                    if let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind {
                        let off = layer.start_offset.0.to_f64();
                        for clip in clips {
                            let cs = x_of(off + clip.place_start.to_f64());
                            let ce = x_of(off + clip.place_end().to_f64());
                            let crect = egui::Rect::from_min_max(
                                egui::pos2(cs, bar.top() + 1.0),
                                egui::pos2(ce, bar.bottom() - 1.0),
                            );
                            // Click a clip to select it (for per-clip speed editing).
                            let cresp = ui.interact(
                                crect,
                                ui.id().with(("clip", clip.id)),
                                egui::Sense::click(),
                            );
                            if cresp.clicked() {
                                app.selected_clip = Some(clip.id);
                                app.selected_layer = Some(layer.id);
                            }
                            let sel = app.selected_clip == Some(clip.id);
                            ui.painter().rect(
                                crect,
                                2.0,
                                if sel {
                                    theme.surface_3
                                } else {
                                    theme.surface_2
                                },
                                egui::Stroke::new(
                                    if sel { 1.5_f32 } else { 1.0_f32 },
                                    if sel {
                                        theme.accent
                                    } else {
                                        theme.hairline_strong
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                            // A non-100% clip shows its speed.
                            if let Some(sp) = clip.constant_speed() {
                                if (sp - 1.0).abs() > 1e-6 && ce - cs > 24.0 {
                                    ui.painter().text(
                                        crect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        format!("{:.0}%", sp * 100.0),
                                        egui::FontId::monospace(8.0),
                                        theme.text_secondary,
                                    );
                                }
                            }
                            // Edit point (clip boundary) — the beat-sync landmark.
                            ui.painter().line_segment(
                                [egui::pos2(ce, bar.top()), egui::pos2(ce, bar.bottom())],
                                egui::Stroke::new(1.0_f32, theme.accent),
                            );
                        }
                    }

                    // Only bars visible in the (possibly zoomed/scrolled) lane area take
                    // pointer interaction, so an off-screen bar can't steal outline clicks.
                    let bar_visible = bar.right() > track_left + 0.5 && bar.left() < panel_right;
                    // Edge handles: drag to trim in/out (one SetLayerSpan op per release).
                    for out_edge in [false, true] {
                        if !bar_visible {
                            continue;
                        }
                        let edge_x = if out_edge { bar.right() } else { bar.left() };
                        let handle = egui::Rect::from_center_size(
                            egui::pos2(edge_x, bar.center().y),
                            egui::vec2(8.0, bar.height()),
                        );
                        let resp = ui.interact(
                            handle,
                            ui.id().with(("trim", layer.id, out_edge)),
                            egui::Sense::drag(),
                        );
                        if resp.hovered() || resp.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }
                        if resp.dragged() {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                // Snap the trimmed edge to a nearby beat/marker (~6 px) so
                                // clips cut on the beat.
                                let threshold = drag_secs(6.0, px_per_sec);
                                let secs = lumit_core::markers::snap_time(
                                    rational_at(seconds_of(pos.x)),
                                    &comp.markers,
                                    rational_at(threshold),
                                )
                                .to_f64();
                                app.trim_edit = Some((layer.id, out_edge, secs));
                            }
                        }
                        if resp.drag_stopped() {
                            if let Some((id, is_out, secs)) = app.trim_edit.take() {
                                if id == layer.id && is_out == out_edge {
                                    let (mut new_in, mut new_out) =
                                        (layer.in_point.0.to_f64(), layer.out_point.0.to_f64());
                                    if is_out {
                                        new_out = secs;
                                    } else {
                                        new_in = secs;
                                    }
                                    let min_len = 1.0 / comp.frame_rate.fps().max(1.0);
                                    if new_out - new_in >= min_len {
                                        pending = Some(lumit_core::Op::SetLayerSpan {
                                            comp: comp_id,
                                            layer: layer.id,
                                            in_point: lumit_core::time::CompTime(rational_at(
                                                new_in,
                                            )),
                                            out_point: lumit_core::time::CompTime(rational_at(
                                                new_out,
                                            )),
                                            start_offset: layer.start_offset,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    // Body drag: move the whole layer in comp time — shift in/out and
                    // start_offset together so the bar and its content move as one. Sequence
                    // layers keep their bodies for clip selection.
                    if bar_visible
                        && !matches!(layer.kind, lumit_core::model::LayerKind::Sequence { .. })
                    {
                        let body = bar.shrink2(egui::vec2(6.0, 0.0));
                        if body.width() > 2.0 {
                            let resp = ui.interact(
                                body,
                                ui.id().with(("move", layer.id)),
                                egui::Sense::click_and_drag(),
                            );
                            if resp.dragged() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            } else if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            // Hovering the held (overrun) tail says what the kraft
                            // wash means — but not mid-drag, where a tooltip would
                            // sit under the hand.
                            #[cfg(feature = "media")]
                            if let Some(span) = overrun_span {
                                if !resp.dragged()
                                    && resp.hover_pos().is_some_and(|p| span.contains(p))
                                {
                                    resp.clone()
                                        .on_hover_text("Source ends here — holding the last frame");
                                }
                            }
                            if resp.clicked() {
                                app.selected_layer = Some(layer.id);
                            }
                            if resp.dragged() {
                                let dx_secs = drag_secs(resp.drag_delta().x as f64, px_per_sec);
                                let base = match app.move_edit {
                                    Some((id, s)) if id == layer.id => s,
                                    _ => layer.in_point.0.to_f64(),
                                };
                                app.move_edit = Some((layer.id, (base + dx_secs).max(0.0)));
                            }
                            if resp.drag_stopped() {
                                if let Some((id, raw_in)) = app.move_edit.take() {
                                    if id == layer.id {
                                        let thr = drag_secs(6.0, px_per_sec);
                                        let snapped = lumit_core::markers::snap_time(
                                            rational_at(raw_in.max(0.0)),
                                            &comp.markers,
                                            rational_at(thr),
                                        )
                                        .to_f64();
                                        let delta = snapped - layer.in_point.0.to_f64();
                                        if delta.abs() > 1e-9 {
                                            let (in_point, out_point, start_offset) = moved_span(
                                                layer.in_point,
                                                layer.out_point,
                                                layer.start_offset,
                                                delta,
                                            );
                                            pending = Some(lumit_core::Op::SetLayerSpan {
                                                comp: comp_id,
                                                layer: layer.id,
                                                in_point,
                                                out_point,
                                                start_offset,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Live trim feedback: provisional edge drawn in clay.
                    if let Some((id, _is_out, secs)) = app.trim_edit {
                        if id == layer.id {
                            let x = x_of(secs);
                            ui.painter().line_segment(
                                [egui::pos2(x, bar.top()), egui::pos2(x, bar.bottom())],
                                egui::Stroke::new(2.0_f32, theme.accent),
                            );
                        }
                    }
                } // end of the lanes-view-only lane content
                if expanded {
                    // Layer options live in the left column only — the track area to
                    // the right of the separator stays for the bar and keyframes.
                    ui.scope(|ui| {
                        ui.set_max_width(name_w - 10.0);
                        ui.indent(("layer-opts", layer.id), |ui| {
                            // Masks only appear once the layer has one (add via right-click or
                            // the toolbar's mask tool).
                            if !layer.masks.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        egui::RichText::new("Masks")
                                            .small()
                                            .color(theme.text_muted),
                                    );
                                    for (mi, mask) in layer.masks.iter().enumerate() {
                                        let mut masks = layer.masks.clone();
                                        if ui
                                            .selectable_label(
                                                mask.inverted,
                                                egui::RichText::new(format!("{} inv", mask.name))
                                                    .small(),
                                            )
                                            .clicked()
                                        {
                                            masks[mi].inverted = !masks[mi].inverted;
                                            pending = Some(lumit_core::Op::SetLayerMasks {
                                                comp: comp_id,
                                                layer: layer.id,
                                                masks,
                                            });
                                        } else if ui
                                            .small_button("×")
                                            .on_hover_text("Remove mask")
                                            .clicked()
                                        {
                                            masks.remove(mi);
                                            pending = Some(lumit_core::Op::SetLayerMasks {
                                                comp: comp_id,
                                                layer: layer.id,
                                                masks,
                                            });
                                        }
                                    }
                                });
                            }
                        });
                        if let lumit_core::model::LayerKind::Text { document } = &layer.kind {
                            ui.indent(("text", layer.id), |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("Text").small().color(theme.text_muted),
                                    );
                                    let mut text = document.text.clone();
                                    let resp = ui.add(
                                        egui::TextEdit::singleline(&mut text).desired_width(180.0),
                                    );
                                    let mut size = document.size;
                                    let size_resp = ui.add(
                                        egui::DragValue::new(&mut size)
                                            .speed(1.0)
                                            .range(4.0..=512.0)
                                            .suffix(" px"),
                                    );
                                    if (resp.lost_focus() && text != document.text)
                                        || (size_resp.drag_stopped() || size_resp.lost_focus())
                                            && (size - document.size).abs() > f64::EPSILON
                                    {
                                        let mut doc_new = document.clone();
                                        doc_new.text = text;
                                        doc_new.size = size;
                                        pending = Some(lumit_core::Op::SetTextDocument {
                                            comp: comp_id,
                                            layer: layer.id,
                                            document: doc_new,
                                        });
                                    }
                                });
                            });
                        }
                        if let lumit_core::model::LayerKind::Camera { zoom } = &layer.kind {
                            ui.indent(("camera", layer.id), |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("Zoom px")
                                            .small()
                                            .color(theme.text_muted),
                                    );
                                    let fps = comp.frame_rate.fps().max(1.0);
                                    let lt = app.preview_frame as f64 / fps
                                        - layer.start_offset.0.to_f64();
                                    let committed = zoom.value_at(lt);
                                    let id = egui::Id::new(("zoom_edit", layer.id));
                                    let mut value =
                                        ui.data(|d| d.get_temp::<f64>(id)).unwrap_or(committed);
                                    let resp = ui.add(
                                        egui::DragValue::new(&mut value)
                                            .speed(4.0)
                                            .range(1.0..=100_000.0)
                                            .max_decimals(1),
                                    );
                                    if resp.dragged() || resp.has_focus() {
                                        ui.data_mut(|d| d.insert_temp(id, value));
                                    }
                                    if resp.drag_stopped() || resp.lost_focus() {
                                        if (value - committed).abs() > f64::EPSILON {
                                            let animation = if zoom.is_animated() {
                                                Animation::Keyframed(upsert_key(zoom, lt, value))
                                            } else {
                                                Animation::Static(value)
                                            };
                                            pending = Some(lumit_core::Op::SetCameraZoom {
                                                comp: comp_id,
                                                layer: layer.id,
                                                animation,
                                            });
                                        }
                                        ui.data_mut(|d| d.remove::<f64>(id));
                                    }
                                });
                            });
                        }
                        // Per-clip speed for the selected clip in a Sequence layer.
                        if let lumit_core::model::LayerKind::Sequence { clips } = &layer.kind {
                            if let Some(cid) = app.selected_clip {
                                if let Some(clip) = clips.iter().find(|c| c.id == cid) {
                                    ui.indent(("clipspeed", layer.id), |ui| {
                                        // Speed ramp: start % → end % with an ease (equal
                                        // ends = constant speed). The montage gesture.
                                        ui.horizontal(|ui| {
                                            use lumit_core::retime::Ease;
                                            ui.label(
                                                egui::RichText::new("Speed %")
                                                    .small()
                                                    .color(theme.text_muted),
                                            );
                                            let (rv0, rv1, rease) = clip
                                                .ramp_view()
                                                .map(|(a, b, e)| (a * 100.0, b * 100.0, e))
                                                .unwrap_or((100.0, 100.0, Ease::Linear));
                                            let id0 = egui::Id::new(("clipv0", cid));
                                            let mut s0 =
                                                ui.data(|d| d.get_temp::<f64>(id0)).unwrap_or(rv0);
                                            let r0 = ui.add(
                                                egui::DragValue::new(&mut s0)
                                                    .speed(1.0)
                                                    .range(-800.0..=800.0),
                                            );
                                            if r0.dragged() || r0.has_focus() {
                                                ui.data_mut(|d| d.insert_temp(id0, s0));
                                            }
                                            ui.label(egui::RichText::new("→").small());
                                            let id1 = egui::Id::new(("clipv1", cid));
                                            let mut s1 =
                                                ui.data(|d| d.get_temp::<f64>(id1)).unwrap_or(rv1);
                                            let r1 = ui.add(
                                                egui::DragValue::new(&mut s1)
                                                    .speed(1.0)
                                                    .range(-800.0..=800.0)
                                                    .suffix(" %"),
                                            );
                                            if r1.dragged() || r1.has_focus() {
                                                ui.data_mut(|d| d.insert_temp(id1, s1));
                                            }
                                            let mut new_ease = rease;
                                            bare_dropdown(ui, ease_label(rease), |ui| {
                                                for e in [
                                                    Ease::Linear,
                                                    Ease::Slow,
                                                    Ease::Fast,
                                                    Ease::Smooth,
                                                    Ease::Sharp,
                                                ] {
                                                    if ui
                                                        .selectable_label(e == rease, ease_label(e))
                                                        .clicked()
                                                    {
                                                        new_ease = e;
                                                        ui.close_menu();
                                                    }
                                                }
                                            });
                                            let released = r0.drag_stopped()
                                                || r0.lost_focus()
                                                || r1.drag_stopped()
                                                || r1.lost_focus();
                                            if released
                                                && ((s0 - rv0).abs() > 1e-6
                                                    || (s1 - rv1).abs() > 1e-6)
                                            {
                                                clip_ramp_edit = Some((s0, s1, rease));
                                                ui.data_mut(|d| {
                                                    d.remove::<f64>(id0);
                                                    d.remove::<f64>(id1);
                                                });
                                            }
                                            if new_ease != rease {
                                                clip_ramp_edit = Some((s0, s1, new_ease));
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            use lumit_core::retime::{FlowParams, Interpolation};
                                            ui.label(
                                                egui::RichText::new("Frames")
                                                    .small()
                                                    .color(theme.text_muted),
                                            );
                                            for (label, val, active) in [
                                                (
                                                    "Nearest",
                                                    Interpolation::Nearest,
                                                    matches!(
                                                        clip.interpolation,
                                                        Interpolation::Nearest
                                                    ),
                                                ),
                                                (
                                                    "Blend",
                                                    Interpolation::Blend,
                                                    matches!(
                                                        clip.interpolation,
                                                        Interpolation::Blend
                                                    ),
                                                ),
                                                (
                                                    "Flow",
                                                    Interpolation::Flow(FlowParams::default()),
                                                    matches!(
                                                        clip.interpolation,
                                                        Interpolation::Flow(_)
                                                    ),
                                                ),
                                            ] {
                                                if ui.selectable_label(active, label).clicked()
                                                    && !active
                                                {
                                                    clip_interp_edit = Some(val);
                                                }
                                            }
                                        });
                                    });
                                }
                            }
                        }
                    });

                    // (Frame-interpolation choice — Nearest / Blend / Flow — is not
                    // surfaced here for now; it will return in a dedicated place.)
                    // Transform group: its own twirl (open by default) revealing each
                    // animatable property as a timeline row — stopwatch/name/value in
                    // the left column, that property's keyframes on the track to the
                    // right; click a row to graph it (K-072).
                    let tf_id = ui.id().with(("transform-group", layer.id));
                    if group_header_row(ui, theme, "Transform", tf_id, true, viewport) {
                        transform_property_rows(
                            ui,
                            theme,
                            app,
                            comp,
                            comp_id,
                            layer,
                            name_w,
                            track_left,
                            track_w,
                            px_per_sec,
                            view_start,
                            viewport,
                            &mut pending,
                        );
                    }
                    // Effects group (docs/08): the layer's effect stack. Each
                    // effect is a compact block — enable / name / remove on its
                    // title row, then one animatable row per parameter.
                    let fx_id = ui.id().with(("effects-group", layer.id));
                    if group_header_row(ui, theme, "Effects", fx_id, false, viewport) {
                        let fps2 = comp.frame_rate.fps().max(1.0);
                        let fx_ctx = RowCtx {
                            theme,
                            comp_id,
                            layer,
                            lt: app.preview_frame as f64 / fps2 - layer.start_offset.0.to_f64(),
                            off: layer.start_offset.0.to_f64(),
                            fps: fps2,
                            viewport,
                            track_left,
                            track_w,
                            px_per_sec,
                            view_start,
                            graph_mode: app.timeline_graph_mode,
                        };
                        effects_rows(ui, &fx_ctx, &mut pending);
                    }
                    // Flow group (K-088): present only while the option is on.
                    if matches!(
                        &layer.kind,
                        lumit_core::model::LayerKind::Footage { retime: Some(rt), .. }
                            if matches!(rt.interpolation, lumit_core::retime::Interpolation::Flow(_))
                    ) {
                        let flow_id = ui.id().with(("flow-group", layer.id));
                        if group_header_row(ui, theme, "Flow", flow_id, true, viewport) {
                            let fps3 = comp.frame_rate.fps().max(1.0);
                            let flow_ctx = RowCtx {
                                theme,
                                comp_id,
                                layer,
                                lt: app.preview_frame as f64 / fps3
                                    - layer.start_offset.0.to_f64(),
                                off: layer.start_offset.0.to_f64(),
                                fps: fps3,
                                viewport,
                                track_left,
                                track_w,
                                px_per_sec,
                                view_start,
                                graph_mode: app.timeline_graph_mode,
                            };
                            flow_group_rows(ui, &flow_ctx, &mut pending);
                        }
                    }
                }
            }
        });
    // Time-positioned overlays (marker guides, playhead) re-clip to the lane area.
    ui.set_clip_rect(saved_clip.intersect(lane_area));
    // Vertical separator + drag handle: resizes the left column (Mack).
    let sep_bottom = ui.cursor().top();
    // Faint vertical guide lines through the track rows — beats by default so
    // cuts line up across every layer and the waveform, or the time grid
    // (seconds, subdividing with zoom), per the bottom-bar Grid pick. Lanes
    // view only: the graph draws its own grid over that area.
    if !app.timeline_graph_mode {
        match app.timeline_grid {
            crate::app_state::TimelineGrid::Beats => {
                for m in &comp.markers {
                    let x = x_of(m.time.0.to_f64());
                    if x < track_left - 1.0 || x > track_left + track_w + 1.0 {
                        continue;
                    }
                    let a = match m.kind {
                        lumit_core::markers::MarkerKind::Beat { confidence } => {
                            0.10 + 0.15 * confidence.clamp(0.0, 1.0)
                        }
                        _ => 0.4,
                    };
                    ui.painter().line_segment(
                        [egui::pos2(x, rows_top), egui::pos2(x, sep_bottom)],
                        egui::Stroke::new(1.0_f32, theme.accent.gamma_multiply(a)),
                    );
                }
            }
            crate::app_state::TimelineGrid::Time => {
                // Neutral gridlines at the largest step under ~70 px, whole
                // seconds a touch stronger than their subdivisions.
                let step = time_grid_step(px_per_sec);
                let view_end = view_start + track_w as f64 / px_per_sec.max(1e-6);
                let mut k = (view_start / step).floor().max(0.0) as u64;
                loop {
                    let t = k as f64 * step;
                    if t > view_end || t > comp.duration.0.to_f64() {
                        break;
                    }
                    let x = x_of(t);
                    if x >= track_left - 1.0 && x <= track_left + track_w + 1.0 {
                        let whole = (t - t.round()).abs() < step * 0.25;
                        let a = if whole { 0.5 } else { 0.25 };
                        ui.painter().line_segment(
                            [egui::pos2(x, rows_top), egui::pos2(x, sep_bottom)],
                            egui::Stroke::new(1.0_f32, theme.hairline_strong.gamma_multiply(a)),
                        );
                    }
                    k += 1;
                }
            }
            crate::app_state::TimelineGrid::Off => {}
        }
    }
    let sep_x = track_left - 4.0;
    let handle = egui::Rect::from_min_max(
        egui::pos2(sep_x - 3.0, rows_top),
        egui::pos2(sep_x + 3.0, sep_bottom.max(rows_top + 1.0)),
    );
    let hresp = ui.interact(
        handle,
        ui.id().with("name-col-resize"),
        egui::Sense::click_and_drag(),
    );
    if hresp.hovered() || hresp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if hresp.dragged() {
        app.timeline_name_w = (app.timeline_name_w + hresp.drag_delta().x).clamp(96.0, 900.0);
    }
    ui.painter().line_segment(
        [egui::pos2(sep_x, rows_top), egui::pos2(sep_x, sep_bottom)],
        egui::Stroke::new(
            1.0_f32,
            if hresp.hovered() || hresp.dragged() {
                theme.accent
            } else {
                theme.hairline
            },
        ),
    );
    // Playhead over ruler and rows (clay, the one accent). In graph mode it
    // stops at the ruler: the curve draws its own playhead on its own axis.
    if app.preview_comp == Some(comp_id) {
        let x = x_of(app.preview_frame as f64 / comp.frame_rate.fps().max(1.0));
        let playhead_bottom = if app.timeline_graph_mode {
            rows_top
        } else {
            ui.cursor().top().max(rows_top)
        };
        ui.painter().line_segment(
            [
                egui::pos2(x, ruler_rect.top()),
                egui::pos2(x, playhead_bottom),
            ],
            egui::Stroke::new(1.5_f32, theme.accent),
        );
    }
    if let Some(op) = pending {
        follow_edit(app, &op); // the graph follows the key you just touched
        app.commit(op);
    }
    if let Some((v0, v1, ease)) = clip_ramp_edit {
        app.set_selected_clip_ramp(v0, v1, ease);
    }
    if let Some(interp) = clip_interp_edit {
        app.set_selected_clip_interp(interp);
    }
    // Graph mode (K-070): the curve editor fills the lane area — the outline,
    // ruler, scrollbars and bottom bar around it are the same in both views.
    // (Sharing the lanes' zoomed time axis is the next increment.)
    if app.timeline_graph_mode {
        let plot_rect = graph_lane_rect(track_left, track_w, rows_top, ui.max_rect().bottom());
        graph_lane_plot(ui, theme, app, comp, px_per_sec, view_start, plot_rect);
    } else {
        // No plot on screen: drop any in-flight band and keyframe selection
        // (a selection must never outlive the curve it was made on).
        app.graph_marquee = None;
        app.graph_selection = None;
    }
    ui.set_clip_rect(saved_clip); // release the lane clip for the bottom bar
    timeline_bottom_bar(
        ui,
        theme,
        app,
        track_left,
        panel_right,
        duration,
        view_start,
    );
}

/// The layer-view / graph-view switch (K-070), drawn right-anchored in `rect`.
pub(crate) fn graph_toggle(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState, rect: egui::Rect) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.set_clip_rect(rect);
    let graph = app.timeline_graph_mode;
    if icon_button(&mut child, theme, Icon::GraphCurve, graph)
        .on_hover_text("Graph editor")
        .clicked()
    {
        app.timeline_graph_mode = true;
    }
    if icon_button(&mut child, theme, Icon::TimelineBars, !graph)
        .on_hover_text("Layers")
        .clicked()
    {
        app.timeline_graph_mode = false;
    }
}

/// The lane-view bottom bar spanning the lanes area: zoom controls (`− + Fit` +
/// readout) on the left — joined in graph mode by the value/speed lens toggle,
/// its own group behind a hairline divider — the view toggle on the right, and
/// a draggable horizontal scrollbar just above it (shown when zoomed in).
pub(crate) fn timeline_bottom_bar(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    track_left: f32,
    panel_right: f32,
    duration: f64,
    view_start: f64,
) {
    let panel = ui.max_rect();
    let bar_top = panel.bottom() - 24.0;
    let sb_top = bar_top - 12.0;

    // Horizontal scrollbar (meaningful only when zoomed in), just above the bar.
    if app.timeline_zoom > 1.001 {
        let track = egui::Rect::from_min_max(
            egui::pos2(track_left, sb_top),
            egui::pos2(panel_right - 2.0, sb_top + 8.0),
        );
        ui.painter().rect_filled(track, 3.0, theme.surface_1);
        let visible = duration / app.timeline_zoom;
        let fs = (view_start / duration.max(1e-6)) as f32;
        let fl = (visible / duration.max(1e-6)) as f32;
        let thumb = egui::Rect::from_min_max(
            egui::pos2(track.left() + fs * track.width(), track.top()),
            egui::pos2(track.left() + (fs + fl) * track.width(), track.bottom()),
        );
        let resp = ui.interact(thumb, ui.id().with("h-scrollbar"), egui::Sense::drag());
        let col = if resp.hovered() || resp.dragged() {
            theme.accent
        } else {
            theme.text_muted
        };
        ui.painter().rect_filled(thumb, 3.0, col);
        if resp.dragged() {
            let dsec = resp.drag_delta().x as f64 / track.width().max(1.0) as f64 * duration;
            app.timeline_view_start = view_start + dsec;
        }
    }

    // Controls-bar background across the lanes — a faint step above the panel
    // (rerun's bottom-bar treatment, K-084), parted from it by a hairline.
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(track_left, bar_top),
            egui::pos2(panel_right, panel.bottom()),
        ),
        0.0,
        theme.surface_2,
    );
    ui.painter().line_segment(
        [
            egui::pos2(track_left, bar_top),
            egui::pos2(panel_right, bar_top),
        ],
        egui::Stroke::new(1.0_f32, theme.hairline),
    );

    // Zoom controls, bottom-left of the lanes (with room for the lens toggle
    // that joins them in graph mode).
    let zr = egui::Rect::from_min_max(
        egui::pos2(track_left + 4.0, bar_top),
        egui::pos2((panel_right - 64.0).max(track_left + 8.0), panel.bottom()),
    );
    let mut zc = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(zr)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    zc.set_clip_rect(zr);
    if zc
        .small_button("−")
        .on_hover_text("Zoom out (Alt-wheel)")
        .clicked()
    {
        app.timeline_zoom = (app.timeline_zoom / 1.4).max(1.0);
    }
    if zc
        .small_button("+")
        .on_hover_text("Zoom in (Alt-wheel)")
        .clicked()
    {
        app.timeline_zoom = (app.timeline_zoom * 1.4).min(400.0);
    }
    if zc
        .small_button("Fit")
        .on_hover_text("Fit the whole comp")
        .clicked()
    {
        app.timeline_zoom = 1.0;
        app.timeline_view_start = 0.0;
    }
    if app.timeline_zoom > 1.001 {
        zc.label(
            egui::RichText::new(format!("{:.0}%", app.timeline_zoom * 100.0))
                .small()
                .color(theme.text_muted),
        );
    }

    // Guide-line mode for the lanes (owner request): beats, the time grid, or
    // nothing. Lives with the zoom cluster since both are about reading time.
    {
        use crate::app_state::TimelineGrid;
        let label = match app.timeline_grid {
            TimelineGrid::Beats => "Grid: beats",
            TimelineGrid::Time => "Grid: time",
            TimelineGrid::Off => "Grid: off",
        };
        zc.menu_button(egui::RichText::new(label).small(), |ui| {
            for (mode, name) in [
                (TimelineGrid::Beats, "Beats"),
                (TimelineGrid::Time, "Time (seconds and subdivisions)"),
                (TimelineGrid::Off, "Off"),
            ] {
                if ui.radio(app.timeline_grid == mode, name).clicked() {
                    app.timeline_grid = mode;
                    ui.close_menu();
                }
            }
        });
    }

    // The value/speed lens toggle (graph mode only): one lens shared by every
    // curve, so it lives here rather than in each plot's header. Its own group,
    // split from the zoom cluster by a hairline divider. The wording follows
    // the graphed channel: a retimed layer's Speed reads source timecode and
    // per cent, a transform property reads its value and rate of change.
    if app.timeline_graph_mode {
        zc.add_space(10.0);
        let x = zc.cursor().left();
        zc.painter().line_segment(
            [
                egui::pos2(x, bar_top + 5.0),
                egui::pos2(x, panel.bottom() - 5.0),
            ],
            egui::Stroke::new(1.0_f32, theme.hairline_strong),
        );
        zc.add_space(10.0);
        let retime = app.graph_retime;
        let (value_label, speed_label) = if retime {
            ("Time", "Velocity") // K-076: the Retime channel's lenses
        } else {
            ("Value", "Speed")
        };
        if zc
            .selectable_label(!app.graph_speed_view, value_label)
            .on_hover_text(if retime {
                "Value lens — the source frame showing at each point (HH:MM:SS:FF)"
            } else {
                "Value lens — the property's value over time"
            })
            .clicked()
        {
            app.graph_speed_view = false;
            app.graph_reset_fit(); // a manual range doesn't carry across lenses
        }
        if zc
            .selectable_label(app.graph_speed_view, speed_label)
            .on_hover_text(if retime {
                "Derivative lens — playback speed per cent (Vegas-style)"
            } else {
                "The rate-of-change view — drag a key to set its speed (K-070)"
            })
            .clicked()
        {
            app.graph_speed_view = true;
            app.graph_reset_fit();
        }

        // Fit toggle (K-079): the value graph keeps re-fitting the curve (and
        // its tangent handles) vertically while this is on — lit — and
        // scrolling or zooming vertically switches it off with a manual range.
        // Clicking it while lit freezes the view where it is; clicking it back
        // on drops the manual range and resumes fitting. Only the value lens
        // has a manual range.
        if !app.graph_speed_view {
            zc.add_space(10.0);
            if zc
                .selectable_label(app.graph_auto_fit, "Fit")
                .on_hover_text("Keep the curve and its handles fitted to the graph height. Click to freeze the view; scroll or zoom vertically to take over.")
                .clicked()
            {
                if app.graph_auto_fit {
                    // Freeze: last frame's fit becomes the manual range; the
                    // plot stamps the height it is framed at next pass.
                    if let Some(fit) = app.graph_last_fit {
                        app.graph_auto_fit = false;
                        app.graph_view_y = Some(fit);
                        app.graph_view_h = None;
                    }
                } else {
                    app.graph_reset_fit();
                }
            }
        }

        // Interpolation buttons for a graphed transform property (not the Retime
        // channel): convert the selected keys — or all of them — to straight or
        // eased. Linear is the default key; Bezier is AE's easy-ease (also F9),
        // after which the key's yellow tangent handles are draggable.
        if !retime && app.graph_prop.is_some() {
            zc.add_space(10.0);
            let x = zc.cursor().left();
            zc.painter().line_segment(
                [
                    egui::pos2(x, bar_top + 5.0),
                    egui::pos2(x, panel.bottom() - 5.0),
                ],
                egui::Stroke::new(1.0_f32, theme.hairline_strong),
            );
            zc.add_space(10.0);
            if zc
                .small_button("Linear")
                .on_hover_text("Straighten the selected keys (or all)")
                .clicked()
            {
                app.graph_set_interp = Some(lumit_core::anim::SideInterp::Linear);
            }
            if zc
                .small_button("Bezier")
                .on_hover_text("Easy-ease the selected keys (or all) — AE's F9")
                .clicked()
            {
                app.graph_set_interp = Some(lumit_core::anim::EASY_EASE);
            }
        }
    }

    // View toggle, bottom-right of the lanes.
    let gr = egui::Rect::from_min_max(
        egui::pos2(panel_right - 60.0, bar_top),
        egui::pos2(panel_right - 4.0, panel.bottom()),
    );
    graph_toggle(ui, theme, app, gr);
}

/// Footage preview: the frame fit to the surround, scrub bar, resolution picker.
#[cfg(feature = "media")]
/// The layer↔screen mapping the Viewer overlays share: the layer's evaluated
/// 2D transform at the playhead, then the view placement.
#[cfg(feature = "media")]
pub(crate) struct LayerMap {
    px: f64,
    py: f64,
    ax: f64,
    ay: f64,
    sx: f64,
    sy: f64,
    sin: f64,
    cos: f64,
    origin: egui::Pos2,
    view_scale: f32,
}

#[cfg(feature = "media")]
impl LayerMap {
    pub(crate) fn of(
        layer: &lumit_core::model::Layer,
        lt: f64,
        draw: egui::Rect,
        scale: f32,
    ) -> Self {
        let tr = &layer.transform;
        let rot = tr.rotation.value_at(lt).to_radians();
        let (sin, cos) = rot.sin_cos();
        Self {
            px: tr.position_x.value_at(lt),
            py: tr.position_y.value_at(lt),
            ax: tr.anchor_x.value_at(lt),
            ay: tr.anchor_y.value_at(lt),
            sx: (tr.scale_x.value_at(lt) / 100.0).max(1e-6),
            sy: (tr.scale_y.value_at(lt) / 100.0).max(1e-6),
            sin,
            cos,
            origin: draw.min,
            view_scale: scale,
        }
    }

    /// Layer space → screen.
    pub(crate) fn to_screen(&self, p: (f64, f64)) -> egui::Pos2 {
        let (dx, dy) = ((p.0 - self.ax) * self.sx, (p.1 - self.ay) * self.sy);
        let (rx, ry) = (dx * self.cos - dy * self.sin, dx * self.sin + dy * self.cos);
        self.origin + egui::vec2((self.px + rx) as f32, (self.py + ry) as f32) * self.view_scale
    }

    /// Screen → layer space (drag and pen positions come back through this).
    pub(crate) fn layer_of(&self, pos: egui::Pos2) -> (f64, f64) {
        let c = (pos - self.origin) / self.view_scale;
        let (dx, dy) = (f64::from(c.x) - self.px, f64::from(c.y) - self.py);
        let (rx, ry) = (
            dx * self.cos + dy * self.sin,
            -dx * self.sin + dy * self.cos,
        );
        (rx / self.sx + self.ax, ry / self.sy + self.ay)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod effect_drop_tests {
    use super::*;
    use lumit_core::model::LayerKind;

    // Regression for K-101: the two layer kinds an effect stack is dropped
    // onto from the Effects & Presets browser must keep accepting the drop.
    #[test]
    fn footage_and_adjustment_layers_accept_an_effect_drop() {
        assert!(accepts_effect_drop(&LayerKind::Footage {
            item: uuid::Uuid::nil(),
            retime: None,
        }));
        assert!(accepts_effect_drop(&LayerKind::Adjustment));
    }

    // Other layer kinds still gain effects through the existing "Add effect"
    // row (untouched by K-101); a drop on their Timeline row is a no-op.
    #[test]
    fn other_layer_kinds_do_not_accept_an_effect_drop() {
        assert!(!accepts_effect_drop(&LayerKind::Solid {
            def: uuid::Uuid::nil(),
        }));
        assert!(!accepts_effect_drop(&LayerKind::Precomp {
            comp: uuid::Uuid::nil(),
        }));
        assert!(!accepts_effect_drop(&LayerKind::Sequence {
            clips: Vec::new(),
        }));
    }
}
