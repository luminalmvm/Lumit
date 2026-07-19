//! Keyframe navigators: the stopwatch toggle, the previous/next-key
//! navigators, and the small key-time helper functions they share.

use super::*;

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

/// What the user asked of a [`keyframe_navigator`] this frame: jump the playhead
/// to a key time, or add/remove a key at the playhead. The caller performs the
/// commit — each channel keys differently (a transform property, a linked pair,
/// the Retime channel, an effect parameter) — while the navigator's look and
/// prev/toggle/next semantics stay identical everywhere (owner parity rule).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum KeyNavAction {
    /// Move the playhead to this key time (layer-local seconds).
    Jump(f64),
    /// Add a key at the playhead (`on_key` false) or remove the one there
    /// (`on_key` true).
    Toggle { on_key: bool },
}

/// The one shared AE-style `◄ ◆ ► ` keyframe navigator, next to the stopwatch:
/// ◄ jumps to the previous key, the diamond adds a key at the playhead (a filled
/// ◆ when one is already there — clicking then removes it), ► jumps to the next.
/// `times` are the channel's key times (layer-local seconds); `allow_remove` is
/// false only where a key at the playhead is structural and must not be deleted
/// (the Retime lens endpoints). Returns the action the user invoked, if any —
/// the caller commits it and, for a jump, moves the playhead. Consolidating the
/// four drifted navigators here keeps transform, Retime and effect rows
/// identical (Iconoir glyphs, K-085; the old ◄ ◆ ► characters aren't in the UI
/// fonts, and no colour is set so disabled buttons dim).
pub(crate) fn keyframe_navigator(
    c: &mut egui::Ui,
    times: &[f64],
    lt: f64,
    fps: f64,
    allow_remove: bool,
) -> Option<KeyNavAction> {
    let tol = 0.5 / fps.max(1.0); // within half a frame counts as "on" it
    let (prev, on_key, next) = key_nav_targets(times, lt, tol);
    let small = |i: Icon| egui::Button::new(crate::icons::text(i, 11.0)).frame(false);
    let mut out = None;

    if c.add_enabled(prev.is_some(), small(Icon::PrevKeyframe))
        .on_hover_text("Previous keyframe")
        .clicked()
    {
        if let Some(t) = prev {
            out = Some(KeyNavAction::Jump(t));
        }
    }

    let can_toggle = !on_key || allow_remove;
    if c.add_enabled(
        can_toggle,
        small(if on_key {
            Icon::KeyframeFilled
        } else {
            Icon::Keyframe
        }),
    )
    .on_hover_text(if on_key {
        "Remove keyframe here"
    } else {
        "Add keyframe here"
    })
    .clicked()
    {
        out = Some(KeyNavAction::Toggle { on_key });
    }

    if c.add_enabled(next.is_some(), small(Icon::NextKeyframe))
        .on_hover_text("Next keyframe")
        .clicked()
    {
        if let Some(t) = next {
            out = Some(KeyNavAction::Jump(t));
        }
    }

    out
}

/// Move the playhead to a navigator's jump target (layer-local seconds) and
/// refresh the preview — the shared tail of every navigator that owns `app`.
pub(crate) fn nav_jump_playhead(app: &mut AppState, ctx: &RowCtx, kt: f64) {
    app.preview_frame = ((kt + ctx.off) * ctx.fps).round().max(0.0) as usize;
    #[cfg(feature = "media")]
    app.refresh_preview();
}

/// The transform single-property navigator: the shared navigator plus a
/// `SetTransformProperty` commit for the add/remove.
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
    let times: Vec<f64> = keys.iter().map(|k| k.time.to_f64()).collect();
    match keyframe_navigator(ui, &times, ctx.lt, ctx.fps, true) {
        Some(KeyNavAction::Jump(kt)) => nav_jump_playhead(app, ctx, kt),
        Some(KeyNavAction::Toggle { on_key }) => {
            let tol = 0.5 / ctx.fps.max(1.0);
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
        None => {}
    }
}

/// The keyframe navigator for a linked transform pair (Anchor, Position,
/// Scale) — the same shared navigator as every other row, plus a
/// `two_prop_batch` commit so the diamond adds or removes a key on **both**
/// axes at once and the pair keeps matching keys. Prev/next jump across the
/// *union* of both axes' key times (a just-unlinked-then-relinked pair might not
/// match). Shown only once either axis is animated.
pub(crate) fn keyframe_nav_pair(
    c: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    px: lumit_core::model::TransformProp,
    py: lumit_core::model::TransformProp,
    pending: &mut Option<lumit_core::Op>,
) {
    let sx = ctx.layer.transform.get(px);
    let sy = ctx.layer.transform.get(py);
    if !(sx.is_animated() || sy.is_animated()) {
        return;
    }
    let tol = 0.5 / ctx.fps.max(1.0); // within half a frame counts as "on" it
    let times = union_key_times(sx, sy, tol);
    match keyframe_navigator(c, &times, ctx.lt, ctx.fps, true) {
        Some(KeyNavAction::Jump(kt)) => nav_jump_playhead(app, ctx, kt),
        Some(KeyNavAction::Toggle { on_key }) => {
            // Add on both axes, or remove the key at the playhead from both, so
            // the linked pair stays in step (the stopwatch drives them together
            // too). `remove = on_key`.
            *pending = Some(two_prop_batch(
                ctx.comp_id,
                ctx.layer.id,
                (px, toggle_key_at(sx, ctx.lt, tol, on_key)),
                (py, toggle_key_at(sy, ctx.lt, tol, on_key)),
            ));
        }
        None => {}
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
