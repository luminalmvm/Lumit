//! The Effect Controls rows: each effect's title bar and parameter rows,
//! plus the per-parameter keyframe navigator.

use super::*;

/// The keyframe navigator for an animated Float effect parameter — the effect
/// twin of [`keyframe_nav`], which drives a transform property. Shown once the
/// param is animated, right after its stopwatch: ◄ / ► jump the playhead to the
/// previous / next key (routed out through `nav_jump` as a layer-local time,
/// since `effects_rows` carries no `AppState`), and the diamond adds a key at
/// the playhead or removes the one already there. Each commits one whole-stack
/// `SetLayerEffects` (never `SetTransformProperty` — the keys live on the effect
/// instance), so every step is one undo. Without this an animated effect
/// parameter showed a stopwatch but no way to step or add/remove its keys from
/// the row (the owner-reported defect).
pub(crate) fn effect_param_nav(
    c: &mut egui::Ui,
    ctx: &RowCtx,
    idx: usize,
    pi: usize,
    prop: &lumit_core::anim::Property,
    pending: &mut Option<lumit_core::Op>,
    nav_jump: &mut Option<f64>,
) {
    use lumit_core::anim::Animation;
    use lumit_core::model::EffectValue;
    let Animation::Keyframed(keys) = &prop.animation else {
        return;
    };
    let tol = 0.5 / ctx.fps.max(1.0); // within half a frame counts as "on" it
    let small = |i: Icon| egui::Button::new(crate::icons::text(i, 11.0)).frame(false);
    // One whole-stack op writing this param's new animation.
    let write = |ctx: &RowCtx, animation: Animation| -> lumit_core::Op {
        let mut effects = ctx.layer.effects.clone();
        effects[idx].params[pi].value = EffectValue::Float(lumit_core::anim::Property {
            animation,
            extra: serde_json::Map::new(),
        });
        lumit_core::Op::SetLayerEffects {
            comp: ctx.comp_id,
            layer: ctx.layer.id,
            effects,
        }
    };

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
        *pending = Some(write(ctx, animation));
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

/// The Effects group's rows (docs/08): an "Add effect" menu, then one block
/// per effect — bypass / name / remove on its title row, one row per
/// parameter beneath. Float parameters are fully animatable (stopwatch +
/// key diamonds on the lane, like any transform property); every change
/// commits one whole-stack SetLayerEffects, so each edit is one undo step.
pub(crate) fn effects_rows(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    pending: &mut Option<lumit_core::Op>,
    // Set to (layer, effect index, param index, provisional value) while a
    // Float effect parameter is being dragged, so the caller can drive a live
    // preview (`AppState::fx_edit`) without committing until release.
    fx_edit: &mut Option<(uuid::Uuid, usize, usize, f64)>,
    // Set to the clicked row when an effect param row is clicked (note 2.8.1),
    // for the caller to apply to `AppState::selected_prop`.
    select: &mut Option<crate::app_state::PropSel>,
    // Set to a layer-local time when the effect-parameter navigator's prev/next
    // arrow is clicked: `effects_rows` has no `AppState`, so the caller jumps the
    // playhead (both the Timeline and the Effect Controls panel do this).
    nav_jump: &mut Option<f64>,
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
                        // Default the dialogue to the preset library (K-129),
                        // created lazily, so a saved preset appears in the
                        // Effects & Presets browser straight away; the user can
                        // still navigate elsewhere.
                        let mut dialog = rfd::FileDialog::new()
                            .set_file_name(format!("effects.{}", crate::preset::PRESET_EXTENSION))
                            .add_filter("Lumit effect preset", &[crate::preset::PRESET_EXTENSION]);
                        if let Some(dir) = lumit_project::presets_dir() {
                            let _ = std::fs::create_dir_all(&dir);
                            dialog = dialog.set_directory(&dir);
                        }
                        if let Some(path) = dialog.save_file() {
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
            // The title bar lifts when one of this effect's own param rows is the
            // highlighted property (note 2.8.2), so the highlighted param's effect
            // reads at a glance.
            let title_hl = matches!(
                ctx.selected_prop,
                Some(crate::app_state::PropSel { layer: l, row: crate::app_state::PropRow::Effect { effect, .. } })
                    if l == layer.id && effect == idx
            );
            let (row_rect, mut c) = row_frame(ui, ctx, false);
            section_bar(ui, ctx, row_rect, title_hl);
            fx_title_rows.push(row_rect);
            // The per-effect visibility toggle (K-090 confirmation of §1.5): the
            // same eye as layer visibility, and it swaps to a closed eye when the
            // effect is bypassed — the state-matching-icon parity a toggleable eye
            // gets everywhere in the app (owner request; note 2.8.4).
            let (eye_icon, eye_col) = if e.enabled {
                (Icon::Eye, ctx.theme.text_secondary)
            } else {
                (Icon::EyeClosed, ctx.theme.text_disabled)
            };
            let (eye_rect, eye_resp) =
                c.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
            crate::icons::paint(c.painter(), eye_rect, eye_icon, eye_col, 1.4);
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
        // Collapsible parameter groups (P4, K-145): a group's params render
        // under a disclosure twirl and hide when it is closed. Members are a
        // contiguous run in the schema, so tracking the current group as the
        // loop walks the params is enough — the header draws when we step into
        // a group, and its open state gates the members until we step out.
        let mut current_group: Option<&'static str> = None;
        let mut group_open = true;
        for (pi, param) in e.params.iter().enumerate() {
            let Some(ps) = schema.params.iter().find(|p| p.id == param.id) else {
                continue;
            };
            let group = schema.groups.iter().find(|g| g.params.contains(&ps.id));
            if group.map(|g| g.label) != current_group {
                current_group = group.map(|g| g.label);
                if let Some(g) = group {
                    let gid = egui::Id::new(("fxgroup", e.id, g.label));
                    group_open =
                        group_header_row(ui, ctx.theme, g.label, gid, !g.collapsed, ctx.viewport);
                }
            }
            // Inside a closed group: the member row is hidden.
            if group.is_some() && !group_open {
                continue;
            }
            // Row selection (note 2.8.1): this param's row identity, whether it
            // is the highlighted one, and — set below on a click anywhere in the
            // row — the selection to hand back to the caller.
            let eff_row = crate::app_state::PropRow::Effect {
                effect: idx,
                param: pi,
            };
            let row_hl = ctx.is_selected(eff_row);
            let mut set_sel = |ui: &egui::Ui, rect: egui::Rect| {
                if row_click(ui, rect) {
                    *select = Some(crate::app_state::PropSel {
                        layer: layer.id,
                        row: eff_row,
                    });
                }
            };
            match (&param.value, ps.kind) {
                (EffectValue::Float(prop), ParamKind::Float { slider, hard, .. }) => {
                    let is_animated = prop.is_animated();
                    let (row_rect, mut c) = row_frame(ui, ctx, row_hl);
                    set_sel(ui, row_rect);
                    if let Some(animation) = stopwatch(&mut c, ctx.theme, prop, ctx.lt) {
                        let mut effects = layer.effects.clone();
                        effects[idx].params[pi].value =
                            EffectValue::Float(lumit_core::anim::Property {
                                animation,
                                extra: serde_json::Map::new(),
                            });
                        *pending = Some(commit(effects));
                    }
                    // The ◄ ◆ ► navigator, once the param is animated — the effect
                    // twin of the transform rows' `keyframe_nav` (the reported bug:
                    // effect params had a stopwatch but no navigator).
                    effect_param_nav(&mut c, ctx, idx, pi, prop, pending, nav_jump);
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
                    // Depth of field's Focus: an eyedropper that samples depth
                    // (the luma of the picked pixel as a proxy — the depth
                    // layer's own pixels are not separately readable from the
                    // UI) and sets Focus to it (docs/08 §3.22, K-123 companion).
                    if schema.match_name == "dof" && ps.id == "focus" {
                        let (eye, eye_resp) =
                            c.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
                        let eye_col = if eye_resp.hovered() {
                            ctx.theme.text_primary
                        } else {
                            ctx.theme.text_secondary
                        };
                        eyedropper::paint_icon(c.painter(), eye, eye_col);
                        if eye_resp
                            .on_hover_text("Pick focus depth from the Viewer")
                            .clicked()
                        {
                            eyedropper::request_arm(
                                c.ctx(),
                                EyedropperTarget {
                                    layer: layer.id,
                                    effect: idx,
                                    param: pi,
                                    mode: EyedropperMode::Depth,
                                },
                            );
                        }
                    }
                    // Selectable, draggable keys on the lane, like any property
                    // row (notes 2.1/2.6). The row is this effect's parameter.
                    if let lumit_core::anim::Animation::Keyframed(keys) = &prop.animation {
                        lane_keys(
                            ui,
                            app,
                            ctx,
                            row_rect,
                            crate::app_state::PropRow::Effect {
                                effect: idx,
                                param: pi,
                            },
                            keys,
                        );
                    }
                }
                (EffectValue::Choice(cur), ParamKind::Choice { options, .. }) => {
                    let (row_rect, mut c) = row_frame(ui, ctx, row_hl);
                    set_sel(ui, row_rect);
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
                    let (row_rect, mut c) = row_frame(ui, ctx, row_hl);
                    set_sel(ui, row_rect);
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
                    let (row_rect, mut c) = row_frame(ui, ctx, row_hl);
                    set_sel(ui, row_rect);
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
                    // A swatch that opens egui's colour picker, an eyedropper
                    // to sample from the Viewer, then scene-linear RGB drag
                    // boxes. The parameter's channels are animatable in the
                    // model; the row edits static values for now, like
                    // Bool/Choice.
                    let (row_rect, mut c) = row_frame(ui, ctx, row_hl);
                    set_sel(ui, row_rect);
                    c.label(
                        egui::RichText::new(ps.label)
                            .small()
                            .color(ctx.theme.text_muted),
                    );
                    // Swatch → the built-in colour picker (wheel + sliders).
                    // The parameter is scene-linear, exactly what egui's Rgb
                    // button edits, so the values pass straight through (clamped
                    // to 0..1 for the picker's gamut). A change commits the same
                    // undoable SetLayerEffects the RGB drags use.
                    let mut rgb = [
                        chs[0].value_at(ctx.lt).clamp(0.0, 1.0) as f32,
                        chs[1].value_at(ctx.lt).clamp(0.0, 1.0) as f32,
                        chs[2].value_at(ctx.lt).clamp(0.0, 1.0) as f32,
                    ];
                    if egui::color_picker::color_edit_button_rgb(&mut c, &mut rgb).changed() {
                        let mut effects = layer.effects.clone();
                        if let EffectValue::Colour(arr) = &mut effects[idx].params[pi].value {
                            arr[0] = lumit_core::anim::Property::fixed(rgb[0] as f64);
                            arr[1] = lumit_core::anim::Property::fixed(rgb[1] as f64);
                            arr[2] = lumit_core::anim::Property::fixed(rgb[2] as f64);
                        }
                        *pending = Some(commit(effects));
                    }
                    // Eyedropper: arm a colour pick from the Viewer for this
                    // parameter (the Matte key's key-colour param comes free —
                    // it is a Colour param and lands here too).
                    let (eye, eye_resp) =
                        c.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
                    let eye_col = if eye_resp.hovered() {
                        ctx.theme.text_primary
                    } else {
                        ctx.theme.text_secondary
                    };
                    eyedropper::paint_icon(c.painter(), eye, eye_col);
                    if eye_resp
                        .on_hover_text("Sample a colour from the Viewer")
                        .clicked()
                    {
                        eyedropper::request_arm(
                            c.ctx(),
                            EyedropperTarget {
                                layer: layer.id,
                                effect: idx,
                                param: pi,
                                mode: EyedropperMode::Colour,
                            },
                        );
                    }
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
                    let (row_rect, mut c) = row_frame(ui, ctx, row_hl);
                    set_sel(ui, row_rect);
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
                (EffectValue::Layer(cur), ParamKind::Layer { .. }) => {
                    // A picker for a layer-reference parameter (K-123), e.g. the
                    // DoF depth layer: this comp's other layers, plus None.
                    let (row_rect, mut c) = row_frame(ui, ctx, row_hl);
                    set_sel(ui, row_rect);
                    c.label(
                        egui::RichText::new(ps.label)
                            .small()
                            .color(ctx.theme.text_muted),
                    );
                    let cur_name = (*cur)
                        .and_then(|id| ctx.comp.layers.iter().find(|l| l.id == id))
                        .map_or("None", |l| l.name.as_str());
                    bare_dropdown(&mut c, egui::RichText::new(cur_name).small(), |ui| {
                        if ui.selectable_label(cur.is_none(), "None").clicked() {
                            let mut effects = layer.effects.clone();
                            effects[idx].params[pi].value = EffectValue::Layer(None);
                            *pending = Some(commit(effects));
                            ui.close_menu();
                        }
                        for other in ctx.comp.layers.iter().filter(|l| l.id != layer.id) {
                            if ui
                                .selectable_label(*cur == Some(other.id), other.name.as_str())
                                .clicked()
                            {
                                let mut effects = layer.effects.clone();
                                effects[idx].params[pi].value = EffectValue::Layer(Some(other.id));
                                *pending = Some(commit(effects));
                                ui.close_menu();
                            }
                        }
                    });
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
