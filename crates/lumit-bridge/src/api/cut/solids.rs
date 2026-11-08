//! A solid from the Project panel put down in the Cut workspace.

use super::*;

/// How long a solid is when it is put down, in seconds, as long as a still.
const SOLID_SECONDS: i64 = 5;

impl Edit<'_> {
    /// Put solid `solid` down at `at_frame` as one clip, on `target` or a new
    /// picture Sequence layer at the top the solid's size.
    pub(super) fn place_solid(
        mut self,
        target: Option<LayerReference>,
        solid: Uuid,
        at_frame: i64,
    ) -> Cut {
        let (width, height) = {
            let doc = self.of.document()?;
            let sd = doc.solid(solid).ok_or(BridgeError::InvalidItem)?;
            (f64::from(sd.width), f64::from(sd.height))
        };
        let at = self.time(at_frame.max(0))?;
        let r = match target {
            Some(layer) => self.row_of(layer.id()).ok_or(BridgeError::NotSequence)?,
            None => self.add_row(None, false, width, height),
        };
        if self.rows[r].audio_only {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        if self.rows[r].locked {
            return Err(BridgeCutResult::Locked.into());
        }
        let length = Rational::new(SOLID_SECONDS, 1).map_err(|_| BridgeError::InvalidTime)?;
        let clip = Clip::new(ClipSource::Solid(solid), Rational::ZERO, length, at, length);
        let dropped = clip.id;
        let row = &mut self.rows[r];
        row.clips.push(clip);
        row.clips = placed(std::mem::take(&mut row.clips), dropped, false);
        self.commit()
    }
}
