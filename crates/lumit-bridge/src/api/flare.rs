//! The Flare window's side of the bridge: what a design can hold, the design
//! an effect keeps, the preview the window draws, and the user's own folder
//! of `.flare` files beside the built-in ones.

use flutter_rust_bridge::frb;
use lumit_core::fx::flare::controls::{self, Edit, Unit};
use lumit_core::fx::flare::design::{self, ColourMode, Design, Element, Kind};
use lumit_core::fx::flare::presets;
use lumit_core::fx::flare::raster;
use lumit_core::pixels::srgb_encode;

use crate::api::composition::CompositionReference;
use crate::api::state::BridgeRenderedFrame;
use crate::api::{effect::BridgeEffectInstance, BridgeError};

/// One kind of element, as the Add menu lists it.
#[derive(Debug, Clone)]
pub struct BridgeFlareKind {
    /// The key the design's JSON uses.
    pub key: String,
    pub label: String,
    /// The Add menu heading it sits under.
    pub place: String,
}

/// One of a short list, by key and label.
#[derive(Debug, Clone)]
pub struct BridgeFlareOption {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFlareEdit {
    Slider,
    Counter,
    Toggle,
    Angle,
    Choice,
}

/// One control of an element's kind, as `controls::Control` has it.
#[derive(Debug, Clone)]
pub struct BridgeFlareControl {
    pub key: String,
    pub label: String,
    pub group: String,
    pub edit: BridgeFlareEdit,
    /// A Choice's options, in index order. Empty otherwise.
    pub options: Vec<String>,
    /// What the number is shown with: nothing, `px`, `°` or `°/s`.
    pub suffix: String,
    pub min: f32,
    pub max: f32,
    pub hard_min: f32,
    pub hard_max: f32,
    pub default: f32,
}

/// A flare the presets strip offers: built in, or one of the user's own.
#[derive(Debug, Clone)]
pub struct BridgeFlarePreset {
    pub name: String,
    /// Empty for the user's own.
    pub group: String,
    pub json: String,
    pub built_in: bool,
}

/// The kind's JSON key, which is its serde name.
fn kind_key(kind: Kind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn kind_of(key: &str) -> Option<Kind> {
    serde_json::from_value(serde_json::Value::String(key.to_owned())).ok()
}

/// Every kind, in Add menu order.
#[frb(sync)]
pub fn flare_kinds() -> Vec<BridgeFlareKind> {
    Kind::ALL
        .iter()
        .map(|&k| BridgeFlareKind {
            key: kind_key(k),
            label: k.label().to_owned(),
            place: k.place().label().to_owned(),
        })
        .collect()
}

/// The controls `kind` shows, in the window's order. Empty for a key that is
/// not a kind.
#[frb(sync)]
pub fn flare_controls(kind: String) -> Vec<BridgeFlareControl> {
    let Some(kind) = kind_of(&kind) else {
        return Vec::new();
    };
    controls::controls(kind)
        .map(|c| BridgeFlareControl {
            key: c.key.to_owned(),
            label: c.label.to_owned(),
            group: c.group.to_owned(),
            edit: match c.edit {
                Edit::Slider => BridgeFlareEdit::Slider,
                Edit::Counter => BridgeFlareEdit::Counter,
                Edit::Toggle => BridgeFlareEdit::Toggle,
                Edit::Angle => BridgeFlareEdit::Angle,
                Edit::Choice(_) => BridgeFlareEdit::Choice,
            },
            options: match c.edit {
                Edit::Choice(opts) => opts.iter().map(|&o| o.to_owned()).collect(),
                _ => Vec::new(),
            },
            suffix: match c.unit {
                Unit::None => "",
                Unit::Px => "px",
                Unit::Degrees => "\u{b0}",
                Unit::DegreesPerSecond => "\u{b0}/s",
            }
            .to_owned(),
            min: c.min,
            max: c.max,
            hard_min: c.hard_min,
            hard_max: c.hard_max,
            default: c.default,
        })
        .collect()
}

/// A fresh element of `kind` as JSON, every value at its default.
#[frb(sync)]
pub fn flare_new_element(kind: String) -> String {
    kind_of(&kind)
        .map(|k| serde_json::to_string(&Element::new(k)).unwrap_or_default())
        .unwrap_or_default()
}

/// The colour modes, in the segmented control's order.
#[frb(sync)]
pub fn flare_colour_modes() -> Vec<BridgeFlareOption> {
    ColourMode::ALL
        .iter()
        .map(|&m| BridgeFlareOption {
            key: serde_json::to_value(m)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            label: m.label().to_owned(),
        })
        .collect()
}

/// At most this many pixels a preview is drawn at: a thumbnail or the
/// window's own box, never a frame.
const PREVIEW_MAX_PX: u32 = 4096 * 2304;

/// The design drawn on black with one white light at (`light_x`, `light_y`)
/// in preview pixels, the centre in the middle, `px_scale` preview pixels
/// per comp pixel. RGBA8, sRGB encoded the way the Viewer shows scene-linear
/// light. Empty when the size is nothing or too big. Not `sync`: it runs off
/// the interface thread.
pub fn flare_preview(
    design_json: String,
    width: u32,
    height: u32,
    light_x: f32,
    light_y: f32,
    px_scale: f32,
) -> Vec<u8> {
    let px = width.saturating_mul(height);
    if px == 0 || px > PREVIEW_MAX_PX {
        return Vec::new();
    }
    let design = Design::parse(&design_json).unwrap_or_else(presets::default_design);
    let frame = raster::Frame::preview(width, height, [light_x, light_y], px_scale.max(1e-3));
    let mut rgba = vec![0.0_f32; px as usize * 4];
    for a in rgba.iter_mut().skip(3).step_by(4) {
        *a = 1.0;
    }
    raster::draw(&design, &frame, width, height, &mut rgba);
    rgba.chunks_exact(4)
        .flat_map(|p| [srgb_encode(p[0]), srgb_encode(p[1]), srgb_encode(p[2]), 255])
        .collect()
}

/// The user's `.flare` files, by name.
fn user_flares() -> Vec<(String, std::path::PathBuf, Design)> {
    let Some(dir) = lumit_project::flares_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, std::path::PathBuf, Design)> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("flare"))
            {
                return None;
            }
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let mut design = Design::parse(&std::fs::read_to_string(&path).ok()?)?;
            if design.name.trim().is_empty() {
                design.name = stem;
            }
            Some((design.name.clone(), path, design))
        })
        .collect();
    out.sort_by_key(|(name, _, _)| name.to_lowercase());
    out
}

/// The built-in flares, then the user's own.
#[frb(sync)]
pub fn flare_presets() -> Vec<BridgeFlarePreset> {
    let built_in = presets::BUILT_IN.iter().filter_map(|&(group, json)| {
        let d = Design::parse(json)?;
        Some(BridgeFlarePreset {
            name: d.name,
            group: group.to_owned(),
            json: json.to_owned(),
            built_in: true,
        })
    });
    let yours = user_flares()
        .into_iter()
        .map(|(name, _, d)| BridgeFlarePreset {
            name,
            group: String::new(),
            json: d.to_json(),
            built_in: false,
        });
    built_in.chain(yours).collect()
}

/// A file stem for `name`, numbered when the name is taken.
fn fresh_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
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
    let mut p = dir.join(format!("{stem}.flare"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem} {n}.flare"));
        n += 1;
    }
    p
}

/// Write `design` to the user's folder under its name and answer the name it
/// was saved as, which is numbered when the name was taken.
fn write_user_flare(mut design: Design) -> Result<String, BridgeError> {
    let name = design.name.trim().to_owned();
    if name.is_empty() {
        return Err(BridgeError::EmptyName);
    }
    let dir = lumit_project::flares_dir().ok_or(BridgeError::WriteFailed)?;
    std::fs::create_dir_all(&dir).map_err(|_| BridgeError::WriteFailed)?;
    let path = fresh_path(&dir, &name);
    design.name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(name);
    std::fs::write(&path, design.to_json()).map_err(|_| BridgeError::WriteFailed)?;
    Ok(design.name)
}

/// Save a design to the user's flares and answer the name it got.
///
/// # Errors
/// [`BridgeError::ReadFailed`] when `json` is not a design,
/// [`BridgeError::EmptyName`] for a blank name, [`BridgeError::WriteFailed`]
/// when the folder cannot be written.
pub fn save_flare(json: String) -> Result<String, BridgeError> {
    write_user_flare(Design::parse(&json).ok_or(BridgeError::ReadFailed)?)
}

fn user_flare_path(name: &str) -> Result<std::path::PathBuf, BridgeError> {
    user_flares()
        .into_iter()
        .find(|(n, _, _)| n == name)
        .map(|(_, path, _)| path)
        .ok_or(BridgeError::ReadFailed)
}

/// Delete one of the user's flares by name.
///
/// # Errors
/// [`BridgeError::ReadFailed`] when there is no such flare,
/// [`BridgeError::WriteFailed`] when the file will not go.
pub fn delete_flare(name: String) -> Result<(), BridgeError> {
    std::fs::remove_file(user_flare_path(&name)?).map_err(|_| BridgeError::WriteFailed)
}

/// Rename one of the user's flares and answer the name it got.
///
/// # Errors
/// As [`save_flare`] and [`delete_flare`].
pub fn rename_flare(name: String, new_name: String) -> Result<String, BridgeError> {
    let path = user_flare_path(&name)?;
    let mut design =
        Design::parse(&std::fs::read_to_string(&path).map_err(|_| BridgeError::ReadFailed)?)
            .ok_or(BridgeError::ReadFailed)?;
    design.name = new_name;
    let saved = write_user_flare(design)?;
    std::fs::remove_file(path).map_err(|_| BridgeError::WriteFailed)?;
    Ok(saved)
}

/// Read a `.flare` file anywhere on disk and answer its design as JSON.
///
/// # Errors
/// [`BridgeError::ReadFailed`] when the file cannot be read or is not a design.
pub fn import_flare(path: String) -> Result<String, BridgeError> {
    let text = std::fs::read_to_string(&path).map_err(|_| BridgeError::ReadFailed)?;
    let mut design = Design::parse(&text).ok_or(BridgeError::ReadFailed)?;
    if design.name.trim().is_empty() {
        design.name = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    Ok(design.to_json())
}

/// Write a design to `path` as a `.flare` file.
///
/// # Errors
/// [`BridgeError::ReadFailed`] when `json` is not a design,
/// [`BridgeError::WriteFailed`] when the file cannot be written.
pub fn export_flare(json: String, path: String) -> Result<(), BridgeError> {
    let design = Design::parse(&json).ok_or(BridgeError::ReadFailed)?;
    std::fs::write(path, design.to_json()).map_err(|_| BridgeError::WriteFailed)
}

impl CompositionReference {
    /// A still of this composition at `frame`, its longest edge `max_edge`
    /// pixels, with effect `effect` switched off wherever it sits: the Flare
    /// window's Frame background, which the flare preview is then added over.
    /// Rides the welcome row's thumbnail road, so it is small and asked for
    /// once. `None` on a machine with no graphics adapter. Not `sync`: it
    /// renders.
    ///
    /// # Errors
    /// [`BridgeError::InvalidComp`] when this is not a composition.
    pub fn still_without_effect(
        &self,
        frame: u64,
        max_edge: u32,
        effect: uuid::Uuid,
    ) -> Result<Option<BridgeRenderedFrame>, BridgeError> {
        let mut document = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            (*proj.store.snapshot()).clone()
        };
        let comp = document.comp_mut(self.id).ok_or(BridgeError::InvalidComp)?;
        let longest = comp.width.max(comp.height).max(1);
        // The flare may be on a layer, on a clip, or an Fx box in the comp's
        // own graph. All of them are looked at, and only the one that matches
        // is turned off.
        let mut off = |i: &mut lumit_core::model::EffectInstance| {
            if i.id == effect {
                i.enabled = false;
            }
        };
        for layer in &mut comp.layers {
            layer.effects.iter_mut().for_each(&mut off);
            if let lumit_core::model::LayerKind::Sequence { clips } = &mut layer.kind {
                clips
                    .iter_mut()
                    .flat_map(|c| c.effects.iter_mut())
                    .for_each(&mut off);
            }
        }
        if let Some(graph) = comp.graph.as_mut() {
            for node in &mut graph.nodes {
                if let lumit_core::comp_graph::GraphNode::Fx(i) = node {
                    off(i);
                }
            }
        }
        let scale = (f64::from(max_edge.max(1)) / f64::from(longest)).clamp(1e-4, 1.0) as f32;
        let document = std::sync::Arc::new(document);
        Ok(
            crate::render::thumbnail(&document, self.id, frame, scale).map(
                |(rgba, width, height)| BridgeRenderedFrame {
                    frame,
                    width,
                    height,
                    rgba,
                },
            ),
        )
    }
}

impl BridgeEffectInstance {
    /// The design this Sprite flare keeps, or the default design when it
    /// keeps none, as JSON.
    #[frb(sync)]
    pub fn flare_design(&self) -> String {
        design::embedded(&self.effect)
            .unwrap_or_else(presets::default_design)
            .to_json()
    }

    /// Keep `json` as the effect's design. Staged like `set_value`;
    /// `LayerReference::set_effects` commits it.
    ///
    /// # Errors
    /// [`BridgeError::ReadFailed`] when `json` is not a design.
    #[frb(sync)]
    pub fn set_flare_design(&mut self, json: String) -> Result<(), BridgeError> {
        let d = Design::parse(&json).ok_or(BridgeError::ReadFailed)?;
        design::set_embedded(&mut self.effect, Some(&d));
        Ok(())
    }
}
