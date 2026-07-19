/// The **constant-luminance** hue-rotation matrix for `deg` degrees, row-major
/// (docs/08 §3.17). Rec.709 luma weights keep perceived brightness fixed as the
/// hue turns — the standard SVG `feColorMatrix` hue-rotate — so this is the Hue
/// shift effect's Preserve-luminance mode (K-136, on by default). Computed
/// host-side (f64 then cast) so the CPU reference and the WGSL kernel multiply
/// by the identical `f32` coefficients. Exactly the identity at 0°, so the
/// effect's neutral point is bit-exact.
pub fn hue_matrix(deg: f64) -> [f32; 9] {
    if deg % 360.0 == 0.0 {
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let (s, c) = deg.to_radians().sin_cos();
    // Rec.709 luma; the standard SVG feColorMatrix hue-rotate coefficients.
    let (lr, lg, lb) = (0.2126_f64, 0.7152, 0.0722);
    [
        (lr + c * (1.0 - lr) - s * lr) as f32,
        (lg - c * lg - s * lg) as f32,
        (lb - c * lb + s * (1.0 - lb)) as f32,
        (lr - c * lr + s * 0.143) as f32,
        (lg + c * (1.0 - lg) + s * 0.140) as f32,
        (lb - c * lb - s * 0.283) as f32,
        (lr - c * lr - s * (1.0 - lr)) as f32,
        (lg - c * lg + s * lg) as f32,
        (lb + c * (1.0 - lb) + s * lb) as f32,
    ]
}

/// The **plain-RGB** hue-rotation matrix for `deg` degrees, row-major (docs/08
/// §3.17, K-136): a geometric rotation of the RGB vector about the neutral grey
/// axis `(1,1,1)/√3` (Rodrigues' formula). It is *not* luminance-weighted, so it
/// preserves the raw R+G+B sum rather than perceived brightness — a saturated
/// colour may brighten or dim as its hue turns, the plain colour-wheel spin.
/// This is the Hue shift effect's Preserve-luminance-off mode. Computed host-side
/// (f64 then cast) so the CPU reference and the WGSL kernel multiply by identical
/// `f32` coefficients. Exactly the identity at 0°, like [`hue_matrix`], so the
/// neutral point stays bit-exact; every row and column sums to 1, so a neutral
/// grey stays grey.
pub fn hue_matrix_rgb(deg: f64) -> [f32; 9] {
    if deg % 360.0 == 0.0 {
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let (s, c) = deg.to_radians().sin_cos();
    // Rotation about n = (1,1,1)/√3: R = c·I + (1−c)/3·J + (s/√3)·[n]×,
    // with J the all-ones matrix and [n]× the cross-product (skew) matrix.
    let a = (1.0 - c) / 3.0;
    let b = s / 3.0_f64.sqrt();
    [
        (c + a) as f32,
        (a - b) as f32,
        (a + b) as f32,
        (a + b) as f32,
        (c + a) as f32,
        (a - b) as f32,
        (a - b) as f32,
        (a + b) as f32,
        (c + a) as f32,
    ]
}

/// The inverse affine of a Transform effect (docs/08 §3.5): the forward map
/// is `p_out = position + R(rotation) · S(scale) · (p_in − anchor)` — the
/// layer transform's own shape — so each output pixel centre `p` samples the
/// input at `q = m·p + o` with `m = S⁻¹·R⁻¹` (row-major 2×2) and
/// `o = anchor − m·position`. Host-computed so the WGSL kernel never runs
/// its own trigonometry (its `cos`/`sin` are not correctly rounded) and the
/// CPU reference consumes bit-identical numbers. `None` when a scale axis is
/// degenerate (|s| < 1e-6): the image has collapsed to nothing and renders
/// fully transparent — never a division blow-up (docs/14 no-panic rule).
pub fn transform_inverse(
    anchor: [f32; 2],
    position: [f32; 2],
    scale: [f32; 2],
    rotation_deg: f32,
) -> Option<([f32; 4], [f32; 2])> {
    if scale[0].abs() < 1e-6 || scale[1].abs() < 1e-6 {
        return None;
    }
    let rad = (rotation_deg as f64).to_radians();
    let (sin, cos) = (rad.sin() as f32, rad.cos() as f32);
    let m = [
        cos / scale[0],
        sin / scale[0],
        -sin / scale[1],
        cos / scale[1],
    ];
    let o = [
        anchor[0] - (m[0] * position[0] + m[1] * position[1]),
        anchor[1] - (m[2] * position[0] + m[3] * position[1]),
    ];
    Some((m, o))
}

/// [`transform_inverse`] folded with the degenerate case, as the GPU op
/// ingredients `(m, offset, effective opacity)`: a zero-scale transform
/// maps to an identity matrix with opacity 0 — fully transparent. The CPU
/// reference and both render paths all build from this one function, so
/// every path consumes bit-identical numbers.
pub fn transform_op(
    anchor: [f32; 2],
    position: [f32; 2],
    scale: [f32; 2],
    rotation_deg: f32,
    opacity: f32,
) -> ([f32; 4], [f32; 2], f32) {
    match transform_inverse(anchor, position, scale, rotation_deg) {
        Some((m, o)) => (m, o, opacity),
        None => ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0], 0.0),
    }
}

/// The glow bright pass on one channel (docs/08 §3.3 step 1):
/// `max(0, x − threshold)` with a soft knee — the hinge's onset is weighted
/// by a smoothstep over `threshold ± knee`, so the bloom fades in over the
/// knee width instead of snapping on at the threshold. Knee 0 is the hard
/// subtract. Written with the exact arithmetic order the WGSL kernel uses
/// (§1.6: both paths must agree), and shared with the CPU reference below.
pub fn glow_bright(x: f32, threshold: f32, knee: f32) -> f32 {
    let d = x - threshold;
    if d <= 0.0 {
        return 0.0;
    }
    if knee > 0.0 {
        let t = ((x - (threshold - knee)) / (2.0 * knee)).clamp(0.0, 1.0);
        let w = t * t * (3.0 - 2.0 * t);
        return d * w;
    }
    d
}

/// The SplitMix64 finaliser — the integer mixer behind the shake noise
/// lattice. Chosen for its published avalanche quality and its five-line
/// portability: any future twin (a WGSL noise, an expression binding)
/// can reproduce it exactly.
fn splitmix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

/// The lattice value for one noise channel at integer coordinate `i`:
/// `hash(seed, channel, i)` mapped to [−1, 1] — the §2.4 seeded-stateless
/// shape, a pure function of its inputs with no history.
fn noise_lattice(seed: u32, channel: u32, i: i64) -> f64 {
    let mixed = splitmix64(splitmix64(splitmix64(u64::from(seed)) ^ u64::from(channel)) ^ i as u64);
    // Top 53 bits → [0, 1) exactly representable in f64, then to [−1, 1].
    (mixed >> 11) as f64 * (2.0 / 9_007_199_254_740_992.0) - 1.0
}

/// One octave of seeded 1D value noise: the lattice values at the two
/// surrounding integers, smoothstep-interpolated. C¹-continuous, so the
/// wobble it drives is hop-free; deterministic per §2.4 (same inputs, same
/// output, on every machine and every run).
pub fn value_noise_1d(seed: u32, channel: u32, x: f64) -> f64 {
    let x0 = x.floor();
    let i0 = x0 as i64; // saturating cast: astronomically distant times clamp
    let f = x - x0;
    let t = f * f * (3.0 - 2.0 * f);
    let a = noise_lattice(seed, channel, i0);
    let b = noise_lattice(seed, channel, i0.wrapping_add(1));
    a + (b - a) * t
}

/// The Shake generator (docs/08 §3.4): two octaves of value noise (the
/// sketch's "Normal" fBm — lacunarity 2, gain 0.5, octaves decorrelated by
/// channel offset), normalised so the result stays within [−1, 1]. One
/// independent channel each for x, y, rotation and zoom.
pub fn shake_noise(seed: u32, channel: u32, x: f64) -> f64 {
    (value_noise_1d(seed, channel, x) + 0.5 * value_noise_1d(seed, channel + 4, x * 2.0)) / 1.5
}

/// The time-independent configuration of one shake's wobble (docs/08 §3.4):
/// the seed, the per-axis amplitudes (already in raster pixels for x/y and a
/// 0..1 scale-pump for z) and the per-axis frequency multipliers. `at(base)`
/// samples the wobble at a noise base — local time × the master frequency —
/// returning the `(offset_px, rotation_deg, zoom)` ingredients
/// [`shake_affine`] turns into a transform. One sampler serves both the
/// frame-time wobble and the motion-blur sub-frames (T18), so a shake and its
/// own motion blur are drawn from bit-identical numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShakeWobble {
    pub seed: u32,
    /// Master amplitude in raster pixels (% comp diagonal × the preview factor).
    pub amp_px: f32,
    pub x_amp: f32,
    pub y_amp: f32,
    /// Peak rotation wobble in degrees.
    pub rot_amount: f32,
    /// Depth (z) scale-pump magnitude, 0..1.
    pub z_amp: f32,
    pub x_freq: f64,
    pub y_freq: f64,
    pub z_freq: f64,
}

impl ShakeWobble {
    /// The wobble at noise base `base`: `(offset_px, rotation_deg, zoom)`. The
    /// arithmetic order matches the historical inline resolve, so an unchanged
    /// shake stays bit-for-bit itself.
    pub fn at(self, base: f64) -> ([f32; 2], f32, f32) {
        (
            [
                self.amp_px * self.x_amp * shake_noise(self.seed, 0, base * self.x_freq) as f32,
                self.amp_px * self.y_amp * shake_noise(self.seed, 1, base * self.y_freq) as f32,
            ],
            self.rot_amount * shake_noise(self.seed, 2, base) as f32,
            1.0 + self.z_amp * shake_noise(self.seed, 3, base * self.z_freq) as f32,
        )
    }
}

/// The fixed number of sub-frame samples the shake's own motion blur averages
/// (T18, K-165): odd, so the centre sample lands exactly on the frame time.
/// A fixed count and order keep the smear deterministic (docs/14 §3), and the
/// value is small because a shake moves little — a Cheap effect stays cheap.
pub const SHAKE_MB_SAMPLES: usize = 9;

/// The motion-blur shutter's full width in the noise base domain at amount 1
/// (T18, K-165). The wobble is a pure function of time, so its motion blur
/// samples it across `± SHAKE_MB_SPAN_BASE · amount / 2` around the frame's
/// base. The window is expressed in **base units** (local time × frequency),
/// not seconds: this keeps the smear frame-rate independent and needs no fps
/// in the frame-rate-agnostic effect resolver. A faster axis (higher frequency
/// multiplier) advances further through its noise over the same window, so it
/// smears more — the shake's own inter-frame movement, scaled by amount.
pub const SHAKE_MB_SPAN_BASE: f64 = 1.0;

/// The signed base offsets of the motion-blur sub-frames across the shutter
/// (T18, K-165), symmetric about 0 so the centre sample is the frame itself.
/// `amount` is the shutter fraction, clamped to 0..1.
pub fn shake_mb_offsets(amount: f64) -> [f64; SHAKE_MB_SAMPLES] {
    let window = amount.clamp(0.0, 1.0) * SHAKE_MB_SPAN_BASE;
    let last = (SHAKE_MB_SAMPLES - 1) as f64;
    let mut out = [0.0f64; SHAKE_MB_SAMPLES];
    for (k, o) in out.iter_mut().enumerate() {
        *o = (k as f64 / last - 0.5) * window;
    }
    out
}

/// A 32-bit avalanche mixer, in the same five-line-portability spirit as
/// [`splitmix64`] above (public-domain "splitmix32" shape: golden-ratio
/// increment, xorshift/multiply/xorshift finalisation) — Glitch's per-block
/// hash (docs/08 §3.12 status note) needs this narrower sibling because the
/// block index is a *per-pixel* quantity the WGSL kernel must hash itself
/// (there are too many blocks to precompute a host-side table into the
/// uniform), and WGSL has no 64-bit integer type to host the real
/// splitmix64 lattice. Both the CPU reference and the kernel run this exact
/// sequence of wrapping u32 ops, so they agree on the integer hash
/// bit-for-bit; Shake's splitmix64/[`value_noise_1d`] are untouched.
fn splitmix32(mut x: u32) -> u32 {
    x = x.wrapping_add(0x9e37_79b9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x21f0_aaad);
    x ^= x >> 15;
    x = x.wrapping_mul(0x735a_2d97);
    x ^= x >> 15;
    x
}

/// One Glitch per-block hash channel (docs/08 §3.12 status note): folds
/// `(seed, channel, block x, block y, tick)` through [`splitmix32`] and
/// maps the top 24 bits to `[0, 1)` — exactly representable in f32/f64, so
/// CPU and GPU read the identical value. Discrete and unfiltered on
/// purpose (unlike [`value_noise_1d`]'s smooth interpolation): adjacent
/// blocks are meant to be independent draws, and "tick" is the tick-rate
/// discretisation of local time that gives block glitching its per-frame
/// pop rather than a continuous wobble.
pub fn block_hash01(seed: u32, channel: u32, bx: i32, by: i32, tick: i32) -> f32 {
    let mut h = seed;
    h = splitmix32(h ^ channel);
    h = splitmix32(h ^ (bx as u32));
    h = splitmix32(h ^ (by as u32));
    h = splitmix32(h ^ (tick as u32));
    (h >> 8) as f32 / 16_777_216.0 // top 24 bits, /2^24 → exact in f32
}

/// Glitch's fixed, unexposed block-glitch update rate (docs/08 §3.12
/// status note): the spec's "time-derived tick" without a listed rate
/// parameter, pinned as an internal constant — fast enough that block
/// glitching reads as chaotic, slow enough that individual pops stay
/// visible instead of blurring into continuous noise.
pub const GLITCH_TICK_HZ: f64 = 8.0;

/// A resolved Shake as the transform-effect ingredients it dispatches as
/// (docs/08 §3.4: a transform-domain effect — perturb a virtual camera,
/// resample once): `(anchor, position, scale, rotation)` for
/// [`transform_op`] / [`cpu::transform`], wobbling about the frame centre.
/// Both the CPU reference and the GPU path build from this one function,
/// so every path consumes bit-identical numbers. The revealed border is the
/// caller's Edges policy (P3, K-145), applied by the resample itself — there
/// is no cover-scale any more (FX-11/K-146 dropped Auto-scale).
pub fn shake_affine(
    w: u32,
    h: u32,
    offset_px: [f32; 2],
    rotation_deg: f32,
    zoom: f32,
) -> ([f32; 2], [f32; 2], [f32; 2], f32) {
    let centre = [w as f32 * 0.5, h as f32 * 0.5];
    (
        centre,
        [centre[0] + offset_px[0], centre[1] + offset_px[1]],
        [zoom, zoom],
        rotation_deg,
    )
}

/// The linear-mode channel offset vector for an RGB split: `amount_px`
/// along `angle_deg`. Shared by the CPU reference and the GPU op
/// construction so both paths carry the same host-computed sines (WGSL's
/// `cos`/`sin` are not correctly rounded, so the kernel never computes its
/// own).
pub fn rgb_split_offset(amount_px: f32, angle_deg: f32) -> (f32, f32) {
    let rad = angle_deg.to_radians();
    (amount_px * rad.cos(), amount_px * rad.sin())
}

/// The wavelength → linear-sRGB basis behind the RGB split's Wavelength
/// mode (docs/08 §3.6, K-090): nine taps across the visible spectrum. Tap
/// `i` sits at spectral fraction `t = i/4 − 1`, sampling `position +
/// t·offset` — so the red end (t = −1, 650 nm) lands where the classic
/// mode's R samples and the blue end (t = +1, 450 nm) where its B does,
/// and the two modes disperse in the same direction. Derived offline: CIE
/// 1931 x̄ȳz̄ via the Wyman et al. (2013) multi-lobe Gaussian fits at
/// 650–450 nm in 25 nm steps, through the sRGB D65 matrix, negatives
/// clipped, then each channel's column normalised to sum 1 (within one
/// f32 ULP) so a uniform image passes through unchanged. The CPU reference
/// reads this table directly and the WGSL kernel receives it in its
/// uniform, so both paths consume bit-identical numbers.
/// The three picker colours sampled as a smooth gradient at offset fraction
/// `t ∈ [-1, +1]` (A1/K-163): `tints[0]` at −1, `tints[1]` at 0, `tints[2]`
/// at +1, linearly interpolated between the stops. This gradient replaces the
/// old fixed physical spectral basis, so the three-colour picker now drives the
/// Wavelength dispersion (the owner's choice): the default red / green / blue
/// tints give a red→green→blue fringe.
pub fn tint_gradient(tints: [[f32; 3]; 3], t: f64) -> [f64; 3] {
    let (a, b, f) = if t <= 0.0 {
        (tints[0], tints[1], t + 1.0) // [-1, 0] → f in [0, 1]
    } else {
        (tints[1], tints[2], t) // [0, 1]
    };
    [
        f64::from(a[0]) * (1.0 - f) + f64::from(b[0]) * f,
        f64::from(a[1]) * (1.0 - f) + f64::from(b[1]) * f,
        f64::from(a[2]) * (1.0 - f) + f64::from(b[2]) * f,
    ]
}

/// The largest number of spectral samples the Wavelength dispersion supports
/// (docs/08 §3.6, K-090/K-144): the GPU uniform carries a fixed-size basis
/// array, so the sample count clamps here — the same bounded-cost shape Motion
/// blur's 64-tap cap uses. High enough that a heavy dispersion reads as a
/// smooth rainbow rather than a few discrete stacked copies.
pub const SPECTRAL_MAX_SAMPLES: i32 = 64;

/// Build `samples` spectral taps for the Wavelength dispersion (docs/08 §3.6,
/// K-090; the "more samples" refinement K-144; picker-driven since A1/K-163).
/// Each entry is `[r_weight, g_weight, b_weight, fraction]`: the RGB weight to
/// multiply the tap's sample by, and the offset **fraction** in `[-1, +1]`
/// (−1 = the `tints[0]` end, +1 = the `tints[2]` end). The colour column at each
/// tap is the three-colour picker sampled as a gradient ([`tint_gradient`]),
/// then each colour column is normalised to sum 1 across the taps, so a uniform
/// image still passes through unchanged (the dispersion tints the fringe, never
/// the exposure). More taps simply fill the same `±offset` span more densely, so
/// a large offset disperses smoothly instead of showing a few discrete copies.
/// Computed host-side in f64 then cast, so the CPU reference and the WGSL kernel
/// (which reads the fraction straight from each tap's `w`) consume bit-identical
/// `f32` numbers. `samples` is clamped to `3..=SPECTRAL_MAX_SAMPLES` so the
/// middle stop (`tints[1]`) is always represented.
pub fn spectral_taps(samples: i32, tints: [[f32; 3]; 3]) -> Vec<[f32; 4]> {
    let n = samples.clamp(3, SPECTRAL_MAX_SAMPLES) as usize;
    let mut taps: Vec<[f64; 4]> = Vec::with_capacity(n);
    for i in 0..n {
        let t = -1.0 + 2.0 * i as f64 / (n - 1) as f64;
        let c = tint_gradient(tints, t);
        taps.push([c[0], c[1], c[2], t]);
    }
    // Normalise each colour column to sum 1 (uniform-image preservation).
    for c in 0..3 {
        let sum: f64 = taps.iter().map(|t| t[c]).sum();
        if sum > 0.0 {
            for t in taps.iter_mut() {
                t[c] /= sum;
            }
        }
    }
    taps.iter()
        .map(|t| [t[0] as f32, t[1] as f32, t[2] as f32, t[3] as f32])
        .collect()
}

/// [`spectral_taps`] packed into the fixed-size GPU uniform array plus its
/// active `count` — the kernel loops `0..count`, reading each tap's weight and
/// its offset fraction (the `w` lane). Entries beyond `count` are zero. Shares
/// [`spectral_taps`] with the CPU reference, so both paths consume identical
/// numbers.
pub fn spectral_basis_uniform(
    samples: i32,
    tints: [[f32; 3]; 3],
) -> ([[f32; 4]; SPECTRAL_MAX_SAMPLES as usize], u32) {
    let taps = spectral_taps(samples, tints);
    let mut out = [[0.0f32; 4]; SPECTRAL_MAX_SAMPLES as usize];
    for (dst, src) in out.iter_mut().zip(taps.iter()) {
        *dst = *src;
    }
    (out, taps.len() as u32)
}
