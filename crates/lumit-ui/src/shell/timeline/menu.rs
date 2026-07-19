//! Row furniture around the layer loop: the effect-drop predicate, the
//! per-layer right-click context menu, and the lane column headers.

use super::*;

/// Whether a layer of this kind accepts an effect dragged from the Effects &
/// Presets browser onto its Timeline row (K-101). Every `Layer` carries an
/// effect stack regardless of kind (`model::Layer::effects`), but v1 narrows
/// the drop target to footage and adjustment layers — an effect stack's two
/// ordinary homes (an adjustment layer exists only to host one). Every other
/// kind still gains effects the existing way: the "Add effect" row in its
/// own Effects group, untouched by this change.
pub(crate) fn accepts_effect_drop(kind: &lumit_core::model::LayerKind) -> bool {
    matches!(
        kind,
        lumit_core::model::LayerKind::Footage { .. } | lumit_core::model::LayerKind::Adjustment
    )
}

/// The right-click menu for a layer (opened from its name in the outline): the
/// things you can do to a layer, in one place (the house pattern — right-click
/// or menu, never scattered buttons). Ops it can build are returned through
/// `ctx_op`; app-level actions (rename, duplicate, delete, convert, trim) are
/// flagged for the caller to run once the outline has drawn. `mask` is the
/// layer's natural pixel size, for the "Add mask" default geometry.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layer_context_menu(
    ui: &mut egui::Ui,
    layer: &lumit_core::model::Layer,
    comp_id: uuid::Uuid,
    mask: (f64, f64),
    ctx_op: &mut Option<lumit_core::Op>,
    start_rename: &mut bool,
    duplicate_this: &mut bool,
    delete_this: &mut bool,
    convert_layer: &mut bool,
    trim_to_source: &mut bool,
    // Set to (effect index, its param count) when "Add effect" applied one, so
    // the caller can select it and focus the Effect Controls tab (owner).
    applied_effect: &mut Option<(usize, usize)>,
) {
    use lumit_core::fx;
    ui.set_min_width(170.0);
    if ui.button("Rename").clicked() {
        *start_rename = true;
        ui.close_menu();
    }
    ui.menu_button("Add effect", |ui| {
        // Grouped by category (K-090), mirroring the "Add effect" row; empty
        // categories don't show.
        for cat in fx::FxCategory::ALL {
            let members: Vec<_> = fx::BUILTINS.iter().filter(|s| s.category == cat).collect();
            if members.is_empty() {
                continue;
            }
            ui.menu_button(cat.label(), |ui| {
                for schema in members {
                    if ui.button(schema.label).clicked() {
                        if let Some(inst) = fx::instantiate(schema.match_name) {
                            *applied_effect = Some((layer.effects.len(), inst.params.len()));
                            let mut effects = layer.effects.clone();
                            effects.push(inst);
                            *ctx_op = Some(lumit_core::Op::SetLayerEffects {
                                comp: comp_id,
                                layer: layer.id,
                                effects,
                            });
                        }
                        ui.close_menu();
                    }
                }
            });
        }
    });
    let (w, h) = mask;
    ui.menu_button("Add mask", |ui| {
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
            *ctx_op = Some(lumit_core::Op::SetLayerMasks {
                comp: comp_id,
                layer: layer.id,
                masks,
            });
        }
    });
    ui.separator();
    if ui.button("Duplicate").clicked() {
        *duplicate_this = true;
        ui.close_menu();
    }
    if ui.button("Delete").clicked() {
        *delete_this = true;
        ui.close_menu();
    }
    ui.separator();
    // Solo and enable (visibility) toggles: the switches you reach for most,
    // ticked to show their current state.
    if ui
        .selectable_label(layer.switches.solo, "Solo")
        .on_hover_text("Isolate: while any layer is soloed, only soloed layers render")
        .clicked()
    {
        *ctx_op = Some(lumit_core::Op::SetLayerSolo {
            comp: comp_id,
            layer: layer.id,
            solo: !layer.switches.solo,
        });
        ui.close_menu();
    }
    if ui
        .selectable_label(layer.switches.visible, "Enabled")
        .on_hover_text("Show or hide this layer")
        .clicked()
    {
        *ctx_op = Some(lumit_core::Op::SetLayerVisible {
            comp: comp_id,
            layer: layer.id,
            visible: !layer.switches.visible,
        });
        ui.close_menu();
    }
    if ui
        .selectable_label(layer.switches.motion_blur, "Motion blur")
        .on_hover_text("Blur this layer along its own motion (needs the comp's motion blur on)")
        .clicked()
    {
        *ctx_op = Some(lumit_core::Op::SetLayerMotionBlur {
            comp: comp_id,
            layer: layer.id,
            motion_blur: !layer.switches.motion_blur,
        });
        ui.close_menu();
    }
    // Footage → sequenced layer (K-071).
    if matches!(layer.kind, lumit_core::model::LayerKind::Footage { .. })
        && ui.button("Convert to sequenced layer").clicked()
    {
        *convert_layer = true;
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
        *trim_to_source = true;
        ui.close_menu();
    }
}

/// Header icons over the Timeline outline's switch columns, aligned to the same
/// x-slots the per-layer rows use (see the `slot`/`edge` geometry below): an eye
/// over visibility, "Layer" over the names, the flow glyph over the flow toggle,
/// "3D" over the 3D switch, and a speaker over the audio/mute column. Purely
/// decorative — themed, muted, and clipped to the outline so they never touch
/// the lane ruler.
pub(crate) fn column_header_icons(
    ui: &egui::Ui,
    theme: &Theme,
    panel_left: f32,
    track_left: f32,
    ruler: egui::Rect,
) {
    let mut p = ui.painter().clone();
    let outline = egui::Rect::from_min_max(
        egui::pos2(panel_left, ruler.top()),
        egui::pos2((track_left - 6.0).max(panel_left + 1.0), ruler.bottom()),
    );
    p.set_clip_rect(outline);
    let edge = track_left - 6.0;
    let cy = ruler.center().y;
    let icon = |cx: f32, ic: Icon| {
        crate::icons::paint(
            &p,
            egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(13.0, 13.0)),
            ic,
            theme.text_muted,
            1.2,
        );
    };
    let label = |cx: f32, s: &str, align: egui::Align2| {
        p.text(
            egui::pos2(cx, cy),
            align,
            s,
            egui::FontId::proportional(9.0),
            theme.text_muted,
        );
    };
    // Slots mirror the row loop: eye at left+27, the volume switch beside it at
    // left+47, names from left+58, then the right-anchored switch cluster
    // measured back from `edge`. Matte and blend sit over their dropdowns; the
    // 3D column wears a cube glyph.
    icon(panel_left + 27.0, Icon::Eye);
    icon(panel_left + 47.0, Icon::Audio);
    label(panel_left + 58.0, "Layer", egui::Align2::LEFT_CENTER);
    label(edge - 179.0, "Matte", egui::Align2::CENTER_CENTER);
    label(edge - 120.0, "Blend", egui::Align2::CENTER_CENTER);
    icon(edge - 75.0, Icon::Flow);
    icon(edge - 49.0, Icon::Cube3d);
}
