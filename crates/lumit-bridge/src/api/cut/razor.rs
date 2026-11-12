//! The razor in the Cut workspace. A plain footage layer it falls inside is
//! made a Sequence layer first, in the same undo step.

use super::*;

/// `layer` as a Sequence layer holding one clip of `item`, the footage it is
/// showing.
///
/// The clip is placed at the layer's own in point with its trim the same
/// distance into the source, so a layer that has been trimmed or slid becomes
/// the same seconds of the same file at the same moment on the comp's clock.
#[frb(ignore)]
pub(crate) fn sequenced(layer: &Layer, item: Uuid) -> Layer {
    // The layer's own span length is the fallback when the media has not
    // probed; a floor of 0.04 seconds keeps a clip from being unclickable.
    let span = (layer.out_point.0.to_f64() - layer.in_point.0.to_f64()).max(0.04);
    let duration =
        Rational::from_f64_on_grid(span, Rational::FLICK_DEN).unwrap_or(layer.out_point.0);

    // Where the layer starts showing, on its own clock. A footage layer
    // shows the source moment that far past its zero, so the clip is
    // placed there and trimmed there and the two read the same source at
    // the same moment. Never negative: a clip's place cannot be.
    let local = layer
        .in_point
        .0
        .checked_sub(layer.start_offset.0)
        .unwrap_or(Rational::ZERO);
    let local = if local.is_negative() {
        Rational::ZERO
    } else {
        local
    };
    let source_out = local.checked_add(duration).unwrap_or(duration);

    let mut converted = layer.clone();
    // The choice of audio streams moves onto the clip, which is where a
    // Sequence layer keeps one.
    converted.audio_streams = Default::default();
    converted.kind = LayerKind::Sequence {
        clips: vec![Clip {
            id: Uuid::now_v7(),
            source: ClipSource::Footage(item),
            source_in: local,
            source_out,
            place_start: local,
            place_duration: duration,
            // None by construction: a retimed layer is never brought here,
            // and `convert_from_sequenced` still brings a clip's own map back
            // the other way.
            retime: None,
            interpolation: layer.interpolation.clone(),
            fade_in: Default::default(),
            fade_out: Default::default(),
            effects: Vec::new(),
            fx: true,
            gain_db: 0.0,
            link: None,
            audio_streams: layer.audio_streams.clone(),
            audible: true,
            blend: None,
            label: None,
            group: None,
            opacity: 1.0,
            takes: Vec::new(),
            transition: None,
            extra: serde_json::Map::new(),
        }],
    };
    converted
}

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
    /// [`sequenced`] makes of it. The layer becomes a Sequence layer ahead of
    /// the rows when the edit commits. `None` for a retimed layer, a layer of
    /// any other kind, and a time on or outside the clip's edges. Refused
    /// when the time falls inside a locked layer.
    fn sequence_row(&mut self, id: Uuid, at: Rational) -> Cut<Option<usize>> {
        let Some(index) = self.comp.layers.iter().position(|l| l.id == id) else {
            return Ok(None);
        };
        let layer = &self.comp.layers[index];
        let LayerKind::Footage { item } = layer.kind else {
            return Ok(None);
        };
        if layer.retime.is_some() {
            return Ok(None);
        }
        let made = sequenced(layer, item);
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
