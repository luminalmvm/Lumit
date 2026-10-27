//! Type fill (SICELIOT) fills the frame with rows of words that stretch to
//! fill the room a shape leaves them.
//!
//! **In plain terms.** The words, typed as `Flood|the|Grid`, are set in rows
//! from top to bottom, each word in its own column, the columns as wide as the
//! words naturally are. A **Region layer** (any layer: a shape, a letter, a
//! precomp of shapes put together with blend modes) says where the words may
//! go: along each row, a column keeps only the stretches the region leaves
//! free, and the column's word is stretched across and down from its first
//! stretch to its last, each stretch showing its own slice: a shape that
//! cuts a column in two cuts the word, never shows it twice. Animate the
//! region and the words flood the room it leaves, as Cavalry's Boolean,
//! Contours to Sub-Meshes and Mesh distribution do together.
//!
//! The look comes from a Text layer, as Label points' does. This file says
//! which word goes in which box; the renderer draws them, because the text
//! engine sits above this crate.

use crate::fx::builtins::MAP_PLACEMENT_OPTIONS;
use crate::fx::{
    params::ParamId, params::ShortText, params::Value, registry::EffectDef,
    registry::EffectMetadata, registry::ResolveCx, registry::Signature, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// What Order offers.
pub const TYPE_FILL_ORDER: &[&str] = &["In turn", "Shift per row", "Random"];

/// Where the region is read from.
pub const REGION_FROM: &[&str] = &["Alpha", "Luminance"];

/// The dice Random order rolls.
const ORDER_ATTR: u32 = 130;

/// Type fill's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "siceliot_type_fill",
    label = "Type fill",
    version = 1,
    category = Siceliot,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
)]
pub struct TypeFill {
    /// The Text layer whose look the words take: font, style and colour.
    #[layer(label = "Text layer")]
    pub text_layer: bool,

    /// The words, between bars, set left to right in every row.
    #[text(label = "Words", default = "Flood|the|Grid")]
    pub words: ShortText,

    /// The layer that says where the words may go. Unset, the whole frame is
    /// theirs.
    #[layer(label = "Region layer")]
    pub region_layer: bool,

    /// Where the Region layer's picture sits on this one: as it sits in the
    /// comp, stretched over the frame, or centred at its own size.
    #[choice(label = "Region placement", options = *MAP_PLACEMENT_OPTIONS, default = 0)]
    pub region_layer_placement: u32,

    /// Whether the region is where the Region layer is opaque or where it is
    /// bright.
    #[choice(label = "Region from", options = REGION_FROM, default = 0)]
    pub region_from: u32,

    /// Fill where the region is not, instead of where it is.
    #[toggle(label = "Invert region", default = false)]
    pub invert: bool,

    /// Keep every word whole at its column's width, instead of stretching
    /// it into the room it has: two Type fills on both sides of a shape then
    /// share each word, "th" on one, "e" on the other. Word gap no longer
    /// insets the region's edges. Either way the region cuts the words pixel
    /// by pixel.
    #[toggle(label = "Cut words", default = false)]
    pub cut_words: bool,

    /// How much of the region a place needs to be in it, per cent.
    #[slider(
        label = "Threshold",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub threshold: f32,

    /// How many rows fill the frame from top to bottom.
    #[counter(
        label = "Rows",
        min = 1,
        max = 200,
        default = 24,
        hard_min = 1,
        hard_max = 400,
        unit = Raw
    )]
    pub rows: i32,

    /// The room between one row and the next, px@comp.
    #[slider(
        label = "Row gap",
        min = 0.0,
        max = 200.0,
        default = 6.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub row_gap: f32,

    /// The room between the words of a row and at the edges of the region,
    /// px@comp.
    #[slider(
        label = "Word gap",
        min = 0.0,
        max = 200.0,
        default = 8.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub word_gap: f32,

    /// A column whose word would be narrower than this is left empty, px@comp.
    #[slider(
        label = "Min width",
        min = 0.0,
        max = 1000.0,
        default = 24.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub min_width: f32,

    /// Which word each column of a row has: the same order every row, the
    /// list moved on one word a row, or shuffled per row by the Seed.
    #[choice(label = "Order", options = TYPE_FILL_ORDER, default = 0)]
    pub order: u32,

    /// Which shuffle Random order makes.
    #[seed]
    pub seed: u32,

    /// The Mix every effect ends with, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// One word in its box: which word of the list, and where, left, top, right
/// and bottom in raster px; only the slice between `clip` (left and right)
/// shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Piece {
    pub word: usize,
    pub rect: [f32; 4],
    pub clip: [f32; 2],
}

impl TypeFill {
    /// The raster factor.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: crate::fx::params::Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The words between the bars, trimmed, empty ones left out.
    #[must_use]
    pub fn word_list(&self) -> Vec<String> {
        self.words
            .as_str()
            .split('|')
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// The top and bottom of each row in a raster `height` px high.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
    pub fn row_bands(&self, height: f32, px_scale: f32) -> Vec<[f32; 2]> {
        let n = self.rows.clamp(1, 400) as usize;
        let gap = (self.row_gap * px_scale).max(0.0);
        let row = ((height - gap * (n - 1) as f32) / n as f32).max(0.0);
        (0..n)
            .map(|r| {
                let top = r as f32 * (row + gap);
                [top, top + row]
            })
            .collect()
    }

    /// Which word each column of row `row` takes, left to right, out of `n`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn row_order(&self, row: usize, n: usize) -> Vec<usize> {
        match self.order {
            1 => (0..n).map(|k| (k + row) % n.max(1)).collect(),
            2 => {
                let mut keyed: Vec<(f32, usize)> = (0..n)
                    .map(|k| {
                        let attr = ORDER_ATTR.wrapping_add(k as u32);
                        (crate::fx::points::draw(self.seed, row as u64, attr), k)
                    })
                    .collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
                keyed.into_iter().map(|(_, k)| k).collect()
            }
            _ => (0..n).collect(),
        }
    }

    /// How much of a place the words keep, 0..1, from the region read there
    /// (`v`, 0..1): a soft step at Threshold, so a Type fill and its Invert
    /// region twin add up to exactly one along the region's edge.
    #[must_use]
    pub fn coverage(&self, v: f32) -> f32 {
        let c = ((v - self.threshold / 100.0) * 2.0 + 0.5).clamp(0.0, 1.0);
        if self.invert {
            1.0 - c
        } else {
            c
        }
    }

    /// The free stretches along one row, left to right, from the region read
    /// across it (`samples`, one 0..1 value a pixel). A place is in the
    /// region at Threshold and above; Invert region takes the rest.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn free_spans(&self, samples: &[f32]) -> Vec<[f32; 2]> {
        let cut = self.threshold / 100.0;
        let mut spans = Vec::new();
        let mut start: Option<usize> = None;
        for (x, v) in samples.iter().enumerate() {
            let inside = (*v >= cut) != self.invert;
            match (inside, start) {
                (true, None) => start = Some(x),
                (false, Some(s)) => {
                    spans.push([s as f32, x as f32]);
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            spans.push([s as f32, samples.len() as f32]);
        }
        spans
    }

    /// Every word's box. `free[r]` is row r's free stretches; `natural` each
    /// word's width at one px of height, so the columns are as wide as their
    /// words naturally are; `width` the raster's width.
    ///
    /// Each row is cut into columns across the whole width, one word each,
    /// with Word gap between them; a column keeps the parts of the free
    /// stretches it covers, less Word gap at the region's edges. Its word is
    /// stretched from the first part to the last, if that is as wide as Min
    /// width, and each part shows its own slice of it: a shape that cuts a
    /// column in two cuts its word, "t" on one side and "he" on the other,
    /// and never shows the word twice.
    #[must_use]
    pub fn pieces(
        &self,
        bands: &[[f32; 2]],
        free: &[Vec<[f32; 2]>],
        natural: &[f32],
        width: f32,
        px_scale: f32,
    ) -> Vec<Piece> {
        let n = natural.len();
        if n == 0 {
            return Vec::new();
        }
        let gap = (self.word_gap * px_scale).max(0.0);
        let least = (self.min_width * px_scale).max(1.0);
        let mut out = Vec::new();
        for (r, band) in bands.iter().enumerate() {
            let order = self.row_order(r, n);
            let height = band[1] - band[0];
            if height <= 0.0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let room = (width - gap * (n.saturating_sub(1)) as f32).max(0.0);
            let total: f32 = order.iter().map(|&k| natural[k].max(1e-3)).sum();
            let mut left = 0.0f32;
            for &k in &order {
                let column = [left, left + room * natural[k].max(1e-3) / total];
                left = column[1] + gap;
                if self.cut_words {
                    // Every word whole; the region cuts it pixel by pixel
                    // (`TypeFill::coverage`).
                    if column[1] - column[0] >= least {
                        out.push(Piece {
                            word: k,
                            rect: [column[0], band[0], column[1], band[1]],
                            clip: column,
                        });
                    }
                    continue;
                }
                let parts: Vec<[f32; 2]> = free
                    .get(r)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .filter_map(|span| {
                        // Inset from the region's own edges, not the frame's.
                        let s0 = if span[0] > 0.0 {
                            span[0] + gap * 0.5
                        } else {
                            span[0]
                        };
                        let s1 = if span[1] < width {
                            span[1] - gap * 0.5
                        } else {
                            span[1]
                        };
                        let (x0, x1) = (column[0].max(s0), column[1].min(s1));
                        (x1 > x0).then_some([x0, x1])
                    })
                    .collect();
                let (Some(first), Some(last)) = (parts.first(), parts.last()) else {
                    continue;
                };
                let across = [first[0], last[1]];
                if across[1] - across[0] < least {
                    continue;
                }
                for part in &parts {
                    out.push(Piece {
                        word: k,
                        rect: [across[0], band[0], across[1], band[1]],
                        clip: *part,
                    });
                }
            }
        }
        out
    }
}

/// Type fill's behaviour. No CPU reference through the trait: the Text layer
/// and the Region layer's picture are not in the bag.
pub struct TypeFillDef;

impl EffectDef for TypeFillDef {
    fn schema(&self) -> &'static EffectSchema {
        &<TypeFill as EffectMetadata>::SCHEMA
    }

    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: &[],
        }
    }

    fn reads_text_layer(&self) -> bool {
        true
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(TypeFill::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Piece, TypeFill};
    use crate::fx::params::Params;
    use crate::fx::registry::EffectMetadata;

    /// SICELIOT's Type fill: rows split the height, columns are as wide as
    /// their words, a shape in the middle of a row leaves each column only
    /// the parts outside it (inset by Word gap), a part narrower than Min
    /// width stays empty, Invert region swaps inside and out, and the orders
    /// move or shuffle the words. Guards words drawn over the shape, or
    /// boxes that do not fill the room.
    #[test]
    fn words_fill_the_room_the_region_leaves() {
        let mut t = TypeFill::read(Params::EMPTY);
        t.rows = 2;
        t.row_gap = 10.0;
        t.word_gap = 10.0;
        t.min_width = 20.0;
        assert_eq!(t.word_list(), vec!["Flood", "the", "Grid"]);
        let bands = t.row_bands(110.0, 1.0);
        assert_eq!(bands, vec![[0.0, 50.0], [60.0, 110.0]]);

        // A row 400 px wide with the region everywhere but 150..250.
        let samples: Vec<f32> = (0..400)
            .map(|x| if (150..250).contains(&x) { 0.0 } else { 1.0 })
            .collect();
        let free = t.free_spans(&samples);
        assert_eq!(free, vec![[0.0, 150.0], [250.0, 400.0]]);
        // Words 2, 1 and 1 wide: columns 0..190, 200..295, 305..400.
        let natural = [2.0, 1.0, 1.0];
        let pieces = t.pieces(
            &bands[..1],
            std::slice::from_ref(&free),
            &natural,
            400.0,
            1.0,
        );
        assert_eq!(
            pieces,
            vec![
                Piece {
                    word: 0,
                    rect: [0.0, 0.0, 145.0, 50.0],
                    clip: [0.0, 145.0]
                },
                Piece {
                    word: 1,
                    rect: [255.0, 0.0, 295.0, 50.0],
                    clip: [255.0, 295.0]
                },
                Piece {
                    word: 2,
                    rect: [305.0, 0.0, 400.0, 50.0],
                    clip: [305.0, 400.0]
                },
            ]
        );
        // A free stretch of 15 px is too narrow for a word.
        let narrow = vec![vec![[0.0, 15.0]]];
        assert!(t
            .pieces(&bands[..1], &narrow, &natural, 400.0, 1.0)
            .is_empty());

        // A shape at 230..250 cuts "the" (200..295) in two: one "the" across
        // the whole column, its left slice on one side, its right on the
        // other, never a second "the".
        let cut: Vec<f32> = (0..400)
            .map(|x| if (230..250).contains(&x) { 0.0 } else { 1.0 })
            .collect();
        let pieces = t.pieces(&bands[..1], &[t.free_spans(&cut)], &natural, 400.0, 1.0);
        assert_eq!(
            pieces[1..3],
            [
                Piece {
                    word: 1,
                    rect: [200.0, 0.0, 295.0, 50.0],
                    clip: [200.0, 225.0]
                },
                Piece {
                    word: 1,
                    rect: [200.0, 0.0, 295.0, 50.0],
                    clip: [255.0, 295.0]
                },
            ]
        );

        let mut inverted = t;
        inverted.invert = true;
        assert_eq!(inverted.free_spans(&samples), vec![[150.0, 250.0]]);

        // Cut words: every word whole at its column's width, wherever the
        // region is; the region then cuts it pixel by pixel, and a Type fill
        // and its inverted twin share each edge pixel exactly.
        let mut cut_out = t;
        cut_out.cut_words = true;
        let mut cut_in = inverted;
        cut_in.cut_words = true;
        let whole = cut_out.pieces(&bands[..1], &[vec![]], &natural, 400.0, 1.0);
        assert_eq!(
            whole.iter().map(|p| (p.word, p.clip)).collect::<Vec<_>>(),
            vec![(0, [0.0, 190.0]), (1, [200.0, 295.0]), (2, [305.0, 400.0])]
        );
        for v in [0.0, 0.3, 0.5, 0.7, 1.0] {
            let sum = cut_out.coverage(v) + cut_in.coverage(v);
            assert!((sum - 1.0).abs() < 1e-6, "edge pixel {v} shared");
        }
        assert!(cut_out.coverage(1.0) == 1.0 && cut_out.coverage(0.0) == 0.0);

        let mut shifted = t;
        shifted.order = 1;
        assert_eq!(shifted.row_order(1, 3), vec![1, 2, 0]);
        let mut random = t;
        random.order = 2;
        let a = random.row_order(4, 3);
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2], "a shuffle of the words");
        assert_eq!(a, random.row_order(4, 3), "the same every frame");
    }
}
