//! An OpenVDB volume: density and heat, laid out dense for the card.
//!
//! A `.vdb` opens as a model with no faces, so it comes in, relinks, packs and
//! drops like one, and its box is what the layer's transform carries. What it
//! draws goes through the smoke marcher. A numbered file reads its neighbours as
//! the frames after it.

use std::io::Cursor;
use std::path::{Path as FilePath, PathBuf};

use vdb_rs::{Map, VdbLevel, VdbReader};

use crate::{Bounds, ModelScene, SceneError};

/// The most cells along any side, past which a volume is read every so many.
const MOST_CELLS: i64 = 256;

/// The grid names read as density, and as heat, the first found of each.
const DENSITY: [&str; 2] = ["density", "smoke"];
const HEAT: [&str; 4] = ["flame", "fire", "temperature", "heat"];

/// A volume's density and heat in half floats, two to a cell, for a 3D texture.
#[derive(Debug, Clone, PartialEq)]
pub struct DenseVolume {
    pub cells: [u32; 3],
    /// The cells, a row of `pitch` at a time.
    pub values: Vec<u16>,
    pub pitch: u32,
    /// Where its box is, in model space.
    pub bounds: Bounds,
}

impl DenseVolume {
    /// Roughly the memory it holds.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        self.values.len() * 2
    }
}

fn fail(path: &FilePath, reason: impl Into<String>) -> SceneError {
    SceneError::Read {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

/// A grid's index space to model space: its scale, then where cell nought is.
fn placed(map: &Map) -> ([f64; 3], [f64; 3]) {
    match map {
        Map::UniformScaleMap { voxel_size, .. } => {
            ([voxel_size.x, voxel_size.y, voxel_size.z], [0.0; 3])
        }
        Map::ScaleTranslateMap {
            voxel_size,
            translation,
            ..
        } => (
            [voxel_size.x, voxel_size.y, voxel_size.z],
            [translation.x, translation.y, translation.z],
        ),
    }
}

fn read_file(path: &FilePath) -> Result<VdbReader<Cursor<Vec<u8>>>, SceneError> {
    let bytes = lumit_ingress::files::read(path).map_err(|e| fail(path, e.to_string()))?;
    VdbReader::new(Cursor::new(bytes)).map_err(|e| fail(path, e.to_string()))
}

/// The first of `names` the file holds as a grid of floats.
fn grid_named(reader: &VdbReader<Cursor<Vec<u8>>>, names: &[&str]) -> Option<String> {
    let have = reader.available_grids();
    names
        .iter()
        .find(|n| have.iter().any(|h| h == *n))
        .map(|n| (*n).to_owned())
}

/// What to multiply heat by across the frames `files`, so flames hold one
/// brightness through a sequence: a flame grid is nought to one already, and any
/// other kind is read against its top in the first, middle and last frames.
#[must_use]
pub fn heat_gain(files: &[PathBuf]) -> f32 {
    let top = |file: &PathBuf| -> Option<f32> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut reader = read_file(file).ok()?;
            let heat = grid_named(&reader, &HEAT)?;
            if matches!(heat.as_str(), "flame" | "fire") {
                return Some(1.0);
            }
            let g = reader
                .read_grid::<half::f16>(&heat)
                .map(Grid::Half)
                .or_else(|_| reader.read_grid::<f32>(&heat).map(Grid::Full))
                .ok()?;
            let mut most = 0.0f32;
            g.each(&mut |_, v, _| most = most.max(v));
            Some(most)
        }))
        .ok()
        .flatten()
    };
    let picks = [files.first(), files.get(files.len() / 2), files.last()];
    let most = picks
        .into_iter()
        .flatten()
        .filter_map(top)
        .fold(0.0, f32::max);
    1.0 / most.max(1e-6)
}

/// The file at `path` read dense, its heat multiplied by `gain`. Other people's
/// parser, so a panic in it is a file that won't open.
pub fn read_dense(path: &FilePath, gain: f32) -> Result<DenseVolume, SceneError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dense(path, gain)))
        .unwrap_or_else(|_| Err(fail(path, "the file is damaged")))
}

fn dense(path: &FilePath, gain: f32) -> Result<DenseVolume, SceneError> {
    let mut reader = read_file(path)?;
    let density = grid_named(&reader, &DENSITY)
        .or_else(|| reader.available_grids().into_iter().next())
        .ok_or_else(|| fail(path, "the file holds no grids"))?;
    let grid = reader
        .read_grid::<half::f16>(&density)
        .map(Grid::Half)
        .or_else(|_| reader.read_grid::<f32>(&density).map(Grid::Full))
        .map_err(|e| fail(path, e.to_string()))?;
    let (scale, shift) = grid.place();
    // The box from the cells in use, not from the file's own note of it, which
    // writers leave out or leave stale.
    let (mut lo, mut hi) = ([i64::MAX; 3], [i64::MIN; 3]);
    grid.each(&mut |at, _, tile| {
        for k in 0..3 {
            lo[k] = lo[k].min(at[k]);
            hi[k] = hi[k].max(at[k] + tile);
        }
    });
    if lo[0] > hi[0] {
        // Nothing in it, a frame before the smoke starts: an empty cell.
        return Ok(DenseVolume {
            cells: [1; 3],
            values: vec![0; 2],
            pitch: 1,
            bounds: Bounds::default(),
        });
    }
    let longest = (0..3).map(|k| hi[k] - lo[k]).max().unwrap_or(1).max(1);
    let every = (longest + MOST_CELLS - 1) / MOST_CELLS;
    let cells = [0, 1, 2].map(|k| ((hi[k] - lo[k] + every - 1) / every).max(1) as u32);
    // A copy into a 3D texture wants a round number of bytes in a row.
    let pitch = (cells[0] * 4).next_multiple_of(256) / 4;
    let total = pitch as usize * cells[1] as usize * cells[2] as usize * 2;
    let mut values = vec![0u16; total];
    let at = |c: [i64; 3], k: usize| {
        let x = ((c[0] - lo[0]) / every) as usize;
        let y = ((c[1] - lo[1]) / every) as usize;
        let z = ((c[2] - lo[2]) / every) as usize;
        (x + y * pitch as usize + z * pitch as usize * cells[1] as usize) * 2 + k
    };
    let mut lay = |g: &Grid, k: usize, gain: f32| {
        g.each(&mut |c, v, tile| {
            let bits = half::f16::from_f32(v * gain).to_bits();
            let mut step = |d: [i64; 3]| {
                let i = at([c[0] + d[0], c[1] + d[1], c[2] + d[2]], k);
                if let Some(slot) = values.get_mut(i) {
                    *slot = bits;
                }
            };
            let by = every.max(1) as usize;
            for dz in (0..tile).step_by(by) {
                for dy in (0..tile).step_by(by) {
                    for dx in (0..tile).step_by(by) {
                        step([dx, dy, dz]);
                    }
                }
            }
        });
    };
    lay(&grid, 0, 1.0);
    if let Some(heat) = grid_named(&reader, &HEAT).filter(|h| *h != density) {
        let got = reader
            .read_grid::<half::f16>(&heat)
            .map(Grid::Half)
            .or_else(|_| reader.read_grid::<f32>(&heat).map(Grid::Full));
        if let Ok(g) = got {
            lay(&g, 1, gain);
        }
    }
    let side = |k: usize, v: i64| (v as f64 * scale[k] + shift[k]) as f32;
    Ok(DenseVolume {
        cells,
        values,
        pitch,
        bounds: Bounds {
            min: [0, 1, 2].map(|k| side(k, lo[k])),
            max: [0, 1, 2].map(|k| side(k, hi[k])),
        },
    })
}

/// A grid of either width of float.
enum Grid {
    Half(vdb_rs::Grid<half::f16>),
    Full(vdb_rs::Grid<f32>),
}

impl Grid {
    fn place(&self) -> ([f64; 3], [f64; 3]) {
        match self {
            Self::Half(g) => placed(&g.transform),
            Self::Full(g) => placed(&g.transform),
        }
    }

    /// Every cell or tile in use: where it is, its value, and how wide it is.
    fn each(&self, f: &mut dyn FnMut([i64; 3], f32, i64)) {
        let tile = |l: VdbLevel| l.scale() as i64;
        match self {
            Self::Half(g) => {
                for (p, v, l) in g.iter() {
                    f([p.x as i64, p.y as i64, p.z as i64], v.to_f32(), tile(l));
                }
            }
            Self::Full(g) => {
                for (p, v, l) in g.iter() {
                    f([p.x as i64, p.y as i64, p.z as i64], v, tile(l));
                }
            }
        }
    }
}

/// A model with no faces whose box is the volume's, for the rest of the app. A
/// numbered volume grows as it goes, so its box is read from its first frame, its
/// middle one and its last.
pub(crate) fn scene(path: &FilePath) -> Result<ModelScene, SceneError> {
    let all = frames(path);
    let mut bounds = read_dense(path, 1.0)?.bounds;
    for file in [all.get(all.len() / 2), all.last()].into_iter().flatten() {
        let Ok(v) = read_dense(file, 1.0) else {
            continue;
        };
        if v.bounds == Bounds::default() {
            continue;
        }
        if bounds == Bounds::default() {
            bounds = v.bounds;
        }
        for k in 0..3 {
            bounds.min[k] = bounds.min[k].min(v.bounds.min[k]);
            bounds.max[k] = bounds.max[k].max(v.bounds.max[k]);
        }
    }
    Ok(ModelScene {
        bounds,
        ..ModelScene::default()
    })
}

/// The frames of the volume at `path`: its numbered neighbours in order, from
/// itself on, or only itself.
#[must_use]
pub fn frames(path: &FilePath) -> Vec<PathBuf> {
    let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
        return vec![path.to_path_buf()];
    };
    let digits = stem.chars().rev().take_while(char::is_ascii_digit).count();
    let (Some(dir), true) = (path.parent(), digits > 0) else {
        return vec![path.to_path_buf()];
    };
    let head = &stem[..stem.len() - digits];
    let number = |name: &str| -> Option<u64> {
        let file = FilePath::new(name);
        let ext = file.extension()?.to_string_lossy().to_ascii_lowercase();
        let tail = file.file_stem()?.to_string_lossy();
        let tail = tail.strip_prefix(head)?;
        (ext == "vdb").then(|| tail.parse().ok()).flatten()
    };
    let Ok(listed) = lumit_ingress::files::list(dir) else {
        return vec![path.to_path_buf()];
    };
    let first = stem[stem.len() - digits..].parse::<u64>().unwrap_or(0);
    let mut found: Vec<(u64, String)> = listed
        .into_iter()
        .filter_map(|(name, _)| number(&name).filter(|n| *n >= first).map(|n| (n, name)))
        .collect();
    found.sort();
    if found.is_empty() {
        return vec![path.to_path_buf()];
    }
    found.into_iter().map(|(_, name)| dir.join(name)).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A file that is no volume is an error, never a panic.
    #[test]
    fn a_broken_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.vdb");
        std::fs::write(&path, b"this is no volume").unwrap();
        assert!(read_dense(&path, 1.0).is_err());
        assert!(scene(&path).is_err());
    }

    /// A numbered file reads the neighbours after it, in number order.
    #[test]
    fn numbered_neighbours_are_its_frames() {
        let dir = tempfile::tempdir().unwrap();
        for n in [
            "fire_0009.vdb",
            "fire_0010.vdb",
            "fire_0002.vdb",
            "other_0003.vdb",
        ] {
            std::fs::write(dir.path().join(n), b"").unwrap();
        }
        let got = frames(&dir.path().join("fire_0002.vdb"));
        let names: Vec<_> = got
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["fire_0002.vdb", "fire_0009.vdb", "fire_0010.vdb"]);
        let lone = dir.path().join("lone.vdb");
        assert_eq!(frames(&lone), [lone]);
    }
}
