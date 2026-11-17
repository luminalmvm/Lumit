//! Learn measures the noise that is there.

use super::atrous::level_of;
use super::learn::learn;
use super::space::ycc_to_linear;

/// Normal noise, deterministic: Box-Muller over a small xorshift.
struct Gauss(u64);

impl Gauss {
    fn uniform(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }

    fn next(&mut self) -> f32 {
        let (a, b) = (self.uniform(), self.uniform());
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
    }
}

/// One plane of unit noise, blurred by `blur` B3 passes (0 is white), scaled
/// back to a measured sigma of exactly 1.
fn plane(g: &mut Gauss, w: u32, h: u32, blur: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..w * h).map(|_| g.next()).collect();
    for _ in 0..blur {
        v = level_of::<1>(&v, w, h, 1);
    }
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    let sd = (v.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / v.len() as f32).sqrt();
    v.iter().map(|x| (x - mean) / sd).collect()
}

/// A picture of `base` Y'CbCr per pixel plus noise of `luma` and `chroma`
/// code values, as premultiplied linear RGBA.
fn noisy(
    w: u32,
    h: u32,
    base: impl Fn(u32, u32) -> [f32; 3],
    luma: f32,
    chroma: f32,
    blur: usize,
) -> Vec<f32> {
    let mut g = Gauss(0x2545_F491_4F6C_DD1D);
    let (ny, nb, nr) = (
        plane(&mut g, w, h, blur),
        plane(&mut g, w, h, blur),
        plane(&mut g, w, h, blur),
    );
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let [by, bb, br] = base(x, y);
            let [r, gr, b] = ycc_to_linear([
                by + ny[i] * luma / 255.0,
                bb + nb[i] * chroma / 255.0,
                br + nr[i] * chroma / 255.0,
            ]);
            out.extend_from_slice(&[r, gr, b, 1.0]);
        }
    }
    out
}

fn close(got: f32, want: f32, what: &str) {
    assert!(
        (got - want).abs() <= want * 0.1,
        "{what}: measured {got}, made {want}"
    );
}

#[test]
fn learn_measures_white_and_blurred_noise_on_a_flat_patch() {
    for blur in [0, 1] {
        let (w, h) = (256, 256);
        let rgba = noisy(w, h, |_, _| [0.45, 0.0, 0.0], 4.0, 3.0, blur);
        let got = learn(&rgba, w, h, Some([128.0, 128.0, 160.0]));
        eprintln!("blur {blur}: {got:?}");
        close(got.luma_noise, 4.0, "luma");
        close(got.chroma_noise, 3.0, "chroma");
        assert!(got.quality > 0.8, "a flat patch is a good one: {got:?}");
    }
}

#[test]
fn learn_finds_the_flat_part_of_a_textured_picture() {
    for blur in [0, 1] {
        let (w, h) = (320, 192);
        // Stripes and a ramp across the left half, flat on the right.
        let flat = |x: u32, _y: u32| x >= 160;
        let base = move |x: u32, y: u32| {
            if flat(x, y) {
                [0.5, 0.0, 0.0]
            } else {
                let s = (x as f32 * 0.4).sin() * (y as f32 * 0.23).cos();
                [0.4 + 0.25 * s + 0.0008 * x as f32, 0.05 * s, -0.04 * s]
            }
        };
        let rgba = noisy(w, h, base, 5.0, 2.5, blur);
        let got = learn(&rgba, w, h, None);
        eprintln!("blur {blur}: {got:?}");
        let [cx, cy, side] = got.sample;
        let half = side * 0.5;
        assert!(
            cx - half >= 160.0 && cy - half >= 0.0 && cy + half <= h as f32,
            "the sample is on the flat half: {:?}",
            got.sample
        );
        close(got.luma_noise, 5.0, "luma");
        close(got.chroma_noise, 2.5, "chroma");
    }
}
