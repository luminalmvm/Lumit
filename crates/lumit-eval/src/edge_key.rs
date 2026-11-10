//! What one node graph wire adds to a frame's name.

/// Fold one wire into the frame key.
///
/// Where it comes from and where it goes, tag byte first so two shapes cannot
/// hash alike, and every port id length-prefixed so `a` into `bc` and `ab` into
/// `c` are different wires. Node **ids** are hashed here — unlike a mask, where
/// the position is what counts — because a wire has no position: the id is the
/// only thing that says which of two identical drivers is feeding the socket.
pub(super) fn feed_edge(h: &mut blake3::Hasher, edge: &lumit_core::graph::Edge) {
    use lumit_core::graph::{InputRef, NodeRef, OutputRef};
    let name = |h: &mut blake3::Hasher, s: &str| {
        h.update(&(s.len() as u64).to_le_bytes());
        h.update(s.as_bytes());
    };
    match &edge.from {
        OutputRef::SourceMatte => {
            h.update(&[0]);
        }
        OutputRef::Driver { node, port } => {
            h.update(&[1]);
            h.update(node.as_bytes());
            name(h, port);
        }
        // A points wire is an edge like any other, so it folds like one
        // (points-stream.md §3.4): cutting it names a different frame, and no
        // new term is needed to say so. A fresh tag byte, so the two id-and-port
        // shapes above cannot hash alike with it.
        OutputRef::EffectData { effect, port } => {
            h.update(&[2]);
            h.update(effect.as_bytes());
            name(h, port);
        }
        OutputRef::EffectOutput { effect } => {
            h.update(&[3]);
            h.update(effect.as_bytes());
        }
    }
    match &edge.to {
        InputRef::Matte { effect } => {
            h.update(&[0]);
            h.update(effect.as_bytes());
        }
        InputRef::Param { node, port } => {
            h.update(&[1]);
            match node {
                NodeRef::Source => {
                    h.update(&[0]);
                }
                NodeRef::Effect(id) => {
                    h.update(&[1]);
                    h.update(id.as_bytes());
                }
                NodeRef::Driver(id) => {
                    h.update(&[2]);
                    h.update(id.as_bytes());
                }
                NodeRef::Out => {
                    h.update(&[3]);
                }
            }
            name(h, port);
        }
    }
}
