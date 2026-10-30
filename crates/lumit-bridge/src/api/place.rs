//! Several footage items placed in a composition at once: what a drop of a
//! Project panel selection on the Timeline makes.

use flutter_rust_bridge::frb;
use lumit_core::model::Layer;
use lumit_core::ops::Op;
use lumit_core::time::{CompTime, Duration, Rational};

use crate::api::{
    composition::{insert_row, CompositionReference},
    footage::FootageReference,
    streams::BridgeStreamsArrival,
    BridgeError,
};

impl CompositionReference {
    /// Place `footage` in this composition as new layers, the first given on
    /// top, as one undo step.
    ///
    /// `end_to_end` lays them one after another, each starting where the one
    /// before it ends, as Premiere and Vegas lay a multi-item drop. The
    /// composition grows to the last one's end if it runs past it, as a cut
    /// does. Without it they all start together, as After Effects stacks them.
    ///
    /// Each layer is what [`Self::add_footage_layer`] makes of its item, with
    /// `as_sequence` and `arrival` as there.
    #[frb(sync)]
    pub fn add_footage_layers(
        &self,
        footage: Vec<FootageReference>,
        as_sequence: bool,
        row: Option<u32>,
        arrival: Option<BridgeStreamsArrival>,
        end_to_end: bool,
    ) -> Result<(), BridgeError> {
        let arrival = arrival.unwrap_or(BridgeStreamsArrival::First);
        let proj = self.project()?;
        let comp = self.composition()?;
        let mut index = insert_row(row, comp.layers.len());
        let mut start = Rational::ZERO;
        let mut ops = Vec::new();
        {
            let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = p.store.snapshot();
            for item in &footage {
                let (layer, beside) =
                    Self::footage_layers(&p, &doc, &comp, item.id(), as_sequence, false, arrival)?;
                let length = layer
                    .out_point
                    .0
                    .checked_sub(layer.in_point.0)
                    .map_err(|_| BridgeError::InvalidTime)?;
                for mut layer in std::iter::once(layer).chain(beside) {
                    if end_to_end {
                        shift(&mut layer, Duration(start))?;
                    }
                    ops.push(Op::AddLayer {
                        comp: self.id,
                        index,
                        layer: Box::new(layer),
                    });
                    index += 1;
                }
                start = start
                    .checked_add(length)
                    .map_err(|_| BridgeError::InvalidTime)?;
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        if end_to_end && start > comp.duration.0 {
            ops.push(Op::SetCompSettings {
                comp: self.id,
                name: comp.name.clone(),
                width: comp.width,
                height: comp.height,
                frame_rate: comp.frame_rate,
                duration: Duration(start),
                background: comp.background,
            });
        }
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }
}

/// Move a layer and what it holds later by `by`.
fn shift(layer: &mut Layer, by: Duration) -> Result<(), BridgeError> {
    let at = |t: CompTime| t.add_dur(by).map_err(|_| BridgeError::InvalidTime);
    layer.in_point = at(layer.in_point)?;
    layer.out_point = at(layer.out_point)?;
    layer.start_offset = at(layer.start_offset)?;
    Ok(())
}
