//! `shell::graph` — split out of the monolithic shell.rs (mechanical,
//! no logic changes). Shared names resolve through the parent module
//! via `use super::*` and the glob re-exports in `shell/mod.rs`.

use super::*;

/// The lane-area rectangle the curve editor fills in graph mode: the lanes'
/// width, from just under the ruler to just above the bottom bar (the same
/// 38 px strip the lane ScrollArea reserves for the scrollbar and the bar).
pub(crate) fn graph_lane_rect(
    track_left: f32,
    track_w: f32,
    rows_top: f32,
    panel_bottom: f32,
) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(track_left, rows_top),
        egui::pos2(
            track_left + track_w,
            (panel_bottom - 38.0).max(rows_top + 24.0),
        ),
    )
}

/// Select-on-edit: point the graph at whatever a timeline or graph edit just
/// touched. Committing a transform-property or Retime op selects that layer
/// and graphs that channel, so the curve follows the key you just added or
/// moved — a stopwatch click, a value scrub, or a key drag all land you on
/// the right graph. Ops that touch neither kind of channel pass through
/// untouched; a Batch follows its first property op (linked scale edits lead
/// with Scale x, matching a click on the row's name).
pub(crate) fn follow_edit(app: &mut AppState, op: &lumit_core::Op) {
    match op {
        lumit_core::Op::SetTransformProperty { layer, prop, .. } => {
            app.selected_layer = Some(*layer);
            app.graph_prop = Some(*prop);
            app.graph_retime = false;
        }
        lumit_core::Op::SetLayerRetime { layer, .. } => {
            app.selected_layer = Some(*layer);
            app.graph_retime = true;
        }
        lumit_core::Op::Batch { ops } => {
            if let Some(first) = ops.first() {
                follow_edit(app, first);
            }
        }
        _ => {}
    }
}

/// The graph editor drawn into the timeline's lane area (07-UI-SPEC; K-070):
/// the selected layer's picked channel as a live curve — the Retime speed
/// channel for retimed footage (K-075), else a transform property. Keys drag
/// (one op per release), double-click adds, right-click removes. The outline,
/// ruler, scrollbars and bottom bar around it are the timeline's own; what to
/// graph is picked in the outline's twirled-open property rows.
pub(crate) fn graph_lane_plot(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp: &lumit_core::model::Composition,
    // The shared timeline time axis (07-UI-SPEC §4): pixels per second and the
    // scrolled left-edge time, so the graph pans and zooms in step with the
    // lanes (K-079). Same values the lane bars use.
    px_per_sec: f64,
    view_start: f64,
    plot_rect: egui::Rect,
) {
    use lumit_core::model::TransformProp;

    // The curve for the selected layer / property.
    let Some(layer_id) = app.selected_layer else {
        app.graph_marquee = None;
        app.graph_selection = None;
        hint_in_rect(ui, theme, plot_rect, "Select a layer to edit its curves.");
        return;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
        app.graph_marquee = None;
        app.graph_selection = None;
        hint_in_rect(ui, theme, plot_rect, "The selected layer is gone.");
        return;
    };
    // Retime channel (K-075): a retimed footage layer's curve. The Time (value)
    // lens is now the ordinary graph editor on the source-position channel
    // (K-078); only the Velocity (speed) lens keeps the bespoke retime editor
    // with its ramp presets.
    if app.graph_retime {
        if let lumit_core::model::LayerKind::Footage {
            retime: Some(rt), ..
        } = &layer.kind
        {
            if app.graph_speed_view {
                // Source frame rate for the timecode: the probed footage fps
                // when media is present, else the comp's rate as a fallback.
                let src_fps = layer_source_fps(app, layer, comp.frame_rate.fps());
                // The marquee lives on curve channels only; a keyframe selection
                // lives only while its channel is the one graphed.
                app.graph_marquee = None;
                app.graph_selection = None;
                graph_plot_retime(ui, theme, app, comp, layer, rt, src_fps, plot_rect);
            } else {
                // `current` is unused for the Time channel; pass a placeholder.
                graph_plot(
                    ui,
                    theme,
                    app,
                    comp,
                    layer,
                    TransformProp::PositionX,
                    true,
                    px_per_sec,
                    view_start,
                    plot_rect,
                );
            }
            return;
        }
        app.graph_retime = false; // selected layer isn't retimed footage
    }
    // Any property is graphable (not only animated ones): a still property draws
    // a flat line you can double-click to add the first keyframe to.
    let current = app.graph_prop.unwrap_or(TransformProp::PositionX);
    app.graph_prop = Some(current);
    graph_plot(
        ui, theme, app, comp, layer, current, false, px_per_sec, view_start, plot_rect,
    );
}

/// The Velocity (speed) lens of the Retime channel (K-075; the Time lens is
/// the ordinary graph editor, K-078): the curve plots speed per cent. Plain
/// keyframe stores drag their keys (2b); eased stores drag their boundary
/// joins (§9.2, square handles) with every ease preserved — the retime
/// rebuilds from the edit and downstream boundaries recompute (K-070). Map
/// segments are edited through the Time lens's own handles.
#[allow(clippy::too_many_arguments)]
pub(crate) fn graph_plot_retime(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    retime: &lumit_core::retime::Retime,
    src_fps: f64,
    rect: egui::Rect,
) {
    // Header: the Vegas default-lens setting and (in the speed lens) the
    // ramp-preset shelf that eases the segment under the playhead. The
    // Source/Speed lens toggle lives in the timeline's bottom bar.
    let mut preset_ease: Option<lumit_core::retime::Ease> = None;
    // "Convert to rate" (docs/04-RETIMING.md §5.2): fit the Map segment under the
    // playhead to a constant-ease Rate, surfacing any fit drift as a notice.
    let mut convert_to_rate = false;
    let header = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + 22.0));
    {
        use lumit_core::retime::Ease;
        let mut h = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        h.set_clip_rect(header);
        h.checkbox(&mut app.vegas_default_lens, "Vegas")
            .on_hover_text("Open the Speed channel to the speed-% lens by default (K-075)");
        // Ramp presets ease the speed segment under the playhead (§9.2).
        if app.graph_speed_view {
            h.separator();
            h.label("Ramp");
            for (label, ease) in [
                ("Lin", Ease::Linear),
                ("Slow", Ease::Slow),
                ("Fast", Ease::Fast),
                ("Smth", Ease::Smooth),
                ("Shrp", Ease::Sharp),
            ] {
                if h.small_button(label)
                    .on_hover_text("Ease the speed ramp under the playhead")
                    .clicked()
                {
                    preset_ease = Some(ease);
                }
            }
            h.separator();
            if h
                .small_button("→Rate")
                .on_hover_text("Convert the mapped segment under the playhead to a constant-ease rate (docs/04 §5.2)")
                .clicked()
            {
                convert_to_rate = true;
            }
        }
    }
    let rect = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 22.0), rect.max);
    ui.painter().rect_filled(rect, 0.0, theme.surface_0);

    let duration = comp.duration.0.to_f64().max(1e-6);
    let x_of = |t: f64| rect.left() + ((t / duration) as f32) * rect.width();

    let speed_view = app.graph_speed_view;

    // Speed keyframes in % (K-075, 2b): draggable when the retime is a plain
    // Linear-Rate keyframe store. Eased stores get boundary handles instead
    // (below); Map segments are the Time lens's business.
    let dur = layer.out_point.0;
    let kfs: Vec<(f64, f64)> = retime
        .speed_keyframes()
        .map(|ks| {
            ks.iter()
                .map(|(t, s)| (t.to_f64(), s.to_f64() * 100.0))
                .collect()
        })
        .unwrap_or_default();
    let pct_to_speed = |pct: f64| {
        lumit_core::Rational::from_f64_on_grid(pct / 100.0, 1000)
            .unwrap_or(lumit_core::Rational::ONE)
    };
    // Eased stores expose no plain speed keyframes, but every boundary still
    // has a draggable speed — the join of the eased ramps (§9.2). Handles are
    // (boundary index, local time, speed %), skipping Map-adjacent joins.
    let boundary_handles: Vec<(usize, f64, f64)> = if kfs.is_empty() {
        use lumit_core::retime::RetimeSegment;
        retime
            .boundaries
            .iter()
            .enumerate()
            .filter_map(|(j, b)| {
                let incoming_rate =
                    j == 0 || matches!(retime.segments.get(j - 1), Some(RetimeSegment::Rate(_)));
                let v = if j < retime.segments.len() {
                    match &retime.segments[j] {
                        RetimeSegment::Rate(seg) if incoming_rate => Some(seg.v0.to_f64()),
                        _ => None,
                    }
                } else {
                    match retime.segments.last() {
                        Some(RetimeSegment::Rate(seg)) => Some(seg.v1.to_f64()),
                        _ => None,
                    }
                };
                v.map(|v| (j, b.t.to_f64(), v * 100.0))
            })
            .collect()
    } else {
        Vec::new()
    };

    // While dragging a handle, a provisional retime drives the live curve.
    let provisional = app.graph_retime_edit.and_then(|(idx, pct)| {
        if kfs.is_empty() {
            retime.with_boundary_speed(idx, pct_to_speed(pct))
        } else {
            let &(t, _) = kfs.get(idx)?;
            speed_with_key(&Some(retime.clone()), dur, t, pct_to_speed(pct))
        }
    });
    let sampled: &lumit_core::retime::Retime = provisional.as_ref().unwrap_or(retime);

    // Sample the active lens across the pane width.
    let samples = (rect.width() as usize / 2).max(16);
    let values: Vec<(f64, f64)> = (0..=samples)
        .map(|i| {
            let t = duration * i as f64 / samples as f64;
            let v = if speed_view {
                sampled.speed_at(t) * 100.0
            } else {
                sampled.evaluate(t)
            };
            (t, v)
        })
        .collect();
    let (mut lo, mut hi) = values.iter().fold((f64::MAX, f64::MIN), |(l, h), (_, v)| {
        (l.min(*v), h.max(*v))
    });
    if speed_view {
        lo = lo.min(0.0); // always frame 0% and 100%
        hi = hi.max(100.0);
        if let Some((_, p)) = app.graph_retime_edit {
            lo = lo.min(p); // keep the dragged handle in frame
            hi = hi.max(p);
        }
    }
    let pad = ((hi - lo).abs().max(1.0)) * 0.12;
    let (lo, hi) = (lo - pad, hi + pad);
    let y_of = |v: f64| rect.bottom() - (((v - lo) / (hi - lo)) as f32) * rect.height();

    // Y-axis scale: labelled gridlines in the active lens's units.
    if speed_view {
        // The speed lens marks its 0% and 100% references.
        for val in [0.0_f64, 100.0] {
            let y = y_of(val);
            ui.painter().line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(0.5_f32, theme.hairline),
            );
        }
        graph_y_axis(ui, theme, rect, lo, hi, |v| format!("{v:.0}%"));
    } else {
        // The value lens reads as source-frame timecode.
        graph_y_axis(ui, theme, rect, lo, hi, |v| {
            fmt_timecode_frames(v.max(0.0), src_fps)
        });
    }

    // The curve.
    let points: Vec<egui::Pos2> = values
        .iter()
        .map(|(t, v)| egui::pos2(x_of(*t), y_of(*v)))
        .collect();
    let colour = if speed_view {
        theme.curve[1]
    } else {
        theme.curve[0]
    };
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5_f32, colour),
    ));

    // Draggable speed-keyframe handles (K-075, 2b) — the speed lens only.
    let mut pending: Option<lumit_core::Op> = None;

    // Apply a ramp preset: ease the Rate segment under the playhead (§9.2).
    // Works on any Rate segment, including one already eased; a no-op over a Map
    // segment or when the playhead is outside the retime.
    if let Some(ease) = preset_ease {
        let lt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
            - layer.start_offset.0.to_f64();
        if let Some(new_rt) = retime.with_segment_ease(rational_at(lt.max(0.0)), ease) {
            pending = Some(lumit_core::Op::SetLayerRetime {
                comp: comp.id,
                layer: layer.id,
                retime: Some(new_rt),
            });
        }
    }

    // Convert the mapped segment under the playhead to a rate (§5.2). The fit is
    // exact in source advance; a non-zero drift means the ease shape can't follow
    // the map perfectly, which we report rather than draw a badge for (yet).
    if convert_to_rate {
        let lt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
            - layer.start_offset.0.to_f64();
        match retime.with_segment_as_rate(rational_at(lt.max(0.0))) {
            Some((new_rt, drift)) => {
                pending = Some(lumit_core::Op::SetLayerRetime {
                    comp: comp.id,
                    layer: layer.id,
                    retime: Some(new_rt),
                });
                app.notice = Some(if drift.abs() > 5e-4 {
                    format!("Converted to rate — fitted, {:.0} ms drift", drift * 1000.0)
                } else {
                    "Converted to rate".into()
                });
            }
            None => {
                app.notice = Some(
                    "Can't convert here — already a rate, or the map can't fit one ease".into(),
                );
            }
        }
    }

    if speed_view {
        for (idx, &(t, pct)) in kfs.iter().enumerate() {
            let shown_pct = match app.graph_retime_edit {
                Some((i, p)) if i == idx => p,
                _ => pct,
            };
            let pos = egui::pos2(x_of(t), y_of(shown_pct));
            let resp = ui.interact(
                egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0)),
                ui.id().with(("rtkey", layer.id, idx)),
                egui::Sense::click_and_drag(),
            );
            let active = app.graph_retime_edit.is_some_and(|(i, _)| i == idx);
            let colour = if resp.hovered() || active {
                theme.accent
            } else {
                theme.curve[1]
            };
            ui.painter().circle_filled(pos, 4.0, colour);
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let frac = ((rect.bottom() - p.y) / rect.height()) as f64;
                    app.graph_retime_edit = Some((idx, lo + frac * (hi - lo)));
                }
            }
            if resp.drag_stopped() {
                if let Some((i, p)) = app.graph_retime_edit.take() {
                    if i == idx {
                        if let Some(new_rt) =
                            speed_with_key(&Some(retime.clone()), dur, t, pct_to_speed(p))
                        {
                            pending = Some(lumit_core::Op::SetLayerRetime {
                                comp: comp.id,
                                layer: layer.id,
                                retime: Some(new_rt),
                            });
                        }
                    }
                }
            }
        }
        // Eased stores: the boundary joins drag the same way (§9.2). The
        // square glyph tells them apart from plain keyframes; the eases
        // themselves are untouched by the drag.
        for &(j, t, pct) in &boundary_handles {
            let shown_pct = match app.graph_retime_edit {
                Some((i, p)) if i == j => p,
                _ => pct,
            };
            let pos = egui::pos2(x_of(t), y_of(shown_pct));
            let resp = ui.interact(
                egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0)),
                ui.id().with(("rtbound", layer.id, j)),
                egui::Sense::click_and_drag(),
            );
            let active = app.graph_retime_edit.is_some_and(|(i, _)| i == j);
            let colour = if resp.hovered() || active {
                theme.accent
            } else {
                theme.curve[1]
            };
            ui.painter().rect_filled(
                egui::Rect::from_center_size(pos, egui::vec2(7.0, 7.0)),
                1.0,
                colour,
            );
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let frac = ((rect.bottom() - p.y) / rect.height()) as f64;
                    app.graph_retime_edit = Some((j, lo + frac * (hi - lo)));
                }
            }
            if resp.drag_stopped() {
                if let Some((i, p)) = app.graph_retime_edit.take() {
                    if i == j {
                        if let Some(new_rt) = retime.with_boundary_speed(j, pct_to_speed(p)) {
                            pending = Some(lumit_core::Op::SetLayerRetime {
                                comp: comp.id,
                                layer: layer.id,
                                retime: Some(new_rt),
                            });
                        }
                    }
                }
            }
        }
    }

    // Playhead + a header readout (source timecode and speed %).
    if app.preview_comp == Some(comp.id) {
        let lt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
            - layer.start_offset.0.to_f64();
        let x = x_of(lt.clamp(0.0, duration));
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, theme.accent),
        );
        let readout = format!(
            "{}   {:.1}%",
            fmt_timecode_frames(sampled.evaluate(lt), src_fps),
            sampled.speed_at(lt) * 100.0
        );
        ui.painter().text(
            egui::pos2(rect.right() - 6.0, rect.top() + 4.0),
            egui::Align2::RIGHT_TOP,
            readout,
            egui::FontId::monospace(11.0),
            theme.text_secondary,
        );
    }

    if let Some(op) = pending {
        follow_edit(app, &op); // the graph follows the key you just touched
        app.commit(op);
    }
}

/// Centre a muted hint inside `rect` (empty state of a sub-pane).
pub(crate) fn hint_in_rect(ui: &egui::Ui, theme: &Theme, rect: egui::Rect, msg: &str) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        msg,
        egui::FontId::proportional(12.0),
        theme.text_muted,
    );
}

/// Y-axis scale for a graph pane: a few horizontal gridlines with their values
/// labelled down the left inside edge, in small muted monospace so the curve
/// stays the focus. `fmt` turns a gridline's value into its label — the units
/// live there (%, °, timecode, per second). Drawn before the curve, which
/// paints over the gridlines.
pub(crate) fn graph_y_axis(
    ui: &egui::Ui,
    theme: &Theme,
    rect: egui::Rect,
    lo: f64,
    hi: f64,
    fmt: impl Fn(f64) -> String,
) {
    const LINES: u32 = 4; // evenly spaced, clear of the pane's edges
    for i in 1..=LINES {
        let frac = i as f32 / (LINES + 1) as f32;
        let y = rect.bottom() - frac * rect.height();
        let v = lo + f64::from(frac) * (hi - lo);
        ui.painter().line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(0.5_f32, theme.hairline),
        );
        ui.painter().text(
            egui::pos2(rect.left() + 4.0, y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            fmt(v),
            egui::FontId::monospace(9.0),
            theme.text_muted,
        );
    }
}

/// A y-axis value formatted to suit the axis span: whole numbers once the
/// range is wide, more decimals as it narrows.
pub(crate) fn fmt_axis_value(v: f64, span: f64) -> String {
    if span.abs() >= 20.0 {
        format!("{v:.0}")
    } else if span.abs() >= 2.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

/// The unit a transform property's y-axis labels carry ("" for the pixel
/// properties, which read cleaner bare).
pub(crate) fn prop_unit(prop: lumit_core::model::TransformProp) -> &'static str {
    use lumit_core::model::TransformProp as P;
    match prop {
        P::ScaleX | P::ScaleY | P::Opacity => "%",
        P::Rotation | P::RotationX | P::RotationY => "°",
        _ => "",
    }
}

/// The glyph a keyframe draws with, coding its interpolation at a glance:
/// a square holds, a diamond is linear, a circle is a bezier (eased) key.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum KeyShape {
    Square,
    Diamond,
    Circle,
}

pub(crate) fn key_shape(k: &lumit_core::anim::Keyframe) -> KeyShape {
    use lumit_core::anim::SideInterp;
    if matches!(k.interp_in, SideInterp::Hold) || matches!(k.interp_out, SideInterp::Hold) {
        KeyShape::Square
    } else if matches!(k.interp_in, SideInterp::Bezier { .. })
        || matches!(k.interp_out, SideInterp::Bezier { .. })
    {
        KeyShape::Circle
    } else {
        KeyShape::Diamond
    }
}

/// A keyframe side's influence (bezier handle reach), defaulting to the AE
/// easy-ease third for Linear/Hold sides that carry none.
pub(crate) fn side_influence(side: lumit_core::anim::SideInterp) -> f64 {
    match side {
        lumit_core::anim::SideInterp::Bezier { influence, .. } => influence,
        _ => 1.0 / 3.0,
    }
}

/// A keyframe side's bezier slope (value-units/second), or None if that side is
/// Linear/Hold and carries no single slope.
pub(crate) fn side_speed(side: lumit_core::anim::SideInterp) -> Option<f64> {
    match side {
        lumit_core::anim::SideInterp::Bezier { speed, .. } => Some(speed),
        _ => None,
    }
}

/// Write a tangent-handle drag back onto a keyframe: the dragged side takes the
/// new `speed`/`influence`; when `mirror` is set the other side goes collinear
/// (same slope, its own reach — `partner_inf` overrides that reach when the
/// caller has length-preserving maths). Whether to mirror is the caller's call
/// via [`tangent_mirrors`] — this is the AE handle: unified keys mirror,
/// Alt-drag breaks one end loose, Alt-drag on a broken key re-unifies it.
pub(crate) fn apply_tangent(
    k: &mut lumit_core::anim::Keyframe,
    is_out: bool,
    speed: f64,
    influence: f64,
    mirror: bool,
    partner_inf: Option<f64>,
) {
    use lumit_core::anim::SideInterp::Bezier;
    if is_out {
        let in_reach = partner_inf.unwrap_or_else(|| side_influence(k.interp_in));
        k.interp_out = Bezier { speed, influence };
        if mirror {
            k.interp_in = Bezier {
                speed,
                influence: in_reach,
            };
        }
    } else {
        let out_reach = partner_inf.unwrap_or_else(|| side_influence(k.interp_out));
        k.interp_in = Bezier { speed, influence };
        if mirror {
            k.interp_out = Bezier {
                speed,
                influence: out_reach,
            };
        }
    }
}

/// Whether an in-flight tangent drag mirrors across the key. The drag keeps the
/// unification the key *started* with, and Alt — held at any moment during the
/// drag — toggles it: break a smooth key, or re-unify a broken one. The toggle
/// latches, so releasing Alt mid-drag doesn't snap the handles back together
/// (Mack): once a drag has broken (or re-joined) a key, it stays that way until
/// the next Alt-drag says otherwise.
pub(crate) fn tangent_mirrors(unified_at_start: bool, alt_seen: bool) -> bool {
    unified_at_start != alt_seen
}

/// A vertical wheel over the value graph, turned into the new y-range (K-079).
/// A plain wheel (`ctrl` false) pans: the span is unchanged, both ends shift by
/// `dy` scaled to the range (wheel up moves the view up). Ctrl-wheel zooms about
/// `cursor_v`, which stays put while the span grows or shrinks (wheel up = zoom
/// in). `height` is the plot height in pixels.
pub(crate) fn graph_v_pan_zoom(
    range: (f64, f64),
    dy: f64,
    ctrl: bool,
    cursor_v: f64,
    height: f64,
) -> (f64, f64) {
    let (lo, hi) = range;
    let span = (hi - lo).max(1e-9);
    if ctrl {
        let factor = (-dy * 0.0015).exp(); // wheel up (dy > 0) shrinks the span
        (
            cursor_v - (cursor_v - lo) * factor,
            cursor_v + (hi - cursor_v) * factor,
        )
    } else {
        let dv = dy / height.max(1.0) * span;
        (lo + dv, hi + dv)
    }
}

/// The value extremes the graph's auto-fit must cover: every keyframe's value
/// plus, for each bezier side that has a neighbour, that side's tangent-handle
/// endpoint. A handle of slope `speed` reaching `influence · seg` seconds
/// towards its neighbour ends at value `v ± speed · reach`, and a steep handle
/// can poke well past the curve itself — the fit keeps it on screen (Mack).
/// Reads ALL keys, not just the selection, so selecting a key never jumps the
/// view. Returns `(f64::MAX, f64::MIN)` for an empty slice, like the plain
/// min/max fold it extends.
pub(crate) fn fit_values_with_handles(keys: &[lumit_core::anim::Keyframe]) -> (f64, f64) {
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for (idx, k) in keys.iter().enumerate() {
        lo = lo.min(k.value);
        hi = hi.max(k.value);
        let kt = k.time.to_f64();
        for is_out in [false, true] {
            let side = if is_out { k.interp_out } else { k.interp_in };
            // Only a bezier side grows a handle, and only towards a neighbour.
            let Some(speed) = side_speed(side) else {
                continue;
            };
            let seg = if is_out {
                match keys.get(idx + 1) {
                    Some(next) => next.time.to_f64() - kt,
                    None => continue,
                }
            } else if idx > 0 {
                kt - keys[idx - 1].time.to_f64()
            } else {
                continue;
            };
            if seg <= 1e-9 {
                continue;
            }
            let reach = side_influence(side) * seg;
            let end = if is_out {
                k.value + speed * reach
            } else {
                k.value - speed * reach
            };
            lo = lo.min(end);
            hi = hi.max(end);
        }
    }
    (lo, hi)
}

/// Rescale a manual y-range for a new plot height, keeping the centre value
/// and the value scale (units per pixel): a taller graph reveals *more* range
/// about the same centre, a shorter one reveals less — the curve never
/// stretches. Degenerate heights leave the range untouched.
pub(crate) fn rescale_range_for_height(range: (f64, f64), old_h: f32, new_h: f32) -> (f64, f64) {
    if old_h <= 0.0 || new_h <= 0.0 {
        return range;
    }
    let (lo, hi) = range;
    let centre = (lo + hi) * 0.5;
    let half = (hi - lo) * 0.5 * (new_h as f64 / old_h as f64);
    (centre - half, centre + half)
}

/// The influence a unified partner handle needs so its on-screen *length* stays
/// fixed while the dragged side's slope changes to `new_speed` — only the angle
/// rotates, the length holds (Mack). `sx`/`sy` are screen px per unit time /
/// value; `seg` is the partner side's segment length in seconds. A degenerate
/// segment leaves the influence untouched.
pub(crate) fn partner_influence(
    partner: lumit_core::anim::SideInterp,
    seg: f64,
    new_speed: f64,
    sx: f64,
    sy: f64,
) -> f64 {
    if seg <= 1e-9 {
        return side_influence(partner);
    }
    let old_inf = side_influence(partner);
    let old_speed = side_speed(partner).unwrap_or(0.0);
    let screen_len = |sp: f64| (sx * sx + sp * sp * sy * sy).sqrt().max(1e-9);
    let target = old_inf * seg * screen_len(old_speed);
    // The floor is tiny so length holds even for a near-vertical drag — a bigger
    // floor let the partner visibly lengthen as the slope grew (Mack).
    (target / (seg * screen_len(new_speed))).clamp(1e-4, 1.0)
}

/// Indices of the plotted keyframe points that fall inside a marquee band.
pub(crate) fn keys_in_band(points: &[egui::Pos2], band: egui::Rect) -> Vec<usize> {
    points
        .iter()
        .enumerate()
        .filter(|(_, p)| band.contains(**p))
        .map(|(i, _)| i)
        .collect()
}

/// Add `delta` to the value of every selected keyframe (the relative
/// multi-drag). Out-of-range indices are ignored, never a panic.
pub(crate) fn nudge_selected_values(
    keys: &mut [lumit_core::anim::Keyframe],
    selection: &[usize],
    delta: f64,
) {
    for &i in selection {
        if let Some(k) = keys.get_mut(i) {
            k.value += delta;
        }
    }
}

/// Set every selected keyframe to exactly `value` (the typed absolute set).
/// Returns whether anything changed; out-of-range indices are ignored.
pub(crate) fn set_selected_values(
    keys: &mut [lumit_core::anim::Keyframe],
    selection: &[usize],
    value: f64,
) -> bool {
    let mut changed = false;
    for &i in selection {
        if let Some(k) = keys.get_mut(i) {
            if k.value != value {
                k.value = value;
                changed = true;
            }
        }
    }
    changed
}

/// The validated marquee multi-selection (two or more keys) for exactly this
/// layer + property, or `None`. The outline value field's typed commit
/// applies to these indices; anything stale reads as no selection.
pub(crate) fn graph_multi_selection(
    app: &AppState,
    layer: uuid::Uuid,
    prop: lumit_core::model::TransformProp,
    keys: &[lumit_core::anim::Keyframe],
) -> Option<Vec<usize>> {
    let s = app.graph_selection.as_ref()?;
    if s.layer != layer || s.prop != prop {
        return None;
    }
    s.indices_for(keys).filter(|sel| sel.len() >= 2)
}

/// The local time a dragged Retime Time keyframe commits to (K-078). The Time
/// channel's first key is the domain-start boundary — the clip's own clock start
/// — which docs/04-RETIMING §3 pins at local time 0: a drag may change its
/// source value but never its time. Pinning it here keeps the rebuilt store's
/// first boundary at 0 so [`lumit_core::retime::Retime::from_source_keyframes`]
/// accepts the edited list; without the pin, dragging the first key off 0 makes
/// the rebuild return None and the whole edit is silently dropped (the reported
/// "Retime Time keyframes won't drag" defect). Any interior or trailing boundary
/// keeps its (snapped) dragged time, so those drag freely — like a transform
/// property's keys.
pub(crate) fn retime_drag_time(is_retime: bool, idx: usize, dragged_time: f64) -> f64 {
    if is_retime && idx == 0 {
        0.0
    } else {
        dragged_time
    }
}

/// Draw one keyframed property's value/speed curve inside `rect`, with a
/// compact Ease/Linear header and draggable keys (the Value/Speed lens toggle
/// lives in the timeline's bottom bar). In the speed lens each key's tangent
/// is draggable (K-070); the derivative curve updates live and the release
/// writes bezier speeds back to the keyframes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn graph_plot(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    current: lumit_core::model::TransformProp,
    // When set, this is the footage layer's Retime *Time* channel (K-078): the
    // curve is source position over local time, read from and committed back to
    // the layer's Retime rather than a transform property. Otherwise `current`
    // names the transform property being graphed.
    is_retime: bool,
    // The shared timeline time axis (K-079): pixels per second and the scrolled
    // left-edge time, so the graph's x maps exactly like the lane bars.
    px_per_sec: f64,
    view_start: f64,
    rect: egui::Rect,
) {
    use lumit_core::anim::{Animation, Keyframe, SideInterp};
    let layer_id = layer.id;

    // Interpolation change for this pass: the bottom-bar Linear/Bezier buttons
    // (which set graph_set_interp) or F9 (easy-ease), applied to the selection —
    // or every key when nothing is selected. The value/speed lens toggle and the
    // Linear/Bezier buttons live in the timeline's bottom bar, not a plot header.
    let mut set_sides: Option<SideInterp> = app.graph_set_interp.take();
    if ui.input(|i| i.key_pressed(egui::Key::F9)) {
        set_sides = Some(lumit_core::anim::EASY_EASE);
    }

    // The Time channel's keys are synthesised from the Retime store (owned, so
    // they outlive the borrow); a transform property borrows its slot directly.
    let retime_keys: Option<Vec<Keyframe>> = if is_retime {
        match &layer.kind {
            lumit_core::model::LayerKind::Footage {
                retime: Some(rt), ..
            } => Some(rt.source_keyframes()),
            _ => None,
        }
    } else {
        None
    };
    // A still (Static) property has no keys yet: graph a flat line at its value
    // that you can double-click to add the first keyframe to (Mack). The Time
    // channel always has ≥ 2 boundary keys, so its static value is just the
    // first key (only used if a curve were somehow empty).
    let empty_keys: Vec<Keyframe> = Vec::new();
    let (keys, static_val): (&Vec<Keyframe>, f64) = match &retime_keys {
        Some(ks) => (ks, ks.first().map_or(0.0, |k| k.value)),
        None => {
            let slot = layer.transform.get(current);
            let sv = slot.value_at(0.0);
            match &slot.animation {
                Animation::Keyframed(k) => (k, sv),
                _ => (&empty_keys, sv),
            }
        }
    };

    // The marquee selection for this channel. A selection made on any other
    // layer/property, or one whose time pins no longer line up with the keys
    // (something edited them underneath), clears here rather than risk ever
    // touching the wrong keyframes.
    let selection: Vec<usize> = match &app.graph_selection {
        Some(s)
            if s.layer == layer_id && s.retime == is_retime && (is_retime || s.prop == current) =>
        {
            match s.indices_for(keys) {
                Some(sel) => sel,
                None => {
                    app.graph_selection = None;
                    Vec::new()
                }
            }
        }
        Some(_) => {
            app.graph_selection = None;
            Vec::new()
        }
        None => Vec::new(),
    };
    // Two or more selected keys make drags relative multi-edits; a single
    // selected key keeps today's free drag exactly.
    let multi = selection.len() >= 2;

    // ---- plot geometry: x = layer time over the comp span, y = value ----
    ui.painter().rect_filled(rect, 0.0, theme.surface_0);
    let duration = comp.duration.0.to_f64().max(1e-6);
    // Everything the graph draws — curve, keys, handles, playhead — stays inside
    // its rect: a steep bezier must not paint over the ruler, outline or bottom
    // bar (Mack). The clip also gates hit-tests, so an off-plot key isn't
    // grabbable. Restored before the commit at the end.
    let saved_clip = ui.clip_rect();
    ui.set_clip_rect(saved_clip.intersect(rect));
    // The partner side's segment length in seconds, for keyframe `idx` when its
    // `is_out` side is dragged (partner is the opposite side's neighbouring gap).
    let partner_seg = |idx: usize, is_out: bool| -> f64 {
        if is_out {
            if idx > 0 {
                keys[idx].time.to_f64() - keys[idx - 1].time.to_f64()
            } else {
                0.0
            }
        } else if idx + 1 < keys.len() {
            keys[idx + 1].time.to_f64() - keys[idx].time.to_f64()
        } else {
            0.0
        }
    };

    // Provisional keys during a drag (visual only until release) — computed up
    // here so the y-range can fit the *drawn* curve, not just the keyframes.
    let multi_delta: Option<f64> = match app.graph_edit {
        Some((i, _, v)) if multi && selection.contains(&i) => keys.get(i).map(|k| v - k.value),
        _ => None,
    };
    let mut shown: Vec<Keyframe> = keys.clone();
    if let Some(delta) = multi_delta {
        nudge_selected_values(&mut shown, &selection, delta);
    } else if let Some((idx, kt, kv)) = app.graph_edit {
        if let Some(k) = shown.get_mut(idx) {
            k.time = rational_at(kt);
            k.value = kv;
        }
        shown.sort_by_key(|k| k.time);
    }
    if let Some((idx, sp)) = app.graph_speed_edit {
        if let Some(k) = shown.get_mut(idx) {
            let side = SideInterp::Bezier {
                speed: sp,
                influence: side_influence(k.interp_out),
            };
            k.interp_in = side;
            k.interp_out = side;
        }
    }
    // (An in-flight tangent-handle drag is applied to `shown` further down,
    // *after* the y-range is fixed — the axis must hold still during the drag.)

    // Key values and every bezier side's tangent-handle endpoint, for ALL keys
    // (fit_values_with_handles): the fit keeps the whole editable picture on
    // screen — a steep handle mustn't poke past the plot — and reading every
    // key, not just the selection, means selecting one never jumps the view.
    let (mut vmin, mut vmax) = fit_values_with_handles(&shown);
    if keys.is_empty() {
        vmin = static_val;
        vmax = static_val;
    }
    if let Some((_, _, v)) = app.graph_edit {
        vmin = vmin.min(v);
        vmax = vmax.max(v);
    }
    // Fit the drawn value curve too: a bezier can overshoot past its keyframes,
    // so sample it and grow the range to keep the whole curve on screen (Mack).
    {
        let n = (rect.width() as usize / 2).max(16);
        for i in 0..=n {
            let t = duration * i as f64 / n as f64;
            let v = lumit_core::anim::evaluate(&shown, t).unwrap_or(static_val);
            vmin = vmin.min(v);
            vmax = vmax.max(v);
        }
    }
    let pad = ((vmax - vmin).abs().max(1.0)) * 0.15;
    let auto_fit = (vmin - pad, vmax + pad);
    // Remember the auto-fit so a first vertical scroll can seed a manual range
    // from what's on screen; then honour any manual range the user scrolled or
    // zoomed to (K-079). The Fit toggle clears `graph_view_y` back to None and
    // resumes continuous fitting.
    if !is_retime || !app.graph_speed_view {
        app.graph_last_fit = Some(auto_fit);
    }
    // A manual range answers a panel resize by keeping its value scale (units
    // per pixel): it grows or shrinks about its centre by the height ratio, so
    // a taller graph reveals more curve instead of stretching it. Auto-fit
    // needs none of this — it simply re-fits to whatever height it is given.
    if !app.graph_speed_view {
        match (app.graph_view_y, app.graph_view_h) {
            (Some(range), Some(old_h)) if (old_h - rect.height()).abs() > 0.01 => {
                app.graph_view_y = Some(rescale_range_for_height(range, old_h, rect.height()));
                app.graph_view_h = Some(rect.height());
            }
            // A freshly frozen range (Fit toggled off) hasn't seen the plot
            // yet: stamp the height it is being framed at.
            (Some(_), None) => app.graph_view_h = Some(rect.height()),
            (None, _) => app.graph_view_h = None,
            _ => {}
        }
    }
    let (vmin, vmax) = app.graph_view_y.unwrap_or(auto_fit);
    // x follows the shared timeline axis (K-079): the same pixels-per-second and
    // scrolled left edge as the lane bars, so panning/zooming the timeline moves
    // the curve in step. Keys outside the view clip to the lane area.
    let x_of = |t: f64| rect.left() + ((t - view_start) * px_per_sec) as f32;
    let y_of = |v: f64| rect.bottom() - (((v - vmin) / (vmax - vmin)) as f32) * rect.height();
    let t_of = |x: f32| {
        (view_start + (x - rect.left()) as f64 / px_per_sec.max(1e-6)).clamp(0.0, duration)
    };
    let v_of = |y: f32| {
        vmin + ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0) as f64 * (vmax - vmin)
    };

    // The *live* screen scales — px per second and px per value unit of what is
    // actually on screen — for the unified partner-handle length maths. Length
    // must hold in pixels, exactly as drawn (Mack): the old whole-duration /
    // key-range scales made the partner's length look rotation-dependent. The
    // y-range above deliberately excluded the in-flight tangent drag, so both
    // scales are stable for the whole drag.
    let sx_px = px_per_sec;
    let sy_px = rect.height() as f64 / (vmax - vmin).max(1e-9);
    // Apply the in-flight tangent drag to the drawn curve now that the axis is
    // pinned: the dragged side (and, when mirroring, its partner) re-tangents so
    // the curve bends live under the cursor.
    if let Some((idx, is_out, sp, inf)) = app.graph_tangent_edit {
        let seg_p = partner_seg(idx, is_out);
        if let Some(k) = shown.get_mut(idx) {
            let mirror = app
                .graph_tangent_mode
                .is_some_and(|(u, a)| tangent_mirrors(u, a));
            let partner = if is_out { k.interp_in } else { k.interp_out };
            let pinf = partner_influence(partner, seg_p, sp, sx_px, sy_px);
            apply_tangent(k, is_out, sp, inf, mirror, Some(pinf));
        }
    }

    // Vertical scroll / zoom of the value graph (K-079): the outer wheel handler
    // freed `raw_scroll_delta` for us (the outline list scrolls on its own).
    // A plain wheel pans the value range; Ctrl-wheel zooms it around the cursor.
    // Either switches auto-fit off and takes over — the bottom-bar Fit toggle
    // resumes it.
    if !app.graph_speed_view {
        let (dy, ptr, ctrl) = ui.input(|i| {
            (
                i.raw_scroll_delta.y,
                i.pointer.hover_pos(),
                i.modifiers.ctrl || i.modifiers.command,
            )
        });
        if dy.abs() > 0.01 && ptr.is_some_and(|p| rect.contains(p)) {
            let cursor_v = v_of(ptr.map_or(rect.center().y, |p| p.y));
            app.graph_auto_fit = false;
            app.graph_view_y = Some(graph_v_pan_zoom(
                (vmin, vmax),
                dy as f64,
                ctrl,
                cursor_v,
                rect.height() as f64,
            ));
            app.graph_view_h = Some(rect.height());
        }
    }

    // The plot background: a press-and-drag on empty space rubber-bands a
    // marquee; a plain click clears the selection; a double-click adds a key.
    // The small key handles win the hit-test over this full-rect widget, so
    // a drag starting on a keyframe never opens a marquee.
    let mut pending: Option<Vec<Keyframe>> = None;
    let bg = ui.interact(
        rect,
        ui.id().with(("graph-bg", layer_id)),
        egui::Sense::click_and_drag(),
    );
    // (The marquee runs in both lenses; its handling sits below, after the
    // speed-lens y-mapping exists, so the band can hit-test speed points too.)
    if bg.clicked() {
        app.graph_selection = None;
    }
    if bg.double_clicked() {
        if let Some(p) = bg.interact_pointer_pos() {
            let mut new_keys = keys.clone();
            new_keys.push(Keyframe {
                time: rational_at(t_of(p.x)),
                value: v_of(p.y),
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            });
            new_keys.sort_by_key(|k| k.time);
            app.graph_selection = None; // indices shift under an insert
            pending = Some(new_keys);
        }
    }

    // (`shown` — the provisional keys with any in-flight drag applied — and its
    // multi_delta are computed above. Key drags fold into the y-range; a tangent
    // drag deliberately does not, so the axis holds still until release.)

    // Curve polyline: value, or its exact derivative in the speed lens (K-080).
    let samples = (rect.width() as usize / 2).max(16);
    let sample_at = |t: f64| -> f64 {
        if app.graph_speed_view {
            // The exact derivative of the value bezier, so the speed curve is
            // precisely the slope of what the value lens draws (K-080) — the
            // bezier shaping carries across, no finite-difference smearing.
            lumit_core::anim::evaluate_speed(&shown, t).unwrap_or(0.0)
        } else {
            lumit_core::anim::evaluate(&shown, t).unwrap_or(static_val)
        }
    };
    // Sample across the *visible* time window (K-079), so a zoomed-in view keeps
    // full curve resolution instead of stretching a whole-duration polyline.
    let vis_lo = view_start.clamp(0.0, duration);
    let vis_hi = (view_start + rect.width() as f64 / px_per_sec.max(1e-6)).clamp(0.0, duration);
    let vis_span = (vis_hi - vis_lo).max(1e-6);
    let values: Vec<(f64, f64)> = (0..=samples)
        .map(|i| {
            let t = vis_lo + vis_span * i as f64 / samples as f64;
            (t, sample_at(t))
        })
        .collect();
    // The speed lens scales to its own sampled range; `speed_of` inverts the
    // mapping so a dragged handle reads back as value-units/second.
    let (s_lo, s_hi) = {
        let (mut lo, mut hi) = values.iter().fold((f64::MAX, f64::MIN), |(l, h), (_, v)| {
            (l.min(*v), h.max(*v))
        });
        if let Some((_, sp)) = app.graph_speed_edit {
            lo = lo.min(sp);
            hi = hi.max(sp);
        }
        let pad = ((hi - lo).abs().max(1.0)) * 0.15;
        (lo - pad, hi + pad)
    };
    let speed_y =
        move |v: f64| rect.bottom() - (((v - s_lo) / (s_hi - s_lo)) as f32) * rect.height();
    let speed_of = move |y: f32| {
        s_lo + ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0) as f64 * (s_hi - s_lo)
    };

    // Marquee (rubber-band) selection, both lenses (Mack): press-and-drag on
    // empty plot; on release, select the keys whose plotted point — the value
    // point here, the speed point in the derivative lens — falls in the band.
    if bg.drag_started() {
        if let Some(p) = bg.interact_pointer_pos() {
            app.graph_marquee = Some((p, p));
        }
    } else if bg.dragged() {
        if let Some(p) = bg.interact_pointer_pos() {
            if let Some(band) = &mut app.graph_marquee {
                band.1 = p;
            }
        }
    }
    if bg.drag_stopped() {
        if let Some((a, b)) = app.graph_marquee.take() {
            let band = egui::Rect::from_two_pos(a, b);
            let points: Vec<egui::Pos2> = keys
                .iter()
                .map(|k| {
                    let x = x_of(k.time.to_f64());
                    if app.graph_speed_view {
                        // The speed lens plots each key at its resting speed.
                        let rest = match (k.interp_out, k.interp_in) {
                            (SideInterp::Bezier { speed, .. }, _) => speed,
                            (_, SideInterp::Bezier { speed, .. }) => speed,
                            _ => sample_at(k.time.to_f64()),
                        };
                        egui::pos2(x, speed_y(rest))
                    } else {
                        egui::pos2(x, y_of(k.value))
                    }
                })
                .collect();
            let hit = keys_in_band(&points, band);
            app.graph_selection = (!hit.is_empty()).then(|| crate::app_state::GraphSelection {
                layer: layer_id,
                prop: current,
                retime: is_retime,
                keys: hit.into_iter().map(|i| (i, keys[i].time)).collect(),
            });
        }
    } else if !bg.dragged() && app.graph_marquee.is_some() {
        app.graph_marquee = None; // abandoned mid-drag (channel switched)
    }

    // Y-axis scale: labelled gridlines in the active lens's units — the value
    // itself, or its rate of change per second in the speed lens.
    {
        // The Time channel reads in seconds of source; transform props use
        // their own unit (%, °, or none).
        let unit = if is_retime { "s" } else { prop_unit(current) };
        if app.graph_speed_view {
            graph_y_axis(ui, theme, rect, s_lo, s_hi, |v| {
                format!("{}{unit}/s", fmt_axis_value(v, s_hi - s_lo))
            });
        } else {
            graph_y_axis(ui, theme, rect, vmin, vmax, |v| {
                format!("{}{unit}", fmt_axis_value(v, vmax - vmin))
            });
        }
    }
    let points: Vec<egui::Pos2> = values
        .iter()
        .map(|(t, v)| {
            let y = if app.graph_speed_view {
                speed_y(*v)
            } else {
                y_of(*v)
            };
            egui::pos2(x_of(*t), y)
        })
        .collect();
    let stroke_colour = if app.graph_speed_view {
        theme.curve[1]
    } else {
        theme.curve[0]
    };
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5_f32, stroke_colour),
    ));

    // Playhead (layer time).
    if app.preview_comp == Some(comp.id) {
        let lt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
            - layer.start_offset.0.to_f64();
        let x = x_of(lt);
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, theme.accent),
        );
    }

    // Keys: draggable squares (value lens); draggable speed handles in the
    // speed lens (K-070 — editing the tangent, round-tripping to the store).
    if app.graph_speed_view {
        for (idx, key) in keys.iter().enumerate() {
            let x = x_of(key.time.to_f64());
            ui.painter().line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(0.5_f32, theme.hairline_strong),
            );
            // Where the handle rests: the key's bezier speed, or the sampled
            // derivative for a Linear/Hold key that carries no single speed.
            let rest = match (key.interp_out, key.interp_in) {
                (SideInterp::Bezier { speed, .. }, _) => speed,
                (_, SideInterp::Bezier { speed, .. }) => speed,
                _ => sample_at(key.time.to_f64()),
            };
            let sp = match app.graph_speed_edit {
                Some((i, s)) if i == idx => s,
                // Follow a live tangent-handle drag on this key too, so the
                // point rides with the handle (K-081).
                _ => match app.graph_tangent_edit {
                    Some((i, _, s, _)) if i == idx => s,
                    _ => rest,
                },
            };
            let pos = egui::pos2(x, speed_y(sp));
            let resp = ui.interact(
                egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0)),
                ui.id().with(("gspd", layer_id, idx)),
                egui::Sense::click_and_drag(),
            );
            let selected = selection.contains(&idx);
            let active = app.graph_speed_edit.is_some_and(|(i, _)| i == idx) || selected;
            let colour = if resp.hovered() || active {
                theme.accent
            } else {
                theme.curve[1]
            };
            ui.painter().circle_filled(pos, 4.0, colour);
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    app.graph_speed_edit = Some((idx, speed_of(p.y)));
                }
            }
            if resp.drag_stopped() {
                if let Some((i, s)) = app.graph_speed_edit.take() {
                    if i == idx {
                        let mut new_keys = keys.clone();
                        let side = SideInterp::Bezier {
                            speed: s,
                            influence: side_influence(new_keys[i].interp_out),
                        };
                        new_keys[i].interp_in = side;
                        new_keys[i].interp_out = side;
                        pending = Some(new_keys);
                    }
                }
            }
            // A plain click selects the key, so its tangent handles appear —
            // the same select-then-shape gesture as the value lens.
            if resp.clicked() {
                app.graph_selection = Some(crate::app_state::GraphSelection {
                    layer: layer_id,
                    prop: current,
                    retime: is_retime,
                    keys: vec![(idx, key.time)],
                });
            }
        }

        // Gold tangent handles in the speed lens (K-081): a selected key shows
        // the same handles as the value lens, here shaping the derivative
        // directly — a handle's *height* is that side's speed and its horizontal
        // reach is its influence (AE's speed-graph ease bars). They write the
        // same bezier store, so the value and speed lenses stay in lock-step.
        let handle_colour = theme.curve[3];
        let alt_now = ui.input(|i| i.modifiers.alt);
        // Whether the in-flight drag mirrors, from the latched mode (see
        // tangent_mirrors); only consulted while a drag is live.
        let mirror_live = app
            .graph_tangent_mode
            .is_some_and(|(u, a)| tangent_mirrors(u, a));
        for &idx in &selection {
            let Some(key) = keys.get(idx) else { continue };
            let kt = key.time.to_f64();
            let x = x_of(kt);
            let key_unified = matches!(
                (side_speed(key.interp_in), side_speed(key.interp_out)),
                (Some(a), Some(b)) if (a - b).abs() < 1e-6
            );
            for is_out in [true, false] {
                let side = if is_out {
                    key.interp_out
                } else {
                    key.interp_in
                };
                let has_neighbour = if is_out {
                    idx + 1 < keys.len()
                } else {
                    idx > 0
                };
                if !has_neighbour || !matches!(side, SideInterp::Bezier { .. }) {
                    continue;
                }
                let seg = if is_out {
                    keys[idx + 1].time.to_f64() - kt
                } else {
                    kt - keys[idx - 1].time.to_f64()
                };
                if seg <= 1e-6 {
                    continue;
                }
                // In-flight drag overrides this side; a mirroring partner takes
                // the dragged speed (keeping its own reach).
                let (sp, influence) = match app.graph_tangent_edit {
                    Some((i, o, s, inf)) if i == idx && o == is_out => (s, inf),
                    Some((i, _, s, _)) if i == idx && mirror_live => (s, side_influence(side)),
                    _ => (side_speed(side).unwrap_or(0.0), side_influence(side)),
                };
                let reach = influence * seg;
                let anchor = egui::pos2(x, speed_y(sp));
                let hend = egui::pos2(
                    x_of(if is_out { kt + reach } else { kt - reach }),
                    speed_y(sp),
                );
                ui.painter()
                    .line_segment([anchor, hend], egui::Stroke::new(1.0_f32, handle_colour));
                let hresp = ui.interact(
                    egui::Rect::from_center_size(hend, egui::vec2(10.0, 10.0)),
                    ui.id().with(("gspdtan", layer_id, idx, is_out)),
                    egui::Sense::click_and_drag(),
                );
                let hot = hresp.hovered()
                    || app
                        .graph_tangent_edit
                        .is_some_and(|(i, o, ..)| i == idx && o == is_out);
                ui.painter()
                    .circle_filled(hend, if hot { 4.5 } else { 3.0 }, handle_colour);
                if hresp.drag_started() {
                    // The drag's mirroring is decided here and only toggled by
                    // Alt (latched); see tangent_mirrors.
                    app.graph_tangent_mode = Some((key_unified, alt_now));
                }
                if hresp.dragged() {
                    if let Some(m) = &mut app.graph_tangent_mode {
                        m.1 |= alt_now; // Alt latches for the rest of the drag
                    }
                    if let Some(p) = hresp.interact_pointer_pos() {
                        // Horizontal reach → influence; height → this side's
                        // speed. No partner-length trickery here: the speed lens
                        // is about the speeds themselves.
                        let pt = t_of(p.x);
                        let dt = (if is_out { pt - kt } else { kt - pt }).clamp(seg * 1e-3, seg);
                        let inf = dt / seg;
                        app.graph_tangent_edit = Some((idx, is_out, speed_of(p.y), inf));
                    }
                }
                if hresp.drag_stopped() {
                    if let Some((i, o, sp2, inf)) = app.graph_tangent_edit.take() {
                        let mode = app.graph_tangent_mode.take();
                        if i == idx {
                            let mirror = mode.is_some_and(|(u, a)| tangent_mirrors(u, a));
                            let mut new_keys = keys.clone();
                            apply_tangent(&mut new_keys[i], o, sp2, inf, mirror, None);
                            pending = Some(new_keys);
                        }
                    }
                }
            }
        }
    }
    for (idx, key) in keys.iter().enumerate() {
        if app.graph_speed_view {
            break; // the speed lens is handled above; this loop is the value lens
        }
        let selected = selection.contains(&idx);
        let (kt, kv) = match app.graph_edit {
            Some((i, t, v)) if i == idx => (t, v),
            _ => match multi_delta {
                // The rest of the selection previews the dragged key's delta.
                Some(d) if selected => (key.time.to_f64(), key.value + d),
                _ => (key.time.to_f64(), key.value),
            },
        };
        let pos = egui::pos2(x_of(kt), y_of(kv));
        let hit = egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0));
        let resp = ui.interact(
            hit,
            ui.id().with(("gkey", layer_id, idx)),
            egui::Sense::click_and_drag(),
        );
        let colour = if resp.hovered() || selected || app.graph_edit.is_some_and(|(i, ..)| i == idx)
        {
            theme.accent
        } else {
            theme.text_secondary
        };
        // A selected key wears an accent ring on top of its glyph, so the
        // marquee's catch is obvious at a glance.
        if selected {
            ui.painter()
                .circle_stroke(pos, 6.5, egui::Stroke::new(1.5_f32, theme.accent));
        }
        match key_shape(key) {
            KeyShape::Square => {
                ui.painter().rect_filled(
                    egui::Rect::from_center_size(pos, egui::vec2(7.0, 7.0)),
                    1.0,
                    colour,
                );
            }
            KeyShape::Circle => {
                ui.painter().circle_filled(pos, 4.0, colour);
            }
            KeyShape::Diamond => {
                let d = 4.5;
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(pos.x, pos.y - d),
                        egui::pos2(pos.x + d, pos.y),
                        egui::pos2(pos.x, pos.y + d),
                        egui::pos2(pos.x - d, pos.y),
                    ],
                    colour,
                    egui::Stroke::NONE,
                ));
            }
        }

        // Tangent handles (value lens): a selected key shows a draggable yellow
        // handle on each bezier side. Dragging sets that side's slope (speed) and
        // reach (influence); a smooth key mirrors the slope across itself, and
        // Alt-drag breaks it to move one end alone (apply_tangent). Handles are
        // interacted after the key so a grab on the handle wins over the key drag.
        if selected {
            let handle_colour = theme.curve[3]; // the tangent (gold) accent
            let alt_now = ui.input(|i| i.modifiers.alt);
            // A smooth key (both sides bezier, equal slope) mirrors a drag across
            // itself, so the partner handle previews the same slope live.
            let key_unified = matches!(
                (side_speed(key.interp_in), side_speed(key.interp_out)),
                (Some(a), Some(b)) if (a - b).abs() < 1e-6
            );
            // Whether the in-flight drag mirrors, from the latched mode.
            let mirror_live = app
                .graph_tangent_mode
                .is_some_and(|(u, a)| tangent_mirrors(u, a));
            for is_out in [true, false] {
                let side = if is_out {
                    key.interp_out
                } else {
                    key.interp_in
                };
                let has_neighbour = if is_out {
                    idx + 1 < keys.len()
                } else {
                    idx > 0
                };
                if !has_neighbour || !matches!(side, SideInterp::Bezier { .. }) {
                    continue;
                }
                let seg = if is_out {
                    keys[idx + 1].time.to_f64() - kt
                } else {
                    kt - keys[idx - 1].time.to_f64()
                };
                if seg <= 1e-6 {
                    continue;
                }
                // The in-flight drag overrides this side's rest tangent; a
                // mirroring partner rotates to the dragged slope with its
                // on-screen length conserved (the live px scales).
                let (speed, influence) = match app.graph_tangent_edit {
                    Some((i, o, sp, inf)) if i == idx && o == is_out => (sp, inf),
                    Some((i, _, sp, _)) if i == idx && mirror_live => {
                        (sp, partner_influence(side, seg, sp, sx_px, sy_px))
                    }
                    _ => (side_speed(side).unwrap_or(0.0), side_influence(side)),
                };
                let reach = influence * seg;
                let (et, ev) = if is_out {
                    (kt + reach, kv + speed * reach)
                } else {
                    (kt - reach, kv - speed * reach)
                };
                let hpos = egui::pos2(x_of(et), y_of(ev));
                ui.painter()
                    .line_segment([pos, hpos], egui::Stroke::new(1.0_f32, handle_colour));
                let hresp = ui.interact(
                    egui::Rect::from_center_size(hpos, egui::vec2(10.0, 10.0)),
                    ui.id().with(("gtan", layer_id, idx, is_out)),
                    egui::Sense::click_and_drag(),
                );
                let hot = hresp.hovered()
                    || app
                        .graph_tangent_edit
                        .is_some_and(|(i, o, ..)| i == idx && o == is_out);
                ui.painter()
                    .circle_filled(hpos, if hot { 4.5 } else { 3.0 }, handle_colour);
                if hresp.drag_started() {
                    // The drag's mirroring is decided here and only toggled by
                    // Alt (latched); see tangent_mirrors.
                    app.graph_tangent_mode = Some((key_unified, alt_now));
                }
                if hresp.dragged() {
                    if let Some(m) = &mut app.graph_tangent_mode {
                        m.1 |= alt_now; // Alt latches for the rest of the drag
                    }
                    if let Some(p) = hresp.interact_pointer_pos() {
                        let (pt, pv) = (t_of(p.x), v_of(p.y));
                        // Horizontal reach, clamped inside the segment with a
                        // small floor. Influence and speed share this same reach
                        // so the handle lands exactly under the cursor — no
                        // magnetise-to-vertical, no sudden lengthening (Mack).
                        let dt = (if is_out { pt - kt } else { kt - pt }).clamp(seg * 1e-3, seg);
                        let inf = dt / seg;
                        let sp = if is_out {
                            (pv - kv) / dt
                        } else {
                            (kv - pv) / dt
                        };
                        app.graph_tangent_edit = Some((idx, is_out, sp, inf));
                    }
                }
                if hresp.drag_stopped() {
                    if let Some((i, o, sp, inf)) = app.graph_tangent_edit.take() {
                        let mode = app.graph_tangent_mode.take();
                        if i == idx {
                            let mirror = mode.is_some_and(|(u, a)| tangent_mirrors(u, a));
                            let mut new_keys = keys.clone();
                            let partner = if o {
                                new_keys[i].interp_in
                            } else {
                                new_keys[i].interp_out
                            };
                            let pinf =
                                partner_influence(partner, partner_seg(i, o), sp, sx_px, sy_px);
                            apply_tangent(&mut new_keys[i], o, sp, inf, mirror, Some(pinf));
                            pending = Some(new_keys);
                        }
                    }
                }
            }
        }

        // A plain click selects just this key, so its handles appear.
        if resp.clicked() {
            app.graph_selection = Some(crate::app_state::GraphSelection {
                layer: layer_id,
                prop: current,
                retime: is_retime,
                keys: vec![(idx, key.time)],
            });
        }
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                if multi && selected {
                    // The whole selection rides this drag: value-only, the
                    // dragged key's time stays locked.
                    app.graph_edit = Some((idx, key.time.to_f64(), v_of(p.y)));
                } else {
                    // Retime features snap to beats (docs/09-AUDIO v1): a Time
                    // key dragged near a marker lands exactly on it, so a ramp
                    // hits the beat. Otherwise the magnet (note 2.7, on by
                    // default) snaps the key to the nearest whole frame the
                    // playhead can land on; toggling it off drags freely.
                    let mut nt = t_of(p.x);
                    if is_retime {
                        let thr = 6.0 / px_per_sec.max(1e-6);
                        nt = lumit_core::markers::snap_time(
                            rational_at(nt),
                            &comp.markers,
                            rational_at(thr),
                        )
                        .to_f64();
                    } else if app.magnet_snap {
                        let fps = comp.frame_rate.fps().max(1.0);
                        nt = (nt * fps).round() / fps;
                    }
                    // The Retime Time channel's first key is the domain-start
                    // boundary, pinned at local time 0 (docs/04-RETIMING §3): a
                    // drag edits only its source value. Pinning keeps the rebuilt
                    // store valid so the edit commits instead of vanishing.
                    nt = retime_drag_time(is_retime, idx, nt);
                    app.graph_edit = Some((idx, nt, v_of(p.y)));
                    if !selected {
                        // Dragging an unselected key collapses the selection
                        // to just it (today's single-key drag, plus select).
                        app.graph_selection = Some(crate::app_state::GraphSelection {
                            layer: layer_id,
                            prop: current,
                            retime: is_retime,
                            keys: vec![(idx, key.time)],
                        });
                    }
                }
            }
        }
        if resp.drag_stopped() {
            if let Some((i, kt, kv)) = app.graph_edit.take() {
                if i == idx {
                    let mut new_keys = keys.clone();
                    if multi && selected {
                        // One op moves every selected key by the same value
                        // delta — a single undo step; times are untouched.
                        nudge_selected_values(&mut new_keys, &selection, kv - key.value);
                    } else {
                        let nt = rational_at(kt.max(0.0));
                        new_keys[i].time = nt;
                        new_keys[i].value = kv;
                        new_keys.sort_by_key(|k| k.time);
                        new_keys.dedup_by(|a, b| a.time == b.time);
                        // The dragged key stays selected where it landed.
                        let ni = new_keys.iter().position(|k| k.time == nt);
                        app.graph_selection = ni.map(|n| crate::app_state::GraphSelection {
                            layer: layer_id,
                            prop: current,
                            retime: is_retime,
                            keys: vec![(n, nt)],
                        });
                    }
                    pending = Some(new_keys);
                }
            }
        }
        // Right-click a key: set its interpolation, or delete it.
        resp.context_menu(|ui| {
            let mut sides: Option<SideInterp> = None;
            let mut delete = false;
            if ui.button("Easy ease").clicked() {
                sides = Some(lumit_core::anim::EASY_EASE);
                ui.close_menu();
            }
            if ui.button("Linear").clicked() {
                sides = Some(SideInterp::Linear);
                ui.close_menu();
            }
            if ui.button("Hold").clicked() {
                sides = Some(SideInterp::Hold);
                ui.close_menu();
            }
            // Re-join a broken bezier key: both handles take the average slope,
            // each keeping its own reach — the inverse of an Alt-drag break.
            let mut unify = false;
            if matches!(key.interp_in, SideInterp::Bezier { .. })
                && matches!(key.interp_out, SideInterp::Bezier { .. })
                && side_speed(key.interp_in) != side_speed(key.interp_out)
                && ui.button("Unify handles").clicked()
            {
                unify = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Delete key").clicked() {
                delete = true;
                ui.close_menu();
            }
            if let Some(si) = sides {
                let mut new_keys = keys.clone();
                new_keys[idx].interp_in = si;
                new_keys[idx].interp_out = si;
                pending = Some(new_keys);
            } else if unify {
                let mut new_keys = keys.clone();
                let avg = 0.5
                    * (side_speed(key.interp_in).unwrap_or(0.0)
                        + side_speed(key.interp_out).unwrap_or(0.0));
                new_keys[idx].interp_in = SideInterp::Bezier {
                    speed: avg,
                    influence: side_influence(key.interp_in),
                };
                new_keys[idx].interp_out = SideInterp::Bezier {
                    speed: avg,
                    influence: side_influence(key.interp_out),
                };
                pending = Some(new_keys);
            } else if delete {
                let mut new_keys = keys.clone();
                new_keys.remove(idx);
                app.graph_selection = None; // indices shift under a removal
                pending = Some(new_keys);
            }
        });
    }

    // The in-flight marquee band: translucent accent fill, hairline outline.
    if let Some((a, b)) = app.graph_marquee {
        let band = egui::Rect::from_two_pos(a, b).intersect(rect);
        ui.painter()
            .rect_filled(band, 0.0, theme.accent.gamma_multiply(0.12));
        ui.painter().rect_stroke(
            band,
            0.0,
            egui::Stroke::new(1.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
    }

    if let Some(sides) = set_sides {
        let mut new_keys = keys.clone();
        // The selection, or every key when nothing is picked out.
        let targets: Vec<usize> = if selection.is_empty() {
            (0..new_keys.len()).collect()
        } else {
            selection.clone()
        };
        for i in targets {
            if let Some(k) = new_keys.get_mut(i) {
                k.interp_in = sides;
                k.interp_out = sides;
            }
        }
        pending = Some(new_keys);
    }

    // A dedicated vertical scrollbar for the graph (K-079), on its right edge:
    // it appears once you've taken a manual value range that doesn't cover the
    // whole curve, and drags the value view up and down. It is independent of
    // the layer list's own scrollbar further right — the graph and the layer
    // list scroll separately.
    if !app.graph_speed_view {
        if let Some((vlo, vhi)) = app.graph_view_y {
            let (clo, chi) = auto_fit;
            let full_lo = clo.min(vlo);
            let full_hi = chi.max(vhi);
            let full = (full_hi - full_lo).max(1e-9);
            let view = (vhi - vlo).max(1e-9);
            if view < full - 1e-9 {
                let x1 = rect.right() - 2.0;
                let track = egui::Rect::from_min_max(
                    egui::pos2(x1 - 5.0, rect.top() + 2.0),
                    egui::pos2(x1, rect.bottom() - 2.0),
                );
                ui.painter().rect_filled(track, 3.0, theme.surface_1);
                let top_frac = ((full_hi - vhi) / full) as f32;
                let h_frac = (view / full) as f32;
                let thumb = egui::Rect::from_min_max(
                    egui::pos2(track.left(), track.top() + top_frac * track.height()),
                    egui::pos2(
                        track.right(),
                        track.top() + (top_frac + h_frac) * track.height(),
                    ),
                );
                let resp = ui.interact(
                    thumb,
                    ui.id().with(("graph-vscroll", layer_id)),
                    egui::Sense::drag(),
                );
                let col = if resp.hovered() || resp.dragged() {
                    theme.accent
                } else {
                    theme.text_muted
                };
                ui.painter().rect_filled(thumb, 3.0, col);
                if resp.dragged() {
                    // Drag down (positive Δy) moves the view to lower values;
                    // the range is clamped to the curve's own extent.
                    let dval =
                        -(resp.drag_delta().y as f64) / track.height().max(1.0) as f64 * full;
                    let (mut nlo, mut nhi) = (vlo + dval, vhi + dval);
                    if nlo < full_lo {
                        nhi += full_lo - nlo;
                        nlo = full_lo;
                    }
                    if nhi > full_hi {
                        nlo -= nhi - full_hi;
                        nhi = full_hi;
                    }
                    app.graph_auto_fit = false;
                    app.graph_view_y = Some((nlo, nhi));
                    app.graph_view_h = Some(rect.height());
                }
            }
        }
    }

    // All graph drawing is done; release the clip before the commit below (it
    // can return early on an unbuildable retime).
    ui.set_clip_rect(saved_clip);

    if let Some(new_keys) = pending {
        let op = if is_retime {
            // Rebuild the Retime store from the edited Time keyframes (K-078).
            // Fewer than two keys (or otherwise unbuildable) can't be a retime,
            // so that edit is dropped and the existing store kept.
            match lumit_core::retime::Retime::from_source_keyframes(&new_keys) {
                Some(rt) => lumit_core::Op::SetLayerRetime {
                    comp: comp.id,
                    layer: layer_id,
                    retime: Some(rt),
                },
                None => return,
            }
        } else {
            let animation = if new_keys.is_empty() {
                Animation::Static(static_val)
            } else {
                Animation::Keyframed(new_keys)
            };
            lumit_core::Op::SetTransformProperty {
                comp: comp.id,
                layer: layer_id,
                prop: current,
                animation,
            }
        };
        follow_edit(app, &op); // the graph follows the key you just touched
        app.commit(op);
    }
}
