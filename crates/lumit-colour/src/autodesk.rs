//! Autodesk's Flame and Lustre look-up table, `.3dl`.
//!
//! In plain terms: whole numbers, one triple per line, and a header line of
//! more than three numbers at the top which is a per-channel shaper curve. The
//! format never says what those whole numbers are out of, so the reader works
//! it out from the largest one it saw. The reference reader does the same, and
//! this file follows it step for step, because two readers guessing differently
//! is two different pictures.
//!
//! Two traps. The cube block is written **blue fastest**, the opposite of
//! Lumit's own order, so it is transposed on the way in; a cube read straight
//! through looks perfectly sensible on greys and is wrong on everything else.
//! And the shaper half of the format was never properly written down, so the
//! writers disagree by a code value or two: a shaper within two code values of
//! a straight line is treated as doing nothing at all.

use crate::error::{ColourError, Result};
use crate::op::{Chain, Direction, Op};
use crate::sample::{Cube, Curve, MAX_CUBE_SIZE, MAX_CURVE_SIZE};

/// Below this, the numbers are too small to be code values, and the file is
/// some other format that happens to be triples of numbers.
const LOWEST_PLAUSIBLE_MAXIMUM: i64 = 128;

fn bad(what: &str, reason: impl Into<String>) -> ColourError {
    ColourError::Parse {
        what: what.to_string(),
        reason: reason.into(),
    }
}

/// The bit depth a table of whole numbers was written at, from the largest one
/// in it. Twice the nominal maximum is allowed, so a table with overshoot in it
/// still reads at the depth it was written; 14-bit is not used in practice, so
/// anything past 12-bit is taken as 16.
fn likely_bit_depth(top: i64) -> u32 {
    for depth in [8_u32, 10, 12] {
        // Twice the nominal maximum, so a table written with overshoot in it
        // still reads at the depth it was written at.
        if top < (1_i64 << depth) * 2 {
            return depth;
        }
    }
    16
}

/// The code value 1.0 is written as at that depth.
fn code_maximum(depth: u32) -> f32 {
    ((1_i64 << depth) - 1) as f32
}

/// Whether a shaper is a straight line, within the two code values the format's
/// writers disagree by.
fn is_identity(values: &[i64], maximum: f32) -> bool {
    let Some(last) = values.len().checked_sub(1).filter(|n| *n > 0) else {
        return true;
    };
    let step = maximum / last as f32;
    values
        .iter()
        .enumerate()
        .all(|(i, v)| (i as f32 * step - *v as f32).abs() < 2.0)
}

/// Parse `.3dl` text into a chain: the shaper, where the file has one that
/// does something, then the cube.
pub fn parse_3dl(what: &str, text: &str) -> Result<Chain> {
    let mut shaper: Option<Vec<i64>> = None;
    let mut rows: Vec<[i64; 3]> = Vec::new();
    let mut cube_top = 0_i64;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('<') {
            return Err(bad(what, "this is XML, not a .3dl table"));
        }
        let mut numbers: Vec<i64> = Vec::new();
        let mut whole = true;
        for token in line.split_whitespace() {
            match token.parse::<i64>() {
                Ok(v) => numbers.push(v),
                Err(_) => {
                    whole = false;
                    break;
                }
            }
        }
        // A word rather than a number: `3DMESH`, `Mesh 4 12`, `LUT8`,
        // `gamma 1.0`. Applications write these and the format allows it.
        if !whole {
            continue;
        }
        match numbers.as_slice() {
            [] => {}
            [r, g, b] => {
                cube_top = cube_top.max(*r).max(*g).max(*b);
                rows.push([*r, *g, *b]);
            }
            // More than three numbers on one line is the shaper, and a file
            // holds at most one.
            [_, _, _, _, ..] => {
                if shaper.is_some() {
                    return Err(bad(what, "the file holds more than one shaper"));
                }
                if numbers.len() > MAX_CURVE_SIZE {
                    return Err(ColourError::TableTooLarge {
                        what: what.to_string(),
                        size: numbers.len(),
                        limit: MAX_CURVE_SIZE,
                    });
                }
                shaper = Some(numbers);
            }
            _ => return Err(bad(what, format!("a row needs three values: {line:?}"))),
        }
    }

    if shaper.is_none() && rows.is_empty() {
        return Err(bad(what, "the file holds neither a shaper nor a cube"));
    }

    let mut ops = Vec::with_capacity(2);
    if let Some(values) = shaper {
        let top = values.iter().copied().max().unwrap_or(0);
        if top < LOWEST_PLAUSIBLE_MAXIMUM {
            return Err(bad(
                what,
                format!("the shaper's largest value, {top}, is too small to be a code value"),
            ));
        }
        let maximum = code_maximum(likely_bit_depth(top));
        if !is_identity(&values, maximum) {
            let data = values.iter().map(|v| [*v as f32 / maximum; 3]).collect();
            ops.push(Op::Lut1d {
                curve: Curve::new(what, [0.0, 1.0], data)?,
                dir: Direction::Forward,
            });
        }
    }

    if !rows.is_empty() {
        if cube_top < LOWEST_PLAUSIBLE_MAXIMUM {
            return Err(bad(
                what,
                format!("the cube's largest value, {cube_top}, is too small to be a code value"),
            ));
        }
        let size = (rows.len() as f64).cbrt().round() as usize;
        if size < 2 || size.checked_pow(3) != Some(rows.len()) {
            return Err(bad(
                what,
                format!("{} rows do not make a whole cube", rows.len()),
            ));
        }
        if size > MAX_CUBE_SIZE {
            return Err(ColourError::TableTooLarge {
                what: what.to_string(),
                size,
                limit: MAX_CUBE_SIZE,
            });
        }
        let maximum = code_maximum(likely_bit_depth(cube_top));
        // The file counts blue fastest and Lumit counts red fastest, so every
        // sample moves. Reading the block straight through transposes the cube,
        // which no neutral ever notices.
        let mut data = vec![[0.0_f32; 3]; rows.len()];
        for (i, row) in rows.iter().enumerate() {
            let b = i % size;
            let g = (i / size) % size;
            let r = i / (size * size);
            if let Some(slot) = data.get_mut(r + g * size + b * size * size) {
                *slot = [
                    row[0] as f32 / maximum,
                    row[1] as f32 / maximum,
                    row[2] as f32 / maximum,
                ];
            }
        }
        ops.push(Op::Lut3d {
            cube: Cube::new(what, size, [0.0; 3], [1.0; 3], data)?,
        });
    }

    Ok(Chain::new(ops))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A Lustre file with a straight shaper and a 12-bit cube whose value
    /// differs on every axis.
    fn lustre() -> String {
        // Five 10-bit samples, straight enough that the reader drops them.
        let mut text = String::from("3DMESH\nMesh 1 12\n0 255 511 767 1023\n");
        for r in 0..2 {
            for g in 0..2 {
                for b in 0..2 {
                    text.push_str(&format!("{} {} {}\n", r * 4095, g * 2048, b * 1024));
                }
            }
        }
        text.push_str("\nLUT8\ngamma 1.0\n");
        text
    }

    /// The trap: the file counts blue fastest, Lumit counts red fastest.
    #[test]
    fn a_cube_block_is_transposed_out_of_blue_fastest_order() {
        let chain = parse_3dl("t.3dl", &lustre()).expect("parses");
        assert_eq!(chain.ops.len(), 1, "the straight shaper does nothing");
        let Some(Op::Lut3d { cube }) = chain.ops.first() else {
            panic!("expected a cube step, got {:?}", chain.ops)
        };
        assert_eq!(cube.data.first().copied(), Some([0.0, 0.0, 0.0]));
        // Flat index 1 is (r=1, g=0, b=0), which the file writes fifth.
        assert_eq!(cube.data.get(1).copied(), Some([1.0, 0.0, 0.0]));
        // Flat index 4 is (r=0, g=0, b=1), which the file writes second.
        assert!(
            matches!(cube.data.get(4), Some([0.0, 0.0, v]) if (v - 1024.0 / 4095.0).abs() < 1e-6),
            "{:?}",
            cube.data.get(4)
        );
    }

    /// A shaper that does something survives; the bit depth comes from its own
    /// largest value, not the cube's.
    #[test]
    fn a_shaper_that_bends_is_kept_and_scaled_by_its_own_bit_depth() {
        let mut text = String::from("0 100 400 1023\n");
        for _ in 0..8 {
            text.push_str("4095 4095 4095\n");
        }
        let chain = parse_3dl("t.3dl", &text).expect("parses");
        let Some(Op::Lut1d { curve, .. }) = chain.ops.first() else {
            panic!("expected a curve step, got {:?}", chain.ops)
        };
        assert_eq!(curve.data.len(), 4);
        assert!(
            matches!(curve.data.get(1), Some([v, _, _]) if (v - 100.0 / 1023.0).abs() < 1e-6),
            "{:?}",
            curve.data.get(1)
        );
    }

    #[test]
    fn rubbish_is_a_typed_error_not_a_panic() {
        assert!(parse_3dl("t.3dl", "").is_err());
        assert!(parse_3dl("t.3dl", "<ProcessList/>").is_err());
        // Whole numbers, but far too small to be code values.
        assert!(parse_3dl("t.3dl", &"1 1 1\n".repeat(8)).is_err());
        // Nine triples do not make a cube.
        assert!(parse_3dl("t.3dl", &"4095 4095 4095\n".repeat(9)).is_err());
        assert!(parse_3dl("t.3dl", "0 512 1023\n0 512 1023\n").is_err());
    }

    #[test]
    fn a_bit_depth_is_inferred_the_way_the_reference_infers_it() {
        assert_eq!(likely_bit_depth(255), 8);
        assert_eq!(likely_bit_depth(511), 8);
        assert_eq!(likely_bit_depth(512), 10);
        assert_eq!(likely_bit_depth(2047), 10);
        assert_eq!(likely_bit_depth(4095), 12);
        // 14-bit scaling is not used in practice, so it reads as 16.
        assert_eq!(likely_bit_depth(16383), 16);
        assert_eq!(likely_bit_depth(65535), 16);
        assert_eq!(code_maximum(12), 4095.0);
    }
}
