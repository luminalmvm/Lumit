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
    /// The mask **on this layer** whose curve the glyphs run along.
    /// Unset lays the line straight, and so does a mask id that names nothing.
    pub path: Option<Uuid>,
    /// How far along that curve the line starts, px@comp, on the composition's
    /// clock like every other animatable channel that crosses here.
    pub path_offset: BridgeScalar,
    /// The animator groups moving the letters separately. Empty is the
    /// ordinary text layer.
    pub animators: Vec<BridgeTextAnimator>,
    /// The font, spacing, scale and outline of the letters.
    pub style: BridgeTextStyle,
    /// Alignment, indents and the room between lines.
    pub paragraph: BridgeParagraphStyle,
}

/// Whether pairs of letters are pulled together by the font's own kerning.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeKerning {
    Off,
    Metrics,
}

/// Capitals: as typed, all capitals, or small capitals for the lower case.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCaps {
    Normal,
    All,
    Small,
}

/// Where the letters sit against the baseline.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeScript {
    Normal,
    Superscript,
    Subscript,
}

/// Which side the lines of a block line up on.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTextAlign {
    Left,
    Centre,
    Right,
}

/// How a text layer's letters are set. One style for the whole layer.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTextStyle {
    /// The font family as the system lists it. Empty is the built-in Inter.
    pub family: String,
    /// The face inside the family, such as "Bold Italic". Empty is the
    /// family's regular face.
    pub face: String,
    /// Baseline to baseline in px. Unset is auto, 120 % of the size.
    pub leading: Option<f64>,
    pub kerning: BridgeKerning,
    /// Extra space after every letter, in thousandths of an em.
    pub tracking: f64,
    /// Per cent.
    pub scale_x: f64,
    /// Per cent.
    pub scale_y: f64,
    /// Px the letters are lifted off the baseline, positive is up.
    pub baseline_shift: f64,
    pub caps: BridgeCaps,
    pub script: BridgeScript,
    pub faux_bold: bool,
    pub faux_italic: bool,
    pub ligatures: bool,
    /// Off draws the outline alone.
    pub fill_on: bool,
    pub stroke_on: bool,
    pub stroke: BridgeColourRgba,
    /// Px, centred on the letter's edge.
    pub stroke_width: f64,
    /// The outline is drawn over the fill. Off puts the fill on top.
    pub stroke_over: bool,
}

/// How the lines of a text layer are laid out against each other. All px.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeParagraphStyle {
    pub align: BridgeTextAlign,
    /// Px the words wrap to. Unset is point text.
    pub box_width: Option<f64>,
    /// Stretch every line but a paragraph's last to the box.
    pub justify: bool,
    pub indent_left: f64,
    pub indent_right: f64,
    pub indent_first: f64,
    pub space_before: f64,
    pub space_after: f64,
}

/// The style a text layer has before anything is styled.
#[frb(sync)]
#[must_use]
pub fn default_text_style() -> BridgeTextStyle {
    read_style(&lumit_core::text::TextStyle::default())
}

/// The paragraph a text layer has before anything is set.
#[frb(sync)]
#[must_use]
pub fn default_paragraph_style() -> BridgeParagraphStyle {
    read_paragraph(&lumit_core::text::ParagraphStyle::default())
}

/// The font families installed on this machine, sorted for a menu. Not sync,
/// since the first call asks the system for its fonts.
#[must_use]
pub fn text_font_families() -> Vec<String> {
    lumit_text::families()
}

/// The faces of one family, such as Regular, Bold and Bold Italic. Empty when
/// the family isn't installed here.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn text_font_faces(family: String) -> Vec<String> {
    lumit_text::faces(&family)
}

/// What a range selector counts.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeSelectorBasis {
    Characters,
    Words,
}

/// How a range selector's weight falls off across its range.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeSelectorShape {
    Square,
    Ramp,
}

/// Which stretch of the words an animator reaches, in per cent of the run.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRangeSelector {
    pub start: BridgeScalar,
    pub end: BridgeScalar,
    pub offset: BridgeScalar,
    pub basis: BridgeSelectorBasis,
    pub shape: BridgeSelectorShape,
}

/// One animator group: what a reached letter is asked to do, and the range
/// saying which letters those are.
///
/// Every animator carries all five property groups — the decision entry argues
/// why there is no menu of properties to add them from — defaulted to values
/// that change nothing: no push, no turn, 100 % size, 100 % opacity, no tint.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTextAnimator {
    pub name: String,
    pub selector: BridgeRangeSelector,
    pub position_x: BridgeScalar,
    pub position_y: BridgeScalar,
    pub rotation: BridgeScalar,
    pub scale_x: BridgeScalar,
    pub scale_y: BridgeScalar,
    pub opacity: BridgeScalar,
    pub fill_r: BridgeScalar,
    pub fill_g: BridgeScalar,
    pub fill_b: BridgeScalar,
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

/// One line of a laid out block.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTextBlockLine {
    /// The index of the line's first character in the whole text, counted in
    /// characters.
    pub start: u32,
    /// The baseline, measured down from the layer's top edge.
    pub baseline: f64,
    /// The x of every gap between the line's letters, one more than it has
    /// characters. The break that ends a line isn't one of them.
    pub carets: Vec<f64>,
}

/// Where a block of text sits inside its layer, in layer pixels. Styled or
/// not, one line or several, it's the engine's own layout.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTextBlock {
    /// The layer's size: the raster the engine draws the block into.
    pub width: f64,
    pub height: f64,
    /// How far a caret reaches above and below a baseline.
    pub ascent: f64,
    pub descent: f64,
    /// The left and right edges of the words' own box.
    pub left: f64,
    pub right: f64,
    /// One per line, top to bottom. There is always at least one.
    pub lines: Vec<BridgeTextBlockLine>,
}

/// Lay out `text` the way the engine draws it in this style, with or without
/// the room animators get round it. The first call with a font loads it.
#[frb(sync)]
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::needless_pass_by_value)]
pub fn measure_text(
    text: String,
    size: f64,
    style: BridgeTextStyle,
    paragraph: BridgeParagraphStyle,
    animated: bool,
) -> BridgeTextBlock {
    let (style, paragraph) = (style_of(style), paragraph_of(paragraph));
    let l = lumit_text::layout(
        &lumit_text::TextBlock {
            text: &text,
            size: size as f32,
            // The colour doesn't move a letter.
            fill: lumit_core::model::LinearColour::BLACK,
            style: &style,
            paragraph: &paragraph,
        },
        animated,
    );
    BridgeTextBlock {
        width: f64::from(l.width),
        height: f64::from(l.height),
        ascent: f64::from(l.ascent),
        descent: f64::from(l.descent),
        left: f64::from(l.left),
        right: f64::from(l.right),
        lines: l
            .lines
            .into_iter()
            .map(|line| BridgeTextBlockLine {
                start: u32::try_from(line.start).unwrap_or(u32::MAX),
                baseline: f64::from(line.baseline),
                carets: line.carets.into_iter().map(f64::from).collect(),
            })
            .collect(),
    }
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
            animators: document
                .animators
                .iter()
                .map(|a| read_animator(a, offset))
                .collect(),
            style: read_style(&document.style),
            paragraph: read_paragraph(&document.paragraph),
        }))
    }

    /// Replace a text layer's document — one op, exactly invertible.
    ///
    /// The whole document rather than a field at a time, for the same reason
    /// every other edit here takes a whole value: retyping a word and changing
    /// its size is one action to the user and should be one undo step.
    ///
    /// **Adding the first animator moves the anchor with it**. An
    /// animated line is drawn into a box one text size larger a side, with the
    /// words that far in, so a letter has somewhere to drop in from — and the
    /// anchor is a fixed coordinate in the layer's own pixels, so without this
    /// the words would jump by that margin the moment the first animator
    /// arrived. Removing the last one puts it back. One `Op::Batch`, so it is
    /// one undo step: the same rule as typing, where committing the document
    /// and the pivot separately made `Ctrl+Z` undo a pivot nobody had moved.
    ///
    /// A change of size, style or paragraph moves the anchor the same way, so
    /// the first baseline stays where it is on screen, at the side the lines
    /// line up on. An outline then grows round the letters, and centred text
    /// that is tracked out grows both ways.
    #[frb(sync)]
    pub fn set_text(&self, document: BridgeTextDocument) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Text { document: before } = &layer.kind else {
            return Err(BridgeError::NotText);
        };
        let offset = layer.start_offset.0;
        let after = text_document_of(document, offset)?;
        let shift = restyle_shift(before, &after);

        let set = lumit_core::Op::SetTextDocument {
            comp: self.comp_id,
            layer: self.layer_id,
            document: after,
        };
        if shift == (0.0, 0.0) {
            return self.commit(set);
        }
        let mut ops = vec![set];
        for (prop, mut property, shift) in [
            (
                lumit_core::model::TransformProp::AnchorX,
                layer.transform.anchor_x.clone(),
                shift.0,
            ),
            (
                lumit_core::model::TransformProp::AnchorY,
                layer.transform.anchor_y.clone(),
                shift.1,
            ),
        ] {
            shift_property(&mut property, shift);
            ops.push(lumit_core::Op::SetTransformProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                prop,
                animation: property.animation,
            });
        }
        self.commit(lumit_core::Op::Batch { ops })
    }

    /// Replace a text layer's document **and its anchor and position
    /// together**, as one op.
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

    /// **Text to shapes**: a copy of this Type layer beside it, whose
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
                    inputs: None,
                },
            ))
            .into_owned();
        let spine = document
            .path
            .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt))
            .filter(|p| !p.is_empty());
        let contents = lumit_text::shape_items(
            &lumit_text::TextBlock::of(document, &words),
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
        // layer's corner is its *art's* bounding box rather than the origin —
        // so a mask carried over would land somewhere else. The path mask has
        // already done its work: the curve is in the outlines.
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
        crate::edits::solo_on_arrival(&mut copy, comp.layers.iter());

        let new_id = copy.id;
        self.commit(lumit_core::Op::AddLayer {
            comp: self.comp_id,
            index,
            layer: Box::new(copy),
        })?;
        Ok(LayerReference::new(self.project_id, self.comp_id, new_id))
    }

    /// **Text to points**: a copy of this Type layer beside it, fitted
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
        crate::edits::solo_on_arrival(&mut copy, comp.layers.iter());

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
        // Keys on the composition's clock, like every other channel.
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

    /// A camera's non-animatable settings (docs/impl/camera.md §9); `None`
    /// on any other kind.
    #[frb(sync)]
    pub fn get_camera_settings(
        &self,
    ) -> Result<Option<crate::api::layer::BridgeCameraSettings>, BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Camera { options, .. } = &layer.kind else {
            return Ok(None);
        };
        Ok(Some(crate::api::layer::BridgeCameraSettings::of(
            options.settings(),
        )))
    }

    /// Replace a camera's settings as one undo step.
    #[frb(sync)]
    pub fn set_camera_settings(
        &self,
        settings: crate::api::layer::BridgeCameraSettings,
    ) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let lumit_core::model::LayerKind::Camera { .. } = layer.kind else {
            return Err(BridgeError::NotCamera);
        };
        self.commit(lumit_core::Op::SetCameraSettings {
            comp: self.comp_id,
            layer: self.layer_id,
            settings: settings.core(),
        })
    }

    /// This camera's evaluated placement at `frame`: the eye, the effective
    /// rotation with a two-node camera's aim already in it, and the zoom, with
    /// a solve link followed (docs/impl/camera.md §2). What the camera tools
    /// start a drag from. `None` on any other kind.
    #[frb(sync)]
    pub fn camera_pose_at(
        &self,
        frame: u64,
    ) -> Result<Option<crate::api::layer::BridgeCameraPose>, BridgeError> {
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let Some(layer) = comp.layers.iter().find(|l| l.id == self.layer_id) else {
            return Err(BridgeError::InvalidLayer);
        };
        let Ok(t) = comp
            .frame_rate
            .time_of_frame(i64::try_from(frame).unwrap_or(i64::MAX))
        else {
            return Ok(None);
        };
        Ok(lumit_core::track::camera_pose_of(
            &doc,
            comp,
            layer,
            t.0.to_f64(),
            &lumit_render::track::Store,
        )
        .map(|p| crate::api::layer::BridgeCameraPose::of(p.pose)))
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
        animators: document
            .animators
            .into_iter()
            .map(|a| animator_from(a, offset))
            .collect::<Result<Vec<_>, _>>()?,
        style: Box::new(style_of(document.style)),
        paragraph: paragraph_of(document.paragraph),
        extra: serde_json::Map::new(),
    })
}

/// How far the anchor has to move for `after` to sit where `before` sat. The
/// live preview and the committed write both ask here, so a value being
/// dragged draws what letting go of it will write.
#[frb(ignore)]
pub(crate) fn restyle_shift(
    before: &lumit_core::model::TextDocument,
    after: &lumit_core::model::TextDocument,
) -> (f64, f64) {
    // A line on a path already has its room and its corner at the layer's
    // origin, so nothing there moves.
    let straight = before.path.is_none() && after.path.is_none();
    // Retyping the words alone leaves the anchor where it is.
    let restyled = before.size != after.size
        || before.style != after.style
        || before.paragraph != after.paragraph
        || before.animators.is_empty() != after.animators.is_empty();
    if !(straight && restyled) {
        return (0.0, 0.0);
    }
    let (was, now) = (reference_point(before), reference_point(after));
    (now.0 - was.0, now.1 - was.1)
}

/// The point of a block that a restyle leaves where it is: the first
/// baseline, at the side the lines line up on. In the layer's own pixels.
fn reference_point(document: &lumit_core::model::TextDocument) -> (f64, f64) {
    use lumit_core::text::TextAlign;
    let l = lumit_text::layout(
        &lumit_text::TextBlock::of(document, &document.text),
        !document.animators.is_empty(),
    );
    let x = match document.paragraph.align {
        TextAlign::Left => l.left,
        TextAlign::Centre => (l.left + l.right) * 0.5,
        TextAlign::Right => l.right,
    };
    let y = l.lines.first().map_or(0.0, |line| line.baseline);
    (f64::from(x), f64::from(y))
}

fn read_style(style: &lumit_core::text::TextStyle) -> BridgeTextStyle {
    use lumit_core::text::{Caps, Kerning, Script};
    BridgeTextStyle {
        family: style.family.clone(),
        face: style.face.clone(),
        leading: style.leading,
        kerning: match style.kerning {
            Kerning::Off => BridgeKerning::Off,
            Kerning::Metrics => BridgeKerning::Metrics,
        },
        tracking: style.tracking,
        scale_x: style.scale_x,
        scale_y: style.scale_y,
        baseline_shift: style.baseline_shift,
        caps: match style.caps {
            Caps::Normal => BridgeCaps::Normal,
            Caps::All => BridgeCaps::All,
            Caps::Small => BridgeCaps::Small,
        },
        script: match style.script {
            Script::Normal => BridgeScript::Normal,
            Script::Superscript => BridgeScript::Superscript,
            Script::Subscript => BridgeScript::Subscript,
        },
        faux_bold: style.faux_bold,
        faux_italic: style.faux_italic,
        ligatures: style.ligatures,
        fill_on: style.fill_on,
        stroke_on: style.stroke_on,
        stroke: colour_of(style.stroke),
        stroke_width: style.stroke_width,
        stroke_over: style.stroke_over,
    }
}

fn style_of(style: BridgeTextStyle) -> lumit_core::text::TextStyle {
    use lumit_core::text::{Caps, Kerning, Script};
    lumit_core::text::TextStyle {
        family: style.family,
        face: style.face,
        leading: style.leading.filter(|l| l.is_finite() && *l >= 0.0),
        kerning: match style.kerning {
            BridgeKerning::Off => Kerning::Off,
            BridgeKerning::Metrics => Kerning::Metrics,
        },
        tracking: style.tracking,
        scale_x: style.scale_x,
        scale_y: style.scale_y,
        baseline_shift: style.baseline_shift,
        caps: match style.caps {
            BridgeCaps::Normal => Caps::Normal,
            BridgeCaps::All => Caps::All,
            BridgeCaps::Small => Caps::Small,
        },
        script: match style.script {
            BridgeScript::Normal => Script::Normal,
            BridgeScript::Superscript => Script::Superscript,
            BridgeScript::Subscript => Script::Subscript,
        },
        faux_bold: style.faux_bold,
        faux_italic: style.faux_italic,
        ligatures: style.ligatures,
        fill_on: style.fill_on,
        stroke_on: style.stroke_on,
        stroke: linear_of(style.stroke),
        stroke_width: style.stroke_width,
        stroke_over: style.stroke_over,
        extra: serde_json::Map::new(),
    }
}

fn read_paragraph(paragraph: &lumit_core::text::ParagraphStyle) -> BridgeParagraphStyle {
    use lumit_core::text::TextAlign;
    BridgeParagraphStyle {
        align: match paragraph.align {
            TextAlign::Left => BridgeTextAlign::Left,
            TextAlign::Centre => BridgeTextAlign::Centre,
            TextAlign::Right => BridgeTextAlign::Right,
        },
        box_width: Some(paragraph.box_width).filter(|w| *w > 0.0),
        justify: paragraph.justify,
        indent_left: paragraph.indent_left,
        indent_right: paragraph.indent_right,
        indent_first: paragraph.indent_first,
        space_before: paragraph.space_before,
        space_after: paragraph.space_after,
    }
}

fn paragraph_of(paragraph: BridgeParagraphStyle) -> lumit_core::text::ParagraphStyle {
    use lumit_core::text::TextAlign;
    lumit_core::text::ParagraphStyle {
        align: match paragraph.align {
            BridgeTextAlign::Left => TextAlign::Left,
            BridgeTextAlign::Centre => TextAlign::Centre,
            BridgeTextAlign::Right => TextAlign::Right,
        },
        box_width: paragraph
            .box_width
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(0.0),
        justify: paragraph.justify,
        indent_left: paragraph.indent_left,
        indent_right: paragraph.indent_right,
        indent_first: paragraph.indent_first,
        space_before: paragraph.space_before,
        space_after: paragraph.space_after,
        extra: serde_json::Map::new(),
    }
}

/// One animator on its way out to the panel, its keys on the comp's clock.
#[frb(ignore)]
pub(crate) fn read_animator(
    animator: &lumit_core::text::TextAnimator,
    offset: lumit_core::time::Rational,
) -> BridgeTextAnimator {
    let read = |p| BridgeScalar::read_at(p, offset);
    BridgeTextAnimator {
        name: animator.name.clone(),
        selector: BridgeRangeSelector {
            start: read(&animator.selector.start),
            end: read(&animator.selector.end),
            offset: read(&animator.selector.offset),
            basis: match animator.selector.basis {
                lumit_core::text::SelectorBasis::Characters => BridgeSelectorBasis::Characters,
                lumit_core::text::SelectorBasis::Words => BridgeSelectorBasis::Words,
            },
            shape: match animator.selector.shape {
                lumit_core::text::SelectorShape::Square => BridgeSelectorShape::Square,
                lumit_core::text::SelectorShape::Ramp => BridgeSelectorShape::Ramp,
            },
        },
        position_x: read(&animator.position_x),
        position_y: read(&animator.position_y),
        rotation: read(&animator.rotation),
        scale_x: read(&animator.scale_x),
        scale_y: read(&animator.scale_y),
        opacity: read(&animator.opacity),
        fill_r: read(&animator.fill_r),
        fill_g: read(&animator.fill_g),
        fill_b: read(&animator.fill_b),
    }
}

/// And back: the panel's animator returned to the layer's own clock.
#[frb(ignore)]
fn animator_from(
    animator: BridgeTextAnimator,
    offset: lumit_core::time::Rational,
) -> Result<lumit_core::text::TextAnimator, BridgeError> {
    let write = |s: &BridgeScalar| -> Result<lumit_core::anim::Property, BridgeError> {
        Ok(lumit_core::anim::Property {
            animation: s.animation_at(offset)?,
            extra: serde_json::Map::new(),
        })
    };
    Ok(lumit_core::text::TextAnimator {
        name: animator.name,
        selector: lumit_core::text::RangeSelector {
            start: write(&animator.selector.start)?,
            end: write(&animator.selector.end)?,
            offset: write(&animator.selector.offset)?,
            basis: match animator.selector.basis {
                BridgeSelectorBasis::Characters => lumit_core::text::SelectorBasis::Characters,
                BridgeSelectorBasis::Words => lumit_core::text::SelectorBasis::Words,
            },
            shape: match animator.selector.shape {
                BridgeSelectorShape::Square => lumit_core::text::SelectorShape::Square,
                BridgeSelectorShape::Ramp => lumit_core::text::SelectorShape::Ramp,
            },
        },
        position_x: write(&animator.position_x)?,
        position_y: write(&animator.position_y)?,
        rotation: write(&animator.rotation)?,
        scale_x: write(&animator.scale_x)?,
        scale_y: write(&animator.scale_y)?,
        opacity: write(&animator.opacity)?,
        fill_r: write(&animator.fill_r)?,
        fill_g: write(&animator.fill_g)?,
        fill_b: write(&animator.fill_b)?,
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
#[frb(ignore)]
pub(crate) fn shift_property(property: &mut lumit_core::anim::Property, delta: f64) {
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
