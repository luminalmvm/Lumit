//! The Custom shader's inner node graph: the stored shape and the v1 node
//! vocabulary (docs/impl/custom-shader.md §4, K-642, CS4).
//!
//! # In plain terms
//!
//! Typing shader code is a skill; wiring boxes together is not. A Custom shader
//! can therefore hold a **graph** — boxes for the picture coming in, boxes for
//! adding and multiplying and mixing, one box for the picture going out — and
//! that graph *compiles into* the same shader text a person could have typed
//! (see [`super::compile`]). The text view and the box view are two views
//! of one thing, and when a graph is present the graph is the one that is
//! believed (§4.1): compiling boxes to text always works, while reading
//! arbitrary text back into boxes does not, so the road runs one way and
//! leaving it — Detach — is a deliberate act.
//!
//! This file is the shape the document stores under `extra.shader.graph` and
//! the table of what each box is. Nothing here runs on a graphics card and
//! nothing here is clever: a node is an id, a kind and a small bag of settings;
//! an edge is four numbers.

use serde::{Deserialize, Serialize};

/// The stored graph, exactly as it serialises under `extra.shader.graph`
/// (§1.2). Additive and optional: an instance with no `graph` key is a
/// hand-written shader and none of this exists for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShaderGraph {
    pub nodes: Vec<ShaderNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<ShaderEdge>,
    /// Canvas positions, by node id. Presentation state: it travels with the
    /// file exactly as `LayerGraph::layout` does, and it is **absent from the
    /// frame key** for the same reason — moving a box changes no pixel (§2.4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layout: Vec<ShaderNodePos>,
}

/// One box: a stable small id, what it is, and — for the few kinds that carry
/// facts of their own (a Parameter's five facts, a swizzle's pattern, a blend's
/// mode) — a settings bag that unknown keys ride through unharmed (K-065).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShaderNode {
    pub id: u32,
    pub kind: String,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

/// One wire: from an output port of one node into an input port of another,
/// ports counted in the order [`ports_of`] lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderEdge {
    pub from: u32,
    pub from_port: u32,
    pub to: u32,
    pub to_port: u32,
}

/// Where one box sits on the canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShaderNodePos {
    pub node: u32,
    pub x: f64,
    pub y: f64,
}

/// What a wire carries (§4.3). Four value widths, plus the one non-value:
/// a **picture**, which is not a number but the identity of a texture the
/// Sample box reads through the host's own helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphTy {
    F32,
    Vec2,
    Vec3,
    Vec4,
    Picture,
}

impl GraphTy {
    /// Component count; 0 for a picture, which has none.
    #[must_use]
    pub const fn width(self) -> u32 {
        match self {
            GraphTy::F32 => 1,
            GraphTy::Vec2 => 2,
            GraphTy::Vec3 => 3,
            GraphTy::Vec4 => 4,
            GraphTy::Picture => 0,
        }
    }

    /// The WGSL spelling of a value of this width.
    #[must_use]
    pub const fn wgsl(self) -> &'static str {
        match self {
            GraphTy::F32 => "f32",
            GraphTy::Vec2 => "vec2<f32>",
            GraphTy::Vec3 => "vec3<f32>",
            GraphTy::Vec4 => "vec4<f32>",
            GraphTy::Picture => "picture",
        }
    }

    #[must_use]
    pub const fn of_width(w: u32) -> GraphTy {
        match w {
            2 => GraphTy::Vec2,
            3 => GraphTy::Vec3,
            4 => GraphTy::Vec4,
            _ => GraphTy::F32,
        }
    }
}

/// One kind of box: its family and its ports. The port types here are the
/// **nominal** ones the canvas colours sockets by; the compiler resolves the
/// generic maths ports to the widths actually wired (§4.3 — a scalar
/// broadcasts, two vectors must match).
pub struct NodeSpec {
    pub kind: &'static str,
    /// `input` | `maths` | `vector` | `texture` | `colour` | `output`.
    pub category: &'static str,
    pub inputs: &'static [(&'static str, GraphTy)],
    pub outputs: &'static [(&'static str, GraphTy)],
}

use GraphTy::{Picture, Vec2, Vec4, F32};

/// The whole v1 vocabulary (§4.3), in the order the add-search lists it.
/// Every node is a pure function of its inputs and compiles to one WGSL `let`.
pub const NODE_SPECS: &[NodeSpec] = &[
    // -- Input --------------------------------------------------------------
    NodeSpec {
        kind: "picture",
        category: "input",
        inputs: &[],
        outputs: &[("colour", Vec4), ("picture", Picture)],
    },
    NodeSpec {
        kind: "picture2",
        category: "input",
        inputs: &[],
        outputs: &[("colour", Vec4), ("picture", Picture)],
    },
    NodeSpec {
        kind: "matte",
        category: "input",
        inputs: &[],
        outputs: &[("strength", F32)],
    },
    NodeSpec {
        kind: "uv",
        category: "input",
        inputs: &[],
        outputs: &[("uv", Vec2)],
    },
    NodeSpec {
        kind: "time",
        category: "input",
        inputs: &[],
        outputs: &[("seconds", F32)],
    },
    NodeSpec {
        kind: "seed",
        category: "input",
        inputs: &[],
        outputs: &[("seed", F32)],
    },
    NodeSpec {
        kind: "param",
        category: "input",
        inputs: &[],
        outputs: &[("value", F32)],
    },
    // -- Maths --------------------------------------------------------------
    NodeSpec {
        kind: "add",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "subtract",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "multiply",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "divide",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "modulo",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "mix",
        category: "maths",
        inputs: &[("a", F32), ("b", F32), ("t", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "clamp",
        category: "maths",
        inputs: &[("x", F32), ("lo", F32), ("hi", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "saturate",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "pow",
        category: "maths",
        inputs: &[("x", F32), ("y", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "sqrt",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "abs",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "sign",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "min",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "max",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "floor",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "ceil",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "fract",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "step",
        category: "maths",
        inputs: &[("edge", F32), ("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "smoothstep",
        category: "maths",
        inputs: &[("lo", F32), ("hi", F32), ("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "sin",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "cos",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "atan2",
        category: "maths",
        inputs: &[("y", F32), ("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "length",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "distance",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "dot",
        category: "maths",
        inputs: &[("a", F32), ("b", F32)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "normalize",
        category: "maths",
        inputs: &[("x", F32)],
        outputs: &[("value", F32)],
    },
    // -- Vector -------------------------------------------------------------
    NodeSpec {
        kind: "split",
        category: "vector",
        inputs: &[("vector", Vec4)],
        outputs: &[("x", F32), ("y", F32), ("z", F32), ("w", F32)],
    },
    NodeSpec {
        kind: "combine2",
        category: "vector",
        inputs: &[("x", F32), ("y", F32)],
        outputs: &[("vector", Vec2)],
    },
    NodeSpec {
        kind: "combine3",
        category: "vector",
        inputs: &[("x", F32), ("y", F32), ("z", F32)],
        outputs: &[("vector", GraphTy::Vec3)],
    },
    NodeSpec {
        kind: "combine4",
        category: "vector",
        inputs: &[("x", F32), ("y", F32), ("z", F32), ("w", F32)],
        outputs: &[("vector", Vec4)],
    },
    NodeSpec {
        kind: "swizzle",
        category: "vector",
        inputs: &[("vector", Vec4)],
        outputs: &[("value", F32)],
    },
    // -- Texture ------------------------------------------------------------
    NodeSpec {
        kind: "sample",
        category: "texture",
        inputs: &[("picture", Picture), ("uv", Vec2)],
        outputs: &[("colour", Vec4)],
    },
    // -- Colour -------------------------------------------------------------
    NodeSpec {
        kind: "luminance",
        category: "colour",
        inputs: &[("colour", Vec4)],
        outputs: &[("value", F32)],
    },
    NodeSpec {
        kind: "premultiply",
        category: "colour",
        inputs: &[("colour", Vec4)],
        outputs: &[("colour", Vec4)],
    },
    NodeSpec {
        kind: "unpremultiply",
        category: "colour",
        inputs: &[("colour", Vec4)],
        outputs: &[("colour", Vec4)],
    },
    NodeSpec {
        kind: "tint",
        category: "colour",
        inputs: &[("colour", Vec4), ("tint", Vec4)],
        outputs: &[("colour", Vec4)],
    },
    NodeSpec {
        kind: "blend",
        category: "colour",
        inputs: &[("base", Vec4), ("blend", Vec4), ("amount", F32)],
        outputs: &[("colour", Vec4)],
    },
    // -- Output -------------------------------------------------------------
    NodeSpec {
        kind: "result",
        category: "output",
        inputs: &[("colour", Vec4)],
        outputs: &[],
    },
];

/// The spec for one kind, or `None` for a kind this build has never heard of.
#[must_use]
pub fn spec_of(kind: &str) -> Option<&'static NodeSpec> {
    NODE_SPECS.iter().find(|s| s.kind == kind)
}

/// A node's ports as the canvas draws them: names and nominal types.
pub type Ports = Vec<(&'static str, GraphTy)>;

/// The ports one node actually shows, settings applied: a Parameter's output
/// takes the width its kind declares, and a swizzle's the width its pattern
/// has. Everything else is the spec verbatim.
#[must_use]
pub fn ports_of(node: &ShaderNode) -> (Ports, Ports) {
    let Some(spec) = spec_of(&node.kind) else {
        return (Vec::new(), Vec::new());
    };
    let mut outputs: Vec<(&'static str, GraphTy)> = spec.outputs.to_vec();
    match node.kind.as_str() {
        "param" => {
            if let Some(first) = outputs.first_mut() {
                first.1 = param_ty(node);
            }
        }
        "swizzle" => {
            if let Some(first) = outputs.first_mut() {
                first.1 = GraphTy::of_width(swizzle_pattern(node).len() as u32);
            }
        }
        _ => {}
    }
    (spec.inputs.to_vec(), outputs)
}

/// The value width a Parameter node's kind produces (§4.3): a colour is a
/// vec4, a point a vec2, everything else one number.
#[must_use]
pub fn param_ty(node: &ShaderNode) -> GraphTy {
    match node.settings.get("kind").and_then(|v| v.as_str()) {
        Some("colour") => GraphTy::Vec4,
        Some("point") => GraphTy::Vec2,
        _ => GraphTy::F32,
    }
}

/// A swizzle's pattern, defaulting to `x` — the identity of nothing rather
/// than a refusal, so a freshly dropped box draws before it is configured.
#[must_use]
pub fn swizzle_pattern(node: &ShaderNode) -> String {
    let raw = node
        .settings
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("x");
    let clean: String = raw
        .chars()
        .filter(|c| "xyzw".contains(*c))
        .take(4)
        .collect();
    if clean.is_empty() {
        "x".to_owned()
    } else {
        clean
    }
}
