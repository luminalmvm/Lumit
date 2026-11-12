//! A shot's grade written out as a `.cube` LUT, for another application to
//! load.

use std::sync::Arc;

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::grade_shots::{frame_in_shot, shot_effects, shot_in, shot_name, shot_time};
use crate::api::BridgeError;
use lumit_render::grade_lut::CubeSpace;

/// What the numbers in an exported LUT mean, going in and coming out.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLutSpace {
    /// Rec. 709 video, a 2.4 gamma.
    Video,
    /// ACEScct.
    Log,
    Srgb,
}

impl CompositionReference {
    /// Write one shot's grade to `path` as a `.cube` of `size` points a side
    /// (17, 33 or 65), read at comp `frame`, which is moved inside the shot.
    ///
    /// The grade is the shot's Colour effects that are switched on, in order,
    /// with anything that depends on where a pixel is left out, since a LUT
    /// can't hold it. The file is written beside its name first and moved
    /// over it at the end, so one stopped half way leaves no half a LUT.
    /// Async: a 65-point cube runs a quarter of a million colours through
    /// the grade.
    pub fn export_grade_lut(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        path: String,
        size: u32,
        space: BridgeLutSpace,
    ) -> Result<(), BridgeError> {
        if ![17, 33, 65].contains(&size) {
            return Err(BridgeError::InvalidParam);
        }
        let (doc, colour) = {
            let proj = self.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let colour = crate::api::colour::with_colour(&state, |c| c.loaded().cloned())?;
            (state.store.snapshot(), colour)
        };
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let (shown, clip_shown) = shot_in(comp, layer, clip)?;
        let frame = frame_in_shot(comp, shown, clip_shown, frame)?;
        let lt = shot_time(comp, shown, clip_shown, frame)?;
        let comp_time = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?;
        let context = Arc::new(lumit_core::expression::ExpressionContext {
            document: doc.clone(),
            comp: Some(self.id),
            layer: Some(layer),
            comp_time: comp_time.0.to_f64(),
            current_depth: 0,
            inputs: None,
        });
        let diag = f64::from(comp.width).hypot(f64::from(comp.height)) as f32;
        let title = shot_name(&doc, shown, clip_shown);
        write_lut(
            shot_effects(shown, clip_shown),
            (lt, diag, context, colour.as_ref()),
            (size, space, &title, &path),
        )
    }

    /// Write the grade kept with the still at `still`, called `name`, to
    /// `path` as a `.cube`, as [`Self::export_grade_lut`] writes a shot's. Its
    /// keys are read at their start.
    pub fn export_still_lut(
        &self,
        still: String,
        name: String,
        path: String,
        size: u32,
        space: BridgeLutSpace,
    ) -> Result<(), BridgeError> {
        if ![17, 33, 65].contains(&size) {
            return Err(BridgeError::InvalidParam);
        }
        let text = crate::api::stills::read_still_grade(still)?;
        let preset =
            lumit_core::preset::from_json(&text).map_err(|_| BridgeError::InvalidPreset)?;
        let mut effects = lumit_core::preset::instantiated(&preset);
        let (doc, colour) = {
            let proj = self.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let colour = crate::api::colour::with_colour(&state, |c| c.loaded().cloned())?;
            (state.store.snapshot(), colour)
        };
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let raster = (f64::from(comp.width), f64::from(comp.height));
        lumit_core::fx::builtins::migrate_in(&mut effects, raster, raster, &|_, _| false);
        let context = Arc::new(lumit_core::expression::ExpressionContext {
            document: doc.clone(),
            comp: Some(self.id),
            layer: None,
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        });
        let diag = raster.0.hypot(raster.1) as f32;
        write_lut(
            &effects,
            (0.0, diag, context, colour.as_ref()),
            (size, space, &name, &path),
        )
    }
}

/// How a grade is read: its layer time, the picture's diagonal, the
/// expression context and the colour config.
type LutRead<'a> = (
    f64,
    f32,
    Arc<lumit_core::expression::ExpressionContext>,
    Option<&'a Arc<lumit_render::colour::Loaded>>,
);

/// Bake `effects` and write them as a `.cube` of the size, space, title and
/// path in `out`: beside its name first and moved over it at the end, so one
/// stopped half way leaves no half a LUT.
#[frb(ignore)]
fn write_lut(
    effects: &[lumit_core::model::EffectInstance],
    read: LutRead<'_>,
    out: (u32, BridgeLutSpace, &str, &str),
) -> Result<(), BridgeError> {
    let (lt, diag, context, colour) = read;
    let (size, space, title, path) = out;
    let space = match space {
        BridgeLutSpace::Video => CubeSpace::Video,
        BridgeLutSpace::Log => CubeSpace::Log,
        BridgeLutSpace::Srgb => CubeSpace::Srgb,
    };
    let cube =
        lumit_render::grade_lut::bake(effects, lt, diag, context, colour, size as usize, space)
            .map_err(|_| BridgeError::InvalidParam)?;
    let text = lumit_core::lut::write_cube(&cube, title);
    let part = format!("{path}.part");
    let written = std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written.map_err(|_| BridgeError::WriteFailed)
}
