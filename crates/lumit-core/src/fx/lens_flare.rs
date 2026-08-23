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
    /// neutral ghosts) toward each surface's anti-reflective coating
    /// (K-261: per-surface layer counts from the lens file; K-371: or the
    /// palette entry that element was set to).
    pub coating: f32,
    /// The coating on each glass element, by [`coating_design`] palette index
    /// (K-371); [`COATING_AS_FILE`] leaves the prescription's own column
    /// alone, which is what every element defaults to.
    ///
    /// Element 0 is the front piece of glass. Entries past the lens's own
    /// element count are ignored, and a lens with more elements than
    /// [`MAX_COATING_ELEMENTS`] keeps its file's coating on the rest.
    pub coating_elements: [u32; MAX_COATING_ELEMENTS],
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
    /// whose emitting rectangle every ray integrates a different point of
    /// (K-367), so its ghosts take the shape of the source rather than of a
    /// point and cost no extra rays. Matte mode measures this
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

/// The iris radius as a fraction of the aperture image's half-extent: the
/// polygon sits at 0.75, leaving rim room for the diffraction spread. It is
/// also the conversion between the aperture image's own frame and the
/// **pupil** coordinates [`pupil_mask`] takes — pupil `u = 1` is aperture
/// ndc 0.75.
pub const APERTURE_SIZE: f32 = 0.75;

/// The wavelength the ghost-edge ringing is scaled at, µm (K-370).
///
/// The rim fringes are a single achromatic profile rather than one per
/// wavelength: their spacing goes as `√λ`, so across the visible band it
/// varies by ±15% about the middle — far less than the blur that
/// [`ghost_mask`] already averages them under, and not worth six times the
/// arithmetic per ray.
pub const RING_LAMBDA_UM: f32 = 0.55;

/// Field-angle slices the starburst is baked at (K-365).
///
/// Off-axis the diffracting hole is not the iris alone: the front and rear
/// mechanical stops clip it into a **cat's-eye**, so the starburst squashes
/// and leans as the light moves out towards the frame corner. The bake
/// therefore renders the aperture at `STARBURST_FIELDS` field angles —
/// slice 0 on-axis (the picture the effect drew before K-365), slice
/// `STARBURST_FIELDS − 1` at the sensor-corner field angle — and the
/// combine blends the two slices bracketing each light's own field
/// fraction. Eight is the point where doubling the count stops changing the
/// blended sprite (the vignette varies smoothly in the field) while the
/// bake still parallelises one slice per core.
pub const STARBURST_FIELDS: usize = 8;

/// Distinct sources a frame detects (Matte mode's top-K cap; Manual is one),
/// and — since K-367 — the light slots the trace carries, because the two are
/// again the same number: one source is one light, whatever its size.
/// 16 since K-267: area sources anchor on one slot each, and eight anchors
/// starved scenes with several practicals plus an area source.
///
/// K-355 briefly split this into a separate `MAX_LIGHTS` of 64, because an
/// area source was then rendered by REPLICATING itself into up to 5×5 point
/// lights. K-367 integrates the source inside the ray loop instead, so the
/// replication — and the four-times-larger slot table that existed only to
/// hold it — is gone.
pub const MAX_SOURCES: usize = 16;

/// Detection tile side, raster pixels (impl note §6).
pub const DETECT_TILE: u32 = 32;
/// Non-max suppression radius, in tiles (Chebyshev): one highlight must not
/// spend the whole light budget on its own neighbouring tiles.
pub const SUPPRESS_TILES: i64 = 2;

/// Ghost pairs dimmer than this on the on-axis probe are dropped at bake
/// (FlareSim's `min_intensity`).
pub const PAIR_MIN_INTENSITY: f32 = 1e-7;

/// The empty bounce slot (K-368): a ghost path is `[a, b, c, d]` and a
/// two-bounce path — every ghost the effect traced before K-368 — is
/// `[a, b, NO_BOUNCE, NO_BOUNCE]`.
///
/// **The path model.** Slot 0 and slot 1 keep exactly the meaning K-261 gave
/// them: the ray runs forward to `b` and reflects there, back to `a` and
/// reflects there, then forward to the sensor, so `a < b` always. A
/// four-bounce path adds the same figure once more: forward from `a` to `c`
/// and reflect, back to `d` and reflect, then forward to the sensor. Hence
/// `a < c` (the third bounce is past the second) and `d < c`, while `c` may
/// sit anywhere relative to `b` and `d` may be `a` itself.
pub const NO_BOUNCE: u32 = u32::MAX;

/// Four-bounce candidates the bake ray-probes (K-368).
///
/// There are about N⁴/4 four-bounce paths on an N-interface prescription —
/// a hundred thousand on a normal lens, a couple of million on a zoom — and
/// probing them all would cost more than the whole rest of the bake. They
/// are ranked instead by a cheap upper bound (the product of the four
/// surfaces' normal-incidence reflectances, which no angle can exceed) and
/// only the best this many are probed. The bound decides only what is
/// *probed*; what survives to render is still decided by the same on-axis
/// ray probe every two-bounce pair faces.
pub const FOUR_BOUNCE_PROBE_CAP: usize = 1500;

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
    /// larger is an AREA source, whose emitting rectangle each ray integrates
    /// a different point of — see [`source_jitter`] (K-367).
    pub extent: [f32; 2],
}

/// Irrational rotations for the two source-integration axes (K-367,
/// re-chosen K-378), driven by the ray's own pupil-grid indices.
///
/// **Two different irrationals, one per axis.** A single constant would put
/// every ray's offset on one diagonal of the source rectangle, sampling a
/// line rather than an area; an irrational RATIO between the two makes the
/// (u, v) pairs cover the rectangle evenly and stay uncorrelated however many
/// rays the grid has.
///
/// **Each must also be a good rotation ALONE** (K-378), because each drives
/// its own axis by its own index. K-367 took the plastic constant's 2D pair
/// (1/ρ, 1/ρ²), whose second number is what a low-discrepancy POINT SET
/// wants — but 1/ρ² = 0.5698 is within 0.002 of 4/7, so as a 1D rotation it
/// lays its samples into seven combs that drift too slowly to wash out
/// across a pupil grid, and every area source wore them as stripes. The
/// replacement is the supergolden ratio's reciprocal (1/ψ, ψ³ = ψ² + 1) —
/// the same family of cubic Pisot units as the plastic constant, rationally
/// independent of it, and measured cleanest of a scanned battery on the
/// stripe metric the regression test now pins.
///
/// Written at the digits an `f32` can actually hold, rounded by hand rather
/// than by the compiler, because the shader's copy of each literal is
/// compared against this one bit for bit by test.
pub const PHI_U: f32 = 0.754_877_7;
/// See [`PHI_U`].
pub const PHI_V: f32 = 0.682_327_8;

/// Phase step between wavelength bands in the source rectangle (K-378): band
/// `b` samples the source shifted by `b` golden-ratio turns. Bands trace and
/// splat independently and their pictures sum, so giving each its own phase
/// multiplies the effective source sampling by the band count for free and
/// averages each band's residual reconstruction ripple toward the mean —
/// which is what buried what remained of the K-367 stripes. A point source
/// shifts a zero extent by any phase and stays exactly zero.
pub const PHI_BAND: f32 = 0.618_034;

/// A triangle wave of `x`, uniform on [−1, 1] — `2·|2·(fract(x) − ½)| − 1`.
///
/// **Why a triangle wave rather than plain `fract`.** `fract` is the usual
/// low-discrepancy trick, but it JUMPS the whole range each time it wraps,
/// and two rays adjacent in the pupil grid can land either side of a wrap.
/// K-366's splat footprints are central differences over exactly those
/// neighbours, so a jump there inflates one splat by the entire width of the
/// source and stamps a bright bar across the ghost. A triangle wave is
/// continuous at every wrap, still uniform, and just as deterministic.
fn tri(x: f32) -> f32 {
    2.0 * (2.0 * (x - x.floor() - 0.5)).abs() - 1.0
}

/// Where in its source's emitting rectangle the ray at pupil-grid `(i, j)`,
/// tracing band `band`, takes its light from (K-367, K-378), as an offset
/// from the source centre in the same raster fractions [`FlareLight::pos`]
/// uses.
///
/// This is what replaced K-355's replication. An area source used to be split
/// into up to 5×5 point lights and the whole ray pipeline run once per
/// sample: 25× the rays, and — wherever a ghost was smaller than the sample
/// spacing — N visibly separate copies of the aperture instead of one smeared
/// shape. Now the source integral is absorbed into the pupil quadrature the
/// trace already performs: same ray count as a point source, and the
/// per-ray footprints inflate by the local source-to-sensor stretch, which is
/// precisely what fills the gaps between neighbouring samples. Replicas
/// cannot form, because no two rays share a source position.
///
/// The offsets hop by more than the whole source between neighbouring rays —
/// that is what equidistributes them — so the reconstruction leans on two
/// K-378 properties: [`ray_axes`] covers the WIDER of each ray's two gaps,
/// and each band re-samples the source at its own [`PHI_BAND`] phase so the
/// bands' summed ripple averages out.
///
/// A zero extent gives a zero offset for every `(i, j)` at every band, so a
/// point source is bit-identical to what it rendered before this existed.
pub(crate) fn source_jitter(i: usize, j: usize, band: usize, extent: [f32; 2]) -> [f32; 2] {
    let p = band as f32 * PHI_BAND;
    [
        tri((i as f32 + 0.5) * PHI_U + p) * extent[0],
        tri((j as f32 + 0.5) * PHI_V + p) * extent[1],
    ]
}

/// The fraction of the raster below which a source's starburst is a single
/// stamp — that is, below which the source is a point of light (K-367).
pub const SB_MIN_EXTENT: f32 = 0.004;
/// Starburst stamps per axis across a source wider than [`SB_MIN_EXTENT`].
pub const SB_STAMPS: u32 = 3;

/// How many starburst stamps a source spends on each axis (K-367).
///
/// The ghosts integrate the source per ray, but the **starburst** cannot: it
/// is a baked sprite, not a traced path. It is also *shift-invariant* — the
/// diffraction pattern of a hole does not change shape as the source moves,
/// only where it is centred — so the starburst of an extended source is
/// exactly the point starburst convolved with the source. This is that
/// convolution in quadrature form: a fixed 3×3 grid spanning ±extent, each
/// stamp carrying `1/(nx·ny)` of the light, which is what K-355's per-sample
/// stamping did as a side effect and what the per-ray integration would
/// otherwise have thrown away. A softbox's spike smears across the softbox;
/// a point's does not move, and stamps once at full strength — bit-identical
/// to a single stamp, which a test pins.
///
/// Three per axis, not more: the sprite is a broad soft star tens of pixels
/// across, so three taps already overlap heavily over any source a frame can
/// hold, and the shader pays this per pixel.
pub(crate) fn starburst_stamp_grid(extent: [f32; 2]) -> (u32, u32) {
    let n = |e: f32| if e >= SB_MIN_EXTENT { SB_STAMPS } else { 1 };
    (n(extent[0]), n(extent[1]))
}

/// Where stamp `i` of `n` sits across a source axis, in units of its
/// half-extent: −1 … +1, and 0 when there is only one.
pub(crate) fn starburst_stamp_offset(i: u32, n: u32) -> f32 {
    if n <= 1 {
        return 0.0;
    }
    i as f32 / (n - 1) as f32 * 2.0 - 1.0
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
/// exactly the per-ray integration K-367 gave detected sources.
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
/// the lowest linear index), then pick the top [`MAX_SOURCES`] cells by luma
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
            // zero and every ray takes it from the same place; a practical
            // measures its real width and each ray integrates a different
            // point of it (see [`source_jitter`]).
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
    /// AR coating layer count from the `.lens` file's own column (as f32 for
    /// the POD mirror).
    pub coating_layers: f32,
    /// 1.0 on the aperture-stop surface, else 0.0 (the f-stop scales it).
    pub is_stop: f32,
    /// Which [`coating_design`] this surface is coated with (K-371), as f32
    /// for the POD mirror: [`COATING_AS_FILE`] means "whatever the
    /// prescription's own column says", which is what every surface held
    /// before per-element coatings existed.
    ///
    /// It occupies the slot that used to be padding, so the WGSL mirror's
    /// stride is unchanged — and the shader still ignores it, because the
    /// design is already resolved into the baked reflectance table by the
    /// time any ray is traced.
    pub coating_design: f32,
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
    /// Ranked ghost paths, brightest first; the frame renders the first
    /// `max_ghosts`. Two-bounce and four-bounce paths share one list and one
    /// ranking (K-368) — see [`NO_BOUNCE`] for the layout.
    pub pairs: Vec<[u32; 4]>,
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
    /// The starburst sprite at [`STARBURST_FIELDS`] field angles (K-365),
    /// slice-major: `STARBURST_FIELDS × STARBURST_RES² × 3` floats, slice
    /// 0 on-axis and slice `STARBURST_FIELDS − 1` at the sensor corner,
    /// each peak-normalised. One azimuth only (the cat's-eye leans along
    /// +x); the combine rotates the sprite to the light's own azimuth.
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
            coating_design: COATING_AS_FILE as f32,
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

/// The palette entry meaning "leave this surface's coating as the `.lens`
/// prescription describes it" (K-371) — the default, and what every surface
/// held before per-element coatings existed.
pub const COATING_AS_FILE: u32 = 0;

/// How many entries [`coating_design`] offers, including [`COATING_AS_FILE`].
pub const COATING_DESIGNS: u32 = 7;

/// Most glass elements a lens may have its coatings set on individually
/// (K-371). The bundled library runs 4 to 18; a user `.lens` file with more
/// elements than this keeps its own coating column on the rest, which is the
/// same fallback an unset row takes.
pub const MAX_COATING_ELEMENTS: usize = 20;

/// One coating design: the layer stack, outermost first, and the wavelength
/// its quarter waves are cut at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoatingDesign {
    /// `(refractive index, optical thickness in quarter waves)` per layer,
    /// outermost first; a zero index ends the stack.
    pub stack: [(f32, f32); MAX_COATING_LAYERS],
    /// The wavelength the quarter waves are cut at, nm.
    pub design_nm: f32,
}

/// The coating on one glass element (K-371), by palette index.
///
/// # In plain terms
///
/// Look at a real flare and the ghosts are not one colour: there will be a
/// blue one, a purple one, a green one and an amber one in the same train.
/// That is not stylisation — it is the lens. A coated surface reflects
/// whatever its coating fails to suppress, and manufacturers cut different
/// elements for different parts of the spectrum, so **each element's residual
/// reflection has its own colour**. This palette is that choice, per element.
///
/// # Why these particular designs
///
/// An AR coating is a stack of quarter waves cut at some wavelength. The
/// reflectance minimum sits there, so what the surface *does* reflect is the
/// complement — and shifting the design wavelength is the honest way to get a
/// differently coloured ghost, because it is what real coating runs differ in.
/// Every entry here is a textbook design of its order (K-356: real recipes are
/// manufacturer secrets and can only be measured), chosen by **measuring** the
/// residual across 420–680 nm at normal incidence and keeping the ones that
/// are both distinctly coloured and, like a real coating, dimmer than bare
/// glass at every wavelength. Their measured band split (red/green/blue share
/// of the reflected energy, bare crown glass being 0.04 flat) is:
///
/// | # | design | peak R | r / g / b |
/// |---|---|---|---|
/// | 1 | uncoated | 0.040 | flat |
/// | 2 | MgF₂ quarter, 520 nm | 0.018 | 0.38 / 0.35 / 0.27 |
/// | 3 | MgF₂ + Al₂O₃ quarters, 520 nm | 0.019 | 0.55 / 0.12 / 0.32 |
/// | 4 | broadband W, 520 nm | 0.004 | 0.37 / 0.38 / 0.25 |
/// | 5 | broadband W, 480 nm | 0.013 | 0.81 / 0.09 / 0.09 |
/// | 6 | broadband W, 560 nm | 0.025 | 0.07 / 0.14 / 0.79 |
///
/// which is, in the words a person would use: straw, magenta, green, amber,
/// blue. The stacks are written out rather than taken from
/// [`coating_stack`]'s layer ladder, because that ladder's 2-, 4- and 6-layer
/// rungs extend the W with extra quarter-wave pairs and **measure brighter
/// than bare glass** (0.06 to 0.31 peak) — they are a plausible-looking
/// extension rather than a real design. Nothing exercises them: every bundled
/// prescription's coating column is 0 or 1.
///
/// `file_layers` is the prescription's own coating column, used by
/// [`COATING_AS_FILE`] alone.
#[must_use]
pub fn coating_design(choice: u32, file_layers: f32) -> CoatingDesign {
    let none = (0.0, 0.0);
    let (stack, design_nm) = match choice {
        // 1 — bare glass. Bright, neutral, uncoated: pre-war glass, and the
        // reason old lenses flare white.
        1 => ([none; MAX_COATING_LAYERS], COATING_DESIGN_NM),
        // 2 — the classic single MgF₂ quarter wave: one broad shallow V, and
        // the straw cast every coated lens of the 1950s shows.
        2 => ([(MGF2_N, 1.0), none, none, none, none, none], 520.0),
        // 3 — two quarters, MgF₂ over alumina: the deep magenta-purple bloom
        // of a mid-century coated surface, seen head-on.
        3 => (
            [(MGF2_N, 1.0), (AL2O3_N, 1.0), none, none, none, none],
            520.0,
        ),
        // 4 — the broadband W (quarter / half / quarter), cut in the green:
        // two minima, the faintest residual of the set, and the green cast a
        // modern multicoated surface throws.
        5 => (
            [
                (MGF2_N, 1.0),
                (ZRO2_N, 2.0),
                (AL2O3_N, 1.0),
                none,
                none,
                none,
            ],
            480.0,
        ),
        6 => (
            [
                (MGF2_N, 1.0),
                (ZRO2_N, 2.0),
                (AL2O3_N, 1.0),
                none,
                none,
                none,
            ],
            560.0,
        ),
        4 => (
            [
                (MGF2_N, 1.0),
                (ZRO2_N, 2.0),
                (AL2O3_N, 1.0),
                none,
                none,
                none,
            ],
            520.0,
        ),
        // 0 and anything unknown — the prescription's own column.
        _ => (coating_stack(file_layers), COATING_DESIGN_NM),
    };
    CoatingDesign { stack, design_nm }
}

/// The parameter id of each element's coating row (K-371), element 0 first.
/// Spelled out rather than formatted so the schema, the resolve step and the
/// tests all name the same strings and a typo is a compile error.
pub const COATING_ELEMENT_IDS: [&str; MAX_COATING_ELEMENTS] = [
    "coating_el1",
    "coating_el2",
    "coating_el3",
    "coating_el4",
    "coating_el5",
    "coating_el6",
    "coating_el7",
    "coating_el8",
    "coating_el9",
    "coating_el10",
    "coating_el11",
    "coating_el12",
    "coating_el13",
    "coating_el14",
    "coating_el15",
    "coating_el16",
    "coating_el17",
    "coating_el18",
    "coating_el19",
    "coating_el20",
];

/// How many glass elements each bundled lens has, in [`LENS_LIBRARY`] order
/// (K-371) — what the panel turns each element row's threshold into.
///
/// Parsed rather than tabulated: the library is generated, and a hand-kept
/// second table would be one import away from lying about it.
#[must_use]
pub fn library_element_counts() -> Vec<u32> {
    LENS_LIBRARY
        .iter()
        .map(|l| {
            parse_lens(l.text)
                .map(|p| element_count(&p.surfaces) as u32)
                .unwrap_or(0)
        })
        .collect()
}

/// Which lenses have at least `n` glass elements, as [`LENS_LIBRARY`] indices
/// (K-371): the set an element row's group is shown for.
#[must_use]
pub fn lenses_with_at_least(n: u32) -> Vec<u32> {
    library_element_counts()
        .into_iter()
        .enumerate()
        .filter(|&(_, c)| c >= n)
        .map(|(i, _)| i as u32)
        .collect()
}

/// The palette's labels, in index order — the Choice options the per-element
/// coating rows offer.
pub const COATING_DESIGN_OPTIONS: &[&str] = &[
    "As the lens file",
    "Uncoated",
    "Single layer, straw",
    "Two layer, magenta",
    "Broadband, green",
    "Broadband, amber",
    "Broadband, blue",
];

/// Which glass element each surface belongs to (K-371), parallel to
/// `surfaces`; `−1` for a surface that bounds no glass (the aperture stop,
/// and the last air gap).
///
/// # In plain terms
///
/// A `.lens` prescription is a list of *surfaces*, but what a person points at
/// is a *piece of glass* — "the front element", "the rear group". A row whose
/// medium is glass opens an element; the row after it closes that element.
/// Elements are numbered front to back, which is how every lens diagram in
/// every patent numbers them.
///
/// A cemented pair shares its middle surface. That surface has cement on it,
/// not air, so it carries no anti-reflective coating in reality — it is
/// assigned to the **earlier** element, whose choice therefore governs the
/// pair's outer front face and the join, while the later element governs the
/// join and its own rear face. The join is the one surface where the two
/// choices meet, and it goes to whichever element reached it first; nothing
/// about a cemented interface is visible enough to deserve a control of its
/// own.
#[must_use]
pub fn surface_elements(surfaces: &[FlareSurface]) -> Vec<i32> {
    let mut out = vec![-1_i32; surfaces.len()];
    let mut element = -1_i32;
    for (i, s) in surfaces.iter().enumerate() {
        let glass_after = s.cauchy_a > 1.0001;
        if glass_after {
            // This row opens a new piece of glass.
            element += 1;
            out[i] = element;
        } else if element >= 0 && i > 0 && surfaces[i - 1].cauchy_a > 1.0001 {
            // …and this one closes the piece the row before opened.
            out[i] = element;
        }
    }
    out
}

/// How many glass elements a surface table has (K-371) — what the panel shows
/// a coating row for.
#[must_use]
pub fn element_count(surfaces: &[FlareSurface]) -> usize {
    surfaces.iter().filter(|s| s.cauchy_a > 1.0001).count()
}

/// Stamp each surface with the palette entry its element was set to (K-371).
/// Surfaces belonging to no element, and elements past
/// [`MAX_COATING_ELEMENTS`] or left at [`COATING_AS_FILE`], keep the
/// prescription's own coating column.
pub fn apply_element_coatings(surfaces: &mut [FlareSurface], choices: &[u32]) {
    let elements = surface_elements(surfaces);
    for (s, &element) in surfaces.iter_mut().zip(&elements) {
        let choice = usize::try_from(element)
            .ok()
            .and_then(|e| choices.get(e).copied())
            .unwrap_or(COATING_AS_FILE);
        s.coating_design = choice.min(COATING_DESIGNS - 1) as f32;
    }
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
    design: &CoatingDesign,
    lambda_nm: f32,
) -> f32 {
    let stack = &design.stack;
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
            let d_nm = quarters * design.design_nm.max(1.0) / (4.0 * n);
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

/// Reflectance of one lens surface: uncoated Fresnel blended toward that
/// surface's AR coating by the Coating dial. The design comes from the
/// surface's own [`FlareSurface::coating_design`] (K-371) — the element's
/// palette choice, or the prescription's own column where the row is left as
/// the file — resolved by [`coating_design`] and evaluated by
/// [`stack_reflectance`] (K-356, superseding the single-layer-times-a-quarter
/// approximation).
pub fn surface_reflectance(
    cos_i: f32,
    n1: f32,
    n2: f32,
    design_choice: f32,
    file_layers: f32,
    lambda_nm: f32,
    coating_mix: f32,
) -> f32 {
    let plain = fresnel_cos(cos_i, n1, n2);
    if coating_mix <= 0.0 {
        return plain;
    }
    let design = coating_design(design_choice.round().max(0.0) as u32, file_layers);
    if design.stack[0].0 <= 0.0 {
        return plain;
    }
    let coated = stack_reflectance(cos_i, n1, n2, &design, lambda_nm);
    (plain + (coated - plain) * coating_mix.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// One surface's resolved coating design (K-371) — the palette entry its
/// element was set to, falling back to the prescription's own column.
#[must_use]
pub fn surface_design(s: &FlareSurface) -> CoatingDesign {
    coating_design(s.coating_design.round().max(0.0) as u32, s.coating_layers)
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

/// Where one light sits in the field, for the starburst's cat's-eye slice
/// (K-365): `(field fraction, azimuth)` for a light at `light` (raster
/// fraction) on a raster of aspect `h/w`.
///
/// The field fraction is the light's offset in sensor millimetres — the
/// same convention [`light_direction`] projects with, the x fraction against
/// the sensor half-width and the y fraction against that half-width times
/// the raster aspect — over the sensor's half-diagonal, clamped to 1. A
/// light at the frame corner of a 3:2 raster reads 1; a squarer raster
/// reaches the corner sooner and clamps.
///
/// The azimuth is measured on the **raster's** offsets, y DOWN, because
/// what it rotates is the sprite in raster space. Sensor y is up, so this
/// mirrors the true meridional angle — invisible, because the cat's-eye is
/// symmetric about the meridional plane and so is its diffraction pattern.
pub fn starburst_field(light: [f32; 2], aspect_h_over_w: f32) -> (f32, f32) {
    let half_w = SENSOR_MM[0] / 2.0;
    let dx = light[0] - 0.5;
    let dy = light[1] - 0.5;
    let x_mm = 2.0 * dx * half_w;
    let y_mm = 2.0 * dy * aspect_h_over_w * half_w;
    let half_diag = 0.5 * (SENSOR_MM[0] * SENSOR_MM[0] + SENSOR_MM[1] * SENSOR_MM[1]).sqrt();
    let frac = (x_mm.hypot(y_mm) / half_diag).clamp(0.0, 1.0);
    (frac, dy.atan2(dx))
}

/// Trace one pupil sample through one ghost path at one wavelength — the
/// FlareSim three-phase walk (K-261) with K-368's two extra phases, the CPU
/// twin the WGSL splat kernel mirrors op-for-op. `origin` is the ray start
/// (mm), `dir` the unit beam direction; the ray transmits through every
/// surface except the path's bounces, where it reflects (weight × R;
/// transmits weight × (1−R)). Returns the sensor landing (mm, y up) and the
/// accumulated Fresnel weight. A two-bounce path — `[a, b, NO_BOUNCE,
/// NO_BOUNCE]` — executes exactly the statements it did before K-368.
#[allow(clippy::too_many_arguments)]
// Negated comparisons deliberate: NaN reads as dead (see `intersect`).
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn trace_splat(
    baked: &FlareBaked,
    path: [u32; 4],
    lambda_nm: f32,
    origin: [f32; 3],
    dir: [f32; 3],
    coating_mix: f32,
    stop_scale: f32,
    sensor_shift_mm: f32,
) -> Option<([f32; 2], f32)> {
    let surfs = &baked.surfaces;
    let n = surfs.len();
    let (a_idx, b_idx) = (path[0] as usize, path[1] as usize);
    let (c_idx, d_idx) = (path[2] as usize, path[3] as usize);
    let four = path[2] != NO_BOUNCE;
    if n < 3 || a_idx >= b_idx || b_idx >= n {
        return None;
    }
    if four && (c_idx >= n || c_idx <= a_idx || d_idx >= c_idx) {
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
        let r = surface_reflectance(
            cos_i,
            n1,
            n2,
            s.coating_design,
            s.coating_layers,
            lambda_nm,
            coating_mix,
        );
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
    // Phase 3: forward through a+1.., reflecting at c if the path has a
    // third bounce. Without one `end3` is n and `c_idx` the sentinel, which
    // no surface index can equal — so a two-bounce path runs K-261's phase 3
    // statement for statement.
    let end3 = if four { c_idx + 1 } else { n };
    for (s_idx, s) in surfs.iter().enumerate().take(end3).skip(a_idx + 1) {
        step(&mut ray, s, ior_at(s), s_idx == c_idx)?;
    }
    if four {
        // Phase 4 (K-368): backward through c-1..=d, reflecting at d. Phase
        // 2's walk again, one leg further in — same reversed media, same
        // hand-back of the ior at the mirror.
        for s_idx in (d_idx..c_idx).rev() {
            let s = &surfs[s_idx];
            let reflect = s_idx == d_idx;
            step(&mut ray, s, ior_before(s_idx), reflect)?;
            if reflect {
                ray.ior = ior_at(s);
            }
        }
        // Phase 5: forward through d+1..n.
        for s in surfs.iter().skip(d_idx + 1) {
            step(&mut ray, s, ior_at(s), false)?;
        }
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

/// The STRAIGHT imaging path through the whole stack (K-365): no
/// reflections, every surface refracted front to back, returning the same
/// housing/vignette feather [`trace_splat`] applies at the sensor — 1 for a
/// ray that clears every clear aperture comfortably, falling smoothly to 0
/// as it grazes one, `None` for a ray that dies (total internal reflection,
/// or a surface it can never reach).
///
/// This is what shapes the cat's-eye: at a field angle the mechanical stops
/// in front of and behind the iris clip the bundle from opposite sides, so
/// the hole light actually diffracts through is a lens-shaped sliver rather
/// than the iris polygon. The **stop surface is deliberately skipped** when
/// accumulating the feather: the iris is already in the polygon mask
/// [`bake_aperture_field`] multiplies this by, and counting it twice would
/// shrink every aperture image by its own edge.
pub fn trace_transmit(
    baked: &FlareBaked,
    lambda_nm: f32,
    origin: [f32; 3],
    dir: [f32; 3],
) -> Option<f32> {
    let surfs = &baked.surfaces;
    if surfs.len() < 3 {
        return None;
    }
    let mut pos = origin;
    let mut d = dir;
    let mut ior = 1.0_f32;
    let mut rrel2 = 0.0_f32;
    for s in surfs.iter() {
        let (hit, norm, missed) = intersect(pos, d, s.radius_mm, s.z_mm)?;
        pos = hit;
        if missed {
            // Outside the glass entirely: the mount absorbs it (the same
            // 4.0 `trace_splat` uses, which the feather reads as gone).
            rrel2 = rrel2.max(4.0);
        }
        if s.is_stop <= 0.5 {
            // The feather denominator is `trace_splat`'s exactly: the
            // smaller of the clear aperture and the glass's own lateral
            // extent (K-264).
            let semi_r = if s.radius_mm.abs() < 1e-6 {
                s.semi_ap_mm.max(1e-6)
            } else {
                s.semi_ap_mm.min(s.radius_mm.abs()).max(1e-6)
            };
            rrel2 = rrel2.max((pos[0] * pos[0] + pos[1] * pos[1]) / (semi_r * semi_r));
        }
        let n2 = cauchy_ior(s.cauchy_a, s.cauchy_b, lambda_nm);
        // TIR on the imaging path means nothing transmits along it.
        d = refract3(d, norm, ior / n2)?;
        ior = n2;
    }
    if !rrel2.is_finite() {
        return None;
    }
    let ft = ((1.0 - rrel2.sqrt()) / 0.05).clamp(0.0, 1.0);
    Some(ft * ft * (3.0 - 2.0 * ft))
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
    path: [u32; 4],
    band: &SpectralBand,
    origin: [f32; 3],
    dir: [f32; 3],
    coating_mix: f32,
    stop_scale: f32,
    sensor_shift_mm: f32,
) -> Option<([f32; 2], f32, [f32; 3])> {
    let surfs = &baked.surfaces;
    let n = surfs.len();
    let (a_idx, b_idx) = (path[0] as usize, path[1] as usize);
    let (c_idx, d_idx) = (path[2] as usize, path[3] as usize);
    let four = path[2] != NO_BOUNCE;
    if n < 3 || a_idx >= b_idx || b_idx >= n {
        return None;
    }
    if four && (c_idx >= n || c_idx <= a_idx || d_idx >= c_idx) {
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
    // Phase 3: forward through a+1.., reflecting at c if the path has a
    // third bounce (K-368) — the geometry `trace_splat` walks, as this
    // sibling's contract requires.
    let end3 = if four { c_idx + 1 } else { n };
    for s_idx in (a_idx + 1)..end3 {
        let n2 = ior_at(&surfs[s_idx]);
        step(&mut ray, s_idx, n2, s_idx == c_idx, false)?;
    }
    if four {
        // Phase 4: backward through c-1..=d, reflecting at d — reversed
        // media, so the reflectance table is read the other way round.
        for s_idx in (d_idx..c_idx).rev() {
            let reflect = s_idx == d_idx;
            let n2 = ior_before(s_idx);
            step(&mut ray, s_idx, n2, reflect, true)?;
            if reflect {
                ray.ior = ior_at(&surfs[s_idx]);
            }
        }
        // Phase 5: forward through d+1..n.
        for s_idx in (d_idx + 1)..n {
            let n2 = ior_at(&surfs[s_idx]);
            step(&mut ray, s_idx, n2, false, false)?;
        }
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
        .map(|&path| {
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
                        path,
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
    pairs: &[[u32; 4]],
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
                coating_design: COATING_AS_FILE as f32,
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

/// A twentieth of a stop — the step [`bake_params`] snaps the working
/// f-number to. A stop is a factor of √2 in f-number, so the step is
/// `2^(1/40)`, about 1.7%.
pub const FSTOP_BAKE_STEP_STOPS: f32 = 0.05;

/// The step the remaining continuous iris dials snap to (degrees for the
/// rotation, a fraction of the 0..1 range for roundness and softness).
pub const APERTURE_ROTATION_BAKE_STEP_DEG: f32 = 0.5;
/// The bake step for Roundness and Iris softness, both 0..1 dials.
pub const APERTURE_BAKE_STEP: f32 = 1.0 / 256.0;

fn snap(v: f32, step: f32) -> f32 {
    if !v.is_finite() {
        return v;
    }
    // `+ 0.0` folds -0.0 onto 0.0: a rotation keyframed through zero can land
    // on the negative one, and the key hashes bits, so the two would ask the
    // cache for the same iris under different names.
    (v / step).round() * step + 0.0
}

/// The aperture the **bake** sees (K-425), which is the frame's aperture with
/// its continuous dials snapped to a step fine enough not to be seen.
///
/// In plain terms: the bake precomputes two things that depend on the iris —
/// the starburst sprite (a Fourier transform of the hole's shape) and the
/// auto-exposure gain — and it costs about two thirds of a second. An
/// *animated* f-stop asks for a slightly different iris on every single
/// frame, so without this every frame would want its own bake, none would
/// ever arrive in time, and no frame would ever be worth keeping. Snapping
/// the dials means a run of frames shares one bake: a half-stop ramp needs
/// about ten of them, which the 24-entry bake cache holds comfortably.
///
/// What is *not* snapped is everything the frame itself computes — the ghost
/// trace's own stop scale, the iris mask it draws each ghost's rim with, the
/// K-260 wide-open blend it applies there. Those stay continuous, so the
/// ghosts move and shrink smoothly as the iris closes; what steps is the
/// starburst's shape and the exposure, by about 1.7% a step.
///
/// Applied inside both [`bake_key_with`] and [`bake_with`], so a key and the
/// bake it names can never disagree about which aperture was used.
#[must_use]
pub fn bake_params(p: &LensFlareParams) -> LensFlareParams {
    let mut q = *p;
    q.fstop = if p.fstop.is_finite() && p.fstop > 0.0 {
        // Stops, not f-numbers: an even step in f-number would be far finer
        // than it needs to be wide open and far coarser than it may be at
        // f/22. One stop is a factor of √2, hence the halves.
        let stops = 2.0 * p.fstop.log2();
        (snap(stops, FSTOP_BAKE_STEP_STOPS) * 0.5).exp2()
    } else {
        p.fstop
    };
    q.aperture_rotation_deg = snap(p.aperture_rotation_deg, APERTURE_ROTATION_BAKE_STEP_DEG);
    q.roundness = snap(p.roundness, APERTURE_BAKE_STEP);
    q.aperture_softness = snap(p.aperture_softness, APERTURE_BAKE_STEP);
    q
}

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
    // The aperture as the bake will see it (K-425), never as the frame holds
    // it: two f-stops inside one step bake identically, so they must key
    // identically or the cache would hand one of them the other's optics.
    let p = &bake_params(p);
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
    // The per-element coatings (K-371): a bake input, since the reflectance
    // table is built from them.
    for c in p.coating_elements {
        fold(u64::from(c));
    }
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

/// The aperture image for the starburst FFT at one field angle (K-365): the
/// pupil mask (iris at 0.75 of the half-extent, leaving rim room for the
/// diffraction spread), multiplied by the **imaging path's vignette** at
/// that angle — which is what turns the iris polygon into the off-axis
/// cat's-eye.
///
/// `field_frac` runs 0 (on-axis) to 1 (the sensor-corner field angle
/// `atan(half sensor diagonal / focal)`). Every aperture pixel becomes a ray
/// through that point of the entrance pupil at the field angle's direction,
/// and [`trace_transmit`] says how much of it survives the mechanical stops.
/// The azimuth is fixed along +x: the cat's-eye is symmetric about the
/// meridional plane, so one azimuth is the whole family and the combine
/// rotates the sprite to each light instead.
///
/// At `field_frac == 0` this is the pre-K-365 aperture image times the
/// on-axis vignette, which a sane prescription passes at ~1 inside its own
/// pupil — so slice 0 is the picture the effect has always drawn.
pub(crate) fn bake_aperture_field(
    p: &LensFlareParams,
    native_fstop: f32,
    res: u32,
    baked: &FlareBaked,
    field_frac: f32,
) -> Vec<f32> {
    let n = res as usize;
    let mut img = vec![0.0_f32; n * n];
    let rot = p.aperture_rotation_deg.to_radians();
    let roundness = effective_roundness(p.roundness, p.fstop, native_fstop);
    let softness = (p.aperture_softness * 0.25).max(0.004);
    let size = APERTURE_SIZE;
    let half_diag = 0.5 * (SENSOR_MM[0] * SENSOR_MM[0] + SENSOR_MM[1] * SENSOR_MM[1]).sqrt();
    let theta = field_frac.clamp(0.0, 1.0) * (half_diag / baked.focal_mm.max(1e-3)).atan();
    let dir = [theta.sin(), 0.0, theta.cos()];
    // Rays start `START_Z_BACKOFF_MM` in front of the glass, as every other
    // trace in this file does, but the pupil point they are sampling is at
    // the FRONT VERTEX plane — so a tilted ray is walked backwards to the
    // start plane first. Without that back-projection the whole sampled disc
    // slides sideways with the field angle and the corner slice is mostly
    // front-element miss: an artefact of where the ray happens to start, not
    // an aperture.
    let front_z = baked.surfaces.first().map_or(0.0, |s| s.z_mm);
    let lead = dir[0] / dir[2] * (front_z - baked.start_z_mm);
    // The aperture image spans the ENTRANCE PUPIL, `focal / 2N` — not
    // `FlareBaked::pupil_mm`, which is that with half again as margin
    // because ghost paths accept rays the imaging pupil rejects. Traced at
    // the wider radius, the imaging vignette clips the iris polygon into a
    // circle at two thirds of its own edge and every aperture image comes
    // out round, on-axis and off.
    let entrance_mm = (baked.focal_mm / (2.0 * baked.native_fstop.max(0.7)))
        .clamp(1.0, baked.front_semi_ap.max(1.0));
    for y in 0..n {
        for x in 0..n {
            let ndc_x = 2.0 * (x as f32 / (n - 1) as f32) - 1.0;
            let ndc_y = 2.0 * (y as f32 / (n - 1) as f32) - 1.0;
            let (u, v) = (ndc_x / size, ndc_y / size);
            let mask = pupil_mask(u, v, p.blades, rot, roundness, softness);
            if mask <= 0.0 {
                continue;
            }
            let origin = [u * entrance_mm - lead, v * entrance_mm, baked.start_z_mm];
            let vignette = trace_transmit(baked, 550.0, origin, dir).unwrap_or(0.0);
            img[y * n + x] = mask * vignette;
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

// ---------------------------------------------------------------------------
// Ghost-edge Fresnel ringing (K-369, re-derived K-370): the knife-edge rim.
// ---------------------------------------------------------------------------

/// The **Fresnel number** of one ghost's own edge diffraction (K-370).
///
/// # In plain terms
///
/// A ghost is an out-of-focus picture of the iris, and the edge of an
/// out-of-focus hole is not a clean cut: light bends round the blade and lays
/// down a set of fine bright and dark fringes just inside the rim. How fine
/// they are is set by one number, and this works it out from things the bake
/// already knows — how big the ghost is, and how open the iris is.
///
/// # The derivation
///
/// The Fresnel number of an aperture of radius `a` seen at defocus distance
/// `z` is `F = a²/(λz)`. For a ghost, both of those follow from its size on
/// the sensor. The ghost patch *is* the defocused aperture, so its radius on
/// the sensor is `a`; and the cone that forms it leaves the pupil at the
/// marginal-ray angle, which the working f-number fixes at `1/(2N)` — so the
/// defocus is `z ≈ 2Na`. The `a²` and the `a` in `z` cancel one power:
///
/// ```text
/// F = a² / (λ · 2Na) = a / (2Nλ)
/// ```
///
/// `spread` is the measured image **diameter** as a fraction of the sensor
/// diagonal, so `a = spread · diagonal / 2`.
///
/// **This is why K-369's ladder had to go.** Put real numbers in: a 5%-of-
/// frame ghost at f/2.8 is `F ≈ 350`, a frame-filling one `F ≈ 7000`, and the
/// widest washes on the bundled lenses reach `F ≈ 50 000`. K-369 baked its
/// masks at `F` of 2 to 64 — the ceiling a 256² single-FFT propagator can
/// reach, since its output window is `±(N−1)/(4F)` aperture units and has to
/// cover the aperture. Every visible ghost therefore landed on the bottom
/// three rungs, where the near field is not an edge effect at all but a
/// whole-aperture pattern: measured on the bundled default, the interior of
/// an `F 2` slice ran 4.7× bright at the centre falling to 0.3 at the rim,
/// 2.4× the flat mask's interior on average. Painted across ghosts that fill
/// the frame, that is a broad concentric interference pattern over the whole
/// picture — which is exactly what it looked like.
///
/// At the real Fresnel numbers no FFT can reach the fringes are a rim effect
/// a few percent of the pupil wide, and the correct model for them is the
/// straight-edge asymptotic ([`knife_edge_intensity`]) rather than a
/// propagated aperture image. Same physics, the regime it actually applies
/// in, no table.
#[must_use]
pub fn ghost_fresnel_number(spread: f32, fstop: f32) -> f32 {
    if !(spread.is_finite() && fstop.is_finite()) || spread <= 0.0 || fstop <= 0.0 {
        return 0.0;
    }
    let sensor_diag = (SENSOR_MM[0] * SENSOR_MM[0] + SENSOR_MM[1] * SENSOR_MM[1]).sqrt();
    // Radius on the sensor in µm, over 2Nλ with λ in µm.
    let a_um = spread * sensor_diag * 0.5 * 1000.0;
    (a_um / (2.0 * fstop * RING_LAMBDA_UM)).clamp(0.0, 1.0e6)
}

/// The Fresnel integrals `C(v)` and `S(v)` (the `π/2` convention:
/// `C(v) = ∫₀ᵛ cos(πt²/2) dt`), by the standard auxiliary-function rational
/// approximation — absolute error under 2e-3, which is far below the blur
/// [`ghost_mask`] averages the profile under.
///
/// Odd in `v`, which is what lets one evaluation serve both sides of the
/// aperture edge: inside it tends to `+½`, outside to `−½`.
#[must_use]
pub fn fresnel_cs(v: f32) -> (f32, f32) {
    let x = v.abs();
    // Abramowitz & Stegun 7.3.32/7.3.33's auxiliary functions.
    let f = (1.0 + 0.926 * x) / (2.0 + 1.792 * x + 3.104 * x * x);
    let g = 1.0 / (2.0 + 4.142 * x + 3.492 * x * x + 6.670 * x * x * x);
    // **The argument is reduced by hand, and it has to be.** `x` reaches the
    // low hundreds deep inside a ghost, so `x²` reaches five figures and the
    // phase `πx²/2` runs to tens of thousands of radians. A CPU `sin` reduces
    // that properly; a GPU one is not required to, and on real hardware does
    // not — the two twins then disagreed by 1.25% of the frame's total energy,
    // spread over every ghost interior, which is exactly the shape of a
    // range-reduction failure. `x²` mod 4 is one f32 multiply and a floor,
    // identical on both sides by IEEE, and leaves both asking for a sine of
    // something under 2π.
    let t = x * x;
    let arg = std::f32::consts::FRAC_PI_2 * (t - 4.0 * (t * 0.25).floor());
    let (s, c) = arg.sin_cos();
    let cc = 0.5 + f * s - g * c;
    let ss = 0.5 - f * c - g * s;
    if v < 0.0 {
        (-cc, -ss)
    } else {
        (cc, ss)
    }
}

/// The intensity a straight diffracting edge casts, relative to the
/// unobstructed beam, at normalised distance `v` inside the geometric edge
/// (negative is outside, in the geometric shadow).
///
/// `I(v) = ½[(C(v) + ½)² + (S(v) + ½)²]` — the textbook knife-edge result. It
/// is 1 deep inside, exactly ¼ on the geometric edge, and rings above 1 just
/// inside it: the first fringe peaks at about 1.37 at `v ≈ 1.22`, then 1.20,
/// then 1.15, dying away as the edge is left behind. Outside it decays
/// smoothly to nothing rather than cutting off, which is the light real
/// diffraction throws past the blade.
///
/// **The interior is flat by construction**, and that is the whole point of
/// the change: whatever else this profile does, it cannot tint or shade the
/// middle of a ghost.
#[must_use]
pub fn knife_edge_intensity(v: f32) -> f32 {
    let (c, s) = fresnel_cs(v);
    0.5 * ((c + 0.5) * (c + 0.5) + (s + 0.5) * (s + 0.5))
}

/// The blur, in `v` units, below which the rim fringes are fully drawn and
/// above which they have averaged away to the plain iris edge.
///
/// Fringes narrower than the thing sampling them do not appear — they
/// **alias**, and an aliased fringe train is a beat pattern spread across the
/// whole ghost, which is the other half of what K-369 put on screen. The
/// honest answer when they cannot be resolved is their average, and a
/// diffraction profile averages to the geometric edge it surrounds. So the
/// mask crosses from one to the other over this window rather than drawing
/// fringes it cannot carry.
const RING_WASH: (f32, f32) = (0.5, 2.0);

/// The iris mask one ray sees, with the ghost's own edge diffraction on it
/// (K-370) — the shared twin of `fx_lens_flare_trace.wgsl`'s `ghost_mask`.
///
/// `fresnel` is the path's [`ghost_fresnel_number`]; at 0 this is exactly
/// [`pupil_mask`] and nothing changes. `blur` is how far apart, in pupil
/// units, the things looking at this mask are — the ray grid's step — and it
/// is what decides whether the fringes can be drawn at all.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn ghost_mask(
    u: f32,
    v: f32,
    blades: u32,
    rot_rad: f32,
    roundness: f32,
    softness: f32,
    fresnel: f32,
    blur: f32,
) -> f32 {
    let analytic = pupil_mask(u, v, blades, rot_rad, roundness, softness);
    if !fresnel.is_finite() || fresnel <= 0.0 {
        return analytic;
    }
    let r = (u * u + v * v).sqrt();
    let blades = blades.clamp(3, 16);
    let sector = std::f32::consts::TAU / blades as f32;
    let apothem = (std::f32::consts::PI / blades as f32).cos();
    let angle = v.atan2(u) - rot_rad;
    let mut a = angle % sector;
    if a < 0.0 {
        a += sector;
    }
    // The same radial bound `pupil_mask` uses, and the cosine that turns a
    // radial gap into the PERPENDICULAR distance to the blade — which is the
    // distance a straight-edge profile is a function of. Exact for the
    // polygon (the edge sits at `apothem`, so `bound − r` along the radius is
    // `(apothem − r cos α)/cos α`), and 1 for a fully round iris.
    let cos_a = (a - sector * 0.5).cos();
    let poly_bound = apothem / cos_a;
    let roundness = roundness.clamp(0.0, 1.0);
    let bound = poly_bound + (1.0 - poly_bound) * roundness;
    let cos_fac = cos_a + (1.0 - cos_a) * roundness;
    // v units: the profile's own scale is `1/√(2F)` of the pupil radius.
    let scale = (2.0 * fresnel).sqrt();
    let ringed = knife_edge_intensity((bound - r) * cos_fac * scale);
    // A soft blade edge smears its own fringes exactly as a coarse ray grid
    // does, so the two enter the same way: whichever is wider decides.
    let soft = (softness.clamp(0.0, 1.0) * bound).max(1e-4);
    let blur_v = blur.max(0.0).max(soft) * scale;
    let t = ((blur_v - RING_WASH.0) / (RING_WASH.1 - RING_WASH.0)).clamp(0.0, 1.0);
    let wash = t * t * (3.0 - 2.0 * t);
    ringed + (analytic - ringed) * wash
}

/// The mean flare-buffer brightness the auto-exposure steers every lens
/// toward, measured by actually rendering the CPU reference at thumbnail
/// size inside the bake (K-258). Cheaper proxies mispredicted real lenses
/// by orders of magnitude; the closed loop cannot.
const TARGET_PROBE_MEAN: f32 = 0.010;

/// The four-bounce paths worth ray-probing (K-368), best bound first.
///
/// In plain terms: a ghost is light that bounced off two surfaces on its way
/// through. Some of it bounces twice more and still lands on the sensor —
/// far fainter, but the sun is ~10⁵ times a normal highlight and a few of
/// those paths focus tightly, so on uncoated glass they are plainly there.
/// The trouble is arithmetic: `N(N−1)/2` two-bounce paths become ~N⁴/4
/// four-bounce ones, and a ray probe each is out of the question.
///
/// So they are pre-ranked by an **upper bound** on the energy the path can
/// carry: the product of the four surfaces' reflectances at normal
/// incidence and 550 nm. It is an upper bound in the coating's own terms —
/// an AR stack is designed to be at its worst on-axis, and the rest of the
/// path only ever removes light — and it is a product of numbers under one,
/// so a partial product bounds the whole. That is what lets the search stop
/// early: once the kept set is full, any `(a, b)` whose pair reflectance
/// cannot beat the worst kept candidate even with the best possible `c` and
/// `d` is skipped whole.
///
/// The bound never decides what renders. It decides only which candidates
/// reach [`bake_with`]'s ray probe, and that probe — the same one, the same
/// [`PAIR_MIN_INTENSITY`] floor — decides the rest.
fn four_bounce_candidates(
    surfs: &[FlareSurface],
    has_interface: &dyn Fn(usize) -> bool,
) -> Vec<[u32; 4]> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let n = surfs.len();
    let ifaces: Vec<usize> = (0..n).filter(|&i| has_interface(i)).collect();
    if ifaces.len() < 2 {
        return Vec::new();
    }
    // One normal-incidence reflectance per surface, at the design
    // wavelength: the whole cost of the prefilter, paid once.
    let r0: Vec<f32> = (0..n)
        .map(|i| {
            let s = &surfs[i];
            let n1 = if i == 0 {
                1.0
            } else {
                cauchy_ior(surfs[i - 1].cauchy_a, surfs[i - 1].cauchy_b, 550.0)
            };
            let n2 = cauchy_ior(s.cauchy_a, s.cauchy_b, 550.0);
            let design = surface_design(s);
            if design.stack[0].0 <= 0.0 {
                fresnel_cos(1.0, n1, n2)
            } else {
                stack_reflectance(1.0, n1, n2, &design, 550.0)
            }
        })
        .collect();
    let r_max = ifaces.iter().fold(0.0_f32, |m, &i| m.max(r0[i]));
    // A bounded top-K: the heap's root is the WORST kept candidate, so the
    // memory is `FOUR_BOUNCE_PROBE_CAP` entries however many paths the
    // prescription has. The bound travels as its bit pattern because f32 has
    // no total order — non-negative finite floats compare as their bits do —
    // and the tuple breaks ties, so the kept set is one deterministic set
    // rather than whichever equal-bound path was seen first.
    let mut heap: BinaryHeap<(Reverse<u32>, [u32; 4])> = BinaryHeap::new();
    for (ai, &a) in ifaces.iter().enumerate() {
        for &b in &ifaces[ai + 1..] {
            let ab = r0[a] * r0[b];
            if heap.len() >= FOUR_BOUNCE_PROBE_CAP {
                if let Some(&(Reverse(bits), _)) = heap.peek() {
                    if ab * r_max * r_max <= f32::from_bits(bits) {
                        continue;
                    }
                }
            }
            for &c in ifaces.iter().filter(|&&c| c > a) {
                for &d in ifaces.iter().filter(|&&d| d < c) {
                    let bound = ab * r0[c] * r0[d];
                    if bound <= 0.0 || !bound.is_finite() {
                        continue;
                    }
                    heap.push((
                        Reverse(bound.to_bits()),
                        [a as u32, b as u32, c as u32, d as u32],
                    ));
                    if heap.len() > FOUR_BOUNCE_PROBE_CAP {
                        heap.pop();
                    }
                }
            }
        }
    }
    // The heap's drain order is unspecified; the probe's input order is not.
    let mut out: Vec<[u32; 4]> = heap.into_iter().map(|(_, path)| path).collect();
    out.sort_unstable();
    out
}

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
    // The snapped aperture (K-425), matching what `bake_key_with` hashed —
    // the two must read the same dials or a cache slot would hold optics its
    // name does not describe.
    let quantised = bake_params(p);
    let p = &quantised;
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
                    coating_design: COATING_AS_FILE as f32,
                },
                FlareSurface {
                    radius_mm: 0.0,
                    z_mm: 4.0,
                    semi_ap_mm: 12.0,
                    cauchy_a: 1.0,
                    cauchy_b: 0.0,
                    coating_layers: 0.0,
                    is_stop: 1.0,
                    coating_design: COATING_AS_FILE as f32,
                },
                FlareSurface {
                    radius_mm: -50.0,
                    z_mm: 8.0,
                    semi_ap_mm: 15.0,
                    cauchy_a: 1.0,
                    cauchy_b: 0.0,
                    coating_layers: 0.0,
                    is_stop: 0.0,
                    coating_design: COATING_AS_FILE as f32,
                },
            ],
            sensor_z_mm: 55.0,
        });
    let mut lens = lens;
    // The per-element coatings are a bake input (K-371): they change what
    // every surface reflects, so they are resolved into the surface table
    // before the reflectance table is built from it, and `bake_key` folds
    // them in so a change here rebakes exactly as a lens change does.
    apply_element_coatings(&mut lens.surfaces, &p.coating_elements);
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
    // add the four-bounce paths the K-368 prefilter picked out, probe each
    // on-axis, and rank the two kinds together in ONE list — the ranking
    // cannot tell them apart, and on a lens with few pairs (the bundled
    // Biotar runs out after 45) the four-bounce paths reach the rendered
    // set, which is exactly why old glass shows doubled ghosts.
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
    let mut candidates: Vec<[u32; 4]> = (0..n)
        .filter(|&a| has_interface(a))
        .flat_map(|a| {
            ((a + 1)..n)
                .filter(|&b| has_interface(b))
                .map(move |b| [a as u32, b as u32, NO_BOUNCE, NO_BOUNCE])
        })
        .collect();
    candidates.extend(four_bounce_candidates(&baked.surfaces, &|i| {
        has_interface(i)
    }));
    use rayon::prelude::*;
    let ranked: Vec<([u32; 4], f32)> = candidates
        .par_iter()
        .filter_map(|&path| {
            // On-axis brightness probe at the R/G/B wavelengths, full file
            // coating (the Coating dial is frame-time). Four-bounce paths
            // face exactly this probe and exactly this floor (K-368): the
            // enumeration bound decided only which of them got here.
            let mut est = 0.0_f32;
            for nm in [650.0, 550.0, 450.0] {
                if let Some((_, w)) = trace_splat(&baked, path, nm, centre, axis, 1.0, 1.0, 0.0) {
                    est += w;
                }
            }
            est /= 3.0;
            (est >= PAIR_MIN_INTENSITY).then_some((path, est))
        })
        .collect();
    let mut ranked = ranked;
    // Descending probe brightness; ties by path order (deterministic).
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
    let probe_pairs: Vec<[u32; 4]> = baked
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

    // The aperture image per field angle (K-365). This is the expensive half
    // of the starburst — one traced ray per texel through the whole
    // prescription — so the slices go wide across the pool.
    let apertures: Vec<Vec<f32>> = (0..STARBURST_FIELDS)
        .into_par_iter()
        .map(|f| {
            let frac = f as f32 / (STARBURST_FIELDS - 1) as f32;
            bake_aperture_field(p, native_fstop, APERTURE_RES, &baked, frac)
        })
        .collect();

    // The starburst, once per field-angle slice (K-365) — the slices are
    // independent FFTs, so they go wide, and `collect` puts them back in
    // slice order whatever order the pool finished them in (determinism).
    let mut slices: Vec<Vec<f32>> = apertures
        .par_iter()
        .map(|aperture| bake_starburst(aperture, STARBURST_RES))
        .collect();
    // A lens that does not cover the full frame — the bundled 7Artisans is
    // an APS-C design, and a user file may be anything — passes NOTHING at
    // the outer field angles, so its aperture is empty and its sprite comes
    // out black. A starburst that vanishes as the light nears the corner is
    // a worse picture than one that stopped changing there, so a dead slice
    // holds the last live one instead.
    let floor = slices.first().map_or(0.0, |s| s.iter().sum::<f32>()) * 1e-3;
    for f in 1..slices.len() {
        if slices[f].iter().sum::<f32>() <= floor {
            let held = slices[f - 1].clone();
            slices[f] = held;
        }
    }
    baked.starburst = slices.concat();

    // Closed-loop auto exposure (K-258): render the reference thumbnail with
    // gain 1 at FIXED frame-time settings — only bake-key inputs may steer
    // the gain, or animating a frame-time dial would rebake — and normalise
    // the mean to the target. Deterministic, and a few milliseconds.
    baked.energy_gain = 1.0;
    let probe_frame = LensFlareParams {
        // The element coatings are already resolved into the surface table,
        // so the probe's own copy is inert — but it is a bake-key input, and
        // carrying the real one keeps "the probe sees the baked lens" true.
        coating_elements: p.coating_elements,
        // Raster pixels of the 96×54 thumbnail (the 0.33/0.30 framing).
        light: [31.7, 16.2],
        // The probe is always one Manual point, whatever the frame's source
        // mode: the gain must be steered by bake-key inputs alone, and the
        // comp's lights are frame-time.
        lights: [DEAD_LIGHT; MAX_SOURCES],
        light_count: 0,
        intensity: 1.0,
        lens: p.lens,
        // The probe is shot at the lens's NATIVE stop, never the working one
        // (K-426): the gain is a property of the glass, not of how far the
        // iris is closed. Reading the working stop made the gain roughly
        // `(f/native)²` and it cancelled the stop-down — a lens rendered the
        // same brightness at f/16 as wide open, which no lens does — and it
        // put the working f-number under the exposure half of the bake, so a
        // ramped aperture stepped the whole flare's brightness at every
        // snapped step. Stopped down, the flare now dims as the light the
        // iris passes dims; Intensity is the dial that puts it back.
        fstop: native_fstop,
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
        let design = surface_design(s);
        let bare = design.stack[0].0 <= 0.0;
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
                    let r = if bare {
                        fresnel_cos(cos_i, n1, n2)
                    } else {
                        stack_reflectance(cos_i, n1, n2, &design, nm)
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

/// Floor on a ray's landed footprint as a fraction of its launch cell —
/// which is really a **cap on caustic density** (K-262, kept by K-366). At
/// a fold the density `cell ÷ landed` genuinely diverges, but the *integral*
/// over a pixel is finite: a discrete ray concentrates that divergence into
/// a few pixels, so an uncapped splat drew a hard chromatic line. Capping at
/// 1/3e-3 ≈ 333× keeps the bright rims and arcs and removes the spikes.
pub const MIN_AREA_FRAC: f32 = 3e-3;

/// Shortest half-axis a splat's footprint may have, px (K-366): the
/// anti-alias floor. A ray whose local footprint collapses below a pixel
/// still deposits over roughly one, so a caustic line is a line and not a
/// row of dropped sub-pixel points — the job [`MIN_QUAD_PX`]'s inflation
/// used to do, without the sliver cases that came with connecting rays.
pub const MIN_SPLAT_AXIS_PX: f32 = 0.75;

/// One traced corner: landing px, geometric weight (housing feather × iris
/// mask) and band-integrated rgb (K-364); the splat path's working type.
pub(crate) type Corner = ([f32; 2], f32, [f32; 3]);

/// Widest kernel span, px, a splat deposits at full resolution (K-380).
/// Past this it moves to a coarser accumulator level — halved once per
/// doubling — so one splat never costs more than about this many pixels
/// squared, however large the ghost. The smoothing that costs is about a
/// twenty-fourth of the splat's own size: invisible on the smooth defocused
/// ghosts that have splats this big, and precisely the deal the owner asked
/// for ("use the grid to speed things up then smooth it"). Spelled in
/// `fx_lens_flare_deposit.wgsl` too, pinned by test.
pub const DEPOSIT_SPAN_PX: f32 = 48.0;

/// Most accumulator levels a deposit pyramid holds (K-380) — level 11 is a
/// 2048-fold reduction, past any raster the engine renders. Pinned against
/// the shader's array size by test.
pub const MAX_DEPOSIT_LEVELS: usize = 12;

/// The deposit pyramid's level dimensions for a `w × h` flare buffer
/// (K-380): level 0 is the buffer itself, each next level halves both axes
/// (rounding up), stopping once a level fits 32 px or the table is full.
/// The GPU mirrors this (`deposit_levels_of`, pinned by test) so the two
/// twins can never disagree about where a level's pixels sit.
pub fn deposit_levels(w: u32, h: u32) -> Vec<(u32, u32)> {
    let (mut lw, mut lh) = (w.max(1), h.max(1));
    let mut out = vec![(lw, lh)];
    while out.len() < MAX_DEPOSIT_LEVELS && lw.max(lh) > 32 {
        lw = lw.div_ceil(2);
        lh = lh.div_ceil(2);
        out.push((lw, lh));
    }
    out
}

/// Which pyramid level a splat with kernel reach `ext` (px each way, level
/// 0) deposits at (K-380): the shallowest whose span is within
/// [`DEPOSIT_SPAN_PX`]. By repeated exact halving, not a logarithm — the
/// shader does the identical loop, and halving is exact in floating point,
/// so the two twins pick the same level for every splat.
pub fn deposit_level(ext_x: f32, ext_y: f32, level_count: u32) -> u32 {
    let mut span = 2.0 * ext_x.max(ext_y);
    let mut level = 0u32;
    while span > DEPOSIT_SPAN_PX && level + 1 < level_count {
        span *= 0.5;
        level += 1;
    }
    level
}

/// The CPU reference's deposit pyramid (K-380): one f32 RGB buffer per
/// level of [`deposit_levels`]. The GPU's is one flat fixed-point buffer
/// with per-level offsets; same shape, same arithmetic per level.
pub(crate) struct DepositLevels {
    pub(crate) dims: Vec<(u32, u32)>,
    pub(crate) bufs: Vec<Vec<f32>>,
}

impl DepositLevels {
    pub(crate) fn new(w: u32, h: u32) -> Self {
        let dims = deposit_levels(w, h);
        let bufs = dims
            .iter()
            .map(|&(lw, lh)| vec![0.0_f32; (lw * lh * 3) as usize])
            .collect();
        Self { dims, bufs }
    }

    /// Sum the levels into a `w × h × 3` buffer: level 0 lands exactly (its
    /// texels are the output pixels), each coarser level is bilinearly
    /// upsampled — the WGSL `resolve` op for op.
    pub(crate) fn resolve(&self, out: &mut [f32]) {
        let (w, h) = self.dims[0];
        for (level, &(lw, lh)) in self.dims.iter().enumerate() {
            let buf = &self.bufs[level];
            let s = (1u32 << level) as f32;
            for y in 0..h as usize {
                let pos_y = ((y as f32 + 0.5) / s - 0.5).max(0.0);
                let y0 = (pos_y as usize).min(lh as usize - 1);
                let y1 = (y0 + 1).min(lh as usize - 1);
                let fy = (pos_y - y0 as f32).clamp(0.0, 1.0);
                for x in 0..w as usize {
                    let pos_x = ((x as f32 + 0.5) / s - 0.5).max(0.0);
                    let x0 = (pos_x as usize).min(lw as usize - 1);
                    let x1 = (x0 + 1).min(lw as usize - 1);
                    let fx = (pos_x - x0 as f32).clamp(0.0, 1.0);
                    let tap = |lx: usize, ly: usize, c: usize| buf[(ly * lw as usize + lx) * 3 + c];
                    let idx = (y * w as usize + x) * 3;
                    for c in 0..3 {
                        let top = tap(x0, y0, c) + (tap(x1, y0, c) - tap(x0, y0, c)) * fx;
                        let bot = tap(x0, y1, c) + (tap(x1, y1, c) - tap(x0, y1, c)) * fx;
                        out[idx + c] += top + (bot - top) * fy;
                    }
                }
            }
        }
    }
}

/// A ray's local footprint axes (K-366, widened K-378): the image of one
/// pupil-grid step under the ghost map, read off the neighbouring rays'
/// landings — one-sided at the grid edge or beside a dead ray, and the
/// anti-alias floor when no neighbour survives at all. Half-steps, so the
/// parallelogram `centre ± a1 ± a2` tiles the grid exactly once.
///
/// **The LONGER of the two one-sided differences, not their average**
/// (K-378). On a smooth map the two sides agree and this is the central
/// difference it always was. Under an area source they do not: the source
/// offsets hop by design, and wherever the two neighbours happened to land
/// on the same side their average cancelled toward zero — a collapsed splat
/// sitting between two wide gaps, quasi-periodically across the whole
/// ghost, which is exactly the woven mesh the owner photographed. Taking
/// the longer side makes under-coverage impossible; the cost is overlap,
/// and overlap is only blur.
pub(crate) fn ray_axes(
    corners: &[Option<Corner>],
    side: usize,
    i: usize,
    j: usize,
) -> ([f32; 2], [f32; 2]) {
    let at = |x: usize, y: usize| corners[y * side + x].map(|(p, _, _)| p);
    let axis = |lo: Option<[f32; 2]>, here: [f32; 2], hi: Option<[f32; 2]>| -> Option<[f32; 2]> {
        match (lo, hi) {
            // Both neighbours live: the longer one-sided step (K-378).
            (Some(a), Some(b)) => {
                let lov = [here[0] - a[0], here[1] - a[1]];
                let hiv = [b[0] - here[0], b[1] - here[1]];
                let l2 = lov[0] * lov[0] + lov[1] * lov[1];
                let h2 = hiv[0] * hiv[0] + hiv[1] * hiv[1];
                Some(if l2 > h2 { lov } else { hiv })
            }
            (None, Some(b)) => Some([b[0] - here[0], b[1] - here[1]]),
            (Some(a), None) => Some([here[0] - a[0], here[1] - a[1]]),
            (None, None) => None,
        }
    };
    let here = at(i, j).unwrap_or([0.0, 0.0]);
    let ax = axis(
        (i > 0).then(|| at(i - 1, j)).flatten(),
        here,
        (i + 1 < side).then(|| at(i + 1, j)).flatten(),
    );
    let ay = axis(
        (j > 0).then(|| at(i, j - 1)).flatten(),
        here,
        (j + 1 < side).then(|| at(i, j + 1)).flatten(),
    );
    // Half-axes: the splat spans centre ± a, so a full grid step is 2a.
    let half = |v: [f32; 2]| [v[0] / 2.0, v[1] / 2.0];
    match (ax, ay) {
        (Some(x), Some(y)) => (half(x), half(y)),
        // One live axis: give the dead one the anti-alias floor, at right
        // angles, so the splat is a thin quad along the live direction.
        (Some(x), None) => {
            let len = (x[0] * x[0] + x[1] * x[1]).sqrt().max(1e-6);
            let n = [
                -x[1] / len * MIN_SPLAT_AXIS_PX,
                x[0] / len * MIN_SPLAT_AXIS_PX,
            ];
            (half(x), n)
        }
        (None, Some(y)) => {
            let len = (y[0] * y[0] + y[1] * y[1]).sqrt().max(1e-6);
            let n = [
                -y[1] / len * MIN_SPLAT_AXIS_PX,
                y[0] / len * MIN_SPLAT_AXIS_PX,
            ];
            (n, half(y))
        }
        // A lone survivor: the floor in both directions.
        (None, None) => ([MIN_SPLAT_AXIS_PX, 0.0], [0.0, MIN_SPLAT_AXIS_PX]),
    }
}

/// The quadratic B-spline, in units of one grid step (K-376).
///
/// `3/4 − t²` inside a half step, `(3/2 − |t|)²/2` out to one and a half, zero
/// beyond. It sums to one over any lattice of unit spacing — a partition of
/// unity, like the tent — but unlike the tent it is **C¹**: no kink where one
/// cell meets the next.
///
/// That difference is the whole reason it is here. A tent reconstructs a
/// surface with a crease along every cell boundary, and the eye finds creases
/// even when they are tiny — it is the same Mach-band sensitivity that makes a
/// polygon silhouette visible in smooth shading. Measured on a real frame, the
/// residual against a local mean falls from 2.42% to 1.91%, and the character
/// of what is left changes from a lattice to ordinary image detail.
///
/// It is the standard answer in the simulation literature for exactly this
/// symptom, where it goes by "grid imprinting" or "cell-crossing noise".
#[inline]
fn bspline_q(t: f32) -> f32 {
    let a = t.abs();
    if a <= 0.5 {
        0.75 - a * a
    } else {
        let e = 1.5 - a;
        0.5 * e * e
    }
}

/// Deposit one ray's flux over its footprint (K-366, reconstruction fixed
/// K-373): a separable tent centred on the ray, reaching **one full grid step**
/// in each direction, so the tents of neighbouring rays overlap and sum to
/// one. Flux is conserved exactly — except at a caustic, where the density cap
/// (see [`MIN_AREA_FRAC`]) deliberately sheds the divergence. The WGSL splat
/// quad and fragment mirror this arithmetic op for op.
///
/// # Why the kernel reaches past its own cell
///
/// `a1` and `a2` are **half**-axes: a full step between neighbouring rays is
/// `2·a1`. K-366 gave the tent a support of `±a1`, which is half a step — so
/// two neighbouring tents met exactly at the point where both had fallen to
/// zero. That is not a partition of unity; it is a lattice of separate
/// pyramids with a seam of zero along every cell boundary. Summed over the
/// grid it reconstructs a **woven grid of dark lines at the ray spacing**,
/// with brighter ridges along the two pupil axes through each ghost's centre,
/// and stepped rims where the seams cross a silhouette. Energy was still
/// conserved, which is why every flux test passed and the artefact was
/// visible on screen anyway.
///
/// A linear B-spline partitions unity when its support is **twice** the sample
/// spacing — tents at spacing `h` reaching `±h`. Here the spacing is `2·a1`,
/// so the reach is `±2·a1`, which is what the kernel below evaluates. The
/// integral grows by 4 with it, so the peak is divided by 4 and the deposited
/// flux is unchanged; `area` and the density cap keep their K-366 meaning in
/// half-axis units, untouched.
///
/// K-376 then widened it again, to the quadratic B-spline's one and a half
/// steps, because a tent is only C⁰ and the crease at each cell boundary is
/// itself visible. Nine cells a splat against the original one, and that is
/// the price of a reconstruction that does not print its own sampling grid on
/// the picture.
///
/// K-380 caps what one splat may COST: a splat whose kernel span exceeds
/// [`DEPOSIT_SPAN_PX`] deposits into a coarser level of the pyramid instead
/// — same kernel, same peak (a density per level-0 pixel is a density at
/// any level), evaluated at texels a power of two apart and read back
/// through the resolve's bilinear upsample. The floors, the guard and the
/// density cap all run at level 0 BEFORE the level is chosen, exactly as
/// the GPU's `build_splats` runs them before its deposit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn splat_ray(
    levels: &mut DepositLevels,
    centre: [f32; 2],
    a1_in: [f32; 2],
    a2_in: [f32; 2],
    flux_rgb: [f32; 3],
    cell_area_px: f32,
) {
    // The anti-alias floor per axis, preserving direction.
    let floor_axis = |a: [f32; 2], fallback: [f32; 2]| -> [f32; 2] {
        let len = (a[0] * a[0] + a[1] * a[1]).sqrt();
        if len < 1e-6 {
            return fallback;
        }
        if len < MIN_SPLAT_AXIS_PX {
            let f = MIN_SPLAT_AXIS_PX / len;
            [a[0] * f, a[1] * f]
        } else {
            a
        }
    };
    let a1 = floor_axis(a1_in, [MIN_SPLAT_AXIS_PX, 0.0]);
    let mut a2 = floor_axis(a2_in, [0.0, MIN_SPLAT_AXIS_PX]);
    // Near-parallel axes are a fold seen edge-on: the footprint collapses
    // even though both axes are long. Push a2's across-component up to the
    // floor so the deposit is at least a pixel-wide line, not a zero-area
    // parallelogram whose flux vanishes.
    let mut det = a1[0] * a2[1] - a1[1] * a2[0];
    let a1_len = (a1[0] * a1[0] + a1[1] * a1[1]).sqrt().max(1e-6);
    if det.abs() < MIN_SPLAT_AXIS_PX * a1_len {
        let n = [-a1[1] / a1_len, a1[0] / a1_len];
        let sign = if det >= 0.0 { 1.0 } else { -1.0 };
        a2 = [
            a2[0] + n[0] * MIN_SPLAT_AXIS_PX * sign,
            a2[1] + n[1] * MIN_SPLAT_AXIS_PX * sign,
        ];
        det = a1[0] * a2[1] - a1[1] * a2[0];
    }
    let area = det.abs().max(1e-6);
    // The density cap: the divisor never drops below the launch cell's
    // capped fraction, so a fold brightens to 333× and stops (K-262's rule,
    // carried over unchanged).
    let divisor = area.max(MIN_AREA_FRAC * cell_area_px);
    // Divided by four beside the reach doubling above: the tent over ±2 in
    // each axis integrates to 4x the parallelogram's area, so the flux the
    // ray deposits is exactly what it was.
    let peak = [
        flux_rgb[0] / (4.0 * divisor),
        flux_rgb[1] / (4.0 * divisor),
        flux_rgb[2] / (4.0 * divisor),
    ];
    // Bounding box of the kernel's REACH, which is three half-axes each way.
    let ext_x = 3.0 * (a1[0].abs() + a2[0].abs());
    let ext_y = 3.0 * (a1[1].abs() + a2[1].abs());
    // The pyramid level this splat can afford (K-380), and everything
    // scaled into its pixels. The kernel below is unchanged by the scale:
    // (u, v) solve the same system whether both sides carry the 1/s, and
    // `peak` is a density per level-0 pixel, which the resolve's upsample
    // reads back out at level 0.
    let level = deposit_level(ext_x, ext_y, levels.dims.len() as u32);
    let s = (1u32 << level) as f32;
    let (lw, lh) = levels.dims[level as usize];
    let out = &mut levels.bufs[level as usize];
    let centre = [centre[0] / s, centre[1] / s];
    let a1 = [a1[0] / s, a1[1] / s];
    let a2 = [a2[0] / s, a2[1] / s];
    let det = det / (s * s);
    let inv_det = 1.0 / det;
    let (ext_x, ext_y) = (ext_x / s, ext_y / s);
    let x0 = ((centre[0] - ext_x).floor().max(0.0)) as i64;
    let x1 = ((centre[0] + ext_x).ceil().min(lw as f32 - 1.0)) as i64;
    let y0 = ((centre[1] - ext_y).floor().max(0.0)) as i64;
    let y1 = ((centre[1] + ext_y).ceil().min(lh as f32 - 1.0)) as i64;
    if x1 < x0 || y1 < y0 {
        return;
    }
    for py in y0..=y1 {
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - centre[0];
            let dy = py as f32 + 0.5 - centre[1];
            // (u, v) in the parallelogram's own frame: solve [a1 a2]·(u,v)ᵀ = d.
            let u = (dx * a2[1] - dy * a2[0]) * inv_det;
            let v = (dy * a1[0] - dx * a1[1]) * inv_det;
            // The quadratic B-spline, in half-axis units where a full grid
            // step is 2 (K-376). Support is 1.5 steps each way.
            if u.abs() >= 3.0 || v.abs() >= 3.0 {
                continue;
            }
            let k = bspline_q(u * 0.5) * bspline_q(v * 0.5);
            let idx = ((py as u32 * lw + px as u32) * 3) as usize;
            if let Some(px3) = out.get_mut(idx..idx + 3) {
                px3[0] += peak[0] * k;
                px3[1] += peak[1] * k;
                px3[2] += peak[2] * k;
            }
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
    // The deposit pyramid (K-380): splats land at the level their size
    // affords, and the resolve below sums the levels into `out`.
    let mut deposit = DepositLevels::new(rw, rh);
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
    let mut corners: Vec<Option<Corner>> = Vec::new();
    // The pair's iris mask per pupil corner — wavelength-independent, so it
    // is computed once a pair and read by every wavelength (K-263).
    let mut masks: Vec<f32> = Vec::new();
    for light in lights {
        if light.rgb[0] <= 0.0 && light.rgb[1] <= 0.0 && light.rgb[2] <= 0.0 {
            continue;
        }
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
            //
            // The shape is not quite the analytic polygon either: the ghost's
            // own edge diffraction rings the rim (K-369, re-derived K-370).
            // Its Fresnel number is worked out here rather than baked because
            // it moves with the working stop — a stopped-down iris makes both
            // a smaller ghost and a coarser fringe.
            let fresnel = ghost_fresnel_number(
                baked.spreads.get(pi).copied().unwrap_or(1.0) * stop_scale,
                p.fstop,
            );
            // The ray grid's own step, which is what decides whether fringes
            // this fine can be drawn at all rather than aliased.
            let grid_step = 2.0 / (side - 1) as f32;
            masks.clear();
            masks.resize(side * side, 0.0);
            for j in 0..side {
                for i in 0..side {
                    let (u, v) = (unit(i), unit(j));
                    masks[j * side + i] = ghost_mask(
                        u,
                        v,
                        p.blades,
                        rot,
                        roundness,
                        p.aperture_softness,
                        fresnel,
                        grid_step,
                    );
                }
            }
            for (bi, band) in bands.iter().enumerate() {
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
                        // **Each ray integrates the source itself** (K-367).
                        // The ray takes its light from its own point of the
                        // source's ±extent rectangle, so the source integral
                        // is absorbed into the pupil quadrature rather than
                        // replicating the whole pipeline per sample. A point
                        // source offsets by zero and this is a no-op.
                        let jit = source_jitter(i, j, bi, light.extent);
                        let dir = light_direction(
                            [light.pos[0] + jit[0], light.pos[1] + jit[1]],
                            aspect,
                            baked.focal_mm,
                        );
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
                // Per-ray splatting (K-366). Each traced ray deposits its
                // pupil cell's flux over its own local footprint — the image
                // of the launch cell under this ghost's map, read off the
                // neighbouring rays as a 2×2 Jacobian. Rays are never
                // connected across a fold the way the old quads were, so a
                // caustic is just splats piling up (the correct integral)
                // and the whole sliver/inflate/pull-in rescue machinery of
                // K-261..K-264 has nothing left to rescue.
                for j in 0..side {
                    for i in 0..side {
                        let Some((pos, wgt, rgb)) = corners[j * side + i] else {
                            continue;
                        };
                        if wgt <= 0.0 {
                            continue;
                        }
                        let (a1, a2) = ray_axes(&corners, side, i, j);
                        let flux = wgt * gain * cell_area_px;
                        splat_ray(
                            &mut deposit,
                            pos,
                            a1,
                            a2,
                            [
                                flux * rgb[0] * light.rgb[0],
                                flux * rgb[1] * light.rgb[1],
                                flux * rgb[2] * light.rgb[2],
                            ],
                            cell_area_px,
                        );
                    }
                }
            }
        }
    }
    // The pyramid's levels sum into the flat buffer (K-380) — the WGSL
    // resolve, op for op.
    deposit.resolve(&mut out);
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
/// One starburst deposit in [`cpu_combine`] (K-367): where the sprite is
/// centred (raster fraction), the colour it carries — the light's, times its
/// share of the stamp grid — and the K-365 field terms for that position.
struct SbStamp {
    pos: [f32; 2],
    rgb: [f32; 3],
    ca: f32,
    sa: f32,
    s0: usize,
    s1: usize,
    ts: f32,
}

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
    let squeeze = p.anamorphic.clamp(0.25, 4.0);
    let fscale = p.scale.clamp(0.05, 20.0);
    let sb_res = STARBURST_RES as usize;
    let sb_half = 0.6 * fscale * w.min(h) as f32;
    let aspect = h as f32 / w.max(1) as f32;
    // A bake always carries every slice; a hand-built one need not, and the
    // sampler below indexes by slice. No slices, no starburst — the same
    // labelled no-op an intensity of zero gives.
    let sb_ready = baked.starburst.len() >= STARBURST_FIELDS * sb_res * sb_res * 3;
    // The frame's starburst stamps (K-367): one per light for a point, a 3×3
    // grid across the source for an area light — the shift-invariant
    // convolution of the sprite with the source, in quadrature form (see
    // [`starburst_stamp_grid`]). Each stamp's cat's-eye slice pair and sprite
    // rotation (K-365) belong to the STAMP, not to the light: a smeared
    // starburst near the frame edge leans a little differently at each end of
    // itself, which is the physical picture. All of it is a property of where
    // the stamp is rather than of the pixel, so it is worked out once here
    // where the shader must redo it per pixel.
    let mut stamps: Vec<SbStamp> = Vec::new();
    for l in lights {
        if l.rgb[0] <= 0.0 && l.rgb[1] <= 0.0 && l.rgb[2] <= 0.0 {
            continue;
        }
        let (nx, ny) = starburst_stamp_grid(l.extent);
        let share = 1.0 / (nx * ny) as f32;
        for iy in 0..ny {
            for ix in 0..nx {
                let pos = [
                    l.pos[0] + starburst_stamp_offset(ix, nx) * l.extent[0],
                    l.pos[1] + starburst_stamp_offset(iy, ny) * l.extent[1],
                ];
                let (fld, az) = starburst_field(pos, aspect);
                let s = fld * (STARBURST_FIELDS - 1) as f32;
                let s0 = (s.floor() as usize).min(STARBURST_FIELDS - 1);
                stamps.push(SbStamp {
                    pos,
                    rgb: [l.rgb[0] * share, l.rgb[1] * share, l.rgb[2] * share],
                    ca: az.cos(),
                    sa: az.sin(),
                    s0,
                    s1: (s0 + 1).min(STARBURST_FIELDS - 1),
                    ts: s - s0 as f32,
                });
            }
        }
    }
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
            // One starburst sprite per stamp, anchored on the stamp, sized by
            // Scale, stretched by the squeeze, tinted by its share of the
            // light. A point source has exactly one stamp at its own position
            // carrying its whole colour, so it lands where it always did.
            let mut sb = [0.0_f32; 3];
            if p.starburst_intensity > 0.0 && sb_half > 0.0 && sb_ready {
                for stamp in &stamps {
                    let light_px = [stamp.pos[0] * w as f32, stamp.pos[1] * h as f32];
                    let rel_x = x as f32 + 0.5 - light_px[0];
                    let rel_y = y as f32 + 0.5 - light_px[1];
                    // The cat's-eye (K-365): the sprite is turned so the
                    // baked +x lean points along the stamp's own radial
                    // direction, and read from the two field slices
                    // bracketing the stamp's field fraction. On-axis the
                    // fraction is 0 and the azimuth arbitrary — but slice 0
                    // is very nearly round, so turning it changes nothing
                    // and the picture stays continuous through the centre.
                    let (ca, sa, s0, s1, ts) = (stamp.ca, stamp.sa, stamp.s0, stamp.s1, stamp.ts);
                    let rot_x = rel_x * ca + rel_y * sa;
                    let rot_y = -rel_x * sa + rel_y * ca;
                    let u = rot_x / (sb_half * squeeze) * 0.5 + 0.5;
                    let v = rot_y / sb_half * 0.5 + 0.5;
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
                    // Slice-major, so a slice is one contiguous sprite: the
                    // bilinear taps can never bleed into its neighbour.
                    let tap = |slice: usize, c: usize| -> f32 {
                        let base = slice * sb_res * sb_res * 3;
                        let a = baked.starburst[base + (y0 * sb_res + x0) * 3 + c] * (1.0 - tx)
                            + baked.starburst[base + (y0 * sb_res + x1) * 3 + c] * tx;
                        let b = baked.starburst[base + (y1 * sb_res + x0) * 3 + c] * (1.0 - tx)
                            + baked.starburst[base + (y1 * sb_res + x1) * 3 + c] * tx;
                        a * (1.0 - ty) + b * ty
                    };
                    for (c, out_c) in sb.iter_mut().enumerate() {
                        let val = tap(s0, c) * (1.0 - ts) + tap(s1, c) * ts;
                        *out_c += val * p.starburst_intensity * stamp.rgb[c];
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
