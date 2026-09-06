//! The three Sony Pictures Imageworks table formats, `.spi1d`, `.spi3d` and
//! `.spimtx`.
//!
//! In plain terms: these are the plain-text table files the widely-used legacy
//! ACES configs are built from — the reason those configs need no code-named
//! built-in transforms at all. All three are a tiny header, or none at all,
//! followed by numbers. The one detail worth stating twice is that `.spi1d`
//! declares its **input domain** on a `From` line and it is frequently not
//! `0 1`; assuming `0 1` silently squashes the curve, which looks like a
//! grading error rather than a parsing one.

use crate::error::{ColourError, Result};
use crate::matrix::Matrix34;
use crate::sample::{Cube, Curve};

fn bad(what: &str, reason: impl Into<String>) -> ColourError {
    ColourError::Parse {
        what: what.to_string(),
        reason: reason.into(),
    }
}

/// Parse `.spi1d` text into a curve.
///
/// Header keys are `Version`, `From <lo> <hi>`, `Length` and `Components`;
/// braces around the data block are ignored. One component means one curve
/// applied to all three channels, three means one per channel.
pub fn parse_spi1d(what: &str, text: &str) -> Result<Curve> {
    let mut domain = [0.0_f32, 1.0];
    let mut length: Option<usize> = None;
    let mut components: usize = 1;
    let mut data: Vec<[f32; 3]> = Vec::new();
    let mut in_data = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "{" {
            in_data = true;
            continue;
        }
        if line == "}" {
            in_data = false;
            continue;
        }
        let mut tokens = line.split_whitespace();
        let head = tokens.next().unwrap_or_default();
        match head {
            "Version" => continue,
            "From" => {
                let lo = tokens.next().and_then(|t| t.parse::<f32>().ok());
                let hi = tokens.next().and_then(|t| t.parse::<f32>().ok());
                match (lo, hi) {
                    (Some(lo), Some(hi)) => domain = [lo, hi],
                    _ => return Err(bad(what, format!("could not read the domain in {line:?}"))),
                }
            }
            "Length" => {
                length = Some(
                    tokens
                        .next()
                        .and_then(|t| t.parse::<usize>().ok())
                        .ok_or_else(|| {
                            bad(what, format!("could not read the length in {line:?}"))
                        })?,
                );
            }
            "Components" => {
                components = tokens
                    .next()
                    .and_then(|t| t.parse::<usize>().ok())
                    .ok_or_else(|| {
                        bad(what, format!("could not read the components in {line:?}"))
                    })?;
                if components == 0 || components > 3 {
                    return Err(bad(
                        what,
                        format!("a curve may have 1 to 3 components, not {components}"),
                    ));
                }
            }
            _ => {
                if !in_data && head.parse::<f32>().is_err() {
                    // An unknown header key: skip it rather than refuse, so a
                    // newer writer's extra line does not break an old file.
                    continue;
                }
                let mut values = Vec::with_capacity(3);
                for token in line.split_whitespace() {
                    let v = token
                        .parse::<f32>()
                        .map_err(|_| bad(what, format!("could not read the number {token:?}")))?;
                    values.push(v);
                }
                if values.len() != components {
                    return Err(bad(
                        what,
                        format!(
                            "a data row must have {components} values, found {}",
                            values.len()
                        ),
                    ));
                }
                let sample = match values.as_slice() {
                    [v] => [*v; 3],
                    [r, g, b] => [*r, *g, *b],
                    // Two components has no meaning in the format; the check
                    // above rejects anything but 1 or 3 in practice.
                    _ => return Err(bad(what, "a data row must have 1 or 3 values")),
                };
                data.push(sample);
            }
        }
    }

    if let Some(n) = length {
        if data.len() != n {
            return Err(bad(
                what,
                format!(
                    "the header declares {n} samples but the file has {}",
                    data.len()
                ),
            ));
        }
    }
    Curve::new(what, domain, data)
}

/// Parse `.spi3d` text into a cube.
///
/// After the `SPILUT` line come the input/output dimensions, the three grid
/// sizes, and then one line per sample: three integer grid indices followed by
/// the three output values. The indices are read from the file rather than
/// assumed, and the samples land in Lumit's red-fastest order.
pub fn parse_spi3d(what: &str, text: &str) -> Result<Cube> {
    let mut sizes: Option<[usize; 3]> = None;
    let mut seen_dimensions = false;
    let mut data: Vec<[f32; 3]> = Vec::new();
    let mut filled: Vec<bool> = Vec::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("SPILUT") {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if sizes.is_none() {
            if !seen_dimensions {
                // The `3 3` line: how many channels in, how many out.
                seen_dimensions = true;
                continue;
            }
            let parsed: Vec<usize> = tokens
                .iter()
                .filter_map(|t| t.parse::<usize>().ok())
                .collect();
            let [r, g, b] = parsed.as_slice() else {
                return Err(bad(
                    what,
                    format!("could not read the grid sizes in {line:?}"),
                ));
            };
            if r != g || g != b {
                return Err(bad(
                    what,
                    format!("Lumit reads cubes with equal sides; this one is {r}×{g}×{b}"),
                ));
            }
            let n = *r;
            if n < 2 {
                return Err(bad(
                    what,
                    format!("a cube needs at least 2 points per axis, not {n}"),
                ));
            }
            if n > crate::sample::MAX_CUBE_SIZE {
                return Err(ColourError::TableTooLarge {
                    what: what.to_string(),
                    size: n,
                    limit: crate::sample::MAX_CUBE_SIZE,
                });
            }
            data = vec![[0.0_f32; 3]; n * n * n];
            filled = vec![false; n * n * n];
            sizes = Some([n, n, n]);
            continue;
        }

        let Some([n, _, _]) = sizes else { continue };
        let [ri, gi, bi, r, g, b] = tokens.as_slice() else {
            return Err(bad(
                what,
                format!("a sample line needs six values: {line:?}"),
            ));
        };
        let idx = |t: &str| -> Result<usize> {
            let v = t
                .parse::<usize>()
                .map_err(|_| bad(what, format!("could not read the index {t:?}")))?;
            if v >= n {
                return Err(bad(
                    what,
                    format!("the index {v} is outside a {n}-point grid"),
                ));
            }
            Ok(v)
        };
        let val = |t: &str| -> Result<f32> {
            t.parse::<f32>()
                .map_err(|_| bad(what, format!("could not read the number {t:?}")))
        };
        let (ri, gi, bi) = (idx(ri)?, idx(gi)?, idx(bi)?);
        let sample = [val(r)?, val(g)?, val(b)?];
        let flat = ri + gi * n + bi * n * n;
        if let (Some(slot), Some(seen)) = (data.get_mut(flat), filled.get_mut(flat)) {
            *slot = sample;
            *seen = true;
        }
    }

    let Some([n, _, _]) = sizes else {
        return Err(bad(what, "the file states no grid size"));
    };
    if filled.iter().any(|f| !f) {
        let missing = filled.iter().filter(|f| !**f).count();
        return Err(bad(
            what,
            format!(
                "{missing} of the cube's {} samples are missing",
                filled.len()
            ),
        ));
    }
    Cube::new(what, n, [0.0; 3], [1.0; 3], data)
}

/// Parse `.spimtx` text into a 3×4 matrix.
///
/// The whole file is twelve numbers, three rows of four, and there is no header
/// at all. The fourth number of each row is an offset written as a 16-bit code
/// value, so it is divided by 65535 to reach the 0 to 1 range everything else
/// here works in. Getting that divide wrong moves a picture by a whole stop and
/// upwards, which is why it is the one line worth pointing at.
pub fn parse_spimtx(what: &str, text: &str) -> Result<Matrix34> {
    let mut values: Vec<f64> = Vec::with_capacity(12);
    for token in text.split_whitespace() {
        values.push(
            token
                .parse::<f64>()
                .map_err(|_| bad(what, format!("could not read the number {token:?}")))?,
        );
    }
    let mut m: Matrix34 = values.try_into().map_err(|v: Vec<f64>| {
        bad(
            what,
            format!(
                "a matrix file holds twelve numbers, this one has {}",
                v.len()
            ),
        )
    })?;
    for i in [3, 7, 11] {
        m[i] /= 65535.0;
    }
    Ok(m)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_one_component_curve_applies_to_all_three_channels() {
        let text = "Version 1\nFrom 0.0 1.0\nLength 3\nComponents 1\n{\n0.0\n0.25\n1.0\n}\n";
        let curve = parse_spi1d("t.spi1d", text).expect("parses");
        assert_eq!(curve.domain, [0.0, 1.0]);
        assert_eq!(curve.data.len(), 3);
        assert_eq!(curve.data.get(1).copied(), Some([0.25; 3]));
    }

    #[test]
    fn the_from_line_is_read_rather_than_assumed() {
        // The trap named in docs/impl/ocio.md §4.3: a log curve's domain is
        // routinely not 0..1, and assuming it is squashes the whole curve.
        let text = "Version 1\nFrom -0.125 1.5\nLength 2\nComponents 3\n{\n0 0 0\n1 1 1\n}\n";
        let curve = parse_spi1d("t.spi1d", text).expect("parses");
        assert_eq!(curve.domain, [-0.125, 1.5]);
        // Halfway along the declared domain is halfway along the curve.
        let mid = -0.125 + (1.5 - -0.125) * 0.5;
        let got = curve.sample([mid; 3]);
        assert!((got[0] - 0.5).abs() < 1e-5, "{got:?}");
    }

    #[test]
    fn a_curve_whose_length_lies_is_refused() {
        let text = "Version 1\nFrom 0 1\nLength 5\nComponents 1\n{\n0.0\n1.0\n}\n";
        assert!(parse_spi1d("t.spi1d", text).is_err());
    }

    #[test]
    fn a_cube_lands_red_fastest_whatever_order_the_file_uses() {
        // Deliberately written blue-fastest, so a parser that ignored the
        // indices and pushed in file order would transpose the cube.
        let mut text = String::from("SPILUT 1.0\n3 3\n2 2 2\n");
        for r in 0..2 {
            for g in 0..2 {
                for b in 0..2 {
                    text.push_str(&format!(
                        "{r} {g} {b} {} {} {}\n",
                        r as f32,
                        g as f32 * 0.5,
                        b as f32 * 0.25
                    ));
                }
            }
        }
        let cube = parse_spi3d("t.spi3d", &text).expect("parses");
        assert_eq!(cube.size, 2);
        // Flat index 1 is (r=1, g=0, b=0).
        assert_eq!(cube.data.first().copied(), Some([0.0, 0.0, 0.0]));
        assert_eq!(cube.data.get(1).copied(), Some([1.0, 0.0, 0.0]));
        assert_eq!(cube.data.get(2).copied(), Some([0.0, 0.5, 0.0]));
        assert_eq!(cube.data.get(4).copied(), Some([0.0, 0.0, 0.25]));
    }

    #[test]
    fn a_cube_with_a_hole_in_it_is_refused_rather_than_filled_with_black() {
        let text = "SPILUT 1.0\n3 3\n2 2 2\n0 0 0 0 0 0\n1 0 0 1 0 0\n";
        let err = parse_spi3d("t.spi3d", text);
        assert!(err.is_err(), "{err:?}");
    }

    #[test]
    fn a_non_cubic_grid_is_refused_by_name() {
        let text = "SPILUT 1.0\n3 3\n4 8 4\n";
        assert!(parse_spi3d("t.spi3d", text).is_err());
    }

    #[test]
    fn rubbish_is_a_typed_error_not_a_panic() {
        assert!(parse_spi1d("t", "Version 1\nLength two\n").is_err());
        assert!(parse_spi3d("t", "SPILUT 1.0\n").is_err());
        assert!(parse_spi3d("t", "").is_err());
        assert!(parse_spimtx("t", "1 0 0 0\n0 1 0 0\n").is_err());
        assert!(parse_spimtx("t", "1 0 0 0 0 1 0 0 0 0 1 x").is_err());
    }

    /// The offset column is in 16-bit code values, and the divide by 65535 is
    /// the one thing this format can get quietly wrong.
    #[test]
    fn a_matrix_files_offsets_are_scaled_out_of_16_bit_code_values() {
        let text = "0.5 0 0 6553.5\n0 2 0 -65535\n0 0 1 0\n";
        let m = parse_spimtx("t.spimtx", text).expect("parses");
        assert_eq!(m[0], 0.5);
        assert_eq!(m[5], 2.0);
        assert!((m[3] - 0.1).abs() < 1e-9, "{m:?}");
        assert_eq!(m[7], -1.0);
        assert_eq!(m[11], 0.0);
    }
}
