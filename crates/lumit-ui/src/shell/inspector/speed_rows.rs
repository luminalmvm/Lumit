//! The footage speed / Retime property row and its supporting helpers.

use super::*;

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
    // The Retime channel is one selectable row like any other (UI-6): highlight
    // when graphed or picked, and route a click through the shared multi-select
    // gestures so it joins transform and effect rows in `selected_props`.
    let sel_row = crate::app_state::PropRow::Retime;
    let (row_rect, mut c) = row_frame(ui, ctx, is_graphed || ctx.is_selected(sel_row));
    prop_row_select(
        app,
        ui,
        row_rect,
        crate::app_state::PropSel {
            layer: ctx.layer.id,
            row: sel_row,
        },
    );

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

    // The shared ◄ ◆ ► navigator, like every other property row — shown once the
    // channel is keyed. The arrows jump the playhead between this lens's keys;
    // the diamond adds a key at the playhead, or removes an interior one (the
    // structural start/end keys stay, so removal is disabled at an endpoint).
    let nav_on = if speed_lens { animated } else { time_enabled };
    if nav_on {
        let tol = 0.5 / fps.max(1.0);
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
        // An endpoint key is structural (the lens must keep its [0, dur] pair),
        // so removal there is disallowed — the only per-row deviation the shared
        // navigator supports.
        let at_endpoint = ctx.lt <= tol || (dur.to_f64() - ctx.lt).abs() < tol;
        match keyframe_navigator(&mut c, &key_times, ctx.lt, ctx.fps, !at_endpoint) {
            Some(KeyNavAction::Jump(kt)) => nav_jump_playhead(app, ctx, kt),
            Some(KeyNavAction::Toggle { on_key }) => {
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
            None => {}
        }
    }

    // "Time" in the value lens, "Velocity" in the derivative lens (K-076).
    let channel_name = if speed_lens { "Velocity" } else { "Time" };
    let name_clicked = c
        .add(
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
        .clicked();
    // A plain click graphs the Retime channel; a Ctrl/Shift-click is a
    // list-select gesture (handled above) and must not re-graph it.
    if name_clicked && !ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl) {
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
        // The provisional retime for a dragged source time — the same value the
        // release commits, reused for the live preview.
        let provisional = |frames: f64| -> Option<lumit_core::retime::Retime> {
            let new_src = frames.max(0.0) / src_fps;
            let mut keys = value_keys
                .clone()
                .unwrap_or_else(|| vec![(Rational::ZERO, Rational::ZERO), (dur, dur)]);
            upsert_value_key(&mut keys, ctx.lt, new_src, dur, fps, src_fps);
            lumit_core::retime::Retime::from_value_keyframes(&keys)
        };
        if resp.dragged() || resp.has_focus() {
            c.data_mut(|d| d.insert_temp(id, frames));
            // Live preview: unlike a transform/effect drag (which re-composites
            // the same decoded frame), a Time drag changes which *source* frame
            // is on screen, so drive `retime_edit` — the decode job builder
            // overrides this layer's retime with it and re-decodes.
            app.retime_edit = provisional(frames).map(|rt| (ctx.layer.id, rt));
        }
        if resp.changed() {
            // Re-request the frame so the viewer shows the dragged source time
            // (the playhead frame is unchanged, so it isn't stale on its own).
            #[cfg(feature = "media")]
            app.refresh_preview();
        }
        if resp.drag_stopped() || resp.lost_focus() {
            app.retime_edit = None;
            let new_src = frames.max(0.0) / src_fps;
            // Editing the time keyframes it (AE Time Remap), seeding endpoints
            // from the current curve when none exist yet.
            if (new_src - src_now).abs() > 0.5 / src_fps {
                *pending = Some(lumit_core::Op::SetLayerRetime {
                    comp: ctx.comp_id,
                    layer: ctx.layer.id,
                    retime: provisional(frames),
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
