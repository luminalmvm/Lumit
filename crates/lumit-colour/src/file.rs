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

/// The look-up table extensions this crate reads.
pub const READABLE: [&str; 4] = ["cube", "spi1d", "spi3d", "clf"];

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

    #[test]
    fn a_clf_file_can_become_several_steps() {
        let text = r#"<ProcessList id="t">
          <Matrix inBitDepth="32f" outBitDepth="32f"><Array dim="3 3 3">1 0 0 0 1 0 0 0 1</Array></Matrix>
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
}
