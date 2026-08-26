//! Editing what a layer is *made of*, as opposed to where it sits.
//!
//! # In plain terms
//!
//! A solid, a text layer and a camera each have content of their own: a colour
//! and a size, some words, a zoom. Moving or fading such a layer is a transform
//! edit and lives elsewhere; changing what it *says* or what colour it *is*
//! lives here.
//!
//! One asymmetry is worth knowing because it surprises people. Editing a solid
//! changes an **asset** in the Project panel, so every layer using that solid
//! changes with it — that is the point of solids being assets rather than
//! per-layer settings. Editing a text layer changes only that layer.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::{effect::BridgeScalar, layer::LayerReference, solid::SolidReference, BridgeError};

/// A colour as the document stores it: scene-linear RGBA, which may exceed 1
/// (an HDR tint) or dip below 0 (a lift), so it is not a byte triple.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeColourRgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

/// A text layer's document (v1: one styled run — docs/03 §9.1).
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTextDocument {
    pub text: String,
    /// When set, the layer's words come from this expression at each frame
    /// rather than from `text`, which is kept so switching the expression off
    /// restores what was typed.
    pub expression: Option<String>,
    /// Pixel size at natural scale.
    pub size: f64,
    pub fill: BridgeColourRgba,
    /// The mask **on this layer** whose curve the glyphs run along (K-607).
    /// Unset lays the line straight, and so does a mask id that names nothing.
    pub path: Option<Uuid>,
    /// How far along that curve the line starts, px@comp, on the composition's
    /// clock like every other animatable channel that crosses here (K-213).
    pub path_offset: BridgeScalar,
}

/// A solid asset's definition.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSolidDef {
    pub name: String,
    pub colour: BridgeColourRgba,
    pub width: u32,
    pub height: u32,
}

impl LayerReference {
    /// This layer's text document, or `None` when it is not a text layer.
    #[frb(sync)]
    pub fn get_text(&self) -> Result<Option<BridgeTextDocument>, BridgeError> {
        let layer = self.item()?;
        let offset = layer.start_offset.0;
        let lumit_core::model::LayerKind::Text { document } = layer.kind else {
            return Ok(None);
        };
        Ok(Some(BridgeTextDocument {
            text: document.text,
            expression: document.expression,
            size: document.size,
            fill: colour_of(document.fill),
            path: document.path,
            path_offset: BridgeScalar::read_at(&document.path_offset, offset),
        }))
    }

    /// Replace a text layer's document — one op, exactly invertible.
    ///
    /// The whole document rather than a field at a time, for the same reason
    /// every other edit here takes a whole value: retyping a word and changing
    /// its size is one action to the user and should be one undo step.
    #[frb(sync)]
    pub fn set_text(&self, document: BridgeTextDocument) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Text { .. } = layer.kind else {
            return Err(BridgeError::NotText);
        };
        self.commit(lumit_core::Op::SetTextDocument {
            comp: self.comp_id,
            layer: self.layer_id,
            document: text_document_of(document, layer.start_offset.0)?,
        })
    }

    /// Replace a text layer's document **and its anchor and position
    /// together**, as one op (K-230).
    ///
    /// For the end of a typing session, which is one action to the user and has
    /// to be one undo step. It is two edits underneath — what the line says, and
    /// the pivot moving to the middle of the line it turned out to be, with
    /// Position compensating so the line does not shift — and committing them
    /// separately made `Ctrl+Z` undo a pivot nobody had moved before it undid
    /// the typing.
    #[frb(sync)]
    pub fn set_text_placed(
        &self,
        document: BridgeTextDocument,
        anchor_x: f64,
        anchor_y: f64,
        position_x: f64,
        position_y: f64,
    ) -> Result<(), BridgeError> {
        use lumit_core::model::TransformProp;
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Text { .. } = layer.kind else {
            return Err(BridgeError::NotText);
        };
        let offset = layer.start_offset.0;
        let mut ops = vec![lumit_core::Op::SetTextDocument {
            comp: self.comp_id,
            layer: self.layer_id,
            document: text_document_of(document, offset)?,
        }];
        for (prop, value) in [
            (TransformProp::AnchorX, anchor_x),
            (TransformProp::AnchorY, anchor_y),
            (TransformProp::PositionX, position_x),
            (TransformProp::PositionY, position_y),
        ] {
            ops.push(lumit_core::Op::SetTransformProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                prop,
                animation: BridgeScalar::Static(value).animation_at(offset)?,
            });
        }
        self.commit(lumit_core::Op::Batch { ops })
    }

    /// **Text to shapes** (K-608): a copy of this Type layer beside it, whose
    /// picture is the glyph outlines as vector art.
    ///
    /// The original is kept and untouched, which is After Effects' convention
    /// and the only one that survives a mistake: the words are still typeable,
    /// still keyed, still expression-driven, and the copy is a drawing.
    ///
    /// Converted **at `frame`**, because a line the words of which come from an
    /// expression says something different at every frame and the honest answer
    /// is what it says at the moment the command is used. A line on a path
    /// converts curved: the outlines are placed by the same walk the rasteriser
    /// uses, so the copy lands on top of the layer it came from rather than
    /// near it.
    ///
    /// One `Op`, so one undo step — the whole layer arrives at once.
    #[frb(sync)]
    pub fn create_shapes_from_text(&self, frame: i64) -> Result<LayerReference, BridgeError> {
        let comp = self.composition()?;
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Text { document } = &layer.kind else {
            return Err(BridgeError::NotText);
        };
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;
        let t = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?;
        let lt = lumit_core::time::layer_time(t.0.to_f64(), layer.start_offset.0);

        // The words at this frame, through the one resolver the rasteriser and
        // the frame key read, so a converted caption says what it was saying.
        let doc = {
            let project = self.project()?;
            let project = project.read().map_err(|_| BridgeError::ReadFailed)?;
            project.store.snapshot()
        };
        let words = document
            .resolved_text(std::sync::Arc::new(
                lumit_core::expression::ExpressionContext {
                    document: doc,
                    comp: Some(self.comp_id),
                    layer: Some(self.layer_id),
                    comp_time: t.0.to_f64(),
                    current_depth: 0,
                },
            ))
            .into_owned();
        let spine = document
            .path
            .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt))
            .filter(|p| !p.is_empty());
        let contents = lumit_text::shape_items_for(
            &words,
            document.size as f32,
            document.fill,
            spine.as_ref(),
            document.path_offset.value_at(lt) as f32,
        );
        if contents.is_empty() {
            return Err(BridgeError::NothingToConvert);
        }

        let mut copy = layer.clone();
        copy.id = Uuid::now_v7();
        copy.name = format!("{} outlines", layer.name);
        for effect in &mut copy.effects {
            effect.id = Uuid::now_v7();
        }
        // **The masks and the paint do not come across.** Both are drawn in
        // layer pixels measured from the layer's box corner, and a shape
        // layer's corner is its *art's* bounding box rather than the origin
        // (K-237) — so a mask carried over would land somewhere else. The path
        // mask has already done its work: the curve is in the outlines.
        copy.masks.clear();
        copy.paint.clear();
        // Which is also why the anchor moves by that corner: the art's box
        // starts at the first glyph's left bearing, not at zero, and without
        // this the copy would sit a few pixels off the line it came from.
        if let Some((x0, y0, _, _)) = lumit_core::shape::contents_bounds(&contents, lt) {
            shift_property(&mut copy.transform.anchor_x, -x0);
            shift_property(&mut copy.transform.anchor_y, -y0);
        }
        copy.kind = lumit_core::model::LayerKind::Shape { contents };

        let new_id = copy.id;
        self.commit(lumit_core::Op::AddLayer {
            comp: self.comp_id,
            index,
            layer: Box::new(copy),
        })?;
        Ok(LayerReference::new(self.project_id, self.comp_id, new_id))
    }

    /// **Text to points** (K-608): a copy of this Type layer beside it, fitted
    /// with **Emit from image**, so the words become a points stream in the
    /// shape of themselves.
    ///
    /// Fill-sampled rather than walked round the outlines, because that is what
    /// the points family consumes — see the decision entry. The original is
    /// kept, as with Text to shapes, and the copy is one `Op`.
    #[frb(sync)]
    pub fn create_points_from_text(&self) -> Result<LayerReference, BridgeError> {
        let comp = self.composition()?;
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Text { .. } = layer.kind else {
            return Err(BridgeError::NotText);
        };
        let index = comp
            .layers
            .iter()
            .position(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;

        let mut instance = lumit_core::fx::instantiate_for_raster(
            "emit_from_image",
            f64::from(comp.width),
            f64::from(comp.height),
        )
        .ok_or(BridgeError::UnknownEffectName)?;

        let mut copy = layer.clone();
        copy.id = Uuid::now_v7();
        copy.name = format!("{} points", layer.name);
        for effect in &mut copy.effects {
            effect.id = Uuid::now_v7();
        }
        // The Source row stays **unset**, which reads this effect's own input —
        // the words underneath it. Pointing it at the copy by name would say
        // the same thing in a way that breaks the moment the layer is renamed
        // or duplicated.
        lumit_core::fx::point_self_layer_params_at(&mut instance, copy.id);
        copy.effects.push(instance);

        let new_id = copy.id;
        self.commit(lumit_core::Op::AddLayer {
            comp: self.comp_id,
            index,
            layer: Box::new(copy),
        })?;
        Ok(LayerReference::new(self.project_id, self.comp_id, new_id))
    }

    /// A camera layer's zoom — focal distance in comp pixels, the After Effects
    /// model where the z=0 plane maps 1:1. `None` on any other kind.
    #[frb(sync)]
    pub fn get_camera_zoom(&self) -> Result<Option<BridgeScalar>, BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Camera { zoom, .. } = layer.kind else {
            return Ok(None);
        };
        // Keys on the composition's clock, like every other channel (K-213).
        Ok(Some(BridgeScalar::read_at(&zoom, layer.start_offset.0)))
    }

    /// Set a camera's zoom. Animatable, so it takes a whole `BridgeScalar` like
    /// every other curve-capable value.
    #[frb(sync)]
    pub fn set_camera_zoom(&self, zoom: BridgeScalar) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Camera { .. } = layer.kind else {
            return Err(BridgeError::NotCamera);
        };
        let animation = zoom.animation_at(layer.start_offset.0)?;
        self.commit(lumit_core::Op::SetCameraZoom {
            comp: self.comp_id,
            layer: self.layer_id,
            animation,
        })
    }
}

impl SolidReference {
    /// This solid asset's definition.
    #[frb(sync)]
    pub fn get_definition(&self) -> Result<BridgeSolidDef, BridgeError> {
        let solid = self.definition()?;
        Ok(BridgeSolidDef {
            name: solid.name,
            colour: colour_of(solid.colour),
            width: solid.width,
            height: solid.height,
        })
    }

    /// Edit the solid. **Every layer using it changes**, because a solid is an
    /// asset in the Project panel rather than a per-layer setting — which is
    /// what makes "recolour every background at once" one edit.
    #[frb(sync)]
    pub fn set_definition(&self, definition: BridgeSolidDef) -> Result<(), BridgeError> {
        if definition.name.trim().is_empty() {
            return Err(BridgeError::EmptyName);
        }
        self.definition()?;
        self.commit(lumit_core::Op::SetSolidDef {
            def: self.id(),
            name: definition.name,
            colour: linear_of(definition.colour),
            // A solid with no area is not a picture; the op would take it, but
            // nothing would ever draw.
            width: definition.width.max(1),
            height: definition.height.max(1),
        })
    }
}

#[frb(ignore)]
pub(crate) fn colour_of(c: lumit_core::model::LinearColour) -> BridgeColourRgba {
    BridgeColourRgba {
        r: f64::from(c.0[0]),
        g: f64::from(c.0[1]),
        b: f64::from(c.0[2]),
        a: f64::from(c.0[3]),
    }
}

#[frb(ignore)]
/// The document as the model holds it. One conversion, used by every path that
/// writes text — a new layer, a retype, a preview.
pub(crate) fn text_document_of(
    document: BridgeTextDocument,
    offset: lumit_core::time::Rational,
) -> Result<lumit_core::model::TextDocument, BridgeError> {
    Ok(lumit_core::model::TextDocument {
        text: document.text,
        // An empty box means no expression, not an expression that says
        // nothing — otherwise clearing the field would leave the layer
        // permanently blank with no way back to its words. Applied here, in
        // the one conversion, so every writer of a text document gets it.
        expression: document.expression.filter(|e| !e.trim().is_empty()),
        size: document.size,
        fill: linear_of(document.fill),
        path: document.path,
        path_offset: lumit_core::anim::Property {
            animation: document.path_offset.animation_at(offset)?,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
    })
}

/// Slide an animatable number by `delta` without changing its shape — every
/// keyframe moves by the same amount, so a keyed anchor keeps the animation it
/// had and simply sits somewhere else.
///
/// An expression is left alone: what it evaluates to is the expression's
/// business, and quietly wrapping somebody's sum in an addition would be a
/// worse surprise than a converted layer a few pixels out.
#[frb(ignore)]
fn shift_property(property: &mut lumit_core::anim::Property, delta: f64) {
    use lumit_core::anim::Animation;
    match &mut property.animation {
        Animation::Static(v) => *v += delta,
        Animation::Keyframed(keys) => {
            for k in keys.iter_mut() {
                k.value += delta;
            }
        }
        Animation::Expression(_) => {}
    }
}

pub(crate) fn linear_of(c: BridgeColourRgba) -> lumit_core::model::LinearColour {
    lumit_core::model::LinearColour([c.r as f32, c.g as f32, c.b as f32, c.a as f32])
}
