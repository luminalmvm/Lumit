//! Lens flare — the physically-based built-in (docs/08-EFFECTS.md §3.27,
//! docs/impl/lens-flare.md; K-256..K-261).
//!
//! In plain terms: a camera lens is a stack of glass surfaces with an iris
//! somewhere in the middle. A tiny fraction of the light reflects off the
//! inside of one surface, bounces backward, reflects off another, and lands
//! on the sensor anyway — one faint "ghost" per such two-bounce pair. This
//! module simulates that literally, in the FlareSim manner (K-261): for each
//! light source it fires a quasi-random spray of parallel rays across the
//! front of a real lens prescription, refracts each ray surface by surface
//! (reflecting at the pair's two surfaces), and SPLATS every survivor onto
//! the sensor as a point of light. Brightness is ray density — where the
//! optics focus rays into folds and rims, many rays land on the same pixel
//! and it burns bright; nothing is a drawn shape. The starburst is separate
//! physics (diffraction at the iris) and stays a baked Fourier sprite.
//!
//! The bake (pure CPU, cached by [`bake_key`]) parses the selected
//! prescription, enumerates and ranks every ghost pair, measures each pair's
//! defocus spread, renders a thumbnail to close the auto-exposure loop, and
//! bakes the starburst. The per-frame splat runs on the GPU with this CPU
//! implementation as its reference (§1.6 staged oracle, K-114 pattern for
//! the CPU rung).

use super::cie;
use super::fft::{fft2_inplace, fftshift2, Cx};
use super::lens_library::LENS_LIBRARY;

/// Resolved Lens flare parameters (docs/08 §3.27).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensFlareParams {
    /// Light position in RASTER PIXELS (px@comp converted through the §2.3
    /// preview factor at resolve, the Transform-anchor convention — K-260;
    /// point parameters are pixels, never % of frame). May leave the frame —
    /// an off-frame light keeps flaring.
    pub light: [f32; 2],
    /// Master gain on everything the effect adds; 0 is the neutral point
    /// (bit-exact passthrough, pinned by test).
    pub intensity: f32,
    /// Index into [`LENS_LIBRARY`] (clamped by the resolve step).
    pub lens: u32,
    /// The working f-stop: stops the iris down from the lens's native
    /// f-number (scales the stop and the pupil mask together).
    pub fstop: f32,
    /// Focus distance, metres (K-260): shifts the sensor plane by the
    /// thin-lens image shift `f²/(1000·d − f)` mm. Real flares change shape
    /// dramatically with focus. Frame-time (animatable, no rebake); large
    /// values are infinity.
    pub focus_m: f32,
    /// Iris blade count, 3..=16 (host-rounded).
    pub blades: u32,
    /// Iris rotation, degrees.
    pub aperture_rotation_deg: f32,
    /// 0..1: blends the blade polygon toward a circle.
    pub roundness: f32,
    /// 0..1: softens the iris edge (feathers the pupil mask and with it
    /// every ghost's rim).
    pub aperture_softness: f32,
    /// Gain on the ghost train alone.
    pub ghost_intensity: f32,
    /// Softens the rendered ghosts (K-261): a box-blur radius as a
    /// percentage of the frame diagonal (3 passes approximate a Gaussian).
    /// This is FlareSim's Ghost Blur — a touch of out-of-focus softness
    /// that also hides the point-splat grain at lower qualities.
    pub ghost_softness: f32,
    /// How many of the brightest-ranked ghost pairs render, 0..=200.
    pub max_ghosts: u32,
    /// Scales each traced wavelength's offset from the spectrum midpoint:
    /// 0 = monochrome trace (no fringing), 1 = physical, 2 = doubled.
    pub dispersion: f32,
    /// 0..1: blends every reflection from plain Fresnel (uncoated, bright
    /// neutral ghosts) toward the prescription's own anti-reflective
    /// coating (K-261: per-surface MgF₂ layer counts from the lens file).
    pub coating: f32,
    /// Gain on the starburst alone.
    pub starburst_intensity: f32,
    /// Scale of the WHOLE flare about the optical centre (ghost train and
    /// starburst together); 1 is natural size.
    pub scale: f32,
    /// Where the light comes from: 0 Manual (the light point above),
    /// 1 Matte (bright sources detected in a referenced layer), 2 Lights
    /// (prepared for light layers; resolves as Manual until they land).
    pub source: u32,
    /// Manual mode: the half-width and half-height of the emitting area, in
    /// raster pixels (px@comp, K-260). Zero — the default — is the point
    /// source the effect has always had; anything larger is an AREA source,
    /// rendered by sampling the flare across it so its ghosts take the shape
    /// of the source rather than of a point (K-355). Matte mode measures this
    /// from the detected source instead and ignores the dial.
    pub source_size: [f32; 2],
    /// **Lights mode** (`source == 2`, K-360): the comp's own Light layers,
    /// resolved at this frame and already in raster fractions. Filled by the
    /// draw builder, which is the only place that can see the rest of the
    /// composition; empty in every other mode, and an empty list in Lights
    /// mode is the labelled no-op — a comp with no lights flares with nothing
    /// rather than falling back to the Manual point, which would put a flare
    /// somewhere nobody asked for.
    ///
    /// A fixed array rather than a `Vec` so these params stay `Copy`, which
    /// the bake cache and the frame-key hash both rely on.
    pub lights: [FlareLight; MAX_SOURCES],
    pub light_count: u32,
    /// Matte mode: linear luma at/above which a detected source flares fully
    /// (open above; a soft gate, see `threshold_softness`).
    pub threshold: f32,
    /// Matte mode: half-width of the soft gate around the threshold.
    pub threshold_softness: f32,
    /// Scene-linear RGB multiplying every light's colour, in every source
    /// mode (K-259): in Manual it *is* the flare's colour (the light is
    /// otherwise white); in Matte it tints what the sources contribute.
    pub light_tint: [f32; 3],
    /// Matte/Lights: whether a detected source's own colour tints its flare.
    /// Off, every source flares white through [`Self::light_tint`] alone —
    /// what a matte used purely as a position mask wants. Ignored in Manual
    /// (there is no source colour to take).
    pub use_source_colour: bool,
    /// Horizontal stretch of the whole flare about the frame centre
    /// (1 = spherical, 1.33/2 = anamorphic looks).
    pub anamorphic: f32,
    /// 0 Draft, 1 Normal, 2 High, 3 Ultra (pupil sample density and
    /// wavelength count; Draft renders the flare buffer at half resolution).
    pub quality: u32,
    /// Ray-budget multiplier on the Quality tier's pupil grid, 0.25..4
    /// (K-265, owner-asked): the tiers pick a sensible base, and this dial
    /// hands the trade to the user when a lens needs more (or a preview
    /// less) without changing wavelength count or buffer resolution.
    /// Frame-time — never rebakes.
    pub detail: f32,
    /// How the flare element combines with the layer under it — an index
    /// into [`BLEND_OPTIONS`] (K-289, replacing the old Transparent/Black
    /// Background choice). Default [`BLEND_ADD`], the old Transparent
    /// behaviour exactly; [`BLEND_NORMAL`] shows the flare alone on its own
    /// opaque black background, which is what the old Black option was for.
    /// Applies only while the effect is live: the Intensity-0 / Mix-0
    /// passthroughs stay bit-exact whatever this holds.
    pub blend: u32,
    /// 0..1.
    pub mix: f32,
}

/// The Blend menu's options, in code order (K-289, docs/08 §3.27). The
/// flare is a black-backed light *element*: everything the effect renders
/// lands on a frame that is pure black where there is no flare, and this
/// menu says how that element combines with the layer beneath it — the same
/// question a layer's own Mode dropdown asks, and the same curated set Echo
/// offers for the same reason (K-149, T21: the HSL / burn / dodge modes are
/// ill-defined on a premultiplied light overlay, so they are not listed).
///
/// [`BLEND_NORMAL`] heads the list because it is the odd one out: the flare
/// element *replaces* the layer, black background and all, which is exactly
/// the "flare over black" the old Background = Black option existed to
/// export. Everything from Add down leaves the layer visible.
pub const BLEND_OPTIONS: &[&str] = &[
    "Normal",
    "Add",
    "Screen",
    "Multiply",
    "Overlay",
    "Soft light",
    "Hard light",
    "Lighten",
    "Darken",
    "Difference",
    "Exclusion",
    "Subtract",
    "Divide",
];

/// The flare element alone on its opaque black background — index 0 of
/// [`BLEND_OPTIONS`].
pub const BLEND_NORMAL: u32 = 0;
/// Light addition — index 1 of [`BLEND_OPTIONS`], and the default a fresh
/// effect carries (the behaviour every flare had before the menu existed).
pub const BLEND_ADD: u32 = 1;

/// Combine the flare element `e` with the layer under it `d` by
/// [`BLEND_OPTIONS`] index `mode` — the CPU twin of
/// `fx_lens_flare_combine.wgsl`'s `flare_blend`, written in the same
/// arithmetic order so the two agree bit-for-bit (§1.6).
///
/// Both sides are **premultiplied linear RGBA** and every mode runs per
/// channel on all four, exactly as Echo's combine does (K-149) and for the
/// same reason: this is light being added to light, not a perceptual
/// re-encode of a finished picture. `e` is the flare element — its RGB is
/// what the trace and starburst put on the frame, its alpha the coverage
/// that light implies — and `d` is the layer.
///
/// [`BLEND_NORMAL`] ignores `d` entirely and returns the element on opaque
/// black; the alpha clamp at the end is the caller's, not this function's.
pub fn flare_blend(mode: u32, d: [f32; 4], e: [f32; 4]) -> [f32; 4] {
    match mode {
        // Normal: the element replaces the layer, on the opaque black it
        // was rendered against. The old Background = Black, as a blend.
        BLEND_NORMAL => [e[0], e[1], e[2], 1.0],
        // 1..: the light-combine table Echo's combine shares (K-149), the
        // layer as backdrop. Index 1 (Add) is bit-identical to the pre-menu
        // behaviour, so a project that never touched this renders the same.
        m => super::cpu::light_blend(m - 1, d, e),
    }
}

/// Per-quality pupil grid side, traced wavelength count, and flare-buffer
/// scale divisor (docs/08 §3.27's Quality ladder). The pupil grid is the
/// Halton candidate count's square root — the accepted sample count is a
/// little under `side²` after the aperture mask.
pub fn quality_ladder(quality: u32) -> (u32, u32, u32) {
    match quality {
        0 => (32, 3, 2),
        2 => (96, 16, 1),
        3 => (144, 32, 1),
        // Normal must stand on its own as the working tier (K-262): the
        // adaptive per-pair budget spends most of this on the big
        // defocused ghosts, where the cell facets used to show.
        _ => (64, 8, 1),
    }
}

/// The full-frame sensor the trace projects onto, mm (fixed: the lens
/// prescriptions are all full-frame stills/cine designs).
pub const SENSOR_MM: [f32; 2] = [36.0, 24.0];

/// Baked starburst sprite side (power of two — the FFT needs it).
pub const STARBURST_RES: u32 = 256;
/// Spectral samples integrated into the starburst bake.
pub const STARBURST_SAMPLES: u32 = 100;
/// Aperture-image side for the starburst FFT.
pub const APERTURE_RES: u32 = 256;

/// Distinct sources a frame detects (Matte mode's top-K cap; Manual is one).
/// 16 since K-267: area sources anchor on one slot each, and eight anchors
/// starved scenes with several practicals plus an area source.
pub const MAX_SOURCES: usize = 16;

/// Light slots the trace can carry in one frame.
///
/// Four per source since K-355, because a source is no longer always a point:
/// an AREA source is rendered by sampling the flare across its emitting area,
/// and every sample needs a slot of its own. A frame of sixteen point sources
/// still costs sixteen slots; one big softbox spends its slots on being the
/// right shape instead.
pub const MAX_LIGHTS: usize = 64;
/// Detection tile side, raster pixels (impl note §6).
pub const DETECT_TILE: u32 = 32;
/// Non-max suppression radius, in tiles (Chebyshev): one highlight must not
/// spend the whole light budget on its own neighbouring tiles.
pub const SUPPRESS_TILES: i64 = 2;

/// Ghost pairs dimmer than this on the on-axis probe are dropped at bake
/// (FlareSim's `min_intensity`).
pub const PAIR_MIN_INTENSITY: f32 = 1e-7;

/// Ranked pairs a frame can ever render — the Max ghosts parameter's own
/// ceiling (docs/08 §3.27), and so the number of pairs the bake measures an
/// image spread for (K-263). A pair past it keeps the neutral spread of 1.0
/// and would render at the Quality ladder's base grid, which is what an
/// unmeasurable pair has always done.
pub const MAX_RENDERED_PAIRS: usize = 200;
/// Rays start this far in front of the first surface, mm.
pub const START_Z_BACKOFF_MM: f32 = 20.0;

/// One flare source: where it sits (raster fraction) and its colour already
/// multiplied by its gate weight. Manual mode is one white light at the
/// parameter position; Matte mode is the detected top-K.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlareLight {
    /// Position as a fraction of the raster (x right, y down).
    pub pos: [f32; 2],
    /// Source colour times gate weight; all-zero entries are dead slots.
    pub rgb: [f32; 3],
    /// Half-extent of the emitting area, as a fraction of the raster (K-355).
    /// Zero is a point source and behaves exactly as it always did; anything
    /// larger is an AREA source and is rendered by sampling across it — see
    /// [`area_samples`].
    pub extent: [f32; 2],
}

/// How many samples an area source is split into along each axis, at most.
///
/// A flare from an extended source is the sum of the flares of the points
/// making it up, and the failure mode of sampling it too sparsely is not noise
/// but **ghost replication** — you see N overlapping copies of the aperture
/// rather than one smeared ghost. So the count is chosen from the source's own
/// size rather than fixed, and the ceiling is what a 2 s/frame budget affords
/// (docs/NEXT-FEATURES.md entry 1).
pub const AREA_SAMPLES_MAX: u32 = 5;

/// The fraction of the raster below which a source is simply a point.
const AREA_MIN_EXTENT: f32 = 0.004;

/// Split one light into the point sources that make it up (K-355).
///
/// **Why sampling rather than something cleverer.** The flare of an area
/// source is genuinely the integral of the point flares over the emitting
/// area, and at a two-second budget we can afford to evaluate that integral
/// directly instead of approximating it. Each sample carries its share of the
/// source's flux, so total energy is unchanged however finely it is split —
/// a source only ever gets *smoother*, never brighter.
///
/// This is what makes an area light's flare look right: a bar-shaped source
/// draws bar-shaped ghosts, a window draws rectangular ones, because the ghost
/// each sample contributes lands in a slightly different place and their sum
/// carries the source's shape. A single point source returns exactly itself,
/// so nothing about point lights moves.
///
/// The grid is regular and centred, which keeps it deterministic (docs/14) —
/// no jitter, so the same frame renders the same way every time.
pub fn area_samples(light: &FlareLight, max_side: u32) -> Vec<FlareLight> {
    let cap = max_side.clamp(1, AREA_SAMPLES_MAX);
    let side = |e: f32| -> u32 {
        if e < AREA_MIN_EXTENT {
            1
        } else {
            // One sample per AREA_MIN_EXTENT of width, capped — enough that
            // neighbouring samples land closer together than a ghost is wide.
            ((e / AREA_MIN_EXTENT).round() as u32).clamp(1, cap)
        }
    };
    let (nx, ny) = (side(light.extent[0]), side(light.extent[1]));
    if nx <= 1 && ny <= 1 {
        return vec![*light];
    }
    let share = 1.0 / (nx * ny) as f32;
    let mut out = Vec::with_capacity((nx * ny) as usize);
    for iy in 0..ny {
        for ix in 0..nx {
            // Centred on the source: the samples span ±extent about `pos`,
            // which is the standard deviation of its flux, so they cover the
            // body of the light rather than its far tails.
            let f = |i: u32, n: u32, e: f32, c: f32| {
                if n <= 1 {
                    return c;
                }
                let t = i as f32 / (n - 1) as f32 * 2.0 - 1.0;
                c + t * e
            };
            out.push(FlareLight {
                pos: [
                    f(ix, nx, light.extent[0], light.pos[0]),
                    f(iy, ny, light.extent[1], light.pos[1]),
                ],
                rgb: [
                    light.rgb[0] * share,
                    light.rgb[1] * share,
                    light.rgb[2] * share,
                ],
                extent: [0.0, 0.0],
            });
        }
    }
    out
}

/// Every light in `lights`, split into its area samples and truncated to the
/// [`MAX_LIGHTS`] the trace can carry. Brighter sources are split first, so a
/// crowded frame spends its slots on the lights that matter.
pub fn expand_area_lights(lights: &[FlareLight], max_side: u32) -> Vec<FlareLight> {
    let mut out: Vec<FlareLight> = Vec::new();
    for light in lights {
        let samples = area_samples(light, max_side);
        if out.len() + samples.len() > MAX_LIGHTS {
            // No room to split this one faithfully: carry it as the single
            // point it started as rather than half an area source, which would
            // lose the rest of its flux.
            if out.len() < MAX_LIGHTS {
                out.push(*light);
            }
            continue;
        }
        out.extend(samples);
    }
    out
}

/// A dead light slot — the value the fixed `lights` array is padded with.
pub const DEAD_LIGHT: FlareLight = FlareLight {
    pos: [0.0, 0.0],
    rgb: [0.0, 0.0, 0.0],
    extent: [0.0, 0.0],
};

/// The frame's light list for whichever source mode is in force.
///
/// Manual is one source at the parameter position (raster pixels over the
/// raster `w × h` — the fraction the trace consumes), carrying the Light tint
/// and the Source size dial's half-extent (K-355; zero is the point source it
/// has always been). **Lights mode** (K-360) hands back the comp's own Light
/// layers instead, which the draw builder resolved into `p.lights`; an area
/// light arrives with a real extent and so flares as its own shape through
/// exactly the machinery K-355 built for detected sources.
///
/// Matte mode never reaches here — its lights are found GPU-side.
pub fn manual_light(p: &LensFlareParams, w: u32, h: u32) -> Vec<FlareLight> {
    if p.source == 2 {
        // A comp with no lights flares with nothing. Falling back to the
        // Manual point would put a flare somewhere nobody asked for.
        let (fw, fh) = (w.max(1) as f32, h.max(1) as f32);
        return p.lights[..(p.light_count as usize).min(MAX_SOURCES)]
            .iter()
            .map(|l| FlareLight {
                // Stored in raster pixels by the resolve, divided here like
                // the Manual position is — one place decides the fraction.
                pos: [l.pos[0] / fw, l.pos[1] / fh],
                rgb: l.rgb,
                extent: [l.extent[0] / fw, l.extent[1] / fh],
            })
            .collect();
    }
    vec![FlareLight {
        pos: [p.light[0] / w.max(1) as f32, p.light[1] / h.max(1) as f32],
        rgb: p.light_tint,
        extent: [
            p.source_size[0] / w.max(1) as f32,
            p.source_size[1] / h.max(1) as f32,
        ],
    }]
}

/// The sensor shift for a focus distance (K-260): the thin-lens image shift
/// from the infinity position, `f²/(1000·d − f)` mm, clamped so a degenerate
/// distance cannot fling the sensor. Shared by the CPU reference and the GPU
/// uniform fill.
pub fn focus_shift_mm(focus_m: f32, efl_mm: f32) -> f32 {
    if focus_m <= 0.0 {
        return 0.0;
    }
    let denom = (1000.0 * focus_m - efl_mm).max(efl_mm);
    (efl_mm * efl_mm / denom).clamp(0.0, efl_mm)
}

/// The soft threshold gate (K-363): 0 at and below `threshold`, 1 at
/// `threshold + softness`, smoothstep between. **One-sided by decision**: the
/// threshold is the absolute scene-linear luma a pixel must EXCEED to flare
/// at all. At 1.0 only over-range highlights flare; at 0.0 anything brighter
/// than black does — and black itself never flares, which the earlier
/// symmetric gate (open from `threshold - softness`) got wrong: at threshold
/// 0 it let pure black through at half strength. Softness softens the onset
/// *above* the line, never below it.
pub fn threshold_gate(luma: f32, threshold: f32, softness: f32) -> f32 {
    if softness <= 0.0 {
        return f32::from(luma > threshold);
    }
    let t = ((luma - threshold) / softness).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Matte-mode source detection (impl note §6), the CPU twin of the WGSL
/// kernels: tile the matte into [`DETECT_TILE`]-sided cells, keep each cell's
/// brightest pixel (Rec. 709 luma of the premultiplied buffer; ties break to
/// the lowest linear index), then pick the top [`MAX_LIGHTS`] cells by luma
/// (ties to the lower cell index) with [`SUPPRESS_TILES`] Chebyshev
/// suppression, gating each through [`threshold_gate`]. Deterministic by
/// construction — no float reduction order depends on threading.
///
/// Each light's colour is the summed flux of every gated tile nearest it —
/// `(use_source ? tile rgb : white) × gate` per tile, times the tint
/// (K-259/K-267) — so the same function serves "the practical's own colour
/// flares", "this matte only says *where*", and area sources whose light
/// is their whole lit extent rather than one pixel.
/// What one detection tile knows about the light inside it (K-355).
///
/// Through K-354 a tile was a single pair — its brightest pixel's luma and
/// index — and everything downstream used that one pixel for the tile's
/// position AND its colour. That is what made a flare *jump* on real footage:
/// inside a practical, which pixel is brightest changes frame to frame with
/// sensor noise and specular sparkle, so the light's reported position hopped
/// about inside a source that had not moved at all. These sums describe the
/// whole lit area of the tile instead, and none of them can be moved by one
/// pixel changing its mind.
#[derive(Clone, Copy, Debug)]
struct TileStat {
    /// The brightest pixel's luma and linear index — still how anchors are
    /// ranked and gated, so a small bright source is still found.
    luma_max: f32,
    index: u32,
    /// Σ gate, Σ colour·gate: the tile's gated coverage and colour, whose
    /// ratio is the mean colour of the light in it.
    wsum: f32,
    csum: [f32; 3],
    /// Σ luma·gate and its first moments — the tile's flux and where in the
    /// tile that flux actually sits.
    fsum: f32,
    fx: f32,
    fy: f32,
}

impl TileStat {
    const EMPTY: Self = Self {
        luma_max: -1.0,
        index: 0,
        wsum: 0.0,
        csum: [0.0; 3],
        fsum: 0.0,
        fx: 0.0,
        fy: 0.0,
    };
}

pub fn detect_lights(
    matte: &[f32],
    w: u32,
    h: u32,
    threshold: f32,
    softness: f32,
    use_source_colour: bool,
    tint: [f32; 3],
) -> Vec<FlareLight> {
    if w == 0 || h == 0 || matte.len() < (w * h * 4) as usize {
        return Vec::new();
    }
    let tx = w.div_ceil(DETECT_TILE) as usize;
    let ty = h.div_ceil(DETECT_TILE) as usize;
    let mut tiles: Vec<TileStat> = vec![TileStat::EMPTY; tx * ty];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let luma = super::cpu::LUMA[0] * matte[i]
                + super::cpu::LUMA[1] * matte[i + 1]
                + super::cpu::LUMA[2] * matte[i + 2];
            let t = (y / DETECT_TILE) as usize * tx + (x / DETECT_TILE) as usize;
            let tile = &mut tiles[t];
            if luma > tile.luma_max {
                tile.luma_max = luma;
                tile.index = y * w + x;
            }
            // Every lit pixel contributes, not just the brightest (K-355):
            // `w` is the pixel's own gate, `f` its gated flux.
            let g = threshold_gate(luma, threshold, softness);
            if g > 0.0 {
                let f = luma * g;
                tile.wsum += g;
                tile.csum[0] += matte[i].max(0.0) * g;
                tile.csum[1] += matte[i + 1].max(0.0) * g;
                tile.csum[2] += matte[i + 2].max(0.0) * g;
                tile.fsum += f;
                tile.fx += x as f32 * f;
                tile.fy += y as f32 * f;
            }
        }
    }
    let mut suppressed = vec![false; tx * ty];
    // Anchors: the top-K picks, ranked by their brightest pixel so a small
    // bright source is still found; where the light is reported is the flux
    // centroid worked out below, not this pixel.
    let mut anchors: Vec<(usize, u32)> = Vec::new();
    for _ in 0..MAX_SOURCES {
        let mut best: Option<usize> = None;
        for (t, stat) in tiles.iter().enumerate() {
            if suppressed[t] || stat.luma_max <= 0.0 {
                continue;
            }
            match best {
                Some(b) if tiles[b].luma_max >= stat.luma_max => {}
                _ => best = Some(t),
            }
        }
        let Some(b) = best else { break };
        let (luma, idx) = (tiles[b].luma_max, tiles[b].index);
        if threshold_gate(luma, threshold, softness) <= 0.0 {
            // Cells are visited brightest-first, so nothing dimmer passes.
            break;
        }
        anchors.push((b, idx));
        let (bx, by) = ((b % tx) as i64, (b / tx) as i64);
        for sy in (by - SUPPRESS_TILES)..=(by + SUPPRESS_TILES) {
            for sx in (bx - SUPPRESS_TILES)..=(bx + SUPPRESS_TILES) {
                if sx >= 0 && sy >= 0 && (sx as usize) < tx && (sy as usize) < ty {
                    suppressed[sy as usize * tx + sx as usize] = true;
                }
            }
        }
    }
    // Area sources (K-267): every gated tile's flux — its brightest pixel's
    // colour (or white) times its gate — lands on the NEAREST anchor
    // (Chebyshev in tiles; ties to the lowest anchor index), tile order
    // fixed for determinism. A one-tile point source is its own anchor's
    // only contributor, so it reads exactly as before K-267; a practical
    // spanning many tiles finally weighs as its whole lit area instead of
    // one pixel.
    let mut acc = vec![[0.0_f32; 3]; anchors.len()];
    // Per anchor: the flux centroid as (Σ f·x, Σ f·y, Σ f), and the second
    // moment (Σ f·x², Σ f·y²) that gives the source its EXTENT — how wide the
    // light actually is, which is what an area source needs sampling across.
    let mut centroid = vec![[0.0_f32; 3]; anchors.len()];
    let mut spread = vec![[0.0_f32; 2]; anchors.len()];
    if !anchors.is_empty() {
        for (t, stat) in tiles.iter().enumerate() {
            if stat.luma_max <= 0.0 {
                continue;
            }
            let weight = threshold_gate(stat.luma_max, threshold, softness);
            if weight <= 0.0 {
                continue;
            }
            let (cx, cy) = ((t % tx) as i64, (t / tx) as i64);
            let mut nearest = 0usize;
            let mut nearest_d = i64::MAX;
            for (a, &(at, _)) in anchors.iter().enumerate() {
                let (ax, ay) = ((at % tx) as i64, (at / tx) as i64);
                let d = (cx - ax).abs().max((cy - ay).abs());
                if d < nearest_d {
                    nearest_d = d;
                    nearest = a;
                }
            }
            // The tile's MEAN colour over its lit pixels, not its brightest
            // pixel's (K-355). One sparkle among a thousand lit pixels now
            // shifts the colour by a thousandth instead of defining it.
            let src = if !use_source_colour {
                [1.0, 1.0, 1.0]
            } else if stat.wsum > 0.0 {
                [
                    stat.csum[0] / stat.wsum,
                    stat.csum[1] / stat.wsum,
                    stat.csum[2] / stat.wsum,
                ]
            } else {
                let i = (stat.index * 4) as usize;
                [
                    matte[i].max(0.0),
                    matte[i + 1].max(0.0),
                    matte[i + 2].max(0.0),
                ]
            };
            for c in 0..3 {
                acc[nearest][c] += src[c] * weight;
            }
            // The tile's own first moments carry straight over: summing them
            // across a source's tiles gives that source's flux centroid, with
            // no pixel anywhere able to move it on its own.
            centroid[nearest][0] += stat.fx;
            centroid[nearest][1] += stat.fy;
            centroid[nearest][2] += stat.fsum;
            // Σ f·x² is accumulated from the tile's mean position rather than
            // per pixel: the tiles are what span a source, and a tile-grained
            // extent is all the sampling below can use anyway.
            if stat.fsum > 0.0 {
                let (mx, my) = (stat.fx / stat.fsum, stat.fy / stat.fsum);
                spread[nearest][0] += stat.fsum * mx * mx;
                spread[nearest][1] += stat.fsum * my * my;
            }
        }
    }
    anchors
        .iter()
        .zip(acc.iter().zip(centroid.iter().zip(&spread)))
        .map(|(&(_, idx), (rgb, (cen, spr)))| {
            // **Where the light IS, is the centre of its light** (K-354, and
            // K-355 made it immovable by any one pixel). Pinning the anchor to
            // its brightest pixel quantised the light to one pixel of a source
            // that may span hundreds, and on FOOTAGE that pixel wanders:
            // sensor noise and specular sparkle move it frame to frame, so the
            // whole flare jittered even though the practical had not moved.
            let (px, py) = if cen[2] > 0.0 {
                (cen[0] / cen[2], cen[1] / cen[2])
            } else {
                ((idx % w) as f32, (idx / w) as f32)
            };
            // The source's half-extent, as the standard deviation of its flux
            // about that centre (√(E[x²] − E[x]²)). A point source measures
            // zero and is sampled once; a practical measures its real width
            // and is sampled across it (see [`area_samples`]).
            let half = |m2: f32, mean: f32| {
                if cen[2] <= 0.0 {
                    return 0.0;
                }
                (m2 / cen[2] - mean * mean).max(0.0).sqrt()
            };
            FlareLight {
                pos: [(px + 0.5) / w as f32, (py + 0.5) / h as f32],
                rgb: [rgb[0] * tint[0], rgb[1] * tint[1], rgb[2] * tint[2]],
                extent: [half(spr[0], px) / w as f32, half(spr[1], py) / h as f32],
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The lens prescription (K-261: parsed from the embedded .lens library).
// ---------------------------------------------------------------------------

/// One optical surface, flattened for the trace and mirrored field-for-field
/// by the WGSL struct. The Cauchy pair describes the medium AFTER this
/// surface (1.0/0.0 = air); `coating_layers` is the .lens coating column
/// (0 bare glass, 1 single-layer MgF₂, 2+ multicoat).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlareSurface {
    /// Signed sphere radius, mm; 0 = flat.
    pub radius_mm: f32,
    /// Surface vertex z, mm (front vertex at 0, increasing toward sensor).
    pub z_mm: f32,
    /// Clear semi-aperture, mm — rays beyond it die.
    pub semi_ap_mm: f32,
    /// Cauchy A of the medium after this surface (1.0 = air).
    pub cauchy_a: f32,
    /// Cauchy B, µm²; 0 for air.
    pub cauchy_b: f32,
    /// AR coating layer count (as f32 for the POD mirror).
    pub coating_layers: f32,
    /// 1.0 on the aperture-stop surface, else 0.0 (the f-stop scales it).
    pub is_stop: f32,
    /// Padding (POD mirror alignment).
    pub _pad: f32,
}

/// Everything the bake produces: pure function of the bake-relevant subset
/// of [`LensFlareParams`] (see [`bake_key`]), consumed by the GPU (uploaded
/// once, cached) and by the CPU reference directly.
#[derive(Debug, Clone)]
pub struct FlareBaked {
    /// The trace surface table, front to back (no appended sensor row — the
    /// sensor plane is `sensor_z_mm`).
    pub surfaces: Vec<FlareSurface>,
    /// Sensor plane z, mm (the prescription's back focal chain).
    pub sensor_z_mm: f32,
    /// The prescription's stated focal length, mm (light direction, focus).
    pub focal_mm: f32,
    /// Native f-number (from the collection filename; estimated from the
    /// front aperture when unknown).
    pub native_fstop: f32,
    /// Front-element clear semi-aperture, mm.
    pub front_semi_ap: f32,
    /// The pupil spray's radius, mm (K-261): the entrance pupil
    /// `focal / (2 · native_fstop)` with half again as margin (ghost paths
    /// accept rays the imaging pupil rejects), clamped to the front
    /// element. Spraying the whole front bezel instead wastes most rays —
    /// the Master Prime's 63 mm bezel passes ~4% of a full-width spray.
    pub pupil_mm: f32,
    /// Ray start z, mm (in front of the first surface).
    pub start_z_mm: f32,
    /// Ranked ghost pairs, brightest first; the frame renders the first
    /// `max_ghosts`.
    pub pairs: Vec<[u32; 2]>,
    /// Each pair's on-axis image extent as a fraction of the sensor
    /// diagonal, parallel to `pairs` (K-262): what [`pair_grid`] spends the
    /// ray budget by. A tight 5%-of-frame ghost needs a fraction of the
    /// grid a frame-filling defocused one does.
    pub spreads: Vec<f32>,
    /// Per-surface spectral reflectance (K-364, entry A2): the thin-film
    /// stack's R evaluated on a fixed (lambda, cos theta) grid at bake time,
    /// so the per-frame trace reads a table instead of chaining 2x2 complex
    /// matrices per ray. Layout `[surface][direction][lambda][cos]` with
    /// direction 0 = crossing front-to-back (n1 = the medium before, n2 =
    /// after) and 1 = the reverse - a ghost's phase-2 walk crosses surfaces
    /// backwards, and tabulating both directions beats Snell-conjugating
    /// angles at trace time. Lambda runs [`cie::LAMBDA_MIN`] to
    /// [`cie::LAMBDA_MAX`] over [`REFL_LAMBDA_BINS`] samples; cos theta over
    /// [`REFL_COS_BINS`] bin centres. Uncoated surfaces store plain Fresnel
    /// at the same grid, so the trace has one path.
    pub reflectance: Vec<f32>,
    /// The auto-exposure gain (closed loop, K-258): multiplies every splat
    /// so all bundled lenses read comparably at default Intensity.
    pub energy_gain: f32,
    /// The starburst sprite, `STARBURST_RES`² × RGB, peak-normalised.
    pub starburst: Vec<f32>,
}

/// A parsed .lens prescription before flattening.
pub struct Prescription {
    /// The stated focal length, mm.
    pub focal_mm: f32,
    /// Surfaces front to back with running vertex z.
    pub surfaces: Vec<FlareSurface>,
    /// Sensor plane z (the thickness chain's end), mm.
    pub sensor_z_mm: f32,
}

/// Parse a .lens text (K-261, the FlareSim/PhotonsToPhotos format): metadata
/// lines (`name:`, `focal_length:`), then `surfaces:` rows of
/// `radius thickness ior abbe semi_ap coating` with `stop`/`inf` keywords.
/// Malformed rows are skipped; a file with under 3 surfaces is rejected.
pub(crate) fn parse_lens(text: &str) -> Option<Prescription> {
    let mut focal = 0.0_f32;
    let mut in_surfaces = false;
    let mut rows: Vec<(f32, f32, f32, f32, f32, f32, bool)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !in_surfaces {
            if let Some(v) = line.strip_prefix("focal_length:") {
                focal = v.trim().parse().unwrap_or(0.0);
            } else if line.starts_with("surfaces:") {
                in_surfaces = true;
            }
            continue;
        }
        let mut it = line.split_whitespace();
        let radius_tok = it.next().unwrap_or("");
        let is_stop = radius_tok.eq_ignore_ascii_case("stop");
        let radius = if is_stop || radius_tok.eq_ignore_ascii_case("inf") {
            0.0
        } else {
            match radius_tok.parse::<f32>() {
                Ok(r) => r,
                Err(_) => continue,
            }
        };
        let mut f = |d: f32| it.next().and_then(|t| t.parse::<f32>().ok()).unwrap_or(d);
        let thickness = f(0.0);
        let ior = f(1.0);
        let abbe = f(0.0);
        let semi_ap = f(0.0);
        let coating = f(0.0);
        if semi_ap <= 0.0 {
            continue;
        }
        rows.push((radius, thickness, ior, abbe, semi_ap, coating, is_stop));
    }
    if rows.len() < 3 || focal <= 0.0 {
        return None;
    }
    let mut z = 0.0_f32;
    let mut surfaces = Vec::with_capacity(rows.len());
    for &(radius, thickness, ior, abbe, semi_ap, coating, is_stop) in &rows {
        let (a, b) = cauchy_from_abbe(ior, abbe);
        surfaces.push(FlareSurface {
            radius_mm: radius,
            z_mm: z,
            semi_ap_mm: semi_ap,
            cauchy_a: a,
            cauchy_b: b,
            coating_layers: coating.max(0.0),
            is_stop: if is_stop { 1.0 } else { 0.0 },
            _pad: 0.0,
        });
        z += thickness;
    }
    Some(Prescription {
        focal_mm: focal,
        surfaces,
        sensor_z_mm: z,
    })
}

/// The library entry a params bundle selects (index clamped).
pub fn lens_entry(lens: u32) -> &'static super::lens_library::LensFile {
    let i = (lens as usize).min(LENS_LIBRARY.len() - 1);
    &LENS_LIBRARY[i]
}

/// The stop-down scale for the working f-stop against the lens's native
/// f-number: 1 wide open, smaller stopped down. Scales the stop surface's
/// semi-aperture and the pupil mask together.
pub fn fstop_scale(native_fstop: f32, fstop: f32) -> f32 {
    if native_fstop <= 0.0 || fstop <= 0.0 {
        return 1.0;
    }
    (native_fstop / fstop).clamp(0.05, 1.0)
}

// ---------------------------------------------------------------------------
// Optics primitives — the exact maths the WGSL splat kernel mirrors.
// ---------------------------------------------------------------------------

/// Cauchy dispersion pair from a prescription's (n_d, V) — impl note §1
/// deviation D1. Returns (A, B[µm²]); air (n ≤ 1 or V ≤ 0) is (n_d, 0).
pub(crate) fn cauchy_from_abbe(n_d: f32, v: f32) -> (f32, f32) {
    if n_d <= 1.0001 || v <= 0.1 {
        return (n_d.max(1.0), 0.0);
    }
    let lam_f = 0.486_13_f64; // hydrogen F line, µm
    let lam_c = 0.656_27_f64; // hydrogen C line, µm
    let lam_d = 0.587_56_f64; // helium d line, µm
    let inv = 1.0 / (lam_f * lam_f) - 1.0 / (lam_c * lam_c);
    let b = (n_d as f64 - 1.0) / (v as f64 * inv);
    let a = n_d as f64 - b / (lam_d * lam_d);
    (a as f32, b as f32)
}

/// Refractive index at `lambda_nm` from a Cauchy pair.
pub fn cauchy_ior(a: f32, b: f32, lambda_nm: f32) -> f32 {
    let um = lambda_nm * 1e-3;
    a + b / (um * um)
}

/// Unpolarised Fresnel reflectance at a dielectric interface, by incidence
/// cosine (K-261, the FlareSim formulation the WGSL mirrors).
pub fn fresnel_cos(cos_i: f32, n1: f32, n2: f32) -> f32 {
    let cos_i = cos_i.abs();
    let eta = n1 / n2;
    let sin2_t = eta * eta * (1.0 - cos_i * cos_i);
    if sin2_t >= 1.0 {
        return 1.0; // total internal reflection
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    let rs = (n1 * cos_i - n2 * cos_t) / (n1 * cos_i + n2 * cos_t);
    let rp = (n2 * cos_i - n1 * cos_t) / (n2 * cos_i + n1 * cos_t);
    0.5 * (rs * rs + rp * rp)
}

/// Single-layer thin-film reflectance (Airy summation): coating index
/// `coating_n`, physical thickness `d_nm`.
pub fn coating_reflectance(
    cos_i: f32,
    n1: f32,
    n2: f32,
    coating_n: f32,
    d_nm: f32,
    lambda_nm: f32,
) -> f32 {
    let cos_i = cos_i.abs();
    let sin2_c = (n1 / coating_n) * (n1 / coating_n) * (1.0 - cos_i * cos_i);
    if sin2_c >= 1.0 {
        return fresnel_cos(cos_i, n1, n2);
    }
    let cos_c = (1.0 - sin2_c).sqrt();
    let delta = 2.0 * std::f32::consts::PI * coating_n * d_nm * cos_c / lambda_nm;
    let r01 = (n1 * cos_i - coating_n * cos_c) / (n1 * cos_i + coating_n * cos_c);
    let sin2_2 = (coating_n / n2) * (coating_n / n2) * (1.0 - cos_c * cos_c);
    if sin2_2 >= 1.0 {
        return fresnel_cos(cos_i, n1, n2);
    }
    let cos_2 = (1.0 - sin2_2).sqrt();
    let r12 = (coating_n * cos_c - n2 * cos_2) / (coating_n * cos_c + n2 * cos_2);
    let cos_2d = (2.0 * delta).cos();
    let num = r01 * r01 + r12 * r12 + 2.0 * r01 * r12 * cos_2d;
    let den = 1.0 + r01 * r01 * r12 * r12 + 2.0 * r01 * r12 * cos_2d;
    (num / den).clamp(0.0, 1.0)
}

/// The design wavelength every coating in the library is cut for, nm.
pub const COATING_DESIGN_NM: f32 = 550.0;
/// Magnesium fluoride — the classic single-layer AR, and the outer layer of
/// every stack below.
pub const MGF2_N: f32 = 1.38;
/// Alumina, the mid-index layer of a broadband stack.
pub const AL2O3_N: f32 = 1.63;
/// Zirconia, the high-index layer.
pub const ZRO2_N: f32 = 2.10;
/// Most layers a stack may have.
pub const MAX_COATING_LAYERS: usize = 6;

/// The canonical stack for a `.lens` coating column, **outermost first** —
/// each entry `(refractive index, optical thickness in quarter waves at
/// [`COATING_DESIGN_NM`])`.
///
/// A lens prescription publishes a layer *count*, never the recipe: real
/// coating designs are manufacturer secrets, and the literature is unanimous
/// that they can only be measured, not predicted (K-356). What a renderer can
/// do is use the textbook design of each order, which is what these are:
///
/// - **1 layer** — MgF₂ quarter wave, the classic single coating. One
///   reflectance minimum at 550 nm, a broad V.
/// - **2 layers** — a V-coat: high-index quarter under a MgF₂ quarter. Deeper
///   minimum, still one of them.
/// - **3 layers** — the classic broadband W: quarter / half / quarter, which
///   is what gives a real multicoated lens two minima and the green-magenta
///   cast between them.
/// - **4+** — the same W with extra quarter-wave pairs beneath, broadening it
///   further and lowering the mean.
///
/// The shape matters more than the exact recipe: it is the *variation of
/// reflectance with wavelength and angle* that makes ghosts change hue as a
/// light crosses the frame, and a single number cannot produce it at all.
pub fn coating_stack(layers: f32) -> [(f32, f32); MAX_COATING_LAYERS] {
    let none = (0.0, 0.0);
    let n = (layers.round().max(0.0) as usize).min(MAX_COATING_LAYERS);
    let mut out = [none; MAX_COATING_LAYERS];
    match n {
        0 => {}
        1 => out[0] = (MGF2_N, 1.0),
        2 => {
            out[0] = (MGF2_N, 1.0);
            out[1] = (ZRO2_N, 1.0);
        }
        _ => {
            out[0] = (MGF2_N, 1.0);
            out[1] = (ZRO2_N, 2.0);
            out[2] = (AL2O3_N, 1.0);
            // Extra pairs alternate high/low beneath the W, each a quarter
            // wave, which is how broadband stacks are actually extended.
            for (i, slot) in out.iter_mut().enumerate().take(n).skip(3) {
                *slot = if i % 2 == 0 {
                    (AL2O3_N, 1.0)
                } else {
                    (ZRO2_N, 1.0)
                };
            }
        }
    }
    out
}

/// Complex multiply, as `(re, im)` — the whole of the complex arithmetic the
/// transfer matrix needs, spelled out rather than pulled in as a dependency.
#[inline]
fn cmul(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

/// Reflectance of a multi-layer thin-film stack by the **characteristic
/// transfer matrix** — the standard treatment, and the one thing that gets
/// ghost colour right (K-356).
///
/// Per layer the phase thickness is `δ = 2π n d cos θ / λ` and the optical
/// admittance `η = n cos θ` (s polarisation) or `n / cos θ` (p); the layer's
/// characteristic matrix is `[[cos δ, i sin δ / η], [i η sin δ, cos δ]]`.
/// Chaining them and closing on the substrate's admittance gives `Y = C / B`,
/// whence `r = (η₀ − Y) / (η₀ + Y)`. Both polarisations are computed and
/// averaged, which is what unpolarised light asks for.
///
/// **Why the angle matters as much as the wavelength.** `δ` carries a `cos θ`,
/// so the whole reflectance band shifts *blue* as the angle of incidence
/// rises. Flare rays strike interfaces at large and varied angles, so this is
/// the dominant term — and it is exactly the observed effect that a ghost
/// changes hue as its source moves off axis. No scalar coating strength can
/// express it.
pub fn stack_reflectance(
    cos_i: f32,
    n1: f32,
    n2: f32,
    stack: &[(f32, f32); MAX_COATING_LAYERS],
    lambda_nm: f32,
) -> f32 {
    let cos_i = cos_i.abs().clamp(1e-6, 1.0);
    // Snell's invariant: n·sinθ is the same in every medium of the stack.
    let sin_i = n1 * (1.0 - cos_i * cos_i).max(0.0).sqrt();
    let cos_in_medium = |n: f32| -> Option<f32> {
        let s = sin_i / n.max(1e-6);
        let c2 = 1.0 - s * s;
        (c2 > 0.0).then(|| c2.sqrt())
    };
    let Some(cos_sub) = cos_in_medium(n2) else {
        // Total internal reflection somewhere in the stack: the film model
        // has nothing to say, so fall back to the bare interface.
        return fresnel_cos(cos_i, n1, n2);
    };

    // `s` polarisation then `p`; unpolarised light is their mean.
    let mut total = 0.0;
    for pol in 0..2 {
        let admittance = |n: f32, c: f32| if pol == 0 { n * c } else { n / c };
        let eta0 = admittance(n1, cos_i);
        let eta_sub = admittance(n2, cos_sub);
        // The identity matrix, as (m00, m01, m10, m11) of complex pairs.
        let mut m = [(1.0f32, 0.0f32), (0.0, 0.0), (0.0, 0.0), (1.0, 0.0)];
        let mut valid = true;
        for &(n, quarters) in stack.iter() {
            if n <= 0.0 || quarters <= 0.0 {
                continue;
            }
            let Some(cos_l) = cos_in_medium(n) else {
                valid = false;
                break;
            };
            // Physical thickness from the optical one: `quarters` quarter
            // waves at the design wavelength.
            let d_nm = quarters * COATING_DESIGN_NM / (4.0 * n);
            let delta = 2.0 * std::f32::consts::PI * n * d_nm * cos_l / lambda_nm.max(1e-6);
            let eta = admittance(n, cos_l);
            let (cd, sd) = (delta.cos(), delta.sin());
            let l = [(cd, 0.0), (0.0, sd / eta), (0.0, eta * sd), (cd, 0.0)];
            // m = m · l
            let a = [
                (
                    cmul(m[0], l[0]).0 + cmul(m[1], l[2]).0,
                    cmul(m[0], l[0]).1 + cmul(m[1], l[2]).1,
                ),
                (
                    cmul(m[0], l[1]).0 + cmul(m[1], l[3]).0,
                    cmul(m[0], l[1]).1 + cmul(m[1], l[3]).1,
                ),
                (
                    cmul(m[2], l[0]).0 + cmul(m[3], l[2]).0,
                    cmul(m[2], l[0]).1 + cmul(m[3], l[2]).1,
                ),
                (
                    cmul(m[2], l[1]).0 + cmul(m[3], l[3]).0,
                    cmul(m[2], l[1]).1 + cmul(m[3], l[3]).1,
                ),
            ];
            m = a;
        }
        if !valid {
            return fresnel_cos(cos_i, n1, n2);
        }
        // [B, C] = M · [1, η_sub]
        let b = (m[0].0 + m[1].0 * eta_sub, m[0].1 + m[1].1 * eta_sub);
        let c = (m[2].0 + m[3].0 * eta_sub, m[2].1 + m[3].1 * eta_sub);
        // r = (η₀·B − C) / (η₀·B + C)
        let num = (eta0 * b.0 - c.0, eta0 * b.1 - c.1);
        let den = (eta0 * b.0 + c.0, eta0 * b.1 + c.1);
        let dm = den.0 * den.0 + den.1 * den.1;
        if dm <= 1e-20 {
            return fresnel_cos(cos_i, n1, n2);
        }
        total += (num.0 * num.0 + num.1 * num.1) / dm;
    }
    (total * 0.5).clamp(0.0, 1.0)
}

/// Reflectance of one lens surface: uncoated Fresnel blended toward the
/// prescription's AR coating by the Coating dial. `layers` is the .lens
/// coating column, resolved to a real multi-layer design by
/// [`coating_stack`] and evaluated by [`stack_reflectance`] (K-356,
/// superseding the single-layer-times-a-quarter approximation).
pub fn surface_reflectance(
    cos_i: f32,
    n1: f32,
    n2: f32,
    layers: f32,
    lambda_nm: f32,
    coating_mix: f32,
) -> f32 {
    let plain = fresnel_cos(cos_i, n1, n2);
    if layers < 0.5 || coating_mix <= 0.0 {
        return plain;
    }
    let coated = stack_reflectance(cos_i, n1, n2, &coating_stack(layers), lambda_nm);
    (plain + (coated - plain) * coating_mix.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// Snell refraction in vector form: incidence `i` and normal `n` (opposing
/// the ray) unit vectors, `o = n1/n2`. Total internal reflection returns
/// None.
// Negated comparison deliberate: NaN reads as dead (see `intersect`).
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub(crate) fn refract3(i: [f32; 3], n: [f32; 3], o: f32) -> Option<[f32; 3]> {
    let cos_i = -(i[0] * n[0] + i[1] * n[1] + i[2] * n[2]);
    let sin2_t = o * o * (1.0 - cos_i * cos_i);
    if sin2_t >= 1.0 {
        return None;
    }
    let k = o * cos_i - (1.0 - sin2_t).sqrt();
    let v = [
        o * i[0] + k * n[0],
        o * i[1] + k * n[1],
        o * i[2] + k * n[2],
    ];
    let sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if !(sq > 1e-18) || !sq.is_finite() {
        return None;
    }
    let inv = 1.0 / sq.sqrt();
    Some([v[0] * inv, v[1] * inv, v[2] * inv])
}

/// Mirror reflection of incidence `i` about unit normal `n`.
pub(crate) fn reflect3(i: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    let d = 2.0 * (i[0] * n[0] + i[1] * n[1] + i[2] * n[2]);
    let v = [i[0] - d * n[0], i[1] - d * n[1], i[2] - d * n[2]];
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / len, v[1] / len, v[2] / len]
}

/// Intersect a ray with one surface (K-261, the FlareSim rule): flat plane
/// at the vertex z, else ray–sphere picking the intersection closest to the
/// vertex. The clear semi-aperture clips with a 10% skirt: rays inside it
/// stay formally alive (the housing feather zeroes their weight), so quads
/// at a housing boundary fade instead of dying corner-by-corner — a
/// frame-filling defocused ghost otherwise shows its cull boundary as giant
/// staircase rectangles. Returns (hit position, normal opposing the ray) or
/// None. `semi_ap` is passed separately so the f-stop can scale the stop
/// surface without touching the table.
// The negated comparisons are deliberate: `!(t > eps)` is false for NaN, so
// a degenerate ray reads as dead instead of propagating NaN (the FlareSim
// guard style the WGSL twin mirrors).
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn intersect(
    pos: [f32; 3],
    dir: [f32; 3],
    radius: f32,
    z_mm: f32,
) -> Option<([f32; 3], [f32; 3], bool)> {
    if dir[2].abs() < 1e-12 {
        return None;
    }
    if radius.abs() < 1e-6 {
        let t = (z_mm - pos[2]) / dir[2];
        if !(t > 1e-6) {
            return None;
        }
        let hit = [
            pos[0] + dir[0] * t,
            pos[1] + dir[1] * t,
            pos[2] + dir[2] * t,
        ];
        let n = if dir[2] > 0.0 {
            [0.0, 0.0, -1.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        return Some((hit, n, false));
    }
    let centre = [0.0, 0.0, z_mm + radius];
    let oc = [pos[0] - centre[0], pos[1] - centre[1], pos[2] - centre[2]];
    let a = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    let b = 2.0 * (oc[0] * dir[0] + oc[1] * dir[1] + oc[2] * dir[2]);
    let c = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - radius * radius;
    let disc = b * b - 4.0 * a * c;
    let mut t = -1.0_f32;
    if disc >= 0.0 {
        let sd = disc.sqrt();
        let inv2a = 0.5 / a;
        let t1 = (-b - sd) * inv2a;
        let t2 = (-b + sd) * inv2a;
        if t1 > 1e-6 && t2 > 1e-6 {
            let z1 = pos[2] + t1 * dir[2];
            let z2 = pos[2] + t2 * dir[2];
            t = if (z1 - z_mm).abs() < (z2 - z_mm).abs() {
                t1
            } else {
                t2
            };
        } else if t1 > 1e-6 {
            t = t1;
        } else if t2 > 1e-6 {
            t = t2;
        }
    }
    if t <= 0.0 {
        // The ray misses the sphere (or it lies behind): continue VIRTUALLY
        // through the surface's vertex plane instead of dying (K-264). A
        // miss means the ray is outside the element's glass — physically it
        // hits the mount and is absorbed, so its weight is forced to zero by
        // the caller — but killing it also killed every grid cell touching
        // it, and a ghost bounded by such misses wore its pupil grid as a
        // staircase. The virtual hit keeps the cell geometry defined; the
        // zero weight keeps the light honest.
        let t = (z_mm - pos[2]) / dir[2];
        if !(t > 1e-6) {
            return None;
        }
        let hit = [
            pos[0] + dir[0] * t,
            pos[1] + dir[1] * t,
            pos[2] + dir[2] * t,
        ];
        let n = if dir[2] > 0.0 {
            [0.0, 0.0, -1.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        return Some((hit, n, true));
    }
    let hit = [
        pos[0] + dir[0] * t,
        pos[1] + dir[1] * t,
        pos[2] + dir[2] * t,
    ];
    let inv_r = 1.0 / radius.abs();
    let mut n = [
        (hit[0] - centre[0]) * inv_r,
        (hit[1] - centre[1]) * inv_r,
        (hit[2] - centre[2]) * inv_r,
    ];
    if n[0] * dir[0] + n[1] * dir[1] + n[2] * dir[2] > 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    Some((hit, n, false))
}

/// The light direction for a light at `light` (raster fraction, y down) on
/// a lens of `focal_mm`, aspect `h/w`. Sensor y is up, so the y fraction
/// flips sign; a light at the frame corner enters at the true corner field
/// angle.
pub fn light_direction(light: [f32; 2], aspect_h_over_w: f32, focal_mm: f32) -> [f32; 3] {
    let half_w = SENSOR_MM[0] / 2.0;
    let x = (light[0] * 2.0 - 1.0) * half_w;
    let y = -(light[1] * 2.0 - 1.0) * aspect_h_over_w * half_w;
    let v = [-x, -y, focal_mm];
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / len, v[1] / len, v[2] / len]
}

/// Trace one pupil sample through one ghost pair at one wavelength — the
/// FlareSim three-phase walk (K-261), the CPU twin the WGSL splat kernel
/// mirrors op-for-op. `origin` is the ray start (mm), `dir` the unit beam
/// direction; the ray transmits through every surface except the pair's
/// two, where it reflects (weight × R; transmits weight × (1−R)). Returns
/// the sensor landing (mm, y up) and the accumulated Fresnel weight.
#[allow(clippy::too_many_arguments)]
// Negated comparisons deliberate: NaN reads as dead (see `intersect`).
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn trace_splat(
    baked: &FlareBaked,
    pair: [u32; 2],
    lambda_nm: f32,
    origin: [f32; 3],
    dir: [f32; 3],
    coating_mix: f32,
    stop_scale: f32,
    sensor_shift_mm: f32,
) -> Option<([f32; 2], f32)> {
    let surfs = &baked.surfaces;
    let n = surfs.len();
    let (a_idx, b_idx) = (pair[0] as usize, pair[1] as usize);
    if n < 3 || a_idx >= b_idx || b_idx >= n {
        return None;
    }
    /// The walking ray: position, direction, surviving energy, the medium it
    /// is in, and its worst relative aperture crossing so far. `rrel2` is
    /// tracked SQUARED and rooted once at the end (K-263): the worst crossing
    /// is the largest ratio either way — `max` and `sqrt` commute for
    /// non-negative values — so this is the same number for one square root a
    /// ray instead of one per surface, on the hottest loop the effect has.
    /// Rays that graze a housing edge fade smoothly through the 0.95..1
    /// feather at the end instead of the hard clip alone — without it, a
    /// defocused ghost's cull boundary shows as giant staircase quads (K-261,
    /// the K-256 rrel feather reinstated).
    struct Ray {
        pos: [f32; 3],
        dir: [f32; 3],
        weight: f32,
        ior: f32,
        rrel2: f32,
    }
    let mut ray = Ray {
        pos: origin,
        dir,
        weight: 1.0,
        ior: 1.0,
        rrel2: 0.0,
    };

    let ior_at = |s: &FlareSurface| cauchy_ior(s.cauchy_a, s.cauchy_b, lambda_nm);
    let ior_before = |idx: usize| -> f32 {
        if idx == 0 {
            1.0
        } else {
            ior_at(&surfs[idx - 1])
        }
    };

    // One surface crossing — the body all three phases share, in the exact
    // per-surface arithmetic order the WGSL twin mirrors. `n2` is the medium
    // past the surface in the walk's direction; `reflect` bounces instead of
    // refracting (the ghost pair's two mirror surfaces).
    let step = |ray: &mut Ray, s: &FlareSurface, n2: f32, reflect: bool| -> Option<()> {
        let semi = if s.is_stop > 0.5 {
            s.semi_ap_mm * stop_scale
        } else {
            s.semi_ap_mm
        };
        let (hit, norm, missed) = intersect(ray.pos, ray.dir, s.radius_mm, s.z_mm)?;
        ray.pos = hit;
        if missed {
            // Outside the element's glass entirely: the mount absorbs it.
            // Weight goes to zero through the housing feather; the ray
            // itself continues so its grid cells stay whole (K-264).
            ray.rrel2 = ray.rrel2.max(4.0);
        }
        // The feather's denominator is the smaller of the clear aperture
        // and the glass's own lateral extent (K-264): a transcribed
        // prescription can claim a clear aperture wider than the sphere it
        // sits on, and rays then MISSED the glass while their feather still
        // read "well inside" — a one-cell hard step at the ghost's bore
        // edge. Clamped, the feather reaches zero before the miss can.
        let semi_r = if s.radius_mm.abs() < 1e-6 {
            semi.max(1e-6)
        } else {
            semi.min(s.radius_mm.abs()).max(1e-6)
        };
        ray.rrel2 = ray
            .rrel2
            .max((ray.pos[0] * ray.pos[0] + ray.pos[1] * ray.pos[1]) / (semi_r * semi_r));
        let n1 = ray.ior;
        let cos_i = (norm[0] * ray.dir[0] + norm[1] * ray.dir[1] + norm[2] * ray.dir[2]).abs();
        let r = surface_reflectance(cos_i, n1, n2, s.coating_layers, lambda_nm, coating_mix);
        if reflect {
            ray.dir = reflect3(ray.dir, norm);
            ray.weight *= r;
        } else {
            match refract3(ray.dir, norm, n1 / n2) {
                Some(d) => ray.dir = d,
                // Total internal reflection: the transmitted energy is
                // already ~0 (Fresnel reaches 1 smoothly on approach), so
                // the ray continues STRAIGHT with its weight forced to
                // zero (K-264) — the last cell-killer with no feather, and
                // the stair-steps on hard vignetted ghost edges.
                None => ray.rrel2 = ray.rrel2.max(4.0),
            }
            ray.weight *= 1.0 - r;
            ray.ior = n2;
        }
        Some(())
    };

    // Phase 1: forward through 0..=b, reflecting at b.
    for (s_idx, s) in surfs.iter().enumerate().take(b_idx + 1) {
        step(&mut ray, s, ior_at(s), s_idx == b_idx)?;
    }
    // Phase 2: backward through b-1..=a, reflecting at a. The mirror at `a`
    // sends the ray forward again, into a's own glass.
    for s_idx in (a_idx..b_idx).rev() {
        let s = &surfs[s_idx];
        let reflect = s_idx == a_idx;
        step(&mut ray, s, ior_before(s_idx), reflect)?;
        if reflect {
            ray.ior = ior_at(s);
        }
    }
    // Phase 3: forward through a+1..n.
    for s in surfs.iter().skip(a_idx + 1) {
        step(&mut ray, s, ior_at(s), false)?;
    }

    // Propagate to the (focus-shifted) sensor plane.
    if ray.dir[2].abs() < 1e-12 {
        return None;
    }
    let t = (baked.sensor_z_mm + sensor_shift_mm - ray.pos[2]) / ray.dir[2];
    if !(t > 0.0) {
        return None;
    }
    let x = ray.pos[0] + ray.dir[0] * t;
    let y = ray.pos[1] + ray.dir[1] * t;
    if !x.is_finite() || !y.is_finite() || !ray.weight.is_finite() {
        return None;
    }
    // Housing feather: full inside 0.95, gone at 1.0 (smoothstep).
    let ft = ((1.0 - ray.rrel2.sqrt()) / 0.05).clamp(0.0, 1.0);
    Some(([x, y], ray.weight * (ft * ft * (3.0 - 2.0 * ft))))
}

/// [`trace_splat`]'s spectral sibling (K-364, entry A2): the identical
/// three-phase walk at the band's geometry wavelength, but the ray's energy
/// is carried per radiometric sub-sample — [`SPECTRAL_SUB`] throughputs, one
/// per 1/8th of the band, each reading the baked reflectance table at its
/// own wavelength — and folded against the band's CIE weights at the sensor.
/// Returns `(landing, geometric weight, rgb)`: the geometric weight is the
/// housing feather alone (what the caller multiplies by the iris mask and
/// the splat smooths), the rgb is the band-integrated energy.
///
/// Kept beside [`trace_splat`] as a sibling rather than folded into it: the
/// scalar walk serves the bake's thousands of ranking probes, where 8×
/// radiometry would be spent on answers nothing reads, and this one is the
/// WGSL trace kernel's oracle. The two walks must stay geometry-identical —
/// any edit to one's phases or feathering belongs in both.
// Range loops deliberate: `step` takes the surface INDEX (the reflectance
// table is keyed by it), so iterator-with-enumerate buys nothing here.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
pub fn trace_splat_spectral(
    baked: &FlareBaked,
    pair: [u32; 2],
    band: &SpectralBand,
    origin: [f32; 3],
    dir: [f32; 3],
    coating_mix: f32,
    stop_scale: f32,
    sensor_shift_mm: f32,
) -> Option<([f32; 2], f32, [f32; 3])> {
    let surfs = &baked.surfaces;
    let n = surfs.len();
    let (a_idx, b_idx) = (pair[0] as usize, pair[1] as usize);
    if n < 3 || a_idx >= b_idx || b_idx >= n {
        return None;
    }
    let lambda_nm = band.traced_nm;
    struct Ray {
        pos: [f32; 3],
        dir: [f32; 3],
        /// Per-sub-sample energy throughput; the geometry is shared.
        thru: [f32; SPECTRAL_SUB],
        ior: f32,
        rrel2: f32,
    }
    let mut ray = Ray {
        pos: origin,
        dir,
        thru: [1.0; SPECTRAL_SUB],
        ior: 1.0,
        rrel2: 0.0,
    };

    let ior_at = |s: &FlareSurface| cauchy_ior(s.cauchy_a, s.cauchy_b, lambda_nm);
    let ior_before = |idx: usize| -> f32 {
        if idx == 0 {
            1.0
        } else {
            ior_at(&surfs[idx - 1])
        }
    };

    // One surface crossing — geometry exactly as `trace_splat`; radiometry
    // per sub-sample from the baked table, blended against plain Fresnel by
    // the frame-time Coating dial. `reverse` says which direction the table
    // is read in (phase 2 crosses surfaces back to front).
    let step = |ray: &mut Ray, s_idx: usize, n2: f32, reflect: bool, reverse: bool| -> Option<()> {
        let s = &surfs[s_idx];
        let semi = if s.is_stop > 0.5 {
            s.semi_ap_mm * stop_scale
        } else {
            s.semi_ap_mm
        };
        let (hit, norm, missed) = intersect(ray.pos, ray.dir, s.radius_mm, s.z_mm)?;
        ray.pos = hit;
        if missed {
            ray.rrel2 = ray.rrel2.max(4.0);
        }
        let semi_r = if s.radius_mm.abs() < 1e-6 {
            semi.max(1e-6)
        } else {
            semi.min(s.radius_mm.abs()).max(1e-6)
        };
        ray.rrel2 = ray
            .rrel2
            .max((ray.pos[0] * ray.pos[0] + ray.pos[1] * ray.pos[1]) / (semi_r * semi_r));
        let n1 = ray.ior;
        let cos_i = (norm[0] * ray.dir[0] + norm[1] * ray.dir[1] + norm[2] * ray.dir[2]).abs();
        // Plain Fresnel at the band wavelength: the smooth part, and the
        // whole answer at Coating 0.
        let plain = fresnel_cos(cos_i, n1, n2);
        let mix = coating_mix.clamp(0.0, 1.0);
        if reflect {
            ray.dir = reflect3(ray.dir, norm);
            for (k, t) in ray.thru.iter_mut().enumerate() {
                let coated =
                    refl_lookup(&baked.reflectance, s_idx, reverse, band.sub_idx[k], cos_i);
                let r = (plain + (coated - plain) * mix).clamp(0.0, 1.0);
                *t *= r;
            }
        } else {
            match refract3(ray.dir, norm, n1 / n2) {
                Some(d) => ray.dir = d,
                None => ray.rrel2 = ray.rrel2.max(4.0),
            }
            for (k, t) in ray.thru.iter_mut().enumerate() {
                let coated =
                    refl_lookup(&baked.reflectance, s_idx, reverse, band.sub_idx[k], cos_i);
                let r = (plain + (coated - plain) * mix).clamp(0.0, 1.0);
                *t *= 1.0 - r;
            }
            ray.ior = n2;
        }
        Some(())
    };

    // Phase 1: forward through 0..=b, reflecting at b.
    for s_idx in 0..=b_idx {
        let n2 = ior_at(&surfs[s_idx]);
        step(&mut ray, s_idx, n2, s_idx == b_idx, false)?;
    }
    // Phase 2: backward through b-1..=a, reflecting at a.
    for s_idx in (a_idx..b_idx).rev() {
        let reflect = s_idx == a_idx;
        let n2 = ior_before(s_idx);
        step(&mut ray, s_idx, n2, reflect, true)?;
        if reflect {
            ray.ior = ior_at(&surfs[s_idx]);
        }
    }
    // Phase 3: forward through a+1..n.
    for s_idx in (a_idx + 1)..n {
        let n2 = ior_at(&surfs[s_idx]);
        step(&mut ray, s_idx, n2, false, false)?;
    }

    if ray.dir[2].abs() < 1e-12 {
        return None;
    }
    let t = (baked.sensor_z_mm + sensor_shift_mm - ray.pos[2]) / ray.dir[2];
    // Negated deliberately, as in `trace_splat`: NaN must read as dead.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(t > 0.0) {
        return None;
    }
    let x = ray.pos[0] + ray.dir[0] * t;
    let y = ray.pos[1] + ray.dir[1] * t;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    // Band-integrate: rgb = sum over subs of throughput × CIE weight.
    let mut rgb = [0.0_f32; 3];
    for k in 0..SPECTRAL_SUB {
        let t = ray.thru[k];
        if !t.is_finite() {
            return None;
        }
        rgb[0] += t * band.sub_rgb[k][0];
        rgb[1] += t * band.sub_rgb[k][1];
        rgb[2] += t * band.sub_rgb[k][2];
    }
    // Housing feather: full inside 0.95, gone at 1.0 (smoothstep).
    let ft = ((1.0 - ray.rrel2.sqrt()) / 0.05).clamp(0.0, 1.0);
    Some(([x, y], ft * ft * (3.0 - 2.0 * ft), rgb))
}

// ---------------------------------------------------------------------------
// Pupil sampling (K-261): deterministic Halton spray over the front element,
// masked by the iris polygon with roundness and softness.
// ---------------------------------------------------------------------------

/// The pupil mask weight for a unit-disc point: 1 inside the iris shape,
/// feathering to 0 at the edge over `softness`. The polygon bound blends
/// toward the circle by `roundness`. Shared by the pupil grid and the
/// aperture image bake so the starburst and the ghosts agree.
pub fn pupil_mask(u: f32, v: f32, blades: u32, rot_rad: f32, roundness: f32, softness: f32) -> f32 {
    let r = (u * u + v * v).sqrt();
    let blades = blades.clamp(3, 16);
    let sector = std::f32::consts::TAU / blades as f32;
    let apothem = (std::f32::consts::PI / blades as f32).cos();
    let angle = v.atan2(u) - rot_rad;
    let mut a = angle % sector;
    if a < 0.0 {
        a += sector;
    }
    // Polygon radial bound at this angle, blended toward the unit circle.
    let poly_bound = apothem / (a - sector * 0.5).cos();
    let bound = poly_bound + (1.0 - poly_bound) * roundness.clamp(0.0, 1.0);
    let soft = (softness.clamp(0.0, 1.0) * bound).max(1e-4);
    let t = ((r - (bound - soft)) / soft).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// The pupil grid one ghost pair gets, from the Quality ladder's base
/// (times the Detail dial) and the pair's image spread (K-262, retuned
/// K-265). A frame-filling defocused ghost is undersampled by a flat grid
/// and shows its cell facets, so big spreads earn more. The K-262 HALF
/// rung for tight blobs is gone (K-265): a small ghost is not a cheap
/// ghost — its caustic rim carries structure the blob-size probe cannot
/// see, and the owner's EF 70-200 rendered that rim as sunflower teeth at
/// Ultra. Shared by the CPU reference and the GPU dispatch so both trace
/// the same rays.
/// The Quality tier's grid base scaled by the Detail dial (K-265), the
/// one place the multiplication lives so the CPU reference and the GPU
/// dispatch cannot disagree about rounding.
pub fn detail_base(tier_base: u32, detail: f32) -> u32 {
    ((tier_base as f32 * detail.clamp(0.25, 4.0)).round() as u32).max(8)
}

/// The Quality tier's traced wavelength count scaled by the Detail dial
/// (K-265). The dial must scale BOTH axes of the budget: the owner's
/// EF 70-200 wore a toothed corona that more rays barely touched, because
/// each of Ultra's 32 discrete wavelengths paints its own rim and the
/// teeth were the 32 rims fanned radially — spectral banding, dissolved
/// only by more bands. Capped at 64: combos scale linearly with it.
pub fn detail_lambda(tier_lambda: u32, detail: f32) -> u32 {
    ((tier_lambda as f32 * detail.clamp(0.25, 4.0)).round() as u32).clamp(3, 64)
}

/// The largest quad cell a frame tolerates, as a fraction of the sensor
/// diagonal (K-267): the frame-time probe below raises a pair's grid until
/// its worst cell sits under this. 0.005 of the diagonal is ~5.5 px at
/// 1080p — under the eye's line-detection threshold once 4×MSAA feathers
/// the edge.
pub const FRAME_CELL_FRAC: f32 = 0.005;

/// How far past the bake-spread grid the frame-time probe may push one
/// pair (K-267): a fold can demand an unbounded grid, and ray cost is
/// quadratic in the side, so the boost is capped at 3× the rung grid.
pub const FRAME_BOOST_CAP: u32 = 3;

/// Per-pair grid NEED for THIS FRAME's light (K-267). The bake spread is a
/// global bounding-box measure and misses folds: a pair whose image stays
/// the same overall size can still stretch 6× locally near a caustic at a
/// corner light, and those cells were the owner's choppy polyline edges.
/// A 12×12 weight-gated probe per renderable pair at the actual light
/// direction (the Hullin patent's "grid resolution adapted at runtime,
/// guided by bounding shape estimations") measures the worst adjacent-ray
/// landing distance; cell size shrinks inversely with the grid side, so
/// the side that puts the worst cell under [`FRAME_CELL_FRAC`] is
/// `(G−1) · d_max / target + 1`. Returned per pair (1.0 = no need);
/// [`boost_grid`] applies it WITHOUT ever lowering the bake floor. Manual
/// mode only — Matte lights exist GPU-side, so both twins keep the bake
/// grids there and parity holds.
pub(crate) fn frame_grid_needs(
    baked: &FlareBaked,
    pair_count: usize,
    dir: [f32; 3],
    coating: f32,
    stop_scale: f32,
    sensor_shift_mm: f32,
) -> Vec<f32> {
    const G: usize = 12;
    let sensor_diag = (SENSOR_MM[0] * SENSOR_MM[0] + SENSOR_MM[1] * SENSOR_MM[1]).sqrt();
    let target_mm = FRAME_CELL_FRAC * sensor_diag;
    let unit = |i: usize| (i as f32 / (G - 1) as f32) * 2.0 - 1.0;
    let mut landings: Vec<Option<[f32; 2]>> = vec![None; G * G];
    baked
        .pairs
        .iter()
        .take(pair_count)
        .map(|&pair| {
            for cell in landings.iter_mut() {
                *cell = None;
            }
            for j in 0..G {
                for i in 0..G {
                    let (u, v) = (unit(i), unit(j));
                    if u * u + v * v > 1.0 {
                        continue;
                    }
                    let o = [
                        u * baked.pupil_mm * stop_scale,
                        v * baked.pupil_mm * stop_scale,
                        baked.start_z_mm,
                    ];
                    landings[j * G + i] = trace_splat(
                        baked,
                        pair,
                        550.0,
                        o,
                        dir,
                        coating,
                        stop_scale,
                        sensor_shift_mm,
                    )
                    .and_then(|(pos, wgt)| (wgt > 1e-6).then_some(pos));
                }
            }
            let mut d_max = 0.0_f32;
            for j in 0..G {
                for i in 0..G {
                    let Some(a) = landings[j * G + i] else {
                        continue;
                    };
                    for (ni, nj) in [(i + 1, j), (i, j + 1)] {
                        if ni >= G || nj >= G {
                            continue;
                        }
                        if let Some(b) = landings[nj * G + ni] {
                            d_max = d_max.max((a[0] - b[0]).hypot(a[1] - b[1]));
                        }
                    }
                }
            }
            if d_max > 0.0 {
                ((G - 1) as f32 * d_max / target_mm + 1.0).min(4096.0)
            } else {
                1.0
            }
        })
        .collect()
}

/// Apply one pair's frame-time grid need to its bake-spread grid (K-267):
/// never below the bake floor, never past [`FRAME_BOOST_CAP`]× it, always
/// inside `pair_grid`'s 8..512 clamp.
pub(crate) fn boost_grid(pair_grid: u32, need: f32) -> u32 {
    let cap = pair_grid.saturating_mul(FRAME_BOOST_CAP);
    pair_grid.max((need.round() as u32).min(cap)).clamp(8, 512)
}

/// Extra rays the frame-time probe may add, as a fraction of the frame's
/// rung baseline (K-267). Uncapped, the probe septupled a frame's cost —
/// every fold demanded its 3× grid at once. Half again over the baseline
/// keeps the boost a bounded, predictable spend.
pub const FRAME_RAY_HEADROOM: f32 = 0.5;

/// The frame's final per-pair grids (K-267): each pair's K-262 rung grid,
/// raised toward its [`frame_grid_needs`] want under a shared ray budget —
/// [`FRAME_RAY_HEADROOM`] extra over the rung baseline, spent worst local
/// stretch first (ties keep rank order, §2.4 determinism), partial grants
/// when the budget runs short. Computed ONCE per frame in lumit-core and
/// carried across the GPU seam as plain numbers, so the CPU reference and
/// the GPU dispatch cannot disagree about a single ray.
pub fn plan_frame_grids(base_side: u32, spreads: &[f32], needs: &[f32]) -> Vec<u32> {
    let n = needs.len();
    let rung: Vec<u32> = (0..n)
        .map(|i| pair_grid(base_side, spreads.get(i).copied().unwrap_or(1.0)))
        .collect();
    let mut out = rung.clone();
    let sq = |g: u32| u64::from(g) * u64::from(g);
    let baseline: u64 = rung.iter().map(|&g| sq(g)).sum();
    let mut budget = (baseline as f64 * f64::from(FRAME_RAY_HEADROOM)) as u64;
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        let ra = needs[a] / rung[a] as f32;
        let rb = needs[b] / rung[b] as f32;
        rb.partial_cmp(&ra)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    for i in order {
        let want = boost_grid(rung[i], needs[i]);
        if want <= rung[i] || budget == 0 {
            continue;
        }
        let extra = sq(want) - sq(rung[i]);
        let grant = if extra <= budget {
            want
        } else {
            // Partial grant: the side whose square spends what remains.
            ((budget + sq(rung[i])) as f64).sqrt().floor() as u32
        };
        let grant = grant.clamp(rung[i], 512);
        budget -= sq(grant) - sq(rung[i]);
        out[i] = grant;
    }
    out
}

/// [`frame_grid_needs`] from the plain buffers the GPU seam carries (the
/// `FlareBakeData` shape — K-267): lumit-render runs the probe against the
/// GPU's cached bake without re-baking, so the surface table arrives as raw
/// rows. Fields the probe never reads (native f-stop, front aperture, gain,
/// starburst) are zeroed in the view.
#[allow(clippy::too_many_arguments)]
pub fn frame_grid_needs_from_rows(
    surfaces: &[[f32; 8]],
    pairs: &[[u32; 2]],
    sensor_z_mm: f32,
    focal_mm: f32,
    pupil_mm: f32,
    start_z_mm: f32,
    pair_count: usize,
    dir: [f32; 3],
    coating: f32,
    stop_scale: f32,
    sensor_shift_mm: f32,
) -> Vec<f32> {
    let view = FlareBaked {
        surfaces: surfaces
            .iter()
            .map(|r| FlareSurface {
                radius_mm: r[0],
                z_mm: r[1],
                semi_ap_mm: r[2],
                cauchy_a: r[3],
                cauchy_b: r[4],
                coating_layers: r[5],
                is_stop: r[6],
                _pad: 0.0,
            })
            .collect(),
        sensor_z_mm,
        focal_mm,
        native_fstop: 0.0,
        front_semi_ap: 0.0,
        pupil_mm,
        start_z_mm,
        pairs: pairs.to_vec(),
        spreads: Vec::new(),
        energy_gain: 0.0,
        starburst: Vec::new(),
        reflectance: Vec::new(),
    };
    frame_grid_needs(&view, pair_count, dir, coating, stop_scale, sensor_shift_mm)
}

pub fn pair_grid(base: u32, spread: f32) -> u32 {
    let mult = if spread < 0.5 {
        1.0
    } else if spread < 1.5 {
        1.75
    } else {
        2.5
    };
    ((base as f32 * mult).round() as u32).clamp(8, 512)
}

/// The flare buffer's dimensions once padding for Squeeze/Scale is applied
/// (K-267). A squeeze or Scale below 1 samples PAST the base buffer in the
/// combine, and K-266's zero-outside tap honestly showed nothing there —
/// the owner's "cuts to black at the edges". The buffer renders larger
/// instead, up to 2× per axis (the working Squeeze floor is 0.5), with the
/// geometry centred so the combine only adds a constant offset. Mirrored
/// in lumit-gpu (pinned by test); the clamps match `cpu_combine`'s.
pub fn flare_pad_dims(fw: u32, fh: u32, squeeze: f32, scale: f32) -> (u32, u32) {
    let squeeze = squeeze.clamp(0.25, 4.0);
    let fscale = scale.clamp(0.05, 20.0);
    let px = (1.0 / (squeeze * fscale)).clamp(1.0, 2.0);
    let py = (1.0 / fscale).clamp(1.0, 2.0);
    (
        ((fw as f32) * px).ceil() as u32,
        ((fh as f32) * py).ceil() as u32,
    )
}

/// The effective iris roundness for a working f-stop (K-260): wide open a
/// real iris retracts behind the housing's circular bore, so ghosts go
/// round whatever the blade count; two stops down the polygon is fully
/// back.
pub fn effective_roundness(roundness: f32, fstop: f32, native_fstop: f32) -> f32 {
    let native = native_fstop.max(0.7);
    let wide_open = (1.0 - (fstop / native - 1.0).clamp(0.0, 2.0) / 2.0).clamp(0.0, 1.0);
    roundness.max(wide_open)
}

// ---------------------------------------------------------------------------
// Bake
// ---------------------------------------------------------------------------

/// The bake-relevant parameter subset hashed into the cache key: everything
/// the baked tables depend on, quantised through `to_bits` so equal floats
/// key equally. Light position, intensities, dispersion, coating, quality
/// and mix are frame-time inputs and deliberately absent — animating them
/// never rebakes.
pub fn bake_key(p: &LensFlareParams) -> u64 {
    bake_key_with(p, None)
}

/// [`bake_key`] with a custom prescription in play (K-264): the `lens_file`
/// override's content hash folds in, so editing the file (or clearing it)
/// rebakes and two different files never share a cache slot.
pub fn bake_key_with(p: &LensFlareParams, lens_text_hash: Option<u64>) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64; // FNV offset basis
    let mut fold = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    fold(p.lens as u64);
    fold(p.fstop.to_bits() as u64);
    fold(p.blades as u64);
    fold(p.aperture_rotation_deg.to_bits() as u64);
    fold(p.roundness.to_bits() as u64);
    fold(p.aperture_softness.to_bits() as u64);
    if let Some(text_hash) = lens_text_hash {
        fold(1);
        fold(text_hash);
    }
    h
}

/// FNV-1a of a custom .lens file's text — [`bake_key_with`]'s ingredient,
/// computed by the caller that read the file (lumit-render, following the
/// LUT pattern of doing the IO outside the engine-pure bake).
pub fn lens_text_hash(text: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The aperture image for the starburst FFT: the pupil mask rendered into a
/// texture (iris at 0.75 of the half-extent, leaving rim room for the
/// diffraction spread).
pub(crate) fn bake_aperture(p: &LensFlareParams, native_fstop: f32, res: u32) -> Vec<f32> {
    let n = res as usize;
    let mut img = vec![0.0_f32; n * n];
    let rot = p.aperture_rotation_deg.to_radians();
    let roundness = effective_roundness(p.roundness, p.fstop, native_fstop);
    let softness = (p.aperture_softness * 0.25).max(0.004);
    let size = 0.75_f32;
    for y in 0..n {
        for x in 0..n {
            let ndc_x = 2.0 * (x as f32 / (n - 1) as f32) - 1.0;
            let ndc_y = 2.0 * (y as f32 / (n - 1) as f32) - 1.0;
            img[y * n + x] = pupil_mask(
                ndc_x / size,
                ndc_y / size,
                p.blades,
                rot,
                roundness,
                softness,
            );
        }
    }
    img
}

/// The starburst sprite: the aperture's Fourier amplitude under the Fresnel
/// propagation term, integrated over the visible spectrum with the chromatic
/// scale `λ_mid/λ`, CIE-weighted into linear working RGB ([Ritschel 2009]
/// §4–5). Peak-normalised so blade edits keep overall brightness.
pub(crate) fn bake_starburst(aperture: &[f32], res: u32) -> Vec<f32> {
    let n = res as usize;
    // Pattern: |fftshift(fft(A · e^{iπ/(λd)(x²+y²)}))|, λ_mid, d = 1 m.
    let lambda_mm = cie::LAMBDA_MID as f64 * 1e-6;
    let d_mm = 1.0e3_f64;
    let mut cx = vec![Cx::ZERO; n * n];
    for y in 0..n {
        let ny = 2.0 * (y as f64 / (n - 1) as f64) - 1.0;
        for x in 0..n {
            let nx = 2.0 * (x as f64 / (n - 1) as f64) - 1.0;
            let arg = std::f64::consts::PI / (lambda_mm * d_mm) * (nx * nx + ny * ny);
            cx[y * n + x] = Cx::cis(arg).scale(aperture[y * n + x] as f64);
        }
    }
    fft2_inplace(&mut cx, n, n, false);
    fftshift2(&mut cx, n, n);
    // Amplitude, not power: |F| instead of |F|² — the power spectrum's DC
    // core sits orders of magnitude above the blade streaks, so after
    // normalisation the spikes vanish; the amplitude spectrum keeps them at
    // a displayable ~1e-2 of the core, which is how the reference apps'
    // starbursts read (the core clips to white either way).
    let pattern: Vec<f32> = cx.iter().map(|z| z.norm_sq().sqrt() as f32).collect();

    // Spectral integration into RGB.
    let samples = STARBURST_SAMPLES;
    let mut out = vec![0.0_f32; n * n * 3];
    let range = cie::LAMBDA_MAX - cie::LAMBDA_MIN;
    let bilinear = |u: f32, v: f32| -> f32 {
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return 0.0;
        }
        let fx = u * (n - 1) as f32;
        let fy = v * (n - 1) as f32;
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(n - 1);
        let y1 = (y0 + 1).min(n - 1);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let a = pattern[y0 * n + x0] * (1.0 - tx) + pattern[y0 * n + x1] * tx;
        let b = pattern[y1 * n + x0] * (1.0 - tx) + pattern[y1 * n + x1] * tx;
        a * (1.0 - ty) + b * ty
    };
    // The spectral ladder, built ONCE (K-263): its chromatic scale and its
    // colour-matching weights are the same for every texel of the sprite, but
    // they used to be derived inside the per-texel loop — so the CIE table was
    // interpolated 6.5 million times to produce a hundred distinct answers.
    // Same numbers, same order, hoisted out.
    let bands: Vec<(f32, [f32; 3])> = (0..samples)
        .map(|k| {
            let step = k as f32 / samples as f32;
            let lambda = cie::LAMBDA_MIN + step * range;
            // Chromatic scale: diffraction grows with wavelength, so the
            // sample position shrinks by λ_mid/λ.
            (lambda / cie::LAMBDA_MID, cie::xyz_at(lambda))
        })
        .collect();
    for y in 0..n {
        for x in 0..n {
            let ndc_x = 2.0 * (x as f32 / (n - 1) as f32) - 1.0;
            let ndc_y = 2.0 * (y as f32 / (n - 1) as f32) - 1.0;
            let mut xyz = [0.0_f32; 3];
            for &(s, w) in &bands {
                let px = ndc_x / s;
                let py = ndc_y / s;
                let val = bilinear(px * 0.5 + 0.5, py * 0.5 + 0.5);
                xyz[0] += w[0] * val;
                xyz[1] += w[1] * val;
                xyz[2] += w[2] * val;
            }
            let rgb = cie::xyz_to_linear_rgb(xyz);
            let i = (y * n + x) * 3;
            out[i] = rgb[0].max(0.0);
            out[i + 1] = rgb[1].max(0.0);
            out[i + 2] = rgb[2].max(0.0);
        }
    }
    // Peak-normalise: the brightest texel becomes 1, the intensity dials
    // own the rest.
    let peak = out.iter().fold(0.0_f32, |m, &v| m.max(v)).max(1e-9);
    for v in out.iter_mut() {
        *v /= peak;
    }
    // Fade the sprite to zero inside its own border (K-264). The
    // diffraction halo's pedestal ran right to the texture edge, and the
    // combine stamps the sprite as a quad — so on a dark scene every
    // starburst sat in a hard-edged grey SQUARE. The window is RADIAL, not
    // square: a square window only softened the seam and left a square
    // halo, and light around a point source falls off in circles. The core
    // and the spike bodies sit well inside r = 0.7 and keep their energy;
    // only the pedestal's rim fades.
    for y in 0..n {
        let ny = 2.0 * (y as f32 / (n - 1) as f32) - 1.0;
        for x in 0..n {
            let nx = 2.0 * (x as f32 / (n - 1) as f32) - 1.0;
            let r = nx.hypot(ny);
            let t = ((r - 0.7) / 0.3).clamp(0.0, 1.0);
            let window = 1.0 - t * t * (3.0 - 2.0 * t);
            let i = (y * n + x) * 3;
            out[i] *= window;
            out[i + 1] *= window;
            out[i + 2] *= window;
        }
    }
    out
}

/// The mean flare-buffer brightness the auto-exposure steers every lens
/// toward, measured by actually rendering the CPU reference at thumbnail
/// size inside the bake (K-258). Cheaper proxies mispredicted real lenses
/// by orders of magnitude; the closed loop cannot.
const TARGET_PROBE_MEAN: f32 = 0.010;

/// Run the full bake for a params bundle — pure, deterministic, CPU-only
/// (K-261): parse the prescription, enumerate and rank the ghost pairs,
/// measure per-pair defocus boosts, bake the starburst, close the exposure
/// loop.
pub fn bake(p: &LensFlareParams) -> FlareBaked {
    bake_with(p, None)
}

/// [`bake`] with an optional custom .lens text (K-264, the `lens_file`
/// parameter): parsed, it replaces the library pick entirely — its native
/// f-number estimated from the geometry, since only the bundled collection
/// carries one in its filename. Unparsable (or absent) degrades to the
/// picked library lens: a labelled fallback, never a fault, and exactly
/// what an unset parameter renders.
pub fn bake_with(p: &LensFlareParams, lens_text: Option<&str>) -> FlareBaked {
    let entry = lens_entry(p.lens);
    let custom = lens_text.and_then(parse_lens);
    let entry_native = if custom.is_some() {
        0.0
    } else {
        entry.native_fstop
    };
    let lens = custom
        .or_else(|| parse_lens(entry.text))
        .unwrap_or_else(|| Prescription {
            // A degenerate fallback biconvex singlet: the library is regression-
            // tested to parse in full, so this exists only to keep the engine
            // panic-free if a future import breaks a file.
            focal_mm: 50.0,
            surfaces: vec![
                FlareSurface {
                    radius_mm: 50.0,
                    z_mm: 0.0,
                    semi_ap_mm: 15.0,
                    cauchy_a: 1.5,
                    cauchy_b: 0.004,
                    coating_layers: 0.0,
                    is_stop: 0.0,
                    _pad: 0.0,
                },
                FlareSurface {
                    radius_mm: 0.0,
                    z_mm: 4.0,
                    semi_ap_mm: 12.0,
                    cauchy_a: 1.0,
                    cauchy_b: 0.0,
                    coating_layers: 0.0,
                    is_stop: 1.0,
                    _pad: 0.0,
                },
                FlareSurface {
                    radius_mm: -50.0,
                    z_mm: 8.0,
                    semi_ap_mm: 15.0,
                    cauchy_a: 1.0,
                    cauchy_b: 0.0,
                    coating_layers: 0.0,
                    is_stop: 0.0,
                    _pad: 0.0,
                },
            ],
            sensor_z_mm: 55.0,
        });
    let front_semi_ap = lens.surfaces[0].semi_ap_mm;
    let native_fstop = if entry_native > 0.0 {
        entry_native
    } else {
        (lens.focal_mm / (2.0 * front_semi_ap.max(0.1))).max(0.7)
    };
    let pupil_mm = (lens.focal_mm / (2.0 * native_fstop) * 1.5).clamp(1.0, front_semi_ap);
    let mut baked = FlareBaked {
        pupil_mm,
        start_z_mm: lens.surfaces[0].z_mm - START_Z_BACKOFF_MM,
        sensor_z_mm: lens.sensor_z_mm,
        focal_mm: lens.focal_mm,
        native_fstop,
        front_semi_ap,
        reflectance: bake_reflectance(&lens.surfaces),
        surfaces: lens.surfaces,
        pairs: Vec::new(),
        spreads: Vec::new(),
        energy_gain: 1.0,
        starburst: Vec::new(),
    };

    // Enumerate every a<b pair where both surfaces actually change medium
    // (a reflection needs an interface; the stop is air-air and drops out),
    // probe each on-axis, rank by probe brightness.
    let n = baked.surfaces.len();
    let ior_at =
        |i: usize| baked.surfaces[i].cauchy_a + baked.surfaces[i].cauchy_b / (0.587_56 * 0.587_56);
    let has_interface = |i: usize| {
        let before = if i == 0 { 1.0 } else { ior_at(i - 1) };
        (before - ior_at(i)).abs() >= 0.001
    };
    let centre = [0.0, 0.0, baked.start_z_mm];
    let axis = [0.0, 0.0, 1.0];
    // Every candidate first, then the probes in parallel (the bake was a
    // one-core wait on a many-core machine). Each pair's probe is
    // independent and `collect` keeps input order, so the ranking sees the
    // same numbers in the same order as the serial loop did — determinism
    // by construction, not by luck (docs/14).
    let candidates: Vec<[u32; 2]> = (0..n)
        .filter(|&a| has_interface(a))
        .flat_map(|a| {
            ((a + 1)..n)
                .filter(|&b| has_interface(b))
                .map(move |b| [a as u32, b as u32])
        })
        .collect();
    use rayon::prelude::*;
    let ranked: Vec<([u32; 2], f32)> = candidates
        .par_iter()
        .filter_map(|&pair| {
            // On-axis brightness probe at the R/G/B wavelengths, full file
            // coating (the Coating dial is frame-time).
            let mut est = 0.0_f32;
            for nm in [650.0, 550.0, 450.0] {
                if let Some((_, w)) = trace_splat(&baked, pair, nm, centre, axis, 1.0, 1.0, 0.0) {
                    est += w;
                }
            }
            est /= 3.0;
            (est >= PAIR_MIN_INTENSITY).then_some((pair, est))
        })
        .collect();
    let mut ranked = ranked;
    // Descending probe brightness; ties by pair order (deterministic).
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    baked.pairs = ranked.iter().map(|&(g, _)| g).collect();
    // Image extent per pair: an 8×8 on-axis spray's landing bounding box
    // against the sensor diagonal (K-262) — the adaptive grid budget's input.
    //
    // Measured AFTER the ranking and only for the pairs a frame can reach
    // (K-263). It costs 64 traced rays a pair, and a 60-surface prescription
    // leaves well over a thousand surviving pairs of which a frame renders at
    // most `MAX_RENDERED_PAIRS` — so probing them all spent most of the bake
    // measuring ghosts nothing would ever draw. Beyond the cap the spread is
    // the neutral 1.0, the same value an unmeasurable pair has always had.
    let mut spreads = vec![1.0_f32; baked.pairs.len()];
    let sensor_diag = (SENSOR_MM[0] * SENSOR_MM[0] + SENSOR_MM[1] * SENSOR_MM[1]).sqrt();
    let probe_pairs: Vec<[u32; 2]> = baked
        .pairs
        .iter()
        .take(MAX_RENDERED_PAIRS)
        .copied()
        .collect();
    // Two probe directions per pair (K-264): on-axis, and a representative
    // off-axis beam (a light a third of the way into the frame). The K-262
    // probe was on-axis only, and some designs land a COMPACT on-axis ghost
    // that fills the frame the moment the light moves off-centre — those
    // pairs were handed the half grid and rendered their wash as 9 px
    // staircase blocks. The budget takes the larger of the two answers.
    let off_axis = light_direction([0.33, 0.30], 0.5625, baked.focal_mm);
    let probed: Vec<f32> = probe_pairs
        .par_iter()
        .map(|&pair| {
            const G: u32 = 8;
            let mut spread = 0.0_f32;
            for dir in [axis, off_axis] {
                let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
                let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
                let mut seen = 0u32;
                for gy in 0..G {
                    for gx in 0..G {
                        let u = ((gx as f32 + 0.5) / G as f32) * 2.0 - 1.0;
                        let v = ((gy as f32 + 0.5) / G as f32) * 2.0 - 1.0;
                        if u * u + v * v > 1.0 {
                            continue;
                        }
                        let o = [u * baked.pupil_mm, v * baked.pupil_mm, baked.start_z_mm];
                        // Zero-weight rays are K-264's virtual continuations —
                        // real geometry, no light — and must not widen the
                        // measured image extent.
                        if let Some((pos, wgt)) =
                            trace_splat(&baked, pair, 550.0, o, dir, 1.0, 1.0, 0.0)
                        {
                            if wgt <= 1e-6 {
                                continue;
                            }
                            min_x = min_x.min(pos[0]);
                            max_x = max_x.max(pos[0]);
                            min_y = min_y.min(pos[1]);
                            max_y = max_y.max(pos[1]);
                            seen += 1;
                        }
                    }
                }
                if seen >= 2 {
                    spread = spread
                        .max(((max_x - min_x).hypot(max_y - min_y) / sensor_diag).clamp(0.0, 8.0));
                }
            }
            spread
        })
        .collect();
    for (pi, &spread) in probed.iter().enumerate() {
        if spread > 0.0 {
            spreads[pi] = spread;
        }
    }
    baked.spreads = spreads;

    let aperture = bake_aperture(p, native_fstop, APERTURE_RES);
    baked.starburst = bake_starburst(&aperture, STARBURST_RES);

    // Closed-loop auto exposure (K-258): render the reference thumbnail with
    // gain 1 at FIXED frame-time settings — only bake-key inputs may steer
    // the gain, or animating a frame-time dial would rebake — and normalise
    // the mean to the target. Deterministic, and a few milliseconds.
    baked.energy_gain = 1.0;
    let probe_frame = LensFlareParams {
        // Raster pixels of the 96×54 thumbnail (the 0.33/0.30 framing).
        light: [31.7, 16.2],
        // The probe is always one Manual point, whatever the frame's source
        // mode: the gain must be steered by bake-key inputs alone, and the
        // comp's lights are frame-time.
        lights: [DEAD_LIGHT; MAX_SOURCES],
        light_count: 0,
        intensity: 1.0,
        lens: p.lens,
        fstop: p.fstop,
        focus_m: 100.0,
        blades: p.blades,
        aperture_rotation_deg: p.aperture_rotation_deg,
        roundness: p.roundness,
        aperture_softness: p.aperture_softness,
        ghost_intensity: 1.0,
        // A point source for the exposure probe whatever the user's Source
        // size: the gain must be steered by bake-key inputs alone, and an
        // area source is a frame-time dial.
        source_size: [0.0, 0.0],
        ghost_softness: 0.05,
        max_ghosts: 32,
        dispersion: 1.0,
        coating: 0.6,
        starburst_intensity: 0.0,
        scale: 1.0,
        source: 0,
        threshold: 1.0,
        threshold_softness: 0.25,
        light_tint: [1.0, 1.0, 1.0],
        use_source_colour: true,
        anamorphic: 1.0,
        quality: 0,
        detail: 1.0,
        blend: BLEND_ADD,
        mix: 1.0,
    };
    let (pw, ph) = (96u32, 54u32);
    let thumb = cpu_flare(
        &probe_frame,
        &baked,
        pw,
        ph,
        &manual_light(&probe_frame, pw, ph),
    );
    let mean: f32 = thumb.iter().sum::<f32>() / thumb.len().max(1) as f32;
    // The gain ceiling matters (K-261): a lens whose every ghost is an
    // extreme defocused wash has almost no probe energy after the
    // giant-quad fade, and an unbounded loop would amplify the residue into
    // a lit-up artefact field. Capped, such a lens renders honestly dim —
    // a bright star and little else, which is what that glass does.
    baked.energy_gain = if mean > 1e-12 {
        (TARGET_PROBE_MEAN / mean).clamp(1e-2, 64.0)
    } else {
        1.0
    };
    baked
}

// ---------------------------------------------------------------------------
// Frame-time shared derivations (CPU reference and GPU uniforms).
// ---------------------------------------------------------------------------

/// The traced wavelengths with their linear-RGB weights. Each traced λ
/// *represents its whole band* of the visible range, so its weight is the
/// band's INTEGRAL of the colour-matching functions (sampled at 2 nm), not a
/// point sample — a 3-band ladder point-sampled at 673 nm would weigh red at
/// a tenth of its true energy and tint every flare blue-green (found by
/// eye). Brightness-normalised by ΣY so the wavelength count (a quality
/// setting) does not change exposure.
pub fn lambda_weights(count: u32, dispersion: f32) -> Vec<(f32, [f32; 3])> {
    let ladder = cie::wavelength_ladder(count as usize, dispersion);
    let band = (cie::LAMBDA_MAX - cie::LAMBDA_MIN) / count.max(1) as f32;
    let band_xyz: Vec<[f32; 3]> = (0..ladder.len())
        .map(|k| {
            let lo = cie::LAMBDA_MIN + band * k as f32;
            let mut acc = [0.0_f32; 3];
            let steps = (band / 2.0).ceil().max(1.0) as usize;
            for i in 0..steps {
                let nm = lo + band * (i as f32 + 0.5) / steps as f32;
                let w = cie::xyz_at(nm);
                acc[0] += w[0];
                acc[1] += w[1];
                acc[2] += w[2];
            }
            let inv = 1.0 / steps as f32;
            [acc[0] * inv, acc[1] * inv, acc[2] * inv]
        })
        .collect();
    let sum_y: f32 = band_xyz.iter().map(|w| w[1]).sum();
    let norm = 1.0 / sum_y.max(1e-6);
    ladder
        .iter()
        .zip(band_xyz)
        .map(|(&(traced_nm, _), xyz)| {
            let rgb = cie::xyz_to_linear_rgb(xyz);
            (
                traced_nm,
                [
                    rgb[0].max(0.0) * norm,
                    rgb[1].max(0.0) * norm,
                    rgb[2].max(0.0) * norm,
                ],
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Spectral radiometry (K-364, entry A2)
// ---------------------------------------------------------------------------

/// Lambda samples in the baked reflectance table: [`cie::LAMBDA_MIN`] to
/// [`cie::LAMBDA_MAX`] inclusive at 5 nm - fine enough to resolve a 7-layer
/// stack's W-shaped reflectance, which the traced bands alone cannot.
pub const REFL_LAMBDA_BINS: usize = 69;

/// cos theta bins in the baked reflectance table (bin centres, linear).
pub const REFL_COS_BINS: usize = 16;

/// Radiometric sub-samples per traced wavelength band (K-364). Geometry
/// varies slowly with lambda (dispersion is smooth), so the ray path is
/// traced once per band - but the coating reflectance oscillates several
/// times across the visible, so each ray's *energy* is integrated at 8
/// points across its band. Even the lowest quality tier then samples the
/// spectrum 24 times where it sampled 3.
pub const SPECTRAL_SUB: usize = 8;

/// One traced wavelength band with its radiometric sub-samples (K-364):
/// the geometry wavelength, and per sub-sample the reflectance table's
/// lambda index plus the CIE weight (already through XYZ to RGB and the
/// ladder's Y-normalisation). The sub-weights of a band sum to what
/// [`lambda_weights`] gave the whole band — exactly in Y, and up to the
/// out-of-gamut clamp in R and B, which now applies per sub-sample rather
/// than per band and so throws strictly less away — so a spectrally flat
/// throughput renders at the old exposure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralBand {
    /// The wavelength the geometry is traced at (dispersion-scaled).
    pub traced_nm: f32,
    /// Reflectance-table lambda index per sub-sample (the sub-lambda
    /// snapped to the table's 5 nm grid, so the lookup is exact in lambda).
    pub sub_idx: [u32; SPECTRAL_SUB],
    /// Linear working RGB weight per sub-sample.
    pub sub_rgb: [[f32; 3]; SPECTRAL_SUB],
}

/// The traced bands with their radiometric sub-samples - [`lambda_weights`]
/// refined (K-364). Same geometry ladder, same total normalisation; each
/// band's single RGB weight becomes [`SPECTRAL_SUB`] weights whose sum is
/// the old value (XYZ to RGB is linear, so splitting the band's CIE
/// integral splits its RGB weight exactly).
pub fn spectral_bands(count: u32, dispersion: f32) -> Vec<SpectralBand> {
    let ladder = cie::wavelength_ladder(count as usize, dispersion);
    let count = count.max(1) as usize;
    let range = cie::LAMBDA_MAX - cie::LAMBDA_MIN;
    let band_w = range / count as f32;
    // Per band, per sub: the mean CIE XYZ over the sub-span (matching
    // lambda_weights' ~2 nm integration step), divided by SPECTRAL_SUB so
    // the subs sum to the band mean.
    let mut bands_xyz: Vec<[[f32; 3]; SPECTRAL_SUB]> = Vec::with_capacity(count);
    let mut sum_y = 0.0_f32;
    for k in 0..count {
        let lo = cie::LAMBDA_MIN + band_w * k as f32;
        let sub_w = band_w / SPECTRAL_SUB as f32;
        let mut subs = [[0.0_f32; 3]; SPECTRAL_SUB];
        for (si, sub) in subs.iter_mut().enumerate() {
            let s_lo = lo + sub_w * si as f32;
            let steps = (sub_w / 2.0).ceil().max(1.0) as usize;
            let mut acc = [0.0_f32; 3];
            for i in 0..steps {
                let nm = s_lo + sub_w * (i as f32 + 0.5) / steps as f32;
                let w = cie::xyz_at(nm);
                acc[0] += w[0];
                acc[1] += w[1];
                acc[2] += w[2];
            }
            let inv = 1.0 / (steps * SPECTRAL_SUB) as f32;
            *sub = [acc[0] * inv, acc[1] * inv, acc[2] * inv];
            sum_y += sub[1];
        }
        bands_xyz.push(subs);
    }
    let norm = 1.0 / sum_y.max(1e-6);
    ladder
        .iter()
        .zip(bands_xyz)
        .enumerate()
        .map(|(k, (&(traced_nm, _), subs))| {
            let lo = cie::LAMBDA_MIN + band_w * k as f32;
            let sub_w = band_w / SPECTRAL_SUB as f32;
            let mut sub_idx = [0u32; SPECTRAL_SUB];
            let mut sub_rgb = [[0.0_f32; 3]; SPECTRAL_SUB];
            for si in 0..SPECTRAL_SUB {
                let nm = lo + sub_w * (si as f32 + 0.5);
                sub_idx[si] = refl_lambda_index(nm);
                let rgb = cie::xyz_to_linear_rgb(subs[si]);
                sub_rgb[si] = [
                    rgb[0].max(0.0) * norm,
                    rgb[1].max(0.0) * norm,
                    rgb[2].max(0.0) * norm,
                ];
            }
            SpectralBand {
                traced_nm,
                sub_idx,
                sub_rgb,
            }
        })
        .collect()
}

/// A wavelength snapped to the reflectance table's 5 nm grid.
pub fn refl_lambda_index(nm: f32) -> u32 {
    let step = (cie::LAMBDA_MAX - cie::LAMBDA_MIN) / (REFL_LAMBDA_BINS - 1) as f32;
    (((nm - cie::LAMBDA_MIN) / step).round() as i64).clamp(0, REFL_LAMBDA_BINS as i64 - 1) as u32
}

/// Bake the per-surface reflectance table (see [`FlareBaked::reflectance`]).
/// Direction 0 keys `(n1 = medium before, n2 = medium after)`; direction 1
/// swaps them, which is what a phase-2 backward crossing evaluates.
fn bake_reflectance(surfs: &[FlareSurface]) -> Vec<f32> {
    let step = (cie::LAMBDA_MAX - cie::LAMBDA_MIN) / (REFL_LAMBDA_BINS - 1) as f32;
    let mut out = vec![0.0_f32; surfs.len() * 2 * REFL_LAMBDA_BINS * REFL_COS_BINS];
    for (si, s) in surfs.iter().enumerate() {
        let stack = coating_stack(s.coating_layers);
        for dir in 0..2usize {
            for li in 0..REFL_LAMBDA_BINS {
                let nm = cie::LAMBDA_MIN + step * li as f32;
                let before = if si == 0 {
                    1.0
                } else {
                    cauchy_ior(surfs[si - 1].cauchy_a, surfs[si - 1].cauchy_b, nm)
                };
                let after = cauchy_ior(s.cauchy_a, s.cauchy_b, nm);
                let (n1, n2) = if dir == 0 {
                    (before, after)
                } else {
                    (after, before)
                };
                for ci in 0..REFL_COS_BINS {
                    let cos_i = (ci as f32 + 0.5) / REFL_COS_BINS as f32;
                    let r = if s.coating_layers < 0.5 {
                        fresnel_cos(cos_i, n1, n2)
                    } else {
                        stack_reflectance(cos_i, n1, n2, &stack, nm)
                    };
                    out[((si * 2 + dir) * REFL_LAMBDA_BINS + li) * REFL_COS_BINS + ci] = r;
                }
            }
        }
    }
    out
}

/// Read the baked table at (surface, direction, lambda index, cos theta):
/// linear interpolation across the cos bins, exact in lambda (the caller's
/// index is already on the grid). The WGSL trace mirrors this arithmetic
/// op for op.
pub fn refl_lookup(table: &[f32], surf: usize, reverse: bool, lambda_idx: u32, cos_i: f32) -> f32 {
    let base = ((surf * 2 + usize::from(reverse)) * REFL_LAMBDA_BINS + lambda_idx as usize)
        * REFL_COS_BINS;
    let c = cos_i.clamp(0.0, 1.0) * REFL_COS_BINS as f32 - 0.5;
    let j0 = (c.floor().max(0.0) as usize).min(REFL_COS_BINS - 1);
    let j1 = (j0 + 1).min(REFL_COS_BINS - 1);
    let f = (c - j0 as f32).clamp(0.0, 1.0);
    let (Some(&a), Some(&b)) = (table.get(base + j0), table.get(base + j1)) else {
        return 0.0;
    };
    a + (b - a) * f
}

/// Raster pixels per sensor mm for a `w`-wide target: resolution-independent
/// framing (§2.3), matching [`light_direction`]'s half-sensor convention.
pub fn screen_transform(w: u32) -> f32 {
    w as f32 / SENSOR_MM[0]
}

// ---------------------------------------------------------------------------
// CPU reference renderer (the §1.6 staged oracle's frame side; not a
// production path — the CPU degradation rung renders the effect as identity,
// the K-114/K-256 pattern).
// ---------------------------------------------------------------------------

/// One rasterisation vertex (matches the WGSL vertex buffer): raster
/// position, RGB-weighted intensity.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlareVertex {
    pub pos: [f32; 2],
    pub rgb: [f32; 3],
}

/// Minimum screen area a drawn quad may have, px² (K-261, retuned K-264).
/// A caustic-folded quad shrinks below a pixel, and a centre-sampled
/// rasteriser drops triangles that cover no pixel centre — deleting exactly
/// the flux that makes a flare's bright rims and fold lines. Quads below
/// this inflate about their centroid to this area with their colour scaled
/// by (true / inflated) area, so the deposited flux is conserved.
///
/// One px², down from K-261's four (K-264): the floor was sized for a
/// raster that sampled once per pixel, and at four it also caught the
/// MERELY SMALL — a defocused wash ghost rendered small (a Draft preview, a
/// zoomed-out Viewer) has every cell at a couple of px², and inflating them
/// all tiled the ghost with overlapping diamonds, a mosaic. With the 4×
/// multisampled raster (and its coverage-sampling CPU twin) a one-px² quad
/// reliably covers samples, so only truly sub-sample cells need rescuing.
pub const MIN_QUAD_PX: f32 = 1.0;
/// Longest screen edge a quad may have and still be *inflated* (K-262). A
/// cell whose ray corners straddle a caustic fold or a housing clip lands
/// as a SLIVER: near-zero area but large extent. Scaling such a quad up to
/// [`MIN_QUAD_PX`] stretches it along its long axis — a 20 px sliver became
/// a 2000 px line, which is the "random lines across the flare" artefact.
/// Slivers are dropped instead: their flux is genuinely smeared to nothing,
/// and the neighbouring well-formed cells carry the ghost's light.
pub const MAX_INFLATE_EDGE_PX: f32 = 6.0;
/// Floor on a landed quad's area as a fraction of its launch cell — which
/// is really a **cap on caustic density** (K-262). At a fold the density
/// `cell ÷ landed` genuinely diverges, but the *integral* over a pixel is
/// finite: our discrete cell concentrates that whole divergence into a few
/// pixels, so one cell reaching 10 000× drew a hard chromatic line. Capping
/// at 1/3e-3 ≈ 333× keeps the bright rims and arcs and removes the spikes.
pub const MIN_AREA_FRAC: f32 = 3e-3;

/// Flux-conserving screen-space floor on a quad's size (K-261, refined by
/// K-262 and K-264; mirrored by the WGSL build). Returns false when the
/// quad must be dropped.
///
/// - Area at or above [`MIN_QUAD_PX`]: drawn unchanged — including long
///   thin fold cells, which K-262 dropped. The drop existed because a
///   cell-flat density at the 333× cap painted such a cell as a hard
///   chromatic line; with the K-264 vertex-smoothed density its brightness
///   is its neighbourhood's, and dropping it is what cut the triangular
///   notches out of every caustic rim the owner reported at Ultra.
/// - Below it and COMPACT (longest edge within [`MAX_INFLATE_EDGE_PX`]):
///   inflated about its centroid to the floor, colour scaled by the true ÷
///   inflated area ratio — flux exact, and the rasteriser can no longer
///   drop it for covering no sample.
/// - Below it and STRETCHED (a sub-sample sliver): dropped. Inflating one
///   multiplies its length — the K-261 "random lines" artefact — and
///   un-inflated it covers nothing; its neighbours carry the rim.
pub(crate) fn inflate_quad(v: &mut [FlareVertex; 4]) -> bool {
    let e = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        (a[0] - b[0]) * (c[1] - a[1]) - (a[1] - b[1]) * (c[0] - a[0])
    };
    let a0 = e(v[0].pos, v[1].pos, v[2].pos);
    let a1 = e(v[0].pos, v[2].pos, v[3].pos);
    let area_px = ((a0 + a1) / 2.0).abs();
    if area_px >= MIN_QUAD_PX {
        return true;
    }
    let mut longest = 0.0_f32;
    for i in 0..4 {
        let a = v[i].pos;
        let b = v[(i + 1) % 4].pos;
        longest = longest.max((a[0] - b[0]).hypot(a[1] - b[1]));
    }
    if longest > MAX_INFLATE_EDGE_PX {
        return false;
    }
    let eps = MIN_QUAD_PX * 1e-4;
    let s = (MIN_QUAD_PX / area_px.max(eps)).sqrt();
    let cx = (v[0].pos[0] + v[1].pos[0] + v[2].pos[0] + v[3].pos[0]) / 4.0;
    let cy = (v[0].pos[1] + v[1].pos[1] + v[2].pos[1] + v[3].pos[1]) / 4.0;
    let scale = area_px.max(eps) / MIN_QUAD_PX;
    for c in v.iter_mut() {
        c.pos[0] = cx + (c.pos[0] - cx) * s;
        c.pos[1] = cy + (c.pos[1] - cy) * s;
        for ch in &mut c.rgb {
            *ch *= scale;
        }
    }
    true
}

/// The 4× multisample positions inside a pixel, as offsets from its origin
/// — the STANDARD sample locations every Vulkan/D3D device uses at count 4,
/// which is what makes the CPU twin agree with the hardware resolve rather
/// than merely resemble it (K-264).
pub const MSAA_SAMPLES: [[f32; 2]; 4] = [
    [0.375, 0.125],
    [0.875, 0.375],
    [0.125, 0.625],
    [0.625, 0.875],
];

/// Rasterise one triangle with barycentric colour interpolation into the
/// additive RGB buffer — the CPU twin of the hardware fill, INCLUDING its
/// 4× multisampling (K-264): coverage is tested at the four standard sample
/// positions, the colour is evaluated once at the pixel centre (the
/// hardware's non-centroid interpolation, extrapolated when the centre is
/// outside), and the pixel takes colour × covered/4 — exactly what the
/// multisample resolve of an additively-blended fp16 target produces.
/// Jagged silhouettes were one of the three Ultra artefacts the owner
/// reported; sampling coverage instead of the centre alone is the fix.
fn raster_triangle(out: &mut [f32], w: u32, h: u32, v: [FlareVertex; 3]) {
    // The bounding box widens by one so pixels whose centre is outside but
    // whose samples are covered still get their fraction.
    let min_x = (v[0].pos[0].min(v[1].pos[0]).min(v[2].pos[0]).floor() as i64 - 1).max(0);
    let max_x = (v[0].pos[0].max(v[1].pos[0]).max(v[2].pos[0]).ceil() as i64 + 1).min(w as i64 - 1);
    let min_y = (v[0].pos[1].min(v[1].pos[1]).min(v[2].pos[1]).floor() as i64 - 1).max(0);
    let max_y = (v[0].pos[1].max(v[1].pos[1]).max(v[2].pos[1]).ceil() as i64 + 1).min(h as i64 - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }
    let edge = |a: [f32; 2], b: [f32; 2], px: f32, py: f32| {
        (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])
    };
    let area = edge(v[0].pos, v[1].pos, v[2].pos[0], v[2].pos[1]);
    if area.abs() < 1e-9 {
        return;
    }
    // Edge functions with their constant terms lifted out of the loop
    // (K-263); the sample-coverage test works on their raw signs, so a
    // wholly-outside pixel costs multiplies, never divisions.
    let (e0_dx, e0_dy) = (v[2].pos[0] - v[1].pos[0], v[2].pos[1] - v[1].pos[1]);
    let (e1_dx, e1_dy) = (v[0].pos[0] - v[2].pos[0], v[0].pos[1] - v[2].pos[1]);
    let (e2_dx, e2_dy) = (v[1].pos[0] - v[0].pos[0], v[1].pos[1] - v[0].pos[1]);
    let sign = if area > 0.0 { 1.0 } else { -1.0 };
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let mut covered = 0u32;
            for smp in MSAA_SAMPLES {
                let px = x as f32 + smp[0];
                let py = y as f32 + smp[1];
                let e0 = e0_dx * (py - v[1].pos[1]) - e0_dy * (px - v[1].pos[0]);
                let e1 = e1_dx * (py - v[2].pos[1]) - e1_dy * (px - v[2].pos[0]);
                let e2 = e2_dx * (py - v[0].pos[1]) - e2_dy * (px - v[0].pos[0]);
                if e0 * sign >= 0.0 && e1 * sign >= 0.0 && e2 * sign >= 0.0 {
                    covered += 1;
                }
            }
            if covered == 0 {
                continue;
            }
            // Colour at the pixel CENTRE, unclamped — extrapolated when the
            // centre sits outside the triangle, as the hardware's
            // non-centroid interpolation does.
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let e0 = e0_dx * (py - v[1].pos[1]) - e0_dy * (px - v[1].pos[0]);
            let e1 = e1_dx * (py - v[2].pos[1]) - e1_dy * (px - v[2].pos[0]);
            let w0 = e0 / area;
            let w1 = e1 / area;
            let w2 = 1.0 - w0 - w1;
            let frac = covered as f32 / 4.0;
            let i = ((y as u32 * w + x as u32) * 3) as usize;
            out[i] += frac * (w0 * v[0].rgb[0] + w1 * v[1].rgb[0] + w2 * v[2].rgb[0]);
            out[i + 1] += frac * (w0 * v[0].rgb[1] + w1 * v[1].rgb[1] + w2 * v[2].rgb[1]);
            out[i + 2] += frac * (w0 * v[0].rgb[2] + w1 * v[1].rgb[2] + w2 * v[2].rgb[2]);
        }
    }
}

/// Render the ghost train alone into an RGB flare buffer (`w × h × 3`),
/// mirroring the GPU chain (K-261): a REGULAR grid of rays over the pupil
/// square is traced through each ranked pair per wavelength (the FlareSim
/// optics), and each live grid cell draws as two triangles whose density is
/// the energy-conservation ratio `launch cell area ÷ landed area` — smooth
/// noise-free ghosts, with sub-pixel fold quads inflated so caustic flux
/// survives. The iris mask (blades, roundness, softness) weights each
/// corner, which is what shapes the ghosts. Used by tests and small enough
/// to read as the spec of the GPU path.
pub fn cpu_flare(
    p: &LensFlareParams,
    baked: &FlareBaked,
    w: u32,
    h: u32,
    lights: &[FlareLight],
) -> Vec<f32> {
    // The rendered raster is the PADDED buffer (K-267): larger than the
    // base `w × h` when Squeeze/Scale under 1 will sample past it, with the
    // optics centred in it. The screen transform and the light direction's
    // aspect stay derived from the base dims — padding adds border, it
    // never rescales the image.
    let (rw, rh) = flare_pad_dims(w, h, p.anamorphic, p.scale);
    let mut out = vec![0.0_f32; (rw * rh * 3) as usize];
    if w == 0 || h == 0 || p.ghost_intensity <= 0.0 {
        return out;
    }
    // Area sources are sampled across their extent here, exactly as the GPU
    // does — Manual's list is expanded by the caller that builds the op, and
    // Matte's inside the detection kernel, so the reference must expand too
    // or the two stop being twins (K-355). A list of point sources is
    // returned unchanged, so nothing about a point light moves.
    let lights = &expand_area_lights(lights, AREA_SAMPLES_MAX)[..];
    let (tier_base, tier_lambda, _) = quality_ladder(p.quality);
    // The Detail dial scales the tier's base AND its wavelength count
    // before the per-pair budget (K-265); both the CPU reference and the
    // GPU dispatch derive the same numbers, so the oracle holds at any
    // setting.
    let base_side = detail_base(tier_base, p.detail);
    let lambda_count = detail_lambda(tier_lambda, p.detail);
    let bands = spectral_bands(lambda_count, p.dispersion);
    let roundness = effective_roundness(p.roundness, p.fstop, baked.native_fstop);
    let rot = p.aperture_rotation_deg.to_radians();
    let stop_scale = fstop_scale(baked.native_fstop, p.fstop);
    let sensor_shift = focus_shift_mm(p.focus_m, baked.focal_mm);
    let aspect = h as f32 / w.max(1) as f32;
    let st = screen_transform(w);
    let gain = p.ghost_intensity * baked.energy_gain;
    let pair_count = baked.pairs.len().min(p.max_ghosts as usize);
    // Manual mode's frame-time grid probe (K-267): the bake spreads are a
    // floor, and a pair folded tight by this frame's light earns more grid
    // under the frame's bounded ray headroom. Matte lights exist GPU-side
    // only, so Matte mode keeps the bake grids on both twins and parity
    // holds.
    let frame_grids: Vec<u32> = if p.source != 1 {
        lights
            .first()
            .map(|l| {
                let needs = frame_grid_needs(
                    baked,
                    pair_count,
                    light_direction(l.pos, aspect, baked.focal_mm),
                    p.coating,
                    stop_scale,
                    sensor_shift,
                );
                plan_frame_grids(base_side, &baked.spreads, &needs)
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Per-corner trace results for one (pair, band): landing px, geometric
    // weight (housing feather × iris mask) and band-integrated rgb (K-364);
    // None = dead. Sized for the widest grid in play.
    type Corner = ([f32; 2], f32, [f32; 3]);
    let mut corners: Vec<Option<Corner>> = Vec::new();
    // The pair's iris mask per pupil corner — wavelength-independent, so it
    // is computed once a pair and read by every wavelength (K-263).
    let mut masks: Vec<f32> = Vec::new();
    // Landed area per grid cell for the (pair, λ) being drawn; 0 = dead.
    let mut areas: Vec<f32> = Vec::new();
    for light in lights {
        if light.rgb[0] <= 0.0 && light.rgb[1] <= 0.0 && light.rgb[2] <= 0.0 {
            continue;
        }
        let dir = light_direction(light.pos, aspect, baked.focal_mm);
        for (pi, pair) in baked.pairs.iter().take(pair_count).enumerate() {
            // The pair's own grid (K-262), by its measured image spread,
            // raised — never lowered — by this frame's budgeted probe
            // (K-267).
            let rung = pair_grid(base_side, baked.spreads.get(pi).copied().unwrap_or(1.0));
            let side = frame_grids.get(pi).copied().unwrap_or(rung).max(2) as usize;
            let unit = |i: usize| (i as f32 / (side - 1) as f32) * 2.0 - 1.0;
            let cell_mm = 2.0 * baked.pupil_mm * stop_scale / (side - 1) as f32;
            let cell_area_px = cell_mm * cell_mm * st * st;
            corners.clear();
            corners.resize(side * side, None);
            // The iris mask per pupil corner, computed ONCE for the pair
            // (K-263). It does not depend on the wavelength — it is the shape
            // of the hole the light comes through — but it used to be
            // recomputed inside the wavelength loop, so every corner paid for
            // an `atan2` and a `cos` three to thirty-two times over depending
            // on the Quality tier. Same numbers, computed once.
            masks.clear();
            masks.resize(side * side, 0.0);
            for j in 0..side {
                for i in 0..side {
                    masks[j * side + i] = pupil_mask(
                        unit(i),
                        unit(j),
                        p.blades,
                        rot,
                        roundness,
                        p.aperture_softness,
                    );
                }
            }
            for band in &bands {
                // The corner traces in parallel — this is where the bake's
                // time actually went (the auto-exposure thumbnail traces
                // tens of thousands of rays through the whole prescription,
                // one core at a time). Each corner is independent and the
                // results land at their own indices, so the splat below
                // reads exactly the numbers the serial loop produced, in
                // exactly the order it read them — the output is
                // bit-identical, only sooner (docs/14 determinism).
                use rayon::prelude::*;
                corners
                    .par_iter_mut()
                    .enumerate()
                    .for_each(|(idx, corner)| {
                        let (j, i) = (idx / side, idx % side);
                        let (u, v) = (unit(i), unit(j));
                        // …unless it is so far outside the iris that no
                        // cell touching it can hold ANY lit corner: the
                        // mask is zero beyond radius 1, so corners past
                        // one cell-diagonal beyond that never share a cell
                        // with light, and tracing them would spend a fifth
                        // of the frame's rays on the pupil square's dark
                        // corners for bit-identical output.
                        let spacing = 2.0 / (side - 1) as f32;
                        if u * u + v * v > (1.0 + 1.5 * spacing).powi(2) {
                            *corner = None;
                            return;
                        }
                        // A masked-out corner still TRACES (K-264): its
                        // weight is zero but its geometry is real, so the
                        // cell it belongs to draws and the iris edge fades
                        // INSIDE the cell. Killing the ray instead killed
                        // the whole cell, and every iris-shaped ghost edge
                        // was quantised to the pupil grid.
                        let mask = masks[idx];
                        let origin = [
                            u * baked.pupil_mm * stop_scale,
                            v * baked.pupil_mm * stop_scale,
                            baked.start_z_mm,
                        ];
                        *corner = trace_splat_spectral(
                            baked,
                            *pair,
                            band,
                            origin,
                            dir,
                            p.coating,
                            stop_scale,
                            sensor_shift,
                        )
                        .map(|(pos, wt, rgb)| {
                            (
                                [pos[0] * st + rw as f32 / 2.0, rh as f32 / 2.0 - pos[1] * st],
                                wt * mask,
                                rgb,
                            )
                        });
                    });
                // Landed area per grid cell, 0 = dead (any corner dead) —
                // the input to the K-264 vertex-smoothed density below.
                let qs = side - 1;
                areas.clear();
                areas.resize(qs * qs, 0.0);
                let e = |a: [f32; 2], b: [f32; 2], q: [f32; 2]| {
                    (a[0] - b[0]) * (q[1] - a[1]) - (a[1] - b[1]) * (q[0] - a[0])
                };
                for j in 0..qs {
                    for i in 0..qs {
                        let c = [
                            corners[j * side + i],
                            corners[j * side + i + 1],
                            corners[(j + 1) * side + i + 1],
                            corners[(j + 1) * side + i],
                        ];
                        if let [Some(c0), Some(c1), Some(c2), Some(c3)] = c {
                            let a0 = e(c0.0, c1.0, c2.0);
                            let a1 = e(c0.0, c2.0, c3.0);
                            areas[j * qs + i] = ((a0 + a1) / 2.0).abs();
                        }
                    }
                }
                // Density at a GRID CORNER: launch cell area over the mean
                // of the live cells that touch the corner (K-264, the
                // [Hullin 2011] rule — each vertex carries the average area
                // of its surrounding quads). A per-CELL density is constant
                // across the cell and jumps at its edge, which is exactly
                // the faceting the owner saw at Ultra; averaged to the
                // corners and interpolated by the raster it is continuous
                // across the whole ghost. The [`MIN_AREA_FRAC`] floor on
                // the mean is still the caustic density cap.
                let corner_mean_area = |ci: usize, cj: usize| -> f32 {
                    let (mut sum, mut n) = (0.0_f32, 0u32);
                    for qj in cj.saturating_sub(1)..=cj.min(qs - 1) {
                        for qi in ci.saturating_sub(1)..=ci.min(qs - 1) {
                            let a = areas[qj * qs + qi];
                            if a > 0.0 {
                                sum += a;
                                n += 1;
                            }
                        }
                    }
                    if n > 0 {
                        sum / n as f32
                    } else {
                        0.0
                    }
                };
                let corner_density = |ci: usize, cj: usize| -> f32 {
                    cell_area_px / corner_mean_area(ci, cj).max(MIN_AREA_FRAC * cell_area_px)
                };
                // A corner's weight for COLOUR: the mean over its 3×3 ray
                // neighbourhood, dead rays as zero (K-266) — the WGSL
                // `smooth_weight` twin. The raw per-ray weight cliffs (a
                // housing feather compressed into less than a cell, a
                // vignette cut) land inside one cell and drew every wash
                // ghost's edge as chunky facets; smoothed one lattice step,
                // a cliff becomes a two-cell ramp. Geometry decisions (lit
                // corners, the pull-in) stay on RAW weights: smearing light
                // onto a virtual continuation's far-flung corner would draw
                // the K-264 fan lines again.
                // Since K-364 the smooth carries the corner's spectral rgb
                // through the same 3×3 mean the scalar weight took — with a
                // constant rgb this is exactly the old smooth × that rgb, so
                // nothing about the K-266 cliff-smoothing changed shape.
                let smooth_rgb = |ci: usize, cj: usize| -> [f32; 3] {
                    let mut sum = [0.0_f32; 3];
                    for dj in -1i64..=1 {
                        for di in -1i64..=1 {
                            let x = (ci as i64 + di).clamp(0, side as i64 - 1) as usize;
                            let y = (cj as i64 + dj).clamp(0, side as i64 - 1) as usize;
                            if let Some((_, wt, rgb)) = corners[y * side + x] {
                                let w = wt.max(0.0);
                                sum[0] += w * rgb[0];
                                sum[1] += w * rgb[1];
                                sum[2] += w * rgb[2];
                            }
                        }
                    }
                    [sum[0] / 9.0, sum[1] / 9.0, sum[2] / 9.0]
                };
                // The SMALLEST live cell touching a corner — the pull-in's
                // length scale (K-265). The mean is wrong for that job: at
                // a fold the stretched cells inflate it, so pulled corners
                // stuck out by the fold's size instead of the local cell's,
                // and every hard ghost edge grew a grid-independent
                // sawtooth corona (the owner's EF 70-200).
                let corner_min_area = |ci: usize, cj: usize| -> f32 {
                    let mut min = f32::MAX;
                    for qj in cj.saturating_sub(1)..=cj.min(qs - 1) {
                        for qi in ci.saturating_sub(1)..=ci.min(qs - 1) {
                            let a = areas[qj * qs + qi];
                            if a > 0.0 && a < min {
                                min = a;
                            }
                        }
                    }
                    if min == f32::MAX {
                        0.0
                    } else {
                        min
                    }
                };
                for j in 0..qs {
                    for i in 0..qs {
                        if areas[j * qs + i] <= 0.0 {
                            continue;
                        }
                        let c = [
                            corners[j * side + i],
                            corners[j * side + i + 1],
                            corners[(j + 1) * side + i + 1],
                            corners[(j + 1) * side + i],
                        ];
                        let [Some(c0), Some(c1), Some(c2), Some(c3)] = c else {
                            continue;
                        };
                        let density = [
                            corner_density(i, j),
                            corner_density(i + 1, j),
                            corner_density(i + 1, j + 1),
                            corner_density(i, j + 1),
                        ];
                        let mut v: [FlareVertex; 4] = [
                            FlareVertex {
                                pos: c0.0,
                                rgb: [0.0; 3],
                            },
                            FlareVertex {
                                pos: c1.0,
                                rgb: [0.0; 3],
                            },
                            FlareVertex {
                                pos: c2.0,
                                rgb: [0.0; 3],
                            },
                            FlareVertex {
                                pos: c3.0,
                                rgb: [0.0; 3],
                            },
                        ];
                        let smoothed = [
                            smooth_rgb(i, j),
                            smooth_rgb(i + 1, j),
                            smooth_rgb(i + 1, j + 1),
                            smooth_rgb(i, j + 1),
                        ];
                        for ((vert, srgb), d) in v.iter_mut().zip(smoothed).zip(density) {
                            let b = d * gain;
                            vert.rgb = [
                                b * srgb[0] * light.rgb[0],
                                b * srgb[1] * light.rgb[1],
                                b * srgb[2] * light.rgb[2],
                            ];
                        }
                        // Rein in the unlit corners (K-264). A cell that
                        // spans from lit geometry to a mount-absorbed
                        // virtual continuation can be hundreds of px long;
                        // drawn it fans a faint line out of the ghost's
                        // bore, dropped it notches the bore's edge (both
                        // shipped, both reported). The zero-weight corner
                        // carries no light — its only job is geometry — so
                        // it is PULLED toward the lit corners' centroid to
                        // within a few cell-widths, and the fade to zero
                        // lands where the boundary is, smoothly.
                        let corners4 = [c0, c1, c2, c3];
                        let lit: Vec<usize> = (0..4).filter(|&k| corners4[k].1 > 0.0).collect();
                        if lit.is_empty() {
                            // No light anywhere in the cell: draw nothing
                            // rather than rasterise an invisible quad that
                            // may span the frame.
                            continue;
                        }
                        if lit.len() < 4 {
                            let mut cx = 0.0_f32;
                            let mut cy = 0.0_f32;
                            for &k in &lit {
                                cx += corners4[k].0[0];
                                cy += corners4[k].0[1];
                            }
                            cx /= lit.len() as f32;
                            cy /= lit.len() as f32;
                            // Smallest lit neighbour sets the scale: the
                            // true boundary lies within one cell of the
                            // last lit corner, so one COMPACT cell-width
                            // is the reach — the mean let fold-stretched
                            // neighbours grow it into sawteeth (K-265).
                            let mut min_area = f32::MAX;
                            for &k in &lit {
                                let (ci, cj) = match k {
                                    0 => (i, j),
                                    1 => (i + 1, j),
                                    2 => (i + 1, j + 1),
                                    _ => (i, j + 1),
                                };
                                let m = corner_min_area(ci, cj);
                                if m > 0.0 {
                                    min_area = min_area.min(m);
                                }
                            }
                            let reach = min_area.min(1e12).sqrt().max(1.0);
                            for (k, vert) in v.iter_mut().enumerate() {
                                if corners4[k].1 > 0.0 {
                                    continue;
                                }
                                let dx = vert.pos[0] - cx;
                                let dy = vert.pos[1] - cy;
                                let d = dx.hypot(dy);
                                if d > reach {
                                    let f = reach / d;
                                    vert.pos = [cx + dx * f, cy + dy * f];
                                }
                            }
                        }
                        if !inflate_quad(&mut v) {
                            continue;
                        }
                        raster_triangle(&mut out, rw, rh, [v[0], v[1], v[2]]);
                        raster_triangle(&mut out, rw, rh, [v[0], v[2], v[3]]);
                    }
                }
            }
        }
    }
    // Blur radius from the BASE dims (the look must not change with the
    // padding), applied over the padded raster.
    blur_flare(
        &mut out,
        rw,
        rh,
        ghost_blur_radius(p.ghost_softness, w, h),
        3,
    );
    out
}

/// Separable box blur over an RGB buffer, `passes` times (3 passes
/// approximate a Gaussian) — FlareSim's Ghost Blur (K-261), shared by the
/// CPU reference and mirrored by the WGSL blur kernel. `radius_px` 0 is a
/// no-op.
pub(crate) fn blur_flare(buf: &mut [f32], w: u32, h: u32, radius_px: u32, passes: u32) {
    if radius_px == 0 || w == 0 || h == 0 {
        return;
    }
    let (w, h, r) = (w as usize, h as usize, radius_px as usize);
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut tmp = vec![0.0_f32; buf.len()];
    for _ in 0..passes.max(1) {
        // Horizontal into tmp.
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0_f32; 3];
                for dx in -(r as i64)..=(r as i64) {
                    let sx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                    let i = (y * w + sx) * 3;
                    acc[0] += buf[i];
                    acc[1] += buf[i + 1];
                    acc[2] += buf[i + 2];
                }
                let o = (y * w + x) * 3;
                tmp[o] = acc[0] * norm;
                tmp[o + 1] = acc[1] * norm;
                tmp[o + 2] = acc[2] * norm;
            }
        }
        // Vertical back into buf.
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0_f32; 3];
                for dy in -(r as i64)..=(r as i64) {
                    let sy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
                    let i = (sy * w + x) * 3;
                    acc[0] += tmp[i];
                    acc[1] += tmp[i + 1];
                    acc[2] += tmp[i + 2];
                }
                let o = (y * w + x) * 3;
                buf[o] = acc[0] * norm;
                buf[o + 1] = acc[1] * norm;
                buf[o + 2] = acc[2] * norm;
            }
        }
    }
}

/// The Ghost-softness blur radius in pixels for a buffer size: the dial is
/// a percentage of the frame diagonal (0.3 ≈ FlareSim's suggested 0.003).
///
/// Capped at [`MAX_BLUR_RADIUS_PX`] (K-262): the box blur is a naive
/// `2r+1`-tap loop run six times, so an uncapped 2% radius on a 4K frame
/// submits ~1000 taps per pixel — firmly in GPU-timeout territory. Three
/// passes of an 80 px box already read as a heavy defocus, so the cap
/// costs nothing anyone can see.
pub fn ghost_blur_radius(softness: f32, w: u32, h: u32) -> u32 {
    let diag = ((w * w + h * h) as f32).sqrt();
    ((softness.clamp(0.0, 2.0) * 0.01 * diag).round() as u32).min(MAX_BLUR_RADIUS_PX)
}

/// See [`ghost_blur_radius`].
pub const MAX_BLUR_RADIUS_PX: u32 = 80;

/// The combine stage, mirrored by the WGSL combine kernel: `out = orig +
/// intensity · (flare(scaled · squeezed) + starbursts)`, alpha saturating
/// toward 1, Mix lerping against the untouched input. The Scale parameter
/// scales the WHOLE flare about the optical centre — the ghost buffer is
/// sampled through it, and each light's starburst sprite grows by it while
/// staying anchored on its light. `flare` is the ghost buffer at `fw × fh`
/// (Draft renders it at half size; sampling is resolution-relative so both
/// agree). Operates on the premultiplied working buffer in place.
#[allow(clippy::too_many_arguments)]
pub fn cpu_combine(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    p: &LensFlareParams,
    baked: &FlareBaked,
    flare: &[f32],
    fw: u32,
    fh: u32,
    lights: &[FlareLight],
) {
    if p.intensity <= 0.0 || p.mix <= 0.0 {
        return;
    }
    // The same expansion `cpu_flare` makes: the starburst is stamped per
    // light, so an area source stamps one per sample at its share of the
    // flux, which is what the GPU's combine does over its filled slots.
    let lights = &expand_area_lights(lights, AREA_SAMPLES_MAX)[..];
    let squeeze = p.anamorphic.clamp(0.25, 4.0);
    let fscale = p.scale.clamp(0.05, 20.0);
    let sb_res = STARBURST_RES as usize;
    let sb_half = 0.6 * fscale * w.min(h) as f32;
    // The flare buffer arrives PADDED (K-267): `cpu_flare` renders it at
    // `flare_pad_dims`, geometry centred, so the resolution-relative tap
    // only shifts by the border. Unpadded, the constant is zero and the
    // maths is K-266's exactly.
    let (rw, rh) = flare_pad_dims(fw, fh, p.anamorphic, p.scale);
    let sample_flare = |x: f32, y: f32| -> [f32; 3] {
        // Resolution-relative bilinear tap of the flare buffer. OUTSIDE it
        // there is no flare (K-266): a squeeze or scale below 1 asks for
        // coordinates past the buffer, and clamp-addressing repeated the
        // edge row outward — the owner's "dreadful" anamorphic smear. Half
        // a texel of grace keeps the true border texels filtered.
        let u = (x / w as f32) * fw as f32 - 0.5 + (rw - fw) as f32 / 2.0;
        let v = (y / h as f32) * fh as f32 - 0.5 + (rh - fh) as f32 / 2.0;
        if u < -0.5 || v < -0.5 || u > rw as f32 - 0.5 || v > rh as f32 - 0.5 {
            return [0.0; 3];
        }
        let x0 = u.floor().max(0.0) as usize;
        let y0 = v.floor().max(0.0) as usize;
        let x1 = (x0 + 1).min(rw as usize - 1);
        let y1 = (y0 + 1).min(rh as usize - 1);
        let x0 = x0.min(rw as usize - 1);
        let y0 = y0.min(rh as usize - 1);
        let (tx, ty) = (
            (u - u.floor()).clamp(0.0, 1.0),
            (v - v.floor()).clamp(0.0, 1.0),
        );
        let mut rgb = [0.0_f32; 3];
        for (c, out_c) in rgb.iter_mut().enumerate() {
            let a = flare[(y0 * rw as usize + x0) * 3 + c] * (1.0 - tx)
                + flare[(y0 * rw as usize + x1) * 3 + c] * tx;
            let b = flare[(y1 * rw as usize + x0) * 3 + c] * (1.0 - tx)
                + flare[(y1 * rw as usize + x1) * 3 + c] * tx;
            *out_c = a * (1.0 - ty) + b * ty;
        }
        rgb
    };
    for y in 0..h {
        for x in 0..w {
            // Whole-flare scale plus the anamorphic squeeze (x only), both
            // about the frame centre.
            let cx = w as f32 / 2.0;
            let cyc = h as f32 / 2.0;
            let sx = cx + (x as f32 + 0.5 - cx) / (squeeze * fscale);
            let sy = cyc + (y as f32 + 0.5 - cyc) / fscale;
            let f = sample_flare(sx, sy);
            // One starburst sprite per live light, anchored on the light,
            // sized by Scale, stretched by the squeeze, tinted by the light.
            let mut sb = [0.0_f32; 3];
            if p.starburst_intensity > 0.0 && sb_half > 0.0 {
                for light in lights {
                    if light.rgb[0] <= 0.0 && light.rgb[1] <= 0.0 && light.rgb[2] <= 0.0 {
                        continue;
                    }
                    let light_px = [light.pos[0] * w as f32, light.pos[1] * h as f32];
                    let rel_x = x as f32 + 0.5 - light_px[0];
                    let rel_y = y as f32 + 0.5 - light_px[1];
                    let u = rel_x / (sb_half * squeeze) * 0.5 + 0.5;
                    let v = rel_y / sb_half * 0.5 + 0.5;
                    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                        continue;
                    }
                    let fx = u * (sb_res - 1) as f32;
                    let fy = v * (sb_res - 1) as f32;
                    let x0 = fx.floor() as usize;
                    let y0 = fy.floor() as usize;
                    let x1 = (x0 + 1).min(sb_res - 1);
                    let y1 = (y0 + 1).min(sb_res - 1);
                    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                    for (c, out_c) in sb.iter_mut().enumerate() {
                        let a = baked.starburst[(y0 * sb_res + x0) * 3 + c] * (1.0 - tx)
                            + baked.starburst[(y0 * sb_res + x1) * 3 + c] * tx;
                        let b = baked.starburst[(y1 * sb_res + x0) * 3 + c] * (1.0 - tx)
                            + baked.starburst[(y1 * sb_res + x1) * 3 + c] * tx;
                        *out_c += (a * (1.0 - ty) + b * ty) * p.starburst_intensity * light.rgb[c];
                    }
                }
            }
            let add = [
                (f[0] + sb[0]) * p.intensity,
                (f[1] + sb[1]) * p.intensity,
                (f[2] + sb[2]) * p.intensity,
            ];
            let i = ((y * w + x) * 4) as usize;
            let o = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
            let luma = super::cpu::LUMA[0] * add[0]
                + super::cpu::LUMA[1] * add[1]
                + super::cpu::LUMA[2] * add[2];
            // The flare element (K-289): the light this frame drew, with the
            // coverage that light implies as its alpha — a premultiplied
            // black-backed overlay. Blend it with the layer, then saturate
            // alpha at 1. Add reduces to `o + add` with alpha `min(o.a +
            // luma, 1)`, which is exactly what the effect did before the
            // menu existed. Only reached while live: the Intensity-0/Mix-0
            // early return above keeps the passthroughs bit-exact.
            let e = [add[0], add[1], add[2], luma];
            let mut flared = flare_blend(p.blend, o, e);
            flared[3] = flared[3].min(1.0);
            rgba[i] = o[0] * (1.0 - p.mix) + flared[0] * p.mix;
            rgba[i + 1] = o[1] * (1.0 - p.mix) + flared[1] * p.mix;
            rgba[i + 2] = o[2] * (1.0 - p.mix) + flared[2] * p.mix;
            rgba[i + 3] = o[3] * (1.0 - p.mix) + flared[3] * p.mix;
        }
    }
}
