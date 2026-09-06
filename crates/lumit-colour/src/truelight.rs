//! FilmLight's Truelight cube, `.cub`.
//!
//! In plain terms: a Truelight cube is two tables in one file. First an
//! **input LUT**, a per-channel curve that spreads the incoming values out so
//! the cube after it has its samples where the picture actually is; then the
//! **cube** itself. A file may carry either half on its own, and the small
//! FilmLight T-Log files in real configs carry only the curve.
//!
//! Two details are worth stating rather than re-deriving. The input LUT's
//! numbers are **grid positions**, not colours: they run from 0 to one less
//! than the cube's width, so they are divided back down before the cube reads
//! them, and forgetting that sends every value off the top of the cube. And the
//! cube block is written **red fastest**, which is Lumit's own order, so it is
//! copied straight across.

use crate::error::{ColourError, Result};
use crate::op::{Chain, Direction, Op};
use crate::sample::{Cube, Curve, MAX_CUBE_SIZE, MAX_CURVE_SIZE};

fn bad(what: &str, reason: impl Into<String>) -> ColourError {
    ColourError::Parse {
        what: what.to_string(),
        reason: reason.into(),
    }
}

/// Which block of the file the reader is inside.
enum Block {
    None,
    Shaper,
    Cube,
}

/// Parse `.cub` text into a chain: the input LUT, then the cube.
pub fn parse_cub(what: &str, text: &str) -> Result<Chain> {
    let mut lines = text.lines();
    if !lines
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .starts_with("# truelight cube")
    {
        return Err(bad(what, "the first line does not say `# Truelight Cube`"));
    }

    let mut size_1d = 0_usize;
    let mut size_3d = 0_usize;
    let mut shaper: Vec<[f32; 3]> = Vec::new();
    let mut cube: Vec<[f32; 3]> = Vec::new();
    let mut block = Block::None;

    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let mut words = rest.split_whitespace();
            let key = words.next().unwrap_or_default().to_ascii_lowercase();
            match key.as_str() {
                "width" => {
                    let sizes: Vec<usize> = words.filter_map(|t| t.parse::<usize>().ok()).collect();
                    let [r, g, b] = sizes.as_slice() else {
                        return Err(bad(what, format!("could not read the width in {line:?}")));
                    };
                    if r != g || g != b {
                        return Err(bad(
                            what,
                            format!("Lumit reads cubes with equal sides; this one is {r}×{g}×{b}"),
                        ));
                    }
                    if *r < 2 {
                        return Err(bad(
                            what,
                            format!("a cube needs at least 2 points per axis, not {r}"),
                        ));
                    }
                    if *r > MAX_CUBE_SIZE {
                        return Err(ColourError::TableTooLarge {
                            what: what.to_string(),
                            size: *r,
                            limit: MAX_CUBE_SIZE,
                        });
                    }
                    size_3d = *r;
                }
                "lutlength" => {
                    let n = words
                        .next()
                        .and_then(|t| t.parse::<usize>().ok())
                        .ok_or_else(|| {
                            bad(what, format!("could not read the lut length in {line:?}"))
                        })?;
                    if n > MAX_CURVE_SIZE {
                        return Err(ColourError::TableTooLarge {
                            what: what.to_string(),
                            size: n,
                            limit: MAX_CURVE_SIZE,
                        });
                    }
                    size_1d = n;
                }
                "inputlut" => block = Block::Shaper,
                "cube" => block = Block::Cube,
                "end" => break,
                // Any other header line, `# iDims` and `# oDims` among them,
                // says nothing this reader needs.
                _ => {}
            }
            continue;
        }

        let mut sample = [0.0_f32; 3];
        let mut count = 0_usize;
        for token in line.split_whitespace() {
            match token.parse::<f32>() {
                Ok(v) if count < 3 => {
                    sample[count] = v;
                    count += 1;
                }
                // A line that is not three numbers is not data. The format
                // lets a writer put its own notes between the blocks, so this
                // is skipped rather than refused, as the reference reader does.
                _ => {
                    count = 0;
                    break;
                }
            }
        }
        if count != 3 {
            continue;
        }
        match block {
            Block::Shaper => shaper.push(sample),
            Block::Cube => cube.push(sample),
            Block::None => {}
        }
    }

    if shaper.len() != size_1d {
        return Err(bad(
            what,
            format!(
                "the header declares {size_1d} input LUT samples but the file has {}",
                shaper.len()
            ),
        ));
    }
    let cells = size_3d * size_3d * size_3d;
    if cube.len() != cells {
        return Err(bad(
            what,
            format!(
                "the header declares {cells} cube samples but the file has {}",
                cube.len()
            ),
        ));
    }

    let mut ops = Vec::with_capacity(2);
    if size_1d > 0 {
        if size_3d > 0 {
            // The input LUT's numbers are positions in the cube, 0 to width−1,
            // so they come back down to the 0 to 1 the cube reads.
            let descale = 1.0 / (size_3d - 1) as f32;
            for sample in &mut shaper {
                for v in sample.iter_mut() {
                    *v *= descale;
                }
            }
        }
        ops.push(Op::Lut1d {
            curve: Curve::new(what, [0.0, 1.0], shaper)?,
            dir: Direction::Forward,
        });
    }
    if size_3d > 0 {
        ops.push(Op::Lut3d {
            cube: Cube::new(what, size_3d, [0.0; 3], [1.0; 3], cube)?,
        });
    }
    if ops.is_empty() {
        return Err(bad(what, "the file holds neither an input LUT nor a cube"));
    }
    Ok(Chain::new(ops))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A two-point cube written the way the format writes one, with a
    /// different value on every axis so a block read in the wrong order gives
    /// a different answer.
    fn two_point() -> String {
        let mut text = String::from(
            "# Truelight Cube v2.0\n# lutLength 2\n# width 2 2 2\n\n# InputLUT\n0 0 0\n1 1 1\n\n# Cube\n",
        );
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text.push_str(&format!(
                        "{} {} {}\n",
                        r as f32,
                        g as f32 * 0.5,
                        b as f32 * 0.25
                    ));
                }
            }
        }
        text.push_str("# end\n");
        text
    }

    #[test]
    fn a_cube_block_is_read_red_fastest() {
        let chain = parse_cub("t.cub", &two_point()).expect("parses");
        let Some(Op::Lut3d { cube }) = chain.ops.get(1) else {
            panic!("expected a cube step, got {:?}", chain.ops)
        };
        // Flat index 1 is (r=1, g=0, b=0), which the file writes second.
        assert_eq!(cube.data.first().copied(), Some([0.0, 0.0, 0.0]));
        assert_eq!(cube.data.get(1).copied(), Some([1.0, 0.0, 0.0]));
        assert_eq!(cube.data.get(2).copied(), Some([0.0, 0.5, 0.0]));
        assert_eq!(cube.data.get(4).copied(), Some([0.0, 0.0, 0.25]));
    }

    /// The trap this format is here for: the input LUT counts in cube cells,
    /// so without the divide every value lands on the last one.
    #[test]
    fn the_input_lut_is_scaled_out_of_cube_positions() {
        let mut text =
            String::from("# Truelight Cube v2.0\n# lutLength 2\n# width 3 3 3\n# InputLUT\n0 0 0\n2 2 2\n# Cube\n");
        for i in 0..27 {
            text.push_str(&format!("{} 0 0\n", i as f32 / 26.0));
        }
        text.push_str("# end\n");
        let chain = parse_cub("t.cub", &text).expect("parses");
        let Some(Op::Lut1d { curve, .. }) = chain.ops.first() else {
            panic!("expected a curve step, got {:?}", chain.ops)
        };
        assert_eq!(curve.data.last().copied(), Some([1.0; 3]));
    }

    /// The real FilmLight T-Log files carry a curve and nothing else.
    #[test]
    fn a_file_with_only_an_input_lut_is_a_curve_on_its_own() {
        let text = "# Truelight Cube v2.1\n# lutLength 3\n# iDims 3\n\n# InputLUT\n-0.5 -0.5 -0.5\n0.25 0.25 0.25\n2 2 2\n\n# end\n";
        let chain = parse_cub("t.cub", text).expect("parses");
        assert_eq!(chain.ops.len(), 1);
        let Some(Op::Lut1d { curve, .. }) = chain.ops.first() else {
            panic!("expected a curve step")
        };
        // No cube, so no descale: the samples are the colours themselves.
        assert_eq!(curve.data.get(2).copied(), Some([2.0; 3]));
    }

    #[test]
    fn rubbish_is_a_typed_error_not_a_panic() {
        assert!(parse_cub("t.cub", "").is_err());
        assert!(parse_cub("t.cub", "LUT_3D_SIZE 2\n").is_err());
        // The counts the header promises have to be the counts the file has.
        assert!(parse_cub(
            "t.cub",
            "# Truelight Cube v2.0\n# lutLength 4\n# InputLUT\n0 0 0\n"
        )
        .is_err());
        assert!(parse_cub("t.cub", "# Truelight Cube v2.0\n# width 2 2 4\n").is_err());
        assert!(parse_cub("t.cub", "# Truelight Cube v2.0\n# end\n").is_err());
    }
}
