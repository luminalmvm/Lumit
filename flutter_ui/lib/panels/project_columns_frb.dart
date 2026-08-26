// The Project panel's measurements, its columns, and the small value helpers
// the panel, its rows and its menu all read.
//
// **In plain terms**: this is the ruler and the tape measure. It holds the
// numbers the mockup gave the panel — how tall a row is, how wide each column
// starts — the arithmetic that turns a panel width into a set of columns, and
// the handful of "write this number the way the mockup writes it" helpers the
// three other files share.
//
// Every measurement here comes from the mockups' *computed* styles — the
// browser's own resolved numbers, not a reading of the CSS — so the comments
// quote real pixels rather than intentions. Where a number disagreed with
// docs/15-DESIGN.md §12A.6's table the mockup won and the table was corrected
// in the same commit (K-450, K-454).

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';

// ---------------------------------------------------------------------------
// The mockups' own metrics. Every constant below is a measured value from
// `ProjectPanel` (the panel's own artboard, 360 wide) or `Main` (the same panel
// docked at 260), and the two agree everywhere they overlap.
// ---------------------------------------------------------------------------

/// The column-header row: a **secondary row**, and so a density token
/// (K-454) — 19 under Regular, where the hairline beneath it is counted in,
/// and 18 under Compact.
double projectColumnHeaderHeight(LumitTheme t) => t.density.secondaryRow;

/// One item row. The mockup draws the Project panel's rows without the seam
/// the Timeline's outline carries, so this is a plain 22 under both densities
/// (§12A.6) rather than the lane row's token.
const double projectRowHeight = 22;

/// The metadata columns, right-anchored, in the widths the header and every
/// row share — which is what makes a value sit under its own heading.
const double projectItemsColumn = 36;
const double projectSizeColumn = 64;
const double projectFpsColumn = 22;

/// The Path column, at the list's right (§12A.3a). **Its starting width**, not
/// its width: Path is the column the panel's spare room goes to (owner, desk
/// test), so 40 is the narrowest it is ever drawn and every pixel the panel
/// gains past the mockup's own 360 lands here. It earns that room more than
/// any other column — it is the one value longer than its column by nature,
/// and the one that elides rather than clipping.
const double projectPathColumn = 40;

/// The width the Name column settles at. **Name no longer stretches** (owner,
/// desk test) — it was the flexible column and Path the fixed one, which is
/// backwards: a name with room to spare gains nothing by having more, while a
/// path is cut short at every width. So Name keeps this size and Path takes
/// the slack.
///
/// **Why 220 rather than the 148 the mockup's 360 artboard resolves to.** That
/// 148 is what a *flexible* Name was left with once the columns had taken
/// theirs — it was never a width anybody chose, and it was measured on a
/// drawing whose rows wear no badges. A row does: `in use`, `proxy` and
/// `missing` all sit between the name and the columns, and the whole badge run
/// comes to about 150. Freezing Name at 148 and handing every spare pixel to
/// Path left a badged row with no name at all and a row that overflowed its
/// panel. 220 is the mockup's own column with a badge run's room added to it.
///
/// Below the width this arrangement asks for, Name is still the one that
/// gives — it is the row's flexible slot — so the 360 artboard draws exactly
/// as it did, at 148.
const double projectNameColumn = 220;

/// The panel's columns, left to right. The seams between their headings drag,
/// the way the Timeline outline's group seams do
/// (`state/timeline_columns.dart`): a seam widens the column to its left and
/// leaves the rest alone.
///
/// Session-lived, like the Timeline's group widths — nothing writes a column
/// width to the settings file, and the two panels answer this question the
/// same way.
enum ProjectColumn { name, items, size, fps, path }

/// Each column's width before anybody has dragged it.
const Map<ProjectColumn, double> defaultProjectColumnWidths = {
  ProjectColumn.name: projectNameColumn,
  ProjectColumn.items: projectItemsColumn,
  ProjectColumn.size: projectSizeColumn,
  ProjectColumn.fps: projectFpsColumn,
  ProjectColumn.path: projectPathColumn,
};

/// **Items and fps are fixed**, on the Timeline's own rule
/// ([groupIsFixedWidth]): a column whose cells cannot use more room buys only
/// blank space by being widened, so the seam beside it is not a handle at all.
/// A count of children and a frame rate are both as wide as the number they
/// write. Name and Size each hold something that gains from more room, and
/// Path takes whatever is left over, so it has no width of its own to drag.
bool projectColumnIsFixedWidth(ProjectColumn column) =>
    column == ProjectColumn.items ||
    column == ProjectColumn.fps ||
    column == ProjectColumn.path;

/// How narrow a column may be dragged: enough for what cannot shrink.
double minProjectColumnWidth(ProjectColumn column) => switch (column) {
      // **Enough for everything else a row keeps in this column.** The twirl,
      // the type mark and their gaps come to 42, and the badge run — `in use`,
      // `proxy`, `missing` — to about 150 more. Narrower than this and the
      // slack handed to Path would push a badged row's own marks off the right
      // of the panel, which is what a drag past here used to do.
      ProjectColumn.name => 200,
      ProjectColumn.items => projectItemsColumn,
      // Enough for the widest size the cell writes ("3840x2160" clipped to its
      // tail is still a reading).
      ProjectColumn.size => 40,
      ProjectColumn.fps => projectFpsColumn,
      ProjectColumn.path => projectPathColumn,
    };

/// The gap between every element in a row, headers included.
const double projectRowGap = 8;

/// What a glyph in this panel draws at (K-456). The set is still drawn on the
/// 16 grid — that is the drawing grid, not the display size — and each panel
/// renders it at the size its own mockup computed: **13 in the tree's rows,
/// 14 on the bottom bar**. The slight softening of a 1.5-unit stroke at these
/// sizes is the mockup's own look, and is accepted as such.
///
/// The twirl's slot takes the row size too, so the twirl, the type glyph and
/// the name stay one cluster: shrinking one of the two and not the other would
/// leave the names 3px out of the mockup's columns.
const double projectRowIconSize = 13;

/// A row's own inset. The header's left is 10 rather than 8: the heading words
/// stand a touch in from the twirls below them, as the mockup draws it.
const double projectRowPadding = 8;
const double projectHeaderPadLeft = 10;

/// How far each nesting level indents a row — the mockup's 24 for a child
/// against 8 for its parent.
const double projectIndentPerDepth = 16;

/// Below this the panel scrolls sideways rather than degrading further
/// (§12A.6's ladder, step 5).
const double projectMinWidth = 180;

/// Where each optional column gives way (§12A.6's ladder, step 3: metadata
/// columns hide, least essential first). The two mockups are the two ends of
/// this ladder — the 360-wide artboard shows everything, the 260-wide docked
/// panel has already dropped the preview card and the Items column.
/// The preview card, the Path column and the Items column leave **together**,
/// at the first step down from the 360 artboard.
///
/// §12A.3a lists them one after the other, but the two mockups are the only
/// measurements there are — 360 wide shows all three, 260 shows none — so
/// nothing renders a width between them and a third step would be a drawing
/// nobody approved. The number inside that gap is set by what a *row* can
/// carry rather than by what the header can: a row wears badges as well as
/// columns, and four columns plus an `in use` pill need more room than the
/// header alone does. Below this the row keeps its badges and sheds the two
/// least essential columns, which is §12A.6's ladder in the order it asks for.
const double _widthForPath = 340;
const double _widthForItems = 340;
const double _widthForFps = 230;
const double _widthForSize = 190;

/// Which optional columns a given panel width can carry, how wide each of them
/// has been dragged, and — because the two answers cannot be given apart —
/// where the Name column ends.
class ProjectColumns {
  final bool items;
  final bool size;
  final bool fps;
  final bool path;

  /// Each column's dragged width. Absent falls back to the default, so a map
  /// only has to carry what somebody actually moved.
  final Map<ProjectColumn, double> widths;

  /// The panel width these were worked out for. Kept because the Name column
  /// is capped by what the metadata columns leave — see [nameWidth].
  final double panelWidth;

  const ProjectColumns({
    required this.items,
    required this.size,
    required this.fps,
    required this.path,
    required this.panelWidth,
    this.widths = defaultProjectColumnWidths,
  });

  factory ProjectColumns.forWidth(
    double width, {
    Map<ProjectColumn, double> widths = defaultProjectColumnWidths,
  }) =>
      ProjectColumns(
        items: width >= _widthForItems,
        size: width >= _widthForSize,
        fps: width >= _widthForFps,
        path: width >= _widthForPath,
        panelWidth: width,
        widths: widths,
      );

  double widthOf(ProjectColumn column) =>
      widths[column] ?? defaultProjectColumnWidths[column]!;

  bool shows(ProjectColumn column) => switch (column) {
        ProjectColumn.name => true,
        ProjectColumn.items => items,
        ProjectColumn.size => size,
        ProjectColumn.fps => fps,
        ProjectColumn.path => path,
      };

  /// The columns drawn at this width, in order.
  List<ProjectColumn> get visible => [
        for (final c in ProjectColumn.values)
          if (shows(c)) c
      ];

  /// **The panel's spare width goes to the last column** (owner, desk test).
  /// Normally that is Path; at the widths where the ladder has already dropped
  /// Path it is whichever metadata column is still standing, so the values
  /// stay anchored to the panel's right edge as they always have.
  ///
  /// Null when no metadata column is drawn at all: there is nothing but Name,
  /// and Name is still the row's flexible slot, so the room is already its.
  ProjectColumn? get stretching {
    final drawn = visible.where((c) => c != ProjectColumn.name);
    return drawn.isEmpty ? null : drawn.last;
  }

  /// The width the metadata columns are laid out at: each column's own width,
  /// except the last, which is given every pixel the panel has past the
  /// mockup's own arrangement.
  ///
  /// **Name is still the row's `Expanded` slot, and that is the whole trick.**
  /// Everything to its right is a fixed box, so what Name is left with is the
  /// panel width less all of them — and because the last box swallows the
  /// slack, that remainder settles at [projectNameColumn] and stays there
  /// however wide the panel grows. Narrow the panel past it and the arithmetic
  /// runs the other way with nothing extra to write: the last column is
  /// already at its minimum, and Name gives way exactly as it used to, taking
  /// a row's indent and its badges with it.
  double laidOutWidth(ProjectColumn column) =>
      widthOf(column) + (column == stretching ? _slack : 0);

  /// The width the panel has beyond the arrangement its columns ask for. Never
  /// negative: below that the columns are at their widths and Name is short.
  double get _slack {
    var wanted =
        projectHeaderPadLeft + widthOf(ProjectColumn.name) + projectRowPadding;
    for (final column in visible) {
      if (column == ProjectColumn.name) continue;
      wanted += projectRowGap + widthOf(column);
    }
    return math.max(0, panelWidth - wanted);
  }

  /// The trailing cells of a row or of the header — the same widths, the same
  /// gaps, the same right edge, so a value lands under its heading. The owner
  /// corrected this alignment twice in the mockup rounds; building both sides
  /// from one function is what stops it drifting again.
  ///
  /// [pathStyle] is separate because the Path column is quieter than the rest
  /// (§12A.3a: `text_disabled`), and because it is the one column that elides
  /// rather than clipping — a path is longer than its column by nature, and an
  /// ellipsis says so where a hard cut looks like the value ending there.
  ///
  /// [seam] builds the gap that precedes a column's cell. The header passes a
  /// drag handle — a seam resizes the column to its *left*, as the Timeline's
  /// does — and the rows pass nothing, so they keep the plain [projectRowGap]
  /// the header's handle is drawn inside. Both sides therefore reserve the
  /// same width and stay column-aligned.
  List<Widget> cells({
    String? items,
    String? size,
    String? fps,
    String? path,
    required TextStyle style,
    TextStyle? pathStyle,
    Widget Function(ProjectColumn before)? seam,
  }) =>
      [
        for (final column in visible)
          if (column != ProjectColumn.name)
            ..._cell(
              column,
              seam?.call(column) ?? const SizedBox(width: projectRowGap),
              switch (column) {
                ProjectColumn.items => items,
                ProjectColumn.size => size,
                ProjectColumn.fps => fps,
                ProjectColumn.path => path,
                ProjectColumn.name => null,
              },
              column == ProjectColumn.path ? pathStyle ?? style : style,
              overflow: column == ProjectColumn.path
                  ? TextOverflow.ellipsis
                  : TextOverflow.clip,
            ),
      ];

  List<Widget> _cell(
    ProjectColumn column,
    Widget gap,
    String? text,
    TextStyle style, {
    TextOverflow overflow = TextOverflow.clip,
  }) {
    return [
      gap,
      SizedBox(
        width: laidOutWidth(column),
        child: text == null
            ? null
            : Text(text,
                style: style,
                // **Path reads from its own left edge; every other column
                // reads from its right** (owner, desk test). The metadata
                // columns are fixed boxes whose values are numbers, and a
                // number belongs against the column's right edge. Path is the
                // one box that grows with the panel, and a value anchored to
                // the right of a growing box travels with it — which is
                // exactly the "Path's column still shifts" the owner kept
                // reading as the panel was widened. Anchored left, its heading
                // and its values stand still and only the room after them
                // grows, which is what "only Path stretches" has to look like.
                textAlign: column == ProjectColumn.path
                    ? TextAlign.left
                    : TextAlign.right,
                maxLines: 1,
                overflow: overflow),
      ),
    ];
  }
}

/// The mono face every column value and every meta line is set in: 10px, muted
/// (§7.1's mono-for-numbers rule, at the mockup's own size).
TextStyle projectMetaStyle(LumitTheme t) =>
    t.mono.copyWith(fontSize: 10, color: t.textMuted);

/// The em dash a column shows when the item cannot answer — a missing file has
/// no size and no rate, and the mockup writes the dash rather than a blank.
const String projectNoValue = '—';

/// A rate as the mockup writes it: bare integers where the rate is whole, two
/// places where it is not, and never a trailing `.00`.
String projectRateText(int num, int den) {
  if (den == 0) return projectNoValue;
  final fps = num / den;
  final rounded = fps.roundToDouble();
  return (fps - rounded).abs() < 0.001
      ? rounded.toInt().toString()
      : fps.toStringAsFixed(2);
}

/// One row's column values, worked out once by the panel's walk so the row
/// itself never asks the engine anything.
class ProjectCells {
  final String? items;
  final String? size;
  final String? fps;
  final String? path;
  const ProjectCells({this.items, this.size, this.fps, this.path});
}

/// A sound's channel layout in the words the preview card uses. Two names for
/// the two counts anyone recognises, and a bare count for the rest — "6 ch"
/// says more than "hexaphonic" ever would.
String projectChannelText(int channels) => switch (channels) {
      1 => l10n.audioMono,
      2 => l10n.audioStereo,
      _ => l10n.audioChannels(channels),
    };

/// A sample rate as the mockup writes it: `48 kHz`, and `44.1 kHz` where the
/// rate is not a whole number of thousands.
String projectSampleRateText(int hz) {
  final khz = hz / 1000;
  final rounded = khz.roundToDouble();
  final text = (khz - rounded).abs() < 0.001
      ? rounded.toInt().toString()
      : khz.toStringAsFixed(1);
  return '$text ${l10n.unitKhz}';
}

/// An item's id as a string, for keys and selection.
///
/// The generated references expose their ids under `internalid`; this is the one
/// place that name appears, so a future rename of the frb field is a one-line
/// change here rather than a sweep.
String projectItemId(ItemReference item) => switch (item) {
      ItemReference_Footage(:final field0) => field0.internalid.toString(),
      ItemReference_Solid(:final field0) => field0.internalid.toString(),
      ItemReference_Composition(:final field0) => field0.internalid.toString(),
      ItemReference_Folder(:final field0) => field0.internalid.toString(),
    };
