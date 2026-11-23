//! Grid's behaviour.

use super::*;

impl EffectDef for GridDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Grid as EffectMetadata>::SCHEMA
    }

    /// The picture *and* the data, exactly as Particulate declares it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    /// The raster factor, so the composition's camera reaches the pixels this
    /// frame is drawn at.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Grid::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    /// Lines hides the Points rows rather than greying them, since a group
    /// can only answer to one dropdown and the Rings rows already answer to
    /// Layout.
    fn row_state(&self, inst: &EffectInstance) -> RowState {
        let lines = matches!(inst.param("kind"), Some(EffectValue::Choice(1)));
        RowState {
            hidden: if lines {
                POINTS_ONLY.to_vec()
            } else {
                Vec::new()
            },
            ..RowState::default()
        }
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let grid = Grid::read(p);
        if grid.kind == 1 {
            GridLines::of(&grid, Grid::px_scale_of(p)).draw(rgba, w, h);
        }
    }
}
