//! An effect draws at every colour depth a project can be set to. Its shaders
//! are written for sixteen bits, so one that isn't told the depth fails to build
//! and leaves nothing but an empty picture.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::edits::{base_layer, centred_transform, new_comp_ops, CompSettings};
use lumit_core::model::LinearColour;
use lumit_core::model::{ColourDepth, Document, EffectValue, LayerKind, ProjectItem, SolidDef};
use lumit_core::ops::{apply, Op};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const SIZE: u32 = 64;

#[test]
fn an_effect_draws_at_every_colour_depth() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    for depth in [
        ColourDepth::Sixteen,
        ColourDepth::Eight,
        ColourDepth::ThirtyTwo,
    ] {
        let mut doc = Document::new();
        doc.colour_depth = depth;
        let settings = CompSettings {
            width: SIZE,
            height: SIZE,
            ..CompSettings::default()
        };
        let (comp, ops) = new_comp_ops(&doc, settings, None);
        for op in &ops {
            apply(&mut doc, op).unwrap();
        }
        let def = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def,
            name: "White".into(),
            colour: LinearColour([1.0; 4]),
            width: SIZE,
            height: SIZE,
            extra: serde_json::Map::new(),
        }));
        let mut layer = base_layer(
            "White".into(),
            LayerKind::Solid { def },
            doc.comp(comp).unwrap().duration.0,
            centred_transform(f64::from(SIZE), f64::from(SIZE), SIZE, SIZE),
        );
        let mut exposure = lumit_core::fx::builtins::instantiate("exposure").unwrap();
        for p in &mut exposure.params {
            if p.id == "stops" {
                p.value = EffectValue::Float(Property::fixed(-1.0));
            }
        }
        layer.effects.push(exposure);
        apply(
            &mut doc,
            &Op::AddLayer {
                comp,
                index: 0,
                layer: Box::new(layer),
            },
        )
        .unwrap();
        let px = r.render_rgba(&Arc::new(doc), comp, 0, 1.0).unwrap().0;
        let centre = &px[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize..][..4];
        // White a stop down, which shows at about 188 after the sRGB curve.
        assert!(
            (centre[0] as i32 - 188).abs() < 4 && centre[3] == 255,
            "{depth:?}: {centre:?}"
        );
    }
}
