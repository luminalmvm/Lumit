//! Resolution: turning what a config *says* into chains Lumit can execute.
//!
//! In plain terms: a config describes every colour space relative to its own
//! **reference space** — "to get from here to the reference, do this". So
//! getting from one space to another is "this space to the reference, then the
//! reference to that space", and the answer is one flat list of steps. Roles are
//! one more indirection: `scene_linear` is a nickname for whichever space the
//! config points it at.
//!
//! The part that needs stating out loud is the **bridge**. A config's reference
//! space is its own; Lumit's working space is scene-linear Rec.709,
//! premultiplied fp16, unless the project chooses the config's `scene_linear`
//! ([`LoadedConfig::with_working`]), and the two are not the same thing. There
//! are three ways this crate joins them, and which one is in force is something
//! the project settings face states rather than hides:
//!
//! - **Exact.** An OCIO v2 config that declares `aces_interchange` (or
//!   `cie_xyz_d65_interchange`) is saying "this named space *is* ACES2065-1"
//!   (or CIE XYZ under D65), and from there a fixed, Bradford-adapted matrix
//!   reaches linear Rec.709. Nothing is assumed.
//! - **Compose through.** A legacy config declares no such role, so this crate
//!   takes the config's `scene_linear` role as the working space's equal. Any
//!   input → working → display trip is still exact end to end, because the two
//!   halves of the assumption cancel; what is affected is Lumit's *own*
//!   perceptual maths (Oklab, the perceptual blends, luma), which reads the
//!   pixels as Rec.709 when they may be, say, ACEScg. This is precisely what
//!   every OCIO v1-era host did.
//! - **The reference itself.** A config with neither role has nothing to
//!   compose through, so its reference space is taken as the working space.
//!   Same honesty, less information.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::bake::Shaper;
use crate::config::{Allocation, Config, TransformSpec, View};
use crate::error::{ColourError, Result};
use crate::matrix;
use crate::op::{Chain, Direction, Op};
use crate::{builtin, file};

/// What a shared view writes where a display colour space would go: "the
/// display showing me, by its own name". OCIO's own spelling, verbatim.
const USE_DISPLAY_NAME: &str = "<USE_DISPLAY_NAME>";

/// Wrap a curve in its stated behaviour below zero, or leave it as it is when
/// the config asked for the curve's own (`crate::op::Negatives`).
fn with_negatives(style: Option<crate::op::Negatives>, curve: Op) -> Op {
    match style {
        Some(style) => Op::Negatives {
            style,
            curve: Box::new(curve),
        },
        None => curve,
    }
}

/// How this config's reference space is joined to Lumit's working space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bridge {
    /// `aces_interchange`: the named space is ACES2065-1. Exact.
    Aces { space: String },
    /// `cie_xyz_d65_interchange`: the named space is CIE XYZ under D65. Exact.
    CieXyzD65 { space: String },
    /// Legacy: the named `scene_linear` space is taken as the working space.
    ComposeThrough { space: String },
    /// Nothing to go on: the config's reference space is taken as the working space.
    ReferenceIsWorking,
}

/// How deep a chain of `ColorSpaceTransform`s may nest before this crate calls
/// it a loop. Real configs nest two or three deep; nothing legitimate reaches
/// this, and a config that points a space at itself must not hang the editor.
const MAX_DEPTH: usize = 32;

/// A parsed config plus the bridge to Lumit's working space. Immutable once
/// built, so it is `Send + Sync` and one parse can serve every document that
/// names the same file (docs/impl/ocio.md §3.2).
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Config,
    bridge: Bridge,
    /// The project composites in the config's `scene_linear` role.
    from_config: bool,
    /// Look-up table files already parsed, by path. A config names each file
    /// from many spaces and views, and some of those files are megabytes.
    /// Held only while a map entry is read or written, never across a
    /// parse. ponytail: grows to the config's own file list and no further,
    /// which is the bound; drop entries by size if a config ever outgrows it.
    lut_cache: Arc<Mutex<BTreeMap<PathBuf, Chain>>>,
}

impl LoadedConfig {
    /// Read and resolve a `config.ocio`.
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Self::new(Config::load(path)?))
    }

    /// Wrap an already-parsed config, compositing in Lumit's own linear
    /// Rec.709 with the config at the edges.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self::with_working(config, false)
    }

    /// Wrap an already-parsed config, and say whose working space the project
    /// composites in. `from_config` takes the config's `scene_linear` role as
    /// the working space outright, as Nuke and Blender do: the bridge is then
    /// the compose-through one whatever roles the config declares, and
    /// [`Self::rec709_to_working`] says how Lumit's own Rec.709 numbers reach
    /// it. A config with no `scene_linear` has nothing to offer and keeps
    /// Lumit's own working space.
    #[must_use]
    pub fn with_working(config: Config, from_config: bool) -> Self {
        if from_config {
            if let Some(space) = config.role("scene_linear") {
                let space = space.to_string();
                return Self {
                    config,
                    bridge: Bridge::ComposeThrough { space },
                    from_config: true,
                    lut_cache: Arc::default(),
                };
            }
        }
        let bridge = if let Some(space) = config.role("aces_interchange") {
            Bridge::Aces {
                space: space.to_string(),
            }
        } else if let Some(space) = config.role("cie_xyz_d65_interchange") {
            Bridge::CieXyzD65 {
                space: space.to_string(),
            }
        } else if let Some(space) = config.role("scene_linear") {
            Bridge::ComposeThrough {
                space: space.to_string(),
            }
        } else {
            Bridge::ReferenceIsWorking
        };
        Self {
            config,
            bridge,
            from_config: false,
            lut_cache: Arc::default(),
        }
    }

    #[must_use]
    pub fn bridge(&self) -> &Bridge {
        &self.bridge
    }

    /// The working space's name when it is the config's `scene_linear` role,
    /// `None` when it is Lumit's own linear Rec.709.
    #[must_use]
    pub fn working_space(&self) -> Option<&str> {
        match (&self.from_config, &self.bridge) {
            (true, Bridge::ComposeThrough { space }) => Some(space),
            _ => None,
        }
    }

    /// Linear Rec.709 → the config-defined working space, as one matrix, for
    /// the pixels Lumit makes in its own primaries: untagged footage after the
    /// built-in decode, and the built-in view's encode run backwards.
    ///
    /// Only a config with an interchange role can say what its `scene_linear`
    /// primaries are; without one, or with Lumit's own working space in
    /// force, there is no matrix and Rec.709 is taken as the working space,
    /// which is what compose-through always meant. A chain that is not pure
    /// matrices (a `scene_linear` with a curve in it) answers `None` too, by
    /// name of what it is rather than by an approximation.
    pub fn rec709_to_working(&self) -> Result<Option<matrix::Matrix34>> {
        let Some(working) = self.working_space() else {
            return Ok(None);
        };
        let via = if let Some(space) = self.config.role("aces_interchange") {
            Chain::new(vec![Op::Matrix(matrix::invert(&matrix::ap0_to_rec709()?)?)])
                .then(self.to_reference(space)?)
        } else if let Some(space) = self.config.role("cie_xyz_d65_interchange") {
            Chain::new(vec![Op::Matrix(matrix::invert(
                &matrix::xyz_d65_to_rec709()?,
            )?)])
            .then(self.to_reference(space)?)
        } else {
            return Ok(None);
        };
        let chain = via.then(self.from_reference(working)?);
        let mut folded = matrix::IDENTITY;
        for op in &chain.ops {
            match op {
                Op::Matrix(m) => folded = matrix::concat(&folded, m),
                _ => return Ok(None),
            }
        }
        Ok(Some(folded))
    }

    /// Every look-up-table file this config's transforms name, resolved through
    /// `search_path` and deduplicated, in path order.
    ///
    /// In plain terms: a config is not one file. It points at `.spi3d`, `.cube`
    /// and `.clf` files beside it, and those files are as much a part of what
    /// the colour comes out as is what the config itself says — so anything
    /// asking "has this config changed" has to ask about them too (§5.5).
    ///
    /// It is the files a resolve *can* read rather than the ones some
    /// particular resolve did: this walks the declarations, which needs no
    /// bookkeeping inside the resolve and cannot go stale when a different edge
    /// is asked for next. A path that no longer resolves is simply not listed;
    /// the resolve that wants it refuses by name, which is its job, not this
    /// one's.
    #[must_use]
    pub fn files_read(&self) -> Vec<std::path::PathBuf> {
        let mut srcs = Vec::new();
        let mut walk = |spec: &Option<TransformSpec>| {
            if let Some(s) = spec {
                collect_files(s, &mut srcs);
            }
        };
        for space in self.config.spaces.values() {
            walk(&space.to_reference);
            walk(&space.from_reference);
        }
        for look in self.config.looks.values() {
            walk(&look.transform);
            walk(&look.inverse_transform);
        }
        for vt in self.config.view_transforms.values() {
            walk(&vt.from_scene_reference);
            walk(&vt.to_scene_reference);
            walk(&vt.from_display_reference);
            walk(&vt.to_display_reference);
        }
        let mut out: Vec<_> = srcs
            .iter()
            .filter_map(|src| self.config.resolve_lut_path(src).ok())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    // -- the reference space, and the bridge across it ----------------------

    /// The config's reference space → Lumit's working space.
    pub fn reference_to_working(&self) -> Result<Chain> {
        Ok(match &self.bridge {
            Bridge::Aces { space } => self
                .from_reference(space)?
                .then(Chain::new(vec![Op::Matrix(matrix::ap0_to_rec709()?)])),
            Bridge::CieXyzD65 { space } => self
                .from_reference(space)?
                .then(Chain::new(vec![Op::Matrix(matrix::xyz_d65_to_rec709()?)])),
            Bridge::ComposeThrough { space } => self.from_reference(space)?,
            Bridge::ReferenceIsWorking => Chain::identity(),
        })
    }

    /// Lumit's working space → the config's reference space.
    pub fn working_to_reference(&self) -> Result<Chain> {
        Ok(match &self.bridge {
            Bridge::Aces { space } => {
                Chain::new(vec![Op::Matrix(matrix::invert(&matrix::ap0_to_rec709()?)?)])
                    .then(self.to_reference(space)?)
            }
            Bridge::CieXyzD65 { space } => Chain::new(vec![Op::Matrix(matrix::invert(
                &matrix::xyz_d65_to_rec709()?,
            )?)])
            .then(self.to_reference(space)?),
            Bridge::ComposeThrough { space } => self.to_reference(space)?,
            Bridge::ReferenceIsWorking => Chain::identity(),
        })
    }

    // -- space to space -----------------------------------------------------

    /// A named space → the config's reference space.
    pub fn to_reference(&self, name: &str) -> Result<Chain> {
        self.to_reference_at(name, 0)
    }

    /// The config's reference space → a named space.
    pub fn from_reference(&self, name: &str) -> Result<Chain> {
        self.from_reference_at(name, 0)
    }

    fn space(&self, name: &str) -> Result<&crate::config::ColourSpace> {
        // A role is one indirection, resolved here so every caller may pass
        // either a space name or a role name (§4.2), and an alias or a
        // different case is one more.
        let named = self.config.role(name).unwrap_or(name);
        let resolved = self.config.canonical(named).unwrap_or(named);
        self.config
            .spaces
            .get(resolved)
            .ok_or_else(|| ColourError::UnknownColourSpace {
                name: name.to_string(),
            })
    }

    fn to_reference_at(&self, name: &str, depth: usize) -> Result<Chain> {
        let space = self.space(name)?;
        // A data space carries no colour, so nothing is done to it.
        if space.is_data {
            return Ok(Chain::identity());
        }
        if let Some(spec) = &space.to_reference {
            return self.resolve_spec(&space.name, spec, depth);
        }
        if let Some(spec) = &space.from_reference {
            return self
                .resolve_spec(&space.name, spec, depth)?
                .inverted(&space.name);
        }
        // A space that declares nothing *is* the reference space.
        Ok(Chain::identity())
    }

    // `to_reference` and `from_reference` are the config format's own keys, so
    // the naming follows OCIO rather than Rust's constructor convention.
    #[allow(clippy::wrong_self_convention)]
    fn from_reference_at(&self, name: &str, depth: usize) -> Result<Chain> {
        let space = self.space(name)?;
        if space.is_data {
            return Ok(Chain::identity());
        }
        if let Some(spec) = &space.from_reference {
            return self.resolve_spec(&space.name, spec, depth);
        }
        if let Some(spec) = &space.to_reference {
            return self
                .resolve_spec(&space.name, spec, depth)?
                .inverted(&space.name);
        }
        Ok(Chain::identity())
    }

    /// "This space to that space", the chain a `ColorSpaceTransform` means.
    pub fn space_to_space(&self, from: &str, to: &str) -> Result<Chain> {
        self.space_to_space_at(from, to, 0)
    }

    /// Whether this name is a **data** space (`isdata: true`) — a mask, a
    /// depth pass, an ID map: numbers that are not a colour.
    ///
    /// It short-circuits the *whole* conversion, not just that space's own
    /// half. Leaving the bridge to Lumit's working space in place would put a
    /// primaries matrix through a matte, which is silently wrong in the way
    /// that only shows up as a soft edge somebody blames on the render.
    fn is_data(&self, name: &str) -> bool {
        self.space(name).is_ok_and(|s| s.is_data)
    }

    fn space_to_space_at(&self, from: &str, to: &str, depth: usize) -> Result<Chain> {
        if self.is_data(from) || self.is_data(to) {
            return Ok(Chain::identity());
        }
        Ok(self
            .to_reference_at(from, depth)?
            .then(self.from_reference_at(to, depth)?))
    }

    /// A named space → Lumit's working space: the input transform for a piece
    /// of footage tagged with that name.
    pub fn from_space(&self, name: &str) -> Result<Chain> {
        self.convert(Some(name), None)
    }

    /// Lumit's working space → a named space: the export's output transform.
    pub fn to_space(&self, name: &str) -> Result<Chain> {
        self.convert(None, Some(name))
    }

    /// One space to another, where `None` on either side is Lumit's own
    /// working space. The chain the OCIO colour space transform effect bakes:
    /// its rows default to the working space, so a fresh instance with one
    /// row set is an input transform or an output transform, and one with
    /// both set is a space-to-space conversion between two of the config's
    /// names.
    pub fn convert(&self, from: Option<&str>, to: Option<&str>) -> Result<Chain> {
        if self.names_data(from) || self.names_data(to) {
            return Ok(Chain::identity());
        }
        Ok(self.reference_from(from)?.then(self.reference_to(to)?))
    }

    /// A look applied between two spaces (`None` is the working space, as in
    /// [`Self::convert`]): the chain the OCIO look transform effect bakes. The
    /// look runs in its own process space; `spec` is written as a view's
    /// `looks:` field is, so `-name` inverts it.
    pub fn look_between(&self, from: Option<&str>, spec: &str, to: Option<&str>) -> Result<Chain> {
        if self.names_data(from) || self.names_data(to) {
            return Ok(Chain::identity());
        }
        Ok(self
            .reference_from(from)?
            .then(self.looks(spec)?)
            .then(self.reference_to(to)?))
    }

    fn names_data(&self, name: Option<&str>) -> bool {
        name.is_some_and(|n| self.is_data(n))
    }

    /// A named space, or the working space for `None`, → the reference.
    fn reference_from(&self, name: Option<&str>) -> Result<Chain> {
        match name {
            Some(n) => self.to_reference(n),
            None => self.working_to_reference(),
        }
    }

    /// The reference → a named space, or the working space for `None`.
    fn reference_to(&self, name: Option<&str>) -> Result<Chain> {
        match name {
            Some(n) => self.from_reference(n),
            None => self.reference_to_working(),
        }
    }

    // -- displays and views -------------------------------------------------

    /// Scene reference → display reference for one view transform.
    ///
    /// A view transform that states no scene-referred transform of its own is
    /// not doing nothing: it is doing whatever the config's
    /// `default_view_transform` does, and then its own display-referred step on
    /// top. Treating "states nothing" as the identity leaves out the whole
    /// scene-to-display rendering, which is a picture in the wrong primaries
    /// that still looks like a picture.
    fn scene_to_display(&self, transform: &crate::config::ViewTransform) -> Result<Chain> {
        match (
            &transform.from_scene_reference,
            &transform.to_scene_reference,
        ) {
            (Some(spec), _) => self.resolve_spec(&transform.name, spec, 0),
            (None, Some(spec)) => self
                .resolve_spec(&transform.name, spec, 0)?
                .inverted(&transform.name),
            (None, None) => match self
                .config
                .default_view_transform
                .as_deref()
                .and_then(|name| self.config.view_transforms.get(name))
            {
                // `!=` on the name, so a default that names itself cannot
                // recurse for ever.
                Some(default) if default.name != transform.name => self.scene_to_display(default),
                _ => Ok(Chain::identity()),
            },
        }
    }

    /// Lumit's working space → what a display shows through a view: the chain
    /// the Viewer's display pass and the export's identical blit both sample.
    pub fn display_view(&self, display: &str, view: &str) -> Result<Chain> {
        self.display_view_from(None, display, view)
    }

    /// A named space (`None`: the working space) → what a display shows
    /// through a view. The chain the OCIO display transform effect bakes; the
    /// Viewer's is the `None` case.
    pub fn display_view_from(
        &self,
        input: Option<&str>,
        display: &str,
        view: &str,
    ) -> Result<Chain> {
        if self.config.display(display).is_none() {
            return Err(ColourError::UnknownDisplay {
                name: display.to_string(),
            });
        }
        let view =
            self.config
                .view(display, view)
                .cloned()
                .ok_or_else(|| ColourError::UnknownView {
                    display: display.to_string(),
                    view: view.to_string(),
                })?;

        // A view onto a data space shows the numbers as they are — the "Raw"
        // view every ACES config ships, which exists precisely so an artist can
        // look at a matte without a display transform on it.
        let target = view
            .display_colour_space
            .as_deref()
            .or(view.colour_space.as_deref());
        if target.is_some_and(|name| self.is_data(name)) {
            return Ok(Chain::identity());
        }

        let mut chain = self.reference_from(input)?;
        chain = chain.then(self.looks(&view.looks)?);

        if let Some(name) = &view.view_transform {
            // The v2 shape: scene reference → display reference → the display's
            // own colour space.
            let transform = self.config.view_transforms.get(name).ok_or_else(|| {
                ColourError::UnknownColourSpace {
                    name: name.to_string(),
                }
            })?;
            chain = chain.then(self.scene_to_display(transform)?);
            // Then the view transform's own display-referred leg, if it states
            // one. It runs in the display-referred domain, before the display
            // colour space's encoding, and it is why "Video (colorimetric)"
            // and "Un-tone-mapped" are two views rather than one.
            match (
                &transform.from_display_reference,
                &transform.to_display_reference,
            ) {
                (Some(spec), _) => {
                    chain = chain.then(self.resolve_spec(&transform.name, spec, 0)?);
                }
                (None, Some(spec)) => {
                    chain = chain.then(
                        self.resolve_spec(&transform.name, spec, 0)?
                            .inverted(&transform.name)?,
                    );
                }
                (None, None) => {}
            }
            let display_space =
                view.display_colour_space
                    .as_deref()
                    .ok_or_else(|| ColourError::UnknownView {
                        display: display.to_string(),
                        view: view.name.clone(),
                    })?;
            // A **shared view** is written once and listed under many displays,
            // so it cannot name a display colour space — it writes the literal
            // `<USE_DISPLAY_NAME>` and means "whichever display is showing me,
            // by its own name". Every view of the ACES v2 configs is one of
            // these, so missing it is not an edge case: it is the whole of
            // their display half (docs/impl/ocio.md §4.2).
            let display_space = if display_space == USE_DISPLAY_NAME {
                display
            } else {
                display_space
            };
            return Ok(chain.then(self.from_reference(display_space)?));
        }

        // The v1 shape: the view names its display colour space directly.
        let space = view
            .colour_space
            .as_deref()
            .ok_or_else(|| ColourError::UnknownView {
                display: display.to_string(),
                view: view.name.clone(),
            })?;
        Ok(chain.then(self.from_reference(space)?))
    }

    /// A view's `looks:` field: comma-separated names, `+` forward (the
    /// default), `-` inverted, each applied inside its own process space.
    pub fn looks(&self, looks: &str) -> Result<Chain> {
        let mut chain = Chain::identity();
        for entry in looks.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let (name, dir) = match entry.strip_prefix('-') {
                Some(rest) => (rest.trim(), Direction::Inverse),
                None => (
                    entry.strip_prefix('+').unwrap_or(entry).trim(),
                    Direction::Forward,
                ),
            };
            let look = self
                .config
                .looks
                .get(name)
                .ok_or_else(|| ColourError::UnknownLook {
                    name: name.to_string(),
                })?;
            let step = match (dir, &look.transform, &look.inverse_transform) {
                (Direction::Forward, Some(spec), _) => self.resolve_spec(&look.name, spec, 0)?,
                (Direction::Inverse, _, Some(spec)) => self.resolve_spec(&look.name, spec, 0)?,
                (Direction::Inverse, Some(spec), None) => self
                    .resolve_spec(&look.name, spec, 0)?
                    .inverted(&look.name)?,
                _ => Chain::identity(),
            };
            if step.is_identity() {
                continue;
            }
            // A look is defined in its own process space, so the picture is
            // taken there and brought back.
            if look.process_space.is_empty() {
                chain = chain.then(step);
            } else {
                chain = chain
                    .then(self.from_reference(&look.process_space)?)
                    .then(step)
                    .then(self.to_reference(&look.process_space)?);
            }
        }
        Ok(chain)
    }

    /// The names a picker should list for one display, in the config's order.
    #[must_use]
    pub fn view_names(&self, display: &str) -> Vec<&str> {
        self.config
            .display(display)
            .map(|d| d.views.iter().map(|v| v.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// The shaper a bake should use when this space is the domain: the config
    /// author's own `allocation` where it states one, Lumit's default otherwise
    /// (docs/impl/ocio.md §5.1).
    #[must_use]
    pub fn shaper_for(&self, name: &str) -> Shaper {
        let allocation = self.space(name).ok().and_then(|s| s.allocation);
        match allocation {
            Some(Allocation::Lg2 { min, max, offset }) => Shaper::Lg2 {
                min_log2: min,
                max_log2: max,
                offset,
            },
            Some(Allocation::Uniform { min, max }) => Shaper::Uniform { min, max },
            None => Shaper::DEFAULT,
        }
    }

    // -- the recursion ------------------------------------------------------

    fn resolve_spec(&self, what: &str, spec: &TransformSpec, depth: usize) -> Result<Chain> {
        if depth > MAX_DEPTH {
            return Err(ColourError::Parse {
                what: what.to_string(),
                reason: "the transforms refer to each other in a loop".to_string(),
            });
        }
        Ok(match spec {
            TransformSpec::Group(children, dir) => {
                let mut chain = Chain::identity();
                for child in children {
                    chain = chain.then(self.resolve_spec(what, child, depth + 1)?);
                }
                match dir {
                    Direction::Forward => chain,
                    Direction::Inverse => chain.inverted(what)?,
                }
            }
            TransformSpec::Matrix {
                matrix: m,
                offset,
                dir,
            } => {
                // The config writes 4×4 row-major with a 4-vector offset; only
                // the upper 3×4 has any meaning for RGB.
                let mut m34: matrix::Matrix34 = [0.0; 12];
                for row in 0..3 {
                    for col in 0..3 {
                        m34[row * 4 + col] =
                            f64::from(m.get(row * 4 + col).copied().unwrap_or(0.0));
                    }
                    m34[row * 4 + 3] = f64::from(offset.get(row).copied().unwrap_or(0.0));
                }
                let m34 = match dir {
                    Direction::Forward => m34,
                    Direction::Inverse => matrix::invert(&m34)?,
                };
                Chain::new(vec![Op::Matrix(m34)])
            }
            TransformSpec::Exponent {
                value,
                negatives,
                dir,
            } => Chain::new(vec![with_negatives(
                *negatives,
                Op::Exponent {
                    exp: *value,
                    dir: *dir,
                },
            )]),
            TransformSpec::ExponentWithLinear {
                gamma,
                offset,
                negatives,
                dir,
            } => Chain::new(vec![with_negatives(
                *negatives,
                Op::MonCurve {
                    gamma: *gamma,
                    offset: *offset,
                    dir: *dir,
                },
            )]),
            TransformSpec::Log { params, dir } => Chain::new(vec![Op::Log {
                params: *params,
                dir: *dir,
            }]),
            TransformSpec::Cdl { params, dir } => Chain::new(vec![Op::Cdl {
                params: *params,
                dir: *dir,
            }]),
            TransformSpec::Range(params, dir) => {
                let op = Op::Range(*params);
                match dir {
                    Direction::Forward => Chain::new(vec![op]),
                    Direction::Inverse => Chain::new(vec![op.inverted(what)?]),
                }
            }
            TransformSpec::File { src, dir } => {
                let path = self.config.resolve_lut_path(src)?;
                let chain = self.file_chain(&path)?;
                match dir {
                    Direction::Forward => chain,
                    Direction::Inverse => chain.inverted(what)?,
                }
            }
            TransformSpec::ColourSpace { src, dst, dir } => {
                let (src, dst) = match dir {
                    Direction::Forward => (src, dst),
                    Direction::Inverse => (dst, src),
                };
                self.space_to_space_at(src, dst, depth + 1)?
            }
            TransformSpec::Builtin { style, dir } => builtin::resolve(style, *dir)?,
            TransformSpec::Unsupported { name } => {
                return Err(ColourError::UnsupportedTransform { name: name.clone() })
            }
        })
    }

    /// A look-up table file, parsed once per loaded config. Every space, view
    /// and look that names it gets a copy of the parsed steps, rather than
    /// another read of a file that can run to megabytes.
    fn file_chain(&self, path: &Path) -> Result<Chain> {
        if let Ok(cache) = self.lut_cache.lock() {
            if let Some(chain) = cache.get(path) {
                return Ok(chain.clone());
            }
        }
        let chain = file::load(path)?;
        if let Ok(mut cache) = self.lut_cache.lock() {
            cache.insert(path.to_path_buf(), chain.clone());
        }
        Ok(chain)
    }
}

/// The `src` of every `FileTransform` in one spec, groups walked through.
fn collect_files(spec: &TransformSpec, out: &mut Vec<String>) {
    match spec {
        TransformSpec::Group(children, _) => {
            for child in children {
                collect_files(child, out);
            }
        }
        TransformSpec::File { src, .. } => out.push(src.clone()),
        _ => {}
    }
}

/// Every view a config offers, flattened: `(display, view)` pairs in the
/// config's own order. The picker's menu is this list, sectioned by display.
#[must_use]
pub fn all_views(config: &Config) -> Vec<(&str, &View)> {
    let mut out = Vec::new();
    for display in &config.displays {
        for view in &display.views {
            out.push((display.name.as_str(), view));
        }
    }
    out
}

/// Names this crate cannot resolve, gathered rather than found one at a time:
/// every colour space a config declares, walked to the reference. The refusal
/// taxonomy test reads it; the renderer walks its own list, per name and in
/// both directions, so a picker can grey out exactly the rows that refuse.
pub fn unresolvable(loaded: &LoadedConfig) -> Vec<(String, ColourError)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for name in &loaded.config.space_order {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Err(e) = loaded.to_reference(name) {
            out.push((name.clone(), e));
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    fn load(text: &str) -> LoadedConfig {
        LoadedConfig::new(Config::parse(Path::new("."), text).expect("parses"))
    }

    /// A legacy-shaped config: no interchange roles, a `scene_linear` role, and
    /// a display whose view names a colour space directly.
    const LEGACY: &str = r#"
ocio_profile_version: 1
roles:
  scene_linear: lin_ap1
  reference: ref
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: out_srgb}
colorspaces:
  - !<ColorSpace>
    name: ref
  - !<ColorSpace>
    name: lin_ap1
    to_reference: !<MatrixTransform> {matrix: [1.4, -0.2, -0.2, 0, -0.1, 1.2, -0.1, 0, 0, -0.3, 1.3, 0, 0, 0, 0, 1]}
  - !<ColorSpace>
    name: srgb_texture
    to_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0]}
  - !<ColorSpace>
    name: out_srgb
    from_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}
"#;

    #[test]
    fn a_legacy_config_composes_through_its_scene_linear_role() {
        let loaded = load(LEGACY);
        assert_eq!(
            loaded.bridge(),
            &Bridge::ComposeThrough {
                space: "lin_ap1".to_string()
            }
        );
    }

    #[test]
    fn composing_through_is_still_exact_end_to_end() {
        // The point of §2.1: the assumption cancels on an input → working →
        // display trip, so the picture is exactly what the config describes.
        let loaded = load(LEGACY);
        let through = loaded
            .from_space("srgb_texture")
            .expect("resolves")
            .then(loaded.display_view("sRGB", "Standard").expect("resolves"));
        let direct = loaded
            .space_to_space("srgb_texture", "out_srgb")
            .expect("resolves");
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            let c = [x, 1.0 - x, 0.5];
            assert!(
                close(through.eval(c), direct.eval(c), 1e-4),
                "at {c:?}: {:?} vs {:?}",
                through.eval(c),
                direct.eval(c)
            );
        }
    }

    #[test]
    fn a_missing_direction_is_the_declared_one_inverted() {
        let loaded = load(LEGACY);
        // `out_srgb` declares only from_reference; asking the other way round
        // must invert it rather than refuse.
        let there = loaded.from_reference("out_srgb").expect("resolves");
        let back = loaded.to_reference("out_srgb").expect("resolves");
        let c = [0.2, 0.5, 0.8];
        assert!(close(back.eval(there.eval(c)), c, 1e-4));
    }

    #[test]
    fn an_allocation_transform_squeezes_as_the_reference_library_does() {
        let loaded = load(
            r#"
ocio_profile_version: 1
colorspaces:
  - !<ColorSpace>
    name: logged
    from_reference: !<AllocationTransform> {allocation: lg2, vars: [-8, 5]}
  - !<ColorSpace>
    name: fitted
    from_reference: !<AllocationTransform> {allocation: uniform, vars: [0, 2]}
"#,
        );
        // lg2 [-8, 5]: 1 lands 8/13 of the way up, 2^5 on the top, 2^-4 at 4/13.
        let logged = loaded.from_reference("logged").expect("resolves");
        assert!(close(
            logged.eval([1.0, 32.0, 0.0625]),
            [8.0 / 13.0, 1.0, 4.0 / 13.0],
            1e-5
        ));
        let fitted = loaded.from_reference("fitted").expect("resolves");
        assert!(close(fitted.eval([1.0, 2.0, 0.5]), [0.5, 1.0, 0.25], 1e-6));
        // The way back is the declared direction inverted, which is how a
        // Filmic or AgX log space is walked to the reference.
        for name in ["logged", "fitted"] {
            let there = loaded.from_reference(name).expect("resolves");
            let back = loaded.to_reference(name).expect("inverts");
            let c = [0.5, 1.0, 4.0];
            assert!(close(back.eval(there.eval(c)), c, 1e-5), "{name}");
        }
    }

    #[test]
    fn an_alias_is_a_name_too() {
        let loaded = load(
            r#"
ocio_profile_version: 2
colorspaces:
  - !<ColorSpace>
    name: Linear CIE-XYZ E
    aliases: [xyz_e, Linear CIE-XYZ I-E]
    from_scene_reference: !<MatrixTransform> {matrix: [2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1]}
  - !<ColorSpace>
    name: plain
  - !<ColorSpace>
    name: through
    from_scene_reference: !<ColorSpaceTransform> {src: Linear CIE-XYZ I-E, dst: plain}
"#,
        );
        // Asked for by alias, and reached by alias from another space.
        let by_alias = loaded.from_reference("xyz_e").expect("resolves");
        assert!(close(by_alias.eval([1.0, 1.0, 1.0]), [2.0, 2.0, 2.0], 1e-6));
        // And in the wrong case, which the reference library also allows.
        let by_case = loaded
            .from_reference("linear CIE-xyz I-E")
            .expect("resolves");
        assert!(close(by_case.eval([1.0, 1.0, 1.0]), [2.0, 2.0, 2.0], 1e-6));
        let through = loaded.from_reference("through").expect("resolves");
        assert!(close(through.eval([2.0, 2.0, 2.0]), [1.0, 1.0, 1.0], 1e-6));
    }

    #[test]
    fn an_unsupported_transform_refuses_only_where_it_is_used() {
        let loaded = load(
            r#"
ocio_profile_version: 2
colorspaces:
  - !<ColorSpace>
    name: fancy
    to_scene_reference: !<FixedFunctionTransform> {style: ACES_RedMod03}
  - !<ColorSpace>
    name: plain
    to_scene_reference: !<ExponentTransform> {value: 2.2}
looks:
  - !<Look>
    name: graded
    process_space: plain
    transform: !<GradingToneTransform> {style: log}
"#,
        );
        let err = loaded.to_reference("fancy");
        assert!(
            matches!(&err, Err(ColourError::UnsupportedTransform { name }) if name == "FixedFunctionTransform"),
            "{err:?}"
        );
        assert!(loaded.to_reference("plain").is_ok());
        let err = loaded.looks("graded");
        assert!(
            matches!(&err, Err(ColourError::UnsupportedTransform { name }) if name == "GradingToneTransform"),
            "{err:?}"
        );
        let bad: Vec<String> = unresolvable(&loaded).into_iter().map(|(n, _)| n).collect();
        assert_eq!(bad, ["fancy"]);
    }

    #[test]
    fn a_role_may_be_used_wherever_a_space_name_is() {
        let loaded = load(LEGACY);
        let by_role = loaded.to_reference("scene_linear").expect("resolves");
        let by_name = loaded.to_reference("lin_ap1").expect("resolves");
        assert_eq!(by_role, by_name);
    }

    const INTERCHANGE: &str = r#"
ocio_profile_version: 2
roles:
  scene_linear: ACEScg
  aces_interchange: ACES2065-1
colorspaces:
  - !<ColorSpace>
    name: ACES2065-1
  - !<ColorSpace>
    name: ACEScg
    to_scene_reference: !<BuiltinTransform> {style: ACEScg_to_ACES2065-1}
"#;

    #[test]
    fn an_interchange_role_makes_the_bridge_exact() {
        let loaded = load(INTERCHANGE);
        assert_eq!(
            loaded.bridge(),
            &Bridge::Aces {
                space: "ACES2065-1".to_string()
            }
        );
        // ACES white through the exact bridge is Rec.709 white.
        let chain = loaded.from_space("ACES2065-1").expect("resolves");
        assert!(close(chain.eval([1.0; 3]), [1.0; 3], 1e-3));
    }

    #[test]
    fn the_bridge_round_trips() {
        let loaded = load(INTERCHANGE);
        let there = loaded.from_space("ACEScg").expect("resolves");
        let back = loaded.to_space("ACEScg").expect("resolves");
        let c = [0.2, 0.5, 0.8];
        assert!(close(back.eval(there.eval(c)), c, 1e-4));
    }

    #[test]
    fn a_config_with_no_roles_at_all_takes_its_reference_as_working() {
        let loaded =
            load("ocio_profile_version: 1\ncolorspaces:\n  - !<ColorSpace>\n    name: ref\n");
        assert_eq!(loaded.bridge(), &Bridge::ReferenceIsWorking);
        assert!(loaded
            .reference_to_working()
            .expect("resolves")
            .is_identity());
    }

    #[test]
    fn a_view_transform_view_composes_through_the_display_reference() {
        let text = r#"
ocio_profile_version: 2
roles:
  scene_linear: lin
displays:
  Rec1886:
    - !<View> {name: SDR, view_transform: tonemap, display_colorspace: display_gamma}
colorspaces:
  - !<ColorSpace>
    name: lin
view_transforms:
  - !<ViewTransform>
    name: tonemap
    from_scene_reference: !<RangeTransform> {minInValue: 0, maxInValue: 4, minOutValue: 0, maxOutValue: 1}
display_colorspaces:
  - !<ColorSpace>
    name: display_gamma
    from_display_reference: !<ExponentTransform> {value: [0.5, 0.5, 0.5, 1]}
"#;
        let loaded = load(text);
        let chain = loaded.display_view("Rec1886", "SDR").expect("resolves");
        // 4.0 scene-linear maps to 1.0 through the view transform, then the
        // display curve leaves 1.0 alone.
        assert!(close(chain.eval([4.0; 3]), [1.0; 3], 1e-4));
        // 1.0 maps to 0.25, then the square root of 0.25 is 0.5.
        assert!(close(chain.eval([1.0; 3]), [0.5; 3], 1e-4));
    }

    /// The shape every ACES v2 config's display half is written in, and three
    /// separate faults the reference fixture caught in it.
    const SHARED_VIEWS: &str = r#"
ocio_profile_version: 2
roles:
  scene_linear: lin
default_view_transform: tonemap
shared_views:
  - !<View> {name: SDR, view_transform: tonemap, display_colorspace: <USE_DISPLAY_NAME>}
  - !<View> {name: Flat, view_transform: passthrough, display_colorspace: <USE_DISPLAY_NAME>}
displays:
  Rec1886:
    - !<View> {name: Raw, colorspace: data}
    - !<Views> [SDR, Flat]
colorspaces:
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: data
    isdata: true
view_transforms:
  - !<ViewTransform>
    name: tonemap
    from_scene_reference: !<RangeTransform> {minInValue: 0, maxInValue: 4, minOutValue: 0, maxOutValue: 1}
  - !<ViewTransform>
    name: passthrough
    from_display_reference: !<MatrixTransform> {}
display_colorspaces:
  - !<ColorSpace>
    name: Rec1886
    from_display_reference: !<ExponentTransform> {value: [0.5, 0.5, 0.5, 1]}
"#;

    #[test]
    fn a_shared_view_takes_the_display_s_own_name_for_its_colour_space() {
        let loaded = load(SHARED_VIEWS);
        let chain = loaded.display_view("Rec1886", "SDR").expect("resolves");
        // 4.0 scene-linear maps to 1.0 through the view transform, and the
        // display curve leaves 1.0 alone — the same answer the view would give
        // if it named `Rec1886` outright, which is the whole point.
        assert!(close(chain.eval([4.0; 3]), [1.0; 3], 1e-4));
        assert!(close(chain.eval([1.0; 3]), [0.5; 3], 1e-4));
    }

    #[test]
    fn a_view_onto_a_data_space_shows_the_numbers_untouched() {
        let loaded = load(SHARED_VIEWS);
        // Not merely "the data space does nothing to it": the bridge into the
        // config's reference must not run either, or a matte comes back
        // through a primaries matrix.
        assert!(loaded
            .display_view("Rec1886", "Raw")
            .expect("resolves")
            .is_identity());
        assert!(loaded.from_space("data").expect("resolves").is_identity());
        assert!(loaded.to_space("data").expect("resolves").is_identity());
        assert!(loaded
            .space_to_space("data", "lin")
            .expect("resolves")
            .is_identity());
    }

    #[test]
    fn a_display_referred_view_transform_borrows_the_default_for_the_rendering() {
        let loaded = load(SHARED_VIEWS);
        // `passthrough` states only a display-referred transform, so the
        // scene-to-display leg is `default_view_transform`'s. Reading its
        // `from_display_reference` as a scene-referred one instead — which is
        // what this parser first did — would invert an identity matrix and
        // quietly drop the rendering, giving 0.5 here instead of 1.0.
        let chain = loaded.display_view("Rec1886", "Flat").expect("resolves");
        assert!(close(chain.eval([4.0; 3]), [1.0; 3], 1e-4));
    }

    #[test]
    fn an_exponent_s_negative_style_is_read_from_the_key_a_config_file_writes() {
        // `style`, not `negativeStyle`: the config file's spelling. Read for
        // the wrong key it finds nothing and clamps, and nothing above zero
        // ever shows it.
        let text = r#"
ocio_profile_version: 2
roles:
  scene_linear: lin
colorspaces:
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: g22
    from_scene_reference: !<ExponentTransform> {value: [2.2, 2.2, 2.2, 1], style: pass_thru, direction: inverse}
  - !<ColorSpace>
    name: mirrored
    from_scene_reference: !<ExponentTransform> {value: [2.2, 2.2, 2.2, 1], style: mirror, direction: inverse}
"#;
        let loaded = load(text);
        let pass_thru = loaded.from_reference("g22").expect("resolves");
        let mirror = loaded.from_reference("mirrored").expect("resolves");
        // Above zero the two agree, which is exactly why the fault hid.
        assert!(close(
            pass_thru.eval([0.25; 3]),
            mirror.eval([0.25; 3]),
            1e-6
        ));
        // Below it they part: pass_thru carries the value, mirror curves it.
        assert!(close(pass_thru.eval([-0.25; 3]), [-0.25; 3], 1e-6));
        // −(0.25 ^ (1 / 2.2)) = −exp(ln 0.25 / 2.2) = −0.532 520 5.
        let m = mirror.eval([-0.25; 3]);
        assert!(close(m, [-0.532_520_5; 3], 1e-6), "{m:?}");
    }

    #[test]
    fn a_look_is_applied_inside_its_process_space() {
        let text = r#"
ocio_profile_version: 1
roles:
  scene_linear: lin
displays:
  sRGB:
    - !<View> {name: Graded, colorspace: lin, looks: warm}
looks:
  - !<Look>
    name: warm
    process_space: lin
    transform: !<CDLTransform> {slope: [1.1, 1.0, 0.9], style: no_clamp}
colorspaces:
  - !<ColorSpace>
    name: lin
"#;
        let loaded = load(text);
        let chain = loaded.display_view("sRGB", "Graded").expect("resolves");
        assert!(close(chain.eval([0.5; 3]), [0.55, 0.5, 0.45], 1e-4));
    }

    #[test]
    fn an_inverted_look_undoes_itself() {
        let text = r#"
ocio_profile_version: 1
roles:
  scene_linear: lin
displays:
  sRGB:
    - !<View> {name: Plain, colorspace: lin, looks: "+warm, -warm"}
looks:
  - !<Look>
    name: warm
    process_space: lin
    transform: !<CDLTransform> {slope: [1.1, 1.0, 0.9], style: no_clamp}
colorspaces:
  - !<ColorSpace>
    name: lin
"#;
        let loaded = load(text);
        let chain = loaded.display_view("sRGB", "Plain").expect("resolves");
        assert!(close(chain.eval([0.5; 3]), [0.5; 3], 1e-4));
    }

    #[test]
    fn a_colour_space_transform_is_followed() {
        let text = r#"
ocio_profile_version: 1
colorspaces:
  - !<ColorSpace>
    name: ref
  - !<ColorSpace>
    name: half
    to_reference: !<MatrixTransform> {matrix: [0.5, 0, 0, 0, 0, 0.5, 0, 0, 0, 0, 0.5, 0, 0, 0, 0, 1]}
  - !<ColorSpace>
    name: via
    to_reference: !<ColorSpaceTransform> {src: half, dst: ref}
"#;
        let loaded = load(text);
        let chain = loaded.to_reference("via").expect("resolves");
        assert!(close(chain.eval([1.0; 3]), [0.5; 3], 1e-6));
    }

    #[test]
    fn a_space_that_points_at_itself_is_refused_rather_than_hanging() {
        let text = r#"
ocio_profile_version: 1
colorspaces:
  - !<ColorSpace>
    name: loop_a
    to_reference: !<ColorSpaceTransform> {src: loop_b, dst: ref}
  - !<ColorSpace>
    name: loop_b
    to_reference: !<ColorSpaceTransform> {src: loop_a, dst: ref}
  - !<ColorSpace>
    name: ref
"#;
        let loaded = load(text);
        assert!(loaded.to_reference("loop_a").is_err());
    }

    #[test]
    fn an_unknown_name_refuses_by_name() {
        let loaded = load(LEGACY);
        let err = loaded.to_reference("no such space");
        assert!(
            matches!(&err, Err(ColourError::UnknownColourSpace { name }) if name == "no such space"),
            "{err:?}"
        );
        assert!(matches!(
            loaded.display_view("no such display", "Standard"),
            Err(ColourError::UnknownDisplay { .. })
        ));
        assert!(matches!(
            loaded.display_view("sRGB", "no such view"),
            Err(ColourError::UnknownView { .. })
        ));
    }

    #[test]
    fn an_allocation_becomes_the_shaper_the_bake_uses() {
        let text = r#"
ocio_profile_version: 1
colorspaces:
  - !<ColorSpace>
    name: wide
    allocation: lg2
    allocationvars: [-10, 8, 0.001]
  - !<ColorSpace>
    name: plain
"#;
        let loaded = load(text);
        assert_eq!(
            loaded.shaper_for("wide"),
            Shaper::Lg2 {
                min_log2: -10.0,
                max_log2: 8.0,
                offset: 0.001
            }
        );
        assert_eq!(loaded.shaper_for("plain"), Shaper::DEFAULT);
    }

    #[test]
    fn a_data_space_is_left_alone() {
        let text = r#"
ocio_profile_version: 1
colorspaces:
  - !<ColorSpace>
    name: raw
    isdata: true
    to_reference: !<MatrixTransform> {matrix: [9, 0, 0, 0, 0, 9, 0, 0, 0, 0, 9, 0, 0, 0, 0, 1]}
"#;
        let loaded = load(text);
        assert!(loaded.to_reference("raw").expect("resolves").is_identity());
    }

    #[test]
    fn every_space_in_a_healthy_config_resolves() {
        assert!(unresolvable(&load(LEGACY)).is_empty());
    }

    #[test]
    fn an_unsupported_builtin_shows_up_in_the_unresolvable_list_by_name() {
        let text = r#"
ocio_profile_version: 2
colorspaces:
  - !<ColorSpace>
    name: fancy
    to_scene_reference: !<BuiltinTransform> {style: ACES-OUTPUT - SOMETHING}
"#;
        let bad = unresolvable(&load(text));
        assert_eq!(bad.len(), 1);
        assert!(
            matches!(bad.first(), Some((name, ColourError::UnsupportedBuiltin { style }))
                if name == "fancy" && style == "ACES-OUTPUT - SOMETHING"),
            "{bad:?}"
        );
    }

    /// The OCIO effects' edges (docs/impl/ocio.md §6.6): `None` is the working
    /// space, so one name is an input or output transform and none at all is
    /// nothing to do.
    #[test]
    fn convert_treats_no_name_as_the_working_space() {
        let text = r#"
ocio_profile_version: 1
roles:
  scene_linear: lin
colorspaces:
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: half
    to_reference: !<MatrixTransform> {matrix: [0.5, 0, 0, 0, 0, 0.5, 0, 0, 0, 0, 0.5, 0, 0, 0, 0, 1]}
  - !<ColorSpace>
    name: matte
    isdata: true
"#;
        let loaded = load(text);
        assert!(loaded.convert(None, None).expect("resolves").is_identity());
        // One name is the edge the footage and export paths already take.
        let a = loaded.convert(Some("half"), None).expect("resolves");
        let b = loaded.from_space("half").expect("resolves");
        assert!(close(a.eval([1.0; 3]), b.eval([1.0; 3]), 1e-6));
        assert!(close(a.eval([1.0; 3]), [0.5; 3], 1e-6));
        let out = loaded.convert(None, Some("half")).expect("resolves");
        assert!(close(out.eval([0.5; 3]), [1.0; 3], 1e-6));
        // Two names go through the reference, and a data space on either
        // side is left alone.
        let round = loaded
            .convert(Some("half"), Some("half"))
            .expect("resolves");
        assert!(close(round.eval([0.3; 3]), [0.3; 3], 1e-6));
        assert!(loaded
            .convert(Some("matte"), Some("half"))
            .expect("resolves")
            .is_identity());
    }

    #[test]
    fn a_look_between_two_spaces_runs_in_its_process_space_and_inverts_by_spec() {
        let text = r#"
ocio_profile_version: 1
roles:
  scene_linear: lin
looks:
  - !<Look>
    name: warm
    process_space: lin
    transform: !<CDLTransform> {slope: [1.1, 1.0, 0.9], style: no_clamp}
colorspaces:
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: half
    to_reference: !<MatrixTransform> {matrix: [0.5, 0, 0, 0, 0, 0.5, 0, 0, 0, 0, 0.5, 0, 0, 0, 0, 1]}
"#;
        let loaded = load(text);
        let forward = loaded.look_between(None, "warm", None).expect("resolves");
        assert!(close(forward.eval([0.5; 3]), [0.55, 0.5, 0.45], 1e-4));
        let back = loaded.look_between(None, "-warm", None).expect("resolves");
        assert!(close(back.eval(forward.eval([0.5; 3])), [0.5; 3], 1e-4));
        // Between two spaces: half in, the look in lin, back out to half.
        let via = loaded
            .look_between(Some("half"), "warm", Some("half"))
            .expect("resolves");
        assert!(close(via.eval([1.0; 3]), [1.1, 1.0, 0.9], 1e-4));
        assert!(matches!(
            loaded.look_between(None, "cold", None),
            Err(ColourError::UnknownLook { .. })
        ));
        assert_eq!(loaded.config.look_names(), vec!["warm"]);
    }

    #[test]
    fn a_display_view_from_a_named_space_starts_there_rather_than_at_working() {
        let text = r#"
ocio_profile_version: 1
roles:
  scene_linear: lin
displays:
  sRGB:
    - !<View> {name: Plain, colorspace: lin}
colorspaces:
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: half
    to_reference: !<MatrixTransform> {matrix: [0.5, 0, 0, 0, 0, 0.5, 0, 0, 0, 0, 0.5, 0, 0, 0, 0, 1]}
"#;
        let loaded = load(text);
        let from_working = loaded
            .display_view_from(None, "sRGB", "Plain")
            .expect("resolves");
        assert!(from_working.is_identity());
        let from_half = loaded
            .display_view_from(Some("half"), "sRGB", "Plain")
            .expect("resolves");
        assert!(close(from_half.eval([1.0; 3]), [0.5; 3], 1e-6));
    }

    /// An ACES-shaped config: AP0 reference, the interchange role on it, and
    /// ACEScg as `scene_linear`, a matrix away.
    fn aces_shaped() -> String {
        let m = matrix::rgb_to_rgb(&matrix::AP1, &matrix::AP0).expect("matrix");
        let row = |r: usize| format!("{}, {}, {}, 0", m[r * 4], m[r * 4 + 1], m[r * 4 + 2]);
        format!(
            r#"
ocio_profile_version: 2
roles:
  aces_interchange: ap0
  scene_linear: acescg
colorspaces:
  - !<ColorSpace>
    name: ap0
  - !<ColorSpace>
    name: acescg
    to_scene_reference: !<MatrixTransform> {{matrix: [{}, {}, {}, 0, 0, 0, 1]}}
"#,
            row(0),
            row(1),
            row(2)
        )
    }

    /// A project that composites in the config's `scene_linear` takes it as
    /// the working space outright, and says how Rec.709 reaches it.
    #[test]
    fn the_config_working_space_is_scene_linear_and_a_matrix_from_rec709() {
        let text = aces_shaped();
        let ours = load(&text);
        assert_eq!(ours.working_space(), None);
        assert!(ours.rec709_to_working().expect("resolves").is_none());
        assert!(matches!(ours.bridge(), Bridge::Aces { .. }));

        let theirs =
            LoadedConfig::with_working(Config::parse(Path::new("."), &text).expect("parses"), true);
        assert_eq!(theirs.working_space(), Some("acescg"));
        assert!(matches!(theirs.bridge(), Bridge::ComposeThrough { space } if space == "acescg"));
        let m = theirs
            .rec709_to_working()
            .expect("resolves")
            .expect("an interchange role places scene_linear");
        let want = matrix::rgb_to_rgb(&matrix::REC709, &matrix::AP1).expect("matrix");
        for (a, b) in m.iter().zip(want.iter()) {
            assert!((a - b).abs() < 1e-6, "{m:?} vs {want:?}");
        }
        // Footage tagged acescg is then the identity: it already is the
        // working space.
        assert!(theirs.from_space("acescg").expect("resolves").is_identity());
    }

    /// Without an interchange role nothing can place `scene_linear`, so it is
    /// taken as Rec.709 and there is no matrix; and a config with no
    /// `scene_linear` at all keeps Lumit's working space.
    #[test]
    fn a_config_that_cannot_place_its_scene_linear_has_no_matrix() {
        let text = r#"
ocio_profile_version: 1
roles:
  scene_linear: lin
colorspaces:
  - !<ColorSpace>
    name: lin
"#;
        let loaded =
            LoadedConfig::with_working(Config::parse(Path::new("."), text).expect("parses"), true);
        assert_eq!(loaded.working_space(), Some("lin"));
        assert!(loaded.rec709_to_working().expect("resolves").is_none());

        let bare = LoadedConfig::with_working(
            Config::parse(Path::new("."), "ocio_profile_version: 1\n").expect("parses"),
            true,
        );
        assert_eq!(bare.working_space(), None);
    }
}
