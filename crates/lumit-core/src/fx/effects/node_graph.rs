//! Node graph (docs/impl/node-graph-comp.md §1.3, §1.5): the effect that
//! applies a node graph composition to a layer.
//!
//! **In plain terms.** A node graph is a composition, so it can be placed like
//! any other. This is the other way to use one: drop it on a layer and the
//! layer's picture goes into the graph's first picture Input, the graph's other
//! Inputs become rows on this effect, and what the graph's Output holds comes
//! back out. A graph with no Read boxes is therefore a reusable effect with a
//! picture in and a picture out - and the same effect is how one node graph
//! nests inside another.
//!
//! **The rows are derived, never adopted** (§1.5). They come from a copy of the
//! graph's Input list the instance carries in `extra.node_graph.inputs`, read
//! through a cache keyed by that list's hash, exactly as the Custom
//! shader reads the rows its source declares. A copy rather than a document
//! lookup because [`EffectDef::derived`] has no document in its hand. The copy
//! is refreshed on the clones the bridge hands out, so a row added inside the
//! graph is *offered* the next time the stack is read and lands in the document
//! with the user's next edit - never written behind anybody's back. The
//! renderer reads none of it: it lowers the graph from the document and reads
//! this instance's parameters by the Inputs' own ids.
//!
//! **`roi = FullFrame`, `cost = Heavy`**, for the Custom shader's reasons: what
//! the graph inside does is not knowable from here, so the honest declarations
//! are the cautious ones. There is no CPU reference - the effect *is* another
//! walk of the engine - so `apply_cpu` keeps its identity default, which is what
//! a dangling graph reference renders anyway.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::comp_graph::{CompGraph, GraphInput, InputKind};
use crate::fx::shader::hash64;
use crate::fx::{EffectDef, EffectMetadata, EffectSchema, ParamKind, ParamSchema, Unit};
use crate::model::{Document, EffectInstance};
use lumit_fx_macros::Effect;
use uuid::Uuid;

/// The `extra` key the bound comp and the Inputs copy live under.
pub const EXTRA_KEY: &str = "node_graph";

/// The ids this effect's own declaration uses. A derived row may not collide
/// with one, or two controls would share an id and the panel would draw the
/// graph's row over the effect's.
///
/// Public because the renderer reads it too: an Input under one of these ids
/// has no row of its own, so the value the host hands it must not be this
/// effect's own Mix, Blend or Matte.
pub const DECLARED_IDS: &[&str] = &[
    "open",
    "mix",
    "blend",
    "matte",
    "matte_invert",
    "matte_channel",
];

/// The Node graph effect's declared controls. Everything else it shows is
/// derived from the graph it names.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "node_graph",
    label = "Node graph",
    version = 1,
    category = Utility,
    // A graph nobody here has read: the honest class is the cautious one.
    cost = Heavy,
    // Anything inside may sample anywhere, so the whole frame is the only
    // correct region.
    roi = FullFrame,
)]
pub struct NodeGraph {
    /// Front the graph this instance names. A button, not a value: which comp
    /// is bound is not a parameter, and there is nothing here to keyframe.
    #[action(label = "Open graph")]
    pub open: (),

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent -
    /// so a graph dissolves back over the layer like any other effect, on the
    /// same seam, with the injected Blend beside it.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// The composition this instance applies, or `None` for one nobody has bound.
///
/// A name that no longer answers is a dangling reference like any other: the
/// effect renders its input unchanged rather than faulting.
#[must_use]
pub fn comp_of(inst: &EffectInstance) -> Option<Uuid> {
    Uuid::parse_str(inst.extra.get(EXTRA_KEY)?.get("comp")?.as_str()?).ok()
}

/// This instance's copy of the graph's Input list - the declaration its derived
/// rows are read from.
///
/// Empty for an unbound instance, and for a copy a hand or an older build wrote
/// in a shape this one cannot read: an unreadable declaration is a stack to
/// render with no extra rows, never a project to refuse.
#[must_use]
pub fn inputs_of(inst: &EffectInstance) -> Vec<GraphInput> {
    inst.extra
        .get(EXTRA_KEY)
        .and_then(|block| block.get("inputs"))
        .and_then(|list| serde_json::from_value(list.clone()).ok())
        .unwrap_or_default()
}

/// Point `inst` at `comp` and write the Inputs copy from `graph` - what adding
/// a Node graph effect does, and what binding it to another graph does.
pub fn bind(inst: &mut EffectInstance, comp: Uuid, graph: &CompGraph) {
    let inputs: Vec<GraphInput> = graph.inputs().cloned().collect();
    inst.extra.insert(
        EXTRA_KEY.to_owned(),
        serde_json::json!({
            "comp": comp.to_string(),
            "inputs": serde_json::to_value(inputs).unwrap_or(serde_json::Value::Null),
        }),
    );
}

/// Rewrite the Inputs copy from the live graph, and say whether that changed
/// anything.
///
/// **Offered, never adopted** (docs/08 §3.95's rule): the bridge calls this on
/// the clones it hands out, so a row added inside the graph appears the next
/// time the stack is read and lands in the document with the user's next edit.
/// A comp that is not there, or is not a node graph, leaves the copy exactly as
/// it was - a row for an Input the graph no longer has stays an ordinary
/// parameter the graph ignores.
pub fn refresh(inst: &mut EffectInstance, doc: &Document) -> bool {
    let Some(comp) = comp_of(inst) else {
        return false;
    };
    let Some(graph) = doc.comp(comp).and_then(|c| c.graph.as_ref()) else {
        return false;
    };
    let live: Vec<GraphInput> = graph.inputs().cloned().collect();
    if live == inputs_of(inst) {
        return false;
    }
    bind(inst, comp, graph);
    true
}

/// The values a host hands a graph's Inputs (§1.5): the instance's own rows,
/// by the Inputs' ids, with a driver wire's number in place of the stored one
/// wherever a wire feeds the socket.
///
/// A picture Input takes no value - it arrives as a socket or a row of its own
/// - so it is skipped here and `project` bakes nothing for it.
///
/// An Input whose id is one of the effect's own (`mix`, `blend`, the matte
/// trio) derives no row (§1.5), so it has no row of its own to read: taking the
/// row under that id would hand the graph this effect's Mix. It bakes its own
/// default instead.
///
/// It sits here rather than in the draw builder because the frame key and the
/// nested name read it too, and one host's values must name one picture.
#[must_use]
pub fn overrides_of(
    inst: &EffectInstance,
    graph: &CompGraph,
    drivers: Option<&crate::fx::ResolvedDrivers>,
) -> Vec<(String, crate::model::EffectValue)> {
    use crate::anim::Property;
    use crate::model::EffectValue;
    graph
        .inputs()
        .filter(|input| input.kind != InputKind::Picture)
        .filter(|input| !DECLARED_IDS.contains(&input.id.as_str()))
        .filter_map(|input| {
            let driven = drivers
                .filter(|d| !d.is_empty())
                .and_then(|d| {
                    d.param(
                        crate::graph::NodeRef::Effect(inst.id),
                        crate::fx::ParamId::new(&input.id),
                    )
                })
                .map(|v| EffectValue::Float(Property::fixed(f64::from(v.as_f32()))));
            let stored = inst
                .params
                .iter()
                .find(|p| p.id == input.id)
                .map(|p| p.value.clone());
            Some((input.id.clone(), driven.or(stored)?))
        })
        .collect()
}

/// The rows `inputs` derives, built once per distinct list and kept until
/// Lumit closes.
///
/// Memoised for [`crate::fx::shader::program_for`]'s reason: the render path
/// asks for these once per op per frame, because the derived rows are part of
/// resolving the stack, so this is a hash and a map lookup rather than a
/// rebuild. The entries are `&'static`, which for words somebody typed means
/// leaked: the honest spelling of "lives as long as the process".
///
/// Nothing is evicted, so editing N distinct Input lists holds N small records,
/// each a handful of rows. That is the same bargain the shader's program cache
/// makes and it is bounded by how many graphs a hand can edit.
#[must_use]
pub fn input_rows(inputs: &[GraphInput]) -> &'static [ParamSchema] {
    let json = serde_json::to_vec(inputs).unwrap_or_default();
    let key = hash64(&json);
    let cache = cache();
    if let Ok(map) = cache.read() {
        if let Some(hit) = map.get(&key) {
            return hit;
        }
    }
    let built: &'static [ParamSchema] = Box::leak(build_rows(inputs).into_boxed_slice());
    if let Ok(mut map) = cache.write() {
        map.insert(key, built);
    }
    built
}

/// Every Input list this process has derived rows from, by its hash.
type RowCache = RwLock<HashMap<u64, &'static [ParamSchema]>>;

fn cache() -> &'static RowCache {
    static CACHE: OnceLock<RowCache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// One Input list's rows, in document order, which is the row order.
///
/// **The first picture Input derives no row**: it is the layer's own picture at
/// this point in the stack, which arrives on the effect's input rather than
/// through a control. Every further picture Input is a layer reference, on the
/// ordinary auxiliary-layer carriage.
fn build_rows(inputs: &[GraphInput]) -> Vec<ParamSchema> {
    let mut rows = Vec::new();
    let mut seen_picture = false;
    for input in inputs {
        if input.kind == InputKind::Picture && !seen_picture {
            seen_picture = true;
            continue;
        }
        // A row that collided with one of this effect's own, or with a row an
        // earlier Input already made, would be two controls under one id.
        if DECLARED_IDS.contains(&input.id.as_str())
            || rows.iter().any(|r: &ParamSchema| r.id == input.id)
        {
            continue;
        }
        let d = input.default;
        let (kind, unit) = match input.kind {
            InputKind::Picture => (
                ParamKind::Layer {
                    self_default: false,
                },
                Unit::Raw,
            ),
            InputKind::Number => (
                ParamKind::Float {
                    default: d[0],
                    slider: (input.min, input.max),
                    // The Input's own range is the travel, not a wall: what a
                    // graph's number means is the graph's business, and typing
                    // past the end is how a rig is built.
                    hard: (None, None),
                },
                input.unit,
            ),
            // An angle is degrees by definition, exactly as `#[dial]` is.
            InputKind::Angle => (
                ParamKind::Angle {
                    default: d[0],
                    dial_step: 15.0,
                },
                Unit::Degrees,
            ),
            InputKind::Colour => (
                ParamKind::Colour {
                    default: d,
                    range: (0.0, 1.0),
                },
                Unit::Raw,
            ),
        };
        rows.push(ParamSchema {
            id: Box::leak(input.id.clone().into_boxed_str()),
            label: Box::leak(input.label.clone().into_boxed_str()),
            kind,
            unit,
        });
    }
    rows
}

/// The Node graph effect's behaviour.
pub struct NodeGraphDef;

impl EffectDef for NodeGraphDef {
    fn schema(&self) -> &'static EffectSchema {
        &<NodeGraph as EffectMetadata>::SCHEMA
    }

    /// The rows this instance's own Inputs copy declares (§1.5), cached per
    /// distinct list - so this is a hash and a map lookup on the render path.
    ///
    /// They are **offered**, not adopted: what the document stores is still the
    /// document's, and the stored values are what render.
    fn derived(&self, inst: &EffectInstance) -> &'static [ParamSchema] {
        input_rows(&inputs_of(inst))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::comp_graph::{CompGraph, GraphNode};
    use crate::model::{Composition, LinearColour, MotionBlur, ProjectItem};
    use crate::time::{Duration, FrameRate, Rational};

    fn declared(id: &str, kind: InputKind, unit: Unit) -> GraphInput {
        GraphInput {
            id: id.to_owned(),
            label: "Amount".to_owned(),
            kind,
            default: [2.0, 0.25, 0.5, 1.0],
            min: -5.0,
            max: 25.0,
            unit,
            preview: None,
        }
    }

    fn graph_of(inputs: &[GraphInput]) -> CompGraph {
        let mut nodes: Vec<GraphNode> = inputs
            .iter()
            .map(|input| GraphNode::Input {
                id: uuid::Uuid::now_v7(),
                input: input.clone(),
            })
            .collect();
        nodes.push(GraphNode::Output {
            id: uuid::Uuid::now_v7(),
        });
        CompGraph {
            nodes,
            edges: Vec::new(),
            layout: Vec::new(),
            exposed: Vec::new(),
            groups: Vec::new(),
        }
    }

    fn comp_of_graph(graph: &CompGraph) -> Composition {
        Composition {
            id: uuid::Uuid::now_v7(),
            name: "Node graph 1".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(25, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            groups: Vec::new(),
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            master_volume_db: 0.0,
            sound_mix: false,
            beat_grid: None,
            graph: Some(graph.clone()),
            extra: serde_json::Map::new(),
        }
    }

    /// The rows follow the Inputs copy, one kind at a time - and the **first**
    /// picture Input derives none, because it is the layer's own picture.
    #[test]
    fn the_derived_rows_follow_the_inputs_copy() {
        let inputs = vec![
            declared("plate", InputKind::Picture, Unit::Raw),
            declared("second", InputKind::Picture, Unit::Raw),
            declared("amount", InputKind::Number, Unit::Px),
            declared("turn", InputKind::Angle, Unit::Degrees),
            declared("tint", InputKind::Colour, Unit::Raw),
        ];
        let rows = input_rows(&inputs);
        let ids: Vec<&str> = rows.iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            vec!["second", "amount", "turn", "tint"],
            "the first picture Input is the effect's own input, not a row"
        );
        assert_eq!(rows[0].label, "Amount", "the Input's own word");
        assert_eq!(
            rows[0].kind,
            ParamKind::Layer {
                self_default: false
            },
            "a further picture is a layer reference"
        );
        assert_eq!(
            rows[1].kind,
            ParamKind::Float {
                default: 2.0,
                slider: (-5.0, 25.0),
                hard: (None, None),
            }
        );
        assert_eq!(rows[1].unit, Unit::Px, "the Input's declared unit");
        assert_eq!(
            rows[2].kind,
            ParamKind::Angle {
                default: 2.0,
                dial_step: 15.0,
            }
        );
        assert_eq!(
            rows[3].kind,
            ParamKind::Colour {
                default: [2.0, 0.25, 0.5, 1.0],
                range: (0.0, 1.0),
            }
        );
    }

    /// A row that collided with one of the effect's own would be two controls
    /// under one id, so it is left off rather than drawn over.
    #[test]
    fn an_input_that_collides_with_a_declared_row_derives_none() {
        let inputs = vec![
            declared("mix", InputKind::Number, Unit::Raw),
            declared("matte", InputKind::Number, Unit::Raw),
            declared("gain", InputKind::Number, Unit::Raw),
            declared("gain", InputKind::Number, Unit::Raw),
        ];
        let ids: Vec<&str> = input_rows(&inputs).iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["gain"], "one row, and none of the effect's own");
    }

    /// What a host hands the Inputs (§1.5): the stored row, the driver's
    /// number in front of it where a wire feeds the socket, and nothing at all
    /// for a picture Input or an Input under one of this effect's own ids.
    #[test]
    fn the_overrides_prefer_a_driver_and_skip_the_declared_ids() {
        use crate::fx::ParamId;
        use crate::graph::NodeRef;
        use crate::model::{EffectParam, EffectValue};

        let inputs = vec![
            declared("plate", InputKind::Picture, Unit::Raw),
            declared("gain", InputKind::Number, Unit::Raw),
            declared("turn", InputKind::Angle, Unit::Degrees),
            declared("mix", InputKind::Number, Unit::Raw),
        ];
        let graph = graph_of(&inputs);
        let mut inst = crate::fx::instantiate("node_graph").expect("the catalogue knows it");
        for id in ["gain", "turn"] {
            inst.params.push(EffectParam {
                id: id.to_owned(),
                value: EffectValue::Float(crate::anim::Property::fixed(4.0)),
                extra: serde_json::Map::new(),
            });
        }

        let plain = overrides_of(&inst, &graph, None);
        let ids: Vec<&str> = plain.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["gain", "turn"],
            "no picture, and none of the effect's own ids"
        );

        // A wire onto the row hands its number over instead of the stored one.
        // Math, because its answer is arithmetic anybody can predict.
        let mut math = crate::fx::instantiate("math").expect("the catalogue knows it");
        for p in &mut math.params {
            match p.id.as_str() {
                "a" => p.value = EffectValue::Float(crate::anim::Property::fixed(3.0)),
                "b" => p.value = EffectValue::Float(crate::anim::Property::fixed(3.0)),
                "operation" => p.value = EffectValue::Choice(2),
                _ => {}
            }
        }
        let wires = crate::graph::LayerGraph {
            nodes: vec![math.clone()],
            edges: vec![crate::graph::Edge {
                from: crate::graph::OutputRef::Driver {
                    node: math.id,
                    port: "value".to_owned(),
                },
                to: crate::graph::InputRef::Param {
                    node: NodeRef::Effect(inst.id),
                    port: "gain".to_owned(),
                },
            }],
            ..crate::graph::LayerGraph::default()
        };
        let driven = crate::fx::resolve_drivers(
            &wires,
            0.0,
            std::sync::Arc::new(crate::expression::ExpressionContext::detached()),
            None,
        );
        assert_eq!(
            driven.param(NodeRef::Effect(inst.id), ParamId::new("gain")),
            Some(crate::fx::Value::Float(9.0)),
            "three times three"
        );
        let with_wire = overrides_of(&inst, &graph, Some(&driven));
        let gain = with_wire
            .iter()
            .find(|(id, _)| id == "gain")
            .map(|(_, v)| v.clone());
        match gain {
            Some(EffectValue::Float(p)) => assert_eq!(p.value_at(0.0), 9.0),
            other => panic!("gain is a number, not {other:?}"),
        }
    }

    /// An instance with nothing bound shows its two declared rows and no more.
    #[test]
    fn an_unbound_instance_derives_nothing() {
        let inst = crate::fx::instantiate("node_graph").expect("the catalogue knows it");
        assert_eq!(comp_of(&inst), None);
        assert!(inputs_of(&inst).is_empty());
        assert!(NodeGraphDef.derived(&inst).is_empty());
    }

    /// **Offered, never adopted**: `refresh` rewrites the copy from the live
    /// graph and says whether it did, so the panel can show a new row without
    /// the document being edited behind anybody's back.
    #[test]
    fn binding_writes_the_copy_and_refresh_follows_the_live_graph() {
        let first = vec![declared("amount", InputKind::Number, Unit::Raw)];
        let graph = graph_of(&first);
        let comp = comp_of_graph(&graph);
        let comp_id = comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));

        let mut inst = crate::fx::instantiate("node_graph").expect("the catalogue knows it");
        bind(&mut inst, comp_id, &graph);
        assert_eq!(comp_of(&inst), Some(comp_id));
        assert_eq!(inputs_of(&inst), first);
        assert_eq!(NodeGraphDef.derived(&inst).len(), 1);

        assert!(
            !refresh(&mut inst, &doc),
            "nothing moved, so nothing is rewritten"
        );

        // A row added inside the graph is picked up on the next read.
        let mut grown = first.clone();
        grown.push(declared("turn", InputKind::Angle, Unit::Degrees));
        let live = graph_of(&grown);
        if let Some(ProjectItem::Composition(c)) = doc.item_mut(comp_id) {
            c.graph = Some(live);
        }
        assert!(refresh(&mut inst, &doc), "the copy was behind the graph");
        assert_eq!(inputs_of(&inst), grown);
        assert_eq!(NodeGraphDef.derived(&inst).len(), 2);

        // A comp that is not there, or is not a node graph, leaves the copy
        // exactly as it was.
        if let Some(ProjectItem::Composition(c)) = doc.item_mut(comp_id) {
            c.graph = None;
        }
        assert!(!refresh(&mut inst, &doc));
        assert_eq!(inputs_of(&inst), grown);
    }
}
