//! Reading whatever file a `FileTransform` points at.
//!
//! In plain terms: a config's `FileTransform` says "the maths is in that file
//! over there". This module works out which of the four grammars the file is in
//! from its extension and hands back a chain — one step for a table file,
//! several for a CLF, which is a list of steps by design. An extension Lumit
//! does not read is refused by name rather than guessed at.

use std::path::Path;

use crate::error::{ColourError, Result};
use crate::op::{Chain, Direction, Op};
use crate::sample::{Cube, Curve};
use crate::{clf, spi};

/// The look-up table and grade file extensions this crate reads.
pub const READABLE: [&str; 8] = ["cube", "spi1d", "spi3d", "clf", "ctf", "cc", "ccc", "cdl"];

/// Read a table file into a chain. `path` is used for the grammar and for the
/// sentence any refusal shows.
pub fn load(path: &Path) -> Result<Chain> {
    let text = std::fs::read_to_string(path).map_err(|e| ColourError::FileRead {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    parse(&name, &extension, &text)
}

/// The grammar half of [`load`], split out so tests need no files on disk.
pub fn parse(name: &str, extension: &str, text: &str) -> Result<Chain> {
    Ok(match extension {
        "cube" => {
            let lut = lumit_core::lut::parse_cube(text).map_err(|e| ColourError::Parse {
                what: name.to_string(),
                reason: e.to_string(),
            })?;
            let op = match lut {
                lumit_core::lut::Lut::Cube3d(cube) => Op::Lut3d {
                    cube: Cube::from(cube),
                },
                lumit_core::lut::Lut::Cube1d(curve) => Op::Lut1d {
                    curve: Curve::from(curve),
                    dir: Direction::Forward,
                },
            };
            Chain::new(vec![op])
        }
        "spi1d" => Chain::new(vec![Op::Lut1d {
            curve: spi::parse_spi1d(name, text)?,
            dir: Direction::Forward,
        }]),
        "spi3d" => Chain::new(vec![Op::Lut3d {
            cube: spi::parse_spi3d(name, text)?,
        }]),
        // `.ctf` is CTF: the same grammar with vendor-specific extras, which
        // this reader refuses individually when it meets one.
        "clf" | "ctf" => clf::parse_clf(name, text)?,
        // An ASC CDL grade: one `ColorCorrection`, or a collection or decision
        // list of them, of which the first is taken. Picking one by its
        // `cccid` is refused where the config names one (`config.rs`), and an
        // effect has no field to name one with.
        "cc" | "ccc" | "cdl" => {
            let mut chain = clf::parse_clf(name, text)?;
            chain.ops.truncate(1);
            if chain.ops.is_empty() {
                return Err(ColourError::Parse {
                    what: name.to_string(),
                    reason: "no ColorCorrection in the file".to_string(),
                });
            }
            chain
        }
        other => {
            return Err(ColourError::UnsupportedLutFormat {
                extension: if other.is_empty() {
                    format!("{name} (no extension)")
                } else {
                    format!(".{other}")
                },
            })
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_1d_cube_file_becomes_a_curve_step() {
        let chain = parse("t.cube", "cube", "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").expect("parses");
        assert!(matches!(chain.ops.first(), Some(Op::Lut1d { .. })));
    }

    #[test]
    fn a_3d_cube_file_becomes_a_cube_step() {
        let mut text = String::from("LUT_3D_SIZE 2\n");
        for _ in 0..8 {
            text.push_str("0 0 0\n");
        }
        let chain = parse("t.cube", "cube", &text).expect("parses");
        assert!(matches!(chain.ops.first(), Some(Op::Lut3d { .. })));
    }

    #[test]
    fn a_spi1d_file_becomes_a_curve_step() {
        let text = "Version 1\nFrom 0 1\nLength 2\nComponents 1\n{\n0.0\n1.0\n}\n";
        let chain = parse("t.spi1d", "spi1d", text).expect("parses");
        assert!(matches!(chain.ops.first(), Some(Op::Lut1d { .. })));
    }

    /// The matrix is a scale rather than the identity on purpose: an identity
    /// matrix is dropped at chain construction (`Chain::new`), so writing one
    /// here would be testing the fold rather than the reader.
    #[test]
    fn a_clf_file_can_become_several_steps() {
        let text = r#"<ProcessList id="t">
          <Matrix inBitDepth="32f" outBitDepth="32f"><Array dim="3 3 3">2 0 0 0 2 0 0 0 2</Array></Matrix>
          <Range inBitDepth="32f" outBitDepth="32f"><minOutValue>0</minOutValue><maxOutValue>1</maxOutValue></Range>
        </ProcessList>"#;
        assert_eq!(parse("t.clf", "clf", text).expect("parses").ops.len(), 2);
    }

    #[test]
    fn an_unread_extension_refuses_by_name() {
        let err = parse("look.3dl", "3dl", "");
        assert!(
            matches!(&err, Err(ColourError::UnsupportedLutFormat { extension }) if extension == ".3dl"),
            "{err:?}"
        );
    }

    #[test]
    fn a_missing_file_is_a_typed_error() {
        let err = load(Path::new("this-file-does-not-exist.spi1d"));
        assert!(matches!(err, Err(ColourError::FileRead { .. })), "{err:?}");
    }

    /// The ASC CDL file grammar: the same block a CLF `ASC_CDL` node carries,
    /// under its own wrappers. A collection yields its first grade only.
    #[test]
    fn a_cdl_file_becomes_one_cdl_step() {
        let cc = r#"<ColorCorrection id="shot1">
          <SOPNode><Slope>1.1 1.0 0.9</Slope><Offset>0 0 0</Offset><Power>1 1 1</Power></SOPNode>
          <SatNode><Saturation>0.8</Saturation></SatNode>
        </ColorCorrection>"#;
        let chain = parse("shot.cc", "cc", cc).expect("parses");
        assert_eq!(chain.ops.len(), 1);
        assert!(matches!(
            &chain.ops[0],
            Op::Cdl { params, .. } if params.slope == [1.1, 1.0, 0.9] && params.saturation == 0.8
        ));

        let ccc = format!(
            r#"<ColorCorrectionCollection xmlns="urn:ASC:CDL:v1.01">{cc}<ColorCorrection id="shot2">
          <SOPNode><Slope>2 2 2</Slope></SOPNode></ColorCorrection></ColorCorrectionCollection>"#
        );
        let chain = parse("grades.ccc", "ccc", &ccc).expect("parses");
        assert_eq!(chain.ops.len(), 1, "the first grade, not both");
        assert!(matches!(
            &chain.ops[0],
            Op::Cdl { params, .. } if params.slope == [1.1, 1.0, 0.9]
        ));

        let err = parse("empty.cdl", "cdl", "<ColorDecisionList/>");
        assert!(matches!(err, Err(ColourError::Parse { .. })), "{err:?}");
    }
}
