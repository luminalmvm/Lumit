//! Type fill fills the frame with rows of words that stretch to
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
    registry::EffectMetadata, registry::ResolveCx, registry::RowState, registry::Signature,
    schema::EffectSchema,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// What Order offers.
pub const TYPE_FILL_ORDER: &[&str] = &["In turn", "Shift per row", "Random"];

/// What Direction offers.
pub const TYPE_FILL_DIRECTION: &[&str] = &["Rows", "Columns"];

/// What the rows are called when Direction is Columns, where each works down
/// a column instead of along a row.
pub const COLUMN_LABELS: &[(&str, &str)] = &[
    ("rows", "Columns"),
    ("grid_height", "Grid width"),
    ("row_gap", "Column gap"),
    ("row_open", "Column open"),
    ("row_stagger", "Column stagger"),
    ("row_shift", "Column shift"),
    ("alternate_rows", "Alternate columns"),
];

/// Where the region is read from.
pub const REGION_FROM: &[&str] = &["Alpha", "Luminance"];

/// The dice Random order rolls.
const ORDER_ATTR: u32 = 130;

/// Type fill's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "type_fill",
    label = "Type fill",
    version = 1,
    category = Generate,
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

    /// How many times Words is set one after another in every row: a short
    /// list fills a wide row without being typed again.
    #[counter(
        label = "Repeat words",
        min = 1,
        max = 64,
        default = 1,
        hard_min = 1,
        hard_max = 256,
        unit = Raw
    )]
    pub repeat_words: i32,

    /// Rows: the words run left to right in rows, one under another.
    /// Columns: they stand one under another in columns side by side, still
    /// upright; every row below then works down a column instead (Rows,
    /// Grid height and Row gap across the frame, Row shift up and down).
    #[choice(label = "Direction", options = TYPE_FILL_DIRECTION, default = 0)]
    pub direction: u32,

    /// How wide each word's column is, as numbers in the order of Words,
    /// between bars, dashes or spaces (`46|8|40`, `46-8-40`): shares of the
    /// width, not pixels. A shorter list goes round again (`1` makes every
    /// column as wide). Empty, or a number that is not above 0, makes each
    /// column as wide as its word naturally is.
    #[text(label = "Column widths", default = "")]
    pub column_widths: ShortText,

    /// How far the columns go from the words' own widths (0 %) to Column
    /// widths (100 %). Keyed, the columns slide from one to the other.
    #[slider(
        label = "Column mix",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub column_mix: f32,

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

    /// Give every piece the region leaves in a cell its own whole word,
    /// fitted to the piece's box across and down, as Cavalry's Contours to
    /// Sub-Meshes and Mesh distribution do: a piece the shape's edge makes
    /// short gets a short word, and nothing is cut.
    #[toggle(label = "Fit piece", default = false)]
    pub fit_piece: bool,

    /// How much of its piece's box a word fills with Fit piece, per cent.
    #[slider(
        label = "Piece scale",
        min = 0.0,
        max = 100.0,
        default = 95.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub piece_scale: f32,

    /// With Fit piece, cut the words where the region stops, pixel by pixel,
    /// so none reaches past a curved or slanted edge.
    #[toggle(label = "Clip to region", default = false)]
    pub clip_to_region: bool,

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

    /// How tall the rows stand together, centred in the frame, px@comp; 0
    /// is the frame's height. Keyed with Rows, one row grows into a grid.
    #[slider(
        label = "Grid height",
        min = 0.0,
        max = 4000.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub grid_height: f32,

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

    /// How open the rows are, per cent: each row grows out of a flat line
    /// through its middle. With Row stagger they open one after another.
    #[slider(
        label = "Row open",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub row_open: f32,

    /// How far apart the rows open, per cent: 0 opens them all together;
    /// higher opens the middle row first and the outer rows last, and closes
    /// them the other way round.
    #[slider(
        label = "Row stagger",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub row_stagger: f32,

    /// How far every row slides sideways, px@comp; the words wrap round, so
    /// keyed or driven by time the rows scroll without end.
    #[slider(label = "Row shift", min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub row_shift: f32,

    /// Every other row slides the other way.
    #[toggle(label = "Alternate rows", default = false)]
    pub alternate_rows: bool,

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

    /// The words between the bars, trimmed, empty ones left out, the list
    /// set Repeat words times.
    #[must_use]
    pub fn word_list(&self) -> Vec<String> {
        let once: Vec<String> = self
            .words
            .as_str()
            .split('|')
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect();
        #[allow(clippy::cast_sign_loss)]
        let times = self.repeat_words.clamp(1, 256) as usize;
        once.iter()
            .cycle()
            .take(once.len() * times)
            .cloned()
            .collect()
    }

    /// The top and bottom of each row in a raster `height` px high.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
    pub fn row_bands(&self, height: f32, px_scale: f32) -> Vec<[f32; 2]> {
        let n = self.rows.clamp(1, 400) as usize;
        let gap = (self.row_gap * px_scale).max(0.0);
        let grid = if self.grid_height > 0.0 {
            (self.grid_height * px_scale).min(height)
        } else {
            height
        };
        let first = (height - grid) * 0.5;
        let row = ((grid - gap * (n - 1) as f32) / n as f32).max(0.0);
        (0..n)
            .map(|r| {
                let top = first + r as f32 * (row + gap);
                let half = row * 0.5 * self.row_openness(r, n);
                let middle = top + row * 0.5;
                [middle - half, middle + half]
            })
            .collect()
    }

    /// How open row `r` of `n` is, 0..1: Row open, each row starting the
    /// later the farther it is from the middle, by Row stagger.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn row_openness(&self, r: usize, n: usize) -> f32 {
        let open = (self.row_open / 100.0).clamp(0.0, 1.0);
        let stagger = (self.row_stagger / 100.0).clamp(0.0, 1.0);
        let centre = (n.max(1) - 1) as f32 * 0.5;
        let far = if centre > 0.0 {
            (r as f32 - centre).abs() / centre
        } else {
            0.0
        };
        ((open - stagger * far) / (1.0 - stagger).max(1e-3)).clamp(0.0, 1.0)
    }

    /// How far row `r` slides sideways, raster px: Row shift, the other way
    /// on odd rows with Alternate rows.
    #[must_use]
    pub fn row_offset(&self, r: usize, px_scale: f32) -> f32 {
        let shift = self.row_shift * px_scale;
        if self.alternate_rows && r % 2 == 1 {
            -shift
        } else {
            shift
        }
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
            let height = band[1] - band[0];
            if height <= 0.0 {
                continue;
            }
            for (k, column) in self.columns(r, natural, width, gap, self.row_offset(r, px_scale)) {
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

    /// The share of the width each word's column takes: Column widths when
    /// it gives positive numbers, gone round as often as the words need,
    /// else `natural`.
    #[must_use]
    pub fn shares(&self, natural: &[f32]) -> Vec<f32> {
        let list: Vec<Option<f32>> = self
            .column_widths
            .as_str()
            .split(|c: char| c == '|' || c == '-' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<f32>().ok().filter(|v| *v > 0.0))
            .collect();
        let given: Vec<f32> = if list.iter().all(Option::is_some) {
            list.iter()
                .flatten()
                .copied()
                .cycle()
                .take(natural.len())
                .collect()
        } else {
            Vec::new()
        };
        if !given.is_empty() && given.len() == natural.len() {
            let mix = (self.column_mix / 100.0).clamp(0.0, 1.0);
            if mix >= 1.0 {
                return given;
            }
            // Both as shares of one, mixed.
            let g: f32 = given.iter().sum();
            let n: f32 = natural.iter().map(|v| v.max(1e-3)).sum();
            given
                .iter()
                .zip(natural)
                .map(|(a, b)| (1.0 - mix) * b.max(1e-3) / n + mix * a / g)
                .collect()
        } else {
            natural.iter().map(|v| v.max(1e-3)).collect()
        }
    }

    /// Row `r`'s columns across `width`, left to right: which word, and its
    /// left and right, as wide as [`TypeFill::shares`] says, Word gap (`gap`,
    /// raster px) between them.
    /// Moved `offset` raster px (see [`TypeFill::row_offset`]), the row
    /// wraps round: the words repeat every width plus one Word gap, and only
    /// the columns that reach into the width are kept.
    fn columns(
        &self,
        r: usize,
        natural: &[f32],
        width: f32,
        gap: f32,
        offset: f32,
    ) -> Vec<(usize, [f32; 2])> {
        let n = natural.len();
        let order = self.row_order(r, n);
        let shares = self.shares(natural);
        #[allow(clippy::cast_precision_loss)]
        let room = (width - gap * (n.saturating_sub(1)) as f32).max(0.0);
        let total: f32 = order.iter().map(|&k| shares[k]).sum();
        let mut left = 0.0f32;
        let row: Vec<(usize, [f32; 2])> = order
            .into_iter()
            .map(|k| {
                let column = [left, left + room * shares[k] / total];
                left = column[1] + gap;
                (k, column)
            })
            .collect();
        let period = width + gap;
        let shift = if period > 0.0 {
            offset.rem_euclid(period)
        } else {
            0.0
        };
        if shift == 0.0 {
            return row;
        }
        [shift - period, shift]
            .iter()
            .flat_map(|&d| row.iter().map(move |&(k, c)| (k, [c[0] + d, c[1] + d])))
            .filter(|(_, c)| c[1] > 0.0 && c[0] < width)
            .collect()
    }

    /// Every word's box with Fit piece. `reach[r][x]` is how far down row
    /// r's band the region reaches at pixel column x, top and bottom, or
    /// `None` where it does not; the other arguments are as
    /// [`TypeFill::pieces`] takes them.
    ///
    /// A row's region is cut by its columns into pieces, one for each run of
    /// pixel columns the region reaches (Word gap is only the room between
    /// columns, not an inset at the region's edges);
    /// each piece narrower than Min width stays empty, and each other
    /// gets its column's word whole, fitted to the piece's box (as wide as
    /// the run, as tall as the region reaches over it) at Piece scale.
    #[must_use]
    pub fn fit_pieces(
        &self,
        reach: &[Vec<Option<[f32; 2]>>],
        natural: &[f32],
        width: f32,
        px_scale: f32,
    ) -> Vec<Piece> {
        if natural.is_empty() {
            return Vec::new();
        }
        let gap = (self.word_gap * px_scale).max(0.0);
        let least = (self.min_width * px_scale).max(1.0);
        let scale = (self.piece_scale / 100.0).clamp(0.0, 1.0);
        let mut out = Vec::new();
        for (r, row) in reach.iter().enumerate() {
            let spans = runs(row);
            for (k, column) in self.columns(r, natural, width, gap, self.row_offset(r, px_scale)) {
                for span in &spans {
                    // The region's own edges are the pieces' edges: Word
                    // gap is only the room between columns.
                    #[allow(clippy::cast_precision_loss)]
                    let (s0, s1) = (span[0] as f32, span[1] as f32);
                    let (x0, x1) = (column[0].max(s0), column[1].min(s1));
                    if x1 - x0 < least {
                        continue;
                    }
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let cols = (x0.floor().max(0.0) as usize)..(x1.ceil() as usize).min(row.len());
                    let (mut top, mut bottom) = (f32::MAX, f32::MIN);
                    for [t, b] in row[cols].iter().flatten() {
                        top = top.min(*t);
                        bottom = bottom.max(*b);
                    }
                    if bottom <= top {
                        continue;
                    }
                    let (cx, cy) = ((x0 + x1) * 0.5, (top + bottom) * 0.5);
                    let (hw, hh) = ((x1 - x0) * 0.5 * scale, (bottom - top) * 0.5 * scale);
                    out.push(Piece {
                        word: k,
                        rect: [cx - hw, cy - hh, cx + hw, cy + hh],
                        clip: [cx - hw, cx + hw],
                    });
                }
            }
        }
        out
    }
}

/// The runs of pixel columns a row's region reaches, start and end.
fn runs(row: &[Option<[f32; 2]>]) -> Vec<[usize; 2]> {
    let mut out = Vec::new();
    let mut start = None;
    for (x, v) in row.iter().enumerate() {
        match (v.is_some(), start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                out.push([s, x]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push([s, row.len()]);
    }
    out
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

    /// Fit piece keeps every word whole, so Cut words does nothing beside
    /// it; Piece scale and Clip to region are only read by Fit piece.
    fn row_state(&self, inst: &EffectInstance) -> RowState {
        let fit = matches!(inst.param("fit_piece"), Some(EffectValue::Bool(true)));
        RowState {
            disabled: if fit {
                vec!["cut_words"]
            } else {
                vec!["piece_scale", "clip_to_region"]
            },
            labels: if matches!(inst.param("direction"), Some(EffectValue::Choice(1))) {
                COLUMN_LABELS.to_vec()
            } else {
                Vec::new()
            },
            ..RowState::default()
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(TypeFill::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
