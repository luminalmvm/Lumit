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
pub const SPECTRAL_BASIS: [[f32; 3]; 9] = [
    [0.112_422_91, 0.0, 0.0],           // 650 nm
    [0.294_590_23, 0.0, 0.0],           // 625 nm
    [0.365_333_56, 0.036_021_75, 0.0],  // 600 nm
    [0.201_592_3, 0.192_775_3, 0.0],    // 575 nm
    [0.0, 0.311_754_2, 0.0],            // 550 nm
    [0.0, 0.300_619_63, 0.0],           // 525 nm
    [0.0, 0.134_424_22, 0.068_714_05],  // 500 nm
    [0.0, 0.024_404_911, 0.339_951_04], // 475 nm
    [0.026_061_023, 0.0, 0.591_334_94], // 450 nm — the violet re-red bump
];

/// [`SPECTRAL_BASIS`] as vec4 rows (w zero) for the GPU uniform — the
/// kernel reads the very same numbers the CPU reference does.
pub fn spectral_basis_vec4() -> [[f32; 4]; 9] {
    let mut out = [[0.0; 4]; 9];
    for (dst, src) in out.iter_mut().zip(SPECTRAL_BASIS.iter()) {
        dst[..3].copy_from_slice(src);
    }
    out
}
