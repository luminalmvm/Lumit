//! The demand-pull pixel-pass executor and its trait seams
//! (docs/05-ARCHITECTURE.md §1.1, docs/06-RENDER-PIPELINE.md §1.1).
//!
//! In plain terms: [`crate::graph`] compiles a composition into a wiring
//! diagram; this module *walks* it. Starting from the final "comp output"
//! box it works backwards — to blend a layer you first need that layer's
//! transformed pixels, to transform them you first need the source frame —
//! producing each box's picture exactly once (two layers sharing a source
//! share the one fetched frame) and handing the actual pixel work to
//! pluggable parts through three sockets:
//!
//! - [`FrameSource`] — "give me this source's frame" (media decode, a solid,
//!   text rasterisation — whatever the source is);
//! - [`KernelExecutor`] — "run this one step" (a transform, a blend, a mask)
//!   on already-produced inputs;
//! - [`CacheStore`] — "have we rendered this exact frame before?" keyed by
//!   the content hash, checked before any work and filled after.
//!
//! The sockets are traits *defined here* so the executor unit-tests against
//! fakes with no GPU, no codecs and no disk — the same seam
//! 05-ARCHITECTURE §1.1 specifies. The real implementations live app-side
//! (GPU kernels in `lumit-gpu`, decode in `lumit-media`, the cache in
//! `lumit-cache`) and are wired in by the shell; frames are passed between
//! them as opaque [`FrameHandle`]s the implementor mints.
//!
//! Cancellation follows docs/impl/playback-scheduler.md §1: the walk checks
//! its [`EpochToken`] between nodes and unwinds quietly with
//! [`ExecError::Cancelled`]; implementors are handed the token so long
//! kernels can check it mid-step too.

use crate::epoch::{Cancelled, EpochToken};
use crate::graph::{EvalGraph, NodeId, NodeKind, SourceRef};
use crate::FrameKey;

/// An opaque reference to one produced frame. The implementor mints these —
/// a texture-pool index, a buffer id — and is the only party that can look
/// inside; the executor just routes them from producers to consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameHandle(pub u64);

/// Why a render did not produce a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// The work was invalidated mid-flight (epoch bump) — unwind quietly,
    /// nothing is wrong (docs/impl/playback-scheduler.md §1).
    Cancelled,
    /// A node could not produce pixels; `message` is implementor-provided
    /// (a decode error, a device error). Never a panic (docs/14 §4).
    Node { node: NodeId, message: String },
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("cancelled"),
            Self::Node { node, message } => write!(f, "node {node}: {message}"),
        }
    }
}

impl std::error::Error for ExecError {}

impl From<Cancelled> for ExecError {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}

/// Supplies source pixels: media frames, solids, rasterised text, nested
/// comps. `t` is comp time; the implementor holds the document snapshot and
/// resolves the layer-local / retimed source time itself, exactly as the
/// shipped renderer does.
pub trait FrameSource {
    fn source_frame(
        &mut self,
        source: &SourceRef,
        t: f64,
        token: &EpochToken,
    ) -> Result<FrameHandle, ExecError>;
}

/// Runs one non-source node — a retime interpolation, a mask stack, a
/// transform, a composite, an adjustment, the final comp output — over
/// already-produced input frames.
pub trait KernelExecutor {
    fn run(
        &mut self,
        node: NodeId,
        kind: &NodeKind,
        inputs: &[FrameHandle],
        t: f64,
        token: &EpochToken,
    ) -> Result<FrameHandle, ExecError>;
}

/// The rendered-frame cache, keyed by content hash (docs/06 §5.2). `get`
/// before any work; `put` after a completed render — including one that
/// turns out stale, because the work is already paid for (docs/06 §6.3).
pub trait CacheStore {
    fn get(&mut self, key: FrameKey) -> Option<FrameHandle>;
    fn put(&mut self, key: FrameKey, frame: FrameHandle);
}

/// Render one comp frame by demand-pulling `graph` from its output node.
///
/// `key` is the frame's content-hash key when the frame is keyable
/// ([`crate::comp_frame_key`]); `None` renders live without touching the
/// cache. Each node is produced exactly once per call (shared sources are
/// fetched once); inputs always run before the node that needs them, bottom
/// composite before the one above it. The token is checked between every
/// node, so a bumped epoch abandons the walk within one node's work.
pub fn render_frame(
    graph: &EvalGraph,
    t: f64,
    key: Option<FrameKey>,
    source: &mut dyn FrameSource,
    kernels: &mut dyn KernelExecutor,
    cache: &mut dyn CacheStore,
    token: &EpochToken,
) -> Result<FrameHandle, ExecError> {
    token.check()?;
    if let Some(k) = key {
        if let Some(hit) = cache.get(k) {
            return Ok(hit);
        }
    }

    let mut memo: Vec<Option<FrameHandle>> = vec![None; graph.len()];
    // Iterative post-order walk (no recursion — comp depth must never be a
    // stack-overflow panic path, docs/14 §4). An entry is (node, expanded):
    // first visit pushes the node back with expanded = true underneath its
    // pending inputs, so by the time it pops again every input is in `memo`.
    let mut stack: Vec<(NodeId, bool)> = vec![(graph.output, false)];
    while let Some((id, expanded)) = stack.pop() {
        if memo.get(id).copied().flatten().is_some() {
            continue;
        }
        let Some(node) = graph.nodes.get(id) else {
            return Err(ExecError::Node {
                node: id,
                message: "node id out of range (malformed graph)".into(),
            });
        };
        if expanded {
            token.check()?;
            let mut inputs = Vec::with_capacity(node.inputs.len());
            for &input in &node.inputs {
                match memo.get(input).copied().flatten() {
                    Some(handle) => inputs.push(handle),
                    None => {
                        return Err(ExecError::Node {
                            node: id,
                            message: "input not produced (malformed graph)".into(),
                        })
                    }
                }
            }
            let produced = match &node.kind {
                NodeKind::Source { source: sref } => source.source_frame(sref, t, token)?,
                kind => kernels.run(id, kind, &inputs, t, token)?,
            };
            if let Some(slot) = memo.get_mut(id) {
                *slot = Some(produced);
            }
        } else {
            stack.push((id, true));
            for &input in &node.inputs {
                // compile() mints ids in push order, so a well-formed graph
                // only ever points a node at earlier nodes. Rejecting the
                // other direction makes the walk provably terminate even on
                // a hand-built malformed graph (no cycle can hang us).
                if input >= id {
                    return Err(ExecError::Node {
                        node: id,
                        message: "input does not precede node (malformed graph)".into(),
                    });
                }
                if memo.get(input).copied().flatten().is_none() {
                    stack.push((input, false));
                }
            }
        }
    }

    let out = memo
        .get(graph.output)
        .copied()
        .flatten()
        .ok_or(ExecError::Node {
            node: graph.output,
            message: "output not produced (malformed graph)".into(),
        })?;
    if let Some(k) = key {
        cache.put(k, out);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::epoch::Epoch;
    use crate::graph::Node;
    use std::cell::RefCell;
    use std::rc::Rc;
    use uuid::Uuid;

    type CallLog = Rc<RefCell<Vec<String>>>;

    /// Mints sequential handles and logs every source fetch.
    struct FakeSource {
        log: CallLog,
        next: u64,
    }

    impl FrameSource for FakeSource {
        fn source_frame(
            &mut self,
            source: &SourceRef,
            _t: f64,
            _token: &EpochToken,
        ) -> Result<FrameHandle, ExecError> {
            self.log.borrow_mut().push(format!("source {source:?}"));
            self.next += 1;
            Ok(FrameHandle(self.next))
        }
    }

    /// Logs every kernel run; optionally bumps an epoch after N runs to
    /// simulate a scrub landing mid-render.
    struct FakeKernels {
        log: CallLog,
        next: u64,
        bump_after: Option<(Epoch, usize)>,
        runs: usize,
    }

    impl KernelExecutor for FakeKernels {
        fn run(
            &mut self,
            node: NodeId,
            kind: &NodeKind,
            inputs: &[FrameHandle],
            _t: f64,
            _token: &EpochToken,
        ) -> Result<FrameHandle, ExecError> {
            self.runs += 1;
            if let Some((epoch, after)) = &self.bump_after {
                if self.runs == *after {
                    epoch.bump();
                }
            }
            let kind_name = match kind {
                NodeKind::Source { .. } => "source?",
                NodeKind::Retime => "retime",
                NodeKind::Masks { .. } => "masks",
                NodeKind::Transform { .. } => "transform",
                NodeKind::Composite { .. } => "composite",
                NodeKind::Adjust { .. } => "adjust",
                NodeKind::CompOutput { .. } => "output",
            };
            self.log
                .borrow_mut()
                .push(format!("run {kind_name} n{node} <- {inputs:?}"));
            self.next += 1;
            Ok(FrameHandle(self.next))
        }
    }

    #[derive(Default)]
    struct FakeCache {
        entries: Vec<(FrameKey, FrameHandle)>,
        gets: usize,
    }

    impl CacheStore for FakeCache {
        fn get(&mut self, key: FrameKey) -> Option<FrameHandle> {
            self.gets += 1;
            self.entries
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, h)| *h)
        }

        fn put(&mut self, key: FrameKey, frame: FrameHandle) {
            self.entries.push((key, frame));
        }
    }

    fn fakes(log: &CallLog) -> (FakeSource, FakeKernels, FakeCache) {
        (
            FakeSource {
                log: Rc::clone(log),
                next: 100,
            },
            FakeKernels {
                log: Rc::clone(log),
                next: 200,
                bump_after: None,
                runs: 0,
            },
            FakeCache::default(),
        )
    }

    /// Two layers sharing one solid source, hand-built exactly as
    /// [`crate::graph::compile`] would lay it out: the shared Source node,
    /// each layer's Transform, the bottom Composite, the top Composite over
    /// it, then CompOutput.
    fn two_layer_shared_source() -> (EvalGraph, Uuid, Uuid) {
        let (l_top, l_bottom) = (Uuid::now_v7(), Uuid::now_v7());
        let solid = Uuid::now_v7();
        let node = |kind, inputs| Node { kind, inputs };
        let graph = EvalGraph {
            nodes: vec![
                node(
                    NodeKind::Source {
                        source: SourceRef::Solid(solid),
                    },
                    vec![],
                ),
                node(NodeKind::Transform { layer: l_bottom }, vec![0]),
                node(
                    NodeKind::Composite {
                        layer: l_bottom,
                        blend: lumit_core::model::BlendMode::Normal,
                        has_matte: false,
                    },
                    vec![1],
                ),
                node(NodeKind::Transform { layer: l_top }, vec![0]),
                node(
                    NodeKind::Composite {
                        layer: l_top,
                        blend: lumit_core::model::BlendMode::Normal,
                        has_matte: false,
                    },
                    vec![3, 2],
                ),
                node(
                    NodeKind::CompOutput {
                        comp: Uuid::now_v7(),
                        width: 8,
                        height: 8,
                    },
                    vec![4],
                ),
            ],
            output: 5,
        };
        (graph, l_top, l_bottom)
    }

    #[test]
    fn demand_pull_runs_inputs_first_and_bottom_composite_before_top() {
        let log: CallLog = CallLog::default();
        let (mut src, mut ker, mut cache) = fakes(&log);
        let (graph, ..) = two_layer_shared_source();
        let token = Epoch::new().token();
        let out = render_frame(&graph, 0.5, None, &mut src, &mut ker, &mut cache, &token).unwrap();
        let log = log.borrow();
        // The shared source is fetched exactly once, before anything uses it.
        assert_eq!(
            log.iter().filter(|l| l.starts_with("source")).count(),
            1,
            "shared source must be fetched once: {log:?}"
        );
        assert!(log[0].starts_with("source"), "source first: {log:?}");
        // Every node's inputs run before it; bottom composite before top.
        let pos = |needle: &str| log.iter().position(|l| l.contains(needle)).unwrap();
        assert!(pos("transform n1") < pos("composite n2"), "{log:?}");
        assert!(pos("transform n3") < pos("composite n4"), "{log:?}");
        assert!(pos("composite n2") < pos("composite n4"), "{log:?}");
        assert!(pos("composite n4") < pos("output n5"), "{log:?}");
        // The returned handle is the output node's product.
        assert!(log.last().unwrap().contains("output"), "{log:?}");
        assert!(out.0 >= 200, "output comes from the kernel executor");
    }

    #[test]
    fn a_cache_hit_short_circuits_all_work() {
        let log: CallLog = CallLog::default();
        let (mut src, mut ker, mut cache) = fakes(&log);
        let key = FrameKey(42);
        cache.entries.push((key, FrameHandle(7)));
        let (graph, ..) = two_layer_shared_source();
        let token = Epoch::new().token();
        let out = render_frame(
            &graph,
            0.0,
            Some(key),
            &mut src,
            &mut ker,
            &mut cache,
            &token,
        )
        .unwrap();
        assert_eq!(out, FrameHandle(7));
        assert!(log.borrow().is_empty(), "no source or kernel work on a hit");
    }

    #[test]
    fn a_cache_miss_renders_then_fills_the_cache_under_the_same_key() {
        let log: CallLog = CallLog::default();
        let (mut src, mut ker, mut cache) = fakes(&log);
        let key = FrameKey(9);
        let (graph, ..) = two_layer_shared_source();
        let token = Epoch::new().token();
        let out = render_frame(
            &graph,
            0.0,
            Some(key),
            &mut src,
            &mut ker,
            &mut cache,
            &token,
        )
        .unwrap();
        assert_eq!(cache.entries, vec![(key, out)]);
        // And a second render of the same key is now a pure hit.
        let calls_before = log.borrow().len();
        let again = render_frame(
            &graph,
            0.0,
            Some(key),
            &mut src,
            &mut ker,
            &mut cache,
            &token,
        )
        .unwrap();
        assert_eq!(again, out);
        assert_eq!(log.borrow().len(), calls_before, "hit does no new work");
    }

    #[test]
    fn an_unkeyable_frame_renders_live_and_never_touches_the_cache() {
        let log: CallLog = CallLog::default();
        let (mut src, mut ker, mut cache) = fakes(&log);
        let (graph, ..) = two_layer_shared_source();
        let token = Epoch::new().token();
        render_frame(&graph, 0.0, None, &mut src, &mut ker, &mut cache, &token).unwrap();
        assert_eq!(cache.gets, 0);
        assert!(cache.entries.is_empty());
    }

    /// A scrub landing mid-render (epoch bump inside a kernel) abandons the
    /// walk with Cancelled and leaves the cache untouched.
    #[test]
    fn an_epoch_bump_mid_render_cancels_and_caches_nothing() {
        let log: CallLog = CallLog::default();
        let epoch = Epoch::new();
        let token = epoch.token();
        let (mut src, mut ker, mut cache) = fakes(&log);
        ker.bump_after = Some((epoch, 2));
        let (graph, ..) = two_layer_shared_source();
        let out = render_frame(
            &graph,
            0.0,
            Some(FrameKey(1)),
            &mut src,
            &mut ker,
            &mut cache,
            &token,
        );
        assert_eq!(out, Err(ExecError::Cancelled));
        assert!(
            cache.entries.is_empty(),
            "a cancelled render caches nothing"
        );
        // The walk stopped early: fewer kernel runs than the full graph.
        assert!(ker.runs < 5, "stopped after the bump, ran {}", ker.runs);
    }

    /// The malformed-graph guards return typed errors instead of hanging or
    /// panicking (docs/14 §4: engine code never crashes on bad input).
    #[test]
    fn malformed_graphs_error_instead_of_hanging_or_panicking() {
        let log: CallLog = CallLog::default();
        let (mut src, mut ker, mut cache) = fakes(&log);
        let token = Epoch::new().token();
        // A self-referencing node (the cycle case push order forbids).
        let cyclic = EvalGraph {
            nodes: vec![Node {
                kind: NodeKind::CompOutput {
                    comp: Uuid::now_v7(),
                    width: 1,
                    height: 1,
                },
                inputs: vec![0],
            }],
            output: 0,
        };
        assert!(matches!(
            render_frame(&cyclic, 0.0, None, &mut src, &mut ker, &mut cache, &token),
            Err(ExecError::Node { .. })
        ));
        // An output pointing past the node list.
        let dangling = EvalGraph {
            nodes: Vec::new(),
            output: 3,
        };
        assert!(matches!(
            render_frame(&dangling, 0.0, None, &mut src, &mut ker, &mut cache, &token),
            Err(ExecError::Node { .. })
        ));
    }

    /// The executor + pool together: a frame rendered on a worker, exactly as
    /// the shell will submit it (docs/05 §2 job = one graph evaluation).
    #[test]
    fn a_render_job_runs_on_the_worker_pool() {
        use crate::pool::{JobClass, WorkerPool};
        let pool = WorkerPool::new(2).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let (graph, ..) = two_layer_shared_source();
        let token = Epoch::new().token();
        pool.try_spawn(JobClass::Interactive, move || {
            // Thread-local fakes: the seams are plain &mut dyn, nothing shared.
            let log: CallLog = CallLog::default();
            let (mut src, mut ker, mut cache) = fakes(&log);
            let result = render_frame(&graph, 0.0, None, &mut src, &mut ker, &mut cache, &token);
            let _ = tx.send(result);
        })
        .unwrap();
        let result = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(result.is_ok());
    }
}
