use super::markers::flash_nth;
use super::*;
use crate::model::{EffectInstance, EffectNamespace, EffectValue};

/// One effect, resolved to plain numbers at a frame — the flat form both the
/// WGSL kernels (lumit-gpu) and the CPU references below consume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolved {
    Blur {
        /// Kernel half-width in *pixels of the target raster* (the caller
        /// converts from % diagonal using the raster it renders at, §2.3).
        radius_px: f32,
        /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
    DirBlur {
        /// Full streak length in raster pixels.
        length_px: f32,
        /// Streak direction, degrees (0° = +x, y-down raster).
        angle_deg: f32,
        /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
    /// Blur's Radial mode (docs/08 §3.8): rays from, or a tangent to the
    /// arc about, a centre — see the schema's status note for why both
    /// reduce to a pure linear scale of (position − centre) with no
    /// division or runtime trig.
    RadialBlur {
        /// Centre as a *fraction* of the raster (not raster pixels):
        /// resolve_stack carries only diag_px, not separate width/height,
        /// so the CPU/GPU function scales this by its own w/h — exactly
        /// how RGB split's radial mode already derives the frame centre.
        centre_frac: [f32; 2],
        /// Peak tap spread in raster pixels, reached at the frame's
        /// farthest corner from Centre (half the raster diagonal away).
        amount_px: f32,
        /// True = Spin (tangent direction), false = Zoom (radial direction).
        spin: bool,
        /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
    Sharpen {
        /// Fraction of the detail signal added back (0..3 = 0–300%).
        amount: f32,
        /// The internal gaussian's half-width, in raster pixels.
        radius_px: f32,
        /// Linear-light detail magnitude below which nothing is added.
        threshold: f32,
        /// True: sharpen the Rec. 709 luma only (no chroma fringing).
        luma_only: bool,
        /// 0..1.
        mix: f32,
    },
    /// Sharpen (docs/08 §3.9, K-138): a plain 3×3 high-pass convolution scaled
    /// by `amount`, on unpremultiplied colour (§2.2), alpha untouched — the
    /// radius-free sibling of [`Resolved::Sharpen`] (the Unsharp mask). `out =
    /// u + amount·(4·u − up − down − left − right)` per RGB channel with
    /// clamp-addressed neighbours, clamped ≥ 0 and re-premultiplied. `amount`
    /// 0 (or `mix` 0) is the bit-exact passthrough.
    SharpenSimple {
        /// High-pass strength; 0 is the neutral point (1 = the classic 5/−1
        /// kernel).
        amount: f32,
        /// 0..1.
        mix: f32,
    },
    RgbSplit {
        /// Peak channel offset in raster pixels.
        amount_px: f32,
        /// Linear-mode shift direction, degrees (0° = +x, y-down raster).
        angle_deg: f32,
        /// True: offsets grow from the frame centre instead.
        radial: bool,
        /// 0..1.
        mix: f32,
    },
    /// The RGB split's Wavelength mode (docs/08 §3.6, K-090): its own
    /// variant, exactly as Blur's Directional mode is — so the classic
    /// mode's path stays byte-identical.
    SpectralSplit {
        /// Peak spectral offset in raster pixels.
        amount_px: f32,
        /// Linear-mode shift direction, degrees (0° = +x, y-down raster).
        angle_deg: f32,
        /// True: offsets grow from the frame centre instead.
        radial: bool,
        /// 0..1.
        mix: f32,
    },
    /// Chromatic aberration (docs/08 §3.15): a dedicated, always-radial
    /// sibling of RGB split's own Radial mode — always centred on the
    /// frame, no angle or linear mode of its own.
    ChromaticAberration {
        /// Peak channel offset in raster pixels, reached at the corner
        /// distance from the frame centre.
        amount_px: f32,
        /// 0..1.
        mix: f32,
    },
    Flash {
        /// The evaluated envelope × intensity, 0..1 (0 = no flash).
        strength: f32,
        /// Scene-linear RGBA flash colour (alpha unused: the flash respects
        /// the layer's own footprint).
        colour: [f32; 4],
        /// 0..1.
        mix: f32,
    },
    ColourBalance {
        /// Added per channel after gain (raises or crushes the blacks).
        lift: [f32; 3],
        /// Per-channel mid-tone exponent's base; 1 is neutral, > 0.
        gamma: [f32; 3],
        /// Per-channel linear multiplier; 1 is neutral.
        gain: [f32; 3],
        /// 0..1.
        mix: f32,
    },
    Saturation {
        /// Factor about Rec. 709 luma: 0 = greyscale, 1 = neutral, 2 =
        /// doubled, and open above (K-135) — the maths extrapolates.
        saturation: f32,
        /// 0..1.
        mix: f32,
    },
    /// Matte key (docs/08 §3.21): a soft chroma key. `key` is the scene-linear
    /// RGBA key colour (resolved at frame time like Vignette's tint, alpha
    /// ignored); the CPU/GPU maths derive its chroma and hue direction from it
    /// identically. `tol`/`soft`/`spill` are plain 0..1 fractions. The keep
    /// factor smoothsteps from 0 (fully keyed, alpha ·= 0) at chroma distance
    /// `tol` to 1 (fully kept) at `tol + soft`, so it is continuous
    /// everywhere. There is no neutral no-op default; Mix 0 is the identity.
    MatteKey {
        /// Scene-linear RGBA key colour (alpha ignored).
        key: [f32; 4],
        /// Chroma-distance threshold, 0..1: at/below it the pixel is fully keyed.
        tol: f32,
        /// Soft-edge width above `tol`, 0..1: the smoothstep transition span.
        soft: f32,
        /// Key-hue spill removal, 0..1: fraction of residual key chroma pulled out.
        spill: f32,
        /// 0..1.
        mix: f32,
    },
    /// Vignette (docs/08 §3.14): darkens toward black away from the frame
    /// centre. `radius`/`softness` are read against the Roundness-blended
    /// distance metric [`cpu::vignette`] computes from `w`/`h` — no raster
    /// conversion happens here, unlike the %-diag family, because the
    /// metric is already resolution-relative by construction.
    Vignette {
        /// 0..1: darkening strength; 0 is the neutral point.
        amount: f32,
        /// 0..1: the clear centre's reach.
        radius: f32,
        /// ≥ 0: feather width beyond radius, open above (K-135).
        softness: f32,
        /// 0..1: 1 = circular, 0 = follows the frame's aspect.
        roundness: f32,
        /// 0..1.
        mix: f32,
    },
    /// Exposure (docs/08 §3.16): RGB × `factor` (= 2^stops), alpha untouched.
    /// `factor` 1.0 is the neutral point.
    Exposure {
        /// Linear gain, 2^stops.
        factor: f32,
        /// 0..1.
        mix: f32,
    },
    /// Hue shift (docs/08 §3.17, K-136): a row-major linear 3×3 colour matrix,
    /// computed host-side — either the constant-luminance rotation (Preserve
    /// luminance on) or the plain-RGB spin (off). The kernel is matrix-general,
    /// so both modes share one op. Identity is the neutral point.
    HueShift {
        /// Row-major 3×3: `[m00,m01,m02, m10,m11,m12, m20,m21,m22]`.
        m: [f32; 9],
        /// 0..1.
        mix: f32,
    },
    /// Contrast (docs/08 §3.18): the affine grade `(in − 0.5) × k + 0.5` per
    /// RGB channel on unpremultiplied colour, alpha untouched. `k` 1.0
    /// (Contrast 100 %) is the neutral point.
    Contrast {
        /// Contrast factor, `contrast_percent / 100`; 1.0 is neutral.
        k: f32,
        /// 0..1.
        mix: f32,
    },
    /// Gamma (docs/08 §3.19): the per-channel power curve
    /// `out = pow(max(in, 0), 1/gamma)` on unpremultiplied colour, alpha
    /// untouched. `gamma` 1.0 is the neutral point.
    Gamma {
        /// Gamma value; the curve raises to `1/gamma`. 1.0 is neutral,
        /// clamped ≥ 0.01 so the reciprocal stays finite.
        gamma: f32,
        /// 0..1.
        mix: f32,
    },
    /// Temperature (docs/08 §3.20): a warm/cool white balance as a per-channel
    /// R/B gain in scene-linear light, computed host-side, alpha untouched.
    /// Gains `(1.0, 1.0)` (Temperature 0) are the neutral point.
    Temperature {
        /// Scene-linear red gain, `max(0, 1 + 0.75·(temperature/100))`.
        gain_r: f32,
        /// Scene-linear blue gain, `max(0, 1 − 0.75·(temperature/100))`.
        gain_b: f32,
        /// 0..1.
        mix: f32,
    },
    /// Invert (docs/08 §3.23): the colour inverse `out.rgb = 1 − in.rgb` per RGB
    /// channel on unpremultiplied colour, alpha untouched. No neutral value —
    /// invert always inverts — so only Mix 0 is the identity.
    Invert {
        /// 0..1.
        mix: f32,
    },
    /// Tint (docs/08 §3.24): a luminance duotone. `out.rgb = black + (white −
    /// black)·luma(in)` with Rec.709 luma on the unpremultiplied colour, alpha
    /// untouched. The two mapped colours resolve to scene-linear RGB at frame
    /// time; Mix 0 is the identity.
    Tint {
        /// Scene-linear RGB the darkest input maps to.
        black: [f32; 3],
        /// Scene-linear RGB the brightest input maps to.
        white: [f32; 3],
        /// 0..1.
        mix: f32,
    },
    Transform {
        /// Anchor point, raster pixels (converted from px@comp, §2.3).
        anchor: [f32; 2],
        /// Where the anchor lands, raster pixels.
        position: [f32; 2],
        /// Per-axis factor; 1 is natural size, negative flips.
        scale: [f32; 2],
        /// Degrees about the anchor (0° = none; y-down raster, so positive
        /// turns clockwise on screen, matching the layer transform).
        rotation_deg: f32,
        /// 0..1, multiplied into the premultiplied output.
        opacity: f32,
        /// 0..1.
        mix: f32,
    },
    Glow {
        /// The halo gaussian's half-width in raster pixels.
        radius_px: f32,
        /// Linear-light bright threshold, ≥ 0 (unbounded above, K-090).
        threshold: f32,
        /// Soft-knee width around the threshold, 0..1.
        knee: f32,
        /// Gain on the added halo; 0 is the neutral point.
        intensity: f32,
        /// Scene-linear RGBA halo tint (alpha unused: the halo's own alpha
        /// is untinted coverage).
        tint: [f32; 4],
        /// 0..1.
        mix: f32,
    },
    /// A shake, already sampled at this frame (the noise runs at resolve
    /// time, host-side): the current wobble, dispatched through the Transform
    /// kernel via [`shake_affine`] — no kernel of its own. `edge` (P3, K-145)
    /// governs the border the resample reveals; there is no Auto-scale cover
    /// any more (FX-11/K-146 replaced it with this Edges control).
    Shake {
        /// This frame's wobble offset, raster pixels.
        offset_px: [f32; 2],
        /// This frame's rotation wobble, degrees.
        rotation_deg: f32,
        /// This frame's zoom factor; 1 = no depth (z) shake.
        zoom: f32,
        /// Edge policy for the revealed border: 0 Transparent, 1 Repeat,
        /// 2 Mirror ([`EdgesMode`]).
        edge: u32,
        /// 0..1.
        mix: f32,
    },
    /// Block glitch (docs/08 §3.12, split out by K-107). `tick` is the
    /// local time already discretised at [`GLITCH_TICK_HZ`] (host-side, so
    /// the kernel never sees raw time or does its own time maths).
    /// Intensity 0 is the bit-exact passthrough (pinned by test) — see the
    /// schema's status note for why every hashed quantity here is scaled by
    /// it.
    BlockGlitch {
        /// The master 0..1 dial; scales every hashed quantity.
        intensity: f32,
        seed: u32,
        /// Local time discretised at [`GLITCH_TICK_HZ`] (§3.12 status
        /// note): per-block hashing reads this, not raw time.
        tick: i32,
        /// Raster pixels (px@comp × the §2.3 preview factor).
        block_size_px: f32,
        /// 0..1, fraction of block_size_px (the "Rows/columns jitter").
        jitter_frac: f32,
        /// Peak per-block displacement, raster pixels (% diag).
        amount_px: f32,
        /// Peak per-block R/B split, raster pixels (% diag).
        chan_px: f32,
        /// 0..1: odds (before the Intensity scale) a block slice-repeats.
        slice_frac: f32,
        /// 0..1.
        mix: f32,
    },
    /// Scanlines (docs/08 §3.12, split out by K-107). `roll_px` is the
    /// scanline pattern's already-computed pixel offset (roll speed × local
    /// time × period), host-computed so the kernel never sees raw time.
    /// Intensity 0 is the bit-exact passthrough (pinned by test).
    Scanlines {
        /// The master 0..1 dial; scales the darken strength.
        intensity: f32,
        /// Raster pixels (px@comp × the §2.3 preview factor).
        period_px: f32,
        /// 0..1.
        darkness: f32,
        /// The scanline pattern's pixel offset at this frame (roll speed ×
        /// local time × period_px, host-computed).
        roll_px: f32,
        interlace: bool,
        /// 0..1.
        mix: f32,
    },
    /// Datamosh (docs/08 §3.12, K-104, its own effect since K-107): re-warp
    /// the -1 source neighbour along the flow measured from this frame to
    /// it. The neighbour frame and its flow field are not carried here —
    /// like Echo's neighbour frames and Motion blur's flow field, they
    /// travel beside the resolved op, supplied only when the layer is
    /// footage and the decode fetched them; a missing pair degrades this to
    /// a no-op, never a fault.
    Datamosh {
        /// 0..1: blended against the current frame.
        intensity: f32,
        /// 0..1, the host Mix. Composes with `intensity` by multiplication
        /// before reaching the kernel (mixing the same two inputs twice
        /// collapses to one mix by the product), so the existing GPU/CPU
        /// maths need not carry a second blend knob.
        mix: f32,
    },
    /// Echo / trails (docs/08 §3.13). `weights[i]` is the intensity of the
    /// echo at frame offset `-(i+1)` (0 = no echo there); the render supplies
    /// the neighbour frame at each live offset. `mode`: 0 = Add, 1 = Behind,
    /// 2 = Max.
    Echo {
        weights: [f32; 8],
        mode: u32,
        /// 0..1.
        mix: f32,
    },
    /// Flow motion blur (docs/08 §3.2). The per-pixel motion vectors are not
    /// carried here — they are a whole flow field, computed in the decode
    /// worker and passed to the kernel as a separate texture (the same way
    /// Echo's neighbour *frames* travel beside the resolved op, not inside
    /// it). This variant carries only the scalars the kernel needs to turn a
    /// vector into a streak.
    MotionBlur {
        /// Shutter ÷ 360: the streak length as a fraction of the inter-frame
        /// motion (0 = no blur; 0.5 at the 180° default).
        shutter_frac: f32,
        /// Evenly spaced bilinear taps along the streak (already rounded and
        /// clamped from the Samples parameter).
        samples: i32,
        /// 0..1.
        mix: f32,
    },
    /// LUT (docs/08 §3.11, docs/impl/lut.md, K-114): a 3D `.cube` colour
    /// lookup. Only the host Mix is `Copy`-carried here; the parsed-and-
    /// uploaded cube is a whole 3D texture, so — like Echo's neighbour frames
    /// and Motion blur's flow field — it travels beside the resolved op (the
    /// caller's LUT cache fills a parallel `luts` slot), not inside it. An
    /// unset/1D/unreadable file leaves that slot empty and the op is a
    /// passthrough. `mix == 0` is the bit-exact input.
    Lut {
        /// 0..1.
        mix: f32,
    },
    /// Depth of field (docs/08 §3.22, docs/impl/layer-input.md): a lens blur
    /// whose per-pixel circle-of-confusion comes from a depth pass. Only the
    /// scalars are `Copy`-carried here; the depth is a whole texture — the
    /// referenced layer rendered alone at comp size — so (like the LUT's cube
    /// and Motion blur's flow field) it travels beside the resolved op (the
    /// caller fills a parallel `layer_inputs` slot), not inside it. An unset,
    /// missing or cyclic depth reference leaves that slot empty and the op is
    /// a passthrough. `aperture == 0`, an all-in-band depth, or `mix == 0` are
    /// bit-exact passthroughs.
    Dof {
        /// The in-focus depth, 0..1.
        focus: f32,
        /// Half-width of the sharp band around `focus`, 0..1.
        range: f32,
        /// Maximum circle-of-confusion radius for the **near** side (depths in
        /// front of focus, `d < focus`), raster pixels — the per-side Near blur
        /// already scaled by the Aperture master and the §2.3 preview factor.
        near_aperture: f32,
        /// Maximum circle-of-confusion radius for the **far** side (depths
        /// behind focus, `d >= focus`), raster pixels — the Far blur already
        /// scaled by the master and the preview factor.
        far_aperture: f32,
        /// When set, the per-pixel depth is inverted (`d' = 1 - d`) before the
        /// circle-of-confusion, swapping near and far. A `Copy` scalar, so the
        /// enum stays `Copy` and threads beside the depth texture unchanged.
        depth_invert: bool,
        /// Diagnostic view: 0 = Rendered (the blurred output), 1 = Depth map
        /// (post-invert greyscale), 2 = Focus map (the smooth in-focus mask).
        /// Modes 1/2 ignore the blur and Mix and write the view directly.
        display: u32,
        /// 0..1.
        mix: f32,
    },
}

/// Resolve a layer's live stack at layer time `lt` for a raster whose
/// diagonal is `diag_px` pixels; `px_scale` is raster pixels per comp pixel
/// (the §2.3 preview-resolution factor — 1.0 at full resolution), which
/// converts px@comp parameters exactly as `diag_px` converts % diag ones.
/// `markers` is the layer's §1.4 marker context ([`MarkerContext::for_layer`],
/// or [`MarkerContext::NONE`] where no comp is in play), consumed by the
/// marker-driven modes (Flash's Trigger and Strobe, §3.7). Placeholders,
/// unknown names and bypassed effects resolve to nothing (they render as
/// identity, docs/03 §8).
pub fn resolve_stack(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Vec<Resolved> {
    effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
        .filter_map(|e| resolve_one(e, lt, diag_px, px_scale, markers))
        .collect()
}

/// Resolve a layer's live stack for a held/sub-frame re-render (docs/impl/
/// temporal-rerender.md §5): an effect flagged `sample_temporally == false`
/// resolves at the true frame time `frame_lt` (so a particle system or other
/// costly/stochastic effect is not re-run per held sample), while every other
/// effect resolves at the held/sample time `sample_lt`. When `sample_lt ==
/// frame_lt` this is byte-identical to [`resolve_stack`], so an ordinary
/// (non-temporal) render is unchanged — the two share [`resolve_one`], differing
/// only in which layer time each effect is handed.
pub fn resolve_stack_temporal(
    effects: &[EffectInstance],
    sample_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Vec<Resolved> {
    effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
        .filter_map(|e| {
            let lt = if e.sample_temporally {
                sample_lt
            } else {
                frame_lt
            };
            resolve_one(e, lt, diag_px, px_scale, markers)
        })
        .collect()
}

/// Resolve one effect instance to its flat [`Resolved`] op at layer time `lt`,
/// or None when it is a placeholder, an unknown name, or an orchestration-only
/// effect (Posterize time, accumulation motion blur) that has no per-pixel op.
/// The shared core of [`resolve_stack`] and [`resolve_stack_temporal`].
fn resolve_one(
    e: &EffectInstance,
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
) -> Option<Resolved> {
    match e.effect.match_name.as_str() {
        "blur" => {
            // Gaussian blur (docs/08 §3.8, K-137). match_name "blur" is kept,
            // so a project saved with the old mode-driven blur — whatever mode
            // it stored — loads here as Gaussian at its Radius, byte-identically
            // (its now-unread mode/length/centre params are simply ignored).
            // Fixed Repeat edge (K-137 dropped the Gaussian Edges control; 1 was
            // its default).
            let radius_pct = e.float_at("radius", lt)? as f32;
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Blur {
                radius_px: (radius_pct / 100.0 * diag_px).max(0.0),
                edge: 1,
                mix,
            })
        }
        "directional_blur" => {
            // Directional blur (docs/08 §3.8, K-137): Length/Angle only, fixed
            // Repeat edge (the Edges control is Radial's alone now).
            let length_pct = e.float_at("length", lt).unwrap_or(0.0) as f32;
            let angle_deg = e.float_at("angle", lt).unwrap_or(0.0) as f32;
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::DirBlur {
                length_px: (length_pct / 100.0 * diag_px).max(0.0),
                angle_deg,
                edge: 1,
                mix,
            })
        }
        "radial_blur" => {
            // Radial blur (docs/08 §3.8, K-137): Centre/Amount/Type, plus the
            // family's own Edges control (kept only here).
            let cx = (e.float_at("centre_x", lt).unwrap_or(50.0) / 100.0) as f32;
            let cy = (e.float_at("centre_y", lt).unwrap_or(50.0) / 100.0) as f32;
            let amount_pct = e.float_at("amount", lt).unwrap_or(0.0) as f32;
            let spin = !matches!(e.param("radial_type"), Some(EffectValue::Choice(1)));
            // The reusable Edges control (P3, K-145): the stored Choice maps
            // through EdgesMode (clamped to the known set, default Repeat).
            let edge = match e.param("edge") {
                Some(EffectValue::Choice(c)) => {
                    EdgesMode::from_code((*c).min(2)).unwrap_or(EdgesMode::Repeat)
                }
                _ => EdgesMode::Repeat,
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::RadialBlur {
                centre_frac: [cx, cy],
                amount_px: (amount_pct / 100.0 * diag_px).max(0.0),
                spin,
                edge: edge.code(),
                mix,
            })
        }
        "sharpen" => {
            let amount = (e.float_at("amount", lt)? as f32 / 100.0).clamp(0.0, 3.0);
            let radius_pct = e.float_at("radius", lt)? as f32;
            let threshold = (e.float_at("threshold", lt).unwrap_or(0.05) as f32).clamp(0.0, 1.0);
            let luma_only = match e.param("luminance_only") {
                Some(EffectValue::Bool(b)) => *b,
                _ => true,
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Sharpen {
                amount,
                radius_px: (radius_pct / 100.0 * diag_px).max(0.0),
                threshold,
                luma_only,
                mix,
            })
        }
        "sharpen_simple" => {
            // The plain 3×3 sharpen (docs/08 §3.9, K-138): Amount is a raw
            // high-pass strength (not a per-cent), clamped ≥ 0.
            let amount = (e.float_at("amount", lt)? as f32).max(0.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::SharpenSimple { amount, mix })
        }
        "rgb_split" => {
            let amount_pct = e.float_at("amount", lt)? as f32;
            let angle_deg = e.float_at("angle", lt).unwrap_or(0.0) as f32;
            let radial = match e.param("radial") {
                Some(EffectValue::Bool(b)) => *b,
                _ => false,
            };
            // Instances saved before the Wavelength mode existed carry
            // no such parameter and resolve as the classic split.
            let wavelength = match e.param("wavelength") {
                Some(EffectValue::Bool(b)) => *b,
                _ => false,
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            let amount_px = (amount_pct / 100.0 * diag_px).max(0.0);
            Some(if wavelength {
                Resolved::SpectralSplit {
                    amount_px,
                    angle_deg,
                    radial,
                    mix,
                }
            } else {
                Resolved::RgbSplit {
                    amount_px,
                    angle_deg,
                    radial,
                    mix,
                }
            })
        }
        "chromatic_aberration" => {
            let amount_px = (e.float_at("amount", lt).unwrap_or(4.0) as f32 * px_scale).max(0.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::ChromaticAberration { amount_px, mix })
        }
        "flash" => {
            // Instances saved before the marker modes existed carry no
            // "mode" parameter and resolve as Manual — byte-identically.
            let mode = match e.param("mode") {
                Some(EffectValue::Choice(c)) => *c,
                _ => 0,
            };
            let envelope = match mode {
                // Trigger (1) and Strobe (2): the §3.7 beat envelope
                // from the §1.4 context; Strobe thins the beat list to
                // every Nth.
                1 | 2 => {
                    let duration = e.float_at("duration", lt).unwrap_or(2.0).max(0.0);
                    let fade = matches!(e.param("shape"), Some(EffectValue::Choice(1)));
                    let nth = if mode == 2 { flash_nth(e, lt) } else { 1 };
                    let phase = e.float_at("phase", lt).unwrap_or(0.0);
                    flash_beat_envelope(markers, lt, duration, fade, nth, phase)
                }
                // Manual: keyframed hits on Trigger, decaying over
                // Decay — the original form, untouched.
                _ => {
                    let decay_s = (e.float_at("decay", lt).unwrap_or(120.0) / 1000.0).max(0.0);
                    match e.param("trigger") {
                        Some(EffectValue::Float(p)) => flash_envelope(p, lt, decay_s),
                        _ => 0.0,
                    }
                }
            };
            let intensity = e.float_at("intensity", lt).unwrap_or(100.0).max(0.0) / 100.0;
            let colour = e.colour_at("colour", lt).unwrap_or([1.0; 4]);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Flash {
                strength: (envelope * intensity).clamp(0.0, 1.0) as f32,
                colour: colour.map(|c| c as f32),
                mix,
            })
        }
        "colour_balance" => {
            let rgb = |id: &str, neutral: f64| -> [f32; 3] {
                let c = e.colour_at(id, lt).unwrap_or([neutral; 4]);
                [c[0] as f32, c[1] as f32, c[2] as f32]
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::ColourBalance {
                lift: rgb("lift", 0.0),
                gamma: rgb("gamma", 1.0).map(|g| g.max(0.01)),
                gain: rgb("gain", 1.0),
                mix,
            })
        }
        "saturation" => {
            // Floored at 0 (greyscale), open above (K-135): the luma/colour
            // mix extrapolates past 200 % cleanly, so no upper clamp.
            let saturation =
                (e.float_at("saturation", lt).unwrap_or(100.0) as f32 / 100.0).max(0.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Saturation { saturation, mix })
        }
        "matte_key" => {
            // The colour is resolved to a scene-linear array at frame time,
            // like Vignette's tint would be; the CPU reference and the WGSL
            // kernel derive its chroma/hue direction from it identically.
            // Tolerance/Softness/Spill are per cent → plain 0..1 fractions.
            let key = e.colour_at("key", lt).unwrap_or([0.0, 0.6, 0.0, 1.0]);
            let tol = (e.float_at("tolerance", lt).unwrap_or(20.0) as f32 / 100.0).max(0.0);
            let soft = (e.float_at("softness", lt).unwrap_or(10.0) as f32 / 100.0).max(0.0);
            let spill = (e.float_at("spill", lt).unwrap_or(0.0) as f32 / 100.0).clamp(0.0, 1.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::MatteKey {
                key: key.map(|c| c as f32),
                tol,
                soft,
                spill,
                mix,
            })
        }
        "vignette" => {
            let amount = (e.float_at("amount", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
            let radius = (e.float_at("radius", lt).unwrap_or(0.75) as f32).clamp(0.0, 1.0);
            // Floored at 0, open above (K-135): softness > 1 is a legal wider
            // feather in the normalised metric, no upper clamp.
            let softness = (e.float_at("softness", lt).unwrap_or(0.5) as f32).max(0.0);
            let roundness = (e.float_at("roundness", lt).unwrap_or(1.0) as f32).clamp(0.0, 1.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Vignette {
                amount,
                radius,
                softness,
                roundness,
                mix,
            })
        }
        "exposure" => {
            let stops = e.float_at("stops", lt).unwrap_or(0.0);
            let factor = 2f64.powf(stops) as f32;
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Exposure { factor, mix })
        }
        "hue_shift" => {
            let angle = e.float_at("angle", lt).unwrap_or(0.0);
            // Preserve luminance (K-136): on (default, and absent on old
            // projects) → the Rec.709 constant-luminance rotation; off → the
            // plain-RGB spin about the grey axis. The bool only picks which
            // host-computed matrix is carried, so CPU and GPU stay in parity.
            let preserve = !matches!(
                e.param("preserve_luminance"),
                Some(EffectValue::Bool(false))
            );
            let m = if preserve {
                hue_matrix(angle)
            } else {
                hue_matrix_rgb(angle)
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::HueShift { m, mix })
        }
        "contrast" => {
            // k = contrast_percent / 100; hard min 0 (no inversion),
            // unbounded above — the schema's own honest shape.
            let k = (e.float_at("contrast", lt).unwrap_or(100.0) as f32 / 100.0).max(0.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Contrast { k, mix })
        }
        "gamma" => {
            // Hard floor 0.01 keeps 1/gamma finite; no ceiling — the
            // schema's own honest shape.
            let gamma = (e.float_at("gamma", lt).unwrap_or(1.0) as f32).max(0.01);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Gamma { gamma, mix })
        }
        "temperature" => {
            // k = Temperature / 100, clamped to the ±2 hard range (±200). The
            // stronger ±0.75·k gain (K-135) makes full deflection a decisive
            // orange/blue; the gains floor at 0 so an extreme never drives a
            // channel negative. Computed here so the CPU reference and the
            // WGSL kernel multiply by byte-identical f32 factors (§1.6);
            // Temperature 0 → k 0 → gains exactly (1.0, 1.0), the neutral
            // point (the .max(0.0) leaves 1.0 untouched).
            let k = (e.float_at("temperature", lt).unwrap_or(0.0) as f32 / 100.0).clamp(-2.0, 2.0);
            let gain_r = (1.0 + 0.75 * k).max(0.0);
            let gain_b = (1.0 - 0.75 * k).max(0.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Temperature {
                gain_r,
                gain_b,
                mix,
            })
        }
        "invert" => {
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Invert { mix })
        }
        "tint" => {
            // The two mapped colours resolve to scene-linear RGB at frame
            // time (alpha ignored); the CPU reference and the WGSL kernel
            // read the identical numbers.
            let rgb = |id: &str, default: [f64; 4]| -> [f32; 3] {
                let c = e.colour_at(id, lt).unwrap_or(default);
                [c[0] as f32, c[1] as f32, c[2] as f32]
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Tint {
                black: rgb("black", [0.0, 0.0, 0.0, 1.0]),
                white: rgb("white", [1.0, 1.0, 1.0, 1.0]),
                mix,
            })
        }
        "lut" => {
            // Only Mix is Copy-carried; the `.cube` file's parsed cube is a
            // 3D texture threaded beside the resolved op (the caller's LUT
            // cache), exactly as the flow field is for Motion blur. A `lut`
            // effect always resolves to exactly one Resolved::Lut, so the
            // ordered enabled-builtin-`lut` list stays 1:1 and in order with
            // the Resolved::Lut ops — the whole threading contract.
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Lut { mix })
        }
        "dof" => {
            // Scalars only; the depth pass (the referenced layer's rendered
            // texture) is threaded beside the op by the caller, exactly as
            // the LUT cube is. A `dof` effect always resolves to exactly one
            // Resolved::Dof, so the ordered enabled-builtin-`dof` list stays
            // 1:1 and in order with the Dof ops — the threading contract.
            let focus = (e.float_at("focus", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
            let range = (e.float_at("range", lt).unwrap_or(0.1) as f32).clamp(0.0, 1.0);
            // Aperture is the px@comp master; Near/Far are the per-side
            // radii it scales about its default 8 (unity). A pre-feature
            // project has only `aperture` and lacks Near/Far, which then
            // read their default 8, so each side resolves to
            // 8·(aperture/8)·px_scale = aperture·px_scale — identical to the
            // old single-aperture behaviour. px@comp is scaled by the §2.3
            // preview factor so a Half preview blurs the same disc as Full.
            let master = e.float_at("aperture", lt).unwrap_or(8.0) as f32 / 8.0;
            let near = e.float_at("near_aperture", lt).unwrap_or(8.0) as f32;
            let far = e.float_at("far_aperture", lt).unwrap_or(8.0) as f32;
            // Budget cap (docs/13, docs/14): the disc gather is O(coc²) taps
            // per pixel, and the Aperture master MULTIPLIES the per-side radii
            // (so Aperture 150 × Near 55 becomes a ~1000 px circle of
            // confusion), which submits quadrillions of taps and hangs the
            // GPU — freezing the preview that renders on the UI thread. Cap
            // the effective per-side radius so the cost stays bounded;
            // ordinary apertures (≤ the 40 px slider) sit far below it.
            const MAX_APERTURE_PX: f32 = 128.0;
            let near_aperture = (near * master * px_scale).clamp(0.0, MAX_APERTURE_PX);
            let far_aperture = (far * master * px_scale).clamp(0.0, MAX_APERTURE_PX);
            // Depth invert (a plain Bool; absent on pre-feature projects,
            // where it reads false — the historical, unchanged behaviour).
            let depth_invert = matches!(e.param("depth_invert"), Some(EffectValue::Bool(true)));
            // Diagnostic view (clamped to the shipped modes; absent on
            // pre-feature projects → 0 Rendered, the normal output).
            let display = match e.param("display") {
                Some(EffectValue::Choice(c)) => (*c).min(2),
                _ => 0,
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Dof {
                focus,
                range,
                near_aperture,
                far_aperture,
                depth_invert,
                display,
                mix,
            })
        }
        "glow" => {
            // Radius is px@comp (K-135), scaled by the §2.3 preview factor so
            // a Half preview blurs the same halo as Full, only softer.
            let radius = e.float_at("radius", lt).unwrap_or(24.0) as f32;
            let threshold = (e.float_at("threshold", lt).unwrap_or(0.8) as f32).max(0.0);
            let knee = (e.float_at("knee", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
            let intensity = (e.float_at("intensity", lt).unwrap_or(1.0) as f32).max(0.0);
            let tint = e.colour_at("tint", lt).unwrap_or([1.0; 4]);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Glow {
                radius_px: (radius * px_scale).max(0.0),
                threshold,
                knee,
                intensity,
                tint: tint.map(|c| c as f32),
                mix,
            })
        }
        "shake" => {
            let amp_pct = (e.float_at("amplitude", lt).unwrap_or(1.5) as f32).max(0.0);
            let freq = e.float_at("frequency", lt).unwrap_or(8.0).max(0.0);
            let rot_amount = (e.float_at("rotation", lt).unwrap_or(1.0) as f32).max(0.0);
            // Per-axis wobble (twirl group, K-146): amount multipliers scale
            // the master Amplitude, frequency multipliers the master rate.
            // Defaults of 1 reproduce the old uniform x/y shake exactly.
            let x_amp = (e.float_at("x_amp", lt).unwrap_or(1.0) as f32).max(0.0);
            let y_amp = (e.float_at("y_amp", lt).unwrap_or(1.0) as f32).max(0.0);
            let x_freq = e.float_at("x_freq", lt).unwrap_or(1.0).max(0.0);
            let y_freq = e.float_at("y_freq", lt).unwrap_or(1.0).max(0.0);
            let z_freq = e.float_at("z_freq", lt).unwrap_or(1.0).max(0.0);
            // z (depth/scale) amount: the new id, else the old `zoom_pump`
            // (migration — a project saved before FX-11 keeps its pump), a
            // scale-pump per cent either way.
            let z_pct = e
                .float_at("z_amp", lt)
                .or_else(|| e.float_at("zoom_pump", lt))
                .unwrap_or(0.0) as f32;
            let z_amp = (z_pct / 100.0).clamp(0.0, 1.0);
            // Edges (P3, K-145): the new `edge` Choice, else migrate the old
            // Auto-scale bool (on → Repeat hides the border as the cover once
            // did; off → Transparent), else the schema default Repeat.
            let edge = match e.param("edge") {
                Some(EffectValue::Choice(c)) => {
                    EdgesMode::from_code((*c).min(2)).unwrap_or(EdgesMode::Repeat)
                }
                _ => match e.param("auto_scale") {
                    Some(EffectValue::Bool(false)) => EdgesMode::Transparent,
                    _ => EdgesMode::Repeat,
                },
            };
            let seed = match e.param("seed") {
                Some(EffectValue::Seed(s)) => *s,
                _ => 0,
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            // The wobble: independent noise channels sampled at local time ×
            // frequency (per axis, §3.4) — deterministic, hop-free, identical
            // on every machine (§2.4).
            let base = lt * freq;
            let amp_px = (amp_pct / 100.0 * diag_px).max(0.0);
            Some(Resolved::Shake {
                offset_px: [
                    amp_px * x_amp * shake_noise(seed, 0, base * x_freq) as f32,
                    amp_px * y_amp * shake_noise(seed, 1, base * y_freq) as f32,
                ],
                rotation_deg: rot_amount * shake_noise(seed, 2, base) as f32,
                zoom: 1.0 + z_amp * shake_noise(seed, 3, base * z_freq) as f32,
                edge: edge.code(),
                mix,
            })
        }
        "block_glitch" => {
            let intensity = (e.float_at("intensity", lt).unwrap_or(0.35) as f32).clamp(0.0, 1.0);
            let seed = match e.param("seed") {
                Some(EffectValue::Seed(s)) => *s,
                _ => 0,
            };
            // Local time discretised at the fixed tick rate (§3.12
            // status note): block hashing reads this, never raw time.
            let tick = (lt * GLITCH_TICK_HZ).floor() as i32;
            let block_size_px =
                (e.float_at("block_size", lt).unwrap_or(24.0) as f32 * px_scale).max(1.0);
            let jitter_frac =
                (e.float_at("block_jitter", lt).unwrap_or(25.0) as f32 / 100.0).clamp(0.0, 1.0);
            let amount_pct = e.float_at("block_amount", lt).unwrap_or(3.0) as f32;
            let chan_pct = e.float_at("channel_offset", lt).unwrap_or(1.0) as f32;
            let slice_frac =
                (e.float_at("slice_repeat", lt).unwrap_or(20.0) as f32 / 100.0).clamp(0.0, 1.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::BlockGlitch {
                intensity,
                seed,
                tick,
                block_size_px,
                jitter_frac,
                amount_px: (amount_pct / 100.0 * diag_px).max(0.0),
                chan_px: (chan_pct / 100.0 * diag_px).max(0.0),
                slice_frac,
                mix,
            })
        }
        "scanlines" => {
            let intensity = (e.float_at("intensity", lt).unwrap_or(0.35) as f32).clamp(0.0, 1.0);
            let period_px =
                (e.float_at("scanline_period", lt).unwrap_or(3.0) as f32 * px_scale).max(1.0);
            let darkness = (e.float_at("scanline_darkness", lt).unwrap_or(40.0) as f32 / 100.0)
                .clamp(0.0, 1.0);
            let roll_speed = e.float_at("scanline_roll", lt).unwrap_or(0.0);
            // The scanline pattern's pixel offset at this frame (roll
            // speed × local time × period), so the kernel never sees
            // raw time or does its own time maths (§2.4: the CPU/GPU
            // must agree, and f32 time would round differently near a
            // tick boundary than f64 does — precomputing sidesteps it).
            let roll_px = (roll_speed * lt * f64::from(period_px)) as f32;
            let interlace = match e.param("scanline_interlace") {
                Some(EffectValue::Bool(b)) => *b,
                _ => false,
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Scanlines {
                intensity,
                period_px,
                darkness,
                roll_px,
                interlace,
                mix,
            })
        }
        "datamosh" => {
            let intensity = (e.float_at("intensity", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Datamosh { intensity, mix })
        }
        "echo" => {
            // Echoes k = 1..count sit at offset -k with intensity
            // decay^k (v1 fixed one-frame spacing); the render supplies
            // the neighbour frame at each offset. weights[i] is the echo
            // at offset -(i+1).
            let count = (e.float_at("echoes", lt).unwrap_or(4.0).round() as i32).clamp(1, 8);
            let decay = (e.float_at("decay", lt).unwrap_or(0.6) as f32).clamp(0.0, 1.0);
            let mode = match e.param("mode") {
                Some(EffectValue::Choice(c)) => (*c).min(2),
                _ => 1,
            };
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            let mut weights = [0.0f32; 8];
            for (i, w) in weights.iter_mut().enumerate() {
                if (i as i32) < count {
                    *w = decay.powi(i as i32 + 1);
                }
            }
            Some(Resolved::Echo { weights, mode, mix })
        }
        "motion_blur" => {
            // Streak length = motion × (shutter ÷ 360); the flow field
            // (the motion itself) is threaded to the kernel separately.
            // Samples is the spec's integer carried as a Float row —
            // rounded and clamped to the same 2..64 the kernel loops.
            let shutter_frac =
                (e.float_at("shutter_angle", lt).unwrap_or(180.0) as f32 / 360.0).max(0.0);
            let samples = (e.float_at("samples", lt).unwrap_or(16.0).round() as i32).clamp(2, 64);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::MotionBlur {
                shutter_frac,
                samples,
                mix,
            })
        }
        "transform" => {
            // px@comp parameters scale by the preview factor (§2.3) so
            // Half preview frames exactly like Full, only softer.
            let px = |id: &str| e.float_at(id, lt).unwrap_or(0.0) as f32 * px_scale;
            let pct = |id: &str| e.float_at(id, lt).unwrap_or(100.0) as f32 / 100.0;
            let opacity =
                (e.float_at("opacity", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
            Some(Resolved::Transform {
                anchor: [px("anchor_x"), px("anchor_y")],
                position: [px("position_x"), px("position_y")],
                scale: [pct("scale_x"), pct("scale_y")],
                rotation_deg: e.float_at("rotation", lt).unwrap_or(0.0) as f32,
                opacity,
                mix,
            })
        }
        _ => None,
    }
}
