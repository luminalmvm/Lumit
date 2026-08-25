//! CLF and CTF — the Academy's Common LUT Format.
//!
//! In plain terms: `.clf` (and `.ctf`, the same grammar with vendor extras) is
//! an XML file holding a *list* of steps rather than a single table — a matrix,
//! then a curve, then a cube. It is the format the ACES organisation publishes
//! its own transforms in, so reading it is what lets a config point at one.
//!
//! Two traps are worth naming once, loudly, because both look like a grading
//! mistake rather than a parsing one:
//!
//! - **Bit depths scale the numbers.** Every node declares what scale its input
//!   and output are on: a `10i` node's values run 0–1023, not 0–1. Ignoring
//!   that makes a picture a thousand times too bright. Every node here is
//!   normalised to 0–1 as it is read.
//! - **CLF stores a cube blue-fastest**, the opposite of the `.cube` and
//!   `.spi3d` files everything else in Lumit uses. A parser that copies the
//!   numbers in file order transposes red and blue — the single most common LUT
//!   bug, and one that produces a picture that looks *plausibly* graded.
//!
//! Everything CLF can say that Lumit's op set cannot is refused by name
//! (docs/impl/ocio.md §4.3): `rawHalfs`, `halfDomain`, the mirrored and
//! pass-through exponent styles, external `Reference` nodes, and integer bit
//! depths on the nodes whose maths CLF defines on normalised values.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

use crate::error::{ColourError, Result};
use crate::matrix::Matrix34;
use crate::op::{CdlParams, Chain, Direction, LogParams, Op, RangeParams};
use crate::sample::{Cube, Curve};

fn bad(what: &str, reason: impl Into<String>) -> ColourError {
    ColourError::Parse {
        what: what.to_string(),
        reason: reason.into(),
    }
}

/// The scale a bit depth puts its numbers on, or a refusal naming the depth.
fn depth_scale(what: &str, depth: &str) -> Result<f32> {
    Ok(match depth {
        "8i" => 255.0,
        "10i" => 1023.0,
        "12i" => 4095.0,
        "16i" => 65535.0,
        "16f" | "32f" => 1.0,
        other => {
            return Err(bad(
                what,
                format!("the bit depth {other:?} is not one CLF defines"),
            ))
        }
    })
}

/// One process node as it was read, before it becomes an [`Op`].
#[derive(Default)]
struct Node {
    kind: String,
    attrs: BTreeMap<String, String>,
    /// Child elements with attributes: `ExponentParams`, `LogParams`, `Array`.
    /// Kept in file order because per-channel params repeat the element.
    children: Vec<(String, BTreeMap<String, String>)>,
    /// Leaf elements with text: `minInValue`, `Slope`, `Saturation`, …
    texts: BTreeMap<String, String>,
    array: Vec<f32>,
    array_dim: Vec<usize>,
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }

    fn style(&self) -> &str {
        self.attr("style").unwrap_or_default()
    }

    fn scales(&self, what: &str) -> Result<(f32, f32)> {
        let in_depth = self.attr("inBitDepth").unwrap_or("32f");
        let out_depth = self.attr("outBitDepth").unwrap_or("32f");
        Ok((depth_scale(what, in_depth)?, depth_scale(what, out_depth)?))
    }

    /// Nodes whose maths CLF defines on normalised values only.
    fn require_float_depths(&self, what: &str) -> Result<()> {
        let (i, o) = self.scales(what)?;
        if i == 1.0 && o == 1.0 {
            Ok(())
        } else {
            Err(ColourError::UnsupportedClfFeature {
                feature: format!("integer bit depths on a {} node", self.kind),
            })
        }
    }

    /// Three per-channel numbers from a whitespace-separated text element.
    fn triple(&self, what: &str, key: &str, default: [f32; 3]) -> Result<[f32; 3]> {
        let Some(text) = self.texts.get(key) else {
            return Ok(default);
        };
        let values: Vec<f32> = text
            .split_whitespace()
            .filter_map(|t| t.parse::<f32>().ok())
            .collect();
        match values.as_slice() {
            [v] => Ok([*v; 3]),
            [r, g, b] => Ok([*r, *g, *b]),
            _ => Err(bad(what, format!("{key} needs one or three numbers"))),
        }
    }

    fn scalar(&self, key: &str, default: f32) -> f32 {
        self.texts
            .get(key)
            .and_then(|t| t.trim().parse::<f32>().ok())
            .unwrap_or(default)
    }
}

/// Read a per-channel parameter set (`ExponentParams`, `LogParams`), honouring
/// the optional `channel="R|G|B"` attribute that lets a file state three.
fn per_channel(
    children: &[(String, BTreeMap<String, String>)],
    element: &str,
    key: &str,
    default: f32,
) -> [f32; 3] {
    let mut out = [default; 3];
    for (name, attrs) in children {
        if name != element {
            continue;
        }
        let Some(value) = attrs.get(key).and_then(|v| v.trim().parse::<f32>().ok()) else {
            continue;
        };
        match attrs.get("channel").map(String::as_str) {
            Some("R") => out[0] = value,
            Some("G") => out[1] = value,
            Some("B") => out[2] = value,
            _ => out = [value; 3],
        }
    }
    out
}

/// Whether any of the named parameter elements states `key` at all.
fn states(children: &[(String, BTreeMap<String, String>)], element: &str, key: &str) -> bool {
    children
        .iter()
        .any(|(name, attrs)| name == element && attrs.contains_key(key))
}

fn matrix_op(what: &str, node: &Node) -> Result<Op> {
    let (in_scale, out_scale) = node.scales(what)?;
    let dim = &node.array_dim;
    let (rows, cols) = match dim.as_slice() {
        // CLF 3 writes "3 3" / "3 4"; earlier writers append the channel count.
        [r, c] | [r, c, _] => (*r, *c),
        _ => {
            return Err(bad(
                what,
                "a Matrix node needs an Array with a 'dim' of rows and columns",
            ))
        }
    };
    if rows != 3 || !(cols == 3 || cols == 4) || node.array.len() != rows * cols {
        return Err(bad(
            what,
            format!(
                "Lumit reads 3×3 and 3×4 matrices; this one is {rows}×{cols} with {} values",
                node.array.len()
            ),
        ));
    }
    let mut m: Matrix34 = crate::matrix::IDENTITY;
    let scale = in_scale / out_scale;
    for row in 0..3 {
        for col in 0..3 {
            m[row * 4 + col] = node.array.get(row * cols + col).copied().unwrap_or(0.0) * scale;
        }
        m[row * 4 + 3] = if cols == 4 {
            node.array.get(row * cols + 3).copied().unwrap_or(0.0) / out_scale
        } else {
            0.0
        };
    }
    Ok(Op::Matrix(m))
}

fn lut1d_op(what: &str, node: &Node) -> Result<Op> {
    if node.attr("rawHalfs").is_some_and(|v| v == "true") {
        return Err(ColourError::UnsupportedClfFeature {
            feature: "a rawHalfs look-up table".to_string(),
        });
    }
    if node.attr("halfDomain").is_some_and(|v| v == "true") {
        return Err(ColourError::UnsupportedClfFeature {
            feature: "a halfDomain look-up table".to_string(),
        });
    }
    let (_, out_scale) = node.scales(what)?;
    let (length, channels) = match node.array_dim.as_slice() {
        [n, c] => (*n, *c),
        _ => {
            return Err(bad(
                what,
                "a LUT1D node needs an Array with a 'dim' of length and channels",
            ))
        }
    };
    if !(channels == 1 || channels == 3) || node.array.len() != length * channels {
        return Err(bad(
            what,
            format!(
                "a LUT1D of {length}×{channels} does not match its {} values",
                node.array.len()
            ),
        ));
    }
    let mut data = Vec::with_capacity(length);
    for i in 0..length {
        let at = |c: usize| node.array.get(i * channels + c).copied().unwrap_or(0.0) / out_scale;
        data.push(if channels == 1 {
            [at(0); 3]
        } else {
            [at(0), at(1), at(2)]
        });
    }
    Ok(Op::Lut1d {
        curve: Curve::new(what, [0.0, 1.0], data)?,
        dir: Direction::Forward,
    })
}

fn lut3d_op(what: &str, node: &Node) -> Result<Op> {
    let (_, out_scale) = node.scales(what)?;
    let (n, channels) = match node.array_dim.as_slice() {
        [r, g, b, c] => {
            if r != g || g != b {
                return Err(bad(
                    what,
                    format!("Lumit reads cubes with equal sides; this one is {r}×{g}×{b}"),
                ));
            }
            (*r, *c)
        }
        _ => {
            return Err(bad(
                what,
                "a LUT3D node needs an Array with a 'dim' of three sides and a channel count",
            ))
        }
    };
    if channels != 3 || node.array.len() != n * n * n * 3 {
        return Err(bad(
            what,
            format!(
                "a LUT3D of {n}³×{channels} does not match its {} values",
                node.array.len()
            ),
        ));
    }
    // CLF walks red slowest and blue fastest; Lumit stores red fastest.
    let mut data = vec![[0.0_f32; 3]; n * n * n];
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                let from = ((r * n + g) * n + b) * 3;
                let to = r + g * n + b * n * n;
                if let Some(slot) = data.get_mut(to) {
                    *slot = [
                        node.array.get(from).copied().unwrap_or(0.0) / out_scale,
                        node.array.get(from + 1).copied().unwrap_or(0.0) / out_scale,
                        node.array.get(from + 2).copied().unwrap_or(0.0) / out_scale,
                    ];
                }
            }
        }
    }
    Ok(Op::Lut3d {
        cube: Cube::new(what, n, [0.0; 3], [1.0; 3], data)?,
    })
}

fn range_op(what: &str, node: &Node) -> Result<Op> {
    let (in_scale, out_scale) = node.scales(what)?;
    let read = |key: &str, scale: f32| -> Option<f32> {
        node.texts
            .get(key)
            .and_then(|t| t.trim().parse::<f32>().ok())
            .map(|v| v / scale)
    };
    Ok(Op::Range(RangeParams {
        min_in: read("minInValue", in_scale),
        max_in: read("maxInValue", in_scale),
        min_out: read("minOutValue", out_scale),
        max_out: read("maxOutValue", out_scale),
        no_clamp: node.style() == "noClamp",
    }))
}

fn exponent_op(what: &str, node: &Node) -> Result<Op> {
    node.require_float_depths(what)?;
    let exponent = per_channel(&node.children, "ExponentParams", "exponent", 1.0);
    let offset = per_channel(&node.children, "ExponentParams", "offset", 0.0);
    Ok(match node.style() {
        "basicFwd" => Op::Exponent {
            exp: exponent,
            dir: Direction::Forward,
        },
        "basicRev" => Op::Exponent {
            exp: exponent,
            dir: Direction::Inverse,
        },
        "monCurveFwd" => Op::MonCurve {
            gamma: exponent,
            offset,
            dir: Direction::Forward,
        },
        "monCurveRev" => Op::MonCurve {
            gamma: exponent,
            offset,
            dir: Direction::Inverse,
        },
        other => {
            return Err(ColourError::UnsupportedClfFeature {
                feature: format!("the {other} exponent style"),
            })
        }
    })
}

fn log_op(what: &str, node: &Node) -> Result<Op> {
    node.require_float_depths(what)?;
    let style = node.style();
    let plain = |base: f32, dir: Direction| Op::Log {
        params: LogParams::plain(base),
        dir,
    };
    let parametric = |dir: Direction, camera: bool| -> Op {
        let base = per_channel(&node.children, "LogParams", "base", 2.0);
        let params = LogParams {
            base: base[0],
            lin_side_slope: per_channel(&node.children, "LogParams", "linSideSlope", 1.0),
            lin_side_offset: per_channel(&node.children, "LogParams", "linSideOffset", 0.0),
            log_side_slope: per_channel(&node.children, "LogParams", "logSideSlope", 1.0),
            log_side_offset: per_channel(&node.children, "LogParams", "logSideOffset", 0.0),
            lin_side_break: camera
                .then(|| per_channel(&node.children, "LogParams", "linSideBreak", 0.0)),
            linear_slope: states(&node.children, "LogParams", "linearSlope")
                .then(|| per_channel(&node.children, "LogParams", "linearSlope", 1.0)),
        };
        Op::Log { params, dir }
    };
    Ok(match style {
        "log10" => plain(10.0, Direction::Forward),
        "log2" => plain(2.0, Direction::Forward),
        "antiLog10" => plain(10.0, Direction::Inverse),
        "antiLog2" => plain(2.0, Direction::Inverse),
        "linToLog" => parametric(Direction::Forward, false),
        "logToLin" => parametric(Direction::Inverse, false),
        "cameraLinToLog" => parametric(Direction::Forward, true),
        "cameraLogToLin" => parametric(Direction::Inverse, true),
        other => {
            return Err(ColourError::UnsupportedClfFeature {
                feature: format!("the {other} log style"),
            })
        }
    })
}

fn cdl_op(what: &str, node: &Node) -> Result<Op> {
    node.require_float_depths(what)?;
    let style = node.style();
    let (dir, clamp) = match style {
        "Fwd" | "" => (Direction::Forward, true),
        "Rev" => (Direction::Inverse, true),
        "FwdNoClamp" => (Direction::Forward, false),
        "RevNoClamp" => (Direction::Inverse, false),
        other => {
            return Err(ColourError::UnsupportedClfFeature {
                feature: format!("the {other} CDL style"),
            })
        }
    };
    Ok(Op::Cdl {
        params: CdlParams {
            slope: node.triple(what, "Slope", [1.0; 3])?,
            offset: node.triple(what, "Offset", [0.0; 3])?,
            power: node.triple(what, "Power", [1.0; 3])?,
            saturation: node.scalar("Saturation", 1.0),
            clamp,
        },
        dir,
    })
}

/// The elements this reader treats as process nodes.
const PROCESS_NODES: [&str; 7] = [
    "Matrix", "LUT1D", "LUT3D", "Range", "Log", "Exponent", "ASC_CDL",
];

/// Elements that carry information rather than maths, and are skipped.
const IGNORED: [&str; 6] = [
    "ProcessList",
    "Description",
    "InputDescriptor",
    "OutputDescriptor",
    "Info",
    "ACEStransformID",
];

/// Parse CLF or CTF text into a resolved chain.
pub fn parse_clf(what: &str, text: &str) -> Result<Chain> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);

    let mut ops: Vec<Op> = Vec::new();
    let mut node: Option<Node> = None;
    let mut open_child: Option<String> = None;
    let mut child_text = String::new();

    loop {
        let event = reader
            .read_event()
            .map_err(|e| bad(what, format!("the XML could not be read ({e})")))?;
        match event {
            Event::Eof => {
                if let Some(open) = node.as_ref() {
                    return Err(bad(
                        what,
                        format!("the file ends inside a {} node", open.kind),
                    ));
                }
                break;
            }
            Event::Start(ref e) | Event::Empty(ref e) => {
                let elem = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                // An empty element carries no text and closes on the spot.
                let empty = matches!(event, Event::Empty(_));
                let mut attrs = BTreeMap::new();
                for attr in e.attributes() {
                    let attr = attr.map_err(|err| {
                        bad(what, format!("an attribute could not be read ({err})"))
                    })?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    let value = attr
                        .unescape_value()
                        .map_err(|err| {
                            bad(what, format!("an attribute could not be read ({err})"))
                        })?
                        .into_owned();
                    attrs.insert(key, value);
                }

                if elem == "Reference" {
                    return Err(ColourError::UnsupportedClfNode {
                        node: "Reference".to_string(),
                    });
                }

                if let Some(open) = node.as_mut() {
                    if elem == "Array" {
                        open.array_dim = attrs
                            .get("dim")
                            .map(|d| {
                                d.split_whitespace()
                                    .filter_map(|t| t.parse::<usize>().ok())
                                    .collect()
                            })
                            .unwrap_or_default();
                    }
                    open.children.push((elem.clone(), attrs));
                    open_child = if empty { None } else { Some(elem) };
                    child_text.clear();
                } else if PROCESS_NODES.contains(&elem.as_str()) {
                    let started = Node {
                        kind: elem,
                        attrs,
                        ..Node::default()
                    };
                    if empty {
                        ops.push(node_to_op(what, &started)?);
                    } else {
                        node = Some(started);
                    }
                    open_child = None;
                    child_text.clear();
                } else if !IGNORED.contains(&elem.as_str()) {
                    return Err(ColourError::UnsupportedClfNode { node: elem });
                }
            }
            Event::Text(ref t) => {
                if open_child.is_some() {
                    let chunk = t
                        .xml_content()
                        .map_err(|e| bad(what, format!("text could not be read ({e})")))?;
                    child_text.push_str(chunk.as_ref());
                }
            }
            Event::End(ref e) => {
                let elem = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if node.as_ref().is_some_and(|n| n.kind == elem) {
                    if let Some(finished) = node.take() {
                        ops.push(node_to_op(what, &finished)?);
                    }
                    open_child = None;
                    child_text.clear();
                } else if let Some(open) = node.as_mut() {
                    if open_child.as_deref() == Some(elem.as_str()) {
                        if elem == "Array" {
                            open.array = child_text
                                .split_whitespace()
                                .filter_map(|t| t.parse::<f32>().ok())
                                .collect();
                        } else if !child_text.trim().is_empty() {
                            open.texts.insert(elem, child_text.trim().to_string());
                        }
                        open_child = None;
                        child_text.clear();
                    }
                }
            }
            _ => {}
        }
    }

    Ok(Chain::new(ops))
}

fn node_to_op(what: &str, node: &Node) -> Result<Op> {
    match node.kind.as_str() {
        "Matrix" => matrix_op(what, node),
        "LUT1D" => lut1d_op(what, node),
        "LUT3D" => lut3d_op(what, node),
        "Range" => range_op(what, node),
        "Log" => log_op(what, node),
        "Exponent" => exponent_op(what, node),
        "ASC_CDL" => cdl_op(what, node),
        other => Err(ColourError::UnsupportedClfNode {
            node: other.to_string(),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn a_float_matrix_reads_straight_through() {
        let text = r#"<ProcessList compCLFversion="3" id="t">
          <Matrix inBitDepth="32f" outBitDepth="32f">
            <Array dim="3 3 3">0 1 0  1 0 0  0 0 1</Array>
          </Matrix>
        </ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        assert_eq!(chain.ops.len(), 1);
        assert!(close(chain.eval([0.2, 0.5, 0.8]), [0.5, 0.2, 0.8], 1e-6));
    }

    #[test]
    fn an_integer_matrix_is_rescaled_rather_than_read_a_thousand_times_too_bright() {
        // 10-bit in, float out: the identity in CLF's units is the identity
        // in ours only after the 1023 comes out.
        let text = r#"<ProcessList id="t">
          <Matrix inBitDepth="10i" outBitDepth="32f">
            <Array dim="3 4 3">1 0 0 0  0 1 0 0  0 0 1 0</Array>
          </Matrix>
        </ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        let got = chain.eval([1.0, 1.0, 1.0]);
        assert!(close(got, [1023.0; 3], 1e-2), "{got:?}");
    }

    #[test]
    fn a_cube_is_transposed_out_of_clfs_blue_fastest_order() {
        // A 2³ cube whose samples encode their own red index. Read in file
        // order it would encode blue instead, which is the classic silent bug.
        let mut values = String::new();
        for r in 0..2 {
            for g in 0..2 {
                for b in 0..2 {
                    let _ = (g, b);
                    values.push_str(&format!("{}.0 0.0 0.0 ", r));
                }
            }
        }
        let text = format!(
            r#"<ProcessList id="t"><LUT3D inBitDepth="32f" outBitDepth="32f">
            <Array dim="2 2 2 3">{values}</Array></LUT3D></ProcessList>"#
        );
        let chain = parse_clf("t.clf", &text).expect("parses");
        // Full red must read 1; full blue must read 0.
        assert!(close(chain.eval([1.0, 0.0, 0.0]), [1.0, 0.0, 0.0], 1e-6));
        assert!(close(chain.eval([0.0, 0.0, 1.0]), [0.0, 0.0, 0.0], 1e-6));
    }

    #[test]
    fn a_1d_table_normalises_its_output_scale() {
        let text = r#"<ProcessList id="t"><LUT1D inBitDepth="32f" outBitDepth="10i">
          <Array dim="2 1">0 1023</Array></LUT1D></ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        assert!(close(chain.eval([0.5; 3]), [0.5; 3], 1e-4));
    }

    #[test]
    fn a_moncurve_exponent_node_reads_its_parameters() {
        let text = r#"<ProcessList id="t"><Exponent inBitDepth="32f" outBitDepth="32f" style="monCurveFwd">
          <ExponentParams exponent="2.4" offset="0.055"/></Exponent></ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        assert!(close(chain.eval([1.0; 3]), [1.0; 3], 1e-5));
        assert!(
            close(chain.eval([0.5; 3]), [0.2140; 3], 1e-3),
            "{:?}",
            chain.eval([0.5; 3])
        );
    }

    #[test]
    fn a_per_channel_exponent_states_three() {
        let text = r#"<ProcessList id="t"><Exponent inBitDepth="32f" outBitDepth="32f" style="basicFwd">
          <ExponentParams exponent="2.0" channel="R"/>
          <ExponentParams exponent="3.0" channel="G"/>
          <ExponentParams exponent="1.0" channel="B"/></Exponent></ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        assert!(close(chain.eval([0.5; 3]), [0.25, 0.125, 0.5], 1e-6));
    }

    #[test]
    fn a_camera_log_node_reads_its_break() {
        let text = r#"<ProcessList id="t"><Log inBitDepth="32f" outBitDepth="32f" style="cameraLinToLog">
          <LogParams base="2" logSideSlope="0.0570776" logSideOffset="0.5547945"
                     linSideSlope="1" linSideOffset="0" linSideBreak="0.0078125"
                     linearSlope="10.5402377"/></Log></ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        let at_break = chain.eval([0.0078125; 3]);
        assert!((at_break[0] - 0.155251).abs() < 1e-3, "{at_break:?}");
    }

    #[test]
    fn a_range_node_scales_both_ends_by_their_own_depth() {
        let text = r#"<ProcessList id="t"><Range inBitDepth="10i" outBitDepth="32f">
          <minInValue>64</minInValue><maxInValue>940</maxInValue>
          <minOutValue>0</minOutValue><maxOutValue>1</maxOutValue></Range></ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        // 64/1023 in maps to 0 out, 940/1023 maps to 1.
        assert!(close(chain.eval([64.0 / 1023.0; 3]), [0.0; 3], 1e-5));
        assert!(close(chain.eval([940.0 / 1023.0; 3]), [1.0; 3], 1e-5));
    }

    #[test]
    fn a_cdl_node_reads_its_sop_and_sat() {
        let text = r#"<ProcessList id="t"><ASC_CDL inBitDepth="32f" outBitDepth="32f" style="Fwd">
          <SOPNode><Slope>1.1 1.0 0.9</Slope><Offset>0 0 0</Offset><Power>1 1 1</Power></SOPNode>
          <SatNode><Saturation>1.0</Saturation></SatNode></ASC_CDL></ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        assert!(close(chain.eval([0.5; 3]), [0.55, 0.5, 0.45], 1e-5));
    }

    #[test]
    fn several_nodes_run_in_file_order() {
        let text = r#"<ProcessList id="t">
          <Matrix inBitDepth="32f" outBitDepth="32f"><Array dim="3 3 3">2 0 0 0 2 0 0 0 2</Array></Matrix>
          <Range inBitDepth="32f" outBitDepth="32f"><minOutValue>0</minOutValue><maxOutValue>1</maxOutValue></Range>
        </ProcessList>"#;
        let chain = parse_clf("t.clf", text).expect("parses");
        assert_eq!(chain.ops.len(), 2);
        assert!(
            close(chain.eval([0.6; 3]), [1.0; 3], 1e-6),
            "the range clamped after the matrix"
        );
    }

    #[test]
    fn raw_halfs_and_half_domain_refuse_by_name() {
        for feature in ["rawHalfs", "halfDomain"] {
            let text = format!(
                r#"<ProcessList id="t"><LUT1D inBitDepth="32f" outBitDepth="32f" {feature}="true">
                <Array dim="2 1">0 1</Array></LUT1D></ProcessList>"#
            );
            let err = parse_clf("t.clf", &text);
            assert!(
                matches!(&err, Err(ColourError::UnsupportedClfFeature { feature: f }) if f.contains(feature)),
                "{feature}: {err:?}"
            );
        }
    }

    #[test]
    fn an_unimplemented_process_node_refuses_by_name() {
        let text = r#"<ProcessList id="t"><FixedFunction inBitDepth="32f" outBitDepth="32f" style="ACES_RedMod03"/></ProcessList>"#;
        let err = parse_clf("t.clf", text);
        assert!(
            matches!(&err, Err(ColourError::UnsupportedClfNode { node }) if node == "FixedFunction"),
            "{err:?}"
        );
    }

    #[test]
    fn an_external_reference_refuses_by_name() {
        let text = r#"<ProcessList id="t"><Reference path="other.clf"/></ProcessList>"#;
        assert!(matches!(
            parse_clf("t.clf", text),
            Err(ColourError::UnsupportedClfNode { .. })
        ));
    }

    #[test]
    fn a_mirrored_exponent_style_refuses_by_name() {
        let text = r#"<ProcessList id="t"><Exponent inBitDepth="32f" outBitDepth="32f" style="basicMirrorFwd">
          <ExponentParams exponent="2.4"/></Exponent></ProcessList>"#;
        let err = parse_clf("t.clf", text);
        assert!(
            matches!(&err, Err(ColourError::UnsupportedClfFeature { feature }) if feature.contains("basicMirrorFwd")),
            "{err:?}"
        );
    }

    #[test]
    fn an_integer_depth_on_a_log_node_refuses_rather_than_guesses() {
        let text = r#"<ProcessList id="t"><Log inBitDepth="10i" outBitDepth="32f" style="log2"/></ProcessList>"#;
        assert!(matches!(
            parse_clf("t.clf", text),
            Err(ColourError::UnsupportedClfFeature { .. })
        ));
    }

    #[test]
    fn broken_xml_is_a_typed_error_not_a_panic() {
        assert!(parse_clf("t.clf", "<ProcessList><Matrix>").is_err());
        assert!(parse_clf("t.clf", "not xml at all <<<").is_err());
    }
}
