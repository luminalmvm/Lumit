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
mod tests {
    use super::{Piece, TypeFill};
    use crate::fx::params::Params;
    use crate::fx::registry::EffectMetadata;

    /// Type fill: rows split the height, columns are as wide as
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

    /// Grid height stands the rows together in a band of that height in the
    /// middle of the frame, so one row keyed into forty grows into a grid.
    #[test]
    fn grid_height_centres_the_rows() {
        let mut t = TypeFill::read(Params::EMPTY);
        t.rows = 2;
        t.row_gap = 0.0;
        t.grid_height = 40.0;
        assert_eq!(t.row_bands(100.0, 1.0), vec![[30.0, 50.0], [50.0, 70.0]]);
        t.grid_height = 0.0;
        assert_eq!(t.row_bands(100.0, 1.0), vec![[0.0, 50.0], [50.0, 100.0]]);
    }

    /// Fit piece: a shape that cuts a column in two leaves two pieces, each
    /// with the whole word, fitted to its own box across and down at Piece
    /// scale; a piece the region reaches only part of the way down gets a
    /// short word. Guards a word cut by the shape, or stretched over it.
    #[test]
    fn fit_piece_gives_each_piece_its_own_whole_word() {
        let mut t = TypeFill::read(Params::EMPTY);
        t.words = crate::fx::params::ShortText::new("a");
        t.word_gap = 0.0;
        t.min_width = 1.0;
        t.piece_scale = 50.0;
        // One row 0..20 across 10 px: free at 0..4 all the way down, held
        // at 4..6, free at 6..10 only from 10 to 20.
        let mut row = vec![Some([0.0, 20.0]); 4];
        row.extend([None, None]);
        row.extend(vec![Some([10.0, 20.0]); 4]);
        let pieces = t.fit_pieces(&[row], &[1.0], 10.0, 1.0);
        assert_eq!(
            pieces,
            vec![
                Piece {
                    word: 0,
                    rect: [1.0, 5.0, 3.0, 15.0],
                    clip: [1.0, 3.0]
                },
                Piece {
                    word: 0,
                    rect: [7.0, 12.5, 9.0, 17.5],
                    clip: [7.0, 9.0]
                },
            ]
        );
    }

    /// Column widths sets each column's share of the width; a list that is
    /// not one positive number for each word is left out.
    #[test]
    fn column_widths_set_the_columns() {
        let mut t = TypeFill::read(Params::EMPTY);
        assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![2.0, 1.0, 2.0]);
        t.column_widths = crate::fx::params::ShortText::new("46|8|40");
        assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![46.0, 8.0, 40.0]);
        t.column_widths = crate::fx::params::ShortText::new("46-8 40");
        assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![46.0, 8.0, 40.0]);
        // A shorter list goes round again.
        t.column_widths = crate::fx::params::ShortText::new("46|8");
        assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![46.0, 8.0, 46.0]);
        t.column_widths = crate::fx::params::ShortText::new("1");
        assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![1.0, 1.0, 1.0]);
        // A number that is not above 0 leaves the list out.
        t.column_widths = crate::fx::params::ShortText::new("46|x");
        assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![2.0, 1.0, 2.0]);
        t.column_widths = crate::fx::params::ShortText::new("46|0");
        assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![2.0, 1.0, 2.0]);
    }

    /// Direction: Columns renames the rows that then work down the columns,
    /// and Rows keeps their own names.
    #[test]
    fn columns_rename_the_rows() {
        use crate::fx::registry::EffectDef;
        let mut inst = crate::fx::builtins::instantiate("type_fill").unwrap();
        assert!(super::TypeFillDef.row_state(&inst).labels.is_empty());
        inst.params.retain(|p| p.id != "direction");
        inst.params.push(crate::model::EffectParam {
            id: "direction".to_owned(),
            value: crate::model::EffectValue::Choice(1),
            extra: serde_json::Map::new(),
        });
        let labels = super::TypeFillDef.row_state(&inst).labels;
        assert!(labels.contains(&("rows", "Columns")));
        assert!(labels.contains(&("row_shift", "Column shift")));
        assert!(labels.contains(&("grid_height", "Grid width")));
    }

    /// Repeat words sets the list one after another that many times.
    #[test]
    fn repeat_words_sets_the_list_again() {
        let mut t = TypeFill::read(Params::EMPTY);
        t.words = crate::fx::params::ShortText::new("a|b");
        assert_eq!(t.word_list(), vec!["a", "b"]);
        t.repeat_words = 3;
        assert_eq!(t.word_list(), vec!["a", "b", "a", "b", "a", "b"]);
    }

    /// Column mix goes from the words' own widths (0 %) to Column widths
    /// (100 %) through shares of one. Guards a jump when it is keyed.
    #[test]
    fn column_mix_slides_between_the_widths() {
        let mut t = TypeFill::read(Params::EMPTY);
        t.column_widths = crate::fx::params::ShortText::new("1|1");
        let natural = [3.0, 1.0];
        t.column_mix = 0.0;
        assert_eq!(t.shares(&natural), vec![0.75, 0.25]);
        t.column_mix = 50.0;
        assert_eq!(t.shares(&natural), vec![0.625, 0.375]);
        t.column_mix = 100.0;
        assert_eq!(t.shares(&natural), vec![1.0, 1.0]);
    }

    /// Row open grows every row out of a flat line through its middle; with
    /// Row stagger the middle row opens first and the outer rows last.
    #[test]
    fn row_open_and_stagger_open_the_rows_from_the_middle() {
        let mut t = TypeFill::read(Params::EMPTY);
        t.rows = 3;
        t.row_gap = 0.0;
        t.row_open = 50.0;
        assert_eq!(
            t.row_bands(90.0, 1.0),
            vec![[7.5, 22.5], [37.5, 52.5], [67.5, 82.5]]
        );
        t.row_stagger = 50.0;
        // The middle row is open, the outer ones still flat.
        assert_eq!(
            t.row_bands(90.0, 1.0),
            vec![[15.0, 15.0], [30.0, 60.0], [75.0, 75.0]]
        );
        t.row_open = 75.0;
        assert!((t.row_openness(0, 3) - 0.5).abs() < 1e-6);
        assert!((t.row_openness(2, 3) - 0.5).abs() < 1e-6);
        t.row_open = 100.0;
        assert_eq!(t.row_bands(90.0, 1.0)[0], [0.0, 30.0], "all open at 100 %");
    }

    /// Row shift slides a row's columns and wraps them round every width
    /// plus one Word gap; Alternate rows slides the odd rows the other way.
    /// Guards a gap opening at the edge as the row scrolls.
    #[test]
    fn row_shift_scrolls_the_rows_round() {
        let mut t = TypeFill::read(Params::EMPTY);
        t.words = crate::fx::params::ShortText::new("a|b");
        t.word_gap = 0.0;
        t.min_width = 1.0;
        let natural = [1.0, 1.0];
        let full = |t: &TypeFill, r: usize| {
            let reach = vec![vec![Some([0.0, 10.0]); 100]; r + 1];
            t.fit_pieces(&reach, &natural, 100.0, 1.0)
                .into_iter()
                .filter(|p| p.rect[1] >= 0.0)
                .map(|p| (p.word, p.clip))
                .collect::<Vec<_>>()
        };
        t.piece_scale = 100.0;
        t.row_shift = 20.0;
        // "b" wraps in from the left: 0..20, then "a" 20..70, "b" 70..100.
        let first: Vec<_> = full(&t, 0);
        assert_eq!(
            first,
            vec![(1, [0.0, 20.0]), (0, [20.0, 70.0]), (1, [70.0, 100.0])]
        );
        t.row_shift = 120.0;
        assert_eq!(full(&t, 0), first, "a whole width round is the same row");
        t.row_shift = 20.0;
        t.alternate_rows = true;
        assert_eq!(t.row_offset(0, 1.0), 20.0);
        assert_eq!(t.row_offset(1, 1.0), -20.0);
        assert_eq!(t.row_offset(1, 2.0), -40.0, "px@comp");
    }
}
