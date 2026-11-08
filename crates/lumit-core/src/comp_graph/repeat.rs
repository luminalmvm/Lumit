//! Repeat pairs: finding them, and the graph each time of one draws.

use uuid::Uuid;

use super::{
    is_driver, ports_of, CompGraph, GraphEdge, GraphInput, GraphNode, InputKind, Repeat,
    REPEAT_END, REPEAT_START, VALUE_PORT,
};
use crate::fx::{params::Unit, schema::PortType};
use crate::graph::{GraphError, INPUT_PORT, OUTPUT_PORT};

/// The box standing in for output `k` of a Repeat start's numbers inside one
/// time, or for picture `k` wired in from outside: ids made from the start's,
/// so a time's graph is the same every frame.
fn repeat_node(start: Uuid, kind: u128, k: usize) -> Uuid {
    Uuid::from_u128(start.as_u128() ^ (0x5245_5045_4154 << 64 | kind << 32 | (k as u128 + 1)))
}

/// What a time's Input box stands for: a picture, or a number named `id`.
fn repeat_input(id: String, kind: InputKind) -> GraphInput {
    GraphInput {
        label: id.clone(),
        id,
        kind,
        default: [0.0; 4],
        min: f64::MIN,
        max: f64::MAX,
        unit: Unit::Raw,
        preview: None,
    }
}

/// The Input id the start's number `port` reaches a time by. The start's own
/// id is in it, so a pair inside another reads its own numbers.
pub(super) fn repeat_value_id(start: Uuid, port: &str) -> String {
    format!("repeat_{}_{port}", start.simple())
}

impl CompGraph {
    fn is_box(&self, id: Uuid, match_name: &str) -> bool {
        matches!(self.node(id), Some(GraphNode::Fx(inst)) if inst.effect.match_name == match_name)
    }

    /// Every box `from` reaches along the wires, downstream or up, `from`
    /// among them.
    pub(super) fn reach(&self, from: Uuid, down: bool) -> Vec<Uuid> {
        let mut seen = vec![from];
        let mut stack = vec![from];
        while let Some(at) = stack.pop() {
            for e in &self.edges {
                let (a, b) = if down { (e.from, e.to) } else { (e.to, e.from) };
                if a == at && !seen.contains(&b) {
                    seen.push(b);
                    stack.push(b);
                }
            }
        }
        seen
    }

    /// The Repeat start `end` pairs with: of the starts upstream of it, once
    /// every pair in between is stepped over, the one all the others lead to.
    /// `Ok(None)` when there is none upstream at all, and refused when no one
    /// of them is nearest.
    fn start_of(&self, end: Uuid, depth: u32) -> Result<Option<Uuid>, GraphError> {
        if depth > 64 {
            return Err(GraphError::RepeatUnpaired);
        }
        let into = |node: Uuid| {
            self.edges
                .iter()
                .filter(move |e| e.to == node)
                .map(|e| e.from)
        };
        let mut found = Vec::new();
        let mut seen = vec![end];
        let mut stack: Vec<Uuid> = into(end).collect();
        while let Some(at) = stack.pop() {
            if seen.contains(&at) {
                continue;
            }
            seen.push(at);
            let next = if self.is_box(at, REPEAT_START) {
                found.push(at);
                continue;
            } else if self.is_box(at, REPEAT_END) {
                match self.start_of(at, depth + 1)? {
                    Some(start) => start,
                    None => at,
                }
            } else {
                at
            };
            stack.extend(into(next));
        }
        let near: Vec<Uuid> = found
            .iter()
            .copied()
            .filter(|s| {
                found
                    .iter()
                    .all(|o| o == s || self.reach(*o, true).contains(s))
            })
            .collect();
        match near[..] {
            [start] => Ok(Some(start)),
            [] if found.is_empty() => Ok(None),
            _ => Err(GraphError::RepeatUnpaired),
        }
    }

    /// Every Repeat start and end that pair, with the boxes between them.
    ///
    /// Refused when no one start is nearest an end, two ends share a start, a
    /// wire leaves the boxes between other than into the end's picture
    /// socket, or a pair sits half inside another. A start or an end on its
    /// own is none of these: it hands its picture on.
    pub fn repeats(&self) -> Result<Vec<Repeat>, GraphError> {
        let mut out: Vec<Repeat> = Vec::new();
        for node in &self.nodes {
            let GraphNode::Fx(inst) = node else { continue };
            if inst.effect.match_name != REPEAT_END {
                continue;
            }
            let end = inst.id;
            // An end with no start before it hands its picture on.
            let Some(start) = self.start_of(end, 0)? else {
                continue;
            };
            if out.iter().any(|r| r.start == start) {
                return Err(GraphError::RepeatUnpaired);
            }
            let (down, up) = (self.reach(start, true), self.reach(end, false));
            let body = self
                .nodes
                .iter()
                .map(GraphNode::id)
                .filter(|id| *id != start && *id != end && down.contains(id) && up.contains(id))
                .collect();
            out.push(Repeat { start, end, body });
        }
        for r in &out {
            for e in &self.edges {
                let from_inside = e.from == r.start || r.body.contains(&e.from);
                let to_inside =
                    r.body.contains(&e.to) || (e.to == r.end && e.to_port == INPUT_PORT.id);
                if from_inside && !to_inside {
                    return Err(GraphError::RepeatLeak);
                }
            }
            if out
                .iter()
                .any(|o| r.body.contains(&o.start) != r.body.contains(&o.end))
            {
                return Err(GraphError::RepeatLeak);
            }
        }
        Ok(out)
    }

    /// The graph one time of `r` runs, and the pictures wired in from outside
    /// it, as box and socket.
    ///
    /// The boxes between the pair go in as they are. The start becomes a
    /// picture Input under its own id, so the wires out of it stand, and an
    /// Input of a number for each of its four numbers. The end becomes the
    /// Output. Each picture wired in from outside becomes a picture Input of
    /// its own after the start's, in the order the list gives. The drivers and
    /// number Inputs outside that feed the boxes between are copied in, with
    /// every one upstream of them.
    #[must_use]
    pub fn repeat_graph(&self, r: &Repeat) -> (CompGraph, Vec<(Uuid, String)>) {
        use crate::fx::effects::repeat::PASS_PORTS;
        let inside = |id: Uuid| r.body.contains(&id);
        let value_box = |id: Uuid| match self.node(id) {
            Some(GraphNode::Input { input, .. }) => input.kind != InputKind::Picture,
            Some(GraphNode::Fx(inst)) => is_driver(inst),
            _ => false,
        };
        let picture = |id: Uuid, port: &str| {
            self.node(id).is_some_and(|n| {
                ports_of(self, n, None).1.iter().any(|p| {
                    p.id == port
                        && matches!(p.ty, PortType::Image | PortType::Matte | PortType::Depth)
                })
            })
        };
        let mut nodes = vec![GraphNode::Input {
            id: r.start,
            input: repeat_input(repeat_value_id(r.start, "picture"), InputKind::Picture),
        }];
        for (k, port) in PASS_PORTS.iter().enumerate() {
            nodes.push(GraphNode::Input {
                id: repeat_node(r.start, 1, k),
                input: repeat_input(
                    repeat_value_id(r.start, port.id),
                    if port.ty == PortType::Colour {
                        InputKind::Colour
                    } else {
                        InputKind::Number
                    },
                ),
            });
        }
        nodes.extend(self.nodes.iter().filter(|n| inside(n.id())).cloned());
        let mut edges = Vec::new();
        let mut fed: Vec<(Uuid, String)> = Vec::new();
        let mut copied: Vec<Uuid> = Vec::new();
        for e in &self.edges {
            if !(inside(e.to) || (e.to == r.end && e.to_port == INPUT_PORT.id)) {
                continue;
            }
            let from = if e.from == r.start {
                match PASS_PORTS.iter().position(|p| p.id == e.from_port) {
                    Some(k) => (repeat_node(r.start, 1, k), VALUE_PORT.id.to_owned()),
                    None => (e.from, e.from_port.clone()),
                }
            } else if inside(e.from) {
                (e.from, e.from_port.clone())
            } else if picture(e.from, &e.from_port) {
                let source = (e.from, e.from_port.clone());
                let k = fed.iter().position(|f| *f == source).unwrap_or_else(|| {
                    fed.push(source);
                    fed.len() - 1
                });
                (repeat_node(r.start, 2, k), OUTPUT_PORT.id.to_owned())
            } else if value_box(e.from) {
                // Only through boxes that make numbers: a picture upstream of
                // a driver stays outside, as a wire nobody reads.
                let mut stack = vec![e.from];
                while let Some(at) = stack.pop() {
                    if copied.contains(&at) || !value_box(at) || at == r.start {
                        continue;
                    }
                    copied.push(at);
                    stack.extend(self.edges.iter().filter(|w| w.to == at).map(|w| w.from));
                }
                (e.from, e.from_port.clone())
            } else {
                continue;
            };
            edges.push(GraphEdge {
                from: from.0,
                from_port: from.1,
                to: e.to,
                to_port: e.to_port.clone(),
            });
        }
        nodes.extend(
            self.nodes
                .iter()
                .filter(|n| copied.contains(&n.id()))
                .cloned(),
        );
        edges.extend(
            self.edges
                .iter()
                .filter(|e| copied.contains(&e.from) && copied.contains(&e.to))
                .cloned(),
        );
        for k in 0..fed.len() {
            nodes.push(GraphNode::Input {
                id: repeat_node(r.start, 2, k),
                input: repeat_input(
                    repeat_value_id(r.start, &format!("picture_{k}")),
                    InputKind::Picture,
                ),
            });
        }
        nodes.push(GraphNode::Output { id: r.end });
        let graph = CompGraph {
            nodes,
            edges,
            layout: Vec::new(),
            exposed: Vec::new(),
            groups: Vec::new(),
            notes: Vec::new(),
            ties: Vec::new(),
            looks: Default::default(),
        };
        (graph, fed)
    }
}
