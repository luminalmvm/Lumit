//! The `config.ocio` grammar.
//!
//! In plain terms: an OCIO config is a YAML file with one unusual habit — it
//! labels its values with **tags**, `!<ColorSpace>` or `!<MatrixTransform>`, and
//! the tag is what says which kind of thing follows. Most YAML libraries hide
//! tags; this module reads the file as a stream of events with `yaml-rust2`,
//! which does not, and walks it by hand into the structs below. The grammar is
//! small enough that hand-walking is less code than fighting a derive.
//!
//! Nothing here resolves anything: this is what the file *says*, not what it
//! means. [`crate::resolve`] turns it into chains.
//!
//! Traps, written down so they are not re-derived (docs/impl/ocio.md §4.4):
//!
//! - **Anchors and merge keys are legal and real configs use them.** An alias
//!   repeats a value declared elsewhere; a `<<` key merges a whole mapping in.
//!   Both are resolved here.
//! - **`search_path` is colon-separated or a list**, its entries resolve
//!   against the config file's own directory, in order, first hit wins.
//! - **`ocio_profile_version` gates the grammar.** Versions 1 and 2 are read;
//!   anything higher is refused by name rather than guessed at.
//! - **`inactive_colorspaces` hides names from lists but keeps them
//!   resolvable** — a hidden space is still a legal target.
//! - **`aliases` are names too.** A transform may name a space by any of
//!   them, and Blender's config reaches its XYZ spaces that way.
//! - **Displays, views and colour spaces are three separate namespaces.** The
//!   same word can mean three things in one file.
//! - Config-supplied names are **user data, not engine strings**: they cross the
//!   bridge verbatim and get no `app_en.arb` keys, exactly like file names.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use yaml_rust2::parser::{Event, Parser, Tag};

use crate::error::{ColourError, Result};
use crate::op::Direction;

// ---------------------------------------------------------------------------
// A tagged YAML tree — the shape this module walks.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
enum Node {
    #[default]
    Empty,
    Scalar(String),
    Seq(Vec<Tagged>),
    /// Mapping entries in declaration order; a config's own order is what
    /// resolution follows (docs/impl/ocio.md §4.2).
    Map(Vec<(String, Tagged)>),
}

#[derive(Debug, Clone, Default)]
struct Tagged {
    tag: Option<String>,
    node: Node,
}

impl Tagged {
    fn scalar(&self) -> Option<&str> {
        match &self.node {
            Node::Scalar(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn seq(&self) -> &[Tagged] {
        match &self.node {
            Node::Seq(items) => items,
            _ => &[],
        }
    }

    fn entries(&self) -> &[(String, Tagged)] {
        match &self.node {
            Node::Map(entries) => entries,
            _ => &[],
        }
    }

    fn get(&self, key: &str) -> Option<&Tagged> {
        self.entries()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    fn string(&self, key: &str) -> Option<String> {
        self.get(key).and_then(Tagged::scalar).map(str::to_string)
    }

    fn number(&self, key: &str) -> Option<f32> {
        self.get(key)
            .and_then(Tagged::scalar)
            .and_then(|s| s.trim().parse::<f32>().ok())
    }

    fn bool(&self, key: &str) -> bool {
        matches!(
            self.get(key).and_then(Tagged::scalar),
            Some("true" | "yes" | "on")
        )
    }

    /// A value that may be one number or three: OCIO writes both.
    fn triple(&self, key: &str, default: f32) -> [f32; 3] {
        let Some(value) = self.get(key) else {
            return [default; 3];
        };
        if let Some(one) = value.scalar().and_then(|s| s.trim().parse::<f32>().ok()) {
            return [one; 3];
        }
        let items: Vec<f32> = value
            .seq()
            .iter()
            .filter_map(|t| t.scalar())
            .filter_map(|s| s.trim().parse::<f32>().ok())
            .collect();
        match items.as_slice() {
            [r, g, b] | [r, g, b, _] => [*r, *g, *b],
            [one] => [*one; 3],
            _ => [default; 3],
        }
    }

    /// A number kept in double. The grading transforms work their per channel
    /// numbers out in double before rounding once for the pixel loop, exactly as
    /// the reference library does, so their values are read at that precision.
    fn number64(&self, key: &str) -> Option<f64> {
        self.get(key)
            .and_then(Tagged::scalar)
            .and_then(|s| s.trim().parse::<f64>().ok())
    }

    fn floats64(&self, key: &str) -> Vec<f64> {
        self.get(key)
            .map(|v| {
                v.seq()
                    .iter()
                    .filter_map(|t| t.scalar())
                    .filter_map(|s| s.trim().parse::<f64>().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn floats(&self, key: &str) -> Vec<f32> {
        self.get(key)
            .map(|v| {
                v.seq()
                    .iter()
                    .filter_map(|t| t.scalar())
                    .filter_map(|s| s.trim().parse::<f32>().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A value that may be a list of strings or one comma/colon-separated string.
    fn string_list(&self, key: &str, separators: &[char]) -> Vec<String> {
        let Some(value) = self.get(key) else {
            return Vec::new();
        };
        if let Some(text) = value.scalar() {
            return text
                .split(|c| separators.contains(&c))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        value
            .seq()
            .iter()
            .filter_map(|t| t.scalar())
            .map(str::to_string)
            .collect()
    }

    fn direction(&self) -> Direction {
        match self.string("direction").as_deref() {
            Some("inverse") => Direction::Inverse,
            _ => Direction::Forward,
        }
    }
}

fn tag_name(tag: &Option<Tag>) -> Option<String> {
    tag.as_ref().map(|t| {
        let suffix = t.suffix.trim_start_matches('!');
        // A verbatim `!<Name>` arrives as handle "!" and suffix "Name"; a
        // shorthand `!Name` the same way. Either is the type name.
        suffix.to_string()
    })
}

fn parse_document(what: &str, text: &str) -> Result<Tagged> {
    let mut parser = Parser::new_from_str(text).keep_tags(true);
    let mut anchors: BTreeMap<usize, Tagged> = BTreeMap::new();
    let bad = |reason: String| ColourError::Parse {
        what: what.to_string(),
        reason,
    };

    // Walk to the first document's root node.
    loop {
        let (event, _) = parser
            .peek()
            .map_err(|e| bad(format!("the YAML could not be read ({e})")))?
            .clone();
        match event {
            Event::StreamStart | Event::DocumentStart | Event::Nothing => {
                parser
                    .next_token()
                    .map_err(|e| bad(format!("the YAML could not be read ({e})")))?;
            }
            Event::StreamEnd | Event::DocumentEnd => {
                return Err(bad("the file holds no YAML document".to_string()))
            }
            _ => break,
        }
    }
    parse_node(what, &mut parser, &mut anchors)
}

fn parse_node(
    what: &str,
    parser: &mut Parser<std::str::Chars<'_>>,
    anchors: &mut BTreeMap<usize, Tagged>,
) -> Result<Tagged> {
    let bad = |reason: String| ColourError::Parse {
        what: what.to_string(),
        reason,
    };
    let (event, _) = parser
        .next_token()
        .map_err(|e| bad(format!("the YAML could not be read ({e})")))?;
    Ok(match event {
        Event::Scalar(value, _, anchor, tag) => {
            let node = Tagged {
                tag: tag_name(&tag),
                node: Node::Scalar(value),
            };
            if anchor > 0 {
                anchors.insert(anchor, node.clone());
            }
            node
        }
        Event::Alias(anchor) => anchors.get(&anchor).cloned().unwrap_or_default(),
        Event::SequenceStart(anchor, tag) => {
            let mut items = Vec::new();
            loop {
                let (peeked, _) = parser
                    .peek()
                    .map_err(|e| bad(format!("the YAML could not be read ({e})")))?
                    .clone();
                if matches!(peeked, Event::SequenceEnd) {
                    parser
                        .next_token()
                        .map_err(|e| bad(format!("the YAML could not be read ({e})")))?;
                    break;
                }
                items.push(parse_node(what, parser, anchors)?);
            }
            let node = Tagged {
                tag: tag_name(&tag),
                node: Node::Seq(items),
            };
            if anchor > 0 {
                anchors.insert(anchor, node.clone());
            }
            node
        }
        Event::MappingStart(anchor, tag) => {
            let mut entries: Vec<(String, Tagged)> = Vec::new();
            loop {
                let (peeked, _) = parser
                    .peek()
                    .map_err(|e| bad(format!("the YAML could not be read ({e})")))?
                    .clone();
                if matches!(peeked, Event::MappingEnd) {
                    parser
                        .next_token()
                        .map_err(|e| bad(format!("the YAML could not be read ({e})")))?;
                    break;
                }
                let key = parse_node(what, parser, anchors)?;
                let value = parse_node(what, parser, anchors)?;
                let key = key.scalar().unwrap_or_default().to_string();
                if key == "<<" {
                    // A merge key: the referenced mapping's entries fill in
                    // whatever this one does not state itself.
                    let sources: Vec<&Tagged> = match &value.node {
                        Node::Seq(items) => items.iter().collect(),
                        _ => vec![&value],
                    };
                    for source in sources {
                        for (k, v) in source.entries() {
                            if !entries.iter().any(|(existing, _)| existing == k) {
                                entries.push((k.clone(), v.clone()));
                            }
                        }
                    }
                    continue;
                }
                entries.push((key, value));
            }
            let node = Tagged {
                tag: tag_name(&tag),
                node: Node::Map(entries),
            };
            if anchor > 0 {
                anchors.insert(anchor, node.clone());
            }
            node
        }
        other => return Err(bad(format!("unexpected YAML event {other:?}"))),
    })
}

// ---------------------------------------------------------------------------
// What a config says.
// ---------------------------------------------------------------------------

/// A transform as the config states it, before anything is followed up.
#[derive(Debug, Clone, PartialEq)]
pub enum TransformSpec {
    Group(Vec<TransformSpec>, Direction),
    /// 4×4 row-major with a 4-vector offset, as the config writes it; only the
    /// upper 3×4 reaches a chain.
    Matrix {
        matrix: [f32; 16],
        offset: [f32; 4],
        dir: Direction,
    },
    Exponent {
        value: [f32; 3],
        negatives: Option<crate::op::Negatives>,
        dir: Direction,
    },
    ExponentWithLinear {
        gamma: [f32; 3],
        offset: [f32; 3],
        negatives: Option<crate::op::Negatives>,
        dir: Direction,
    },
    Log {
        params: crate::op::LogParams,
        dir: Direction,
    },
    Cdl {
        params: crate::op::CdlParams,
        dir: Direction,
    },
    Range(crate::op::RangeParams, Direction),
    File {
        src: String,
        dir: Direction,
    },
    ColourSpace {
        src: String,
        dst: String,
        dir: Direction,
    },
    Builtin {
        style: String,
        dir: Direction,
    },
    GradingPrimary {
        params: Box<crate::op::GradingPrimary>,
        dir: Direction,
    },
    GradingTone {
        params: Box<crate::op::GradingTone>,
        dir: Direction,
    },
    /// A transform Lumit does not implement, kept by name so that only the
    /// space, view or look that needs it refuses, and only when asked for.
    Unsupported {
        name: String,
    },
}

/// How a space says its values are spread, which sets the shaper a bake uses
/// when this space is the domain (docs/impl/ocio.md §5.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Allocation {
    Uniform { min: f32, max: f32 },
    Lg2 { min: f32, max: f32, offset: f32 },
}

#[derive(Debug, Clone, Default)]
pub struct ColourSpace {
    pub name: String,
    pub family: String,
    /// Other names the space answers to, good anywhere the name itself is.
    pub aliases: Vec<String>,
    /// A data space carries no colour and is never transformed.
    pub is_data: bool,
    pub to_reference: Option<TransformSpec>,
    pub from_reference: Option<TransformSpec>,
    pub allocation: Option<Allocation>,
    /// True for a `display_colorspaces` entry, whose reference is the display
    /// reference rather than the scene one.
    pub display_referred: bool,
}

#[derive(Debug, Clone, Default)]
pub struct View {
    pub name: String,
    /// The v1 spelling: the display colour space, directly.
    pub colour_space: Option<String>,
    /// The v2 spelling: a view transform into the display reference, then a
    /// display colour space out of it.
    pub view_transform: Option<String>,
    pub display_colour_space: Option<String>,
    /// `looks:` — comma-separated, `+name` forward, `-name` inverted.
    pub looks: String,
}

#[derive(Debug, Clone, Default)]
pub struct Display {
    pub name: String,
    pub views: Vec<View>,
}

#[derive(Debug, Clone, Default)]
pub struct Look {
    pub name: String,
    pub process_space: String,
    pub transform: Option<TransformSpec>,
    pub inverse_transform: Option<TransformSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct ViewTransform {
    pub name: String,
    /// Scene reference → display reference.
    pub from_scene_reference: Option<TransformSpec>,
    /// Display reference → scene reference.
    pub to_scene_reference: Option<TransformSpec>,
    /// The display-referred half, kept apart from the scene-referred one
    /// because they are not two spellings of the same thing. A view transform
    /// with **only** these does no rendering of its own: it borrows the
    /// config's `default_view_transform` to reach the display reference and
    /// then applies this on top. The ACES v2 configs' "Video (colorimetric)"
    /// view is exactly that, and folding these keys into the scene-referred
    /// ones — as this parser first did — inverts a transform that was never
    /// meant to run backwards.
    pub from_display_reference: Option<TransformSpec>,
    pub to_display_reference: Option<TransformSpec>,
}

/// Everything a `config.ocio` states.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub version: u32,
    /// What the config calls itself: its `name`, or the first line of its
    /// `description` when it has no name, or empty. The OCIO effects' read-only
    /// Information row shows it.
    pub name: String,
    /// The directory the config file lives in; every relative path is relative
    /// to it (nothing absolute is ever written back).
    pub dir: PathBuf,
    pub search_paths: Vec<String>,
    pub roles: BTreeMap<String, String>,
    pub spaces: BTreeMap<String, ColourSpace>,
    /// Declaration order, which is the order lists are shown in.
    pub space_order: Vec<String>,
    /// Every alias a space declares, to the space's own name.
    pub aliases: BTreeMap<String, String>,
    /// Every space name and alias lowercased, to the space's own name. The
    /// reference library matches names without regard to case, and the
    /// PixelManager config leans on that.
    pub lowercase: BTreeMap<String, String>,
    pub inactive: BTreeSet<String>,
    pub displays: Vec<Display>,
    pub active_displays: Vec<String>,
    pub active_views: Vec<String>,
    pub shared_views: Vec<View>,
    pub looks: BTreeMap<String, Look>,
    pub view_transforms: BTreeMap<String, ViewTransform>,
    /// `default_view_transform`: the one a view borrows when it states only a
    /// display-referred transform of its own.
    pub default_view_transform: Option<String>,
}

/// Transform tags a config may legally carry that v1 does not implement. Named
/// here so the refusal says the config's own word back to the user.
const REFUSED_TRANSFORMS: [&str; 6] = [
    "FixedFunctionTransform",
    "GradingRGBCurveTransform",
    "ExposureContrastTransform",
    "LookTransform",
    "DisplayViewTransform",
    "CDLTransform:cccid",
];

/// The negative style an exponent transform declares, or `None` for the
/// transform's own default.
///
/// **The key is `style`, not `negativeStyle`.** `negativeStyle` is the name the
/// C++ API and the CLF grammar use; a config *file* writes `style`, and reading
/// for the wrong one finds nothing and silently applies the default. That is
/// how a `pass_thru` gamma space read as a clamping one for as long as this
/// parser existed — visible only below zero, which is precisely where nobody
/// looks.
fn negative_style(
    value: &Tagged,
    tag: &str,
    own_default: &str,
) -> Result<Option<crate::op::Negatives>> {
    let Some(style) = value.string("style") else {
        return Ok(None);
    };
    match style.as_str() {
        s if s == own_default => Ok(None),
        "mirror" => Ok(Some(crate::op::Negatives::Mirror)),
        "pass_thru" => Ok(Some(crate::op::Negatives::PassThru)),
        other => Err(ColourError::UnsupportedTransform {
            name: format!("{tag} with style {other}"),
        }),
    }
}

/// The style a grading transform names, or `log`, which is what the reference
/// library builds one as when a config says nothing.
fn grading_style(value: &Tagged, tag: &str) -> Result<crate::op::GradingStyle> {
    use crate::op::GradingStyle;
    match value.string("style").as_deref() {
        None | Some("log") => Ok(GradingStyle::Log),
        Some("linear") => Ok(GradingStyle::Lin),
        Some("video") => Ok(GradingStyle::Video),
        Some(other) => Err(ColourError::UnsupportedTransform {
            name: format!("{tag} with style {other}"),
        }),
    }
}

/// One `{rgb: [r, g, b], master: m}` block. The reference insists on both keys
/// and so does this: a half-written block would otherwise read as a default and
/// grade differently here than everywhere else.
fn grading_rgbm(
    what: &str,
    node: &Tagged,
    key: &str,
    default: crate::op::GradingRgbm,
) -> Result<crate::op::GradingRgbm> {
    let Some(value) = node.get(key) else {
        return Ok(default);
    };
    let rgb = value.floats64("rgb");
    let (Some(master), [r, g, b]) = (value.number64("master"), rgb.as_slice()) else {
        return Err(ColourError::Parse {
            what: what.to_string(),
            reason: format!("a grading {key} needs both an rgb triple and a master"),
        });
    };
    Ok(crate::op::GradingRgbm {
        rgb: [*r, *g, *b],
        master,
    })
}

/// One tone band. Its two extra numbers are spelled differently per band, so the
/// caller passes the two key names the reference reads for that one.
fn grading_rgbmsw(
    what: &str,
    node: &Tagged,
    key: &str,
    start_key: &str,
    width_key: &str,
    default: crate::op::GradingRgbmsw,
) -> Result<crate::op::GradingRgbmsw> {
    let Some(value) = node.get(key) else {
        return Ok(default);
    };
    let rgb = value.floats64("rgb");
    let (Some(master), Some(start), Some(width), [r, g, b]) = (
        value.number64("master"),
        value.number64(start_key),
        value.number64(width_key),
        rgb.as_slice(),
    ) else {
        return Err(ColourError::Parse {
            what: what.to_string(),
            reason: format!("a grading {key} needs rgb, master, {start_key} and {width_key}"),
        });
    };
    Ok(crate::op::GradingRgbmsw {
        rgb: [*r, *g, *b],
        master,
        start,
        width,
    })
}

/// A transform outside the op set is not a reason to refuse the whole file:
/// Blender's config keeps transforms in looks most people never pick. The name
/// is kept, and the resolve says it back when something asks.
fn parse_transform(what: &str, value: &Tagged) -> Result<TransformSpec> {
    match parse_transform_kind(what, value) {
        Err(ColourError::UnsupportedTransform { name }) => Ok(TransformSpec::Unsupported { name }),
        other => other,
    }
}

fn parse_transform_kind(what: &str, value: &Tagged) -> Result<TransformSpec> {
    let tag = value.tag.clone().unwrap_or_default();
    if REFUSED_TRANSFORMS.contains(&tag.as_str()) {
        return Err(ColourError::UnsupportedTransform { name: tag });
    }
    let dir = value.direction();
    Ok(match tag.as_str() {
        "GroupTransform" => {
            let mut children = Vec::new();
            for child in value.get("children").map(Tagged::seq).unwrap_or_default() {
                children.push(parse_transform(what, child)?);
            }
            // An inverse group is its children reversed, each inverted —
            // resolution's job, so the direction travels with the group.
            TransformSpec::Group(children, dir)
        }
        "MatrixTransform" => {
            let numbers = value.floats("matrix");
            let mut matrix = [0.0_f32; 16];
            for (i, m) in matrix.iter_mut().enumerate() {
                *m = numbers
                    .get(i)
                    .copied()
                    .unwrap_or(if i % 5 == 0 { 1.0 } else { 0.0 });
            }
            let offsets = value.floats("offset");
            let mut offset = [0.0_f32; 4];
            for (i, o) in offset.iter_mut().enumerate() {
                *o = offsets.get(i).copied().unwrap_or(0.0);
            }
            TransformSpec::Matrix {
                matrix,
                offset,
                dir,
            }
        }
        "ExponentTransform" => TransformSpec::Exponent {
            value: value.triple("value", 1.0),
            negatives: negative_style(value, &tag, "clamp")?,
            dir,
        },
        "ExponentWithLinearTransform" => TransformSpec::ExponentWithLinear {
            gamma: value.triple("gamma", 1.0),
            offset: value.triple("offset", 0.0),
            negatives: negative_style(value, &tag, "linear")?,
            dir,
        },
        "LogTransform" => TransformSpec::Log {
            params: crate::op::LogParams::plain(value.number("base").unwrap_or(2.0)),
            dir,
        },
        "LogAffineTransform" | "LogCameraTransform" => {
            let camera = tag == "LogCameraTransform";
            // A config file writes these keys snake_case, `lin_side_break`,
            // and the CLF grammar camelCase, so either spelling is read.
            let key = |snake: &'static str, camel: &'static str| -> &'static str {
                if value.get(snake).is_some() {
                    snake
                } else {
                    camel
                }
            };
            let lin_side_break = key("lin_side_break", "linSideBreak");
            let linear_slope = key("linear_slope", "linearSlope");
            let params = crate::op::LogParams {
                base: value.number("base").unwrap_or(2.0),
                lin_side_slope: value.triple(key("lin_side_slope", "linSideSlope"), 1.0),
                lin_side_offset: value.triple(key("lin_side_offset", "linSideOffset"), 0.0),
                log_side_slope: value.triple(key("log_side_slope", "logSideSlope"), 1.0),
                log_side_offset: value.triple(key("log_side_offset", "logSideOffset"), 0.0),
                lin_side_break: camera.then(|| value.triple(lin_side_break, 0.0)),
                linear_slope: value
                    .get(linear_slope)
                    .map(|_| value.triple(linear_slope, 1.0)),
            };
            if camera && value.get(lin_side_break).is_none() {
                return Err(ColourError::Parse {
                    what: what.to_string(),
                    reason: "a LogCameraTransform must state its lin_side_break".to_string(),
                });
            }
            TransformSpec::Log { params, dir }
        }
        // The `allocation` line as a transform, which is what the Filmic and
        // AgX log spaces in Blender's configs are made of: a fit of [min, max]
        // onto 0 to 1, with a log2 in front for `lg2` and the third var as its
        // lin-side offset. Both are ops the chain already has.
        "AllocationTransform" => {
            let kind = value.string("allocation").unwrap_or_default();
            let vars = value.floats("vars");
            let (min, max) = match (kind.as_str(), vars.as_slice()) {
                (_, [min, max, ..]) => (*min, *max),
                ("lg2", _) => (-10.0, 6.0),
                _ => (0.0, 1.0),
            };
            let span = max - min;
            if span == 0.0 || !span.is_finite() {
                return Err(ColourError::Parse {
                    what: what.to_string(),
                    reason: format!("an AllocationTransform whose vars {vars:?} span nothing"),
                });
            }
            let (scale, shift) = (1.0 / span, -min / span);
            match kind.as_str() {
                "lg2" => TransformSpec::Log {
                    params: crate::op::LogParams {
                        base: 2.0,
                        lin_side_slope: [1.0; 3],
                        lin_side_offset: [vars.get(2).copied().unwrap_or(0.0); 3],
                        log_side_slope: [scale; 3],
                        log_side_offset: [shift; 3],
                        lin_side_break: None,
                        linear_slope: None,
                    },
                    dir,
                },
                "uniform" => TransformSpec::Matrix {
                    matrix: [
                        scale, 0.0, 0.0, 0.0, //
                        0.0, scale, 0.0, 0.0, //
                        0.0, 0.0, scale, 0.0, //
                        0.0, 0.0, 0.0, 1.0,
                    ],
                    offset: [shift, shift, shift, 0.0],
                    dir,
                },
                other => {
                    return Err(ColourError::UnsupportedTransform {
                        name: format!("AllocationTransform with allocation {other}"),
                    })
                }
            }
        }
        "CDLTransform" => {
            if value.get("cccid").is_some() {
                return Err(ColourError::UnsupportedTransform {
                    name: "a CDLTransform that selects a grade by cccid".to_string(),
                });
            }
            TransformSpec::Cdl {
                params: crate::op::CdlParams {
                    slope: value.triple("slope", 1.0),
                    offset: value.triple("offset", 0.0),
                    power: value.triple("power", 1.0),
                    saturation: value.number("sat").unwrap_or(1.0),
                    clamp: !matches!(
                        value.string("style").as_deref(),
                        Some("no_clamp" | "noClamp")
                    ),
                },
                dir,
            }
        }
        "GradingPrimaryTransform" => {
            let style = grading_style(value, &tag)?;
            let d = crate::op::GradingPrimary::new(style);
            let rgbm = |key: &str, default| grading_rgbm(what, value, key, default);
            // `pivot` and `clamp` are little maps of their own, and each key
            // inside them stands alone: a config may state a black clamp and no
            // white one, and the unstated half keeps the style's default.
            let pivot = value.get("pivot");
            let clamp = value.get("clamp");
            let from = |node: Option<&Tagged>, key: &'static str, default: f64| {
                node.and_then(|n| n.number64(key)).unwrap_or(default)
            };
            TransformSpec::GradingPrimary {
                params: Box::new(crate::op::GradingPrimary {
                    brightness: rgbm("brightness", d.brightness)?,
                    contrast: rgbm("contrast", d.contrast)?,
                    gamma: rgbm("gamma", d.gamma)?,
                    offset: rgbm("offset", d.offset)?,
                    exposure: rgbm("exposure", d.exposure)?,
                    lift: rgbm("lift", d.lift)?,
                    gain: rgbm("gain", d.gain)?,
                    saturation: value.number64("saturation").unwrap_or(d.saturation),
                    pivot: from(pivot, "contrast", d.pivot),
                    pivot_black: from(pivot, "black", d.pivot_black),
                    pivot_white: from(pivot, "white", d.pivot_white),
                    clamp_black: from(clamp, "black", d.clamp_black),
                    clamp_white: from(clamp, "white", d.clamp_white),
                    ..d
                }),
                dir,
            }
        }
        "GradingToneTransform" => {
            let style = grading_style(value, &tag)?;
            let d = crate::op::GradingToneValues::new(style);
            let band = |key: &str, start_key: &str, width_key: &str, default| {
                grading_rgbmsw(what, value, key, start_key, width_key, default)
            };
            TransformSpec::GradingTone {
                params: Box::new(crate::op::GradingTone::new(crate::op::GradingToneValues {
                    blacks: band("blacks", "start", "width", d.blacks)?,
                    shadows: band("shadows", "start", "pivot", d.shadows)?,
                    midtones: band("midtones", "center", "width", d.midtones)?,
                    highlights: band("highlights", "start", "pivot", d.highlights)?,
                    whites: band("whites", "start", "width", d.whites)?,
                    scontrast: value.number64("s_contrast").unwrap_or(d.scontrast),
                    ..d
                })),
                dir,
            }
        }
        "RangeTransform" => TransformSpec::Range(
            crate::op::RangeParams {
                min_in: value.number("minInValue"),
                max_in: value.number("maxInValue"),
                min_out: value.number("minOutValue"),
                max_out: value.number("maxOutValue"),
                no_clamp: matches!(
                    value.string("style").as_deref(),
                    Some("noClamp" | "no_clamp")
                ),
            },
            dir,
        ),
        "FileTransform" => {
            let src = value.string("src").unwrap_or_default();
            if value.get("cccid").is_some() {
                return Err(ColourError::UnsupportedTransform {
                    name: "a FileTransform that selects a grade by cccid".to_string(),
                });
            }
            TransformSpec::File { src, dir }
        }
        "ColorSpaceTransform" => TransformSpec::ColourSpace {
            src: value.string("src").unwrap_or_default(),
            dst: value.string("dst").unwrap_or_default(),
            dir,
        },
        "BuiltinTransform" => TransformSpec::Builtin {
            style: value.string("style").unwrap_or_default(),
            dir,
        },
        "" => {
            return Err(ColourError::Parse {
                what: what.to_string(),
                reason: "a transform with no !<…> tag saying what it is".to_string(),
            })
        }
        other => {
            return Err(ColourError::UnsupportedTransform {
                name: other.to_string(),
            })
        }
    })
}

fn parse_allocation(entry: &Tagged) -> Option<Allocation> {
    let vars = entry.floats("allocationvars");
    match entry.string("allocation").as_deref() {
        Some("lg2") => Some(match vars.as_slice() {
            [min, max] => Allocation::Lg2 {
                min: *min,
                max: *max,
                offset: 0.0,
            },
            [min, max, offset] => Allocation::Lg2 {
                min: *min,
                max: *max,
                offset: *offset,
            },
            _ => return None,
        }),
        Some("uniform") => match vars.as_slice() {
            [min, max] => Some(Allocation::Uniform {
                min: *min,
                max: *max,
            }),
            _ => None,
        },
        _ => None,
    }
}

fn parse_space(what: &str, entry: &Tagged, display_referred: bool) -> Result<ColourSpace> {
    let name = entry.string("name").unwrap_or_default();
    let (to_key, from_key) = if display_referred {
        ("to_display_reference", "from_display_reference")
    } else {
        ("to_scene_reference", "from_scene_reference")
    };
    let read = |primary: &str, legacy: &str| -> Result<Option<TransformSpec>> {
        match entry.get(primary).or_else(|| entry.get(legacy)) {
            Some(v) => Ok(Some(parse_transform(what, v)?)),
            None => Ok(None),
        }
    };
    Ok(ColourSpace {
        family: entry.string("family").unwrap_or_default(),
        aliases: entry.string_list("aliases", &[',']),
        is_data: entry.bool("isdata"),
        to_reference: read(to_key, "to_reference")?,
        from_reference: read(from_key, "from_reference")?,
        allocation: parse_allocation(entry),
        display_referred,
        name,
    })
}

fn parse_view(entry: &Tagged) -> View {
    View {
        name: entry.string("name").unwrap_or_default(),
        colour_space: entry.string("colorspace"),
        view_transform: entry.string("view_transform"),
        display_colour_space: entry.string("display_colorspace"),
        looks: entry.string("looks").unwrap_or_default(),
    }
}

impl Config {
    /// Read a `config.ocio` from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| ColourError::FileRead {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Self::parse(&dir, &text)
    }

    /// The grammar half of [`Config::load`], split out so tests need no files.
    pub fn parse(dir: &Path, text: &str) -> Result<Self> {
        let what = "this config";
        let root = parse_document(what, text)?;

        let version_text = root
            .get("ocio_profile_version")
            .and_then(Tagged::scalar)
            .unwrap_or("1")
            .to_string();
        // "2.1" and friends are legal; the major number is what gates grammar.
        let major = version_text
            .split('.')
            .next()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0);
        if !(1..=2).contains(&major) {
            return Err(ColourError::UnsupportedConfigVersion {
                version: version_text,
            });
        }

        let mut config = Config {
            version: major,
            name: root
                .string("name")
                .or_else(|| {
                    root.string("description")
                        .map(|d| d.lines().next().unwrap_or_default().trim().to_string())
                })
                .unwrap_or_default(),
            dir: dir.to_path_buf(),
            search_paths: root.string_list("search_path", &[':']),
            active_displays: root.string_list("active_displays", &[',']),
            active_views: root.string_list("active_views", &[',']),
            default_view_transform: root.string("default_view_transform"),
            inactive: root
                .string_list("inactive_colorspaces", &[','])
                .into_iter()
                .collect(),
            ..Config::default()
        };
        if config.search_paths.is_empty() {
            // OCIO's own default: the config's directory itself.
            config.search_paths.push(".".to_string());
        }

        for (role, target) in root.get("roles").map(Tagged::entries).unwrap_or_default() {
            if let Some(space) = target.scalar() {
                config.roles.insert(role.clone(), space.to_string());
            }
        }

        for (key, display_referred) in [("colorspaces", false), ("display_colorspaces", true)] {
            for entry in root.get(key).map(Tagged::seq).unwrap_or_default() {
                let space = parse_space(what, entry, display_referred)?;
                if space.name.is_empty() {
                    continue;
                }
                config.space_order.push(space.name.clone());
                config
                    .lowercase
                    .insert(space.name.to_lowercase(), space.name.clone());
                for alias in &space.aliases {
                    config.aliases.insert(alias.clone(), space.name.clone());
                    config
                        .lowercase
                        .insert(alias.to_lowercase(), space.name.clone());
                }
                config.spaces.insert(space.name.clone(), space);
            }
        }

        for entry in root
            .get("view_transforms")
            .map(Tagged::seq)
            .unwrap_or_default()
        {
            let name = entry.string("name").unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let read = |key: &str| -> Result<Option<TransformSpec>> {
                match entry.get(key) {
                    Some(v) => Ok(Some(parse_transform(what, v)?)),
                    None => Ok(None),
                }
            };
            // A view transform maps between the scene and display references;
            // a config may state either side, and the other is its inverse.
            config.view_transforms.insert(
                name.clone(),
                ViewTransform {
                    from_scene_reference: read("from_scene_reference")?.or(read("from_reference")?),
                    to_scene_reference: read("to_scene_reference")?.or(read("to_reference")?),
                    from_display_reference: read("from_display_reference")?,
                    to_display_reference: read("to_display_reference")?,
                    name,
                },
            );
        }

        for entry in root.get("looks").map(Tagged::seq).unwrap_or_default() {
            let name = entry.string("name").unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let read = |key: &str| -> Result<Option<TransformSpec>> {
                match entry.get(key) {
                    Some(v) => Ok(Some(parse_transform(what, v)?)),
                    None => Ok(None),
                }
            };
            config.looks.insert(
                name.clone(),
                Look {
                    process_space: entry.string("process_space").unwrap_or_default(),
                    transform: read("transform")?,
                    inverse_transform: read("inverse_transform")?,
                    name,
                },
            );
        }

        for entry in root
            .get("shared_views")
            .map(Tagged::seq)
            .unwrap_or_default()
        {
            config.shared_views.push(parse_view(entry));
        }

        for (name, views) in root
            .get("displays")
            .map(Tagged::entries)
            .unwrap_or_default()
        {
            let mut display = Display {
                name: name.clone(),
                views: Vec::new(),
            };
            for entry in views.seq() {
                // `!<Views> [a, b]` names shared views; `!<View> {…}` states one.
                if entry.tag.as_deref() == Some("Views") {
                    for shared in entry.seq() {
                        if let Some(view_name) = shared.scalar() {
                            display.views.push(View {
                                name: view_name.to_string(),
                                ..View::default()
                            });
                        }
                    }
                    continue;
                }
                display.views.push(parse_view(entry));
            }
            config.displays.push(display);
        }

        Ok(config)
    }

    /// The colour spaces a picker should list: declaration order, inactive ones
    /// hidden. A hidden space stays resolvable by name (§4.4).
    #[must_use]
    pub fn active_space_names(&self) -> Vec<&str> {
        self.space_order
            .iter()
            .filter(|n| !self.inactive.contains(*n))
            .map(String::as_str)
            .collect()
    }

    /// The looks a picker should list, by name.
    #[must_use]
    pub fn look_names(&self) -> Vec<&str> {
        self.looks.keys().map(String::as_str).collect()
    }

    /// Follow a role to the space it names, one indirection (§4.2).
    #[must_use]
    pub fn role(&self, name: &str) -> Option<&str> {
        self.roles.get(name).map(String::as_str)
    }

    /// The space a name stands for: the name itself, one of a space's aliases,
    /// or either of those matched without regard to case, which is how the
    /// reference library matches. Blender's config reaches its XYZ spaces by
    /// alias and the PixelManager config a Sony space by a different case.
    #[must_use]
    pub fn canonical<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        if self.spaces.contains_key(name) {
            return Some(name);
        }
        self.aliases
            .get(name)
            .or_else(|| self.lowercase.get(&name.to_lowercase()))
            .map(String::as_str)
    }

    #[must_use]
    pub fn display(&self, name: &str) -> Option<&Display> {
        self.displays.iter().find(|d| d.name == name)
    }

    /// A display's view by name, falling back to the shared views a `!<Views>`
    /// entry refers to.
    #[must_use]
    pub fn view(&self, display: &str, view: &str) -> Option<&View> {
        let found = self
            .display(display)?
            .views
            .iter()
            .find(|v| v.name == view)?;
        if found.colour_space.is_some() || found.display_colour_space.is_some() {
            return Some(found);
        }
        self.shared_views
            .iter()
            .find(|v| v.name == view)
            .or(Some(found))
    }

    /// Where a look-up table named by a `FileTransform` actually lives:
    /// each `search_path` entry in order, relative to the config's directory,
    /// first hit wins (§4.4).
    pub fn resolve_lut_path(&self, src: &str) -> Result<PathBuf> {
        if src.contains('$') || src.contains('%') {
            return Err(ColourError::ContextVariable {
                path: src.to_string(),
            });
        }
        for entry in &self.search_paths {
            let candidate = self.dir.join(entry).join(src);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        Err(ColourError::LutFileNotFound {
            name: src.to_string(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
ocio_profile_version: 2
search_path: luts:more_luts
roles:
  scene_linear: linear
  aces_interchange: ACES2065-1
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: out_srgb}
    - !<View> {name: Raw, colorspace: linear}
active_displays: [sRGB]
inactive_colorspaces: [hidden]
colorspaces:
  - !<ColorSpace>
    name: linear
    family: scene
    allocation: lg2
    allocationvars: [-8, 5, 0.00390625]
  - !<ColorSpace>
    name: ACES2065-1
    to_scene_reference: !<MatrixTransform> {matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]}
  - !<ColorSpace>
    name: out_srgb
    from_scene_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}
  - !<ColorSpace>
    name: hidden
    isdata: true
"#;

    fn minimal() -> Config {
        Config::parse(Path::new("."), MINIMAL).expect("parses")
    }

    #[test]
    fn the_counts_a_reader_would_check_by_hand() {
        let config = minimal();
        assert_eq!(config.version, 2);
        assert_eq!(config.space_order.len(), 4);
        assert_eq!(config.displays.len(), 1);
        assert_eq!(config.display("sRGB").map(|d| d.views.len()), Some(2));
        assert_eq!(config.role("scene_linear"), Some("linear"));
    }

    #[test]
    fn an_inactive_space_is_hidden_from_lists_but_still_there() {
        let config = minimal();
        assert!(!config.active_space_names().contains(&"hidden"));
        assert!(config.spaces.contains_key("hidden"));
    }

    #[test]
    fn the_search_path_keeps_its_order() {
        assert_eq!(minimal().search_paths, vec!["luts", "more_luts"]);
    }

    #[test]
    fn a_config_with_no_search_path_looks_beside_itself() {
        let config = Config::parse(Path::new("."), "ocio_profile_version: 1\n").expect("parses");
        assert_eq!(config.search_paths, vec!["."]);
    }

    #[test]
    fn a_data_space_says_so() {
        assert!(minimal().spaces.get("hidden").map(|s| s.is_data) == Some(true));
    }

    #[test]
    fn an_allocation_is_read_with_its_offset() {
        let allocation = minimal().spaces.get("linear").and_then(|s| s.allocation);
        assert!(
            matches!(
                allocation,
                Some(Allocation::Lg2 {
                    min: -8.0,
                    max: 5.0,
                    ..
                })
            ),
            "{allocation:?}"
        );
    }

    #[test]
    fn anchors_and_merge_keys_are_resolved() {
        let text = r#"
ocio_profile_version: 1
colorspaces:
  - !<ColorSpace>
    name: base
    family: &shared_family utility
    to_reference: !<MatrixTransform> {matrix: [2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1]}
  - !<ColorSpace>
    name: copy
    family: *shared_family
"#;
        let config = Config::parse(Path::new("."), text).expect("parses");
        assert_eq!(
            config.spaces.get("copy").map(|s| s.family.as_str()),
            Some("utility")
        );
    }

    #[test]
    fn a_merge_key_fills_in_what_a_mapping_does_not_state() {
        let text = r#"
ocio_profile_version: 1
defaults: &defaults
  family: shared
  isdata: true
colorspaces:
  - !<ColorSpace>
    <<: *defaults
    name: merged
"#;
        let config = Config::parse(Path::new("."), text).expect("parses");
        let space = config.spaces.get("merged").expect("a merged space");
        assert_eq!(space.family, "shared");
        assert!(space.is_data);
    }

    #[test]
    fn a_profile_version_lumit_does_not_read_refuses_by_name() {
        let err = Config::parse(Path::new("."), "ocio_profile_version: 3\n");
        assert!(
            matches!(&err, Err(ColourError::UnsupportedConfigVersion { version }) if version == "3"),
            "{err:?}"
        );
    }

    #[test]
    fn a_transform_outside_the_op_set_is_kept_by_name_for_the_resolve() {
        let text = r#"
ocio_profile_version: 2
colorspaces:
  - !<ColorSpace>
    name: fancy
    to_scene_reference: !<FixedFunctionTransform> {style: ACES_RedMod03}
  - !<ColorSpace>
    name: plain
"#;
        let config = Config::parse(Path::new("."), text).expect("the file itself is fine");
        assert!(
            matches!(&config.spaces["fancy"].to_reference, Some(TransformSpec::Unsupported { name }) if name == "FixedFunctionTransform")
        );
        assert!(config.spaces.contains_key("plain"));
    }

    #[test]
    fn a_context_variable_in_a_path_refuses_by_name() {
        let text = r#"
ocio_profile_version: 2
colorspaces:
  - !<ColorSpace>
    name: shot
    to_scene_reference: !<FileTransform> {src: $SHOT/grade.spi1d}
"#;
        let config = Config::parse(Path::new("."), text).expect("parses");
        let err = config.resolve_lut_path("$SHOT/grade.spi1d");
        assert!(
            matches!(&err, Err(ColourError::ContextVariable { path }) if path.contains("$SHOT")),
            "{err:?}"
        );
    }

    #[test]
    fn a_group_transform_keeps_its_children_in_order() {
        let text = r#"
ocio_profile_version: 1
colorspaces:
  - !<ColorSpace>
    name: grouped
    to_reference: !<GroupTransform>
      children:
        - !<LogTransform> {base: 10}
        - !<MatrixTransform> {matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]}
"#;
        let config = Config::parse(Path::new("."), text).expect("parses");
        let Some(TransformSpec::Group(children, _)) = config
            .spaces
            .get("grouped")
            .and_then(|s| s.to_reference.clone())
        else {
            panic!("expected a group");
        };
        assert_eq!(children.len(), 2);
        assert!(matches!(children.first(), Some(TransformSpec::Log { .. })));
    }

    #[test]
    fn an_allocation_transform_is_the_log_or_the_fit_it_stands_for() {
        let text = r#"
ocio_profile_version: 1
colorspaces:
  - !<ColorSpace>
    name: logged
    from_reference: !<AllocationTransform> {allocation: lg2, vars: [-8, 5, 0.25]}
  - !<ColorSpace>
    name: fitted
    from_reference: !<AllocationTransform> {allocation: uniform, vars: [0, 0.5], direction: inverse}
"#;
        let config = Config::parse(Path::new("."), text).expect("parses");
        let spec = |name: &str| {
            config.spaces[name]
                .from_reference
                .clone()
                .expect("declared")
        };
        // lg2 [-8, 5] with an offset: log2(x + 0.25), then (y + 8) / 13.
        match spec("logged") {
            TransformSpec::Log { params, dir } => {
                assert_eq!(params.base, 2.0);
                assert_eq!(params.lin_side_offset, [0.25; 3]);
                assert_eq!(params.log_side_slope, [1.0 / 13.0; 3]);
                assert_eq!(params.log_side_offset, [8.0 / 13.0; 3]);
                assert_eq!(dir, Direction::Forward);
            }
            other => panic!("expected a log, got {other:?}"),
        }
        // uniform [0, 0.5] is a scale by 2, and the direction travels with it.
        match spec("fitted") {
            TransformSpec::Matrix {
                matrix,
                offset,
                dir,
            } => {
                assert_eq!(
                    [matrix[0], matrix[5], matrix[10], matrix[15]],
                    [2.0, 2.0, 2.0, 1.0]
                );
                assert_eq!(offset, [0.0; 4]);
                assert_eq!(dir, Direction::Inverse);
            }
            other => panic!("expected a matrix, got {other:?}"),
        }
        // An allocation Lumit has not heard of is kept by name for the
        // resolve to refuse with.
        let odd = Config::parse(
            Path::new("."),
            "ocio_profile_version: 1\ncolorspaces:\n  - !<ColorSpace>\n    name: odd\n    to_reference: !<AllocationTransform> {allocation: lg10, vars: [0, 1]}\n",
        )
        .expect("parses");
        assert!(
            matches!(&odd.spaces["odd"].to_reference, Some(TransformSpec::Unsupported { name }) if name == "AllocationTransform with allocation lg10")
        );
    }

    #[test]
    fn a_log_camera_transform_may_spell_its_keys_snake_case() {
        // Blender's own config, one camera log space, as it writes it.
        let text = r#"
ocio_profile_version: 2
colorspaces:
  - !<ColorSpace>
    name: camera
    to_scene_reference: !<LogCameraTransform> {base: 2.71828182845905, log_side_slope: 0.0869287606549122, log_side_offset: 0.530013339229194, lin_side_offset: 0.00549407243225781, lin_side_break: 0.005, direction: inverse}
"#;
        let config = Config::parse(Path::new("."), text).expect("parses");
        let Some(TransformSpec::Log { params, dir }) = config.spaces["camera"].to_reference else {
            panic!("expected a log");
        };
        assert!((params.log_side_slope[0] - 0.086_928_76).abs() < 1e-7);
        assert_eq!(params.lin_side_slope, [1.0; 3]);
        assert!((params.lin_side_offset[0] - 0.005_494_072_4).abs() < 1e-8);
        assert_eq!(params.lin_side_break, Some([0.005; 3]));
        assert_eq!(params.linear_slope, None);
        assert_eq!(dir, Direction::Inverse);
    }

    /// The two grading transforms exactly as Blender's and PixelManager's
    /// configs write them: block style, an unstated key meaning the style's own
    /// default, and a tone band whose two extra numbers are spelled per band.
    #[test]
    fn the_grading_transforms_read_the_keys_the_real_configs_write() {
        let text = r#"
ocio_profile_version: 2
colorspaces:
  - !<ColorSpace>
    name: agx
    from_reference: !<GradingPrimaryTransform>
      style: log
      contrast: {rgb: [1.4, 1.4, 1.4], master: 1}
      saturation: 0.95
      pivot: {contrast: -0.2}
  - !<ColorSpace>
    name: punchy
    from_reference: !<GradingToneTransform>
     shadows: {rgb: [0.2, 0.2, 0.2], master: 0.35, start: 0.4, pivot: 0.1}
  - !<ColorSpace>
    name: lin
    from_reference: !<GradingToneTransform>
      style: linear
      midtones: {rgb: [1.2, 1, 1], master: 1.1, center: 0.5, width: 7}
"#;
        let config = Config::parse(Path::new("."), text).expect("parses");
        let Some(TransformSpec::GradingPrimary { params, dir }) =
            config.spaces["agx"].from_reference.clone()
        else {
            panic!("expected a primary grade");
        };
        assert_eq!(params.contrast.rgb, [1.4; 3]);
        assert_eq!(params.saturation, 0.95);
        assert_eq!(params.pivot, -0.2);
        // Unstated keys are the style's own defaults, not zero.
        assert_eq!(params.gamma.master, 1.0);
        assert_eq!(params.pivot_white, 1.0);
        assert_eq!(dir, Direction::Forward);

        let Some(TransformSpec::GradingTone { params, .. }) =
            config.spaces["punchy"].from_reference.clone()
        else {
            panic!("expected a tone grade");
        };
        // `shadows` spells its width `pivot`, and the bands nobody stated keep
        // the log style's own places.
        assert_eq!(params.values.shadows.width, 0.1);
        assert_eq!(params.values.shadows.start, 0.4);
        assert_eq!(params.values.midtones.width, 0.6);
        assert_eq!(params.values.scontrast, 1.0);

        let Some(TransformSpec::GradingTone { params, .. }) =
            config.spaces["lin"].from_reference.clone()
        else {
            panic!("expected a tone grade");
        };
        // `midtones` spells its start `center`, and the linear style's bands sit
        // in stops rather than in code values.
        assert_eq!(params.values.midtones.start, 0.5);
        assert_eq!(params.values.highlights.width, 9.0);
    }

    #[test]
    fn a_grading_transform_refuses_a_style_and_a_half_written_value_by_name() {
        let bad_style = Config::parse(
            Path::new("."),
            "ocio_profile_version: 2\ncolorspaces:\n  - !<ColorSpace>\n    name: odd\n    to_reference: !<GradingPrimaryTransform> {style: filmlike}\n",
        );
        let bad_style = bad_style.expect("the file itself parses");
        assert!(
            matches!(&bad_style.spaces["odd"].to_reference, Some(TransformSpec::Unsupported { name }) if name == "GradingPrimaryTransform with style filmlike"),
            "{:?}",
            bad_style.spaces["odd"].to_reference
        );
        let half_written = Config::parse(
            Path::new("."),
            "ocio_profile_version: 2\ncolorspaces:\n  - !<ColorSpace>\n    name: odd\n    to_reference: !<GradingPrimaryTransform> {style: log, contrast: {rgb: [1.2, 1.2, 1.2]}}\n",
        );
        assert!(
            matches!(&half_written, Err(ColourError::Parse { reason, .. }) if reason.contains("master")),
            "{half_written:?}"
        );
    }

    #[test]
    fn broken_yaml_is_a_typed_error_not_a_panic() {
        assert!(Config::parse(Path::new("."), "colorspaces: [").is_err());
        assert!(Config::parse(Path::new("."), "").is_err());
    }
}
