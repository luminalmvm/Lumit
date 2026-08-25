//! Tile (docs/08 §3.39): one rectangle of the picture repeated across the
//! frame — AE's Motion Tile.
//!
//! **In plain terms.** Cut a rectangle out of the picture and stamp it side by
//! side until the frame is full. Tile width and height say how big the rectangle
//! is in pixels (half the frame gives a 2×2 grid), Tile centre says where it is
//! cut from, and Output width and height say how much gets stamped. Narrower
//! than the frame and the rest of it goes transparent; wider and the stamps carry
//! on past the frame's edges and the working picture grows to hold them, which
//! is what lets a warp or a blur further down the stack find picture there
//! instead of nothing. Mirror edges flips alternate stamps so they meet without
//! a seam, and Phase slides every other row along so the grid stops reading as a
//! grid.
//!
//! Everything starts at the identity (K-542): one whole-frame tile, cut from the
//! middle, stamped over exactly the frame it came from. Dropping Tile on a layer
//! changes nothing until a number is moved.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Tile's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "tile",
    label = "Tile",
    // 2: the tile's size and the output window's size crossed from per cents of
    // the frame to px@comp (K-558). `migrate_percent_to_px` converts a v1
    // instance on load.
    version = 2,
    category = Distortion,
    // One bilinear tap a pixel; the arithmetic before it is a handful of
    // multiplies and a floor.
    cost = Cheap,
    // A tile is cut from anywhere in the frame and stamped anywhere else.
    roi = FullFrame,
    premultiplied = true,
)]
pub struct Tile {
    /// px@comp: the centre of the rectangle that gets stamped. The schema
    /// default is nominal 1080p centre; `instantiate_for_raster` centres a fresh
    /// instance on the actual comp, which is what makes the default the exact
    /// identity on a comp of any size (K-542).
    #[slider(label = "Tile centre x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub tile_centre_x: f32,

    /// px@comp; see [`tile_centre_x`](Self::tile_centre_x).
    #[slider(label = "Tile centre y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub tile_centre_y: f32,

    /// px@comp: how wide the stamped rectangle is — half the frame's width is a
    /// 2×2 repeat. A size is a distance, so it is pixels and not a share of the
    /// frame (K-558, which supersedes K-542's per-cent rationale for the pair).
    ///
    /// The declared default is a nominal 1080p frame's width, and
    /// `instantiate_for_raster` writes the actual comp's, so a fresh Tile is one
    /// whole-frame tile cut from the middle of the frame — AE's Motion Tile, and
    /// the exact identity on a comp of any size (K-542's lands-as-identity, kept
    /// the way the centre already keeps it).
    #[slider(
        label = "Tile width",
        min = 1.0,
        max = 3840.0,
        default = 1920.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub tile_width: f32,

    /// px@comp, the frame's height by default; see [`tile_width`](Self::tile_width).
    #[slider(
        label = "Tile height",
        min = 1.0,
        max = 2160.0,
        default = 1080.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub tile_height: f32,

    /// px@comp: how wide the stamped area is, centred on the frame. Narrower
    /// than the frame and the output is transparent outside it; the frame's own
    /// width covers it exactly, which is the default (a nominal 1080p frame's,
    /// with `instantiate_for_raster` writing the comp's own). **Wider than the
    /// frame and the working picture grows** (K-542): the stamps carry on past
    /// the frame's edges into a wider
    /// raster, and every effect after this one in the stack runs on that raster,
    /// so the copies are real picture to them rather than transparency. The
    /// composite places the wider picture by the layer's own transform, so
    /// nothing moves — the layer simply reaches further.
    #[slider(
        label = "Output width",
        min = 1.0,
        max = 3840.0,
        default = 1920.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub output_width: f32,

    /// px@comp, the frame's height by default; see [`output_width`](Self::output_width).
    #[slider(
        label = "Output height",
        min = 1.0,
        max = 2160.0,
        default = 1080.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub output_height: f32,

    /// Flip alternate stamps instead of butting copies together, which is what
    /// makes a tiled texture seamless without a seamless source.
    #[toggle(label = "Mirror edges", default = false)]
    pub mirror_edges: bool,

    /// Degrees: how far each row of tiles is slid along, as a fraction of a
    /// tile — 180° is the brickwork offset.
    #[dial(default = 0.0, step = 45.0)]
    pub phase: f32,

    /// Slide the *columns* vertically rather than the rows horizontally.
    #[toggle(label = "Horizontal phase shift", default = false)]
    pub horizontal_phase_shift: bool,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
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

impl Tile {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The four sizes are px@comp (K-558) and both kernels want fractions of the
    /// raster — a fraction is what lets the same resolved op be handed a raster
    /// of another size — so the division happens once, here, against the raster
    /// being drawn on. Sizes and raster carry the same preview factor, so a Half
    /// preview tiles exactly as the export does. Phase becomes a fraction of a
    /// tile, and the two switches become flags.
    #[must_use]
    pub fn packed(self, raster_w: f32, raster_h: f32) -> cpu::TileParams {
        let (rw, rh) = (raster_w.max(1.0), raster_h.max(1.0));
        cpu::TileParams {
            centre: [self.tile_centre_x, self.tile_centre_y],
            tile_frac: [
                self.tile_width.max(1e-3) / rw,
                self.tile_height.max(1e-3) / rh,
            ],
            output_frac: [
                self.output_width.max(1e-3) / rw,
                self.output_height.max(1e-3) / rh,
            ],
            phase: self.phase / 360.0,
            mirror_edges: self.mirror_edges,
            horizontal_phase_shift: self.horizontal_phase_shift,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Tile's behaviour.
pub struct TileDef;

impl EffectDef for TileDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Tile as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::tile(rgba, w, h, &Tile::read(p).packed(w as f32, h as f32));
    }
}
