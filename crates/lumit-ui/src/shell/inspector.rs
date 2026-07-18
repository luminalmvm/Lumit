//! `shell::inspector` — split out of the monolithic shell.rs (mechanical,
//! no logic changes). Shared names resolve through the parent module
//! via `use super::*` and the glob re-exports in `shell/mod.rs`.

use super::*;

/// The layer's natural pixel space (mask coordinates live here).
/// Compact matte / blend / 3D / mute controls for a layer's title line
/// (left column). Sets `pending` on any change.
/// Trim a layer title for display: people type what they like, but past a
/// cap the shown value ends with "…" (Mack).
pub(crate) fn trim_title(name: &str) -> String {
    const MAX: usize = 48;
    if name.chars().count() <= MAX {
        name.to_owned()
    } else {
        let mut s: String = name.chars().take(MAX - 1).collect();
        s.push('…');
        s
    }
}

/// Visibility (eye) toggle — its own left-column subcolumn.
pub(crate) fn visible_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    pending: &mut Option<lumit_core::Op>,
) {
    let vis = layer.switches.visible;
    let col = if vis {
        theme.text_secondary
    } else {
        theme.text_disabled
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    crate::icons::paint(ui.painter(), rect, Icon::Eye, col, 1.4);
    if resp.on_hover_text("Show / hide this layer").clicked() {
        *pending = Some(lumit_core::Op::SetLayerVisible {
            comp: comp_id,
            layer: layer.id,
            visible: !vis,
        });
    }
}

/// Matte subcolumn: a labelled "Matte" dropdown (accent when a matte is set)
/// with a drawn caret to show it opens a menu — source pick + luma/invert flags.
pub(crate) fn matte_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp: &lumit_core::model::Composition,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::model::{MatteChannel, MatteRef};
    let has_matte = layer.matte.is_some();
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::click());
    let base = if has_matte {
        theme.accent
    } else {
        theme.text_secondary
    };
    let colour = if resp.hovered() {
        theme.text_primary
    } else {
        base
    };
    ui.painter().text(
        rect.left_center() + egui::vec2(2.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "Matte",
        egui::FontId::proportional(11.0),
        colour,
    );
    crate::icons::caret_down(
        ui.painter(),
        egui::pos2(rect.right() - 6.0, rect.center().y),
        colour,
    );
    let popup_id = ui.make_persistent_id(("matte-popup", layer.id));
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    let mut set: Option<Option<MatteRef>> = None;
    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &resp,
        egui::PopupCloseBehavior::CloseOnClick,
        |ui| {
            ui.set_min_width(150.0);
            if ui.selectable_label(layer.matte.is_none(), "None").clicked() {
                set = Some(None);
            }
            for other in comp.layers.iter().filter(|l| l.id != layer.id) {
                let selected = layer.matte.is_some_and(|m| m.layer == other.id);
                if ui.selectable_label(selected, &other.name).clicked() {
                    set = Some(Some(MatteRef {
                        layer: other.id,
                        channel: layer
                            .matte
                            .map(|m| m.channel)
                            .unwrap_or(MatteChannel::Alpha),
                        inverted: layer.matte.is_some_and(|m| m.inverted),
                    }));
                }
            }
            if let Some(mut m) = layer.matte {
                ui.separator();
                let luma = matches!(m.channel, MatteChannel::Luma);
                if ui.selectable_label(luma, "Luma matte").clicked() {
                    m.channel = if luma {
                        MatteChannel::Alpha
                    } else {
                        MatteChannel::Luma
                    };
                    set = Some(Some(m));
                }
                if ui.selectable_label(m.inverted, "Inverted").clicked() {
                    m.inverted = !m.inverted;
                    set = Some(Some(m));
                }
            }
        },
    );
    if let Some(matte) = set {
        *pending = Some(lumit_core::Op::SetLayerMatte {
            comp: comp_id,
            layer: layer.id,
            matte,
        });
    }
}

pub(crate) fn blend_name(b: lumit_core::model::BlendMode) -> &'static str {
    use lumit_core::model::BlendMode;
    match b {
        BlendMode::Normal => "Normal",
        BlendMode::Add => "Add",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Overlay => "Overlay",
        BlendMode::SoftLight => "Soft light",
        BlendMode::HardLight => "Hard light",
        BlendMode::Lighten => "Lighten",
        BlendMode::Darken => "Darken",
    }
}

/// Blend-mode subcolumn.
pub(crate) fn blend_control(
    ui: &mut egui::Ui,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::model::BlendMode;
    bare_dropdown(
        ui,
        egui::RichText::new(blend_name(layer.blend)).small(),
        |ui| {
            for mode in [
                BlendMode::Normal,
                BlendMode::Add,
                BlendMode::Multiply,
                BlendMode::Screen,
                BlendMode::Overlay,
                BlendMode::SoftLight,
                BlendMode::HardLight,
                BlendMode::Lighten,
                BlendMode::Darken,
            ] {
                if ui
                    .selectable_label(layer.blend == mode, blend_name(mode))
                    .clicked()
                {
                    if layer.blend != mode {
                        *pending = Some(lumit_core::Op::SetLayerBlend {
                            comp: comp_id,
                            layer: layer.id,
                            blend: mode,
                        });
                    }
                    ui.close_menu();
                }
            }
        },
    );
}

/// 3D-switch subcolumn.
pub(crate) fn three_d_control(
    ui: &mut egui::Ui,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    pending: &mut Option<lumit_core::Op>,
) {
    if ui
        .selectable_label(layer.switches.three_d, egui::RichText::new("3D").small())
        .on_hover_text("Place this layer in z-space (needs a Camera layer)")
        .clicked()
    {
        *pending = Some(lumit_core::Op::SetLayerThreeD {
            comp: comp_id,
            layer: layer.id,
            three_d: !layer.switches.three_d,
        });
    }
}

/// Collapse-transformations subcolumn (Precomp layers only, docs/06 §1.4).
/// Accent when active; dimmed when the switch is set but a mask, blend,
/// opacity or matte use forces an intermediate anyway (the spec's required
/// "dimmed collapse switch" indication).
#[allow(clippy::too_many_arguments)]
pub(crate) fn collapse_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    lt: f64,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::model::CollapseState;
    if !matches!(layer.kind, lumit_core::model::LayerKind::Precomp { .. }) {
        return;
    }
    let state = lumit_core::model::collapse_state(doc, comp, layer, lt);
    let col = match state {
        CollapseState::Active => theme.accent,
        CollapseState::Forced => theme.text_disabled,
        CollapseState::Off => theme.text_secondary,
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    crate::icons::paint(ui.painter(), rect, Icon::Collapse, col, 1.4);
    let hover = match state {
        CollapseState::Active => "Collapse transformations: on (inner layers composite directly)",
        CollapseState::Forced => {
            "Collapse is set, but a mask, blend, opacity or matte use forces an intermediate"
        }
        CollapseState::Off => "Collapse transformations: concatenate inner layers' transforms",
    };
    if resp.on_hover_text(hover).clicked() {
        *pending = Some(lumit_core::Op::SetLayerCollapse {
            comp: comp_id,
            layer: layer.id,
            collapse: !layer.switches.collapse,
        });
    }
}

/// The Flow option toggle (K-088), footage layers only: optical-flow frame
/// interpolation as a property of how the layer samples its source. Accent
/// while on. Turning it on keeps any existing retime and sets its
/// interpolation policy to Flow (an identity retime is created when the
/// layer has none — it renders identically, docs/04 §3); turning it off
/// returns the policy to Nearest, and a pure identity store collapses back
/// to no retime at all.
pub(crate) fn flow_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::retime::{FlowParams, Interpolation, Retime};
    let lumit_core::model::LayerKind::Footage { retime, .. } = &layer.kind else {
        return;
    };
    let on = matches!(
        retime.as_ref().map(|r| &r.interpolation),
        Some(Interpolation::Flow(_))
    );
    let col = if on {
        theme.accent
    } else {
        theme.text_secondary
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    crate::icons::paint(ui.painter(), rect, Icon::Flow, col, 1.4);
    let hover = if on {
        "Flow: synthesising in-between frames where the footage is slower than the comp"
    } else {
        "Flow: synthesise in-between frames (optical flow) when the footage's rate          undershoots the comp's"
    };
    if resp.on_hover_text(hover).clicked() {
        let new_retime = if on {
            let mut r = retime
                .clone()
                .unwrap_or_else(|| Retime::identity(layer.out_point.0, lumit_core::Rational::ZERO));
            r.interpolation = Interpolation::Nearest;
            // A pure identity store with the default policy is no retime.
            if r == Retime::identity(layer.out_point.0, lumit_core::Rational::ZERO) {
                None
            } else {
                Some(r)
            }
        } else {
            let mut r = retime
                .clone()
                .unwrap_or_else(|| Retime::identity(layer.out_point.0, lumit_core::Rational::ZERO));
            r.interpolation = Interpolation::Flow(FlowParams::default());
            Some(r)
        };
        *pending = Some(lumit_core::Op::SetLayerRetime {
            comp: comp_id,
            layer: layer.id,
            retime: new_retime,
        });
    }
}

/// The Flow group's rows (K-088): shown while the option is on, beside
/// Transform and Effects, carrying the engine parameters.
pub(crate) fn flow_group_rows(
    ui: &mut egui::Ui,
    ctx: &RowCtx,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::retime::Interpolation;
    let lumit_core::model::LayerKind::Footage {
        retime: Some(rt), ..
    } = &ctx.layer.kind
    else {
        return;
    };
    let Interpolation::Flow(params) = &rt.interpolation else {
        return;
    };
    let (_row, mut c) = row_frame(ui, ctx, false);
    c.label(
        egui::RichText::new("Quality")
            .small()
            .color(ctx.theme.text_muted),
    );
    let cur = if params.half_resolution {
        "Half"
    } else {
        "Full"
    };
    bare_dropdown(&mut c, egui::RichText::new(cur).small(), |ui| {
        for (label, half) in [("Half (fast)", true), ("Full", false)] {
            if ui
                .selectable_label(params.half_resolution == half, label)
                .clicked()
            {
                let mut r = rt.clone();
                let mut p = params.clone();
                p.half_resolution = half;
                r.interpolation = Interpolation::Flow(p);
                *pending = Some(lumit_core::Op::SetLayerRetime {
                    comp: ctx.comp_id,
                    layer: ctx.layer.id,
                    retime: Some(r),
                });
                ui.close_menu();
            }
        }
    });

    // Input rate (K-095): interpret the footage as this fps for flow, so
    // high-framerate clips (whose adjacent frames barely move) interpolate
    // across meaningful gaps. Native = the source's own rate.
    let (_row, mut c) = row_frame(ui, ctx, false);
    c.label(
        egui::RichText::new("Input rate")
            .small()
            .color(ctx.theme.text_muted),
    )
    .on_hover_text(
        "Treat the footage as this frame rate for flow — lower than the clip's own rate to \
         flow-interpolate high-speed footage into real slow motion",
    );
    let cur = match params.input_fps {
        None => "Native".to_string(),
        Some(f) => format!("{} fps", f as i64),
    };
    let commit = |ctx: &RowCtx,
                  rt: &lumit_core::retime::Retime,
                  params: &lumit_core::retime::FlowParams,
                  pending: &mut Option<lumit_core::Op>,
                  input_fps: Option<f64>| {
        let mut r = rt.clone();
        let mut p = params.clone();
        p.input_fps = input_fps;
        r.interpolation = Interpolation::Flow(p);
        *pending = Some(lumit_core::Op::SetLayerRetime {
            comp: ctx.comp_id,
            layer: ctx.layer.id,
            retime: Some(r),
        });
    };
    bare_dropdown(&mut c, egui::RichText::new(cur).small(), |ui| {
        if ui
            .selectable_label(params.input_fps.is_none(), "Native")
            .clicked()
        {
            commit(ctx, rt, params, pending, None);
            ui.close_menu();
        }
        for fps in [8.0, 12.0, 15.0, 24.0, 25.0, 30.0, 60.0] {
            let sel = params.input_fps == Some(fps);
            if ui
                .selectable_label(sel, format!("{} fps", fps as i64))
                .clicked()
            {
                commit(ctx, rt, params, pending, Some(fps));
                ui.close_menu();
            }
        }
    });
}

/// Mute subcolumn (footage layers).
pub(crate) fn mute_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    pending: &mut Option<lumit_core::Op>,
) {
    let muted = !layer.switches.audible;
    let (icon, col) = if muted {
        (Icon::Mute, theme.text_muted)
    } else {
        (Icon::Audio, theme.text_secondary)
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    crate::icons::paint(ui.painter(), rect, icon, col, 1.4);
    if resp
        .on_hover_text("Silence this layer in playback and export")
        .clicked()
    {
        *pending = Some(lumit_core::Op::SetLayerAudible {
            comp: comp_id,
            layer: layer.id,
            audible: muted,
        });
    }
}

/// Every keyframe time (layer-local seconds) across a layer's animated
/// properties — for the timeline's keyframe glyphs.
pub(crate) fn layer_keyframe_times(layer: &lumit_core::model::Layer) -> Vec<f64> {
    use lumit_core::anim::Animation;
    use lumit_core::model::{LayerKind, TransformProp};
    let mut times = Vec::new();
    let mut collect = |anim: &Animation| {
        if let Animation::Keyframed(keys) = anim {
            times.extend(keys.iter().map(|k| k.time.to_f64()));
        }
    };
    for prop in [
        TransformProp::AnchorX,
        TransformProp::AnchorY,
        TransformProp::PositionX,
        TransformProp::PositionY,
        TransformProp::PositionZ,
        TransformProp::ScaleX,
        TransformProp::ScaleY,
        TransformProp::Rotation,
        TransformProp::RotationX,
        TransformProp::RotationY,
        TransformProp::Opacity,
    ] {
        collect(&layer.transform.get(prop).animation);
    }
    if let LayerKind::Camera { zoom } = &layer.kind {
        collect(&zoom.animation);
    }
    times
}

/// Read-only context shared by every property row in a layer's twirl-down.
pub(crate) struct RowCtx<'a> {
    pub(crate) theme: &'a Theme,
    pub(crate) comp_id: uuid::Uuid,
    pub(crate) layer: &'a lumit_core::model::Layer,
    pub(crate) lt: f64,
    pub(crate) off: f64,
    pub(crate) fps: f64,
    /// The lane scroll viewport, so property-row outlines clip to their own x
    /// but the viewport's y (no vertical bleed when a row is half-scrolled).
    pub(crate) viewport: egui::Rect,
    pub(crate) track_left: f32,
    pub(crate) track_w: f32,
    /// The displayed time axis (zoom + scroll), so property-row keyframe
    /// diamonds sit exactly under the layer bars at any zoom.
    pub(crate) px_per_sec: f64,
    pub(crate) view_start: f64,
    /// True in graph mode (K-070): the outline half of every row still draws,
    /// but nothing is painted on the lane side — the curve owns that area.
    pub(crate) graph_mode: bool,
}

/// New (scale_x, scale_y) when the linked Scale control is dragged so x becomes
/// `new_x`, keeping the x:y ratio. A ~zero old x has no defined ratio, so both
/// take the new value (uniform).
pub(crate) fn linked_scale(old_x: f64, old_y: f64, new_x: f64) -> (f64, f64) {
    if old_x.abs() < 1e-9 {
        (new_x, new_x)
    } else {
        (new_x, old_y * new_x / old_x)
    }
}

/// Paint a subtle full-width themed strip behind a section title (Mack): the
/// bar that makes it obvious where one effect (or the Transform group) ends and
/// the next begins. Drawn under the row's widgets, clipped to the outline via
/// the scroll viewport like every other left-column paint. `surface_1` is a
/// quiet step above the panel — never the selection colour.
pub(crate) fn section_bar(ui: &egui::Ui, ctx: &RowCtx, row_rect: egui::Rect) {
    let mut p = ui.painter().clone();
    p.set_clip_rect(ctx.viewport);
    let right = (ctx.track_left - 6.0).max(row_rect.left() + 1.0);
    p.rect_filled(
        egui::Rect::from_min_max(row_rect.min, egui::pos2(right, row_rect.bottom())),
        2.0,
        ctx.theme.surface_1,
    );
}

/// A collapsible sub-group header inside a layer's twirl-down ("Transform",
/// "Effects", …): a disclosure triangle and label, indented under the layer and
/// full width so it reads as a band. Persists and returns its open state. The
/// band is always drawn (a subtle themed strip, brighter on hover) so the
/// section title reads as its own bar.
pub(crate) fn group_header_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: &str,
    id: egui::Id,
    default_open: bool,
    viewport: egui::Rect,
) -> bool {
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(default_open);
    // The header lives in the outline, but the ui's clip is the lanes and egui
    // hit-tests against rect ∩ clip — so widen the clip or the twirl won't click.
    let (rect, resp) = {
        let saved = ui.clip_rect();
        ui.set_clip_rect(viewport);
        let r =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::click());
        ui.set_clip_rect(saved);
        r
    };
    // A group header sits in the outline column (left of the lanes). set_clip_rect
    // replaces the lane clip; with_clip_rect would intersect it and hide the row.
    let mut p = ui.painter().clone();
    p.set_clip_rect(viewport);
    p.rect_filled(
        rect,
        0.0,
        if resp.hovered() {
            theme.surface_2
        } else {
            theme.surface_1
        },
    );
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    let cy = rect.center().y;
    let tx = rect.left() + 22.0;
    let tri = egui::Rect::from_center_size(egui::pos2(tx, cy), egui::vec2(12.0, 12.0));
    crate::icons::disclosure(&p, tri, open, theme.text_muted);
    p.text(
        egui::pos2(tx + 10.0, cy),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        theme.text_secondary,
    );
    open
}

/// The layer's transform properties as full-width timeline rows (K-072): each
/// row shows its stopwatch/name/value in the left column and its own keyframes
/// as diamonds on the track to the right; clicking a row's name graphs it.
/// Scale x/y share one row with a ratio lock (default on); unlocking splits
/// them into two independent rows with a relink control. Anchor and Position
/// x/y are linked by default too, but only as row furniture — one row carries
/// two independent values (AE-style), never coupling them like Scale's ratio.
#[allow(clippy::too_many_arguments)]
pub(crate) fn transform_property_rows(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp: &lumit_core::model::Composition,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    _name_w: f32,
    track_left: f32,
    track_w: f32,
    px_per_sec: f64,
    view_start: f64,
    viewport: egui::Rect,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::model::{LayerKind, TransformProp};

    let is_camera = matches!(layer.kind, LayerKind::Camera { .. });
    let three_d = layer.switches.three_d || is_camera;
    let fps = comp.frame_rate.fps().max(1.0);
    let ctx = RowCtx {
        theme,
        comp_id,
        layer,
        lt: app.preview_frame as f64 / fps - layer.start_offset.0.to_f64(),
        off: layer.start_offset.0.to_f64(),
        fps,
        viewport,
        track_left,
        track_w,
        px_per_sec,
        view_start,
        graph_mode: app.timeline_graph_mode,
    };

    // Footage speed is a keyframable property too (K-072): its own row above
    // the transform, its keys building the retime's speed lens.
    if let LayerKind::Footage { retime, .. } = &layer.kind {
        speed_property_row(ui, app, &ctx, retime, pending);
    }

    // Anchor and Position: x and y share one row by default, AE-style. Unlike
    // Scale's ratio lock the two values never couple — linking only merges the
    // row furniture (one stopwatch, one navigator, one lane). The chain
    // button splits them into today's separate rows, per layer.
    if !is_camera {
        linked_pair_block(
            ui,
            app,
            &ctx,
            "anchor-unlink",
            "Anchor",
            (TransformProp::AnchorX, TransformProp::AnchorY),
            pending,
        );
    }
    linked_pair_block(
        ui,
        app,
        &ctx,
        "pos-unlink",
        "Position",
        (TransformProp::PositionX, TransformProp::PositionY),
        pending,
    );

    // Scale with a ratio lock (default on). Locked: one row edits both, keeping
    // the ratio. Unlocked: two independent rows plus a relink control.
    let scale_id = ui.id().with(("scale-unlink", layer.id));
    let mut unlinked = ui.data(|d| d.get_temp::<bool>(scale_id)).unwrap_or(false);
    if unlinked {
        prop_row(
            ui,
            app,
            &ctx,
            "Scale x %",
            TransformProp::ScaleX,
            0.5,
            pending,
        );
        prop_row(
            ui,
            app,
            &ctx,
            "Scale y %",
            TransformProp::ScaleY,
            0.5,
            pending,
        );
        if link_toggle_row(
            ui,
            &ctx,
            "Link scale",
            "Re-lock the x:y ratio and edit scale as one value",
        ) {
            unlinked = false;
        }
    } else {
        combined_scale_row(ui, app, &ctx, pending, &mut unlinked);
    }
    ui.data_mut(|d| d.insert_temp(scale_id, unlinked));

    prop_row(
        ui,
        app,
        &ctx,
        "Rotation °",
        TransformProp::Rotation,
        0.5,
        pending,
    );
    prop_row(
        ui,
        app,
        &ctx,
        "Opacity %",
        TransformProp::Opacity,
        0.5,
        pending,
    );
    if three_d {
        prop_row(
            ui,
            app,
            &ctx,
            "Position z",
            TransformProp::PositionZ,
            1.0,
            pending,
        );
        prop_row(
            ui,
            app,
            &ctx,
            "Rotation x °",
            TransformProp::RotationX,
            0.5,
            pending,
        );
        prop_row(
            ui,
            app,
            &ctx,
            "Rotation y °",
            TransformProp::RotationY,
            0.5,
            pending,
        );
    }
}

/// One linked-by-default x/y pair (Anchor, Position): a single two-value row,
/// or — once unlinked via the chain button — two independent rows plus a
/// relink control. The choice is per layer, kept in ui temp data under
/// `(key, layer.id)`, and purely presentational: no document state changes.
pub(crate) fn linked_pair_block(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    key: &'static str,
    label: &str,
    props: (
        lumit_core::model::TransformProp,
        lumit_core::model::TransformProp,
    ),
    pending: &mut Option<lumit_core::Op>,
) {
    let id = ui.id().with((key, ctx.layer.id));
    let mut unlinked = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let lower = label.to_lowercase();
    if unlinked {
        prop_row(ui, app, ctx, &format!("{label} x"), props.0, 1.0, pending);
        prop_row(ui, app, ctx, &format!("{label} y"), props.1, 1.0, pending);
        if link_toggle_row(
            ui,
            ctx,
            &format!("Link {lower}"),
            &format!("Rejoin {lower} x and y on one row (values stay independent)"),
        ) {
            unlinked = false;
        }
    } else {
        linked_pair_row(ui, app, ctx, label, props, pending, &mut unlinked);
    }
    ui.data_mut(|d| d.insert_temp(id, unlinked));
}

/// Allocate one 18px timeline row and return (row_rect, left-column child ui).
/// The child is clipped so widgets never spill into the track area.
pub(crate) fn row_frame(
    ui: &mut egui::Ui,
    ctx: &RowCtx,
    highlight: bool,
) -> (egui::Rect, egui::Ui) {
    let (row_rect, _resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    if highlight {
        // Left of the lanes → replace the clip; with_clip_rect would intersect the
        // lane clip and hide this highlight.
        let mut hp = ui.painter().clone();
        hp.set_clip_rect(ctx.viewport);
        hp.rect_filled(
            egui::Rect::from_min_max(
                row_rect.min,
                egui::pos2(ctx.track_left - 6.0, row_rect.bottom()),
            ),
            2.0,
            ctx.theme.surface_2,
        );
    }
    let left_rect = egui::Rect::from_min_max(
        egui::pos2(row_rect.left() + 24.0, row_rect.top()),
        egui::pos2(
            (ctx.track_left - 6.0).max(row_rect.left() + 25.0),
            row_rect.bottom(),
        ),
    );
    let mut c = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    // Clip to the outline column, but bounded by the scroll viewport's y so a
    // half-scrolled property row doesn't bleed past the ruler.
    c.set_clip_rect(left_rect.intersect(ctx.viewport));
    (row_rect, c)
}

/// Draw clay diamonds for `keys` on the track portion of `row_rect`.
pub(crate) fn draw_key_diamonds(
    ui: &egui::Ui,
    ctx: &RowCtx,
    row_rect: egui::Rect,
    keys: &[lumit_core::anim::Keyframe],
) {
    // In graph mode the lane side belongs to the curve — no diamonds there.
    if ctx.graph_mode {
        return;
    }
    let cy = row_rect.center().y;
    // The same displayed (zoomed, scrolled) axis as the layer bars, so a
    // property's diamonds stay under its layer's keys at any zoom.
    let x_of = |s: f64| ctx.track_left + ((s - ctx.view_start) * ctx.px_per_sec) as f32;
    for k in keys {
        let x = x_of(ctx.off + k.time.to_f64());
        if x >= ctx.track_left - 1.0 && x <= ctx.track_left + ctx.track_w + 1.0 {
            let d = 3.0;
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x, cy - d),
                    egui::pos2(x + d, cy),
                    egui::pos2(x, cy + d),
                    egui::pos2(x - d, cy),
                ],
                ctx.theme.accent,
                egui::Stroke::new(1.0_f32, ctx.theme.surface_0),
            ));
        }
    }
}

/// The stopwatch toggle. Returns the new Animation if clicked (animate at the
/// playhead / freeze to the current value), else None.
/// A drawn, clickable stopwatch — a filled dot when animated, a ring when not.
/// Replaces the old `⏱`/`◦` glyph (egui's fonts can't render the emoji, so it
/// vanished), and clips like any child-ui widget. Returns true on click.
pub(crate) fn stopwatch_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    animated: bool,
    hover: &str,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    let color = if resp.hovered() {
        theme.text_primary
    } else if animated {
        theme.accent
    } else {
        theme.text_muted
    };
    crate::icons::stopwatch(ui.painter(), rect.center(), 4.5, animated, color);
    resp.on_hover_text(hover).clicked()
}

pub(crate) fn stopwatch(
    ui: &mut egui::Ui,
    theme: &Theme,
    slot: &lumit_core::anim::Property,
    lt: f64,
) -> Option<lumit_core::anim::Animation> {
    use lumit_core::anim::{Animation, Keyframe, SideInterp};
    let animated = slot.is_animated();
    let hover = if animated {
        "Remove animation (freeze current value)"
    } else {
        "Animate: keyframe at the playhead"
    };
    if stopwatch_button(ui, theme, animated, hover) {
        Some(if animated {
            Animation::Static(slot.value_at(lt))
        } else {
            Animation::Keyframed(vec![Keyframe {
                time: rational_at(lt),
                value: slot.value_at(lt),
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            }])
        })
    } else {
        None
    }
}

/// AE-style keyframe navigator for an animated property, shown next to the
/// stopwatch: ◄ jumps the playhead to the previous keyframe, the diamond adds a
/// keyframe at the playhead (filled ◆ when one is already there — clicking then
/// removes it), ► jumps to the next keyframe.
pub(crate) fn keyframe_nav(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    prop: lumit_core::model::TransformProp,
    slot: &lumit_core::anim::Property,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::anim::Animation;
    let Animation::Keyframed(keys) = &slot.animation else {
        return;
    };
    let tol = 0.5 / ctx.fps.max(1.0); // within half a frame counts as "on" it
                                      // Iconoir glyphs (K-085): the old ◄ ◆ ► characters aren't in the UI fonts
                                      // and rendered as blanks. No colour is set, so disabled buttons dim.
    let small = |i: Icon| egui::Button::new(crate::icons::text(i, 11.0)).frame(false);
    let mut jump_to: Option<f64> = None;

    let has_prev = keys.iter().any(|k| k.time.to_f64() < ctx.lt - tol);
    if ui
        .add_enabled(has_prev, small(Icon::PrevKeyframe))
        .on_hover_text("Previous keyframe")
        .clicked()
    {
        jump_to = keys
            .iter()
            .rev()
            .find(|k| k.time.to_f64() < ctx.lt - tol)
            .map(|k| k.time.to_f64());
    }

    let on_key = keys.iter().any(|k| (k.time.to_f64() - ctx.lt).abs() < tol);
    if ui
        .add(small(if on_key {
            Icon::Keyframe
        } else {
            Icon::KeyframeAdd
        }))
        .on_hover_text(if on_key {
            "Remove keyframe here"
        } else {
            "Add keyframe here"
        })
        .clicked()
    {
        let animation = if on_key {
            let kept: Vec<_> = keys
                .iter()
                .filter(|k| (k.time.to_f64() - ctx.lt).abs() >= tol)
                .cloned()
                .collect();
            if kept.is_empty() {
                Animation::Static(slot.value_at(ctx.lt))
            } else {
                Animation::Keyframed(kept)
            }
        } else {
            Animation::Keyframed(upsert_key(slot, ctx.lt, slot.value_at(ctx.lt)))
        };
        *pending = Some(lumit_core::Op::SetTransformProperty {
            comp: ctx.comp_id,
            layer: ctx.layer.id,
            prop,
            animation,
        });
    }

    let has_next = keys.iter().any(|k| k.time.to_f64() > ctx.lt + tol);
    if ui
        .add_enabled(has_next, small(Icon::NextKeyframe))
        .on_hover_text("Next keyframe")
        .clicked()
    {
        jump_to = keys
            .iter()
            .find(|k| k.time.to_f64() > ctx.lt + tol)
            .map(|k| k.time.to_f64());
    }

    if let Some(kt) = jump_to {
        app.preview_frame = ((kt + ctx.off) * ctx.fps).round().max(0.0) as usize;
        #[cfg(feature = "media")]
        app.refresh_preview();
    }
}

/// One generic property row.
pub(crate) fn prop_row(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    label: &str,
    prop: lumit_core::model::TransformProp,
    speed: f64,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::anim::Animation;
    let slot = ctx.layer.transform.get(prop);
    let is_graphed = app.selected_layer == Some(ctx.layer.id)
        && !app.graph_retime
        && app.graph_prop == Some(prop);
    let (row_rect, mut c) = row_frame(ui, ctx, is_graphed);

    if let Some(animation) = stopwatch(&mut c, ctx.theme, slot, ctx.lt) {
        *pending = Some(lumit_core::Op::SetTransformProperty {
            comp: ctx.comp_id,
            layer: ctx.layer.id,
            prop,
            animation,
        });
    }
    keyframe_nav(&mut c, app, ctx, prop, slot, pending);
    if c.add(
        egui::Label::new(egui::RichText::new(label).small().color(if is_graphed {
            ctx.theme.accent
        } else {
            ctx.theme.text_muted
        }))
        .sense(egui::Sense::click()),
    )
    .clicked()
    {
        app.selected_layer = Some(ctx.layer.id);
        app.graph_prop = Some(prop);
        app.graph_retime = false; // switching to a transform property
        app.graph_reset_fit(); // a fresh channel starts fitted
    }
    axis_drag_value(&mut c, app, ctx, prop, speed, pending);
    if let Animation::Keyframed(keys) = &slot.animation {
        draw_key_diamonds(ui, ctx, row_rect, keys);
    }
}

/// One axis's value box with the shared commit rules (prop_row and the linked
/// Anchor/Position rows both use it): dragging edits live through
/// `app.prop_edit`; on release, a typed value with a marquee multi-selection
/// on this exact channel sets every selected keyframe to that value —
/// absolute, one undo step — while any other commit upserts a key at the
/// playhead (animated) or replaces the static value.
pub(crate) fn axis_drag_value(
    c: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    prop: lumit_core::model::TransformProp,
    speed: f64,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::anim::Animation;
    let slot = ctx.layer.transform.get(prop);
    let committed = slot.value_at(ctx.lt);
    let mut value = match app.prop_edit {
        Some((l, p, v)) if l == ctx.layer.id && p == prop => v,
        _ => committed,
    };
    let resp = c.add(
        egui::DragValue::new(&mut value)
            .speed(speed)
            .max_decimals(2),
    );
    if resp.dragged() || resp.has_focus() {
        app.prop_edit = Some((ctx.layer.id, prop, value));
    }
    if resp.drag_stopped() || resp.lost_focus() {
        // A typed value (the field was focused, not dragged) with a
        // marquee multi-selection on this exact channel sets every
        // selected keyframe to that value — absolute, one undo step.
        // Dragging the field keeps its usual single-value behaviour.
        let multi_set = if resp.drag_stopped() {
            None
        } else if let Animation::Keyframed(keys) = &slot.animation {
            graph_multi_selection(app, ctx.layer.id, prop, keys).map(|sel| {
                let mut new_keys = keys.clone();
                let changed = set_selected_values(&mut new_keys, &sel, value);
                (new_keys, changed)
            })
        } else {
            None
        };
        if let Some((new_keys, changed)) = multi_set {
            if changed {
                *pending = Some(lumit_core::Op::SetTransformProperty {
                    comp: ctx.comp_id,
                    layer: ctx.layer.id,
                    prop,
                    animation: Animation::Keyframed(new_keys),
                });
            }
        } else if (value - committed).abs() > f64::EPSILON {
            let animation = if slot.is_animated() {
                Animation::Keyframed(upsert_key(slot, ctx.lt, value))
            } else {
                Animation::Static(value)
            };
            *pending = Some(lumit_core::Op::SetTransformProperty {
                comp: ctx.comp_id,
                layer: ctx.layer.id,
                prop,
                animation,
            });
        }
        app.prop_edit = None;
    }
}

/// A Batch op setting two transform properties as one undo step — how every
/// linked two-axis row (Scale, Position, Anchor) commits both axes together.
pub(crate) fn two_prop_batch(
    comp: uuid::Uuid,
    layer: uuid::Uuid,
    x: (
        lumit_core::model::TransformProp,
        lumit_core::anim::Animation,
    ),
    y: (
        lumit_core::model::TransformProp,
        lumit_core::anim::Animation,
    ),
) -> lumit_core::Op {
    lumit_core::Op::Batch {
        ops: vec![
            lumit_core::Op::SetTransformProperty {
                comp,
                layer,
                prop: x.0,
                animation: x.1,
            },
            lumit_core::Op::SetTransformProperty {
                comp,
                layer,
                prop: y.0,
                animation: y.1,
            },
        ],
    }
}

/// Sorted key times (seconds, layer-local) across both axes of a linked row,
/// de-duplicated within `tol` — the navigator and its diamond work on this
/// union, so a key on either axis counts.
pub(crate) fn union_key_times(
    a: &lumit_core::anim::Property,
    b: &lumit_core::anim::Property,
    tol: f64,
) -> Vec<f64> {
    use lumit_core::anim::Animation;
    let mut times: Vec<f64> = Vec::new();
    for slot in [a, b] {
        if let Animation::Keyframed(keys) = &slot.animation {
            times.extend(keys.iter().map(|k| k.time.to_f64()));
        }
    }
    times.sort_by(f64::total_cmp);
    times.dedup_by(|p, q| (*p - *q).abs() < tol);
    times
}

/// Where a navigator can go from local time `lt` over sorted key `times`:
/// (previous key time, whether a key sits at the playhead, next key time).
/// The half-frame tolerance matches `keyframe_nav`.
pub(crate) fn key_nav_targets(
    times: &[f64],
    lt: f64,
    tol: f64,
) -> (Option<f64>, bool, Option<f64>) {
    let prev = times.iter().rev().find(|t| **t < lt - tol).copied();
    let on_key = times.iter().any(|t| (t - lt).abs() < tol);
    let next = times.iter().find(|t| **t > lt + tol).copied();
    (prev, on_key, next)
}

/// One axis's share of the linked row's diamond click. Removing strips this
/// axis's keys at the playhead — freezing the axis to its current value if
/// none remain, leaving a Static axis untouched. Adding upserts a key at the
/// playhead with the axis's current value, so both axes always key together.
pub(crate) fn toggle_key_at(
    slot: &lumit_core::anim::Property,
    lt: f64,
    tol: f64,
    remove: bool,
) -> lumit_core::anim::Animation {
    use lumit_core::anim::Animation;
    if !remove {
        return Animation::Keyframed(upsert_key(slot, lt, slot.value_at(lt)));
    }
    match &slot.animation {
        Animation::Keyframed(keys) => {
            let kept: Vec<_> = keys
                .iter()
                .filter(|k| (k.time.to_f64() - lt).abs() >= tol)
                .cloned()
                .collect();
            if kept.is_empty() {
                Animation::Static(slot.value_at(lt))
            } else {
                Animation::Keyframed(kept)
            }
        }
        Animation::Static(v) => Animation::Static(*v),
    }
}

/// The combined "Scale %" row (ratio locked): edits both axes keeping the
/// ratio, with a chain-link button to unlink. Sets `*unlinked` = true when
/// unlinked.
pub(crate) fn combined_scale_row(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    pending: &mut Option<lumit_core::Op>,
    unlinked: &mut bool,
) {
    use lumit_core::anim::Animation;
    use lumit_core::model::TransformProp;
    let sx = ctx.layer.transform.get(TransformProp::ScaleX);
    let sy = ctx.layer.transform.get(TransformProp::ScaleY);
    let is_graphed = app.selected_layer == Some(ctx.layer.id)
        && !app.graph_retime
        && matches!(
            app.graph_prop,
            Some(TransformProp::ScaleX | TransformProp::ScaleY)
        );
    let (row_rect, mut c) = row_frame(ui, ctx, is_graphed);

    // Stopwatch drives both axes together (drawn, like every other row).
    let animated = sx.is_animated() || sy.is_animated();
    let hover = if animated {
        "Remove animation"
    } else {
        "Animate both scale axes"
    };
    if stopwatch_button(&mut c, ctx.theme, animated, hover) {
        let (ax, ay) = if animated {
            (
                Animation::Static(sx.value_at(ctx.lt)),
                Animation::Static(sy.value_at(ctx.lt)),
            )
        } else {
            (
                Animation::Keyframed(upsert_key(sx, ctx.lt, sx.value_at(ctx.lt))),
                Animation::Keyframed(upsert_key(sy, ctx.lt, sy.value_at(ctx.lt))),
            )
        };
        *pending = Some(two_prop_batch(
            ctx.comp_id,
            ctx.layer.id,
            (TransformProp::ScaleX, ax),
            (TransformProp::ScaleY, ay),
        ));
    }
    if c.add(
        egui::Label::new(egui::RichText::new("Scale %").small().color(if is_graphed {
            ctx.theme.accent
        } else {
            ctx.theme.text_muted
        }))
        .sense(egui::Sense::click()),
    )
    .clicked()
    {
        app.selected_layer = Some(ctx.layer.id);
        app.graph_prop = Some(TransformProp::ScaleX);
        app.graph_retime = false; // switching to a transform property
        app.graph_reset_fit(); // a fresh channel starts fitted
    }
    if icon_button(&mut c, ctx.theme, Icon::Link, true)
        .on_hover_text("Unlink scale (edit x and y separately)")
        .clicked()
    {
        *unlinked = true;
    }
    {
        let old_x = sx.value_at(ctx.lt);
        let old_y = sy.value_at(ctx.lt);
        let mut value = match app.prop_edit {
            Some((l, p, v)) if l == ctx.layer.id && p == TransformProp::ScaleX => v,
            _ => old_x,
        };
        let resp = c.add(egui::DragValue::new(&mut value).speed(0.5).max_decimals(2));
        if resp.dragged() || resp.has_focus() {
            app.prop_edit = Some((ctx.layer.id, TransformProp::ScaleX, value));
            // Both axes move together, so the live preview needs both (else it
            // shows only x scaling until release).
            let (nx, ny) = linked_scale(old_x, old_y, value);
            app.scale_preview = Some((ctx.layer.id, nx, ny));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            if (value - old_x).abs() > f64::EPSILON {
                let (nx, ny) = linked_scale(old_x, old_y, value);
                let ax = if sx.is_animated() {
                    Animation::Keyframed(upsert_key(sx, ctx.lt, nx))
                } else {
                    Animation::Static(nx)
                };
                let ay = if sy.is_animated() {
                    Animation::Keyframed(upsert_key(sy, ctx.lt, ny))
                } else {
                    Animation::Static(ny)
                };
                *pending = Some(two_prop_batch(
                    ctx.comp_id,
                    ctx.layer.id,
                    (TransformProp::ScaleX, ax),
                    (TransformProp::ScaleY, ay),
                ));
            }
            app.prop_edit = None;
            app.scale_preview = None;
        }
    }
    // Track: the union of both axes' keys.
    let mut keys: Vec<lumit_core::anim::Keyframe> = Vec::new();
    for slot in [sx, sy] {
        if let Animation::Keyframed(k) = &slot.animation {
            keys.extend(k.iter().cloned());
        }
    }
    draw_key_diamonds(ui, ctx, row_rect, &keys);
}

/// A thin row holding a relink button ("Link scale", "Link position", …);
/// true when clicked.
pub(crate) fn link_toggle_row(ui: &mut egui::Ui, ctx: &RowCtx, label: &str, hover: &str) -> bool {
    let (_row_rect, mut c) = row_frame(ui, ctx, false);
    let clicked = icon_button(&mut c, ctx.theme, Icon::Link, false)
        .on_hover_text(hover)
        .clicked();
    c.label(
        egui::RichText::new(label)
            .small()
            .color(ctx.theme.text_muted),
    );
    clicked
}

/// One linked Anchor/Position row: two independent value boxes (x then y) on
/// a single row, AE-style. Unlike Scale's ratio lock, nothing couples the
/// values — the link only merges the row furniture: one stopwatch animates or
/// freezes both axes as one undo step, one navigator walks the union of both
/// axes' keys (its diamond keys or clears both at the playhead), the name
/// graphs the x channel, and the lane shows both axes' diamonds. The chain
/// button sets `*unlinked` = true to split into separate rows.
pub(crate) fn linked_pair_row(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    label: &str,
    props: (
        lumit_core::model::TransformProp,
        lumit_core::model::TransformProp,
    ),
    pending: &mut Option<lumit_core::Op>,
    unlinked: &mut bool,
) {
    use lumit_core::anim::Animation;
    let (px, py) = props;
    let sx = ctx.layer.transform.get(px);
    let sy = ctx.layer.transform.get(py);
    let lower = label.to_lowercase();
    let is_graphed = app.selected_layer == Some(ctx.layer.id)
        && !app.graph_retime
        && (app.graph_prop == Some(px) || app.graph_prop == Some(py));
    let (row_rect, mut c) = row_frame(ui, ctx, is_graphed);

    // Stopwatch drives both axes together as one undo step.
    let animated = sx.is_animated() || sy.is_animated();
    let hover = if animated {
        "Remove animation".to_owned()
    } else {
        format!("Animate both {lower} axes")
    };
    if stopwatch_button(&mut c, ctx.theme, animated, &hover) {
        let (ax, ay) = if animated {
            (
                Animation::Static(sx.value_at(ctx.lt)),
                Animation::Static(sy.value_at(ctx.lt)),
            )
        } else {
            (
                Animation::Keyframed(upsert_key(sx, ctx.lt, sx.value_at(ctx.lt))),
                Animation::Keyframed(upsert_key(sy, ctx.lt, sy.value_at(ctx.lt))),
            )
        };
        *pending = Some(two_prop_batch(
            ctx.comp_id,
            ctx.layer.id,
            (px, ax),
            (py, ay),
        ));
    }

    // The keyframe navigator over the union of both axes' keys: the arrows
    // jump to the nearest key on either axis; the diamond keys or clears both
    // axes at the playhead in one undo step.
    let tol = 0.5 / ctx.fps.max(1.0); // within half a frame counts as "on" it
    let times = union_key_times(sx, sy, tol);
    if !times.is_empty() {
        let (prev, on_key, next) = key_nav_targets(&times, ctx.lt, tol);
        let small = |i: Icon| egui::Button::new(crate::icons::text(i, 11.0)).frame(false);
        let mut jump_to: Option<f64> = None;
        if c.add_enabled(prev.is_some(), small(Icon::PrevKeyframe))
            .on_hover_text("Previous keyframe")
            .clicked()
        {
            jump_to = prev;
        }
        if c.add(small(if on_key {
            Icon::Keyframe
        } else {
            Icon::KeyframeAdd
        }))
        .on_hover_text(if on_key {
            "Remove keyframe here (both axes)"
        } else {
            "Add keyframe here (both axes)"
        })
        .clicked()
        {
            *pending = Some(two_prop_batch(
                ctx.comp_id,
                ctx.layer.id,
                (px, toggle_key_at(sx, ctx.lt, tol, on_key)),
                (py, toggle_key_at(sy, ctx.lt, tol, on_key)),
            ));
        }
        if c.add_enabled(next.is_some(), small(Icon::NextKeyframe))
            .on_hover_text("Next keyframe")
            .clicked()
        {
            jump_to = next;
        }
        if let Some(kt) = jump_to {
            app.preview_frame = ((kt + ctx.off) * ctx.fps).round().max(0.0) as usize;
            #[cfg(feature = "media")]
            app.refresh_preview();
        }
    }

    // The name graphs the x channel (like Scale graphs ScaleX).
    if c.add(
        egui::Label::new(egui::RichText::new(label).small().color(if is_graphed {
            ctx.theme.accent
        } else {
            ctx.theme.text_muted
        }))
        .sense(egui::Sense::click()),
    )
    .clicked()
    {
        app.selected_layer = Some(ctx.layer.id);
        app.graph_prop = Some(px);
        app.graph_retime = false; // switching to a transform property
        app.graph_view_y = None; // re-fit for the newly graphed channel
    }
    if icon_button(&mut c, ctx.theme, Icon::Link, true)
        .on_hover_text(format!("Unlink {lower} (x and y on separate rows)"))
        .clicked()
    {
        *unlinked = true;
    }
    // Two independent value boxes: x then y, each editing only its own axis.
    axis_drag_value(&mut c, app, ctx, px, 1.0, pending);
    axis_drag_value(&mut c, app, ctx, py, 1.0, pending);

    // Lane: the union of both axes' keys.
    let mut keys: Vec<lumit_core::anim::Keyframe> = Vec::new();
    for slot in [sx, sy] {
        if let Animation::Keyframed(k) = &slot.animation {
            keys.extend(k.iter().cloned());
        }
    }
    draw_key_diamonds(ui, ctx, row_rect, &keys);
}

/// Insert or replace a speed keyframe at local time `lt` (seconds) with `speed`
/// (1.0 = 100%), keeping the [0, dur] endpoints, and rebuild the retime store.
pub(crate) fn speed_with_key(
    retime: &Option<lumit_core::retime::Retime>,
    dur: lumit_core::Rational,
    lt: f64,
    speed: lumit_core::Rational,
) -> Option<lumit_core::retime::Retime> {
    use lumit_core::Rational;
    let mut keys = retime
        .as_ref()
        .and_then(|r| r.speed_keyframes())
        .unwrap_or_else(|| vec![(Rational::ZERO, Rational::ONE), (dur, Rational::ONE)]);
    let t = Rational::from_f64_on_grid(lt.clamp(0.0, dur.to_f64()), 1000).unwrap_or(Rational::ZERO);
    if let Some(k) = keys.iter_mut().find(|k| k.0 == t) {
        k.1 = speed;
    } else {
        keys.push((t, speed));
        keys.sort_by_key(|k| k.0);
    }
    lumit_core::retime::Retime::from_speed_keyframes(Rational::ZERO, &keys)
}

/// The source footage frame rate for a layer's Retime value-lens timecode: the
/// probed footage fps when the media is present, else `fallback_fps` (the comp
/// rate). Frames then count within a second at the *footage's* own rate — a
/// 600 fps clip in a 30 fps comp still reads frames 0..599 — matching the graph
/// editor's value lens.
#[cfg(feature = "media")]
pub(crate) fn layer_source_fps(
    app: &AppState,
    layer: &lumit_core::model::Layer,
    fallback_fps: f64,
) -> f64 {
    use lumit_core::model::LayerKind;
    if let LayerKind::Footage { item, .. } = &layer.kind {
        if let Some(crate::app_state::media::MediaStatus::Ready { probe, .. }) =
            app.media.map.get(item)
        {
            if let Some(v) = probe.video.as_ref() {
                return v.fps();
            }
        }
    }
    fallback_fps
}
#[cfg(not(feature = "media"))]
pub(crate) fn layer_source_fps(
    _app: &AppState,
    _layer: &lumit_core::model::Layer,
    fallback_fps: f64,
) -> f64 {
    fallback_fps
}

/// Where a retimed footage layer runs out of source, as a comp-time span in
/// seconds: from the exhaustion point to the layer's out point, clamped to the
/// layer's visible span (a source that dies before the in point holds for the
/// whole bar). `None` when the source lasts to the out point, or runs out only
/// past it. Indication only — overrun never moves a boundary (K-022).
/// (Only the media-gated timeline drawing calls this, but the maths is
/// feature-free, so it stays compiled and tested in every build.)
#[cfg_attr(not(feature = "media"), allow(dead_code))]
pub(crate) fn overrun_span_secs(
    retime: &lumit_core::retime::Retime,
    source_duration_secs: f64,
    start_offset_secs: f64,
    in_point_secs: f64,
    out_point_secs: f64,
) -> Option<(f64, f64)> {
    let local = retime.overrun_local_time(rational_at(source_duration_secs))?;
    let start = (start_offset_secs + local).max(in_point_secs);
    (start < out_point_secs).then_some((start, out_point_secs))
}

/// Insert or replace a value keyframe (local time → source time) at the playhead
/// `lt`, keeping the list sorted and its times unique. Local time snaps to the
/// comp frame grid (`comp_fps`, the playhead's own rate); source time snaps to
/// the footage frame grid (`src_fps`), so keys land on real source frames. Used
/// by the Retime value lens.
pub(crate) fn upsert_value_key(
    keys: &mut Vec<(lumit_core::Rational, lumit_core::Rational)>,
    lt: f64,
    src: f64,
    dur: lumit_core::Rational,
    comp_fps: f64,
    src_fps: f64,
) {
    use lumit_core::Rational;
    let gt = comp_fps.round().max(1.0) as i64;
    let gs = src_fps.round().max(1.0) as i64;
    let t = Rational::from_f64_on_grid(lt.clamp(0.0, dur.to_f64()), gt).unwrap_or(Rational::ZERO);
    let s = Rational::from_f64_on_grid(src.max(0.0), gs).unwrap_or(Rational::ZERO);
    if let Some(k) = keys.iter_mut().find(|k| k.0 == t) {
        k.1 = s;
    } else {
        keys.push((t, s));
        keys.sort_by_key(|k| k.0);
    }
}

/// The footage speed as a full-width, keyframable property row (K-072): a
/// stopwatch toggles keyframing; editing sets a constant speed or, once
/// animated, a speed keyframe at the playhead; keys show on the track. Linear
/// speed ramps read back as keyframes; smooth-eased ramps live in the graph
/// editor and here read as a constant. Clicking the name graphs the Retime
/// speed channel (K-075), like clicking a transform property's name.
pub(crate) fn speed_property_row(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    retime: &Option<lumit_core::retime::Retime>,
    pending: &mut Option<lumit_core::Op>,
) {
    use lumit_core::Rational;
    let dur = ctx.layer.out_point.0;
    let keys = retime.as_ref().and_then(|r| r.speed_keyframes());
    // Animated = an internal key, or differing endpoint speeds (a ramp).
    let animated = keys
        .as_ref()
        .is_some_and(|k| k.len() > 2 || k.first().map(|f| f.1) != k.last().map(|l| l.1));
    let current = retime
        .as_ref()
        .map(|r| r.speed_at(ctx.lt) * 100.0)
        .unwrap_or(100.0);
    let to_speed =
        |pct: f64| Rational::from_f64_on_grid(pct / 100.0, 1000).unwrap_or(Rational::ONE);

    let is_graphed = app.selected_layer == Some(ctx.layer.id) && app.graph_retime;
    let (row_rect, mut c) = row_frame(ui, ctx, is_graphed);

    // The Retime channel wears two lenses (K-076). The Velocity lens keyframes
    // speed (percentages); the Time lens keyframes the source time on screen (a
    // timecode) — AE's Time Remap. Which one is live follows the graph lens.
    let speed_lens = app.graph_speed_view;
    let fps = ctx.fps;
    // The value lens reads and keys the source time at the footage's own frame
    // rate, not the comp's, so a 600 fps clip counts frames 0..599.
    let src_fps = layer_source_fps(app, ctx.layer, ctx.fps);
    // Value-lens state: the source time at the playhead, and the value keys
    // (every boundary). The Time stopwatch is ON as soon as any retime exists —
    // like AE's Time Remap, enabling it always yields at least the start/end
    // pair, and enabling at the very start or end of the layer simply re-pins
    // an endpoint key (it must not read as "nothing happened").
    let value_keys = retime.as_ref().map(|r| r.value_keyframes());
    let time_enabled = retime.is_some();
    let src_now = retime
        .as_ref()
        .map(|r| r.evaluate(ctx.lt))
        .unwrap_or(ctx.lt);

    // Stopwatch — speed key or value key, per the live lens.
    if speed_lens {
        let hover = if animated {
            "Freeze speed (constant at the current value)"
        } else {
            "Animate speed: keyframe at the playhead"
        };
        if stopwatch_button(&mut c, ctx.theme, animated, hover) {
            let new_retime = if animated {
                if (current - 100.0).abs() < 1e-6 {
                    None
                } else {
                    Some(lumit_core::retime::Retime::constant_speed(
                        dur,
                        Rational::ZERO,
                        to_speed(current),
                    ))
                }
            } else {
                speed_with_key(retime, dur, ctx.lt, to_speed(current))
            };
            *pending = Some(lumit_core::Op::SetLayerRetime {
                comp: ctx.comp_id,
                layer: ctx.layer.id,
                retime: new_retime,
            });
        }
    } else {
        let hover = if time_enabled {
            "Remove time keyframes (pass the source straight through)"
        } else {
            "Keyframe time: source time at the playhead"
        };
        if stopwatch_button(&mut c, ctx.theme, time_enabled, hover) {
            let new_retime = if time_enabled {
                None
            } else {
                let mut keys = value_keys
                    .clone()
                    .unwrap_or_else(|| vec![(Rational::ZERO, Rational::ZERO), (dur, dur)]);
                upsert_value_key(&mut keys, ctx.lt, src_now, dur, fps, src_fps);
                lumit_core::retime::Retime::from_value_keyframes(&keys)
            };
            *pending = Some(lumit_core::Op::SetLayerRetime {
                comp: ctx.comp_id,
                layer: ctx.layer.id,
                retime: new_retime,
            });
        }
    }

    // Keyframe navigator, like every other property row — shown once the
    // channel is keyed. The arrows jump the playhead between this lens's keys;
    // the keyframe button adds a key at the playhead, or removes an interior
    // one (the structural start/end keys stay, shown disabled). Iconoir glyphs
    // (K-085) — the old ◄ ◆ ► characters weren't in the UI fonts.
    let nav_on = if speed_lens { animated } else { time_enabled };
    if nav_on {
        let tol = 0.5 / fps.max(1.0);
        let small = |i: Icon| egui::Button::new(crate::icons::text(i, 11.0)).frame(false);
        let key_times: Vec<f64> = if speed_lens {
            keys.as_ref()
                .map(|k| k.iter().map(|(t, _)| t.to_f64()).collect())
                .unwrap_or_default()
        } else {
            value_keys
                .as_ref()
                .map(|k| k.iter().map(|(t, _)| t.to_f64()).collect())
                .unwrap_or_default()
        };
        let mut jump_to: Option<f64> = None;

        let has_prev = key_times.iter().any(|&t| t < ctx.lt - tol);
        if c.add_enabled(has_prev, small(Icon::PrevKeyframe))
            .on_hover_text("Previous keyframe")
            .clicked()
        {
            jump_to = key_times
                .iter()
                .copied()
                .filter(|&t| t < ctx.lt - tol)
                .reduce(f64::max);
        }

        let on_key = key_times.iter().any(|&t| (t - ctx.lt).abs() < tol);
        let at_endpoint = ctx.lt <= tol || (dur.to_f64() - ctx.lt).abs() < tol;
        let removable = on_key && !at_endpoint;
        let diamond = c
            .add_enabled(
                !on_key || removable,
                small(if on_key {
                    Icon::Keyframe
                } else {
                    Icon::KeyframeAdd
                }),
            )
            .on_hover_text(if on_key {
                "Remove keyframe here"
            } else {
                "Add keyframe here"
            });
        if diamond.clicked() {
            let new_retime = if speed_lens {
                if on_key {
                    let kept: Vec<(Rational, Rational)> = keys
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(t, _)| (t.to_f64() - ctx.lt).abs() >= tol)
                        .collect();
                    lumit_core::retime::Retime::from_speed_keyframes(Rational::ZERO, &kept)
                } else {
                    speed_with_key(retime, dur, ctx.lt, to_speed(current))
                }
            } else if on_key {
                let mut kv = value_keys.clone().unwrap_or_default();
                kv.retain(|(t, _)| (t.to_f64() - ctx.lt).abs() >= tol);
                lumit_core::retime::Retime::from_value_keyframes(&kv)
            } else {
                let mut kv = value_keys
                    .clone()
                    .unwrap_or_else(|| vec![(Rational::ZERO, Rational::ZERO), (dur, dur)]);
                upsert_value_key(&mut kv, ctx.lt, src_now, dur, fps, src_fps);
                lumit_core::retime::Retime::from_value_keyframes(&kv)
            };
            *pending = Some(lumit_core::Op::SetLayerRetime {
                comp: ctx.comp_id,
                layer: ctx.layer.id,
                retime: new_retime,
            });
        }

        let has_next = key_times.iter().any(|&t| t > ctx.lt + tol);
        if c.add_enabled(has_next, small(Icon::NextKeyframe))
            .on_hover_text("Next keyframe")
            .clicked()
        {
            jump_to = key_times
                .iter()
                .copied()
                .filter(|&t| t > ctx.lt + tol)
                .reduce(f64::min);
        }

        if let Some(kt) = jump_to {
            app.preview_frame = ((kt + ctx.off) * ctx.fps).round().max(0.0) as usize;
            #[cfg(feature = "media")]
            app.refresh_preview();
        }
    }

    // "Time" in the value lens, "Velocity" in the derivative lens (K-076).
    let channel_name = if speed_lens { "Velocity" } else { "Time" };
    if c.add(
        egui::Label::new(
            egui::RichText::new(channel_name)
                .small()
                .color(if is_graphed {
                    ctx.theme.accent
                } else {
                    ctx.theme.text_muted
                }),
        )
        .sense(egui::Sense::click()),
    )
    .clicked()
    {
        app.selected_layer = Some(ctx.layer.id);
        app.graph_retime = true; // graph the Retime channel (K-075)
        app.graph_reset_fit(); // a fresh channel starts fitted
        app.graph_speed_view = app.vegas_default_lens; // open to the preferred lens
    }

    // Value widget — a speed percentage, or a source timecode.
    if speed_lens {
        // Temp-value pattern: a drag is one commit.
        let id = egui::Id::new(("speedv", ctx.layer.id));
        let mut v = c.data(|d| d.get_temp::<f64>(id)).unwrap_or(current);
        let resp = c.add(
            egui::DragValue::new(&mut v)
                .speed(1.0)
                .range(-800.0..=800.0)
                .suffix(" %"),
        );
        if resp.dragged() || resp.has_focus() {
            c.data_mut(|d| d.insert_temp(id, v));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            if (v - current).abs() > 1e-6 {
                let new_retime = if animated {
                    speed_with_key(retime, dur, ctx.lt, to_speed(v))
                } else if (v - 100.0).abs() < 1e-6 {
                    None
                } else {
                    Some(lumit_core::retime::Retime::constant_speed(
                        dur,
                        Rational::ZERO,
                        to_speed(v),
                    ))
                };
                *pending = Some(lumit_core::Op::SetLayerRetime {
                    comp: ctx.comp_id,
                    layer: ctx.layer.id,
                    retime: new_retime,
                });
            }
            c.data_mut(|d| d.remove::<f64>(id));
        }
    } else {
        // Source time as an editable HH:MM:SS:FF timecode, scrubbed in source
        // frames at the footage's own rate.
        let id = egui::Id::new(("timev", ctx.layer.id));
        let mut frames = c
            .data(|d| d.get_temp::<f64>(id))
            .unwrap_or(src_now * src_fps);
        let resp = c.add(
            egui::DragValue::new(&mut frames)
                .speed(1.0)
                .range(0.0..=1.0e9)
                .custom_formatter(move |n, _| fmt_timecode_frames(n / src_fps, src_fps))
                .custom_parser(move |s| parse_timecode_frames(s, src_fps)),
        );
        if resp.dragged() || resp.has_focus() {
            c.data_mut(|d| d.insert_temp(id, frames));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            let new_src = frames.max(0.0) / src_fps;
            // Editing the time keyframes it (AE Time Remap), seeding endpoints
            // from the current curve when none exist yet.
            if (new_src - src_now).abs() > 0.5 / src_fps {
                let mut keys = value_keys
                    .clone()
                    .unwrap_or_else(|| vec![(Rational::ZERO, Rational::ZERO), (dur, dur)]);
                upsert_value_key(&mut keys, ctx.lt, new_src, dur, fps, src_fps);
                *pending = Some(lumit_core::Op::SetLayerRetime {
                    comp: ctx.comp_id,
                    layer: ctx.layer.id,
                    retime: lumit_core::retime::Retime::from_value_keyframes(&keys),
                });
            }
            c.data_mut(|d| d.remove::<f64>(id));
        }
    }

    // Track: keyframes as diamonds — speed keys, or value keys (boundaries).
    let track_keys: Option<Vec<lumit_core::Rational>> = if speed_lens {
        animated
            .then(|| keys.as_ref().map(|k| k.iter().map(|(t, _)| *t).collect()))
            .flatten()
    } else {
        // Time lens: every boundary is a key, endpoints included — a freshly
        // enabled channel shows its first and last keys straight away.
        time_enabled.then(|| {
            value_keys
                .as_ref()
                .map(|k| k.iter().map(|(t, _)| *t).collect())
                .unwrap_or_default()
        })
    };
    if let Some(times) = track_keys {
        let kf: Vec<lumit_core::anim::Keyframe> = times
            .iter()
            .map(|t| lumit_core::anim::Keyframe {
                time: *t,
                value: 0.0,
                interp_in: lumit_core::anim::SideInterp::Linear,
                interp_out: lumit_core::anim::SideInterp::Linear,
            })
            .collect();
        draw_key_diamonds(ui, ctx, row_rect, &kf);
    }
}

/// The Effects group's rows (docs/08): an "Add effect" menu, then one block
/// per effect — bypass / name / remove on its title row, one row per
/// parameter beneath. Float parameters are fully animatable (stopwatch +
/// key diamonds on the lane, like any transform property); every change
/// commits one whole-stack SetLayerEffects, so each edit is one undo step.
pub(crate) fn effects_rows(
    ui: &mut egui::Ui,
    ctx: &RowCtx,
    pending: &mut Option<lumit_core::Op>,
    // Set to (layer, effect index, param index, provisional value) while a
    // Float effect parameter is being dragged, so the caller can drive a live
    // preview (`AppState::fx_edit`) without committing until release.
    fx_edit: &mut Option<(uuid::Uuid, usize, usize, f64)>,
) {
    use lumit_core::fx::{self, ParamKind};
    use lumit_core::model::{EffectValue, FileParam};
    let layer = ctx.layer;
    let commit =
        |effects: Vec<lumit_core::model::EffectInstance>| lumit_core::Op::SetLayerEffects {
            comp: ctx.comp_id,
            layer: layer.id,
            effects,
        };

    // The add row.
    {
        let (_row, mut c) = row_frame(ui, ctx, false);
        c.menu_button(
            egui::RichText::new("Add effect")
                .small()
                .color(ctx.theme.text_secondary),
            |ui| {
                // Grouped by category (K-090); empty categories don't show.
                for cat in fx::FxCategory::ALL {
                    let members: Vec<_> =
                        fx::BUILTINS.iter().filter(|s| s.category == cat).collect();
                    if members.is_empty() {
                        continue;
                    }
                    ui.menu_button(cat.label(), |ui| {
                        for schema in members {
                            if ui.button(schema.label).clicked() {
                                if let Some(inst) = fx::instantiate(schema.match_name) {
                                    let mut effects = layer.effects.clone();
                                    effects.push(inst);
                                    *pending = Some(commit(effects));
                                }
                                ui.close_menu();
                            }
                        }
                    });
                }
            },
        );
        // Preset save/load (docs/07-UI-SPEC §7, K-065): save the whole stack
        // to a `.lumfx` file, or load one and append it to this layer.
        c.menu_button(
            egui::RichText::new("Presets")
                .small()
                .color(ctx.theme.text_secondary),
            |ui| {
                ui.add_enabled_ui(!layer.effects.is_empty(), |ui| {
                    if ui.button("Save stack as preset…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name(format!("effects.{}", crate::preset::PRESET_EXTENSION))
                            .add_filter("Lumit effect preset", &[crate::preset::PRESET_EXTENSION])
                            .save_file()
                        {
                            let name = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("preset")
                                .to_owned();
                            if let Ok(json) = crate::preset::to_json(&name, &layer.effects) {
                                // Best-effort: a failed write leaves the
                                // document untouched (never an edit).
                                let _ = std::fs::write(&path, json);
                            }
                        }
                        ui.close_menu();
                    }
                });
                if ui.button("Load preset…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Lumit effect preset", &[crate::preset::PRESET_EXTENSION])
                        .pick_file()
                    {
                        if let Ok(preset) = std::fs::read_to_string(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|t| crate::preset::from_json(&t))
                        {
                            // Append the preset's effects (fresh ids) to the
                            // stack — one undoable SetLayerEffects.
                            let mut effects = layer.effects.clone();
                            effects.extend(crate::preset::instantiated(&preset));
                            *pending = Some(commit(effects));
                        }
                    }
                    ui.close_menu();
                }
            },
        );
    }

    // Reorder-by-drag bookkeeping (docs/07 §6): each effect's title-row rect, so a
    // drop can be resolved against every effect after the loop. `fx_drag_id` holds
    // the live drag (source index + pointer y) in ui temp so it survives frames.
    let fx_drag_id = ui.id().with(("fx-reorder", layer.id));
    let fx_dragging: Option<(usize, f32)> = ui.data(|d| d.get_temp(fx_drag_id));
    let mut fx_title_rows: Vec<egui::Rect> = Vec::new();
    let mut fx_reorder_release: Option<(usize, f32)> = None;
    for (idx, e) in layer.effects.iter().enumerate() {
        let schema = fx::schema(&e.effect.match_name);
        // Title row: bypass, name (dimmed when bypassed), remove — sitting in a
        // subtle full-width bar so each effect's start is obvious (Mack). The name
        // is a drag handle: dragging it up or down reorders the stack (one
        // SetLayerEffects, so one undo step).
        {
            let (row_rect, mut c) = row_frame(ui, ctx, false);
            section_bar(ui, ctx, row_rect);
            fx_title_rows.push(row_rect);
            // The per-effect visibility toggle (K-090 confirmation of §1.5):
            // the same eye as layer visibility, dimmed while bypassed.
            let eye_col = if e.enabled {
                ctx.theme.text_secondary
            } else {
                ctx.theme.text_disabled
            };
            let (eye_rect, eye_resp) =
                c.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
            crate::icons::paint(c.painter(), eye_rect, Icon::Eye, eye_col, 1.4);
            if eye_resp
                .on_hover_text(if e.enabled {
                    "Bypass this effect"
                } else {
                    "Enable this effect"
                })
                .clicked()
            {
                let mut effects = layer.effects.clone();
                effects[idx].enabled = !e.enabled;
                *pending = Some(commit(effects));
            }
            let name = schema.map_or(e.effect.match_name.as_str(), |s| s.label);
            let colour = if e.enabled {
                ctx.theme.text_secondary
            } else {
                ctx.theme.text_disabled
            };
            // The name doubles as the reorder handle: a frameless click-and-drag
            // button (not a Label, so dragging never highlights its characters).
            let name_resp = c
                .add(
                    egui::Button::new(egui::RichText::new(name).small().color(colour))
                        .frame(false)
                        .truncate()
                        .sense(egui::Sense::click_and_drag()),
                )
                .on_hover_text("Drag to reorder");
            if name_resp.dragged() {
                if let Some(p) = name_resp.interact_pointer_pos() {
                    c.data_mut(|d| d.insert_temp(fx_drag_id, (idx, p.y)));
                }
                c.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if name_resp.drag_stopped() {
                let y = fx_dragging
                    .filter(|(i, _)| *i == idx)
                    .map(|(_, y)| y)
                    .or_else(|| name_resp.interact_pointer_pos().map(|p| p.y));
                if let Some(y) = y {
                    fx_reorder_release = Some((idx, y));
                }
                c.data_mut(|d| d.remove::<(usize, f32)>(fx_drag_id));
            }
            if c.small_button("\u{00d7}")
                .on_hover_text("Remove this effect")
                .clicked()
            {
                let mut effects = layer.effects.clone();
                effects.remove(idx);
                *pending = Some(commit(effects));
            }
        }
        // One row per parameter, driven by the schema.
        let Some(schema) = schema else { continue };
        for (pi, param) in e.params.iter().enumerate() {
            let Some(ps) = schema.params.iter().find(|p| p.id == param.id) else {
                continue;
            };
            match (&param.value, ps.kind) {
                (EffectValue::Float(prop), ParamKind::Float { slider, hard, .. }) => {
                    let is_animated = prop.is_animated();
                    let (row_rect, mut c) = row_frame(ui, ctx, false);
                    if let Some(animation) = stopwatch(&mut c, ctx.theme, prop, ctx.lt) {
                        let mut effects = layer.effects.clone();
                        effects[idx].params[pi].value =
                            EffectValue::Float(lumit_core::anim::Property {
                                animation,
                                extra: serde_json::Map::new(),
                            });
                        *pending = Some(commit(effects));
                    }
                    c.label(
                        egui::RichText::new(ps.label)
                            .small()
                            .color(ctx.theme.text_muted),
                    );
                    let committed = prop.value_at(ctx.lt);
                    let id = egui::Id::new(("fxparam", e.id, pi));
                    let mut v = c.data(|d| d.get_temp::<f64>(id)).unwrap_or(committed);
                    let lo = hard.0.unwrap_or(f64::NEG_INFINITY);
                    let hi = hard.1.unwrap_or(f64::INFINITY);
                    let resp = c.add(
                        egui::DragValue::new(&mut v)
                            .speed((slider.1 - slider.0).abs().max(1.0) / 200.0)
                            .range(lo..=hi)
                            .max_decimals(2),
                    );
                    if resp.dragged() || resp.has_focus() {
                        c.data_mut(|d| d.insert_temp(id, v));
                        // Drive the live preview: re-run the effect stack with
                        // this provisional value each frame until release.
                        *fx_edit = Some((layer.id, idx, pi, v));
                    }
                    if resp.drag_stopped() || resp.lost_focus() {
                        if (v - committed).abs() > 1e-9 {
                            let mut effects = layer.effects.clone();
                            let animation = if is_animated {
                                lumit_core::anim::Animation::Keyframed(upsert_key(prop, ctx.lt, v))
                            } else {
                                lumit_core::anim::Animation::Static(v)
                            };
                            effects[idx].params[pi].value =
                                EffectValue::Float(lumit_core::anim::Property {
                                    animation,
                                    extra: serde_json::Map::new(),
                                });
                            *pending = Some(commit(effects));
                        }
                        c.data_mut(|d| d.remove::<f64>(id));
                    }
                    // Keys on the lane, like any property row.
                    if let lumit_core::anim::Animation::Keyframed(keys) = &prop.animation {
                        draw_key_diamonds(ui, ctx, row_rect, keys);
                    }
                }
                (EffectValue::Choice(cur), ParamKind::Choice { options, .. }) => {
                    let (_row, mut c) = row_frame(ui, ctx, false);
                    c.label(
                        egui::RichText::new(ps.label)
                            .small()
                            .color(ctx.theme.text_muted),
                    );
                    let cur_label = options.get(*cur as usize).copied().unwrap_or("?");
                    bare_dropdown(&mut c, egui::RichText::new(cur_label).small(), |ui| {
                        for (oi, opt) in options.iter().enumerate() {
                            if ui.selectable_label(oi as u32 == *cur, *opt).clicked() {
                                let mut effects = layer.effects.clone();
                                effects[idx].params[pi].value = EffectValue::Choice(oi as u32);
                                *pending = Some(commit(effects));
                                ui.close_menu();
                            }
                        }
                    });
                }
                (EffectValue::Bool(cur), ParamKind::Bool { .. }) => {
                    let (_row, mut c) = row_frame(ui, ctx, false);
                    let mut v = *cur;
                    if c.checkbox(&mut v, egui::RichText::new(ps.label).small())
                        .changed()
                    {
                        let mut effects = layer.effects.clone();
                        effects[idx].params[pi].value = EffectValue::Bool(v);
                        *pending = Some(commit(effects));
                    }
                }
                (EffectValue::Seed(cur), ParamKind::Seed) => {
                    // An integer drag plus the §2.4 reseed button; the
                    // chosen value is stored project data, so determinism
                    // is untouched.
                    let (_row, mut c) = row_frame(ui, ctx, false);
                    c.label(
                        egui::RichText::new(ps.label)
                            .small()
                            .color(ctx.theme.text_muted),
                    );
                    let id = egui::Id::new(("fxseed", e.id, pi));
                    let mut v = c.data(|d| d.get_temp::<u32>(id)).unwrap_or(*cur);
                    let resp = c.add(egui::DragValue::new(&mut v).speed(1));
                    if resp.dragged() || resp.has_focus() {
                        c.data_mut(|d| d.insert_temp(id, v));
                    }
                    if resp.drag_stopped() || resp.lost_focus() {
                        if v != *cur {
                            let mut effects = layer.effects.clone();
                            effects[idx].params[pi].value = EffectValue::Seed(v);
                            *pending = Some(commit(effects));
                        }
                        c.data_mut(|d| d.remove::<u32>(id));
                    }
                    if c.small_button("Reseed")
                        .on_hover_text("Pick a fresh seed")
                        .clicked()
                    {
                        let mut effects = layer.effects.clone();
                        effects[idx].params[pi].value =
                            EffectValue::Seed(lumit_core::fx::fresh_seed());
                        *pending = Some(commit(effects));
                    }
                }
                (EffectValue::Colour(chs), ParamKind::Colour { range, .. }) => {
                    // Scene-linear RGB drag values plus a live swatch (the
                    // swatch colour is data, not theme: it is the parameter).
                    // Channels are animatable in the model; the row edits
                    // static values for now, like Bool/Choice.
                    let (_row, mut c) = row_frame(ui, ctx, false);
                    c.label(
                        egui::RichText::new(ps.label)
                            .small()
                            .color(ctx.theme.text_muted),
                    );
                    let preview = egui::Rgba::from_rgb(
                        chs[0].value_at(ctx.lt).clamp(0.0, 1.0) as f32,
                        chs[1].value_at(ctx.lt).clamp(0.0, 1.0) as f32,
                        chs[2].value_at(ctx.lt).clamp(0.0, 1.0) as f32,
                    );
                    let (swatch, _) =
                        c.allocate_exact_size(egui::vec2(14.0, 10.0), egui::Sense::hover());
                    c.painter()
                        .rect_filled(swatch, 2.0, egui::Color32::from(preview));
                    for (ci, chan) in ["R", "G", "B"].iter().enumerate() {
                        let committed = chs[ci].value_at(ctx.lt);
                        let id = egui::Id::new(("fxcolour", e.id, pi, ci));
                        let mut v = c.data(|d| d.get_temp::<f64>(id)).unwrap_or(committed);
                        let resp = c.add(
                            egui::DragValue::new(&mut v)
                                .prefix(format!("{chan} "))
                                .speed(0.01)
                                .range(range.0..=range.1)
                                .max_decimals(3),
                        );
                        if resp.dragged() || resp.has_focus() {
                            c.data_mut(|d| d.insert_temp(id, v));
                        }
                        if resp.drag_stopped() || resp.lost_focus() {
                            if (v - committed).abs() > 1e-9 {
                                let mut effects = layer.effects.clone();
                                if let EffectValue::Colour(arr) = &mut effects[idx].params[pi].value
                                {
                                    arr[ci] = lumit_core::anim::Property::fixed(v);
                                }
                                *pending = Some(commit(effects));
                            }
                            c.data_mut(|d| d.remove::<f64>(id));
                        }
                    }
                }
                (
                    EffectValue::File(fp),
                    ParamKind::File {
                        filter,
                        filter_name,
                    },
                ) => {
                    // The file's basename plus a dialog button. The path is
                    // project data (the hold-keyed index picks it at this time);
                    // choosing a file replaces the path set with the one pick.
                    let (_row, mut c) = row_frame(ui, ctx, false);
                    c.label(
                        egui::RichText::new(ps.label)
                            .small()
                            .color(ctx.theme.text_muted),
                    );
                    let shown = fp
                        .path_at(ctx.lt)
                        .and_then(|p| std::path::Path::new(p).file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("No file");
                    c.label(
                        egui::RichText::new(shown)
                            .small()
                            .color(ctx.theme.text_secondary),
                    )
                    .on_hover_text(fp.path_at(ctx.lt).unwrap_or("No file selected"));
                    if c.small_button(format!("Select {filter_name}\u{2026}"))
                        .on_hover_text(format!("Choose a {filter_name} file"))
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter(filter_name, filter)
                            .pick_file()
                        {
                            if let Some(p) = path.to_str() {
                                let mut effects = layer.effects.clone();
                                effects[idx].params[pi].value =
                                    EffectValue::File(FileParam::single(p.to_owned()));
                                *pending = Some(commit(effects));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Resolve an effect reorder drag: the target slot is where the dropped
    // effect's centre lands among the other title rows (top = 0). A landing that
    // changes nothing commits nothing. One SetLayerEffects = one undo step.
    if let Some((from, y)) = fx_reorder_release {
        let target = fx_title_rows
            .iter()
            .enumerate()
            .filter(|(i, r)| *i != from && r.center().y < y)
            .count();
        if target != from && from < layer.effects.len() {
            let mut effects = layer.effects.clone();
            let moved = effects.remove(from);
            effects.insert(target.min(effects.len()), moved);
            *pending = Some(commit(effects));
        }
    }
    // While an effect name is being dragged, draw an accent insertion line at the
    // gap it would drop into, across the control column.
    if let Some((from, y)) = fx_dragging {
        if from < fx_title_rows.len() {
            let others: Vec<f32> = fx_title_rows
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != from)
                .map(|(_, r)| r.center().y)
                .collect();
            if !others.is_empty() {
                let target = others.iter().filter(|cy| **cy < y).count();
                let gap_y = if target == 0 {
                    others[0] - 9.0
                } else if target >= others.len() {
                    others[others.len() - 1] + 9.0
                } else {
                    (others[target - 1] + others[target]) * 0.5
                };
                let left = fx_title_rows[0].left();
                let right = (ctx.track_left - 6.0).max(left + 1.0);
                let mut p = ui.painter().clone();
                p.set_clip_rect(ctx.viewport);
                p.line_segment(
                    [egui::pos2(left, gap_y), egui::pos2(right, gap_y)],
                    egui::Stroke::new(2.0_f32, ctx.theme.accent),
                );
            }
        }
    }
}

pub(crate) fn mask_space(
    layer: &lumit_core::model::Layer,
    app: &AppState,
    comp: &lumit_core::model::Composition,
) -> (f64, f64) {
    match &layer.kind {
        // An adjustment layer is comp-sized: its masks live in comp space.
        lumit_core::model::LayerKind::Adjustment => (f64::from(comp.width), f64::from(comp.height)),
        lumit_core::model::LayerKind::Solid { def } => app
            .store
            .snapshot()
            .solid(*def)
            .map(|sd| (f64::from(sd.width), f64::from(sd.height)))
            .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
        lumit_core::model::LayerKind::Precomp { comp: nested } => app
            .store
            .snapshot()
            .comp(*nested)
            .map(|n| (f64::from(n.width), f64::from(n.height)))
            .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
        lumit_core::model::LayerKind::Camera { .. }
        | lumit_core::model::LayerKind::Sequence { .. }
        | lumit_core::model::LayerKind::Text { .. } => {
            (f64::from(comp.width), f64::from(comp.height))
        }
        #[cfg(feature = "media")]
        lumit_core::model::LayerKind::Footage { item, .. } => match app.media.map.get(item) {
            Some(crate::app_state::media::MediaStatus::Ready { probe, .. }) => probe
                .video
                .as_ref()
                .map(|v| (f64::from(v.width), f64::from(v.height)))
                .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
            _ => (f64::from(comp.width), f64::from(comp.height)),
        },
        #[cfg(not(feature = "media"))]
        lumit_core::model::LayerKind::Footage { .. } => {
            (f64::from(comp.width), f64::from(comp.height))
        }
    }
}

#[cfg(feature = "media")]
pub(crate) fn blend_of(b: lumit_core::model::BlendMode) -> lumit_gpu::Blend {
    use lumit_core::model::BlendMode;
    match b {
        BlendMode::Normal => lumit_gpu::Blend::Normal,
        BlendMode::Add => lumit_gpu::Blend::Add,
        BlendMode::Multiply => lumit_gpu::Blend::Multiply,
        BlendMode::Screen => lumit_gpu::Blend::Screen,
        BlendMode::Overlay => lumit_gpu::Blend::Overlay,
        BlendMode::SoftLight => lumit_gpu::Blend::SoftLight,
        BlendMode::HardLight => lumit_gpu::Blend::HardLight,
        BlendMode::Lighten => lumit_gpu::Blend::Lighten,
        BlendMode::Darken => lumit_gpu::Blend::Darken,
    }
}

/// Layer time → rational on the flick grid (the only f64→rational route).
pub(crate) fn rational_at(seconds: f64) -> lumit_core::Rational {
    lumit_core::Rational::from_f64_on_grid(seconds.max(0.0), lumit_core::Rational::FLICK_DEN)
        .unwrap_or(lumit_core::Rational::ZERO)
}

/// Insert or replace a keyframe at layer time `lt` with `value`, keeping the
/// list sorted and times unique (half-frame tolerance for "same time").
pub(crate) fn upsert_key(
    slot: &lumit_core::anim::Property,
    lt: f64,
    value: f64,
) -> Vec<lumit_core::anim::Keyframe> {
    use lumit_core::anim::{Animation, Keyframe, SideInterp};
    let mut keys = match &slot.animation {
        Animation::Keyframed(k) => k.clone(),
        Animation::Static(v) => vec![Keyframe {
            time: rational_at(0.0),
            value: *v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        }],
    };
    const EPS: f64 = 1.0 / 240.0;
    if let Some(existing) = keys.iter_mut().find(|k| (k.time.to_f64() - lt).abs() < EPS) {
        existing.value = value;
    } else {
        keys.push(Keyframe {
            time: rational_at(lt),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        });
        keys.sort_by_key(|k| k.time);
    }
    keys
}
