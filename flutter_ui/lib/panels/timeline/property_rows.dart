// Which transform property rows a layer's Transform twirl shows, and in what
// grouped order. Ported from the egui `transform_property_rows`
// (crates/lumit-ui/src/shell/inspector/transform_rows.rs): Anchor point,
// Position, Scale, Rotation, Opacity, with the x/y pairs sharing one row
// (AE-style — the two values stay independent, the row furniture merges), plus
// the 3D-only rows (Position z, Rotation x, Rotation y) when the layer is 3D.
// Pure data so the row set is unit-tested without a widget tree.

import '../../bridge/bridge.dart';

/// One property row descriptor: its display [label] and the snake_case bridge
/// property name(s) it edits — a pair (`anchor_x`, `anchor_y`) on one row, or a
/// single name for Rotation/Opacity.
class PropRowSpec {
  final String label;
  final List<String> props;

  const PropRowSpec(this.label, this.props);

  /// A two-axis (x/y) row that shows two value readouts.
  bool get isPair => props.length == 2;

  /// The property name whose animation drives the row's stopwatch/navigator
  /// (the x channel of a pair, like egui's linked rows).
  String get primary => props.first;
}

/// The transform rows for a layer, in the egui outline's order. [threeD] adds
/// the depth rows; [isCamera] drops Anchor point (cameras have no anchor row).
List<PropRowSpec> transformRows({
  required bool threeD,
  required bool isCamera,
}) {
  return [
    if (!isCamera) const PropRowSpec('Anchor point', ['anchor_x', 'anchor_y']),
    const PropRowSpec('Position', ['position_x', 'position_y']),
    const PropRowSpec('Scale', ['scale_x', 'scale_y']),
    const PropRowSpec('Rotation', ['rotation']),
    const PropRowSpec('Opacity', ['opacity']),
    if (threeD) ...const [
      PropRowSpec('Position z', ['position_z']),
      PropRowSpec('Rotation x', ['rotation_x']),
      PropRowSpec('Rotation y', ['rotation_y']),
    ],
  ];
}

/// The union of a row's keyframes across its axes, sorted by frame and
/// de-duplicated by frame (a linked pair keys both axes together, so one glyph
/// stands for both) — the lane and navigator both work on this union, mirroring
/// egui's `union_key_times`.
List<BridgeKeyframe> rowKeys(BridgeTransform? transform, PropRowSpec spec) {
  if (transform == null) return const [];
  final out = <BridgeKeyframe>[];
  final seen = <int>{};
  for (final name in spec.props) {
    final prop = transform[name];
    if (prop == null) continue;
    for (final k in prop.keys) {
      if (seen.add(k.frame)) out.add(k);
    }
  }
  out.sort((a, b) => a.frame.compareTo(b.frame));
  return out;
}

/// Whether any axis of a row is animated (its stopwatch shows accent).
bool rowAnimated(BridgeTransform? transform, PropRowSpec spec) {
  if (transform == null) return false;
  for (final name in spec.props) {
    if (transform[name]?.animated == true) return true;
  }
  return false;
}
