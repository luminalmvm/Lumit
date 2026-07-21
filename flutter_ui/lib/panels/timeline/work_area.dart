// The work-area band geometry on the ruler: where its in/out edges sit and
// which edge (if any) a pointer has grabbed. The band's look mirrors the egui
// draw in crates/lumit-ui/src/shell/timeline/panel.rs (a filled strip in the
// success tint along the ruler top); here it also gains draggable edges, so the
// hit-test is its own pure, unit-tested helper.

/// Which work-area edge a gesture is over.
enum WorkAreaEdge { inEdge, outEdge }

/// The lane-local x pixels of a work-area band's two edges (0 = the lane's left,
/// i.e. `LaneScale.trackLeft`).
class WorkAreaEdges {
  final double inX;
  final double outX;

  const WorkAreaEdges(this.inX, this.outX);
}

/// The edge within [tol] px of lane-local [localX], or null when neither is
/// near. The nearer edge wins a tie so a drag never grabs the wrong side; the
/// out edge is preferred only when it is strictly closer.
WorkAreaEdge? workAreaEdgeAt(
  double localX,
  double inX,
  double outX, {
  double tol = 6,
}) {
  final dIn = (localX - inX).abs();
  final dOut = (localX - outX).abs();
  final nearIn = dIn <= tol;
  final nearOut = dOut <= tol;
  if (nearIn && nearOut) {
    return dOut < dIn ? WorkAreaEdge.outEdge : WorkAreaEdge.inEdge;
  }
  if (nearIn) return WorkAreaEdge.inEdge;
  if (nearOut) return WorkAreaEdge.outEdge;
  return null;
}
