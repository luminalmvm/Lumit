// The Timeline outline's column groups (docs/07 §4.2).
//
// In plain terms: the outline's columns come in four clusters — the A/V
// switches, the layer's identity (twirl, label, number, name), the render
// switches (flow, fx, motion blur, 3D), and the compositing pickers (matte,
// blend, parent). Dragging a cluster's header moves the whole cluster, so the
// column order is a list of these groups, not of individual columns.
//
// This file is the pure part: the group list, their fixed widths, the reorder
// rule, and where the fold-out's value column lands for a given order. The
// widgets that draw them live in the Timeline panel.

/// One draggable cluster of outline columns.
enum TimelineGroup {
  /// Visibility · audio · solo · lock · shy.
  switches,

  /// Twirl · label chip · layer number · name. The one flexible-width group:
  /// the name takes whatever the fixed groups leave.
  identity,

  /// Flow (or collapse on a Precomp) · fx · motion blur · 3D. Also the span
  /// the fold-out rows align their value cells to.
  render,

  /// Matte · blend · parent.
  compose,

  /// Render time — what this layer's picture cost in the frame the playhead is
  /// on, and, on a twirled-open effect's heading, what that effect cost
  /// (docs/13 §7.1). One narrow cell, and empty until the column's own switch
  /// asks the engine to measure: the measuring is not free, so a column nobody
  /// is reading costs nothing.
  timings,
}

/// The order shipped: switches, identity, render, compose — the After Effects
/// arrangement the owner asked for.
const List<TimelineGroup> defaultGroupOrder = [
  TimelineGroup.switches,
  TimelineGroup.identity,
  TimelineGroup.render,
  TimelineGroup.compose,
  TimelineGroup.timings,
];

/// One switch cell's width, shared by the header and the rows so the icons
/// stack into columns. Wide enough for the boxed icon plus breathing room.
const double switchCellWidth = 22;

/// The gap between two cells of the *same* group — the switch cells' own
/// breathing room, reused between the compose group's pickers so every gap
/// inside a group reads the same.
const double cellGap = 4;

/// The five A/V switch cells.
const double switchesGroupWidth = 5 * switchCellWidth;

/// The render group's span. Wider than its five switch cells because the
/// fold-out property rows put their value cells inside exactly this span
/// (docs/07 §4.3): a 3-axis position needs the room. The switches themselves
/// pack to the left in ordinary [switchCellWidth] cells.
const double renderGroupWidth = 150;

/// The compose group's cells. The matte cell's width covers the dropdown plus
/// its two mode toggles even when unset, so the blend column never shifts as
/// mattes come and go.
const double matteCellWidth = 118;
const double blendCellWidth = 112;
const double parentCellWidth = 96;
const double composeGroupWidth =
    matteCellWidth + cellGap + blendCellWidth + cellGap + parentCellWidth;

/// The render-time cell: wide enough for "1234 ms" and its switch, and no
/// wider — it is a readout beside the work, not a column of the outline that
/// earns its space when nothing is being measured.
const double timingsGroupWidth = 74;

/// The seam between two adjacent groups: a hairline in the header with a
/// margin each side, plain space in the rows (the header's rule is enough to
/// read the grouping by; repeating it down every row is noise). Part of the
/// fixed geometry either way, so the layout maths count it.
const double groupDividerWidth = 7;

/// A dropdown's text inset — [HouseButton]'s horizontal padding plus its
/// always-there 1 px border. The compose group's header titles carry the same
/// inset so each sits directly over the text in the cell below it.
const double dropdownTextInset = 9;

/// The gutter down the right of the lane area (and, in graph view, the
/// outline): where the vertical scrollbar lives. Reserved in the column
/// header too — as a fixed, undraggable block — so the columns do not shift
/// when the view changes.
const double scrollGutterWidth = 12;

/// The width each group starts at. Dragging a header seam changes one of
/// them and leaves the rest alone, so the outline grows or shrinks by exactly
/// what the drag moved (docs/07 §4.2).
const Map<TimelineGroup, double> defaultGroupWidths = {
  TimelineGroup.switches: switchesGroupWidth,
  TimelineGroup.identity: 250,
  TimelineGroup.render: renderGroupWidth,
  TimelineGroup.compose: composeGroupWidth,
  TimelineGroup.timings: timingsGroupWidth,
};

/// How narrow a group may be dragged: enough for the cells that cannot
/// shrink — its icons, or a dropdown you can still read a name in.
double minGroupWidth(TimelineGroup group) => switch (group) {
      TimelineGroup.switches => switchesGroupWidth,
      TimelineGroup.identity => 120,
      TimelineGroup.render => 5 * switchCellWidth,
      TimelineGroup.compose => 180,
      // Enough for the widest number the readout writes ("12.34 s").
      TimelineGroup.timings => 56,
    };

/// The outline's total width for a set of group widths: the groups, the seam
/// between each pair, and the row's own edge padding.
double outlineWidthOf(Map<TimelineGroup, double> widths) =>
    8 +
    widths.values.fold(0.0, (a, b) => a + b) +
    (widths.length - 1) * groupDividerWidth;

/// The compose group's three cells at a given group width, keeping the
/// proportions the defaults set — so widening the group widens the pickers
/// rather than leaving dead space beside them.
(double, double, double) composeCellWidths(double width) {
  final usable = (width - 2 * cellGap).clamp(60.0, 1e6);
  final total = matteCellWidth + blendCellWidth + parentCellWidth;
  return (
    usable * matteCellWidth / total,
    usable * blendCellWidth / total,
    usable * parentCellWidth / total,
  );
}

/// The order after dropping [dragged] onto [target]: the dragged group takes
/// the target's slot, everything between shuffles one place along.
List<TimelineGroup> reorderedGroups(
  List<TimelineGroup> order,
  TimelineGroup dragged,
  TimelineGroup target,
) {
  if (dragged == target) return order;
  final next = [...order]..remove(dragged);
  final at = next.indexOf(target);
  next.insert(
      order.indexOf(dragged) < order.indexOf(target) ? at + 1 : at, dragged);
  return next;
}

/// Where the fold-out rows put their value cells so they sit exactly under
/// the render group (docs/07 §4.3): the span's width, and the fixed width of
/// everything right of it in the current order.
class ValueColumn {
  final double width;
  final double rightInset;
  const ValueColumn(this.width, this.rightInset);
}

/// The fixed width of everything to the right of [group] in the current order,
/// seams included. The flexible identity group counts as whatever width it was
/// last given — see [valueColumnFor] on why that is near enough.
double rightInsetOf(List<TimelineGroup> order,
    Map<TimelineGroup, double> widths, TimelineGroup group) {
  final at = order.indexOf(group);
  // A group the outline is not drawing at all (its bottom-bar toggle is off,
  // or nothing is being measured) has nothing to its right: whatever lines up
  // with it sits at the outline's own right edge. Without this the loop below
  // started at zero and counted *every* group as being to the right of a
  // column that was not there, which pushed the fold-out's value cells clean
  // off the panel the moment the switches column was hidden.
  if (at < 0) return 0;
  var right = 0.0;
  for (var i = at + 1; i < order.length; i++) {
    right += groupDividerWidth + (widths[order[i]] ?? 0);
  }
  return right;
}

/// The value column for a group order. If the identity group has been dragged
/// to the right of the render group its flexible width cannot be measured
/// here, so it counts as zero — the values sit near enough until a real
/// measurement is worth its plumbing.
ValueColumn valueColumnFor(
    List<TimelineGroup> order, Map<TimelineGroup, double> widths) {
  // The value cells span the render group *as it is now*, so dragging that
  // group wider widens the fields under it.
  return ValueColumn(widths[TimelineGroup.render] ?? renderGroupWidth,
      rightInsetOf(order, widths, TimelineGroup.render));
}

/// Where a fold-out row puts its render-time readout so it sits under the
/// timings column's header, whatever order the groups have been dragged into
/// (docs/13 §7.1).
ValueColumn timingsColumnFor(
    List<TimelineGroup> order, Map<TimelineGroup, double> widths) {
  // Width zero when the column is not being drawn at all (nothing is being
  // measured): a fold row asks this before it reserves any space, so an effect
  // heading goes back to its old shape rather than keeping an empty cell.
  if (!order.contains(TimelineGroup.timings)) return const ValueColumn(0, 0);
  return ValueColumn(widths[TimelineGroup.timings] ?? timingsGroupWidth,
      rightInsetOf(order, widths, TimelineGroup.timings));
}

/// Where the identity group (and so the layer's own twirl) starts, in the
/// current order — the fold-out rows hang their indent off this, so a
/// property's twirl sits just inside its layer's (docs/07 §4.3).
double identityStart(
    List<TimelineGroup> order, Map<TimelineGroup, double> widths) {
  var x = 4.0; // the row's edge padding
  for (final group in order) {
    if (group == TimelineGroup.identity) return x;
    x += (widths[group] ?? 0) + groupDividerWidth;
  }
  return x;
}
