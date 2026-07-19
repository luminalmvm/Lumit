//! The Timeline panel proper: the comp-tab strip, the ruler and lane
//! geometry, and the per-layer row loop with its columns and lanes.

use super::*;

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
    // Live effect-value drag (layer, effect index, param index, value), applied
    // to `app.fx_edit` after the loop for the live preview.
    let mut fx_edit: Option<(uuid::Uuid, usize, usize, f64)> = None;
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
    // Shift/horizontal = scroll through time. In the layers view a plain wheel
    // falls through to the one shared vertical ScrollArea (outline + lanes move
    // together); in the graph view the outline scrolls on its own (its scroll
    // area stops at the outline's right edge — see the ScrollArea below) and a
    // plain wheel over the curve pans it, not the layer list (UI-8, K-079).
    let lane_area = egui::Rect::from_min_max(
        egui::pos2(track_left, ui.max_rect().top()),
        egui::pos2(panel_right, ui.max_rect().bottom()),
    );
    let (scroll, mods, hover) =
        ui.input(|i| (i.raw_scroll_delta, i.modifiers, i.pointer.hover_pos()));
    let over_lane = hover.is_some_and(|p| lane_area.contains(p));
    let horizontal = scroll.x.abs() > 0.01;
    let vertical = scroll.y.abs() > 0.01;
    let cursor_x = hover.map(|p| p.x).unwrap_or(track_left);
    let consume = |ui: &mut egui::Ui| {
        ui.input_mut(|i| {
            i.raw_scroll_delta = egui::Vec2::ZERO;
            i.smooth_scroll_delta = egui::Vec2::ZERO;
        });
    };
    match timeline_wheel_route(
        app.timeline_graph_mode,
        over_lane,
        mods,
        horizontal,
        vertical,
    ) {
        TimelineWheel::ZoomTime => {
            // Alt-wheel: zoom the time axis around the cursor.
            let cursor_t = view_start + (cursor_x - track_left) as f64 / px_per_sec.max(1e-6);
            let factor = (scroll.y as f64 * 0.004).exp();
            app.timeline_zoom = (app.timeline_zoom * factor).clamp(1.0, 400.0);
            let (new_ppx, _) = lane_view(track_w, duration, app.timeline_zoom, view_start);
            app.timeline_view_start = cursor_t - (cursor_x - track_left) as f64 / new_ppx.max(1e-6);
            consume(ui);
        }
        TimelineWheel::PanTime => {
            // A horizontal wheel (Shift-wheel arrives as scroll.x on most
            // platforms) or Shift + a vertical wheel scrolls through time.
            let h = if horizontal { scroll.x } else { scroll.y };
            app.timeline_view_start = view_start - h as f64 / px_per_sec.max(1e-6);
            consume(ui);
        }
        // Graph: the curve reads `raw_scroll_delta` itself and its outline-only
        // scroll area never sees this wheel. Scroll: left to a ScrollArea (the
        // shared outline+lane scroll in the layers view, or the outline's own
        // scroll over the outline column). Either way, nothing to consume here.
        TimelineWheel::Graph | TimelineWheel::Scroll => {}
    }

    // The taller time bar's top row (TL4): current time/frame, a layer-search
    // box, and the view + motion-blur-master toggles (moved up from the bottom
    // bar, T22). Drawn above the ruler so the whole time bar reads taller.
    timeline_top_row(
        ui,
        theme,
        app,
        comp_id,
        comp.frame_rate.fps(),
        app.preview_frame,
    );

    let (ruler_rect, ruler_resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 20.0),
        egui::Sense::click_and_drag(),
    );
    ui.painter().rect_filled(ruler_rect, 0.0, theme.surface_2);
    // Column headers (Mack): small icons over the outline switch columns, on the
    // ruler's own level, so each column reads at a glance. Painted before the
    // lane clip narrows below, and clipped to the outline so they never bleed
    // over the ruler ticks.
    column_header_icons(ui, theme, panel_left, track_left, ruler_rect);
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
            app.pause_playback(); // scrubbing pauses — stop audio + transport, not just the frame advance
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
    // time axis so beats and transients line up. Optional (T25): right-clicking
    // the strip hides it, and right-clicking the ruler shows it again.
    #[cfg(feature = "media")]
    {
        let mut toggle_bar = false;
        if app.show_audio_bar {
            if let Some((wc, wf)) = &app.comp_waveform {
                if *wc == comp_id && !wf.is_empty() {
                    let (wave_rect, wave_resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 26.0),
                        egui::Sense::click(),
                    );
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
                    let n = wf.len().max(1) as f64;
                    let col = theme.text_muted.gamma_multiply(0.7);
                    // Map each peak through the SAME time axis as the ruler and
                    // the layers (`x_of`), not a full-width stretch (T25): the
                    // waveform then tracks zoom, scroll and a moved audio layer's
                    // transients instead of standing still. Peaks off-screen are
                    // skipped so a zoomed view never draws past the track.
                    for (i, (lo, hi)) in wf.iter().enumerate() {
                        let t = (i as f64 / n) * duration;
                        let x = x_of(t);
                        if x < track_left || x > track_left + track_w {
                            continue;
                        }
                        ui.painter().line_segment(
                            [egui::pos2(x, cy - hi * half), egui::pos2(x, cy - lo * half)],
                            egui::Stroke::new(1.0_f32, col),
                        );
                    }
                    wave_resp.context_menu(|ui| {
                        if ui.button("Hide audio waveform").clicked() {
                            toggle_bar = true;
                            ui.close_menu();
                        }
                    });
                }
            }
        } else {
            // Hidden: offer to bring it back from the ruler's context menu.
            ruler_resp.context_menu(|ui| {
                if ui.button("Show audio waveform").clicked() {
                    toggle_bar = true;
                    ui.close_menu();
                }
            });
        }
        if toggle_bar {
            app.show_audio_bar = !app.show_audio_bar;
        }
    }
    let rows_top = ui.cursor().top();

    // Graph mode (K-070) keeps everything around the lanes — the ruler, the
    // layer outline, the scrollbars and the bottom bar — and swaps only the
    // lane content for the curve editor (the AE shape). The rows below still
    // render their outline columns in both modes; only the drawing to the
    // right of `track_left` is suppressed, and the curve fills that area
    // after the ScrollArea.

    // Lanes scroll vertically when there are more layers than fit. In the layers
    // view the scroll area spans the whole panel, so one wheel moves the outline
    // and the lanes together (synced) and its scrollbar sits at the far right. In
    // the graph view the curve owns the lane area and pans on its own wheel, so
    // the width caps at the outline column (`max_width(name_w)`): the scrollbar
    // then lands at the outline's right edge, and — since a scroll area only
    // reacts to the wheel over its own rect — a wheel over the curve never
    // scrolls the layer list (UI-8). The list still scrolls on its own bar or a
    // wheel over the outline column. `INFINITY` is the builder's own default (no
    // cap), so the layers view spans the whole panel exactly as before.
    ui.set_clip_rect(saved_clip);
    let lane_max_w = if app.timeline_graph_mode {
        name_w
    } else {
        f32::INFINITY
    };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height((ui.available_height() - 38.0).max(48.0)) // leave the bottom bar + hscrollbar
        .max_width(lane_max_w)
        .id_salt(("timeline-lanes", comp_id))
        .show(ui, |ui| {
            // The scroll viewport: the whole panel in the layers view, the outline
            // column alone in the graph view (the capped width above). Lane content
            // clips to lane_area ∩ it (empty in the graph view, where the lane side
            // is suppressed); outline columns clip to their own x but this
            // viewport's y, so a half-scrolled row never bleeds above the ruler.
            let viewport = ui.clip_rect();
            ui.set_clip_rect(viewport.intersect(lane_area));
            // Lane keyframe glyphs, the linked-pair register and the property-row
            // draw order are rebuilt every frame: clear them before the rows
            // repopulate them (notes 2.1/2.6).
            app.lane_glyphs.clear();
            app.lane_linked.clear();
            app.prop_row_order.clear();
            // Lane keyframe marquee (notes 2.1/2.6c): a press-drag on empty
            // timeline space rubber-bands a selection box. Added BEFORE the rows
            // so the layer bars and keyframe glyphs (drawn after, topmost) win
            // the hit-test — a drag that begins on a bar or a key never opens the
            // marquee. Layers view only; the graph editor owns the lane otherwise.
            if !app.timeline_graph_mode {
                let bg = ui.interact(
                    ui.clip_rect(),
                    ui.id().with(("lane-marquee-bg", comp_id)),
                    egui::Sense::click_and_drag(),
                );
                if bg.clicked() {
                    app.lane_selection.clear();
                    // Reveal this comp in the Project panel (TL3): a click on
                    // empty timeline space deselects the keys and makes the comp
                    // the selected project item, so its header shows and it
                    // highlights in the tree.
                    app.selected_item = Some(comp_id);
                    app.selected_items.clear();
                }
                // Right-click empty space: the comp's settings, or reveal +
                // focus it in the Project panel (TL3). Deferred via flags so the
                // menu closure never double-borrows `app`.
                let mut open_settings = false;
                let mut reveal = false;
                bg.context_menu(|ui| {
                    if ui.button("Composition settings\u{2026}").clicked() {
                        open_settings = true;
                        ui.close_menu();
                    }
                    if ui.button("Reveal in project").clicked() {
                        reveal = true;
                        ui.close_menu();
                    }
                });
                if open_settings {
                    app.open_comp_settings(comp_id);
                }
                if reveal {
                    app.selected_item = Some(comp_id);
                    app.selected_items.clear();
                    app.focus_project_tab = true;
                }
                if bg.drag_started() {
                    if let Some(p) = bg.interact_pointer_pos() {
                        app.lane_marquee = Some((p, p));
                        // Shift or Ctrl makes the marquee a toggle (UI-5), so it
                        // can deselect covered keys just like the click gesture.
                        app.lane_marquee_add =
                            ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);
                    }
                } else if bg.dragged() {
                    if let Some(p) = bg.interact_pointer_pos() {
                        if let Some(m) = &mut app.lane_marquee {
                            m.1 = p;
                        }
                    }
                }
                if bg.drag_stopped() {
                    // Defer the hit-test until the rows below refill lane_glyphs.
                    app.lane_marquee_commit = app.lane_marquee.take();
                }
            }
            // Is an effect being dragged out of the Effects & Presets browser this
            // frame. Only then does each row raise a drop zone (see below), so the
            // zone never steals ordinary hover/clicks.
            let fx_drag =
                egui::DragAndDrop::has_payload_of_type::<EffectDragPayload>(ui.ctx());
            // Reorder-by-drag bookkeeping (Mack): each top-level row records its
            // centre y, so a drop can be resolved against every row after the loop;
            // `commit_reorder` carries the dragged layer and release y.
            let mut layer_row_centers: Vec<(uuid::Uuid, f32)> = Vec::new();
            let mut commit_reorder: Option<(uuid::Uuid, f32)> = None;
            // Layer-search filter + hide-switched-off filter (TL4): both are
            // view-only (the render is unaffected).
            let layer_search = app.timeline_layer_search.trim().to_lowercase();
            let hide_invisible = app.timeline_hide_invisible;
            for layer in &comp.layers {
                if !layer_search.is_empty()
                    && !layer.name.to_lowercase().contains(&layer_search)
                {
                    continue;
                }
                if hide_invisible && !layer.switches.visible {
                    continue;
                }
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
                layer_row_centers.push((layer.id, row_rect.center().y));
                if row_resp.clicked() {
                    app.selected_layer = Some(layer.id);
                }
                // Effect drop target (K-101, extended): while an effect is dragged
                // out of the Effects & Presets browser, the whole row — outline and
                // lane alike — accepts it, appended through the same SetLayerEffects
                // the "Add effect" menu commits (one undo step). `contains_pointer`
                // ignores occlusion, so the layer bar or a switch under the cursor
                // never blocks the drop, and the zone exists only mid-drag, so it
                // steals no ordinary input. Non-effect-stack layer kinds still gain
                // effects through their own row's "Add effect" menu. The release is
                // read through `dnd_release_of`, and so is every other zone's — the
                // panel-wide item zone above this ScrollArea used to take-and-discard
                // the effect payload on the release frame before any row could read
                // it, which is why dropping on a row silently did nothing.
                if fx_drag && accepts_effect_drop(&layer.kind) {
                    let full = egui::Rect::from_min_max(
                        row_rect.left_top(),
                        egui::pos2(panel_right, row_rect.bottom()),
                    );
                    let drop = {
                        let saved = ui.clip_rect();
                        ui.set_clip_rect(viewport);
                        let r = ui.interact(
                            full,
                            ui.id().with(("fx-drop", layer.id)),
                            egui::Sense::hover(),
                        );
                        ui.set_clip_rect(saved);
                        r
                    };
                    if let Some(payload) = dnd_release_of::<EffectDragPayload>(&drop) {
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
                    } else if drop.dnd_hover_payload::<EffectDragPayload>().is_some() {
                        // Hover highlight while the drag is over this row: a faint
                        // accent wash under an accent outline, across outline and
                        // lane alike, so the landing row is unmistakable.
                        let mut hp = ui.painter().clone();
                        hp.set_clip_rect(viewport);
                        hp.rect_filled(full, 2.0, theme.accent.gamma_multiply(0.08));
                        hp.rect_stroke(
                            full,
                            2.0,
                            egui::Stroke::new(1.0_f32, theme.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                // Deferred layer actions, set by the name's context menu / rename
                // (drawn with the outline below) and applied once it has drawn.
                let mut ctx_op: Option<lumit_core::Op> = None;
                let mut convert_layer = false;
                let mut trim_to_source = false;
                let mut start_rename = false;
                let mut duplicate_this = false;
                let mut delete_this = false;
                // Layer's natural pixel space, for the "Add mask" sizes (computed
                // outside the menu closure so it needn't borrow `app`).
                let (mask_w, mask_h) = mask_space(layer, app, comp);
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
                // Volume sits right beside the eye (Mack); the far-right slot it
                // vacated still carries a Precomp's collapse switch.
                let vol_r = slot(row_rect.left() + 38.0, row_rect.left() + 56.0);
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
                // Layer name (Mack): a non-selectable, click-and-drag button — a
                // click selects, a double-click renames inline, a drag reorders the
                // stack, a right-click opens the layer menu. While renaming, the
                // button is swapped for a single-line editor: Enter or focus-loss
                // commits a RenameLayer, Escape cancels. A Button (not a Label) is
                // used so dragging over the name never highlights its characters.
                let renaming = app
                    .renaming_layer
                    .as_ref()
                    .is_some_and(|(id, _)| *id == layer.id);
                {
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("layer-title", layer.id))
                            .max_rect(title_r)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    child.set_clip_rect(title_r.intersect(viewport));
                    if renaming {
                        let focus_id = ui.id().with(("rename-focus", layer.id));
                        let mut buf = app
                            .renaming_layer
                            .as_ref()
                            .map(|(_, s)| s.clone())
                            .unwrap_or_default();
                        let resp = child.add(
                            egui::TextEdit::singleline(&mut buf)
                                .desired_width(title_r.width())
                                .font(egui::TextStyle::Small),
                        );
                        if ui.data(|d| d.get_temp::<bool>(focus_id).unwrap_or(false)) {
                            resp.request_focus();
                            ui.data_mut(|d| d.remove::<bool>(focus_id));
                        }
                        if resp.lost_focus() {
                            let escape = child.input(|i| i.key_pressed(egui::Key::Escape));
                            if !escape {
                                let name = buf.trim();
                                if !name.is_empty() && name != layer.name {
                                    pending = Some(lumit_core::Op::RenameLayer {
                                        comp: comp_id,
                                        layer: layer.id,
                                        name: name.to_owned(),
                                    });
                                }
                            }
                            app.renaming_layer = None;
                        } else {
                            app.renaming_layer = Some((layer.id, buf));
                        }
                    } else {
                        let title_resp = child.add(
                            egui::Button::new(
                                egui::RichText::new(trim_title(&layer.name))
                                    .small()
                                    .color(theme.text_secondary),
                            )
                            .frame(false)
                            .truncate()
                            .sense(egui::Sense::click_and_drag()),
                        );
                        if title_resp.double_clicked() {
                            start_rename = true;
                        } else if title_resp.clicked() {
                            select_this = true;
                        }
                        if title_resp.dragged() {
                            if let Some(p) = title_resp.interact_pointer_pos() {
                                app.layer_reorder = Some((layer.id, p.y));
                            }
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        }
                        if title_resp.drag_stopped() {
                            let y = app
                                .layer_reorder
                                .filter(|(id, _)| *id == layer.id)
                                .map(|(_, y)| y)
                                .or_else(|| title_resp.interact_pointer_pos().map(|p| p.y));
                            if let Some(y) = y {
                                commit_reorder = Some((layer.id, y));
                            }
                            app.layer_reorder = None;
                        }
                        title_resp.context_menu(|ui| {
                            layer_context_menu(
                                ui,
                                layer,
                                comp_id,
                                (mask_w, mask_h),
                                &mut ctx_op,
                                &mut start_rename,
                                &mut duplicate_this,
                                &mut delete_this,
                                &mut convert_layer,
                                &mut trim_to_source,
                            );
                        });
                    }
                }
                place(ui, matte_r, &mut |ui| {
                    matte_control(ui, theme, comp, comp_id, layer, &mut pending)
                });
                place(ui, blend_r, &mut |ui| {
                    blend_control(ui, comp_id, layer, &mut pending)
                });
                place(ui, td_r, &mut |ui| {
                    three_d_control(ui, theme, comp_id, layer, &mut pending)
                });
                let is_precomp = matches!(layer.kind, lumit_core::model::LayerKind::Precomp { .. });
                if is_footage {
                    place(ui, vol_r, &mut |ui| {
                        mute_control(ui, theme, comp_id, layer, &mut pending)
                    });
                    place(ui, flow_r, &mut |ui| {
                        flow_control(ui, theme, comp_id, layer, &mut pending)
                    });
                } else if is_precomp {
                    // Precomp layers have no audio; their far-right slot carries
                    // the collapse switch (docs/06 §1.4) instead.
                    let clt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
                        - layer.start_offset.0.to_f64();
                    place(ui, mute_r, &mut |ui| {
                        collapse_control(ui, theme, &doc, comp, comp_id, layer, clt, &mut pending)
                    });
                }
                // Per-layer motion blur (K-120): the far-right slot for every
                // layer, except a Precomp — whose far-right slot holds its
                // collapse switch — where it takes the (unused) flow slot instead.
                let mb_slot = if is_precomp { flow_r } else { mute_r };
                place(ui, mb_slot, &mut |ui| {
                    motion_blur_control(ui, theme, comp_id, layer, &mut pending)
                });
                if select_this {
                    app.selected_layer = Some(layer.id);
                }
                if start_rename {
                    app.selected_layer = Some(layer.id);
                    app.renaming_layer = Some((layer.id, layer.name.clone()));
                    ui.data_mut(|d| {
                        d.insert_temp(ui.id().with(("rename-focus", layer.id)), true)
                    });
                }
                if let Some(op) = ctx_op {
                    app.selected_layer = Some(layer.id);
                    pending = Some(op);
                }
                if duplicate_this {
                    app.selected_layer = Some(layer.id);
                    app.duplicate_layer();
                }
                if delete_this {
                    app.selected_layer = Some(layer.id);
                    app.delete_selected_layer();
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
                // Lane content — the bar, its labels and its drag handles — only
                // draws in the layers view; in graph mode the lane area belongs
                // to the curve (drawn after the ScrollArea).
                if !app.timeline_graph_mode {
                    // A whole-layer move (dragging the bar body) slides the bar for preview;
                    // the snapped landing is what draws.
                    let move_dx = match app.move_edit {
                        Some((id, raw_in)) if id == layer.id => {
                            let thr = drag_secs(6.0, px_per_sec);
                            // A layer may sit before comp 0 (K-153): snap the raw
                            // in point sign-preserved, never clamped to 0.
                            let snapped = lumit_core::markers::snap_time(
                                rational_at_signed(raw_in),
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
                                // A layer may start before comp 0 (K-153), so the
                                // move is not clamped to 0; the comp window clips
                                // whatever falls outside [0, comp_end) at render.
                                app.move_edit = Some((layer.id, base + dx_secs));
                            }
                            if resp.drag_stopped() {
                                if let Some((id, raw_in)) = app.move_edit.take() {
                                    if id == layer.id {
                                        let thr = drag_secs(6.0, px_per_sec);
                                        let snapped = lumit_core::markers::snap_time(
                                            rational_at_signed(raw_in),
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
                    // Transform group: its own twirl, starting collapsed (Mack) so
                    // opening a layer's twirl doesn't also unfurl every transform
                    // property. Revealing it lists each animatable property as a
                    // timeline row — stopwatch/name/value in the left column, that
                    // property's keyframes on the track to the right; click a row to
                    // graph it (K-072).
                    let tf_id = ui.id().with(("transform-group", layer.id));
                    if group_header_row(ui, theme, "Transform", tf_id, false, viewport) {
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
                            comp,
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
                            selected_prop: app.selected_prop,
                            selected_props: app.selected_props.clone(),
                        };
                        let mut fx_nav_jump = None;
                        effects_rows(
                            ui,
                            app,
                            &fx_ctx,
                            &mut pending,
                            &mut fx_edit,
                            &mut fx_nav_jump,
                        );
                        if let Some(kt) = fx_nav_jump {
                            app.preview_frame =
                                ((kt + fx_ctx.off) * fx_ctx.fps).round().max(0.0) as usize;
                            #[cfg(feature = "media")]
                            app.refresh_preview();
                        }
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
                                comp,
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
                                selected_prop: app.selected_prop,
                                selected_props: app.selected_props.clone(),
                            };
                            let mut flow_nav_jump = None;
                            flow_group_rows(ui, &flow_ctx, &mut pending, &mut flow_nav_jump);
                            if let Some(kt) = flow_nav_jump {
                                app.preview_frame =
                                    ((kt + flow_ctx.off) * flow_ctx.fps).round().max(0.0) as usize;
                                #[cfg(feature = "media")]
                                app.refresh_preview();
                            }
                        }
                    }
                }
            }
            // Resolve a name-drag reorder (Mack). The target slot is where the
            // dropped layer's centre lands among the *other* rows (top = 0), which
            // is exactly the ReorderLayer index (the position after the layer is
            // lifted out). A landing that changes nothing commits nothing.
            if let Some((lid, y)) = commit_reorder {
                let others: Vec<f32> = layer_row_centers
                    .iter()
                    .filter(|(id, _)| *id != lid)
                    .map(|(_, cy)| *cy)
                    .collect();
                let target = others.iter().filter(|cy| **cy < y).count();
                let old = layer_row_centers.iter().position(|(id, _)| *id == lid);
                if old != Some(target) && pending.is_none() {
                    pending = Some(lumit_core::Op::ReorderLayer {
                        comp: comp_id,
                        layer: lid,
                        new_index: target,
                    });
                }
            }
            // While a name drag is live, draw an accent insertion line at the gap
            // the layer would drop into, across the outline column.
            if !app.timeline_graph_mode {
                if let Some((lid, y)) = app.layer_reorder {
                    let others: Vec<f32> = layer_row_centers
                        .iter()
                        .filter(|(id, _)| *id != lid)
                        .map(|(_, cy)| *cy)
                        .collect();
                    if !others.is_empty() {
                        let target = others.iter().filter(|cy| **cy < y).count();
                        let gap_y = if target == 0 {
                            others[0] - 10.0
                        } else if target >= others.len() {
                            others[others.len() - 1] + 10.0
                        } else {
                            (others[target - 1] + others[target]) * 0.5
                        };
                        let mut p = ui.painter().clone();
                        p.set_clip_rect(viewport);
                        p.line_segment(
                            [
                                egui::pos2(panel_left + 4.0, gap_y),
                                egui::pos2(track_left - 6.0, gap_y),
                            ],
                            egui::Stroke::new(2.0_f32, theme.accent),
                        );
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
    {
        // The separator lives LEFT of the lane area, but the clip in force here
        // was narrowed to the lanes (x ≥ track_left) for the grid overlays — under
        // it the hairline painted into the clipped-away margin (invisible) and the
        // grab zone hit-tested against an empty rect ∩ clip (undraggable): egui
        // hit-tests a widget against its rect intersected with the ui clip. Widen
        // back to the whole panel for the handle, then restore the lane clip for
        // the playhead below.
        ui.set_clip_rect(saved_clip);
        // The divider doubles as the column-resize grip — but only in the layers
        // view (UI-8). In the graph view the outline's own scrollbar sits in this
        // same right-edge strip, and egui would hand an overlapping drag to this
        // grip (the last-added widget), leaving the scrollbar thumb unusable. So
        // the grip is a layers-view control (the column width it sets is kept in
        // the graph view), and the graph view draws a plain division at
        // `track_left`, clear of the scrollbar gutter to its left.
        let (sep_x, active) = if app.timeline_graph_mode {
            (track_left, false)
        } else {
            (track_left - 4.0, true)
        };
        let mut hot = false;
        if active {
            let handle = egui::Rect::from_min_max(
                egui::pos2(sep_x - 4.0, ruler_rect.top()),
                egui::pos2(sep_x + 4.0, sep_bottom.max(ruler_rect.top() + 1.0)),
            );
            let hresp = ui.interact(
                handle,
                ui.id().with("name-col-resize"),
                egui::Sense::click_and_drag(),
            );
            hot = hresp.hovered() || hresp.dragged();
            if hot {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if hresp.drag_started() {
                app.timeline_divider_raw = Some(app.timeline_name_w);
            }
            if hresp.dragged() {
                // Clamp to the same bounds the layout applies, so the divider never
                // drags past where the outline can actually go. Overshoot tracking
                // (drag-catch-up note 1): accumulate the *raw* pointer travel and
                // clamp only the shown width, so once the drag is pinned at a limit
                // the divider doesn't start moving back until the cursor returns to
                // the divider's actual position — not the instant the mouse reverses.
                let max_w = (panel_right - panel_left - 120.0).clamp(96.0, 900.0);
                let raw =
                    app.timeline_divider_raw.unwrap_or(app.timeline_name_w) + hresp.drag_delta().x;
                app.timeline_divider_raw = Some(raw);
                app.timeline_name_w = raw.clamp(96.0, max_w);
            }
            if hresp.drag_stopped() {
                app.timeline_divider_raw = None;
            }
        }
        // Full-height division between the outline and the lanes: a strong
        // hairline from the ruler down to the bottom of the rows, accent while
        // hovered or dragged so the handle reads as interactive (layers view).
        ui.painter().line_segment(
            [
                egui::pos2(sep_x, ruler_rect.top()),
                egui::pos2(sep_x, sep_bottom),
            ],
            egui::Stroke::new(
                1.0_f32,
                if hot {
                    theme.accent
                } else {
                    theme.hairline_strong
                },
            ),
        );
        ui.set_clip_rect(saved_clip.intersect(lane_area));
    }
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
    // Lane keyframe drag release (note 2.1): slide every selected key by the
    // grabbed key's delta, as one Batch (a single undo step). Re-pin the
    // selection to the moved times so it stays selected where it landed.
    if let Some(delta) = app.lane_drag_commit.take() {
        let fps = comp.frame_rate.fps().max(1.0);
        if delta.abs() > 1e-9 && !app.lane_selection.is_empty() {
            if let Some(op) =
                build_lane_drag_op(comp, &app.lane_selection, &app.lane_linked, delta, fps)
            {
                for s in app.lane_selection.iter_mut() {
                    s.time = rational_at((s.time.to_f64() + delta).max(0.0));
                }
                follow_edit(app, &op);
                app.commit(op);
                #[cfg(feature = "media")]
                app.refresh_preview();
            }
        }
    }
    // Lane marquee release (notes 2.1/2.6): hit-test the box against the glyphs
    // the rows just drew, across every property row. A Shift/Ctrl drag toggles
    // each covered key (so it can deselect too, UI-5); a plain drag replaces.
    if let Some((a, b)) = app.lane_marquee_commit.take() {
        let band = egui::Rect::from_two_pos(a, b);
        let mut hits: Vec<crate::app_state::LaneKeySel> = Vec::new();
        for g in &app.lane_glyphs {
            if band.contains(g.pos) && !hits.contains(&g.sel) {
                hits.push(g.sel);
            }
        }
        if app.lane_marquee_add {
            for h in hits {
                if let Some(i) = app.lane_selection.iter().position(|s| *s == h) {
                    app.lane_selection.remove(i);
                } else {
                    app.lane_selection.push(h);
                }
            }
        } else {
            app.lane_selection = hits;
        }
    }
    // The in-flight marquee band: translucent accent fill, hairline outline —
    // the graph editor's look, on the lanes.
    if let Some((a, b)) = app.lane_marquee {
        let band = egui::Rect::from_two_pos(a, b).intersect(lane_area);
        ui.painter()
            .rect_filled(band, 0.0, theme.accent.gamma_multiply(0.12));
        ui.painter().rect_stroke(
            band,
            0.0,
            egui::Stroke::new(1.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
    }
    // Live preview while an effect value is dragged. Only WRITE when this panel
    // has an active drag — the Effect Controls panel draws the same effect rows
    // in the same frame, and an unconditional `= None` here would clobber its
    // drag (or vice versa). The shell clears fx_edit once at the top of the frame.
    if fx_edit.is_some() {
        app.fx_edit = fx_edit;
    }
    // A Shift-click on a property name ranges from the anchor over the rows drawn
    // between (note 2.6b), resolved here now the whole draw order is known. The
    // order now holds transform, Retime and effect rows alike, so a range can
    // span all three (UI-6).
    if let Some(target) = app.prop_range_target.take() {
        let (range, anchor_to_target) = prop_range(&app.prop_row_order, app.selected_prop, target);
        if anchor_to_target {
            app.selected_prop = Some(target);
        }
        app.selected_props = range;
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
        // A curve owns the lane in graph mode: no lane marquee/drag there.
        app.lane_marquee = None;
        app.lane_marquee_commit = None;
        app.lane_key_drag = None;
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
