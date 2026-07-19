//! Sub-column switches on a layer's title line and the Flow group rows:
//! visibility, matte, blend, 3D, collapse, flow and mute controls.

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
    // Swap the glyph to a closed eye when hidden, not just dim it (note 2.8.4:
    // a toggle shows the state's matching icon, as Mute/Audio already do).
    let (icon, col) = if vis {
        (Icon::Eye, theme.text_secondary)
    } else {
        (Icon::EyeClosed, theme.text_disabled)
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    crate::icons::paint(ui.painter(), rect, icon, col, 1.4);
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
    use lumit_core::model::{LayerInputSource, MatteChannel, MatteRef};
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
                        source: layer.matte.map(|m| m.source).unwrap_or_default(),
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
                // Matte source (K-142): what of the matte layer the matte reads —
                // its raw picture, its masked picture, or its finished picture
                // (a keyed or blurred matte). Replaces the K-125 "After effects"
                // switch.
                ui.separator();
                ui.label(
                    egui::RichText::new("Source")
                        .small()
                        .color(theme.text_secondary),
                );
                for (mode, label, hint) in [
                    (
                        LayerInputSource::None,
                        "None",
                        "Gate with the matte layer's raw picture — no masks, no effects",
                    ),
                    (
                        LayerInputSource::Masks,
                        "Masks",
                        "Gate with the matte layer plus its masks, but not its effects",
                    ),
                    (
                        LayerInputSource::EffectsAndMasks,
                        "Effects and masks",
                        "Gate with the matte layer's finished picture — its effects and masks \
                         (a keyed or blurred matte)",
                    ),
                ] {
                    if ui
                        .selectable_label(m.source == mode, label)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        m.source = mode;
                        set = Some(Some(m));
                    }
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
        BlendMode::Subtract => "Subtract",
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
                BlendMode::Subtract,
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

/// 3D-switch subcolumn: a small square box, empty when flat and holding a
/// centred dot when the layer is 3D (note 2.8.5) — a clearer state read than the
/// old "3D" text toggle. The box corner follows the theme shape (crisp under
/// Sharp, softened under Round).
pub(crate) fn three_d_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    pending: &mut Option<lumit_core::Op>,
) {
    let on = layer.switches.three_d;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    let hovered = resp.hovered();
    let box_col = if on || hovered {
        theme.accent
    } else {
        theme.text_secondary
    };
    let side = 11.0;
    let sq = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
    let round = f32::from(theme.tokens.control_radius).min(side * 0.5);
    ui.painter().rect_stroke(
        sq,
        round,
        egui::Stroke::new(1.2_f32, box_col),
        egui::StrokeKind::Inside,
    );
    if on {
        ui.painter().circle_filled(rect.center(), 2.0, theme.accent);
    }
    if resp
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

/// Per-layer motion-blur subcolumn (K-120, docs/06 §4): a compact "MB" toggle in
/// the layer's far-right switch slot. Accent when on, secondary otherwise, and
/// bright under the cursor like the other switches. The hover note reminds that
/// it only shows once the comp's motion-blur master is on.
///
/// No Iconoir glyph in the [`Icon`] set means "motion blur", and the only
/// motion-adjacent one (Flow's wind) is already the flow column's icon — reusing
/// it would put two identical wind glyphs on the same footage row — so this reads
/// as a short label, like the Matte and Blend text switches.
pub(crate) fn motion_blur_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp_id: uuid::Uuid,
    layer: &lumit_core::model::Layer,
    pending: &mut Option<lumit_core::Op>,
) {
    let on = layer.switches.motion_blur;
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::click());
    let col = if on || resp.hovered() {
        theme.accent
    } else {
        theme.text_secondary
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "MB",
        egui::FontId::proportional(10.0),
        col,
    );
    if resp
        .on_hover_text(
            "Motion blur: smear this layer along its own motion (needs the comp's motion blur on)",
        )
        .clicked()
    {
        *pending = Some(lumit_core::Op::SetLayerMotionBlur {
            comp: comp_id,
            layer: layer.id,
            motion_blur: !on,
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
///
/// `nav_jump` carries the input-rate navigator's prev/next click out as a
/// layer-local time (the group holds no `AppState`, so the caller jumps the
/// playhead — the same routing the effect rows use).
pub(crate) fn flow_group_rows(
    ui: &mut egui::Ui,
    ctx: &RowCtx,
    pending: &mut Option<lumit_core::Op>,
    nav_jump: &mut Option<f64>,
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

    // Input rate (K-095, K-160): the fps the footage is interpreted at for
    // flow, so high-framerate clips (whose adjacent frames barely move)
    // interpolate across meaningful gaps. A keyframeable value the user types
    // any rate into — 0 = Native (the source's own rate). Rendered like any
    // animatable parameter: stopwatch, ◄ ◆ ► navigator, then the value field.
    let (_row, mut c) = row_frame(ui, ctx, false);
    let prop = params.input_fps.clone();
    let is_animated = prop.is_animated();

    // Stopwatch: animate the rate at the playhead, or freeze it to its current
    // value. One whole-retime SetLayerRetime, so each edit is one undo step.
    if let Some(animation) = stopwatch(&mut c, ctx.theme, &prop, ctx.lt) {
        commit_input_rate(ctx, rt, params, pending, animation);
    }
    // The ◄ ◆ ► navigator, once the rate is animated (the input-rate twin of
    // the effect rows' navigator; routes prev/next out through `nav_jump`).
    flow_input_rate_nav(&mut c, ctx, rt, params, &prop, pending, nav_jump);

    c.label(
        egui::RichText::new("Input rate")
            .small()
            .color(ctx.theme.text_muted),
    )
    .on_hover_text(
        "Treat the footage as this frame rate for flow — lower than the clip's own rate to \
         flow-interpolate high-speed footage into real slow motion. Type a rate, or 0 for native",
    );

    // The value field: a numeric box the user types any rate into. It defaults
    // to the layer's frame rate (T8) — an unset (0 = Native) value shows and
    // edits from the comp's fps rather than the word "Native", so the box always
    // holds a concrete float. 0 still reads as Native (the source's own rate) at
    // render, so typing 0 conforms to native.
    let committed = prop.value_at(ctx.lt);
    let shown_default = if committed < 0.5 { ctx.fps } else { committed };
    let id = egui::Id::new(("flow-input-rate", ctx.layer.id));
    let mut v = c.data(|d| d.get_temp::<f64>(id)).unwrap_or(shown_default);
    let resp = c
        .add(
            egui::DragValue::new(&mut v)
                .speed(0.25)
                .range(0.0..=1000.0)
                .max_decimals(2)
                .custom_formatter(|n, _| format!("{n:.0} fps"))
                .custom_parser(|s| {
                    let t = s.trim();
                    if t.is_empty() || t.eq_ignore_ascii_case("native") {
                        return Some(0.0);
                    }
                    // A leading number, ignoring any trailing unit like "fps".
                    let num: String = t
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '.')
                        .collect();
                    num.parse::<f64>().ok()
                }),
        )
        .on_hover_text("Frame rate the footage is read at (type 0 for native)");
    if resp.dragged() || resp.has_focus() {
        c.data_mut(|d| d.insert_temp(id, v));
    }
    if resp.drag_stopped() || resp.lost_focus() {
        // Only an actual change off the shown default commits (so merely focusing
        // the box, which shows the comp fps for a Native value, never writes).
        if (v - shown_default).abs() > 1e-9 {
            let animation = if is_animated {
                lumit_core::anim::Animation::Keyframed(upsert_key(&prop, ctx.lt, v))
            } else {
                lumit_core::anim::Animation::Static(v)
            };
            commit_input_rate(ctx, rt, params, pending, animation);
        }
        c.data_mut(|d| d.remove::<f64>(id));
    }
}

/// Write a new input-rate animation onto a Flow retime as one undoable
/// `SetLayerRetime` (K-160). Clones the retime, swaps in the property carrying
/// `animation`, and leaves every other Flow parameter untouched.
fn commit_input_rate(
    ctx: &RowCtx,
    rt: &lumit_core::retime::Retime,
    params: &lumit_core::retime::FlowParams,
    pending: &mut Option<lumit_core::Op>,
    animation: lumit_core::anim::Animation,
) {
    use lumit_core::retime::Interpolation;
    let mut r = rt.clone();
    let mut p = params.clone();
    p.input_fps = lumit_core::anim::Property {
        animation,
        extra: serde_json::Map::new(),
    };
    r.interpolation = Interpolation::Flow(p);
    *pending = Some(lumit_core::Op::SetLayerRetime {
        comp: ctx.comp_id,
        layer: ctx.layer.id,
        retime: Some(r),
    });
}

/// The ◄ ◆ ► navigator for the animated Flow input rate — the input-rate twin
/// of `effect_param_nav`. Shown once the rate is animated: the arrows jump the
/// playhead to the previous / next key (routed out through `nav_jump` as a
/// layer-local time, since `flow_group_rows` carries no `AppState`), and the
/// diamond adds a key at the playhead or removes the one already there. Each
/// commits one whole-retime `SetLayerRetime`, so every step is one undo.
#[allow(clippy::too_many_arguments)]
fn flow_input_rate_nav(
    c: &mut egui::Ui,
    ctx: &RowCtx,
    rt: &lumit_core::retime::Retime,
    params: &lumit_core::retime::FlowParams,
    prop: &lumit_core::anim::Property,
    pending: &mut Option<lumit_core::Op>,
    nav_jump: &mut Option<f64>,
) {
    use lumit_core::anim::Animation;
    let Animation::Keyframed(keys) = &prop.animation else {
        return;
    };
    let tol = 0.5 / ctx.fps.max(1.0); // within half a frame counts as "on" it
    let small = |i: Icon| egui::Button::new(crate::icons::text(i, 11.0)).frame(false);

    let has_prev = keys.iter().any(|k| k.time.to_f64() < ctx.lt - tol);
    if c.add_enabled(has_prev, small(Icon::PrevKeyframe))
        .on_hover_text("Previous keyframe")
        .clicked()
    {
        *nav_jump = keys
            .iter()
            .rev()
            .find(|k| k.time.to_f64() < ctx.lt - tol)
            .map(|k| k.time.to_f64());
    }

    let on_key = keys.iter().any(|k| (k.time.to_f64() - ctx.lt).abs() < tol);
    if c.add(small(if on_key {
        Icon::KeyframeFilled
    } else {
        Icon::Keyframe
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
                Animation::Static(prop.value_at(ctx.lt))
            } else {
                Animation::Keyframed(kept)
            }
        } else {
            Animation::Keyframed(upsert_key(prop, ctx.lt, prop.value_at(ctx.lt)))
        };
        commit_input_rate(ctx, rt, params, pending, animation);
    }

    let has_next = keys.iter().any(|k| k.time.to_f64() > ctx.lt + tol);
    if c.add_enabled(has_next, small(Icon::NextKeyframe))
        .on_hover_text("Next keyframe")
        .clicked()
    {
        *nav_jump = keys
            .iter()
            .find(|k| k.time.to_f64() > ctx.lt + tol)
            .map(|k| k.time.to_f64());
    }
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
