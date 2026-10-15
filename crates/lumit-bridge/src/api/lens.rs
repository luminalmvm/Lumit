//! The lens window's side of the bridge, shared by Lens flare and Depth of
//! field pro: the bundled lenses, the user's own folder of them, reading and saving
//! `.lens` files, the outline the window draws, and the lens an effect keeps.

use flutter_rust_bridge::frb;
use lumit_core::fx::lens::{self, LensRow, LensRows};
use lumit_core::fx::lens_library::LENS_LIBRARY;

use crate::api::{effect::BridgeEffectInstance, BridgeError};

/// One surface, as the file has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeLensRow {
    /// 0 for flat.
    pub radius_mm: f32,
    pub thickness_mm: f32,
    pub ior: f32,
    pub abbe: f32,
    pub semi_ap_mm: f32,
    pub coating: f32,
    pub stop: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLens {
    pub name: String,
    pub focal_mm: f32,
    pub rows: Vec<BridgeLensRow>,
}

/// A lens in the user's folder.
#[derive(Debug, Clone)]
pub struct BridgeLensFile {
    pub name: String,
    pub path: String,
}

/// One surface's outline for the cross-section: `points` are (z, y) pairs in
/// mm, front vertex at z 0, y up. `glass_after` says the space up to the next
/// surface is glass.
#[derive(Debug, Clone)]
pub struct BridgeLensLine {
    pub points: Vec<f32>,
    pub glass_after: bool,
    pub stop: bool,
}

impl From<LensRows> for BridgeLens {
    fn from(l: LensRows) -> Self {
        Self {
            name: l.name,
            focal_mm: l.focal_mm,
            rows: l
                .rows
                .into_iter()
                .map(|r| BridgeLensRow {
                    radius_mm: r.radius_mm,
                    thickness_mm: r.thickness_mm,
                    ior: r.ior,
                    abbe: r.abbe,
                    semi_ap_mm: r.semi_ap_mm,
                    coating: r.coating,
                    stop: r.stop,
                })
                .collect(),
        }
    }
}

fn rows_of(l: &BridgeLens) -> LensRows {
    LensRows {
        name: l.name.clone(),
        focal_mm: l.focal_mm,
        rows: l
            .rows
            .iter()
            .map(|r| LensRow {
                radius_mm: r.radius_mm,
                thickness_mm: r.thickness_mm,
                ior: r.ior,
                abbe: r.abbe,
                semi_ap_mm: r.semi_ap_mm,
                coating: r.coating,
                stop: r.stop,
            })
            .collect(),
    }
}

/// A file's lens, named for the library entry or the file if it has no name.
fn parsed(text: &str, fallback: &str) -> Option<BridgeLens> {
    let mut l: BridgeLens = lens::rows(text)?.into();
    if l.name.trim().is_empty() {
        fallback.clone_into(&mut l.name);
    }
    Some(l)
}

/// The bundled lenses' names, in the Lens row's order.
#[frb(sync)]
pub fn library_lens_names() -> Vec<String> {
    LENS_LIBRARY.iter().map(|l| l.name.to_owned()).collect()
}

/// A bundled lens, by its Lens row index.
#[frb(sync)]
pub fn library_lens(index: u32) -> Option<BridgeLens> {
    let entry = LENS_LIBRARY.get(index as usize)?;
    parsed(entry.text, entry.name).map(|mut l| {
        entry.name.clone_into(&mut l.name);
        l
    })
}

/// The lenses in the user's folder, by name.
#[frb(sync)]
pub fn user_lenses() -> Vec<BridgeLensFile> {
    let Some(dir) = lumit_project::lenses_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<BridgeLensFile> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lens"))
            {
                return None;
            }
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let lens = parsed(&std::fs::read_to_string(&path).ok()?, &stem)?;
            Some(BridgeLensFile {
                name: lens.name,
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect();
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

/// Read a `.lens` file anywhere on disk.
///
/// # Errors
/// [`BridgeError::ReadFailed`] when the file cannot be read or is not a lens.
pub fn read_lens(path: String) -> Result<BridgeLens, BridgeError> {
    let text = std::fs::read_to_string(&path).map_err(|_| BridgeError::ReadFailed)?;
    let stem = std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    parsed(&text, &stem).ok_or(BridgeError::ReadFailed)
}

/// Save a lens into the user's folder and answer its path. With `path` set to
/// a file already in the folder, that file is replaced; otherwise a new file is
/// made, numbered if the name is taken. Bundled lenses are built in, so nothing
/// here can change them.
///
/// # Errors
/// [`BridgeError::EmptyName`] for a blank name, [`BridgeError::ReadFailed`]
/// for a lens [`lens::rows`] would not read back, and
/// [`BridgeError::WriteFailed`] when the folder cannot be written.
pub fn save_user_lens(lens: BridgeLens, path: Option<String>) -> Result<String, BridgeError> {
    let name = lens.name.trim();
    if name.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    let text = lens::write(&rows_of(&lens));
    if lens::rows(&text).is_none() {
        return Err(BridgeError::ReadFailed);
    }
    let dir = lumit_project::lenses_dir().ok_or(BridgeError::WriteFailed)?;
    std::fs::create_dir_all(&dir).map_err(|_| BridgeError::WriteFailed)?;
    let target = match path.map(std::path::PathBuf::from) {
        Some(p) if p.parent() == Some(dir.as_path()) => p,
        _ => {
            let stem: String = name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || " -_.()".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let mut p = dir.join(format!("{stem}.lens"));
            let mut n = 2;
            while p.exists() {
                p = dir.join(format!("{stem} {n}.lens"));
                n += 1;
            }
            p
        }
    };
    std::fs::write(&target, text).map_err(|_| BridgeError::WriteFailed)?;
    Ok(target.to_string_lossy().into_owned())
}

/// The cross-section's outlines, front to back.
#[frb(sync)]
pub fn lens_drawing(lens: BridgeLens) -> Vec<BridgeLensLine> {
    const STEPS: usize = 24;
    let mut z = 0.0_f32;
    lens.rows
        .iter()
        .map(|r| {
            let mut points = Vec::with_capacity((STEPS + 1) * 2);
            for i in 0..=STEPS {
                // The column is the clear diameter, whatever the file calls it.
                let y = 0.5 * r.semi_ap_mm * (i as f32 / STEPS as f32 * 2.0 - 1.0);
                // The sphere's sag at height y; flat when it has no radius or
                // the semi-aperture outruns it.
                let sag = if r.radius_mm.abs() > 1e-6 && y.abs() < r.radius_mm.abs() {
                    r.radius_mm - r.radius_mm.signum() * (r.radius_mm * r.radius_mm - y * y).sqrt()
                } else {
                    0.0
                };
                points.push(z + sag);
                points.push(y);
            }
            let line = BridgeLensLine {
                points,
                glass_after: r.ior > 1.0001,
                stop: r.stop,
            };
            z += r.thickness_mm;
            line
        })
        .collect()
}

impl BridgeEffectInstance {
    /// The lens this effect keeps, which wins over its Lens row.
    #[frb(sync)]
    pub fn kept_lens(&self) -> Option<BridgeLens> {
        let (name, text) = lens::embedded(&self.effect)?;
        parsed(text, name)
    }

    /// Keep `lens` in the effect, or `None` to go back to the Lens row. Staged
    /// like `set_value`; `LayerReference::set_effects` commits it.
    #[frb(sync)]
    pub fn set_kept_lens(&mut self, lens: Option<BridgeLens>) {
        match lens {
            Some(l) => {
                let text = lens::write(&rows_of(&l));
                lens::set_embedded(&mut self.effect, Some((&l.name, &text)));
            }
            None => lens::set_embedded(&mut self.effect, None),
        }
    }
}
