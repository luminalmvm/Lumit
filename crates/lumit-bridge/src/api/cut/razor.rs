//! The razor in the Cut workspace. A plain footage layer it falls inside is
//! made a Sequence layer first, in the same undo step.

use super::*;

impl Edit<'_> {
    pub(super) fn razor_at(
        mut self,
        layers: &[LayerReference],
        at_frame: i64,
        linked: bool,
    ) -> Cut {
        let at = self.time(at_frame)?;
        let rows: Vec<usize> = if layers.is_empty() {
            (0..self.rows.len()).collect()
        } else {
            let mut rows = Vec::new();
            for layer in layers {
                match self.row_of(layer.id()) {
                    Some(r) => rows.push(r),
                    None => rows.extend(self.sequence_row(layer.id(), at)?),
                }
            }
            rows
        };
        if self.razor(&rows, at, linked)? == 0 {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }

    /// A plain footage layer that `at` falls inside, as a row of the one clip
    /// [`crate::api::layer::sequenced`] makes of it. The layer becomes a
    /// Sequence layer ahead of the rows when the edit commits. `None` for a
    /// retimed layer, a layer of any other kind, and a time on or outside the
    /// clip's edges. Refused when the time falls inside a locked layer.
    fn sequence_row(&mut self, id: Uuid, at: Rational) -> Cut<Option<usize>> {
        let Some(index) = self.comp.layers.iter().position(|l| l.id == id) else {
            return Ok(None);
        };
        let layer = &self.comp.layers[index];
        // A layer that is not footage, or is retimed, is refused here and the
        // razor leaves it alone.
        let Ok(made) = crate::api::layer::sequenced(layer) else {
            return Ok(None);
        };
        let Some(row) = Row::of(&made)? else {
            return Ok(None);
        };
        if !row
            .clips
            .iter()
            .any(|c| c.place_start < at && at < c.place_end())
        {
            return Ok(None);
        }
        if made.switches.locked {
            return Err(BridgeCutResult::Locked.into());
        }
        let comp = self.comp.id;
        self.first.push(Op::RemoveLayer { comp, layer: id });
        self.first.push(Op::AddLayer {
            comp,
            index,
            layer: Box::new(made),
        });
        self.rows.push(row);
        Ok(Some(self.rows.len() - 1))
    }
}
