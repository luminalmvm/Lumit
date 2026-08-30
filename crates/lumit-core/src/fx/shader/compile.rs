//! The inner graph's compiler: a DAG of boxes into the WGSL a competent person
//! would have written (docs/impl/custom-shader.md §4.4, K-642, CS4).
//!
//! # In plain terms
//!
//! Every box becomes one `let` line, in a fixed order; the Parameter boxes
//! become the annotated `struct Params` the §1.4 reader already understands,
//! so a parameter declared by wiring a box and one declared by typing a doc
//! comment are one mechanism with two front doors. The output is ordinary
//! shader text — the same text pipeline (assembly, validation, caching, the
//! badge) takes it from there and never knows a graph existed.
//!
//! **The emitted text must be byte-identical for a given graph, on every
//! machine, for ever** (§4.4): the text's hash is the pipeline cache's key and
//! a term of the frame key, so an emission that varied between runs would miss
//! the cache and rename every frame. Order is therefore topological with ties
//! broken by node id, and nothing iterates a `HashMap`.
//!
//! No loops and no branches in v1: `mix`, `step`, `smoothstep` and `clamp`
//! cover what people reach a branch for, and every one is uniform-cost, which
//! keeps the effect's `cost = Heavy` declaration honest.

use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

use super::graph::{ports_of, spec_of, GraphTy, ShaderEdge, ShaderGraph, ShaderNode};

/// Why a graph will not compile (§4.4). These are edits this application made,
/// so they refuse — but the *badge* is how they reach the user: a stored graph
/// that will not compile degrades to a calm badge and the last good picture,
/// exactly as a text that will not parse does (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// No Result box, or more than one — a graph makes exactly one picture.
    NoResult,
    ManyResults,
    UnknownKind(String),
    DuplicateNode(u32),
    /// An edge names a node or a port that is not there.
    Dangling,
    /// Two wires land on one input.
    DoubleInput,
    Cycle,
    /// A wire carries a value its destination cannot take: two vectors of
    /// different widths, a picture into a number, a tap past a vector's width.
    TypeMismatch {
        node: u32,
        port: String,
    },
    /// A Parameter box (or a swizzle's pattern) with settings the grammar
    /// cannot carry.
    BadParameter(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::NoResult => write!(f, "the graph needs one Result box"),
            GraphError::ManyResults => write!(f, "a graph holds exactly one Result box"),
            GraphError::UnknownKind(k) => write!(f, "`{k}` is not a box this build knows"),
            GraphError::DuplicateNode(id) => write!(f, "two boxes share the id {id}"),
            GraphError::Dangling => write!(f, "a wire names a box or a port that is not there"),
            GraphError::DoubleInput => write!(f, "two wires land on one input"),
            GraphError::Cycle => write!(f, "the wires close a loop"),
            GraphError::TypeMismatch { node, port } => {
                write!(
                    f,
                    "the wire into box {node}'s `{port}` carries a value it cannot take"
                )
            }
            GraphError::BadParameter(why) => write!(f, "{why}"),
        }
    }
}

/// One value in flight during emission: a named number of some width, or a
/// picture — which is not a value at all but the name of the helper that
/// samples the texture it stands for.
#[derive(Clone)]
enum Val {
    Num(String, GraphTy),
    Pic(&'static str),
}

/// Compile one graph to the §1.3 contract's source text. Deterministic:
/// byte-identical for a given graph, for ever (the gate is
/// `a_graph_compiles_to_byte_identical_wgsl`).
///
/// # Errors
/// The [`GraphError`] taxonomy; every one is a sentence for the badge.
pub fn compile(graph: &ShaderGraph) -> Result<String, GraphError> {
    // -- The shape ----------------------------------------------------------
    let mut nodes: BTreeMap<u32, &ShaderNode> = BTreeMap::new();
    for n in &graph.nodes {
        if spec_of(&n.kind).is_none() {
            return Err(GraphError::UnknownKind(n.kind.clone()));
        }
        if nodes.insert(n.id, n).is_some() {
            return Err(GraphError::DuplicateNode(n.id));
        }
    }
    let results: Vec<u32> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "result")
        .map(|n| n.id)
        .collect();
    let result_id = match results.as_slice() {
        [] => return Err(GraphError::NoResult),
        [one] => *one,
        _ => return Err(GraphError::ManyResults),
    };

    // -- The wires ----------------------------------------------------------
    let mut into: BTreeMap<(u32, u32), &ShaderEdge> = BTreeMap::new();
    for e in &graph.edges {
        let (Some(from), Some(to)) = (nodes.get(&e.from), nodes.get(&e.to)) else {
            return Err(GraphError::Dangling);
        };
        let (_, from_outs) = ports_of(from);
        let (to_ins, _) = ports_of(to);
        if e.from_port as usize >= from_outs.len() || e.to_port as usize >= to_ins.len() {
            return Err(GraphError::Dangling);
        }
        if into.insert((e.to, e.to_port), e).is_some() {
            return Err(GraphError::DoubleInput);
        }
    }

    // -- Topological order, ties by node id (§4.4) --------------------------
    let mut indegree: BTreeMap<u32, u32> = nodes.keys().map(|id| (*id, 0)).collect();
    for e in into.values() {
        if let Some(d) = indegree.get_mut(&e.to) {
            *d += 1;
        }
    }
    // ponytail: an O(n²) ready-scan; a graph is tens of boxes. A heap earns
    // its place when someone wires hundreds.
    let mut order: Vec<u32> = Vec::with_capacity(nodes.len());
    let mut done: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    while order.len() < nodes.len() {
        let next = indegree
            .iter()
            .find(|(id, d)| **d == 0 && !done.contains(id))
            .map(|(id, _)| *id);
        let Some(id) = next else {
            return Err(GraphError::Cycle);
        };
        done.insert(id);
        order.push(id);
        for e in into.values() {
            if e.from == id {
                if let Some(d) = indegree.get_mut(&e.to) {
                    *d = d.saturating_sub(1);
                }
            }
        }
    }

    // -- Parameters (node id order — the struct is part of the contract) ----
    let params: Vec<&ShaderNode> = graph.nodes.iter().filter(|n| n.kind == "param").collect();
    let mut fields: Vec<(String, String)> = Vec::new(); // (id, struct lines)
    {
        let mut sorted = params.clone();
        sorted.sort_by_key(|n| n.id);
        for node in sorted {
            let (id, lines) = param_field(node)?;
            if fields.iter().any(|(had, _)| *had == id) {
                return Err(GraphError::BadParameter(format!(
                    "two parameters would both answer to `{id}`"
                )));
            }
            fields.push((id, lines));
        }
    }

    // -- Emission -----------------------------------------------------------
    let mut vals: BTreeMap<(u32, u32), Val> = BTreeMap::new();
    let mut lets: Vec<String> = Vec::new();
    let mut ret = String::from("vec4<f32>(0.0)");

    for id in &order {
        let node = nodes[id];
        let input = |port: u32, name: &'static str| -> Result<Val, GraphError> {
            match into.get(&(*id, port)) {
                Some(e) => vals.get(&(e.from, e.from_port)).cloned().ok_or_else(|| {
                    // The only gap a validated edge can hit: a tap past the
                    // wired vector's width (split's z on a vec2).
                    GraphError::TypeMismatch {
                        node: e.from,
                        port: name.to_owned(),
                    }
                }),
                None => Ok(default_input(&node.kind, port)),
            }
        };
        if node.kind == "result" {
            ret = coerce4(input(0, "colour")?, *id, "colour")?;
            continue;
        }
        let name = format!("n{}", lets.len());
        let (rhs, outs) = emit(node, &name, &input)?;
        lets.push(format!("    let {name} = {rhs};"));
        for (port, val) in outs {
            vals.insert((*id, port), val);
        }
    }
    debug_assert!(done.contains(&result_id));

    // -- The text -----------------------------------------------------------
    let mut out = String::new();
    if !fields.is_empty() {
        out.push_str("struct Params {\n");
        for (_, lines) in &fields {
            out.push_str(lines);
        }
        out.push_str("}\n\n");
    }
    out.push_str("fn shade(uv: vec2<f32>) -> vec4<f32> {\n");
    for line in &lets {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("    return ");
    out.push_str(&ret);
    out.push_str(";\n}\n");
    Ok(out)
}

/// What an unwired input reads (§4.4): nought, except where nought would be a
/// trap — a mix amount of one, a clamp ceiling of one, an unwired Sample
/// reading the effect's own picture at its own `uv`.
fn default_input(kind: &str, port: u32) -> Val {
    match (kind, port) {
        ("clamp", 2) | ("smoothstep", 1) | ("pow", 1) | ("blend", 2) | ("combine4", 3) => {
            Val::Num("1.0".into(), GraphTy::F32)
        }
        ("tint", 1) => Val::Num("vec4<f32>(1.0)".into(), GraphTy::Vec4),
        ("sample", 0) => Val::Pic("lumit_sample"),
        ("sample", 1) => Val::Num("uv".into(), GraphTy::Vec2),
        ("result" | "luminance" | "premultiply" | "unpremultiply" | "tint" | "blend", _) => {
            Val::Num("vec4<f32>(0.0)".into(), GraphTy::Vec4)
        }
        _ => Val::Num("0.0".into(), GraphTy::F32),
    }
}

/// A numeric value, or the mismatch a picture wired into a number is.
fn num(v: Val, node: u32, port: &str) -> Result<(String, GraphTy), GraphError> {
    match v {
        Val::Num(e, t) => Ok((e, t)),
        Val::Pic(_) => Err(GraphError::TypeMismatch {
            node,
            port: port.to_owned(),
        }),
    }
}

/// Splat a scalar up to `to`'s width; a value already that width passes.
fn widen(expr: String, from: GraphTy, to: GraphTy) -> String {
    if from == to || to.width() <= 1 {
        expr
    } else {
        format!("{}({expr})", to.wgsl())
    }
}

/// The width two wired values meet at: a scalar broadcasts to any width, two
/// vectors must match (§4.3).
fn join(a: GraphTy, b: GraphTy, node: u32, port: &str) -> Result<GraphTy, GraphError> {
    match (a.width(), b.width()) {
        (1, _) => Ok(b),
        (_, 1) => Ok(a),
        (x, y) if x == y => Ok(a),
        _ => Err(GraphError::TypeMismatch {
            node,
            port: port.to_owned(),
        }),
    }
}

/// A value as a vec4, for the colour sockets: a scalar splats, a vec4 passes,
/// and a vec2/vec3 is refused rather than padded with a guess.
fn coerce4(v: Val, node: u32, port: &str) -> Result<String, GraphError> {
    let (e, t) = num(v, node, port)?;
    match t.width() {
        1 => Ok(format!("vec4<f32>({e})")),
        4 => Ok(e),
        _ => Err(GraphError::TypeMismatch {
            node,
            port: port.to_owned(),
        }),
    }
}

type Outs = Vec<(u32, Val)>;

/// One box's `let` right-hand side and the values its output ports carry.
#[allow(clippy::too_many_lines)]
fn emit(
    node: &ShaderNode,
    name: &str,
    input: &dyn Fn(u32, &'static str) -> Result<Val, GraphError>,
) -> Result<(String, Outs), GraphError> {
    let id = node.id;
    let one = |ty: GraphTy| vec![(0, Val::Num(name.to_owned(), ty))];
    // A two-input op over the broadcast rule, spelt either as an operator or
    // as a call.
    let binary = |op: &str, call: bool| -> Result<(String, Outs), GraphError> {
        let (a, at) = num(input(0, "a")?, id, "a")?;
        let (b, bt) = num(input(1, "b")?, id, "b")?;
        let ty = join(at, bt, id, "b")?;
        let (a, b) = (widen(a, at, ty), widen(b, bt, ty));
        let rhs = if call {
            format!("{op}({a}, {b})")
        } else {
            format!("({a} {op} {b})")
        };
        Ok((rhs, one(ty)))
    };
    let unary = |call: &str| -> Result<(String, Outs), GraphError> {
        let (x, t) = num(input(0, "x")?, id, "x")?;
        Ok((format!("{call}({x})"), one(t)))
    };

    match node.kind.as_str() {
        // -- Input ----------------------------------------------------------
        "picture" => Ok((
            "lumit_sample(uv)".into(),
            vec![
                (0, Val::Num(name.to_owned(), GraphTy::Vec4)),
                (1, Val::Pic("lumit_sample")),
            ],
        )),
        "picture2" => Ok((
            "lumit_sample2(uv)".into(),
            vec![
                (0, Val::Num(name.to_owned(), GraphTy::Vec4)),
                (1, Val::Pic("lumit_sample2")),
            ],
        )),
        "matte" => Ok(("lumit_matte(uv)".into(), one(GraphTy::F32))),
        "uv" => Ok(("uv".into(), one(GraphTy::Vec2))),
        "time" => Ok(("lumit.time".into(), one(GraphTy::F32))),
        "seed" => Ok(("f32(lumit.seed)".into(), one(GraphTy::F32))),
        "param" => {
            let (field, _) = param_field(node)?;
            Ok((format!("p.{field}"), one(super::graph::param_ty(node))))
        }
        // -- Maths ----------------------------------------------------------
        "add" => binary("+", false),
        "subtract" => binary("-", false),
        "multiply" => binary("*", false),
        "divide" => binary("/", false),
        "modulo" => binary("%", false),
        "min" => binary("min", true),
        "max" => binary("max", true),
        "pow" => binary("pow", true),
        "atan2" => binary("atan2", true),
        "step" => binary("step", true),
        "distance" => {
            let (a, at) = num(input(0, "a")?, id, "a")?;
            let (b, bt) = num(input(1, "b")?, id, "b")?;
            let ty = join(at, bt, id, "b")?;
            Ok((
                format!("distance({}, {})", widen(a, at, ty), widen(b, bt, ty)),
                one(GraphTy::F32),
            ))
        }
        "dot" => {
            let (a, at) = num(input(0, "a")?, id, "a")?;
            let (b, bt) = num(input(1, "b")?, id, "b")?;
            if at.width() < 2 || at != bt {
                return Err(GraphError::TypeMismatch {
                    node: id,
                    port: "b".into(),
                });
            }
            Ok((format!("dot({a}, {b})"), one(GraphTy::F32)))
        }
        "length" => {
            let (x, _) = num(input(0, "x")?, id, "x")?;
            Ok((format!("length({x})"), one(GraphTy::F32)))
        }
        "normalize" => {
            let (x, t) = num(input(0, "x")?, id, "x")?;
            if t.width() < 2 {
                return Err(GraphError::TypeMismatch {
                    node: id,
                    port: "x".into(),
                });
            }
            Ok((format!("normalize({x})"), one(t)))
        }
        "mix" => {
            let (a, at) = num(input(0, "a")?, id, "a")?;
            let (b, bt) = num(input(1, "b")?, id, "b")?;
            let (t, tt) = num(input(2, "t")?, id, "t")?;
            let ty = join(at, bt, id, "b")?;
            let (a, b) = (widen(a, at, ty), widen(b, bt, ty));
            if tt.width() != 1 && tt != ty {
                return Err(GraphError::TypeMismatch {
                    node: id,
                    port: "t".into(),
                });
            }
            Ok((format!("mix({a}, {b}, {t})"), one(ty)))
        }
        "clamp" => {
            let (x, xt) = num(input(0, "x")?, id, "x")?;
            let (lo, lt) = num(input(1, "lo")?, id, "lo")?;
            let (hi, ht) = num(input(2, "hi")?, id, "hi")?;
            let ty = join(join(xt, lt, id, "lo")?, ht, id, "hi")?;
            Ok((
                format!(
                    "clamp({}, {}, {})",
                    widen(x, xt, ty),
                    widen(lo, lt, ty),
                    widen(hi, ht, ty)
                ),
                one(ty),
            ))
        }
        "smoothstep" => {
            let (lo, lt) = num(input(0, "lo")?, id, "lo")?;
            let (hi, ht) = num(input(1, "hi")?, id, "hi")?;
            let (x, xt) = num(input(2, "x")?, id, "x")?;
            let ty = join(join(xt, lt, id, "lo")?, ht, id, "hi")?;
            Ok((
                format!(
                    "smoothstep({}, {}, {})",
                    widen(lo, lt, ty),
                    widen(hi, ht, ty),
                    widen(x, xt, ty)
                ),
                one(ty),
            ))
        }
        "saturate" => unary("saturate"),
        "sqrt" => unary("sqrt"),
        "abs" => unary("abs"),
        "sign" => unary("sign"),
        "floor" => unary("floor"),
        "ceil" => unary("ceil"),
        "fract" => unary("fract"),
        "sin" => unary("sin"),
        "cos" => unary("cos"),
        // -- Vector ---------------------------------------------------------
        "split" => {
            let (v, t) = num(input(0, "vector")?, id, "vector")?;
            if t.width() < 2 {
                return Err(GraphError::TypeMismatch {
                    node: id,
                    port: "vector".into(),
                });
            }
            let mut outs = Vec::new();
            for (i, c) in ["x", "y", "z", "w"].iter().enumerate() {
                if (i as u32) < t.width() {
                    outs.push((i as u32, Val::Num(format!("{name}.{c}"), GraphTy::F32)));
                }
            }
            Ok((v, outs))
        }
        "combine2" | "combine3" | "combine4" => {
            let width = match node.kind.as_str() {
                "combine2" => 2,
                "combine3" => 3,
                _ => 4,
            };
            let ports = ["x", "y", "z", "w"];
            let mut parts = Vec::new();
            for (i, port) in ports.iter().enumerate().take(width) {
                let (e, t) = num(input(i as u32, ports[i])?, id, port)?;
                if t.width() != 1 {
                    return Err(GraphError::TypeMismatch {
                        node: id,
                        port: (*port).to_owned(),
                    });
                }
                parts.push(e);
            }
            let ty = GraphTy::of_width(width as u32);
            Ok((format!("{}({})", ty.wgsl(), parts.join(", ")), one(ty)))
        }
        "swizzle" => {
            let (v, t) = num(input(0, "vector")?, id, "vector")?;
            if t.width() < 2 {
                return Err(GraphError::TypeMismatch {
                    node: id,
                    port: "vector".into(),
                });
            }
            let pattern = super::graph::swizzle_pattern(node);
            let widest = pattern
                .chars()
                .map(|c| "xyzw".find(c).unwrap_or(0) as u32)
                .max()
                .unwrap_or(0);
            if widest >= t.width() {
                return Err(GraphError::BadParameter(format!(
                    "the pattern `{pattern}` reads past a {} value",
                    t.wgsl()
                )));
            }
            Ok((
                format!("{v}.{pattern}"),
                one(GraphTy::of_width(pattern.len() as u32)),
            ))
        }
        // -- Texture --------------------------------------------------------
        "sample" => {
            let helper = match input(0, "picture")? {
                Val::Pic(h) => h,
                Val::Num(..) => {
                    return Err(GraphError::TypeMismatch {
                        node: id,
                        port: "picture".into(),
                    })
                }
            };
            let (uvx, ut) = num(input(1, "uv")?, id, "uv")?;
            if ut != GraphTy::Vec2 {
                return Err(GraphError::TypeMismatch {
                    node: id,
                    port: "uv".into(),
                });
            }
            Ok((format!("{helper}({uvx})"), one(GraphTy::Vec4)))
        }
        // -- Colour ---------------------------------------------------------
        "luminance" => {
            let c = coerce4(input(0, "colour")?, id, "colour")?;
            Ok((
                format!("dot({c}.rgb, vec3<f32>(0.2126, 0.7152, 0.0722))"),
                one(GraphTy::F32),
            ))
        }
        "premultiply" => {
            let c = coerce4(input(0, "colour")?, id, "colour")?;
            Ok((format!("lumit_premult({c})"), one(GraphTy::Vec4)))
        }
        "unpremultiply" => {
            let c = coerce4(input(0, "colour")?, id, "colour")?;
            Ok((format!("lumit_unpremult({c})"), one(GraphTy::Vec4)))
        }
        "tint" => {
            let a = coerce4(input(0, "colour")?, id, "colour")?;
            let b = coerce4(input(1, "tint")?, id, "tint")?;
            Ok((
                format!("vec4<f32>({a}.rgb * {b}.rgb, {a}.a)"),
                one(GraphTy::Vec4),
            ))
        }
        "blend" => {
            let base = coerce4(input(0, "base")?, id, "base")?;
            let over = coerce4(input(1, "blend")?, id, "blend")?;
            let (amt, at) = num(input(2, "amount")?, id, "amount")?;
            if at.width() != 1 {
                return Err(GraphError::TypeMismatch {
                    node: id,
                    port: "amount".into(),
                });
            }
            let a = format!("{base}.rgb");
            let b = format!("{over}.rgb");
            // ponytail: a linear-light subset of BlendMode::ALL, formulas
            // inlined per chosen mode. The upgrade path is the fx_blend_mix
            // table, when the node claims parity with the layer modes.
            let mode = node
                .settings
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("normal");
            let formula = match mode {
                "normal" => b.clone(),
                "add" => format!("({a} + {b})"),
                "multiply" => format!("({a} * {b})"),
                "screen" => format!("({a} + {b} - {a} * {b})"),
                "darken" => format!("min({a}, {b})"),
                "lighten" => format!("max({a}, {b})"),
                "difference" => format!("abs({a} - {b})"),
                "subtract" => format!("max({a} - {b}, vec3<f32>(0.0))"),
                "overlay" => format!(
                    "mix(2.0 * {a} * {b}, vec3<f32>(1.0) - 2.0 * (vec3<f32>(1.0) - {a}) * (vec3<f32>(1.0) - {b}), step(vec3<f32>(0.5), {a}))"
                ),
                other => {
                    return Err(GraphError::BadParameter(format!(
                        "`{other}` is not a blend mode this box knows"
                    )))
                }
            };
            Ok((
                format!("mix({base}, vec4<f32>({formula}, {base}.a), {amt})"),
                one(GraphTy::Vec4),
            ))
        }
        other => Err(GraphError::UnknownKind(other.to_owned())),
    }
}

// ------------------------------------------------------------ the parameters

/// A Parameter box's field id and its annotated struct lines — the box's five
/// facts (kind, range, default, unit, label) spelt in the §1.4 grammar, so the
/// existing reader derives the row and nothing downstream learns a new shape.
fn param_field(node: &ShaderNode) -> Result<(String, String), GraphError> {
    let s = &node.settings;
    let get = |k: &str| s.get(k).and_then(|v| v.as_str());
    let getn = |k: &str| s.get(k).and_then(serde_json::Value::as_f64);

    let id = get("id").unwrap_or("").trim().to_owned();
    let ok = !id.is_empty()
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(GraphError::BadParameter(format!(
            "a parameter needs a plain name (letters, digits, underscores); `{id}` is not one"
        )));
    }
    let label: String = get("label")
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c == '@' || c == '\n' || c == '\r' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let label = if label.is_empty() {
        super::humanise(&id)
    } else {
        label
    };
    let unit = match get("unit") {
        None | Some("") => String::new(),
        Some(u @ ("px" | "deg" | "s")) => format!(" @unit({u})"),
        Some(other) => {
            return Err(GraphError::BadParameter(format!(
                "`{other}` is not a unit; the units are px, deg and s"
            )))
        }
    };
    let fmt = |v: f64| -> Result<String, GraphError> {
        if !v.is_finite() {
            return Err(GraphError::BadParameter(format!(
                "`{label}` has a number that is not one"
            )));
        }
        let text = format!("{v}");
        Ok(if text.contains('.') || text.contains('e') {
            text
        } else {
            format!("{text}.0")
        })
    };
    let defaults = |n: usize, fallback: &[f64]| -> Result<Vec<String>, GraphError> {
        let raw = s.get("default");
        let list: Vec<f64> = match raw {
            Some(serde_json::Value::Array(a)) => {
                a.iter().filter_map(serde_json::Value::as_f64).collect()
            }
            Some(v) => v.as_f64().into_iter().collect(),
            None => Vec::new(),
        };
        (0..n)
            .map(|i| fmt(*list.get(i).or_else(|| fallback.get(i)).unwrap_or(&0.0)))
            .collect()
    };

    let lo = fmt(getn("min").unwrap_or(0.0))?;
    let hi = fmt(getn("max").unwrap_or(1.0))?;
    let kind = get("kind").unwrap_or("slider");
    let (ann, ty) = match kind {
        "slider" => (
            format!("@slider({lo}, {hi}) @default({}){unit}", defaults(1, &[0.0])?[0]),
            "f32",
        ),
        "bounded" => (
            format!("@bounded({lo}, {hi}) @default({}){unit}", defaults(1, &[0.0])?[0]),
            "f32",
        ),
        "dial" => (format!("@dial @default({})", defaults(1, &[0.0])?[0]), "f32"),
        "colour" => (
            format!(
                "@colour @default({})",
                defaults(4, &[1.0, 1.0, 1.0, 1.0])?.join(", ")
            ),
            "vec4<f32>",
        ),
        "point" => (
            format!("@point @default({})", defaults(2, &[0.0, 0.0])?.join(", ")),
            "vec2<f32>",
        ),
        other => {
            return Err(GraphError::BadParameter(format!(
                "`{other}` is not a parameter kind; the kinds are slider, bounded, dial, colour and point"
            )))
        }
    };
    Ok((
        id.clone(),
        format!("    /// {ann} {label}\n    {id}: {ty},\n"),
    ))
}

// ------------------------------------------------- the session compile cache

/// FNV-1a folded over whatever serde writes, so a graph is hashed without ever
/// being turned into a string on the render path.
struct HashWriter(u64);

impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

type CompileCache = RwLock<BTreeMap<u64, Result<&'static str, &'static str>>>;

fn cache() -> &'static CompileCache {
    static CACHE: OnceLock<CompileCache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// The compiled text for one stored graph, built once per distinct graph and
/// kept for the session — the same memoisation [`super::program_for`] gives a
/// distinct source, and for the same reason: the resolve walk asks per frame.
///
/// **The graph is master** (§4.1): the render calls this and never reads the
/// cached `source` beside it, so a `source` that has been tampered with in the
/// file is a stale convenience, not a conflict.
///
/// # Errors
/// The [`GraphError`] sentence, or what did not parse — either way a badge, a
/// last good picture, and an effect that still resolves.
pub fn source_for(graph: &serde_json::Value) -> Result<&'static str, &'static str> {
    let mut hw = HashWriter(0xcbf2_9ce4_8422_2325);
    // Serialising a Value cannot fail and the writer cannot either; a failure
    // here would still only cost a cache miss, never a wrong answer.
    let _ = serde_json::to_writer(&mut hw, graph);
    let key = hw.0;
    if let Ok(map) = cache().read() {
        if let Some(hit) = map.get(&key) {
            return *hit;
        }
    }
    let built: Result<&'static str, &'static str> =
        match serde_json::from_value::<ShaderGraph>(graph.clone()) {
            Err(why) => Err(&*Box::leak(
                format!("the stored graph does not parse: {why}").into_boxed_str(),
            )),
            Ok(g) => match compile(&g) {
                Ok(text) => Ok(&*Box::leak(text.into_boxed_str())),
                Err(why) => Err(&*Box::leak(why.to_string().into_boxed_str())),
            },
        };
    if let Ok(mut map) = cache().write() {
        map.insert(key, built);
    }
    built
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::graph::{ShaderEdge, ShaderGraph, ShaderNode};
    use super::*;

    fn node(id: u32, kind: &str) -> ShaderNode {
        ShaderNode {
            id,
            kind: kind.to_owned(),
            settings: serde_json::Map::new(),
        }
    }

    fn edge(from: u32, from_port: u32, to: u32, to_port: u32) -> ShaderEdge {
        ShaderEdge {
            from,
            from_port,
            to,
            to_port,
        }
    }

    /// The §8 item 22 fixture: a uv gradient through maths to the output, with
    /// one Parameter box — every family except texture and colour, which have
    /// their own cases below.
    fn gradient() -> ShaderGraph {
        let mut gain = node(6, "param");
        gain.settings = serde_json::json!({
            "id": "gain", "kind": "slider", "min": 0, "max": 2, "default": 1
        })
        .as_object()
        .unwrap()
        .clone();
        ShaderGraph {
            nodes: vec![
                node(1, "uv"),
                node(2, "split"),
                node(3, "multiply"),
                node(4, "combine4"),
                node(5, "result"),
                gain,
                node(7, "multiply"),
            ],
            edges: vec![
                edge(1, 0, 2, 0),
                edge(2, 0, 4, 0),
                edge(2, 1, 4, 1),
                edge(2, 0, 3, 0),
                edge(2, 1, 3, 1),
                edge(3, 0, 4, 2),
                edge(4, 0, 7, 0),
                edge(6, 0, 7, 1),
                edge(7, 0, 5, 0),
            ],
            layout: Vec::new(),
        }
    }

    /// §8 item 22 — the determinism gate for the whole of §4: the same graph,
    /// twice, on two threads, and against a golden string.
    #[test]
    fn a_graph_compiles_to_byte_identical_wgsl() {
        let golden = "struct Params {\n\
                      \x20   /// @slider(0.0, 2.0) @default(1.0) Gain\n\
                      \x20   gain: f32,\n\
                      }\n\
                      \n\
                      fn shade(uv: vec2<f32>) -> vec4<f32> {\n\
                      \x20   let n0 = uv;\n\
                      \x20   let n1 = n0;\n\
                      \x20   let n2 = (n1.x * n1.y);\n\
                      \x20   let n3 = vec4<f32>(n1.x, n1.y, n2, 1.0);\n\
                      \x20   let n4 = p.gain;\n\
                      \x20   let n5 = (n3 * vec4<f32>(n4));\n\
                      \x20   return n5;\n\
                      }\n";
        assert_eq!(compile(&gradient()).unwrap(), golden);
        let threads: Vec<_> = (0..2)
            .map(|_| std::thread::spawn(|| compile(&gradient()).unwrap()))
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), golden, "on every machine, for ever");
        }
    }

    /// §8 item 23's compile half: one graph holding every box in the v1
    /// vocabulary compiles. (The assembled module is validated through the
    /// K-263 road in `lumit-gpu`'s tests, where naga lives.)
    #[test]
    fn every_node_in_the_v1_vocabulary_compiles() {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut id = 100u32;
        // A vec4 spine every colour box eats in turn, ending at the Result.
        let mut spine = 1u32;
        nodes.push(node(1, "picture"));
        for kind in ["premultiply", "unpremultiply", "tint", "blend"] {
            id += 1;
            let mut n = node(id, kind);
            if kind == "blend" {
                n.settings = serde_json::json!({"mode": "overlay"})
                    .as_object()
                    .unwrap()
                    .clone();
            }
            nodes.push(n);
            edges.push(edge(spine, 0, id, 0));
            spine = id;
        }
        // Every scalar box chained off the picture's luminance.
        nodes.push(node(2, "luminance"));
        edges.push(edge(1, 0, 2, 0));
        let mut scalar = 2u32;
        for kind in [
            "add",
            "subtract",
            "multiply",
            "divide",
            "modulo",
            "mix",
            "clamp",
            "saturate",
            "pow",
            "sqrt",
            "abs",
            "sign",
            "min",
            "max",
            "floor",
            "ceil",
            "fract",
            "step",
            "smoothstep",
            "sin",
            "cos",
            "atan2",
            "length",
            "distance",
        ] {
            id += 1;
            nodes.push(node(id, kind));
            edges.push(edge(scalar, 0, id, 0));
            scalar = id;
        }
        // The vector family, the remaining inputs, and the sampler.
        nodes.push(node(3, "uv"));
        nodes.push(node(4, "split"));
        edges.push(edge(3, 0, 4, 0));
        nodes.push(node(5, "combine2"));
        edges.push(edge(4, 0, 5, 0));
        edges.push(edge(4, 1, 5, 1));
        nodes.push(node(6, "normalize"));
        edges.push(edge(5, 0, 6, 0));
        nodes.push(node(7, "dot"));
        edges.push(edge(6, 0, 7, 0));
        edges.push(edge(5, 0, 7, 1));
        let mut sw = node(8, "swizzle");
        sw.settings = serde_json::json!({"pattern": "yx"})
            .as_object()
            .unwrap()
            .clone();
        nodes.push(sw);
        edges.push(edge(5, 0, 8, 0));
        nodes.push(node(9, "picture2"));
        nodes.push(node(10, "sample"));
        edges.push(edge(9, 1, 10, 0));
        edges.push(edge(8, 0, 10, 1));
        nodes.push(node(11, "matte"));
        nodes.push(node(12, "time"));
        nodes.push(node(13, "seed"));
        nodes.push(node(14, "combine3"));
        edges.push(edge(11, 0, 14, 0));
        edges.push(edge(12, 0, 14, 1));
        edges.push(edge(13, 0, 14, 2));
        nodes.push(node(15, "combine4"));
        // Everything left feeds the spine's amount so the chain is one graph.
        edges.push(edge(scalar, 0, spine, 2));
        nodes.push(node(16, "result"));
        edges.push(edge(spine, 0, 16, 0));
        let graph = ShaderGraph {
            nodes,
            edges,
            layout: Vec::new(),
        };
        let text = compile(&graph).expect("the whole vocabulary compiles");
        assert!(text.contains("fn shade"));
    }

    /// §8 item 24, first half.
    #[test]
    fn a_cycle_is_refused_at_the_edit() {
        let graph = ShaderGraph {
            nodes: vec![node(1, "add"), node(2, "add"), node(3, "result")],
            edges: vec![edge(1, 0, 2, 0), edge(2, 0, 1, 0), edge(2, 0, 3, 0)],
            layout: Vec::new(),
        };
        assert_eq!(compile(&graph), Err(GraphError::Cycle));
    }

    /// §8 item 24, second half: a vec2 and a vec4 meeting at one add.
    #[test]
    fn a_type_mismatch_is_refused_at_the_drop() {
        let graph = ShaderGraph {
            nodes: vec![
                node(1, "uv"),
                node(2, "picture"),
                node(3, "add"),
                node(4, "result"),
            ],
            edges: vec![edge(1, 0, 3, 0), edge(2, 0, 3, 1), edge(3, 0, 4, 0)],
            layout: Vec::new(),
        };
        assert!(matches!(
            compile(&graph),
            Err(GraphError::TypeMismatch { node: 3, .. })
        ));
        // And a tap past a vector's width: uv has no z.
        let graph = ShaderGraph {
            nodes: vec![
                node(1, "uv"),
                node(2, "split"),
                node(3, "result"),
                node(4, "combine4"),
            ],
            edges: vec![edge(1, 0, 2, 0), edge(2, 2, 4, 0), edge(4, 0, 3, 0)],
            layout: Vec::new(),
        };
        assert!(matches!(
            compile(&graph),
            Err(GraphError::TypeMismatch { .. })
        ));
    }

    /// §8 item 25: node → annotation → `ParamSchema` — the graph's parameters
    /// and a hand-written shader's are one mechanism with two front doors.
    #[test]
    fn a_parameter_node_becomes_a_row() {
        let text = compile(&gradient()).unwrap();
        let program = super::super::build(&text).expect("the compiled text reads");
        let row = program
            .params
            .iter()
            .find(|r| r.id == "gain")
            .expect("the Parameter box derived a row");
        assert_eq!(row.label, "Gain");
        assert!(matches!(
            row.kind,
            crate::fx::ParamKind::Float { default, slider, .. }
                if default == 1.0 && slider == (0.0, 2.0)
        ));
    }

    /// §8 item 26: with a graph present the render compiles from the graph
    /// even when the cached `source` has been tampered with in the file.
    #[test]
    fn the_graph_is_master() {
        let mut inst = crate::fx::instantiate("custom_shader").expect("the effect exists");
        let mut block = serde_json::Map::new();
        block.insert(
            "source".into(),
            "fn shade(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(9.0); }".into(),
        );
        block.insert(
            "graph".into(),
            serde_json::to_value(gradient()).expect("the graph serialises"),
        );
        inst.extra
            .insert("shader".into(), serde_json::Value::Object(block));
        let program =
            crate::fx::effects::custom_shader::program_of(&inst).expect("the graph compiles");
        assert!(
            !program.assembled.contains("vec4<f32>(9.0)"),
            "the tampered cache never renders"
        );
        assert!(
            program.params.iter().any(|r| r.id == "gain"),
            "the derived rows are the graph's"
        );
    }

    /// A graph the canvas can build but the compiler must refuse: no Result,
    /// two Results, a wire to nowhere, two wires on one input.
    #[test]
    fn the_refusals_each_have_a_sentence() {
        let no_result = ShaderGraph {
            nodes: vec![node(1, "uv")],
            edges: Vec::new(),
            layout: Vec::new(),
        };
        assert_eq!(compile(&no_result), Err(GraphError::NoResult));
        let two = ShaderGraph {
            nodes: vec![node(1, "result"), node(2, "result")],
            edges: Vec::new(),
            layout: Vec::new(),
        };
        assert_eq!(compile(&two), Err(GraphError::ManyResults));
        let dangling = ShaderGraph {
            nodes: vec![node(1, "result")],
            edges: vec![edge(9, 0, 1, 0)],
            layout: Vec::new(),
        };
        assert_eq!(compile(&dangling), Err(GraphError::Dangling));
        let doubled = ShaderGraph {
            nodes: vec![
                node(1, "time"),
                node(2, "seed"),
                node(3, "result"),
                node(4, "add"),
            ],
            edges: vec![edge(1, 0, 4, 0), edge(2, 0, 4, 0), edge(4, 0, 3, 0)],
            layout: Vec::new(),
        };
        assert_eq!(compile(&doubled), Err(GraphError::DoubleInput));
        for e in [
            GraphError::NoResult,
            GraphError::Cycle,
            GraphError::UnknownKind("warp".into()),
        ] {
            assert!(!e.to_string().is_empty());
        }
    }
}
