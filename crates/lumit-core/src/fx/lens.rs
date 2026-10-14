//! Lenses as Lens flare and Depth of field pro both read them: the rows of a
//! `.lens` file, and a lens kept inside the effect so the project carries it.
//!
//! A lens picked from the library is just the effect's Lens row. Anything
//! else, an imported file, an edited copy or one of the user's own, is kept
//! in the instance under [`EXTRA_KEY`] as `{ "name", "text" }`, the way the
//! Custom shader keeps its source. It wins over the Lens row and over the
//! flare's older Lens file row, so the project opens the same anywhere.

use crate::model::EffectInstance;

/// Where an instance keeps its own lens.
pub const EXTRA_KEY: &str = "lens";

/// One surface of a `.lens` prescription, as the file says it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensRow {
    /// Signed radius in mm, 0 for flat.
    pub radius_mm: f32,
    /// Distance to the next surface, mm.
    pub thickness_mm: f32,
    /// Refractive index of what follows (1 for air).
    pub ior: f32,
    /// Abbe number of what follows (0 for air).
    pub abbe: f32,
    /// Clear semi-aperture, mm.
    pub semi_ap_mm: f32,
    /// The coating column: 0 bare, 1 single layer, 2 and up multicoat.
    pub coating: f32,
    /// The aperture stop.
    pub stop: bool,
}

/// A whole `.lens` file.
#[derive(Debug, Clone, PartialEq)]
pub struct LensRows {
    pub name: String,
    pub focal_mm: f32,
    pub rows: Vec<LensRow>,
}

/// Read a `.lens` text (the FlareSim and PhotonsToPhotos format): `name:` and
/// `focal_length:` lines, then `surfaces:` rows of
/// `radius thickness ior abbe semi_ap coating`, with `stop` or `inf` for the
/// radius. Bad rows are skipped. Under three surfaces, or no focal length, is
/// not a lens.
pub fn rows(text: &str) -> Option<LensRows> {
    let mut name = String::new();
    let mut focal = 0.0_f32;
    let mut in_surfaces = false;
    let mut rows = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !in_surfaces {
            if let Some(v) = line.strip_prefix("name:") {
                name = v.trim().to_owned();
            } else if let Some(v) = line.strip_prefix("focal_length:") {
                focal = v.trim().parse().unwrap_or(0.0);
            } else if line.starts_with("surfaces:") {
                in_surfaces = true;
            }
            continue;
        }
        let mut it = line.split_whitespace();
        let radius_tok = it.next().unwrap_or("");
        let stop = radius_tok.eq_ignore_ascii_case("stop");
        let radius_mm = if stop || radius_tok.eq_ignore_ascii_case("inf") {
            0.0
        } else {
            match radius_tok.parse::<f32>() {
                Ok(r) => r,
                Err(_) => continue,
            }
        };
        let mut f = |d: f32| it.next().and_then(|t| t.parse::<f32>().ok()).unwrap_or(d);
        let row = LensRow {
            radius_mm,
            thickness_mm: f(0.0),
            ior: f(1.0),
            abbe: f(0.0),
            semi_ap_mm: f(0.0),
            coating: f(0.0).max(0.0),
            stop,
        };
        if row.semi_ap_mm <= 0.0 {
            continue;
        }
        rows.push(row);
    }
    if rows.len() < 3 || focal <= 0.0 {
        return None;
    }
    Some(LensRows {
        name,
        focal_mm: focal,
        rows,
    })
}

/// Write a lens back out in the format [`rows`] reads.
pub fn write(lens: &LensRows) -> String {
    let mut out = format!(
        "# {}\n\nname: {}\nfocal_length: {:.2}\n\nsurfaces:\n# radius       thickness    ior      abbe     semi_ap   coating\n",
        lens.name, lens.name, lens.focal_mm
    );
    for r in &lens.rows {
        let radius = if r.stop {
            "stop".to_owned()
        } else if r.radius_mm == 0.0 {
            "inf".to_owned()
        } else {
            format!("{:.4}", r.radius_mm)
        };
        out.push_str(&format!(
            "  {radius:<12} {:<12.4} {:<8.5} {:<8.2} {:<9.3} {}\n",
            r.thickness_mm, r.ior, r.abbe, r.semi_ap_mm, r.coating
        ));
    }
    out
}

/// The lens an instance keeps, as `(name, text)`.
pub fn embedded(e: &EffectInstance) -> Option<(&str, &str)> {
    let lens = e.extra.get(EXTRA_KEY)?.as_object()?;
    let text = lens.get("text")?.as_str()?;
    let name = lens.get("name").and_then(|n| n.as_str()).unwrap_or("");
    Some((name, text))
}

/// Keep `text` in the instance, or forget the kept lens with `None`.
pub fn set_embedded(e: &mut EffectInstance, lens: Option<(&str, &str)>) {
    match lens {
        Some((name, text)) => {
            e.extra.insert(
                EXTRA_KEY.to_owned(),
                serde_json::json!({ "name": name, "text": text }),
            );
        }
        None => {
            e.extra.remove(EXTRA_KEY);
        }
    }
}

/// The effects that take a lens.
pub fn takes_a_lens(match_name: &str) -> bool {
    matches!(match_name, "lens_flare" | "dof_pro")
}

/// Where the render finds an effect's lens when it is not the Lens row's.
#[derive(Debug, Clone, PartialEq)]
pub enum LensSource {
    /// A `.lens` file on disk, the flare's older Lens file row.
    File(String),
    /// A lens kept in the effect.
    Text(String),
}

/// The lens an effect reads at `lt`, other than its Lens row's: the kept one
/// first, then a Lens file.
pub fn source_at(e: &EffectInstance, lt: f64) -> Option<LensSource> {
    if let Some((_, text)) = embedded(e) {
        return Some(LensSource::Text(text.to_owned()));
    }
    e.path_at("lens_file", lt)
        .map(|p| LensSource::File(p.to_owned()))
}
