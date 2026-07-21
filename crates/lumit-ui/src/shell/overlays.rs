//! `shell::overlays` — split out of the monolithic shell.rs (mechanical,
//! no logic changes). Shared names resolve through the parent module
//! via `use super::*` and the glob re-exports in `shell/mod.rs`.

use super::*;

/// Mask outlines and draggable vertices over the previewed comp — the seed
/// of the pen tool (07-UI-SPEC §Viewer tools). Outline follows the cursor
/// mid-drag; pixels update on release (one SetLayerMasks per drag, one undo).
#[cfg(feature = "media")]
pub(crate) fn mask_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    scale: f32,
) {
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let Some(layer_id) = app.selected_layer else {
        return;
    };
    let doc = app.store.snapshot();
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
        return;
    };
    if layer.masks.is_empty() || layer.switches.three_d {
        return; // 3D overlay geometry arrives with the object tools
    }
    let fps = comp.frame_rate.fps().max(1.0);
    let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
    let map = LayerMap::of(layer, lt, draw, scale);
    let to_screen = |p: (f64, f64)| map.to_screen(p);
    let from_screen = |pos: egui::Pos2| map.layer_of(pos);

    let stroke = egui::Stroke::new(1.0_f32, theme.accent);
    let mut committed: Option<Vec<lumit_core::mask::Mask>> = None;
    for (mi, mask) in layer.masks.iter().enumerate() {
        // The path being dragged previews with the moved vertex.
        let mut path = mask.path.clone();
        if let Some((dm, dv, pos)) = app.mask_drag {
            if dm == mi && dv < path.vertices.len() {
                path.vertices[dv].pos = pos;
            }
        }
        // Flatten each cubic span exactly as the rasteriser does.
        let n = path.vertices.len();
        if n >= 2 {
            let mut points: Vec<egui::Pos2> = Vec::with_capacity(n * 24 + 1);
            for i in 0..n {
                let a = &path.vertices[i];
                let b = &path.vertices[(i + 1) % n];
                for s in 0..24 {
                    let t = f64::from(s) / 24.0;
                    let u = 1.0 - t;
                    let p0 = a.pos;
                    let p1 = (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1);
                    let p2 = (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1);
                    let p3 = b.pos;
                    let x = u * u * u * p0.0
                        + 3.0 * u * u * t * p1.0
                        + 3.0 * u * t * t * p2.0
                        + t * t * t * p3.0;
                    let y = u * u * u * p0.1
                        + 3.0 * u * u * t * p1.1
                        + 3.0 * u * t * t * p2.1
                        + t * t * t * p3.1;
                    points.push(to_screen((x, y)));
                }
            }
            if let Some(first) = points.first().copied() {
                points.push(first);
            }
            ui.painter().add(egui::Shape::line(points, stroke));
        }
        // Vertex handles: 8px squares, draggable.
        for (vi, v) in path.vertices.iter().enumerate() {
            let centre = to_screen(v.pos);
            let handle = egui::Rect::from_center_size(centre, egui::vec2(8.0, 8.0));
            let resp = ui.interact(
                handle,
                ui.id().with(("mask-vtx", layer_id, mi, vi)),
                egui::Sense::click_and_drag(),
            );
            let active = app
                .mask_drag
                .is_some_and(|(dm, dv, _)| dm == mi && dv == vi);
            ui.painter().rect_filled(
                handle.shrink(if active || resp.hovered() { 0.0 } else { 2.0 }),
                1.0,
                theme.accent,
            );
            if resp.drag_started() || resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    app.mask_drag = Some((mi, vi, from_screen(pos)));
                }
            }
            if resp.drag_stopped() {
                if let Some((dm, dv, pos)) = app.mask_drag.take() {
                    if dm == mi && dv == vi {
                        let mut masks = layer.masks.clone();
                        if let Some(vtx) =
                            masks.get_mut(dm).and_then(|m| m.path.vertices.get_mut(dv))
                        {
                            vtx.pos = pos;
                            committed = Some(masks);
                        }
                    }
                }
            }
            // Right-click removes the vertex (a closed path keeps ≥ 3).
            if resp.secondary_clicked() && path.vertices.len() > 3 {
                let mut masks = layer.masks.clone();
                if let Some(m) = masks.get_mut(mi) {
                    if vi < m.path.vertices.len() {
                        m.path.vertices.remove(vi);
                        committed = Some(masks);
                    }
                }
            }
        }
    }
    if let Some(masks) = committed {
        app.commit(lumit_core::Op::SetLayerMasks {
            comp: comp_id,
            layer: layer_id,
            masks,
        });
        app.refresh_preview();
    }
}

/// Pen tool (slice 2): while armed, Viewer clicks place vertices of a new
/// mask on the selected layer; clicking the first vertex closes it into a
/// mask (one undo); Escape cancels.
/// Draw the selected 2D layer's origin (anchor point) as a crosshair, and let
/// it be dragged — AE's pan-behind: moving the origin keeps the layer visually
/// fixed (position compensates), committed as one undo step.
#[cfg(feature = "media")]
pub(crate) fn anchor_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    scale: f32,
) {
    use lumit_core::anim::Animation;
    use lumit_core::model::TransformProp;
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let doc = app.store.snapshot();
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = app
        .selected_layer
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
    else {
        app.origin_drag = None;
        return;
    };
    if layer.switches.three_d || matches!(layer.kind, lumit_core::model::LayerKind::Camera { .. }) {
        return; // 3D anchor gizmo arrives with the object tools
    }
    let fps = comp.frame_rate.fps().max(1.0);
    let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
    let map = LayerMap::of(layer, lt, draw, scale);
    let tr = &layer.transform;
    let anchor = (tr.anchor_x.value_at(lt), tr.anchor_y.value_at(lt));

    // The crosshair sits at the live anchor (or the dragged one).
    let shown_anchor = app.origin_drag.unwrap_or(anchor);
    let c = map.to_screen(shown_anchor);
    let handle = egui::Rect::from_center_size(c, egui::vec2(18.0, 18.0));
    let resp = ui.interact(
        handle,
        ui.id().with(("origin", layer.id)),
        egui::Sense::click_and_drag(),
    );
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    }
    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            app.origin_drag = Some(map.layer_of(pos));
        }
    }
    if resp.drag_stopped() {
        if let Some(new_anchor) = app.origin_drag.take() {
            let position = (tr.position_x.value_at(lt), tr.position_y.value_at(lt));
            let scale_pct = (tr.scale_x.value_at(lt), tr.scale_y.value_at(lt));
            let new_pos = crate::app_state::pan_behind_position(
                anchor,
                new_anchor,
                position,
                scale_pct,
                tr.rotation.value_at(lt),
            );
            // One op per touched property; if the property is animated, write a
            // key at the playhead, else move the static value.
            let mk = |prop: TransformProp, value: f64| {
                let slot = tr.get(prop);
                let animation = if slot.is_animated() {
                    Animation::Keyframed(upsert_key(slot, lt, value))
                } else {
                    Animation::Static(value)
                };
                lumit_core::Op::SetTransformProperty {
                    comp: comp_id,
                    layer: layer.id,
                    prop,
                    animation,
                }
            };
            let ops = vec![
                mk(TransformProp::AnchorX, new_anchor.0),
                mk(TransformProp::AnchorY, new_anchor.1),
                mk(TransformProp::PositionX, new_pos.0),
                mk(TransformProp::PositionY, new_pos.1),
            ];
            app.commit(lumit_core::Op::Batch { ops });
            app.refresh_preview();
        }
    }

    let stroke = egui::Stroke::new(1.5_f32, theme.accent);
    let r = 6.0;
    ui.painter().circle_stroke(c, r, stroke);
    ui.painter().line_segment(
        [c - egui::vec2(r + 4.0, 0.0), c + egui::vec2(r + 4.0, 0.0)],
        stroke,
    );
    ui.painter().line_segment(
        [c - egui::vec2(0.0, r + 4.0), c + egui::vec2(0.0, r + 4.0)],
        stroke,
    );
}

/// Shape tool: drag a rubber-band in the Viewer to create a rectangle,
/// ellipse or star mask (the current [`ShapeKind`]) on the selected layer.
#[cfg(feature = "media")]
pub(crate) fn shape_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    scale: f32,
    view: &egui::Response,
) {
    if app.tool != ToolMode::Shape || app.mask_drag.is_some() {
        return;
    }
    if view.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let doc = app.store.snapshot();
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = app
        .selected_layer
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
    else {
        return;
    };
    if layer.switches.three_d {
        return;
    }
    let fps = comp.frame_rate.fps().max(1.0);
    let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
    let map = LayerMap::of(layer, lt, draw, scale);

    if view.drag_started() {
        if let Some(pos) = view.interact_pointer_pos() {
            app.shape_drag = Some(map.layer_of(pos));
        }
    }
    if let Some(start) = app.shape_drag {
        let now = view
            .interact_pointer_pos()
            .map(|p| map.layer_of(p))
            .unwrap_or(start);
        // Preview outline in clay.
        let a = map.to_screen(start);
        let b = map.to_screen(now);
        ui.painter().rect_stroke(
            egui::Rect::from_two_pos(a, b),
            2.0,
            egui::Stroke::new(1.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
        if view.drag_stopped() {
            app.shape_drag = None;
            let (x0, x1) = (start.0.min(now.0), start.0.max(now.0));
            let (y0, y1) = (start.1.min(now.1), start.1.max(now.1));
            let (w, h) = (x1 - x0, y1 - y0);
            if w > 2.0 && h > 2.0 {
                let mask = match app.shape_kind {
                    ShapeKind::Rectangle => lumit_core::mask::Mask::rectangle(x0, y0, w, h),
                    ShapeKind::Ellipse => lumit_core::mask::Mask::ellipse(
                        x0 + w * 0.5,
                        y0 + h * 0.5,
                        w * 0.5,
                        h * 0.5,
                    ),
                    ShapeKind::Star => {
                        let outer = w.min(h) * 0.5;
                        lumit_core::mask::Mask::star(
                            x0 + w * 0.5,
                            y0 + h * 0.5,
                            outer,
                            outer * 0.42,
                            5,
                        )
                    }
                };
                let mut masks = layer.masks.clone();
                masks.push(mask);
                app.commit(lumit_core::Op::SetLayerMasks {
                    comp: comp_id,
                    layer: layer.id,
                    masks,
                });
                app.refresh_preview();
            }
        }
    }
}

#[cfg(feature = "media")]
pub(crate) fn pen_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    scale: f32,
    view: &egui::Response,
) {
    if app.tool != ToolMode::Pen {
        return;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.tool = ToolMode::Select;
        app.pen_path.clear();
        return;
    }
    if view.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let doc = app.store.snapshot();
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = app
        .selected_layer
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
    else {
        return;
    };
    if layer.switches.three_d {
        return;
    }
    let fps = comp.frame_rate.fps().max(1.0);
    let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
    let map = LayerMap::of(layer, lt, draw, scale);

    // The in-progress path: open polyline plus handles, first vertex ringed.
    let stroke = egui::Stroke::new(1.0_f32, theme.accent);
    let screen_pts: Vec<egui::Pos2> = app.pen_path.iter().map(|v| map.to_screen(v.pos)).collect();
    if screen_pts.len() >= 2 {
        ui.painter()
            .add(egui::Shape::line(screen_pts.clone(), stroke));
    }
    for (i, pt) in screen_pts.iter().enumerate() {
        let r = egui::Rect::from_center_size(*pt, egui::vec2(8.0, 8.0));
        if i == 0 && screen_pts.len() >= 3 {
            ui.painter().rect_stroke(
                r,
                1.0,
                egui::Stroke::new(1.5_f32, theme.accent),
                egui::StrokeKind::Inside,
            );
        } else {
            ui.painter().rect_filled(r.shrink(2.0), 1.0, theme.accent);
        }
    }

    if view.clicked() {
        if let Some(pos) = view.interact_pointer_pos() {
            let close_hit = screen_pts
                .first()
                .is_some_and(|f| (*f - pos).length() <= 8.0)
                && app.pen_path.len() >= 3;
            if close_hit {
                let mut masks = layer.masks.clone();
                masks.push(lumit_core::mask::Mask {
                    id: uuid::Uuid::now_v7(),
                    name: format!("Path {}", masks.len() + 1),
                    path: lumit_core::mask::BezierPath {
                        vertices: std::mem::take(&mut app.pen_path),
                        closed: true,
                    },
                    inverted: false,
                    opacity: 100.0,
                    extra: serde_json::Map::new(),
                });
                app.tool = ToolMode::Select;
                app.commit(lumit_core::Op::SetLayerMasks {
                    comp: comp_id,
                    layer: layer.id,
                    masks,
                });
                app.refresh_preview();
            } else {
                app.pen_path.push(lumit_core::mask::Vertex {
                    pos: map.layer_of(pos),
                    tan_in: (0.0, 0.0),
                    tan_out: (0.0, 0.0),
                });
            }
        }
    }
}

#[cfg(feature = "media")]
pub(crate) fn viewer_footage(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    tex: Option<(egui::TextureId, egui::Vec2)>,
    rect: egui::Rect,
) {
    use crate::app_state::media::MediaStatus;
    let frames = if let Some(comp_id) = app.preview_comp {
        app.store
            .snapshot()
            .comp(comp_id)
            .map(|c| app.comp_frame_count(c))
            .unwrap_or(0)
    } else if let Some(id) = app.preview_item {
        match app.media.map.get(&id) {
            Some(MediaStatus::Ready { frames, .. }) => *frames,
            _ => 0,
        }
    } else {
        return;
    };

    let bar_h = 30.0;
    let image_area = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - bar_h));

    if let Some((id, size)) = tex {
        if size.x > 0.0 && size.y > 0.0 {
            // Natural size drives layout: dropping decode resolution keeps the
            // image the same size on screen, just softer (07-UI-SPEC §Viewer).
            let natural = if let Some(comp_id) = app.preview_comp {
                app.store
                    .snapshot()
                    .comp(comp_id)
                    .map(|c| egui::vec2(c.width as f32, c.height as f32))
                    .unwrap_or(size)
            } else if let Some(item) = app.preview_item {
                match app.media.map.get(&item) {
                    Some(MediaStatus::Ready { probe, .. }) => probe
                        .video
                        .as_ref()
                        .map(|v| egui::vec2(v.width as f32, v.height as f32))
                        .unwrap_or(size),
                    _ => size,
                }
            } else {
                size
            };

            // View controls: scroll zooms, drag pans (the hand — object tools
            // arrive later), double-click resets to fit. View-only: never part
            // of any render.
            let view = ui.interact(
                image_area,
                ui.id().with("viewer-view"),
                egui::Sense::click_and_drag(),
            );
            if view.hovered() {
                // While the eyedropper is armed, Shift+scroll grows its sample
                // region instead of zooming the view (handled in its overlay).
                let shift = ui.ctx().input(|i| i.modifiers.shift);
                let region_scroll = app.eyedropper.is_some() && shift;
                let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
                if !region_scroll && scroll.abs() > 0.1 {
                    let factor = (scroll * 0.003).exp();
                    app.view_zoom = (app.view_zoom * factor).clamp(0.05, 32.0);
                    if app.preview_auto_res {
                        app.refresh_preview();
                    }
                }
            }
            // Drag pans in Select/Hand; Shape and Pen intercept it below. Held
            // off while the eyedropper is armed so a sampling click never pans.
            if view.dragged()
                && app.eyedropper.is_none()
                && matches!(app.tool, ToolMode::Select | ToolMode::Hand)
            {
                app.view_pan += view.drag_delta();
            }
            if matches!(app.tool, ToolMode::Hand) && view.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if view.double_clicked() {
                app.view_zoom = 1.0;
                app.view_pan = egui::Vec2::ZERO;
                if app.preview_auto_res {
                    app.refresh_preview();
                }
            }

            let fit = (image_area.width() / natural.x)
                .min(image_area.height() / natural.y)
                .min(1.0);
            let scale = fit * app.view_zoom;
            app.last_display_scale = scale;
            let draw =
                egui::Rect::from_center_size(image_area.center() + app.view_pan, natural * scale);
            // Clip the picture to the image area (owner T11 retest,
            // Screenshot_148): a zoomed or panned frame was painted unclipped,
            // so it bled over the panel's edges and rounded corners.
            ui.painter().with_clip_rect(image_area).image(
                id,
                draw,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            #[cfg(feature = "media")]
            mask_overlay(ui, theme, app, draw, scale);
            #[cfg(feature = "media")]
            pen_overlay(ui, theme, app, draw, scale, &view);
            #[cfg(feature = "media")]
            shape_overlay(ui, theme, app, draw, scale, &view);
            #[cfg(feature = "media")]
            anchor_overlay(ui, theme, app, draw, scale);
            // The eyedropper's magnifier + sample-on-click, when armed.
            #[cfg(feature = "media")]
            eyedropper::viewer_overlay(ui, theme, app, draw, image_area, &view);
        }
    } else {
        ui.painter().text(
            image_area.center(),
            egui::Align2::CENTER_CENTER,
            if frames == 0 {
                "probing…"
            } else {
                "decoding…"
            },
            egui::FontId::monospace(10.0),
            theme.text_disabled,
        );
    }

    // Viewer bar: resolution picker · scrub · frame readout (07-UI-SPEC §2).
    // Its bottom corners take the theme's card radius (owner T11 retest,
    // Screenshot_148 bottom-left): rounded under Round, square under Sharp.
    let bar = egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - bar_h), rect.max);
    let bar_round = egui::CornerRadius {
        sw: theme.tokens.card_radius,
        se: theme.tokens.card_radius,
        ..egui::CornerRadius::ZERO
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
        egui::Frame::new()
            .fill(theme.surface_1)
            .stroke(egui::Stroke::new(1.0_f32, theme.hairline))
            .corner_radius(bar_round)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Transport: play/pause the preview (also the space bar).
                    let playing = app.is_playing();
                    if icon_button(
                        ui,
                        theme,
                        if playing { Icon::Pause } else { Icon::Play },
                        playing,
                    )
                    .on_hover_text(if playing {
                        "Pause (space)"
                    } else {
                        "Play (space)"
                    })
                    .clicked()
                    {
                        app.toggle_play();
                    }
                    let labels = ["Full", "Half", "Third", "Quarter"];
                    let current = if app.preview_realtime {
                        // Realtime: the adaptive tier is what's actually decoding.
                        labels[(app.realtime_tier() as usize - 1).min(3)]
                    } else if app.preview_auto_res {
                        "Auto"
                    } else {
                        labels[(app.preview_divisor as usize - 1).min(3)]
                    };
                    bare_dropdown(ui, current, |ui| {
                        if ui
                            .selectable_label(app.preview_auto_res, "Auto")
                            .on_hover_text("Decode at the displayed size, capped at 100%")
                            .clicked()
                        {
                            app.preview_auto_res = true;
                            app.refresh_preview();
                            ui.close_menu();
                        }
                        for (i, label) in labels.iter().enumerate() {
                            let div = i as u32 + 1;
                            let selected = !app.preview_auto_res && app.preview_divisor == div;
                            if ui.selectable_label(selected, *label).clicked() {
                                app.preview_auto_res = false;
                                app.preview_divisor = div;
                                app.refresh_preview();
                                ui.close_menu();
                            }
                        }
                    });
                    // Cached vs Realtime preview mode (K-030, docs/06 §6.5): in
                    // Realtime, playback resolution adapts to load (dropping under
                    // strain, recovering slowly) and overrides the picker above;
                    // Cached decodes at the chosen resolution.
                    if ui
                        .selectable_label(app.preview_realtime, "Realtime")
                        .on_hover_text(
                            "Adapt resolution to playback load — drops under strain, \
                             recovers slowly (overrides the resolution picker)",
                        )
                        .clicked()
                    {
                        app.preview_realtime = !app.preview_realtime;
                        app.refresh_preview();
                    }
                    ui.label(
                        egui::RichText::new(format!("{:.0}%", app.last_display_scale * 100.0))
                            .monospace()
                            .small()
                            .color(theme.text_muted),
                    );
                    if frames > 1 {
                        let mut frame = app.preview_frame;
                        let slider = egui::Slider::new(&mut frame, 0..=frames - 1)
                            .show_value(false)
                            .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.4 });
                        let response =
                            ui.add_sized(egui::vec2(ui.available_width() - 96.0, 18.0), slider);
                        if response.changed() {
                            // Scrubbing the transport pauses playback (audio +
                            // transport state), so it never fights the frame clock.
                            app.pause_playback();
                            app.preview_frame = frame;
                            // Dragging the scrub slider decodes a draft; releasing
                            // it reloads at the specified resolution.
                            app.preview_draft = response.dragged();
                            app.refresh_preview();
                        }
                        if response.drag_stopped() {
                            app.preview_draft = false;
                            app.refresh_preview();
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} / {}",
                                app.preview_frame,
                                frames.saturating_sub(1)
                            ))
                            .monospace()
                            .small()
                            .color(theme.text_muted),
                        );
                    });
                });
            });
    });
}
