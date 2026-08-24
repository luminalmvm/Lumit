//! Tile (docs/08 §3.39): one rectangle of the picture repeated across the
//! frame — AE's Motion Tile.
//!
//! **In plain terms.** Cut a rectangle out of the picture and stamp it side by
//! side until the frame is full. Tile width and height say how big the rectangle
//! is (as a per cent of the frame, so 50 gives a 2×2 grid), Tile centre says
//! where it is cut from, Output width and height say how much of the frame gets
//! stamped and the rest goes transparent. Mirror edges flips alternate stamps so
//! they meet without a seam, and Phase slides every other row along so the grid
//! stops reading as a grid.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Tile's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "tile",
    label = "Tile",
    version = 1,
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
    /// instance on the actual comp.
    #[slider(label = "Tile centre x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub tile_centre_x: f32,

    /// px@comp; see [`tile_centre_x`](Self::tile_centre_x).
    #[slider(label = "Tile centre y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub tile_centre_y: f32,

    /// Per cent of the frame's width. 50 is a 2×2 repeat, which is the default
    /// for §1.2's reason: a Tile that had not tiled anything would be a Tile that
    /// had not been applied (§3.39).
    ///
    /// **Not a spatial unit**, deliberately: a per cent of the raster does not
    /// move when the raster does, so this needs no rescaling and gets none.
    #[slider(
        label = "Tile width",
        min = 1.0,
        max = 500.0,
        default = 50.0,
        hard_min = 1.0,
        unit = Percent
    )]
    pub tile_width: f32,

    /// Per cent of the frame's height; see [`tile_width`](Self::tile_width).
    #[slider(
        label = "Tile height",
        min = 1.0,
        max = 500.0,
        default = 50.0,
        hard_min = 1.0,
        unit = Percent
    )]
    pub tile_height: f32,

    /// Per cent of the frame's width: how wide the stamped area is, centred on
    /// the frame. Outside it the output is transparent. 100 covers the frame,
    /// which is the default; anything above 100 also covers it, since the frame
    /// is all there is to cover (§3.39).
    #[slider(
        label = "Output width",
        min = 1.0,
        max = 500.0,
        default = 100.0,
        hard_min = 1.0,
        unit = Percent
    )]
    pub output_width: f32,

    /// Per cent of the frame's height; see [`output_width`](Self::output_width).
    #[slider(
        label = "Output height",
        min = 1.0,
        max = 500.0,
        default = 100.0,
        hard_min = 1.0,
        unit = Percent
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
    /// The four per cents stay per cents here — they are fractions of the raster
    /// the kernel already knows, and turning them into pixels host-side would
    /// mean the kernel could not be handed a different raster. Phase becomes a
    /// fraction of a tile, and the two switches become flags.
    #[must_use]
    pub fn packed(self) -> cpu::TileParams {
        cpu::TileParams {
            centre: [self.tile_centre_x, self.tile_centre_y],
            tile_frac: [
                self.tile_width.max(1e-3) / 100.0,
                self.tile_height.max(1e-3) / 100.0,
            ],
            output_frac: [
                self.output_width.max(1e-3) / 100.0,
                self.output_height.max(1e-3) / 100.0,
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
        cpu::tile(rgba, w, h, &Tile::read(p).packed());
    }
}
