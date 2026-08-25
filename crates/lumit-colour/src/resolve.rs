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
//! space is its own; Lumit's working space is fixed — scene-linear Rec.709,
//! premultiplied fp16 (K-490) — and the two are not the same thing. There are
//! three ways this crate joins them, and which one is in force is something the
//! project settings face states rather than hides:
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

use std::collections::BTreeSet;
use std::path::Path;

use crate::bake::Shaper;
use crate::config::{Allocation, Config, TransformSpec, View};
use crate::error::{ColourError, Result};
use crate::matrix;
use crate::op::{Chain, Direction, Op};
use crate::{builtin, file};

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
}

impl LoadedConfig {
    /// Read and resolve a `config.ocio`.
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Self::new(Config::load(path)?))
    }

    /// Wrap an already-parsed config.
    #[must_use]
    pub fn new(config: Config) -> Self {
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
        Self { config, bridge }
    }

    #[must_use]
    pub fn bridge(&self) -> &Bridge {
        &self.bridge
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
        // either a space name or a role name (§4.2).
        let resolved = self.config.role(name).unwrap_or(name);
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

    fn space_to_space_at(&self, from: &str, to: &str, depth: usize) -> Result<Chain> {
        Ok(self
            .to_reference_at(from, depth)?
            .then(self.from_reference_at(to, depth)?))
    }

    /// A named space → Lumit's working space: the input transform for a piece
    /// of footage tagged with that name.
    pub fn from_space(&self, name: &str) -> Result<Chain> {
        Ok(self.to_reference(name)?.then(self.reference_to_working()?))
    }

    /// Lumit's working space → a named space: the export's output transform.
    pub fn to_space(&self, name: &str) -> Result<Chain> {
        Ok(self
            .working_to_reference()?
            .then(self.from_reference(name)?))
    }

    // -- displays and views -------------------------------------------------

    /// Lumit's working space → what a display shows through a view: the chain
    /// the Viewer's display pass and the export's identical blit both sample.
    pub fn display_view(&self, display: &str, view: &str) -> Result<Chain> {
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

        let mut chain = self.working_to_reference()?;
        chain = chain.then(self.looks_chain(&view.looks)?);

        if let Some(name) = &view.view_transform {
            // The v2 shape: scene reference → display reference → the display's
            // own colour space.
            let transform = self.config.view_transforms.get(name).ok_or_else(|| {
                ColourError::UnknownColourSpace {
                    name: name.to_string(),
                }
            })?;
            let step = match (
                &transform.from_scene_reference,
                &transform.to_scene_reference,
            ) {
                (Some(spec), _) => self.resolve_spec(&transform.name, spec, 0)?,
                (None, Some(spec)) => self
                    .resolve_spec(&transform.name, spec, 0)?
                    .inverted(&transform.name)?,
                (None, None) => Chain::identity(),
            };
            chain = chain.then(step);
            let display_space =
                view.display_colour_space
                    .as_deref()
                    .ok_or_else(|| ColourError::UnknownView {
                        display: display.to_string(),
                        view: view.name.clone(),
                    })?;
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
    fn looks_chain(&self, looks: &str) -> Result<Chain> {
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
                let mut m34 = [0.0_f32; 12];
                for row in 0..3 {
                    for col in 0..3 {
                        m34[row * 4 + col] = m.get(row * 4 + col).copied().unwrap_or(0.0);
                    }
                    m34[row * 4 + 3] = offset.get(row).copied().unwrap_or(0.0);
                }
                let m34 = match dir {
                    Direction::Forward => m34,
                    Direction::Inverse => matrix::invert(&m34)?,
                };
                Chain::new(vec![Op::Matrix(m34)])
            }
            TransformSpec::Exponent { value, dir } => Chain::new(vec![Op::Exponent {
                exp: *value,
                dir: *dir,
            }]),
            TransformSpec::ExponentWithLinear { gamma, offset, dir } => {
                Chain::new(vec![Op::MonCurve {
                    gamma: *gamma,
                    offset: *offset,
                    dir: *dir,
                }])
            }
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
                let chain = file::load(&path)?;
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
        })
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
/// every colour space a config declares, walked to a chain. Used by the refusal
/// taxonomy test, and by WP3's load path to decide whether a config is usable.
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
}
