//! The LUT the LUT effect's picture is graded with.

/// A small 3D LUT for the LUT effect's picture: cool shadows, warm highlights,
/// which is the grade everyone recognises and the one a `.cube` is usually
/// bought to apply. Written as text, because a `.cube` is text.
pub(super) fn write_example_cube(path: &std::path::Path) -> std::io::Result<()> {
    const N: usize = 17;
    let mut out = String::from(
        "TITLE \"Lumit manual example\"
LUT_3D_SIZE 17

",
    );
    for b in 0..N {
        for g in 0..N {
            for r in 0..N {
                let (rf, gf, bf) = (
                    r as f32 / (N - 1) as f32,
                    g as f32 / (N - 1) as f32,
                    b as f32 / (N - 1) as f32,
                );
                let luma = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
                // Warmth rises with brightness, coolness falls away with it.
                let warm = luma * luma;
                let cool = (1.0 - luma) * (1.0 - luma);
                let clamp = |v: f32| v.clamp(0.0, 1.0);
                out.push_str(&format!(
                    "{:.6} {:.6} {:.6}
",
                    clamp(rf * (1.0 + 0.55 * warm) - 0.10 * cool),
                    clamp(gf * (1.0 + 0.10 * warm) - 0.04 * cool),
                    clamp(bf * (1.0 + 0.70 * cool) - 0.22 * warm),
                ));
            }
        }
    }
    std::fs::write(path, out)
}
