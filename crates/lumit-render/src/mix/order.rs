//! The desk's shape before it is a plan: which node feeds which, the order
//! the nodes are mixed in, and which edges a mute or a solo leaves on.
//!
//! Pure arithmetic on indices, run on the thread building the plan.

/// One node's edges, by index into the list the builder made.
#[derive(Clone, Debug, Default)]
pub(super) struct Edges {
    /// The node its output goes to, None for the master.
    pub out: Option<usize>,
    /// The nodes it sends to.
    pub sends: Vec<usize>,
    /// The nodes of those its muted sends go to: in the order, never on.
    pub quiet: Vec<usize>,
    /// The node it is lowered behind.
    pub key: Option<usize>,
    /// What the solo rule reads: soloed or solo safe, and soloed.
    pub marked: bool,
    pub solo: bool,
    /// Its audible switch is off.
    pub muted: bool,
    /// Where it stands when nothing else decides: buses after strips, then
    /// in layer order.
    pub rank: (bool, usize),
}

/// The order to mix `nodes` in: every output, send and duck key runs from an
/// earlier node to a later one, ties going by rank. The ops refuse a cycle,
/// so one should never be here; if one is, the edge into the node that
/// would come next is dropped (an output to it goes to the master instead).
pub(super) fn order(nodes: &mut [Edges]) -> Vec<usize> {
    let n = nodes.len();
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let feeds = |nodes: &[Edges], u: usize, v: usize| {
        nodes[u].out == Some(v) || nodes[u].sends.contains(&v) || nodes[v].key == Some(u)
    };
    while order.len() < n {
        let next = (0..n)
            .filter(|&v| !done[v] && (0..n).all(|u| done[u] || u == v || !feeds(nodes, u, v)))
            .min_by_key(|&v| nodes[v].rank);
        let v = match next {
            Some(v) => v,
            None => {
                let Some(v) = (0..n).filter(|&v| !done[v]).min_by_key(|&v| nodes[v].rank) else {
                    break;
                };
                for u in (0..n).filter(|&u| !done[u] && u != v) {
                    if nodes[u].out == Some(v) {
                        nodes[u].out = None;
                    }
                    nodes[u].sends.retain(|&s| s != v);
                }
                nodes[v].key = None;
                v
            }
        };
        done[v] = true;
        order.push(v);
    }
    order
}

/// Which edges are on, for nodes already in mixing order (every edge runs
/// to a later index): `(output on, each send on)` per node.
///
/// Mute: a muted node's edges are off, both ways. Solo: a node is marked when
/// soloed or solo safe; it is "up" when marked or fed by an up node, and
/// "down" when soloed or feeding a down node. So a solo safe node keeps its
/// own way out but pulls nothing into the solo. With any node soloed, an edge
/// `u -> v` is on when `u` is up or `v` is down, and an output to the master
/// when `u` is up. Last, every edge of a node with no path left to the master
/// goes off.
pub(super) fn edges_on(nodes: &[Edges]) -> Vec<(bool, Vec<bool>)> {
    let n = nodes.len();
    let any_solo = nodes.iter().any(|e| e.solo);
    let sends_to = |e: &Edges, w: usize| e.sends.contains(&w) && !e.quiet.contains(&w);
    let into =
        |v: usize| (0..v).filter(move |&u| nodes[u].out == Some(v) || sends_to(&nodes[u], v));
    let mut up = vec![false; n];
    for v in 0..n {
        up[v] = nodes[v].marked || into(v).any(|u| up[u]);
    }
    let mut down = vec![false; n];
    for v in (0..n).rev() {
        let e = &nodes[v];
        down[v] = e.solo
            || e.out.is_some_and(|w| down[w])
            || e.sends.iter().any(|&w| sends_to(e, w) && down[w]);
    }
    let on = |u: usize, to: Option<usize>| {
        let muted = nodes[u].muted || to.is_some_and(|w| nodes[w].muted);
        let solo = !any_solo || up[u] || to.is_some_and(|w| down[w]);
        !muted && solo
    };
    let mut flags: Vec<(bool, Vec<bool>)> = nodes
        .iter()
        .enumerate()
        .map(|(u, e)| {
            (
                on(u, e.out),
                e.sends
                    .iter()
                    .map(|&w| sends_to(e, w) && on(u, Some(w)))
                    .collect(),
            )
        })
        .collect();
    // Heard: an edge that is on reaches the master, directly or through a
    // node that is heard. Edges only run upward, so walk down.
    let mut heard = vec![false; n];
    for v in (0..n).rev() {
        let e = &nodes[v];
        let (out, sends) = &flags[v];
        heard[v] = (*out && e.out.is_none_or(|w| heard[w]))
            || e.sends.iter().zip(sends).any(|(&w, &s)| s && heard[w]);
    }
    for (v, (out, sends)) in flags.iter_mut().enumerate() {
        if !heard[v] {
            *out = false;
            sends.fill(false);
        }
    }
    flags
}
