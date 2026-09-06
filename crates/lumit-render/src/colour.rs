//! The project's OCIO config, as the renderer holds it (docs/impl/ocio.md
//! §3.2, §3.3, §5.2).
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
//! the degrade rule.
//!
//! The second is that **nothing here is a second implementation**. A chain is
//! resolved by `lumit-colour`, baked by `lumit-colour`, and the baked table is
//! handed to the graphics card. The Viewer and the export bind the same table
//! to the same pass, so preview equals export because they are one dispatch
//! rather than two implementations that agree.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use lumit_colour::{bake, Artefact, Chain, Direction, LoadedConfig, Op, Shaper, Stage};
use lumit_core::model::{Document, WorkingSpace};

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
    /// One named space to another, an empty name being the working space on
    /// either side: the OCIO colour space transform effect (docs/08 §3.97).
    Convert { from: String, to: String },
    /// A named space (empty: the working space) → a display and view, or the
    /// reverse: the OCIO display transform effect.
    Display {
        input: String,
        display: String,
        view: String,
        inverse: bool,
    },
    /// A look between two named spaces (empty: the working space): the OCIO
    /// look transform effect.
    Look {
        input: String,
        look: String,
        output: String,
        inverse: bool,
    },
    /// Untagged footage under a config-defined working space: the built-in
    /// sRGB decode, then Rec.709 → the working space (docs/impl/ocio.md §2.1).
    Untagged,
    /// The built-in view under a config-defined working space: the working
    /// space → Rec.709, then the sRGB encode the hardware would have done.
    BuiltinDisplay,
}

/// What one slot of a layer's parallel colour-table list is made from
/// (docs/impl/effect-registry.md §2.5a): the k-th such slot belongs to the
/// k-th `lut` or OCIO op in the stack. Strings only - no GPU work happens
/// where these are built.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TableRequest {
    /// A `lut` effect's `.cube` path.
    Cube(String),
    /// One of the OCIO effects' tables.
    Ocio(OcioRequest),
}

/// An OCIO effect's table: an edge of the project's config, or a file read
/// on its own, which needs no config at all.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OcioRequest {
    Config(Edge),
    /// The OCIO file transform effect: a LUT or CDL file, forward or inverted.
    File {
        path: String,
        inverse: bool,
    },
}

/// A refusal as the frontend needs it: the stable id it writes its own
/// translated sentence from, and the facts that fill it, by name.
///
/// The facts are the **config's own words** — a colour space, a display, a
/// look-up-table path — so they cross verbatim and are never translated, in
/// exactly the way a codec name is not.
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
    /// Linear Rec.709 → the working space, when the project composites in
    /// the config's `scene_linear` and the config can say what that is.
    /// `None` is Lumit's own working space, or a config that cannot say.
    rec709_to_working: Option<lumit_colour::matrix::Matrix34>,
    /// One calm sentence, present exactly when `config` is `None`.
    pub problem: Option<String>,
    /// The same refusal, keyed — present exactly when `problem` is.
    ///
    /// `problem` is the engine's English, which is a fallback and nothing more:
    /// every one of these sentences has a config's own name or a file path in
    /// the middle of it, so no whole-text lookup could ever translate one. The
    /// frontend writes its own sentence from this id and its named facts
    /// (docs/17), exactly as the After Effects import report's
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

    /// The working space's name when the project composites in the config's
    /// `scene_linear`, `None` for Lumit's own linear Rec.709.
    #[must_use]
    pub fn working_space(&self) -> Option<&str> {
        self.config.as_ref()?.working_space()
    }

    /// Linear Rec.709 → the working space, or `None` when they are one and
    /// the same for rendering (see the field).
    #[must_use]
    pub fn rec709_to_working(&self) -> Option<&lumit_colour::matrix::Matrix34> {
        self.rec709_to_working.as_ref()
    }

    /// What the config calls itself, for the OCIO effects' Information row:
    /// its own name or description, or the file's name when it states
    /// neither. Empty when unusable.
    #[must_use]
    pub fn name(&self) -> String {
        let Some(c) = &self.config else {
            return String::new();
        };
        if !c.config.name.is_empty() {
            return c.config.name.clone();
        }
        Path::new(&self.path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The names a picker lists, in the config's own order — active spaces
    /// only, then each display with its views, then the looks. Empty when
    /// unusable.
    #[must_use]
    #[allow(clippy::type_complexity)]
    pub fn vocabulary(&self) -> (Vec<String>, Vec<(String, Vec<String>)>, Vec<String>) {
        let Some(c) = &self.config else {
            return (Vec::new(), Vec::new(), Vec::new());
        };
        let looks = c
            .config
            .look_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
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
        (spaces, displays, looks)
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
        // An effect's empty name is the working space, whose shaper is the
        // default one (§5.1): the document has no name for it.
        fn named(s: &str) -> Option<&str> {
            (!s.is_empty()).then_some(s)
        }
        let shaper_of = |s: &String| named(s).map_or(Shaper::DEFAULT, |n| c.shaper_for(n));
        let (chain, shaper) = match edge {
            Edge::Input(name) => (c.from_space(name), c.shaper_for(name)),
            // The working space is the domain of both of these, and the
            // document has no name for it — so the default shaper stands, which
            // is what §5.1 says it is for.
            Edge::DisplayView(display, view) => (c.display_view(display, view), Shaper::DEFAULT),
            Edge::Output(name) => (c.to_space(name), Shaper::DEFAULT),
            Edge::Convert { from, to } => (c.convert(named(from), named(to)), shaper_of(from)),
            // Inverted, the domain is the view's display encoding, which the
            // default shaper covers. A view with a 3D table in it refuses to
            // invert, by name, and the effect passes through.
            Edge::Display {
                input,
                display,
                view,
                inverse,
            } => (
                c.display_view_from(named(input), display, view)
                    .and_then(|chain| {
                        if *inverse {
                            chain.inverted(view)
                        } else {
                            Ok(chain)
                        }
                    }),
                if *inverse {
                    Shaper::DEFAULT
                } else {
                    shaper_of(input)
                },
            ),
            Edge::Look {
                input,
                look,
                output,
                inverse,
            } => {
                let spec = if *inverse {
                    format!("-{look}")
                } else {
                    look.clone()
                };
                (
                    c.look_between(named(input), &spec, named(output)),
                    shaper_of(input),
                )
            }
            // The two built-in edges: the sRGB curve the hardware applies,
            // with the primaries change beside it. Both refuse without the
            // matrix, which is when the hardware path is the right one.
            Edge::Untagged => {
                let m = *self.rec709_to_working.as_ref()?;
                (
                    Ok(Chain::new(vec![
                        srgb_curve(Direction::Forward),
                        Op::Matrix(m),
                    ])),
                    Shaper::DEFAULT,
                )
            }
            Edge::BuiltinDisplay => {
                let m = lumit_colour::matrix::invert(self.rec709_to_working.as_ref()?).ok()?;
                (
                    Ok(Chain::new(vec![
                        Op::Matrix(m),
                        srgb_curve(Direction::Inverse),
                    ])),
                    Shaper::DEFAULT,
                )
            }
        };
        bake(&chain.ok()?, shaper).ok()
    }
}

/// The sRGB transfer curve as a chain step: forward decodes, inverse encodes.
/// The same numbers the hardware's sRGB texture formats use, so a baked
/// built-in edge and the hardware path agree (the double-encode test).
fn srgb_curve(dir: Direction) -> Op {
    Op::MonCurve {
        gamma: [2.4; 3],
        offset: [0.055; 3],
        dir,
    }
}

impl Loaded {}

/// The renderer's colour state: at most one config, reloaded when the document
/// points somewhere else or the file underneath changes.
#[derive(Default)]
pub struct ColourState {
    loaded: Option<Arc<Loaded>>,
    /// What `loaded` was built from, so an unchanged document costs one
    /// comparison rather than one parse.
    from: Option<(String, u64, WorkingSpace)>,
    /// The look-up-table files the loaded config names
    /// ([`LoadedConfig::files_read`]), kept so the next sync can ask whether any
    /// of them has been edited — the file list is only knowable once the config
    /// has been parsed, and editing a LUT leaves the config's own bytes alone.
    files: Vec<std::path::PathBuf>,
}

impl ColourState {
    /// Bring the state into line with the document. Cheap and safe to call at
    /// the top of every render: it reads the config file's bytes and returns
    /// immediately if they hash to what is already loaded.
    pub fn sync(&mut self, doc: &Document) {
        let Some(media) = &doc.colour.config else {
            self.loaded = None;
            self.from = None;
            self.files.clear();
            return;
        };
        let path = if media.absolute_path.is_empty() {
            media.relative_path.clone()
        } else {
            media.absolute_path.clone()
        };
        let bytes = std::fs::read(&path).ok();
        // The LUT list belongs to the config already loaded, which is the only
        // list there is until this one is parsed: an edited LUT changes this
        // hash, the parse below hands back the list again, and a *changed* list
        // can only come from a changed config file, which changed the hash on
        // its own bytes.
        let working = doc.colour.working_space;
        let hash = bytes
            .as_ref()
            .map_or(0, |b| content_hash(b, self.files.as_slice(), working));
        if self.from.as_ref() == Some(&(path.clone(), hash, working)) {
            return;
        }
        let mut loaded = load(&path, hash, bytes.is_some(), working);
        self.files = loaded
            .config
            .as_ref()
            .map(LoadedConfig::files_read)
            .unwrap_or_default();
        // Re-fold with the list this config actually names, so the identity
        // stored is the one the next sync will compute rather than one parse
        // behind it.
        if let Some(b) = &bytes {
            loaded.hash = content_hash(b, &self.files, working);
        }
        self.from = Some((path.clone(), loaded.hash, working));
        self.loaded = Some(Arc::new(loaded));
    }

    /// The config in force, in whichever state it is. `None` means the project
    /// names none at all, which is the built-in family and today's behaviour.
    #[must_use]
    pub fn loaded(&self) -> Option<&Arc<Loaded>> {
        self.loaded.as_ref()
    }

    /// Whether a named colour space can actually be delivered right now — the
    /// question an export's refusal asks, and the one the export
    /// dialogue asks of every name it offers before it draws the row live.
    ///
    /// It is the half of the degrade rule that says no: a preview degrades to
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
fn load(path: &str, hash: u64, present: bool, working: WorkingSpace) -> Loaded {
    let empty = |problem: String, refusal: Refusal| Loaded {
        path: path.to_string(),
        hash,
        config: None,
        rec709_to_working: None,
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
    let loaded = match lumit_colour::Config::load(Path::new(path)) {
        Ok(c) => LoadedConfig::with_working(c, working == WorkingSpace::ConfigSceneLinear),
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
    // A `scene_linear` the config cannot place against Rec.709 is taken as
    // Rec.709, which is what compose-through always meant; the matrix is
    // `None` and the built-in paths stay the hardware ones.
    let rec709_to_working = loaded.rec709_to_working().ok().flatten();
    Loaded {
        path: path.to_string(),
        hash,
        config: Some(loaded),
        rec709_to_working,
        problem: None,
        refusal: None,
        baked: std::sync::Mutex::new(BTreeMap::new()),
    }
}

/// The config's content hash — the config file's own bytes, plus the identity
/// of every look-up-table file it names (§5.5) — folded to the 64 bits a frame
/// key wants.
///
/// A config is not one file: it points at `.spi3d`, `.cube` and `.clf` files
/// beside it, and editing one of those changes the picture just as surely as
/// editing the config does. So each one is folded in here, and a frame made
/// before the edit is named differently from a frame made after it.
///
/// ponytail: the LUT files are folded in by path, length and last-modified
/// stamp rather than by their bytes — the same identity
/// [`crate::fxops::LutCache`] keys an effect's `.cube` on, and for the
/// same reason: this runs at the top of every render, and re-reading tens of
/// megabytes of cube per frame to be told nothing changed is a cost that only
/// shows up on someone else's machine. The ceiling is an edit that changes
/// neither length nor stamp — a hex editor swapping bytes in place, or a build
/// step restoring the mtime it found — which keeps the frames it made until the
/// project is reloaded. If that ever bites, hash the bytes and cache each
/// file's hash against its stamp.
fn content_hash(bytes: &[u8], files: &[std::path::PathBuf], working: WorkingSpace) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(bytes);
    // The working space is part of what the config's tables mean, so the
    // choice renames every frame as surely as an edit to the file does.
    hasher.update(&[u8::from(working == WorkingSpace::ConfigSceneLinear)]);
    for file in files {
        hasher.update(file.as_os_str().as_encoded_bytes());
        if let Ok(meta) = std::fs::metadata(file) {
            hasher.update(&meta.len().to_le_bytes());
            if let Ok(Ok(stamp)) = meta
                .modified()
                .map(|m| m.duration_since(std::time::UNIX_EPOCH))
            {
                hasher.update(&stamp.as_nanos().to_le_bytes());
            }
        }
    }
    let d = hasher.finalize();
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
                // the CPU sampler rounds to the same twelve numbers.
                matrix: lumit_colour::matrix::single(&matrix),
                post: post.map(|c| c.table.data.clone()),
                shaper: shaper(&curve_shaper),
            }
        }
    }
}

/// The footage colour space a layer's pixels arrived in, if the layer is
/// footage and somebody has said.
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
    /// What untagged footage, and footage tagged with a name the config does
    /// not have, linearises through under a config-defined working space: the
    /// built-in sRGB decode with the primaries change behind it. `None` is the
    /// hardware decode, which is right whenever the working space is Rec.709.
    default: Option<lumit_gpu::OcioArtefact>,
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
            return Self {
                by_space,
                default: None,
            };
        };
        let default = loaded
            .artefact(&Edge::Untagged)
            .map(|a| engine.upload_ocio(ctx, &tables(&a)));
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
        Self { by_space, default }
    }

    /// The input transform for a footage item's space, or for none. A name
    /// the config does not have takes the untagged path, as it always did.
    #[must_use]
    pub fn get(&self, space: Option<&str>) -> Option<&lumit_gpu::OcioArtefact> {
        space
            .and_then(|s| self.by_space.get(s))
            .or(self.default.as_ref())
    }

    /// Whether this frame needs any input transform at all — the cheap check
    /// that keeps an ordinary project's render exactly as it was.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_space.is_empty() && self.default.is_none()
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
