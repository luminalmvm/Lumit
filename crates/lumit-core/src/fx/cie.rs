//! Spectral colour for the Lens flare (docs/impl/lens-flare.md §5, K-256):
//! the CIE (2006) 10° XYZ colour-matching functions at 5 nm steps over the
//! visible range, and the XYZ → linear Rec. 709 conversion into the working
//! space. Data from cvrl.org via the realflare reference (GPLv3, as Lumit is).
//!
//! In plain terms: these three curves say how much each wavelength of light
//! excites the eye's three colour senses — they are what turns "620 nm" into
//! "that red". The flare traces individual wavelengths, and this table is how
//! their energies become the pixels' RGB.

/// First tabulated wavelength, nm.
pub const LAMBDA_MIN: f32 = 390.0;
/// Last tabulated wavelength, nm.
pub const LAMBDA_MAX: f32 = 730.0;
/// The spectrum's midpoint, nm — the reference wavelength for the bakes.
pub const LAMBDA_MID: f32 = (LAMBDA_MIN + LAMBDA_MAX) / 2.0;
/// Table step, nm.
const STEP: f32 = 5.0;

/// CIE XYZ colour-matching values per [`STEP`]-nm row from [`LAMBDA_MIN`].
// The rows are tabulated standard data, quoted at their published precision;
// f32 rounds them once here rather than us pre-rounding the table by hand.
#[allow(clippy::excessive_precision)]
const CIE_XYZ: [[f32; 3]; 69] = [
    [2.952420e-03, 4.076779e-04, 1.318752e-02], // 390 nm
    [7.641137e-03, 1.078166e-03, 3.424588e-02], // 395 nm
    [1.879338e-02, 2.589775e-03, 8.508254e-02], // 400 nm
    [4.204986e-02, 5.474207e-03, 1.927065e-01], // 405 nm
    [8.277331e-02, 1.041303e-02, 3.832822e-01], // 410 nm
    [1.395127e-01, 1.712968e-02, 6.568187e-01], // 415 nm
    [2.077647e-01, 2.576133e-02, 9.933444e-01], // 420 nm
    [2.688989e-01, 3.529554e-02, 1.308674e+00], // 425 nm
    [3.281798e-01, 4.698226e-02, 1.624940e+00], // 430 nm
    [3.693084e-01, 6.047429e-02, 1.867751e+00], // 435 nm
    [4.026189e-01, 7.468288e-02, 2.075946e+00], // 440 nm
    [4.042529e-01, 8.820537e-02, 2.132574e+00], // 445 nm
    [3.932139e-01, 1.039030e-01, 2.128264e+00], // 450 nm
    [3.482214e-01, 1.195389e-01, 1.946651e+00], // 455 nm
    [3.013112e-01, 1.414586e-01, 1.768440e+00], // 460 nm
    [2.534221e-01, 1.701373e-01, 1.582342e+00], // 465 nm
    [1.914176e-01, 1.999859e-01, 1.310576e+00], // 470 nm
    [1.283167e-01, 2.312426e-01, 1.010952e+00], // 475 nm
    [7.593120e-02, 2.682271e-01, 7.516389e-01], // 480 nm
    [3.836770e-02, 3.109438e-01, 5.549619e-01], // 485 nm
    [1.400745e-02, 3.554018e-01, 3.978114e-01], // 490 nm
    [3.446810e-03, 4.148227e-01, 2.905816e-01], // 495 nm
    [5.652072e-03, 4.780482e-01, 2.078158e-01], // 500 nm
    [1.561956e-02, 5.491344e-01, 1.394643e-01], // 505 nm
    [3.778185e-02, 6.248296e-01, 8.852389e-02], // 510 nm
    [7.538941e-02, 7.012292e-01, 5.824484e-02], // 515 nm
    [1.201511e-01, 7.788199e-01, 3.784916e-02], // 520 nm
    [1.756832e-01, 8.376358e-01, 2.431375e-02], // 525 nm
    [2.380254e-01, 8.829552e-01, 1.539505e-02], // 530 nm
    [3.046991e-01, 9.233858e-01, 9.753000e-03], // 535 nm
    [3.841856e-01, 9.665325e-01, 6.083223e-03], // 540 nm
    [4.633109e-01, 9.886887e-01, 3.769336e-03], // 545 nm
    [5.374170e-01, 9.907500e-01, 2.323578e-03], // 550 nm
    [6.230892e-01, 9.997775e-01, 1.426627e-03], // 555 nm
    [7.123849e-01, 9.944304e-01, 8.779264e-04], // 560 nm
    [8.016277e-01, 9.848127e-01, 5.408385e-04], // 565 nm
    [8.933408e-01, 9.640545e-01, 3.342429e-04], // 570 nm
    [9.721304e-01, 9.286495e-01, 2.076129e-04], // 575 nm
    [1.034327e+00, 8.775360e-01, 1.298230e-04], // 580 nm
    [1.106886e+00, 8.370838e-01, 8.183954e-05], // 585 nm
    [1.147304e+00, 7.869950e-01, 5.207245e-05], // 590 nm
    [1.160477e+00, 7.272309e-01, 3.347499e-05], // 595 nm
    [1.148163e+00, 6.629035e-01, 2.175998e-05], // 600 nm
    [1.113846e+00, 5.970375e-01, 1.431231e-05], // 605 nm
    [1.048485e+00, 5.282296e-01, 9.530130e-06], // 610 nm
    [9.617111e-01, 4.601308e-01, 6.426776e-06], // 615 nm
    [8.629581e-01, 3.950755e-01, 0.000000e+00], // 620 nm
    [7.603498e-01, 3.351794e-01, 0.000000e+00], // 625 nm
    [6.413984e-01, 2.751807e-01, 0.000000e+00], // 630 nm
    [5.290979e-01, 2.219564e-01, 0.000000e+00], // 635 nm
    [4.323126e-01, 1.776882e-01, 0.000000e+00], // 640 nm
    [3.496358e-01, 1.410203e-01, 0.000000e+00], // 645 nm
    [2.714900e-01, 1.083996e-01, 0.000000e+00], // 650 nm
    [2.056507e-01, 8.137687e-02, 0.000000e+00], // 655 nm
    [1.538163e-01, 6.033976e-02, 0.000000e+00], // 660 nm
    [1.136072e-01, 4.425383e-02, 0.000000e+00], // 665 nm
    [8.281010e-02, 3.211852e-02, 0.000000e+00], // 670 nm
    [5.954815e-02, 2.302574e-02, 0.000000e+00], // 675 nm
    [4.221473e-02, 1.628841e-02, 0.000000e+00], // 680 nm
    [2.948752e-02, 1.136106e-02, 0.000000e+00], // 685 nm
    [2.025590e-02, 7.797457e-03, 0.000000e+00], // 690 nm
    [1.410230e-02, 5.425391e-03, 0.000000e+00], // 695 nm
    [9.816228e-03, 3.776140e-03, 0.000000e+00], // 700 nm
    [6.809147e-03, 2.619372e-03, 0.000000e+00], // 705 nm
    [4.666298e-03, 1.795595e-03, 0.000000e+00], // 710 nm
    [3.194041e-03, 1.229980e-03, 0.000000e+00], // 715 nm
    [2.205568e-03, 8.499903e-04, 0.000000e+00], // 720 nm
    [1.524672e-03, 5.881375e-04, 0.000000e+00], // 725 nm
    [1.061495e-03, 4.098928e-04, 0.000000e+00], // 730 nm
];

/// The colour-matching triple at wavelength `nm`, linearly interpolated
/// between table rows and zero outside the tabulated range (light the eye
/// cannot see contributes nothing).
pub fn xyz_at(nm: f32) -> [f32; 3] {
    let t = (nm - LAMBDA_MIN) / STEP;
    if t < 0.0 || !t.is_finite() {
        return [0.0; 3];
    }
    let i = t.floor() as usize;
    if i + 1 >= CIE_XYZ.len() {
        return if i < CIE_XYZ.len() {
            CIE_XYZ[i]
        } else {
            [0.0; 3]
        };
    }
    let f = t - i as f32;
    let a = CIE_XYZ[i];
    let b = CIE_XYZ[i + 1];
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// XYZ → linear Rec. 709 / sRGB primaries, D65 white — the compositor's
/// working primaries (deviation D4 in the impl note: realflare targets ACES
/// AP1 instead; ours must match the working space). Row-major.
const XYZ_TO_LINEAR_709: [[f32; 3]; 3] = [
    [3.240_97, -1.537_383, -0.498_611],
    [-0.969_244, 1.875_968, 0.041_555],
    [0.055_63, -0.203_977, 1.056_972],
];

/// Convert one XYZ triple to linear working-space RGB (may go negative for
/// out-of-gamut spectral colours; callers clamp at the edge they need).
pub fn xyz_to_linear_rgb(xyz: [f32; 3]) -> [f32; 3] {
    let m = &XYZ_TO_LINEAR_709;
    [
        m[0][0] * xyz[0] + m[0][1] * xyz[1] + m[0][2] * xyz[2],
        m[1][0] * xyz[0] + m[1][1] * xyz[1] + m[1][2] * xyz[2],
        m[2][0] * xyz[0] + m[2][1] * xyz[1] + m[2][2] * xyz[2],
    ]
}

/// The trace's wavelength ladder (realflare's `wavelength_array`): `count`
/// centred steps across the visible range, so 1 gives the midpoint and 3
/// gives a red / green / blue spread. `dispersion` scales each wavelength's
/// *offset from the midpoint* (docs/08 §3.27's Dispersion dial): 0 collapses
/// the trace onto one wavelength, 2 doubles the fringing. The returned pairs
/// are (traced nm — what the glass sees, true nm — what the eye weights).
pub fn wavelength_ladder(count: usize, dispersion: f32) -> Vec<(f32, f32)> {
    let count = count.max(1);
    (0..count)
        .map(|i| {
            let step = (i as f32 + 0.5) / count as f32;
            let true_nm = LAMBDA_MIN + step * (LAMBDA_MAX - LAMBDA_MIN);
            let traced_nm = LAMBDA_MID + (true_nm - LAMBDA_MID) * dispersion.max(0.0);
            (traced_nm, true_nm)
        })
        .collect()
}
