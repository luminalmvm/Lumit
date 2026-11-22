//! What a walk through a comp's layers carries down into the comps it
//! walks into.

/// What a walk carries beyond the frame, handed on to every comp it walks
/// into.
#[derive(Clone, Copy)]
pub(super) struct Walk<'a> {
    /// Which copy of how many this walk's layers are rendered as, for their
    /// Clone index drivers. It goes down into every precomp and node graph
    /// the walk reaches, so a Clone index anywhere in a clone picture reads
    /// that picture's number.
    pub(super) clone: (u32, u32),
    /// A placed node graph's own Input values, when the comp being walked is
    /// that graph (docs/impl/node-graph-comp.md §5.3). Empty on every other
    /// walk, which is a graph on its own defaults.
    pub(super) values: &'a [(String, lumit_core::model::EffectValue)],
    /// This walk is a comp built again at another moment, for a temporal
    /// effect further up to read as its neighbour. A neighbour carries no
    /// neighbours of its own, so nothing in it builds any: otherwise an Echo
    /// over a Precomp that holds an Echo builds the whole of the inside once
    /// for every frame of both windows, only to throw it away.
    pub(super) rebuild: bool,
    /// The camera a collapsed precomp's layers are seen through, the one of
    /// the comp it was collapsed into. `None` is this comp's own.
    pub(super) seen_through: Option<lumit_core::model::CameraPose>,
}

impl Walk<'static> {
    /// An ordinary walk: the first copy of one, and no values handed in.
    pub(super) const PLAIN: Walk<'static> = Walk {
        clone: lumit_core::fx::drivers::NO_CLONE,
        values: &[],
        rebuild: false,
        seen_through: None,
    };
}
