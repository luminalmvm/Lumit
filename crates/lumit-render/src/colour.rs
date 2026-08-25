//! The project's OCIO config, as the renderer holds it (K-490,
//! docs/impl/ocio.md §3.2, §3.3, §5.2).
//!
//! **In plain terms.** The document stores a *name*: the path to a config file.
//! Everything else — the parsed config, the chains it resolves to, the baked
//! tables the graphics card samples — is worked out from that file and thrown
//! away freely, exactly as decoded footage is. This module is where that
//! working-out lives.
//!
//! Two things it is careful about.
//!
//! The first is the **calm degrade** (§3.3). A config that has moved, cannot be
//! read, or asks for something Lumit does not implement must never hold the
//! project hostage. So a failure here is a *state*, not an error: the config
//! becomes "not usable, and here is the one sentence saying why", every name
//! the project was given is kept, and the preview falls back to the built-in
//! colour family so a frame is always produced. The export does the opposite
//! and refuses — a wrong colour space in a delivered file is worse than an
//! export that did not run. That asymmetry is deliberate and is the whole of
//! K-490's degrade rule.
//!
//! The second is that **nothing here is a second implementation**. A chain is
//! resolved by `lumit-colour`, baked by `lumit-colour`, and the baked table is
//! handed to the graphics card. The Viewer and the export bind the same table
//! to the same pass, so preview equals export because they are one dispatch
//! rather than two implementations that agree (K-031, K-185).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use lumit_colour::{bake, Artefact, LoadedConfig, Shaper, Stage};
use lumit_core::model::Document;

/// Which transform a baked artefact is, so two of them are never confused for
/// each other in the cache.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Edge {
    /// A footage item's colour space → the working space.
    Input(String),
    /// The working space → what a display shows through a view.
    DisplayView(String, String),
    /// The working space → a named space, which is what an export delivers.
    Output(String),
}

/// A refusal as the frontend needs it: the stable id it writes its own
/// translated sentence from, and the facts that fill it, by name.
///
/// The facts are the **config's own words** — a colour space, a display, a
/// look-up-table path — so they cross verbatim and are never translated, in
/// exactly the way a codec name is not (K-303).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// `lumit_colour::ColourError::key`, or `config_unreadable` for the one
    /// refusal this module raises itself: the file the document names is not
    /// there to be parsed at all.
    pub key: String,
    pub args: Vec<(String, String)>,
}

impl Refusal {
    /// The refusal a `lumit-colour` error is, optionally noting which colour
    /// space was being resolved when it surfaced — `in_space` rather than
    /// `space`, because a refusal may already name a space of its own.
    fn from_error(e: &lumit_colour::ColourError, in_space: Option<&str>) -> Refusal {
        let mut args: Vec<(String, String)> = e
            .args()
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        if let Some(space) = in_space {
            args.push(("in_space".to_string(), space.to_string()));
        }
        Refusal {
            key: e.key().to_string(),
            args,
        }
    }
}

/// A config the renderer has tried to load, in whichever of the three states it
/// ended up in.
pub struct Loaded {
    /// The path the document named, for the sentence the frontend shows.
    pub path: String,
    /// The content hash of the config file. Part of every frame's name, so
    /// editing the config on disk retires the frames it made.
    pub hash: u64,
    /// `None` when the config could not be loaded or is not usable — the state
    /// the degrade ladder reads.
    config: Option<LoadedConfig>,
    /// One calm sentence, present exactly when `config` is `None`.
    pub problem: Option<String>,
    /// The same refusal, keyed — present exactly when `problem` is.
    ///
    /// `problem` is the engine's English, which is a fallback and nothing more:
    /// every one of these sentences has a config's own name or a file path in
    /// the middle of it, so no whole-text lookup could ever translate one. The
    /// frontend writes its own sentence from this id and its named facts
    /// (K-005, K-303, docs/17), exactly as the After Effects import report's
    /// rows are written.
    pub refusal: Option<Refusal>,
    /// Baked artefacts by edge, worked out on first use and kept. Behind a
    /// lock because a render borrows this immutably and may want an edge it has
    /// not needed before; the lock is never held across a GPU submit.
    baked: std::sync::Mutex<BTreeMap<Edge, Option<Arc<Artefact>>>>,
}

impl Loaded {
    /// Whether this config can actually do colour. The one question the degrade
    /// ladder asks, and the one the export's refusal asks.
    #[must_use]
    pub fn usable(&self) -> bool {
        self.config.is_some()
    }

    /// The names a picker lists, in the config's own order — active spaces
    /// only, then each display with its views. Empty when unusable.
    #[must_use]
    pub fn vocabulary(&self) -> (Vec<String>, Vec<(String, Vec<String>)>) {
        let Some(c) = &self.config else {
            return (Vec::new(), Vec::new());
        };
        let spaces = c
            .config
            .active_space_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let displays = c
            .config
            .displays
            .iter()
            .map(|d| {
                (
                    d.name.clone(),
                    d.views.iter().map(|v| v.name.clone()).collect(),
                )
            })
            .collect();
        (spaces, displays)
    }

    /// The baked table for one edge, or `None` if this config cannot make it.
    ///
    /// Resolution and baking happen once per edge and are kept: a config is
    /// immutable, so the answer cannot change while it is loaded. A refusal is
    /// cached too — asking again would refuse again, and re-resolving a chain
    /// on every frame to be told no is the kind of cost that only shows up on
    /// someone else's machine.
    pub fn artefact(&self, edge: &Edge) -> Option<Arc<Artefact>> {
        if let Ok(cache) = self.baked.lock() {
            if let Some(hit) = cache.get(edge) {
                return hit.clone();
            }
        }
        let made = self.bake_edge(edge).map(Arc::new);
        if let Ok(mut cache) = self.baked.lock() {
            cache.insert(edge.clone(), made.clone());
        }
        made
    }

    fn bake_edge(&self, edge: &Edge) -> Option<Artefact> {
        let c = self.config.as_ref()?;
        let (chain, shaper) = match edge {
            Edge::Input(name) => (c.from_space(name), c.shaper_for(name)),
            // The working space is the domain of both of these, and the
            // document has no name for it — so the default shaper stands, which
            // is what §5.1 says it is for.
            Edge::DisplayView(display, view) => (c.display_view(display, view), Shaper::DEFAULT),
            Edge::Output(name) => (c.to_space(name), Shaper::DEFAULT),
        };
        bake(&chain.ok()?, shaper).ok()
    }
}

/// The renderer's colour state: at most one config, reloaded when the document
/// points somewhere else or the file underneath changes.
#[derive(Default)]
pub struct ColourState {
    loaded: Option<Arc<Loaded>>,
    /// What `loaded` was built from, so an unchanged document costs one
    /// comparison rather than one parse.
    from: Option<(String, u64)>,
}

impl ColourState {
    /// Bring the state into line with the document. Cheap and safe to call at
    /// the top of every render: it reads the config file's bytes and returns
    /// immediately if they hash to what is already loaded.
    pub fn sync(&mut self, doc: &Document) {
        let Some(media) = &doc.colour.config else {
            self.loaded = None;
            self.from = None;
            return;
        };
        let path = if media.absolute_path.is_empty() {
            media.relative_path.clone()
        } else {
            media.absolute_path.clone()
        };
        let bytes = std::fs::read(&path).ok();
        let hash = bytes.as_ref().map_or(0, |b| content_hash(b));
        if self.from.as_ref() == Some(&(path.clone(), hash)) {
            return;
        }
        self.from = Some((path.clone(), hash));
        self.loaded = Some(Arc::new(load(&path, hash, bytes.is_some())));
    }

    /// The config in force, in whichever state it is. `None` means the project
    /// names none at all, which is the built-in family and today's behaviour.
    #[must_use]
    pub fn loaded(&self) -> Option<&Arc<Loaded>> {
        self.loaded.as_ref()
    }

    /// Whether a named colour space can actually be delivered right now — the
    /// question an export's refusal asks (K-479, K-490), and the one the export
    /// dialogue asks of every name it offers before it draws the row live.
    ///
    /// It is the half of K-490's asymmetry that says no: a preview degrades to
    /// the built-in transform, a delivery does not. `false` for a config that is
    /// missing or refused, and for a name this config does not have.
    ///
    /// Answered here rather than on the renderer because the seam has no
    /// renderer in hand — the colour state is the whole of what the question
    /// needs, and one implementation is what stops the dialogue and the
    /// exporter disagreeing about the same name.
    #[must_use]
    pub fn can_deliver(&self, name: &str) -> bool {
        self.loaded
            .as_ref()
            .filter(|l| l.usable())
            .and_then(|l| l.artefact(&Edge::Output(name.to_string())))
            .is_some()
    }

    /// The config's identity for a frame's name (§5.5). Zero when there is
    /// none, so a project without a config names its frames exactly as it did
    /// before this existed, and an unusable config names them as no config at
    /// all — which it is, for rendering.
    #[must_use]
    pub fn frame_identity(&self) -> u64 {
        match &self.loaded {
            Some(l) if l.usable() => l.hash,
            _ => 0,
        }
    }
}

/// Load and check a config, turning every failure into a state rather than an
/// error (§3.3).
fn load(path: &str, hash: u64, present: bool) -> Loaded {
    let empty = |problem: String, refusal: Refusal| Loaded {
        path: path.to_string(),
        hash,
        config: None,
        problem: Some(problem),
        refusal: Some(refusal),
        baked: std::sync::Mutex::new(BTreeMap::new()),
    };
    if !present {
        return empty(
            format!(
                "the colour config {path} could not be read, so the built-in transform is in use"
            ),
            Refusal {
                key: "config_unreadable".to_string(),
                args: vec![("path".to_string(), path.to_string())],
            },
        );
    }
    let loaded = match LoadedConfig::load(Path::new(path)) {
        Ok(l) => l,
        Err(e) => return empty(e.to_string(), Refusal::from_error(&e, None)),
    };
    // `unresolvable` is the crate's own is-this-config-usable answer: every
    // colour space the config declares, walked to a chain. A config that names
    // a transform Lumit does not implement is refused **by name** here rather
    // than being discovered halfway through a render, and the sentence it
    // refuses with is the one the frontend shows.
    if let Some((space, why)) = lumit_colour::resolve::unresolvable(&loaded)
        .into_iter()
        .next()
    {
        let refusal = Refusal::from_error(&why, Some(&space));
        return empty(format!("{why} (the colour space {space})"), refusal);
    }
    Loaded {
        path: path.to_string(),
        hash,
        config: Some(loaded),
        problem: None,
        refusal: None,
        baked: std::sync::Mutex::new(BTreeMap::new()),
    }
}

/// The config file's content hash, folded to the 64 bits a frame key wants.
fn content_hash(bytes: &[u8]) -> u64 {
    // ponytail: the config file's bytes only. §5.5 also names the resolved
    // look-up-table files' bytes, which `lumit-colour` does not report back;
    // editing a `.spi3d` in place without touching `config.ocio` therefore
    // keeps the frames it made. Reloading the project picks it up. The upgrade
    // is a `files_read()` accessor on `LoadedConfig` folded in here.
    let d = blake3::hash(bytes);
    let mut k = [0u8; 8];
    k.copy_from_slice(&d.as_bytes()[..8]);
    // Zero is "no config" (see `frame_identity`), so nudge the one-in-2^64 hash
    // that lands there rather than letting it mean the wrong thing.
    u64::from_le_bytes(k) | 1
}

/// A baked artefact as the tables the graphics card takes (`lumit-gpu` depends
/// on no other Lumit crate, so the shapes are converted rather than shared).
#[must_use]
pub fn tables(artefact: &Artefact) -> lumit_gpu::OcioTables {
    let shaper = |s: &Shaper| match *s {
        Shaper::Lg2 {
            min_log2,
            max_log2,
            offset,
        } => lumit_gpu::OcioShaper::Lg2 {
            min_log2,
            max_log2,
            offset,
        },
        Shaper::Uniform { min, max } => lumit_gpu::OcioShaper::Uniform { min, max },
    };
    match artefact {
        Artefact::ShaperCube { shaper: s, cube } => lumit_gpu::OcioTables::Cube {
            size: cube.size as u32,
            data: cube.data.clone(),
            shaper: shaper(s),
        },
        Artefact::Factorised { .. } => {
            // `bake` guarantees the fixed shape, so this cannot be `None` — but
            // an engine crate answers rather than unwraps (docs/14 §4), and the
            // identity is the honest answer to "a shape I cannot execute".
            let (pre, matrix, post) =
                artefact
                    .fixed_shape()
                    .unwrap_or((None, lumit_colour::matrix::IDENTITY, None));
            let curve_shaper = pre
                .or(post)
                .map_or(lumit_colour::bake::CURVE_SHAPER, |c| c.shaper);
            lumit_gpu::OcioTables::Curves {
                pre: pre.map(|c| c.table.data.clone()),
                // Double on the way here so compositions cancel exactly; single
                // from here on, because this is what the card multiplies and
                // the CPU sampler rounds to the same twelve numbers (K-031).
                matrix: lumit_colour::matrix::single(&matrix),
                post: post.map(|c| c.table.data.clone()),
                shaper: shaper(&curve_shaper),
            }
        }
    }
}

/// The footage colour space a layer's pixels arrived in, if the layer is
/// footage and somebody has said (K-490).
#[must_use]
pub fn footage_colour_space(doc: &Document, kind: &lumit_core::model::LayerKind) -> Option<String> {
    let lumit_core::model::LayerKind::Footage { item } = kind else {
        return None;
    };
    match doc.item(*item) {
        Some(lumit_core::model::ProjectItem::Footage(f)) => f.colour_space.clone(),
        _ => None,
    }
}

/// The input transforms one frame's draws may ask for, baked and uploaded.
///
/// Built once per render rather than per draw: a comp with forty layers of one
/// camera's footage wants one table, not forty. A name with no entry — and a
/// name the loaded config does not have — simply is not here, and the realiser
/// falls back to the built-in interpretation.
#[derive(Default)]
pub struct InputTransforms {
    by_space: BTreeMap<String, lumit_gpu::OcioArtefact>,
}

impl InputTransforms {
    /// Bake and upload one artefact per distinct colour space named by the
    /// document's footage items.
    #[must_use]
    pub fn build(
        doc: &Document,
        state: &ColourState,
        ctx: &lumit_gpu::GpuContext,
        engine: &lumit_gpu::ColourEngine,
    ) -> Self {
        let mut by_space = BTreeMap::new();
        let Some(loaded) = state.loaded().filter(|l| l.usable()) else {
            return Self { by_space };
        };
        for item in &doc.items {
            let lumit_core::model::ProjectItem::Footage(f) = item else {
                continue;
            };
            let Some(space) = &f.colour_space else {
                continue;
            };
            if by_space.contains_key(space) {
                continue;
            }
            if let Some(a) = loaded.artefact(&Edge::Input(space.clone())) {
                by_space.insert(space.clone(), engine.upload_ocio(ctx, &tables(&a)));
            }
        }
        Self { by_space }
    }

    #[must_use]
    pub fn get(&self, space: &str) -> Option<&lumit_gpu::OcioArtefact> {
        self.by_space.get(space)
    }

    /// Whether this frame needs any input transform at all — the cheap check
    /// that keeps an ordinary project's render exactly as it was.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_space.is_empty()
    }
}

/// Whether an artefact is the factorised shape, for the tests that care which
/// form a chain took.
#[must_use]
pub fn is_factorised(artefact: &Artefact) -> bool {
    matches!(artefact, Artefact::Factorised { .. })
}

/// How many stages a factorised artefact has — the number the fixed-shape
/// guard bounds at three.
#[must_use]
pub fn stage_count(artefact: &Artefact) -> usize {
    match artefact {
        Artefact::Factorised { stages } => stages.len(),
        Artefact::ShaperCube { .. } => 0,
    }
}

/// The stages, for a test that wants to look at them.
#[must_use]
pub fn stages(artefact: &Artefact) -> &[Stage] {
    match artefact {
        Artefact::Factorised { stages } => stages,
        Artefact::ShaperCube { .. } => &[],
    }
}
