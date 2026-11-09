//! Lottie import: a `.json` or a `.lottie` as comps, with a report of what
//! didn't come across as it was.
//!
//! # In plain terms
//!
//! A Lottie is a comp written as layers, shapes and keys, much as Lumit keeps
//! one, so most of it maps straight across: a shape layer is a Shape layer, a
//! precomp is a comp placed by a Precomp layer, a key's easing is the same
//! curve ([`keys`]). What has no place in Lumit (a skew, a zig zag, an effect
//! beyond the five a Lottie can carry) is left out and named in the report,
//! and what arrives in another form (a motion path, a group that moves) is
//! rebuilt so it looks the same and named too.
//!
//! The result is the items to add, the main comp first, and the pictures to
//! write beside the file before they are added. The caller adds them in one
//! step, so one undo takes the whole import away.
//!
//! The file is a stranger's: its size, its counts and its nesting are held to
//! [`LIMITS`] before anything is kept.
//!
//! Runs on the caller's thread, a bridge worker or the command line's.

mod assets;
mod extras;
mod keys;
mod layers;
mod paints;
mod read;
mod shapes;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lumit_core::model::{Folder, ProjectItem};
use lumit_import::report::{ItemPath, Outcome, Reason, ReportRow};
use lumit_ingress::{Budget, IngressError, Limits};
use serde_json::Value;
use uuid::Uuid;

use crate::LottieError;
use read::{Clock, Read, Version};

/// What an import made, ready to add.
#[derive(Debug, Clone)]
pub struct Imported {
    /// Comps (the main one first), solids, folders and footage, in the order to
    /// add them.
    pub items: Vec<ProjectItem>,
    /// The main comp.
    pub main: Uuid,
    /// Pictures to write to disk before the items are added: where, and the
    /// bytes.
    pub files: Vec<(PathBuf, Vec<u8>)>,
    /// What didn't come across as it was.
    pub report: Vec<ReportRow>,
    /// How many of the file's layers came across.
    pub layers: u32,
}

/// What an import may spend: a quarter of a gigabyte of file, a million keys,
/// vertices and layers, and nesting 64 deep.
pub const LIMITS: Limits = Limits {
    bytes: 256 << 20,
    items: 1_000_000,
    depth: 64,
    work: 64_000_000,
};

impl From<IngressError> for LottieError {
    fn from(e: IngressError) -> Self {
        LottieError::TooLarge(e.to_string())
    }
}

/// The Lottie in `bytes`, a `.json` or a `.lottie`, as items to add. `source`
/// is the file it was read from: it names the comp and the folder, and the
/// pictures are written beside it, in `<stem> images`.
pub fn import(bytes: &[u8], source: &Path) -> Result<Imported, LottieError> {
    let mut budget = Budget::new(LIMITS);
    budget.take_bytes(u64::try_from(bytes.len()).map_err(|_| IngressError::Overflow)?)?;
    let (json, zipped) = if bytes.starts_with(b"PK\x03\x04") {
        assets::unzip(bytes, &mut budget)?
    } else {
        (bytes.to_vec(), BTreeMap::new())
    };
    let root: Value = serde_json::from_slice(&json).map_err(|_| LottieError::NotLottie)?;
    let layers = root
        .get("layers")
        .and_then(Value::as_array)
        .ok_or(LottieError::NotLottie)?;

    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| String::from("Lottie"));
    let beside = source.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = read::text(&root, "nm")
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(&stem)
        .to_owned();

    let mut report = Vec::new();
    let clock = match read::num(root.get("fr")).and_then(Clock::of) {
        Some(clock) => clock,
        None => {
            report.push(ReportRow {
                path: ItemPath::item(&name),
                outcome: Outcome::Adjusted,
                reason: Reason::CompFrameRateGuessed { used: 30.0 },
            });
            Clock::of(30.0).ok_or(LottieError::NotLottie)?
        }
    };

    let mut cx = Cx {
        clock,
        budget,
        version: Version::of(root.get("v")),
        report,
        assets: read::list(&root, "assets")
            .iter()
            .filter_map(|a| Some((a.get("id").and_then(id_text)?, a)))
            .collect(),
        fonts: read::list(root.get("fonts").unwrap_or(&Value::Null), "list")
            .iter()
            .filter_map(|f| {
                Some((
                    read::text(f, "fName")?.to_owned(),
                    (
                        read::text(f, "fFamily").unwrap_or_default().to_owned(),
                        read::text(f, "fStyle").unwrap_or_default().to_owned(),
                    ),
                ))
            })
            .collect(),
        images: BTreeMap::new(),
        comps: BTreeMap::new(),
        made: Vec::new(),
        files: Vec::new(),
        zipped,
        written: BTreeMap::new(),
        folder: beside.join(format!("{stem} images")),
        beside,
        depth: 0,
        offset: 0.0,
        layers: 0,
    };

    let size = (dimension(root.get("w")), dimension(root.get("h")));
    let span = (
        read::num(root.get("ip")).unwrap_or(0.0),
        read::num(root.get("op")).unwrap_or(0.0),
    );
    let mut main = layers::comp(&mut cx, &name, size, layers, span)?;
    main.markers = layers::markers(&cx, read::list(&root, "markers"));
    let main_id = main.id;

    let mut items = vec![ProjectItem::Composition(main)];
    let children: Vec<Uuid> = cx.made.iter().map(ProjectItem::id).collect();
    items.append(&mut cx.made);
    if !children.is_empty() {
        items.push(ProjectItem::Folder(Folder {
            id: Uuid::now_v7(),
            name: stem,
            children,
            extra: serde_json::Map::new(),
        }));
    }
    Ok(Imported {
        items,
        main: main_id,
        files: cx.files,
        report: cx.report,
        layers: cx.layers,
    })
}

/// A comp's side in pixels, held to what a comp can be.
fn dimension(v: Option<&Value>) -> u32 {
    read::num(v).map_or(512, |n| n.round().clamp(1.0, 16384.0) as u32)
}

/// An id written as text or as a number.
fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// How deep comps may nest inside each other, as the budget's depth.
const MAX_DEPTH: u32 = LIMITS.depth;

/// The import's running state.
struct Cx<'a> {
    clock: Clock,
    budget: Budget,
    version: Version,
    report: Vec<ReportRow>,
    /// The file's assets by id: pictures and precomps.
    assets: BTreeMap<String, &'a Value>,
    /// Font names as text documents give them, as family and face.
    fonts: BTreeMap<String, (String, String)>,
    /// Picture assets already made into footage.
    images: BTreeMap<String, Uuid>,
    /// Precomp assets already made into comps; `None` while one is being made,
    /// so a precomp that places itself is caught.
    comps: BTreeMap<String, Option<Uuid>>,
    /// Items made on the way: precomps, solids and footage.
    made: Vec<ProjectItem>,
    files: Vec<(PathBuf, Vec<u8>)>,
    /// A `.lottie`'s pictures, by their path in the zip.
    zipped: BTreeMap<String, Vec<u8>>,
    /// Where each zip entry already taken was written, by its path in the zip.
    written: BTreeMap<String, PathBuf>,
    /// Where pictures carried in the file are written.
    folder: PathBuf,
    /// The folder the file sits in, which a linked picture is found from.
    beside: PathBuf,
    /// How many precomps deep the walk is.
    depth: u32,
    /// The start (`st`) of the layer being read, in frames: its keys are in
    /// its comp's frames, and Lumit keeps them on the layer's own clock.
    offset: f64,
    layers: u32,
}

impl Cx<'_> {
    /// Add a row to the report.
    fn row(&mut self, path: ItemPath, outcome: Outcome, reason: Reason) {
        self.report.push(ReportRow {
            path,
            outcome,
            reason,
        });
    }

    /// Something left out, by name.
    fn left_out(&mut self, path: ItemPath, name: &str) {
        self.row(
            path,
            Outcome::Skipped,
            Reason::ShapeOperatorNotCarried {
                name: name.to_owned(),
            },
        );
    }

    /// Something drawn another way, by name.
    fn approximated(&mut self, path: ItemPath, name: &str) {
        self.row(
            path,
            Outcome::Adjusted,
            Reason::ShapeOperatorApproximated {
                name: name.to_owned(),
            },
        );
    }

    /// A numeric property, its expression reported: the value under it is
    /// what arrives.
    fn read(
        &mut self,
        path: &ItemPath,
        name: &str,
        p: Option<&Value>,
    ) -> Result<Option<Read>, LottieError> {
        self.read_mapped(path, name, p, &|v| v)
    }

    /// [`Self::read`], each value put through `map`.
    fn read_mapped(
        &mut self,
        path: &ItemPath,
        name: &str,
        p: Option<&Value>,
        map: &dyn Fn(f64) -> f64,
    ) -> Result<Option<Read>, LottieError> {
        let read = read::property(p, &self.clock, self.offset, &mut self.budget, map)?;
        if let Some(source) = read.as_ref().and_then(|r| r.expression.as_ref()) {
            let source: String = source.chars().take(200).collect();
            self.row(
                path.property(name),
                Outcome::Adjusted,
                Reason::ExpressionNotRunnable { source },
            );
        }
        Ok(read)
    }

    /// One number of a property, or `default`.
    fn one(
        &mut self,
        path: &ItemPath,
        name: &str,
        p: Option<&Value>,
        default: f64,
    ) -> Result<lumit_core::anim::Property, LottieError> {
        let read = self.read(path, name, p)?;
        Ok(keys::one(read.as_ref(), 0, &self.clock, default))
    }

    /// A keyed or still path property: its shape at rest and its keys.
    fn path(
        &mut self,
        p: Option<&Value>,
        closed: Option<bool>,
    ) -> Result<
        (
            lumit_core::mask::BezierPath,
            Vec<lumit_core::mask::PathKeyframe>,
        ),
        LottieError,
    > {
        let empty = lumit_core::mask::BezierPath {
            vertices: Vec::new(),
            closed: closed.unwrap_or(true),
        };
        let Some(k) = p.and_then(|p| p.get("k")) else {
            return Ok((empty, Vec::new()));
        };
        if !read::is_keyed(Some(k)) {
            let still = read::bezier(k, closed, &mut self.budget)?.unwrap_or(empty);
            return Ok((still, Vec::new()));
        }
        let raw = read::raw_keys(
            k.as_array().map_or(&[], Vec::as_slice),
            &mut self.budget,
            Some,
        )?;
        let mut keyed = Vec::with_capacity(raw.len());
        for key in &raw {
            let Some(path) = read::bezier(key.value, closed, &mut self.budget)? else {
                continue;
            };
            let side = |h: &(Vec<f64>, Vec<f64>), fallback: [f64; 2]| {
                [
                    read::pick(&h.0, 0, fallback[0]),
                    read::pick(&h.1, 0, fallback[1]),
                ]
            };
            keyed.push((
                key.t - self.offset,
                path,
                key.hold,
                side(&key.out, crate::keys::LINEAR_OUT),
                side(&key.inn, crate::keys::LINEAR_IN),
            ));
        }
        let keys = keys::path_keys(&keyed, &self.clock);
        let still = keys.first().map_or(empty, |k| k.path.clone());
        if keys.len() < 2 {
            return Ok((still, Vec::new()));
        }
        Ok((still, keys))
    }
}

#[cfg(test)]
mod tests;
