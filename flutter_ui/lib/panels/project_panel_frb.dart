// The Project panel, built to the approved redesign mockup (K-451, K-454).
//
// **In plain terms**: this is the shelf the project's things live on. Top to
// bottom the mockup lays it out as a preview card for whatever is picked, a
// search well, a row of column headings, the tree of items, a thin horizontal
// scrollbar, and a bottom bar carrying the new-item controls at the left and a
// factual count at the right.
//
// Every measurement here comes from the mockups' *computed* styles — the
// browser's own resolved numbers, not a reading of the CSS — so the comments
// quote real pixels rather than intentions. Where a number disagreed with
// docs/15-DESIGN.md §12A.6's table the mockup won and the table was corrected
// in the same commit (K-450, K-454): the panel's bottom bar is 20, not the 18
// a secondary row usually gets, and the column-header row is 19 because its
// hairline is counted inside it.
//
// **Behaviour is unchanged from the panel this replaces.** A click selects the
// instant the button goes down; a click on the lone selected row *opens* it,
// which makes a double-click "select, then open" in one motion, and what
// opening means is the item's own answer (K-243): a comp fronts, footage
// raises New composition on it, a folder shows or hides its children. Renaming
// is `Enter` or the row menu. A right-click raises that menu; footage and comp
// rows drag onto the Timeline (a comp lands as a Precomp layer);
// double-clicking empty space imports. Missing footage wears the mockup's
// `missing` badge, and that badge *is* the relink control — clicking it opens
// the file picker, which is where the old inline "Relink…" button's job went.
// The "show only missing" filter moved onto the bottom bar's count, which
// reads `1 missing · 10 items`: the owner's order, with the total hard right
// where the eye looks for it and the broken half beside it.
//
// **What the panel reads and when.** The handles *are* the identity: a row
// holds an `ItemReference` and calls `rename`/`delete`/`moveToRoot` straight on
// it. Everything a row *draws* — its name, its column values, its badge — is
// handed to it by the panel's own walk, and every engine answer that walk needs
// (a status probe, a media probe, a comp's settings, a folder's child count) is
// cached until the document changes. That is what keeps a hover, which rebuilds
// one row, costing nothing at the bridge (the budget test expects zero).

import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/drag_payloads.dart';
import '../shell/comp_settings_frb.dart';
import '../shell/status_line_frb.dart';
import '../state/file_dialogs.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

/// The longer edge the preview card's thumbnail is decoded at: the card draws
/// it 96 logical px wide, so ~2× for crispness on a high-DPI display.
const int _thumbMaxEdge = 224;

// ---------------------------------------------------------------------------
// The mockups' own metrics. Every constant below is a measured value from
// `ProjectPanel` (the panel's own artboard, 360 wide) or `Main` (the same panel
// docked at 260), and the two agree everywhere they overlap.
// ---------------------------------------------------------------------------

/// The preview card: 10px of padding round a 96×54 poster frame, with the
/// hairline under it counted in — 10 + 54 + 10 + 1.
const double projectPreviewHeight = 75;
const double _previewPad = 10;
const double _thumbWidth = 96;
const double _thumbHeight = 54;

/// The gap between the card's three text lines.
const double _previewLineGap = 3;

/// The search row: 8 above the well, 6 under it, and the well itself the 20 a
/// value well is everywhere (§12A.6).
const double projectSearchRowHeight = 34;
const double _searchPadTop = 8;
const double _searchPadBottom = 6;

/// The column-header row: a **secondary row**, and so a density token
/// (K-454) — 19 under Regular, where the hairline beneath it is counted in,
/// and 18 under Compact.
double projectColumnHeaderHeight(LumitTheme t) => t.density.secondaryRow;

/// One item row. The mockup draws the Project panel's rows without the seam
/// the Timeline's outline carries, so this is a plain 22 under both densities
/// (§12A.6) rather than the lane row's token.
const double projectRowHeight = 22;

/// The state badge ("missing"): 14 tall, 4px of padding either side, its text
/// mono at 9 with no tracking — a badge is not a container label, so it is not
/// a kicker however small it is.
const double _badgeHeight = 14;
const double _badgePad = 4;
const double _badgeTextSize = 9;

/// The badge's outline is its own text colour, hushed: the mockup's border
/// resolves to that colour at 28% over the panel, on both badges.
const double _badgeBorderAlpha = 0.28;

/// The horizontal scrollbar strip under the tree: a 6px strip with a 4px track
/// inset 8px either side.
const double projectScrollStripHeight = 6;
const double _scrollTrackHeight = 4;
const double _scrollTrackInset = 8;

/// The bottom bar. **20, not the 18 a secondary row usually gets** — the
/// mockup renders it at 20 and K-454 makes the mockup's own breathing room the
/// default density, so §12A.6's table gained a project-panel line rather than
/// this bar being shaved to fit the old one.
const double projectFooterHeight = 20;
const double _footerPad = 10;
const double _footerGap = 12;
const double _footerIconGap = 5;

/// The bottom bar's two label trackings, in logical pixels at 9px type: the
/// new-item words sit at 0.08em, the count at 0.06em — both quieter than the
/// 0.12em a kicker carries, because neither is naming a container.
const double _footerLabelTracking = 0.72;
const double _footerCountTracking = 0.54;

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

/// The colour-chip filter beside the search well (§12A.3a): the mockup's six
/// 6px dots, 3px apart, in a strip padded 4px either side.
const double _chipSize = 6;
const double _chipGap = 3;
const double _chipStripPad = 4;

/// The search row's own gap, between the well and the chip strip — the
/// mockup's `gap: 6` on that flex line.
const double _searchGap = 6;

/// The label chips the filter row offers, as palette indices, in the mockup's
/// own order — azure, mint, amber, violet, coral. The sixth chip is not a
/// colour: it is the neutral one that clears the filter, so the row can always
/// be got out of.
const List<int> projectFilterLabels = [1, 4, 2, 3, 8];

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
const double projectFooterIconSize = 13;

/// A row's own inset. The header's left is 10 rather than 8: the heading words
/// stand a touch in from the twirls below them, as the mockup draws it.
const double projectRowPadding = 8;
const double _headerPadLeft = 10;

/// How far each nesting level indents a row — the mockup's 24 for a child
/// against 8 for its parent.
const double projectIndentPerDepth = 16;

/// Below this the panel scrolls sideways rather than degrading further
/// (§12A.6's ladder, step 5).
const double projectMinWidth = 180;

/// Where each optional piece gives way (§12A.6's ladder, step 3: metadata
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
const double _widthForPreview = 340;
const double _widthForPath = 340;
const double _widthForItems = 340;
const double _widthForFps = 230;
const double _widthForSize = 190;

/// Where the bottom bar's words go (§12A.6's ladder, step 4: a toolbar sheds
/// rather than shrinks). **Two steps, because this bar carries three controls
/// where the mockup drew two.** Import's word goes first — it is the one the
/// mockup gives no place at all, so it is the one with least claim on the room
/// — and Folder's and Composition's go together below that, since those two
/// are the mockup's own and should read as a pair for as long as they fit.
const double _widthForImportLabel = 420;
const double _widthForFooterLabels = 380;

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
        _headerPadLeft + widthOf(ProjectColumn.name) + projectRowPadding;
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
                textAlign: TextAlign.right,
                maxLines: 1,
                overflow: overflow),
      ),
    ];
  }
}

/// The handle between two column headings: it resizes the column to its left,
/// and everything else keeps its width (docs/07 §4.2's rule for the Timeline
/// outline, which this mirrors). It is drawn *inside* the gap the rows already
/// carry between their cells, so adding it moves no column.
///
/// A hairline marks the ones that take hold; a seam beside a fixed-width
/// column draws nothing and offers no resize cursor, so the panel never shows
/// a handle that does not work.
class _ColumnSeam extends StatelessWidget {
  final ValueChanged<double>? onResize;
  const _ColumnSeam({super.key, required this.onResize});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final resize = onResize;
    if (resize == null) return const SizedBox(width: projectRowGap);
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onHorizontalDragUpdate: (d) => resize(d.delta.dx),
        child: SizedBox(
          width: projectRowGap,
          child: Center(
            child: Container(width: 1, height: 10, color: t.hairlineStrong),
          ),
        ),
      ),
    );
  }
}

/// What a click on a row does to the selection.
enum SelectMode {
  /// Plain click: this row, and only this row.
  replace,

  /// `Ctrl` (or `Cmd`): add this row, or drop it if it was already in.
  toggle,

  /// `Shift`: every row between the anchor and this one.
  range,
}

/// The modifier held right now, as a selection rule. Read from the keyboard
/// rather than carried on the tap because `GestureDetector.onTap` does not
/// report modifiers.
SelectMode _selectModeFromKeyboard() {
  final keys = HardwareKeyboard.instance.logicalKeysPressed;
  bool down(LogicalKeyboardKey a, LogicalKeyboardKey b) =>
      keys.contains(a) || keys.contains(b);
  if (down(LogicalKeyboardKey.shiftLeft, LogicalKeyboardKey.shiftRight)) {
    return SelectMode.range;
  }
  if (down(LogicalKeyboardKey.controlLeft, LogicalKeyboardKey.controlRight) ||
      down(LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.metaRight)) {
    return SelectMode.toggle;
  }
  return SelectMode.replace;
}

/// The mono face every column value and every meta line is set in: 10px, muted
/// (§7.1's mono-for-numbers rule, at the mockup's own size).
TextStyle _metaStyle(LumitTheme t) =>
    t.mono.copyWith(fontSize: 10, color: t.textMuted);

/// The em dash a column shows when the item cannot answer — a missing file has
/// no size and no rate, and the mockup writes the dash rather than a blank.
const String _noValue = '—';

/// A rate as the mockup writes it: bare integers where the rate is whole, two
/// places where it is not, and never a trailing `.00`.
String _rateText(int num, int den) {
  if (den == 0) return _noValue;
  final fps = num / den;
  final rounded = fps.roundToDouble();
  return (fps - rounded).abs() < 0.001
      ? rounded.toInt().toString()
      : fps.toStringAsFixed(2);
}

/// One row's column values, worked out once by the panel's walk so the row
/// itself never asks the engine anything.
class _Cells {
  final String? items;
  final String? size;
  final String? fps;
  final String? path;
  const _Cells({this.items, this.size, this.fps, this.path});
}

/// A sound's channel layout in the words the preview card uses. Two names for
/// the two counts anyone recognises, and a bare count for the rest — "6 ch"
/// says more than "hexaphonic" ever would.
String _channelText(int channels) => switch (channels) {
      1 => l10n.audioMono,
      2 => l10n.audioStereo,
      _ => l10n.audioChannels(channels),
    };

/// A sample rate as the mockup writes it: `48 kHz`, and `44.1 kHz` where the
/// rate is not a whole number of thousands.
String _sampleRateText(int hz) {
  final khz = hz / 1000;
  final rounded = khz.roundToDouble();
  final text = (khz - rounded).abs() < 0.001
      ? rounded.toInt().toString()
      : khz.toStringAsFixed(1);
  return '$text ${l10n.unitKhz}';
}

class ProjectPanelFrb extends StatefulWidget {
  /// The relink file picker seam (chosen path, or null when cancelled). Defaults
  /// to the real footage picker; tests inject their own so no plugin channel
  /// opens.
  final Future<String?> Function()? relinkPicker;

  /// The import picker seam, for the bottom bar's button and the double-click.
  /// Same reason: a widget test must never open a plugin channel.
  final Future<List<String>> Function()? importPicker;

  const ProjectPanelFrb({super.key, this.relinkPicker, this.importPicker});

  @override
  State<ProjectPanelFrb> createState() => _ProjectPanelFrbState();
}

class _ProjectPanelFrbState extends State<ProjectPanelFrb> {
  bool _missingOnly = false;

  /// The live search needle (docs/07 §3.1): lowercase, empty means "show all".
  final TextEditingController _searchController = TextEditingController();
  String _search = '';

  /// The search field's focus, owned here so `Ctrl+F` can put the cursor in it
  /// (docs/07 §15, "Panels").
  final FocusNode _searchFocus = FocusNode();

  /// The sideways scroll the width ladder's last step rides on (§12A.6): below
  /// [projectMinWidth] the tree keeps its width and slides instead of shrinking
  /// any further.
  final ScrollController _hScroll = ScrollController();

  /// The shell state this panel is listening to, so the listener can be taken
  /// off the same object it was put on.
  LumitUiState? _boundUi;

  StreamSubscription<ScopedChange>? _changes;

  @override
  void initState() {
    super.initState();
    // Edits made ELSEWHERE reach us here — the menu bar, an undo, another panel.
    // That is the point of the scoped-change stream: no panel has to be told
    // about an edit it did not make, and none has to poll.
    //
    // Only `items` changes concern this panel. Rebuilding on every change meant a
    // layer tweak in the Timeline dropped the whole missing-media cache and
    // re-probed every footage file on disk — see `op_scope` in api/state.rs.
    //
    // This panel's own edits do *not* wait for the round trip; each calls
    // `_documentChanged` directly. Waiting would put a Rust→Dart hop between a
    // click and the row updating, for information this panel already had — and it
    // would make the panel untestable without real async, since a fake-async test
    // never delivers an FFI stream event.
    final state = Provider.of<LumitState>(context, listen: false);
    _changes = state.onChange.listen((event) {
      if (event.items) _documentChanged();
    });
    _searchController.addListener(() {
      final needle = _searchController.text.trim().toLowerCase();
      if (needle != _search) setState(() => _search = needle);
    });
    // Enter renames the lone selected item (K-321) — the same key the
    // Timeline gives its layers. Registered on the hardware keyboard like the
    // Timeline's commands; the handler stands down for modals, focused
    // fields, and whenever this panel is not the active one.
    HardwareKeyboard.instance.addHandler(_onKey);
    // `Ctrl+F` puts the cursor in the search field (docs/07 §15). The shell
    // asks rather than reaching in, and this answers only while the Project
    // panel is the focused one — the Effects & presets panel answers the same
    // request for its own field.
    _boundUi = Provider.of<LumitUiState>(context, listen: false);
    _boundUi!.panelSearchRequest.addListener(_onSearchRequested);
    // `Ctrl+A` selects every item this panel is showing (K-522), asked for the
    // same way the search focus is: the shell routes the chord to whichever
    // panel is focused rather than deciding what "everything" means itself.
    _boundUi!.selectAllRequest.addListener(_onSelectAllRequested);
  }

  void _onSearchRequested() {
    if (!mounted) return;
    if (_boundUi?.searchRequestIsFor(Panel.project) ?? false) {
      _searchFocus.requestFocus();
    }
  }

  /// Every row currently listed — which is every row the *search and the label
  /// filter* leave, and the open folders' children only. Selecting rows that
  /// are not on screen would be selecting things the user cannot see.
  void _onSelectAllRequested() {
    if (!mounted) return;
    if (!(_boundUi?.selectAllRequestIsFor(Panel.project) ?? false)) return;
    if (_visibleIds.isEmpty) return;
    setState(() {
      _selectedIds
        ..clear()
        ..addAll(_visibleIds);
      _anchorId = _visibleIds.last;
    });
    _publishSelection();
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _boundUi?.panelSearchRequest.removeListener(_onSearchRequested);
    _boundUi?.selectAllRequest.removeListener(_onSelectAllRequested);
    _changes?.cancel();
    _searchController.dispose();
    _searchFocus.dispose();
    _hScroll.dispose();
    _dropThumbs();
    super.dispose();
  }

  /// The Project panel's keyboard commands — just `item.rename` today.
  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent || !mounted) return false;
    if (lumitModalOpen) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // A per-panel binding is live in the *focused* panel (docs/07 §15); this
    // handler hears every key wherever it lands, so the panel checks that it
    // is the active one itself.
    if (ui.activePanel.value != Panel.project) return false;
    final action = ui.keymap.actionFor(BridgeKeyContext.project, event);
    if (action == 'item.rename') {
      if (_selectedIds.length != 1 || _renamingId != null) return false;
      setState(() => _renamingId = _selectedIds.first);
      return true;
    }
    return false;
  }

  /// The items currently selected, by id, in the order the panel lists them.
  /// Held here rather than in `LumitUiState` because nothing outside this panel
  /// reads the full set — only the anchor item is published, for the FX
  /// console (K-327), through [_publishSelection].
  ///
  /// A set rather than one id because more than one row can be picked:
  /// `Ctrl`-click adds or removes one, `Shift`-click takes the run between the
  /// last click and this one, and a plain click goes back to just that row — the
  /// selection rules every file list has. Multi-selection is what lets several
  /// clips be dropped on the Timeline, or on New composition, in one gesture.
  final Set<String> _selectedIds = {};

  /// The row a `Shift`-click measures its run from — the last one clicked
  /// without `Shift`.
  String? _anchorId;

  /// Every row id currently drawn, top to bottom, so a `Shift`-click knows what
  /// "between these two" means. Rebuilt with the rows.
  final List<String> _visibleIds = [];

  /// The footage handle behind each row, so a drag can carry the whole selection
  /// without walking the tree again. Rebuilt with the rows.
  final Map<String, FootageReference> _footageById = {};

  /// Every item drawn this build, by id — what the preview card looks the
  /// anchor up in. Rebuilt with the rows.
  final Map<String, ItemReference> _itemById = {};

  /// Decoded media facts per footage id, for the preview card and the Size and
  /// fps columns. Cached because `mediaInfo` probes the file; cleared with the
  /// epoch.
  final Map<String, BridgeMediaInfo?> _mediaInfo = {};

  /// A composition's own size and rate, for the same two columns. Comp settings
  /// are a synchronous read, so the cache is what keeps them off every rebuild
  /// rather than off the build thread.
  final Map<String, _Cells> _compCells = {};

  /// How many things each folder holds, for the Items column — including the
  /// folders that are shut, whose children the walk would otherwise never ask
  /// about.
  final Map<String, int> _childCounts = {};

  /// Where each footage item's file is, for the Path column. Cached with the
  /// rest: it is document data, so it can only change when the document does.
  final Map<String, String> _paths = {};

  /// Which items a composition places, for the `in use` badge, and each item's
  /// colour tag, for the row-icon tint and the chip filter. Both are one walk
  /// of the document engine-side, so they are asked once per document change
  /// and never in a rebuild.
  final Map<String, bool> _used = {};
  final Map<String, int> _labels = {};

  /// Each footage item's proxy (K-501), or null where it has none — for the
  /// `proxy` badge and for the row menu's four commands. A document read like
  /// every other entry here, so it is asked once per document change and never
  /// in a rebuild: the budget test expects a hover to cost nothing.
  final Map<String, BridgeProxy?> _proxies = {};

  /// The project-wide *use proxies* switch (K-501), cached with the rest: it is
  /// a document read, so a rebuild must never ask for it again.
  bool? _useProxies;

  /// The colour chip the filter row is holding, or null for "show everything"
  /// (§12A.3a). Session state, like the search text and the shut folders: a
  /// filter is where you are looking, not something about the document.
  int? _labelFilter;

  /// Decoded poster frames by footage id, held in RAM for the session so the
  /// preview card never re-decodes for a selection change. A null entry claims
  /// the slot while the decode is in flight (or records that the item has no
  /// picture to give). Cleared — and every image disposed — with the epoch.
  final Map<String, ui.Image?> _thumbs = {};

  /// The selected footage, in the order the panel lists it. Anything selected
  /// that is not footage — a folder, a comp — is simply not part of a drag.
  List<FootageReference> get _selectedFootage => [
        for (final id in _visibleIds)
          if (_selectedIds.contains(id) && _footageById[id] != null)
            _footageById[id]!,
      ];

  /// Apply a click to the selection.
  void _select(String id, SelectMode mode) {
    setState(() {
      switch (mode) {
        case SelectMode.replace:
          _selectedIds
            ..clear()
            ..add(id);
          _anchorId = id;
        case SelectMode.toggle:
          if (!_selectedIds.remove(id)) _selectedIds.add(id);
          _anchorId = id;
        case SelectMode.range:
          final from = _visibleIds.indexOf(_anchorId ?? id);
          final to = _visibleIds.indexOf(id);
          if (from < 0 || to < 0) {
            _selectedIds.add(id);
            return;
          }
          // The anchor stays put, so widening and narrowing the run with
          // repeated Shift-clicks both work.
          _selectedIds
            ..clear()
            ..addAll(_visibleIds.sublist(
              from < to ? from : to,
              (from < to ? to : from) + 1,
            ));
      }
    });
    _publishSelection();
  }

  /// Mirror the anchor item to the shell (K-327), where the FX console reads
  /// it. The anchor, not the set: the console acts on one thing, the way the
  /// preview card describes one thing. Deselected (a toggle off) or unknown
  /// (a stale id after a delete) publishes null rather than a dead handle.
  void _publishSelection() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    ui.selectedProjectItem.value =
        _selectedIds.contains(_anchorId) ? _itemById[_anchorId] : null;
  }

  /// The row being renamed in place, by id.
  String? _renamingId;

  /// Each column's width. Dragging a header seam changes one of these and
  /// leaves the rest alone, so the panel's columns move by exactly what the
  /// drag moved — the Timeline outline's own rule (`_resizeGroup`).
  ///
  /// Session-lived, like the Timeline's group widths and like this panel's own
  /// twirls: nothing writes a column width to the settings file.
  Map<ProjectColumn, double> _columnWidths = {...defaultProjectColumnWidths};

  /// Widen (or narrow) one column, never below what its cells need — and not
  /// at all for a column that has no width of its own to change.
  void _resizeColumn(ProjectColumn column, double delta) => setState(() {
        if (projectColumnIsFixedWidth(column)) return;
        final next = ((_columnWidths[column] ?? 0) + delta)
            .clamp(minProjectColumnWidth(column), 900.0);
        _columnWidths = {..._columnWidths, column: next};
      });

  /// The column a seam belongs to: the one drawn immediately before [column].
  ProjectColumn _leftOf(ProjectColumns cols, ProjectColumn column) {
    final order = cols.visible;
    return order[order.indexOf(column) - 1];
  }

  /// The folders the user has shut, by id (K-243). Closed rather than open, so
  /// a project opens showing everything it has — which is what the panel did
  /// before folders could be shut at all. Session state, like the search text:
  /// a twirl is where you are looking, not something about the document.
  final Set<String> _closedFolders = {};

  /// Which footage items failed to resolve, by id.
  ///
  /// Cached because `getStatus` probes the file, which is far too slow to do in a
  /// build — and because the missing-only filter has to know every item's status
  /// at once to decide what to draw. Dropped when the item list changes, and only
  /// then: a probe of every footage file is far too expensive to repeat because
  /// someone nudged a layer value.
  final Map<String, bool> _missing = {};

  /// Bumped whenever the document changes, to key the thumbnail futures so a
  /// relink re-decodes rather than showing the stale picture. The frb equivalent
  /// of v0's `documentEpoch`.
  int _epoch = 0;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
        builder: (context, box) => _build(context, box.maxWidth),
      );

  Widget _build(BuildContext context, double width) {
    final t = ThemeScope.of(context).theme;
    final state = Provider.of<LumitState>(context);
    final roots = state.project?.getItems() ?? const <ItemReference>[];
    final cols = ProjectColumns.forWidth(width, widths: _columnWidths);

    if (roots.isEmpty) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _searchRow(t),
          _columnHeader(t, cols),
          Expanded(
            child: _importOnDoubleTap(
              child: Center(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 240),
                  child: Text(
                    l10n.projectEmpty,
                    style: t.small,
                    textAlign: TextAlign.center,
                  ),
                ),
              ),
            ),
          ),
          _footer(t,
              items: 0, missing: 0, width: width, project: state.project),
        ],
      );
    }

    _refreshMissing(roots);

    // The filter only bites while something is missing, so a healthy project can
    // never trap the user behind an empty "missing only" view.
    final missingCount = _missing.values.where((m) => m).length;
    final missingOnly = _missingOnly && missingCount > 0;

    final rows = <Widget>[];
    _visibleIds.clear();
    var itemCount = 0;

    // A row shows when its own name matches, or an ancestor folder's did —
    // searching a folder finds what it holds (docs/07 §3.1). Missing-only is
    // stricter: it is never widened by a folder name, so every visible row is
    // something to fix (docs/07 §3.3).
    void walk(ItemReference item, int depth, bool ancestorMatched) {
      final id = _idOf(item);
      _itemById[id] = item;
      itemCount++;
      final name = _nameOf(item);
      final ownMatch = _search.isEmpty || name.toLowerCase().contains(_search);
      final selfMatched = ancestorMatched || ownMatch;
      final isMissingFootage =
          item is ItemReference_Footage && (_missing[id] ?? false);
      final label = _labels[id] ??= _labelOf(item);
      final searchHit = selfMatched ||
          (item is ItemReference_Folder && _subtreeMatches(item));
      // Missing-only is matched on the row's own name alone (docs/07 §3.3).
      // The colour chip narrows *with* whatever else is running, and on the
      // row's own tag alone — a folder's colour is the folder's, not a claim
      // about everything inside it.
      final chipHit = _labelFilter == null || label == _labelFilter;
      final show =
          chipHit && (missingOnly ? isMissingFootage && ownMatch : searchHit);
      // Counted before the row is built rather than after it: the Items cell
      // reads this cache, so counting afterwards left the column one build
      // behind — a folder just filed into still said what it held before.
      final children = item is ItemReference_Folder
          ? item.field0.getChildren()
          : const <ItemReference>[];
      if (item is ItemReference_Folder) _childCounts[id] = children.length;
      if (show) {
        _visibleIds.add(id);
        rows.add(_ProjectRowFrb(
          key: ValueKey<String>('project-row-$id'),
          item: item,
          name: name,
          depth: depth,
          missing: isMissingFootage,
          // Sound with no picture at all — the probe's own answer, not the
          // zero picture width the panel used to infer it from (K-451). A
          // silent still has no sound and a picture that does not run; the
          // old guess called it audio.
          audio: _mediaInfo[id] != null && _mediaInfo[id]!.videoCodec == null,
          label: label,
          inUse: _used[id] ??= _isUsed(item),
          proxy: _proxies.putIfAbsent(
              id,
              () => item is ItemReference_Footage
                  ? item.field0.getProxy()
                  : null),
          selected: _selectedIds.contains(id),
          renaming: _renamingId == id,
          selectionCount: _selectedIds.length,
          columns: cols,
          cells: _cellsFor(item, id, isMissingFootage),
          selectedFootage: () => _selectedFootage,
          onSelect: (modifier) => _select(id, modifier),
          onStartRename: () => setState(() => _renamingId = id),
          onEndRename: () => setState(() => _renamingId = null),
          onFindMissing: () => setState(() => _missingOnly = true),
          onNewComposition: _newComposition,
          folderOpen: _search.isNotEmpty || !_closedFolders.contains(id),
          onToggleFolder: () => setState(() {
            if (!_closedFolders.remove(id)) _closedFolders.add(id);
          }),
          onLocalEdit: _documentChanged,
          onSetLabel: (picked) => _setLabel(item, picked),
          onMoveToFolder: (folder) => _fileInto(folder, _targets(item)),
          folderChoices: () => _folderChoices(item),
          onDropItems: item is ItemReference_Folder
              ? (dropped) => _fileInto(item, dropped)
              : null,
          relinkPicker: widget.relinkPicker,
        ));
      }
      if (item case ItemReference_Footage(:final field0)) {
        _footageById[id] = field0;
        // Decoded ahead of selection and held in RAM, so the preview card
        // shows the picture and the facts the instant a row is clicked.
        _refreshThumb(field0);
        _refreshMediaInfo(field0);
      }
      // A closed folder keeps its children to itself — unless a search is
      // running, which has to be able to find what is inside one.
      if (_search.isNotEmpty || !_closedFolders.contains(id)) {
        for (final child in children) {
          walk(child, depth + 1, selfMatched);
        }
      }
    }

    _footageById.clear();
    _itemById.clear();
    for (final item in roots) {
      walk(item, 0, false);
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (width >= _widthForPreview) _previewCard(t),
        _searchRow(t),
        _columnHeader(t, cols),
        Expanded(
          // Wrapping the list rather than sitting behind it: a sibling under a
          // ListView never sees a pointer, because the list is opaque across
          // its whole extent. As the parent it gets what the rows leave — and
          // a row's own double-tap wins the arena on the row itself.
          child: _importOnDoubleTap(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              controller: _hScroll,
              child: SizedBox(
                width: width < projectMinWidth ? projectMinWidth : width,
                child: ListView(children: rows),
              ),
            ),
          ),
        ),
        _scrollStrip(t),
        _footer(t,
            items: itemCount,
            missing: missingCount,
            width: width,
            project: state.project),
      ],
    );
  }

  /// Whether anything under this folder matches the needle, so a folder that
  /// holds a hit stays visible as the path to it.
  bool _subtreeMatches(ItemReference_Folder folder) {
    if (_search.isEmpty) return true;
    for (final child in folder.field0.getChildren()) {
      if (_nameOf(child).toLowerCase().contains(_search)) return true;
      if (child is ItemReference_Folder && _subtreeMatches(child)) return true;
    }
    return false;
  }

  /// The Size, fps and Items values this row can truthfully state, off the
  /// caches the walk fills. A row never works these out itself — it is handed
  /// finished strings, which is what keeps a hover free at the bridge.
  _Cells _cellsFor(ItemReference item, String id, bool missing) {
    switch (item) {
      case ItemReference_Footage(:final field0):
        // The path is what the *project* records, so it is worth stating even
        // for a file that is not there — it is where the item is pointing,
        // which is exactly what a relink is about to change.
        final path = _paths[id] ??= _pathOf(field0);
        if (missing) {
          return _Cells(size: _noValue, fps: _noValue, path: path);
        }
        final info = _mediaInfo[id];
        if (info == null) return _Cells(path: path);
        if (info.videoCodec == null) {
          // A sound file's cells, as the mockup writes them: the rate where a
          // picture would state its size, and the channel layout — shortened
          // to fit the FPS column — where a picture would state its rate.
          if (info.audioCodec == null) return _Cells(path: path);
          return _Cells(
            size: _sampleRateText(info.sampleRate),
            fps: switch (info.channels) {
              1 => l10n.audioMonoShort,
              2 => l10n.audioStereoShort,
              final n => l10n.audioChannels(n),
            },
            path: path,
          );
        }
        return _Cells(
          size: '${info.width}×${info.height}',
          // A still has no rate to state (K-246). It probes with a video
          // stream of one frame, so a number *is* there — and printing it
          // would say the picture runs when it does not.
          fps: info.isStill ? null : _rateText(info.fpsNum, info.fpsDen),
          path: path,
        );
      case ItemReference_Composition(:final field0):
        return _compCells[id] ??= () {
          final s = field0.getSettings();
          return _Cells(
            size: '${s.width}×${s.height}',
            fps: _rateText(s.fpsNum, s.fpsDen),
          );
        }();
      case ItemReference_Folder():
        final n = _childCounts[id];
        return _Cells(items: n?.toString());
      case ItemReference_Solid():
        return const _Cells();
    }
  }

  /// The search well. Inset on `surface_0` like every other value well (§2.1),
  /// inside the 8/6 the mockup pads the row with.
  Widget _searchRow(LumitTheme t) => SizedBox(
        key: const ValueKey('project-search-row'),
        height: projectSearchRowHeight,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(projectRowPadding, _searchPadTop,
              projectRowPadding, _searchPadBottom),
          child: SizedBox(
            height: wellHeight,
            child: Row(
              children: [
                Expanded(
                  child: HouseTextField(
                    key: const ValueKey('project-search'),
                    controller: _searchController,
                    focusNode: _searchFocus,
                    width: double.infinity,
                    // The well fills its row rather than floating inside it:
                    // the mockup renders it exactly 20 tall, and the default
                    // 3px above and below would burst that.
                    padding: const EdgeInsets.symmetric(horizontal: 6),
                    // `surface2`, not the well's usual recess: this well has
                    // its own row to itself over the panel's `surface1`, and
                    // the mockup computes it a shade lighter rather than a
                    // shade darker.
                    fill: t.surface2,
                    hint: l10n.searchProject,
                  ),
                ),
                // **The mockup's own gap between the well and the chips**
                // (owner, 2026-08-24; measured 1:1 against ProjectPanel's
                // manifest). The row is a flex line with `gap: 6`, and the
                // strip beside it is 59 wide — so the well comes out at 279 in
                // a 360 panel. The app had no gap here and a chip strip of 62,
                // because every chip carried a leading 3; the well measured
                // 282, three pixels over, and that is the difference the owner
                // kept reading. The 3 has gone back to being a gap *between*
                // chips, where the drawing puts it.
                const SizedBox(width: _searchGap),
                _labelChips(t),
              ],
            ),
          ),
        ),
      );

  /// The colour-chip filter, beside the search well (§12A.3a). Five palette
  /// dots and a neutral one: tapping a colour narrows the tree to the items
  /// tagged with it, tapping the neutral chip — or the held colour again —
  /// shows everything.
  ///
  /// The held chip is marked by a ring rather than by growing, so the row does
  /// not change width as the filter is used (§12A.5: nothing changes the
  /// resting state).
  Widget _labelChips(LumitTheme t) => Padding(
        key: const ValueKey('project-label-chips'),
        padding: const EdgeInsets.symmetric(horizontal: _chipStripPad),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final (i, label) in [...projectFilterLabels, null].indexed)
              Padding(
                // Between the chips, not before the first one: the strip's own
                // 4 of padding is what stands the leading dot off the well.
                padding: EdgeInsets.only(left: i == 0 ? 0 : _chipGap),
                child: LumitTooltip(
                  message: label == null
                      ? l10n.tipShowEverything
                      : l10n.tipFilterByLabel,
                  child: GestureDetector(
                    key: ValueKey<String>(
                        'project-label-chip-${label ?? 'none'}'),
                    behavior: HitTestBehavior.opaque,
                    onTap: () => setState(() =>
                        _labelFilter = (label == _labelFilter) ? null : label),
                    child: Container(
                      width: _chipSize,
                      height: _chipSize,
                      decoration: BoxDecoration(
                        color:
                            label == null ? t.surface4 : t.labelColour(label),
                        shape: BoxShape.circle,
                        border: _labelFilter == label && label != null
                            ? Border.all(color: t.textPrimary)
                            : null,
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
      );

  /// The column headings. Kicker words, and the values below them are laid out
  /// by the same [ProjectColumns.cells] call, so they cannot come apart. The
  /// gaps between them are the drag handles that resize the columns.
  Widget _columnHeader(LumitTheme t, ProjectColumns cols) => Container(
        key: const ValueKey('project-column-header'),
        height: projectColumnHeaderHeight(t),
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: t.hairline)),
        ),
        padding: const EdgeInsets.only(
            left: _headerPadLeft, right: projectRowPadding),
        child: Row(
          children: [
            // Name is the flexible slot, and what it is left with is
            // [projectNameColumn] — see [ProjectColumns.laidOutWidth].
            Expanded(child: Text(l10n.name.toUpperCase(), style: t.kicker)),
            ...cols.cells(
              seam: (before) => _ColumnSeam(
                key: ValueKey<String>(
                    'project-seam-${_leftOf(cols, before).name}'),
                // The seam resizes the column it follows, which is the one the
                // eye reads it as belonging to — and nothing at all beside a
                // column that has no width of its own to change.
                onResize: projectColumnIsFixedWidth(_leftOf(cols, before))
                    ? null
                    : (delta) => _resizeColumn(_leftOf(cols, before), delta),
              ),
              items: l10n.projectColumnItems.toUpperCase(),
              size: l10n.projectColumnSize.toUpperCase(),
              fps: l10n.unitFps.toUpperCase(),
              path: l10n.projectColumnPath.toUpperCase(),
              style: t.kicker,
              // The heading is as quiet as its column: Path is context, not a
              // fact about the item, and the mockup hushes both together.
              pathStyle: t.kicker.copyWith(color: t.textDisabled),
            ),
          ],
        ),
      );

  /// Double-clicking the panel's blank space imports, which is the gesture
  /// every editor has and the one people reach for before finding a menu.
  Widget _importOnDoubleTap({required Widget child}) => GestureDetector(
        key: const ValueKey('project-empty-area'),
        behavior: HitTestBehavior.opaque,
        onDoubleTap: _import,
        child: child,
      );

  /// The horizontal scrollbar under the tree: a 4px track inset 8 either side,
  /// with a thumb as wide a share of it as the view is of the content. It is
  /// full width — and so says nothing — until the width ladder's last step
  /// actually bites.
  Widget _scrollStrip(LumitTheme t) => SizedBox(
        key: const ValueKey('project-scroll-strip'),
        height: projectScrollStripHeight,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: _scrollTrackInset),
          child: Align(
            alignment: Alignment.topCenter,
            child: Container(
              height: _scrollTrackHeight,
              decoration: BoxDecoration(
                color: t.surface2,
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              ),
              child: AnimatedBuilder(
                animation: _hScroll,
                builder: (context, _) {
                  // `maxScrollExtent` is only an answer once the list has been
                  // laid out — on the very first build there is nothing to
                  // measure yet, and the thumb simply fills its track.
                  final position =
                      _hScroll.hasClients ? _hScroll.position : null;
                  final p = position != null && position.hasContentDimensions
                      ? position
                      : null;
                  final extent = p == null || p.maxScrollExtent <= 0
                      ? 1.0
                      : p.viewportDimension /
                          (p.viewportDimension + p.maxScrollExtent);
                  final at = p == null || p.maxScrollExtent <= 0
                      ? 0.0
                      : (p.pixels / p.maxScrollExtent).clamp(0.0, 1.0);
                  return Align(
                    // -1 is hard left, 1 is hard right: the thumb travels the
                    // track's leftover room exactly as the view travels the
                    // content's.
                    alignment: Alignment(at * 2 - 1, 0),
                    child: FractionallySizedBox(
                      widthFactor: extent,
                      child: Container(
                        decoration: BoxDecoration(
                          color: t.surface4,
                          borderRadius:
                              BorderRadius.circular(t.tokens.controlRadius),
                        ),
                      ),
                    ),
                  );
                },
              ),
            ),
          ),
        ),
      );

  /// The bottom bar: the new-item controls at the left, the count at the right.
  ///
  /// **Import lives here** although the mockup draws only Folder and
  /// Composition — it is a command the panel has always carried and the mockup
  /// gave it no other home, so it takes the mockup's own icon-and-kicker shape
  /// rather than being dropped. The count's `n missing ·` half is the
  /// "show only missing" filter, which the mockup likewise has no row for.
  Widget _footer(LumitTheme t,
      {required int items,
      required int missing,
      required double width,
      ProjectReference? project}) {
    final active = _missingOnly && missing > 0;
    final labels = width >= _widthForFooterLabels;
    final count = t.kicker.copyWith(letterSpacing: _footerCountTracking);
    return Container(
      key: const ValueKey('project-footer'),
      height: projectFooterHeight,
      color: t.surface2,
      padding: const EdgeInsets.symmetric(horizontal: _footerPad),
      child: Row(
        children: [
          LumitTooltip(
            message: l10n.importFootage,
            child: _footerAction(
              t,
              key: const ValueKey('project-import'),
              icon: LumitIcon.import,
              label: width >= _widthForImportLabel
                  ? l10n.projectFooterImport
                  : null,
              onPressed: _import,
            ),
          ),
          const SizedBox(width: _footerGap),
          LumitTooltip(
            message: l10n.newFolder,
            child: _footerAction(
              t,
              key: const ValueKey('project-new-folder'),
              icon: LumitIcon.folder,
              label: labels ? l10n.projectFooterFolder : null,
              onPressed: _newFolder,
            ),
          ),
          const SizedBox(width: _footerGap),
          // Footage dropped here makes a comp that matches it (docs/07 §3.1)
          // — the same dialog the button opens, with the media's own size,
          // rate and length already filled in, and every dropped item landing
          // in the finished comp as a layer.
          DragTarget<FootageDragData>(
            onAcceptWithDetails: (d) => _newComposition(d.data.footage),
            builder: (context, candidate, _) => Container(
              foregroundDecoration: candidate.isEmpty
                  ? null
                  : BoxDecoration(border: Border.all(color: t.accent)),
              child: LumitTooltip(
                message: l10n.newComposition,
                child: _footerAction(
                  t,
                  key: const ValueKey('project-new-comp'),
                  icon: LumitIcon.newComposition,
                  label: labels ? l10n.projectFooterComposition : null,
                  onPressed: _newComposition,
                ),
              ),
            ),
          ),
          // **The project-wide proxies switch** (K-501, docs/07 §3.3). It sits
          // on the bottom bar after a divider, apart from the new-item
          // controls, for the reason the Timeline's own comp-wide toggles do
          // (§12A.1): a switch that governs the whole document reads apart
          // from the commands that make things.
          //
          // Drawn like the controls beside it — the set's own mark with a
          // kicker word after it — so it sheds the word and keeps the mark as
          // the panel narrows (§12A.6's step 4), rather than vanishing at the
          // width where a word-only control would have to. Its two strengths
          // are the switch conventions': `text_primary` on, `text_muted` off,
          // and never the accent (§3.1's list is closed).
          if (project != null) ...[
            Container(
                width: 1,
                height: 10,
                color: t.hairline,
                margin: const EdgeInsets.symmetric(horizontal: _footerIconGap)),
            Builder(builder: (context) {
              final on = _useProxies ??= project.useProxies();
              final ink = on ? t.textPrimary : t.textMuted;
              return LumitTooltip(
                message: on ? l10n.tipUseProxiesOn : l10n.tipUseProxiesOff,
                child: GestureDetector(
                  key: const ValueKey('project-use-proxies'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () {
                    project.setUseProxies(useProxies: !on);
                    _documentChanged();
                  },
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      glyph.LumitIcon(LumitIcons.proxy,
                          size: projectFooterIconSize, colour: ink),
                      if (labels) ...[
                        const SizedBox(width: _footerIconGap),
                        Text(
                          l10n.projectFooterProxies.toUpperCase(),
                          style: t.kicker.copyWith(
                              letterSpacing: _footerLabelTracking, color: ink),
                        ),
                      ],
                    ],
                  ),
                ),
              );
            }),
          ],
          const Spacer(),
          // The missing half reads *before* the total, so the bar ends on the
          // item count (the owner's order). The two stay separate strings: a
          // translator sees each phrase whole, and the order is the layout's
          // business rather than something spliced into a sentence.
          // Flexible like the total beside it: both halves are the bar's
          // truncating text (§12A.6's step 1), and the missing half used to be
          // fixed — which meant a narrow bar carrying a broken item overflowed
          // rather than shortening.
          if (missing > 0)
            Flexible(
                child: LumitTooltip(
              message: active ? l10n.tipShowEverything : l10n.tipMissingOnly,
              child: GestureDetector(
                key: const ValueKey('missing-toggle'),
                behavior: HitTestBehavior.opaque,
                onTap: () => setState(() => _missingOnly = !_missingOnly),
                child: Text(
                  '${l10n.projectMissingCount(missing)} · ',
                  style:
                      count.copyWith(color: active ? t.warning : t.textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            )),
          // The count never pushes the bar wider than the panel: it is the
          // flexible text the width ladder's first step truncates (§12A.6).
          Flexible(
            child: Text(l10n.projectItemCount(items),
                style: count, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
        ],
      ),
    );
  }

  Widget _footerAction(
    LumitTheme t, {
    required Key key,
    required LumitIcon icon,
    // Null once the bar is too narrow to spell the word out: §12A.6's ladder
    // step 4 says a toolbar sheds rather than shrinks, and an icon that is
    // already the mockup's own is what a shed word leaves behind.
    required String? label,
    required VoidCallback onPressed,
  }) =>
      GestureDetector(
        key: key,
        behavior: HitTestBehavior.opaque,
        onTap: onPressed,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            lumitIcon(icon, size: projectFooterIconSize, color: t.textMuted),
            if (label != null) ...[
              const SizedBox(width: _footerIconGap),
              Text(
                label.toUpperCase(),
                style: t.kicker.copyWith(letterSpacing: _footerLabelTracking),
              ),
            ],
          ],
        ),
      );

  /// The picked item's readout (docs/07 §3.1): poster frame, name, and the
  /// item's own vital statistics. Always present at a fixed height, so the tree
  /// below never jumps when the selection changes; with nothing picked it is
  /// simply quiet.
  Widget _previewCard(LumitTheme t) {
    final id = _anchorId;
    final item = id != null && _selectedIds.contains(id) ? _itemById[id] : null;

    return Container(
      key: const ValueKey('project-preview-card'),
      height: projectPreviewHeight,
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.all(_previewPad),
      child: item == null || id == null
          ? const SizedBox.expand()
          : _previewContent(t, item, id),
    );
  }

  Widget _previewContent(LumitTheme t, ItemReference item, String id) {
    final missing = item is ItemReference_Footage && (_missing[id] ?? false);
    final type = switch (item) {
      ItemReference_Footage() => l10n.projectTypeFootage,
      ItemReference_Folder() => l10n.projectTypeFolder,
      ItemReference_Composition() => l10n.projectTypeComposition,
      ItemReference_Solid() => l10n.projectTypeSolid,
    };

    Widget? thumb;
    if (item case ItemReference_Footage() when !missing) {
      // The picture comes straight from the RAM cache the walk prefilled, so
      // switching the selection redraws it in the same frame.
      final image = _thumbs[id];
      thumb = SizedBox(
        width: _thumbWidth,
        height: _thumbHeight,
        child: image == null
            ? Center(
                child: lumitIcon(LumitIcon.footage,
                    size: iconSize, color: t.textMuted))
            : ClipRRect(
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                child: Container(
                  color: t.surface0,
                  child: RawImage(image: image, fit: BoxFit.contain),
                ),
              ),
      );
    }

    return Row(
      key: const ValueKey('project-info-header'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (thumb != null) ...[thumb, const SizedBox(width: _previewPad)],
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(_nameOf(item),
                  style: t.bodyPrimary, overflow: TextOverflow.ellipsis),
              const SizedBox(height: _previewLineGap),
              _previewFacts(t, item, id, missing),
              const SizedBox(height: _previewLineGap),
              // The card's second fact line: the mockup's `H.264 · 48 kHz
              // stereo`. Codec names are the file's own words, not ours, so
              // they are printed as the container declares them. With nothing
              // to say — a folder, a solid, a file that will not probe — it
              // falls back to the kind of thing this is, which is what the
              // card said before the codec crossed.
              Text(
                _previewCodecs(item, id, missing) ?? type,
                key: const ValueKey('project-info-codec'),
                style: _metaStyle(t),
                overflow: TextOverflow.ellipsis,
              ),
            ],
          ),
        ),
      ],
    );
  }

  /// The card's first fact line: what this item can truthfully state. The
  /// length reads as `HH:MM:SS:FF` timecode at the item's own rate — the same
  /// clock face the Viewer shows — never as a bare frame count.
  Widget _previewFacts(
      LumitTheme t, ItemReference item, String id, bool missing) {
    String? line;
    switch (item) {
      case ItemReference_Footage():
        if (missing) {
          return Text(l10n.projectItemMissing,
              style: _metaStyle(t).copyWith(color: t.warning));
        }
        final info = _mediaInfo[id];
        if (info != null) {
          final fps = info.fpsDen == 0 ? 0.0 : info.fpsNum / info.fpsDen;
          final seconds = info.duration.den == 0
              ? 0.0
              : info.duration.num / info.duration.den;
          final frames = (seconds * fps).round();
          final size = '${info.width}×${info.height}';
          line = switch (info) {
            // No picture at all — the probe's answer, not an inference from a
            // width of zero. Sound has no frames worth counting, so its last
            // field is milliseconds rather than a frame number.
            _ when info.videoCodec == null =>
              '${l10n.projectInfoAudio} · ${timecodeOfSecondsMs(seconds)}',
            // A picture that does not run has no rate and no length: the two
            // numbers a still would print are one frame and one frame's worth
            // of time, which say nothing and imply motion.
            _ when info.isStill => '$size · ${l10n.projectInfoStill}',
            _ => '$size · ${fps.toStringAsFixed(2)} ${l10n.unitFps}'
                ' · ${timecodeOfRate(frames, info.fpsNum, info.fpsDen)}',
          };
        }
      case ItemReference_Composition(:final field0):
        final s = field0.getSettings();
        final fps = s.fpsDen == 0 ? 0.0 : s.fpsNum / s.fpsDen;
        final frames = field0.durationFrames();
        line = '${s.width}×${s.height} · '
            '${fps.toStringAsFixed(2)} ${l10n.unitFps}'
            ' · ${timecodeOfRate(frames, s.fpsNum, s.fpsDen)}';
      case ItemReference_Folder(:final field0):
        line = l10n.projectItemCount(field0.getChildren().length);
      case ItemReference_Solid():
        break;
    }
    if (line == null) return const SizedBox.shrink();
    return Text(line,
        key: const ValueKey('project-info-line'),
        style: _metaStyle(t),
        overflow: TextOverflow.ellipsis);
  }

  /// The card's second fact line for footage: what the container is made of.
  /// `null` when there is nothing truthful to say, which is every other kind
  /// of item and any file that has not probed.
  String? _previewCodecs(ItemReference item, String id, bool missing) {
    if (item is! ItemReference_Footage || missing) return null;
    final info = _mediaInfo[id];
    if (info == null) return null;
    final parts = [
      if (info.videoCodec != null) info.videoCodec!,
      if (info.audioCodec != null && info.sampleRate > 0)
        '${_sampleRateText(info.sampleRate)} ${_channelText(info.channels)}',
    ];
    return parts.isEmpty ? null : parts.join(' · ');
  }

  /// Fill in a footage item's media facts, off the build.
  void _refreshMediaInfo(FootageReference footage) {
    final id = footage.internalid.toString();
    if (_mediaInfo.containsKey(id)) return;
    // Claim the slot first, so a rebuild mid-probe does not probe twice.
    _mediaInfo[id] = null;
    footage.mediaInfo().then((info) {
      if (!mounted || info == null) return;
      setState(() => _mediaInfo[id] = info);
    });
  }

  /// An item's name, calm when it was deleted from under the panel.
  String _nameOf(ItemReference item) {
    try {
      return item.name();
    } catch (_) {
      return '';
    }
  }

  /// The **folder** a footage item points into. Display data — the engine
  /// reads the path the project records (K-173) and touches no disk for it.
  ///
  /// The folder, not the whole path, because the Name column two cells left is
  /// already saying the file name: a Path column that repeated it would be the
  /// same fact twice rather than the context §12A.3a asks it to carry, and the
  /// mockup draws a folder (`~/…`) here for exactly that reason.
  ///
  /// Empty when the recorded path is a bare name — a project that has never
  /// been saved, and the footage-beside-the-project convention — because there
  /// genuinely is no folder to state: the reference is relative to wherever
  /// the project lands.
  String _pathOf(FootageReference footage) {
    try {
      final path = footage.filePath();
      final cut = path.lastIndexOf(RegExp(r'[/\\]'));
      return cut < 0 ? '' : path.substring(0, cut);
    } catch (_) {
      return '';
    }
  }

  /// This item's colour tag, and whether a composition places it. Both are
  /// document questions with document answers, so both are cached with the
  /// epoch rather than asked per rebuild; the same calm-on-a-deleted-item rule
  /// the name read follows applies to both.
  int _labelOf(ItemReference item) {
    try {
      return item.label();
    } catch (_) {
      return 0;
    }
  }

  bool _isUsed(ItemReference item) {
    try {
      return item.isUsed();
    } catch (_) {
      return false;
    }
  }

  /// What a row menu's command acts on: the whole selection when the row is
  /// part of it, and that row alone when it is not. The menu is about what is
  /// picked, and a right-click on an unpicked row is about that row.
  List<ItemReference> _targets(ItemReference item) =>
      _selectedIds.contains(_idOf(item))
          ? [
              for (final id in _visibleIds)
                if (_selectedIds.contains(id)) _itemById[id],
            ].whereType<ItemReference>().toList()
          : [item];

  /// Tag every selected item, or untag them with 0 — one call each, so one
  /// undo step each, which is what the engine's op is.
  void _setLabel(ItemReference item, int label) {
    for (final target in _targets(item)) {
      target.setLabel(label: label);
    }
    _documentChanged();
  }

  /// File `items` into `folder` — the drag onto a folder row, and the row
  /// menu's **Move to folder**.
  ///
  /// One undo step for the whole gesture: the seam files one item per call, so
  /// several are wrapped in an undo group and undo takes the drop back whole
  /// rather than one item at a time.
  ///
  /// A refusal is calm and silent: the engine turns down a folder dropped into
  /// its own descendant (and anything whose item has since been deleted), and
  /// the rest of the drop still lands.
  void _fileInto(ItemReference folder, List<ItemReference> items) {
    if (items.isEmpty) return;
    final id = folder is ItemReference_Folder ? folder.field0.internalid : null;
    if (id == null) return;
    final project = Provider.of<LumitState>(context, listen: false).project;
    final group = items.length > 1 && project != null;
    if (group) project.beginUndoGroup();
    for (final item in items) {
      try {
        item.moveToFolder(folder: id);
      } catch (_) {
        // Refused — a cycle, or an item that is no longer there.
      }
    }
    if (group) project.endUndoGroup();
    _documentChanged();
  }

  /// The folders a **Move to folder** menu may offer, in the order the panel
  /// lists them, walked at gesture time rather than held: a menu is not a
  /// rebuild path, and the tree it must offer includes folders inside shut
  /// ones, which the panel's own walk never reaches.
  ///
  /// `excluding` drops an item's own subtree — a folder cannot be filed into
  /// itself or into anything it holds, and offering the move only to have the
  /// engine refuse it is a dead entry.
  List<(String, ItemReference)> _folderChoices(ItemReference excluding) {
    final out = <(String, ItemReference)>[];
    final skip = _idOf(excluding);
    void walk(ItemReference item) {
      if (item is! ItemReference_Folder) return;
      if (_idOf(item) == skip) return;
      out.add((_nameOf(item), item));
      item.field0.getChildren().forEach(walk);
    }

    (Provider.of<LumitState>(context, listen: false).project?.getItems() ??
            const <ItemReference>[])
        .forEach(walk);
    return out;
  }

  Future<void> _newFolder() async {
    final state = Provider.of<LumitState>(context, listen: false);
    // Filed inside the picked folder when one is picked, at the root
    // otherwise — the folder you are looking at is the one a new folder
    // belongs in. The engine takes it from there: a blank name becomes the
    // next unused "Folder N", and a parent that has since gone leaves the
    // folder at the root rather than refusing to make it.
    final anchor = _anchorId != null ? _itemById[_anchorId] : null;
    final parent =
        anchor is ItemReference_Folder ? anchor.field0.internalid : null;
    state.project?.newFolder(name: '', parent: parent);
    _documentChanged();
  }

  Future<void> _import() async {
    final state = Provider.of<LumitState>(context, listen: false);
    if (await state
        .importFootagePaths(await (widget.importPicker ?? pickFootage)())) {
      _documentChanged();
    }
  }

  /// Ask for the new comp's settings, then make it. `footage` is whatever was
  /// dropped on the button; empty for a plain click.
  Future<void> _newComposition([
    List<FootageReference> footage = const [],
  ]) async {
    final state = Provider.of<LumitState>(context, listen: false);
    final comp = await state.newComposition(context, footage: footage);
    if (comp == null || !mounted) return;
    // Fronted because a comp you just made is the one you want to work on.
    Provider.of<LumitUiState>(context, listen: false).setSelectedComp(comp);
    _documentChanged();
  }

  /// An edit landed: re-probe and re-decode. Bumping the epoch is what makes a
  /// relink show the new picture rather than the cached one.
  void _documentChanged() {
    setState(() {
      _epoch++;
      _missing.clear();
      _mediaInfo.clear();
      _compCells.clear();
      _childCounts.clear();
      _paths.clear();
      _used.clear();
      _labels.clear();
      _proxies.clear();
      _useProxies = null;
      _dropThumbs();
    });
  }

  void _dropThumbs() {
    for (final image in _thumbs.values) {
      image?.dispose();
    }
    _thumbs.clear();
  }

  /// Decode a footage item's poster frame into the RAM cache, off the build.
  void _refreshThumb(FootageReference footage) {
    final id = footage.internalid.toString();
    if (_thumbs.containsKey(id)) return;
    // Claim the slot first, so a rebuild mid-decode does not decode twice.
    _thumbs[id] = null;
    final epoch = _epoch;
    footage.thumbnail(maxEdge: _thumbMaxEdge).then((frame) {
      if (!mounted || epoch != _epoch) return;
      if (frame == null || frame.width == 0 || frame.height == 0) return;
      ui.decodeImageFromPixels(
        frame.rgba,
        frame.width,
        frame.height,
        ui.PixelFormat.rgba8888,
        (image) {
          if (!mounted || epoch != _epoch) {
            image.dispose();
            return;
          }
          setState(() => _thumbs[id] = image);
        },
      );
    });
  }

  /// Fill in any footage status we do not yet know, off the build.
  ///
  /// `getStatus` probes the file, so this must never be awaited inside `build`.
  /// Statuses arrive over one or more frames and each one that changes triggers a
  /// rebuild; an item already known is not re-probed until [_documentChanged]
  /// clears the cache.
  void _refreshMissing(List<ItemReference> roots) {
    void walk(ItemReference item) {
      if (item is ItemReference_Footage) {
        final id = _idOf(item);
        if (!_missing.containsKey(id)) {
          // Claim the slot first, so a rebuild mid-probe does not probe twice.
          _missing[id] = false;
          item.field0.getStatus().then((status) {
            if (!mounted) return;
            final isMissing = status == LumitMediaStatus.missing;
            if (_missing[id] != isMissing) {
              setState(() => _missing[id] = isMissing);
            }
            // A probe can outlive its document: opening a project clears the
            // engine's registry and every reference held from the outgoing one
            // throws. Nothing is missing in a document that is gone, and the
            // panel is about to be rebuilt from the new one.
          }).catchError((_) {});
        }
      }
      if (item is ItemReference_Folder) {
        item.field0.getChildren().forEach(walk);
      }
    }

    roots.forEach(walk);
  }
}

/// An item's id as a string, for keys and selection.
///
/// The generated references expose their ids under `internalid`; this is the one
/// place that name appears, so a future rename of the frb field is a one-line
/// change here rather than a sweep.
String _idOf(ItemReference item) => switch (item) {
      ItemReference_Footage(:final field0) => field0.internalid.toString(),
      ItemReference_Solid(:final field0) => field0.internalid.toString(),
      ItemReference_Composition(:final field0) => field0.internalid.toString(),
      ItemReference_Folder(:final field0) => field0.internalid.toString(),
    };

/// A state badge: the mockup's small outlined pill, in its own colour hushed
/// to an outline. Not a kicker — a kicker names a container, and this reports
/// a state — so it is plain mono with no tracking and no capitals.
class ProjectBadge extends StatelessWidget {
  final String label;
  final Color colour;

  const ProjectBadge({super.key, required this.label, required this.colour});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: _badgeHeight,
      alignment: Alignment.center,
      padding: const EdgeInsets.symmetric(horizontal: _badgePad),
      decoration: BoxDecoration(
        border: Border.all(color: colour.withValues(alpha: _badgeBorderAlpha)),
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
      ),
      child: Text(
        label,
        style: t.mono.copyWith(fontSize: _badgeTextSize, color: colour),
      ),
    );
  }
}

/// One Project panel row.
class _ProjectRowFrb extends StatefulWidget {
  final ItemReference item;

  /// The item's name, read once by the panel's walk. Passed in rather than
  /// fetched here because this row rebuilds on every hover flicker, and a
  /// bridge call per hover was exactly the chatter Airyzz measured.
  final String name;
  final int depth;
  final bool missing;

  /// Sound with no picture — the media probe's own answer, which is what picks
  /// the speaker glyph over the film one.
  final bool audio;

  /// This item's colour tag, an index into the label palette. `0` is untagged,
  /// and an untagged row keeps its per-type tint (§12A.3a: a tag **tints the
  /// glyph's strokes** rather than adding a dot beside it).
  final int label;

  /// Whether any composition places this item — the `in use` badge.
  final bool inUse;

  /// This item's proxy (K-501), or null where it has none. Null on every kind
  /// but footage — nothing else has a media reference to stand in for.
  final BridgeProxy? proxy;
  final bool selected;
  final bool renaming;

  /// Which optional columns this width carries, and the finished strings to put
  /// in them — both worked out by the panel, for the same reason the name is.
  final ProjectColumns columns;
  final _Cells cells;

  /// How many rows are selected in all — a second click renames only when this
  /// row is the whole selection.
  final int selectionCount;
  final ValueChanged<SelectMode> onSelect;

  /// The panel's whole footage selection, read when a drag starts so dragging
  /// any selected row brings the rest with it.
  final List<FootageReference> Function() selectedFootage;
  final VoidCallback onStartRename;
  final VoidCallback onEndRename;
  final VoidCallback onFindMissing;

  /// Make a comp from these items — the panel's own New composition, so a
  /// double-clicked footage item goes through the one funnel every other route
  /// does (K-243).
  final void Function(List<FootageReference>) onNewComposition;

  /// Whether this row's folder is showing its children, and the toggle that
  /// opens or shuts it. Meaningless on every other kind.
  final bool folderOpen;
  final VoidCallback onToggleFolder;

  /// Called after an edit this row made, so the panel re-reads at once rather
  /// than waiting for the engine's change stream to come back around.
  final VoidCallback onLocalEdit;

  /// Tag this row — and the rest of the selection, when this row is part of
  /// one — from the context menu's chip strip.
  final ValueChanged<int> onSetLabel;

  /// File these items into this row's folder — the drop gesture. Null on every
  /// kind but a folder, which is what makes a folder row the only drop target.
  final void Function(List<ItemReference> items)? onDropItems;

  /// File this row (and the rest of the selection, when it is part of one) into
  /// the folder picked from **Move to folder**.
  final void Function(ItemReference folder) onMoveToFolder;

  /// The folders **Move to folder** may offer, read when the menu is raised.
  /// A function rather than a list because a menu is a gesture: reading the
  /// tree per rebuild is exactly the chatter the budget test guards against.
  final List<(String, ItemReference)> Function() folderChoices;
  final Future<String?> Function()? relinkPicker;

  const _ProjectRowFrb({
    super.key,
    required this.item,
    required this.name,
    required this.depth,
    required this.missing,
    required this.audio,
    required this.label,
    required this.inUse,
    required this.proxy,
    required this.selected,
    required this.renaming,
    required this.columns,
    required this.cells,
    required this.selectionCount,
    required this.onSelect,
    required this.selectedFootage,
    required this.onStartRename,
    required this.onEndRename,
    required this.onFindMissing,
    required this.onNewComposition,
    required this.folderOpen,
    required this.onToggleFolder,
    required this.onLocalEdit,
    required this.onSetLabel,
    required this.onMoveToFolder,
    required this.folderChoices,
    this.onDropItems,
    this.relinkPicker,
  });

  @override
  State<_ProjectRowFrb> createState() => _ProjectRowFrbState();
}

class _ProjectRowFrbState extends State<_ProjectRowFrb> {
  bool _hover = false;
  TextEditingController? _rename;
  // Escape on the field's own node, ahead of the shortcut system (K-323): the
  // row's editor is a bare EditableText rather than a HouseTextField, so it
  // wires the same key itself instead of inheriting it.
  late final FocusNode _renameFocus = FocusNode(onKeyEvent: _onRenameKey);

  KeyEventResult _onRenameKey(FocusNode node, KeyEvent event) {
    if (event is KeyDownEvent &&
        event.logicalKey == LogicalKeyboardKey.escape) {
      _cancelRename();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  ItemReference get item => widget.item;

  @override
  void dispose() {
    _rename?.dispose();
    _renameFocus.dispose();
    super.dispose();
  }

  @override
  void didUpdateWidget(_ProjectRowFrb old) {
    super.didUpdateWidget(old);
    if (widget.renaming && !old.renaming) {
      _rename = TextEditingController(text: widget.name);
      _renameFocus.requestFocus();
    }
  }

  /// Escape: shut the editor, rename nothing (K-323).
  void _cancelRename() {
    _rename?.dispose();
    _rename = null;
    widget.onEndRename();
  }

  void _commitRename() {
    final text = _rename?.text.trim() ?? '';
    if (text.isNotEmpty && text != widget.name) {
      item.rename(name: text);
      widget.onLocalEdit();
    }
    _rename?.dispose();
    _rename = null;
    widget.onEndRename();
  }

  /// Whether this row was already selected when the pointer went down — what
  /// decides, at mouse-up, between renaming and collapsing the selection.
  bool _wasSelectedAtDown = false;

  /// The click in flight: where it started, whether it wandered past the
  /// touch slop (a drag, not a click), and whether it was the primary button.
  Offset _downAt = Offset.zero;
  bool _dragged = false;
  bool _primaryDown = false;

  /// Selection happens on pointer DOWN, not on the resolved tap: the tap
  /// gesture waits out the double-click window and the drag arena, which read
  /// as the panel lagging behind the mouse. The one case that must wait is a
  /// plain press on an already-selected row — collapsing a multi-selection on
  /// the down stroke would make dragging that selection impossible.
  void _handlePointerDown(PointerDownEvent event) {
    if (event.buttons != kPrimaryButton) return;
    _primaryDown = true;
    _downAt = event.position;
    _dragged = false;
    _wasSelectedAtDown = widget.selected;
    final mode = _selectModeFromKeyboard();
    if (mode == SelectMode.replace && widget.selected) return;
    widget.onSelect(mode);
  }

  void _handlePointerMove(PointerMoveEvent event) {
    if ((event.position - _downAt).distance > kTouchSlop) _dragged = true;
  }

  /// The click, resolved on the raw pointer UP rather than through the
  /// gesture arena — the arena waits out the empty-area double-tap window,
  /// which is exactly the lag being avoided. A second click on the lone
  /// selected row *opens* it, and what opening means is the item's own answer
  /// (K-243): a composition fronts in the Timeline, footage raises New
  /// composition sized and timed to it, a folder renames in place. The second
  /// click of a double-click, or any later click, both land here. A plain click
  /// on one row of a multi-selection collapses the selection to it.
  void _handlePointerUp(PointerUpEvent event) {
    if (!_primaryDown) return;
    _primaryDown = false;
    if (_dragged || !_wasSelectedAtDown) return;
    if (_selectModeFromKeyboard() != SelectMode.replace) return;
    if (widget.selectionCount <= 1 && !widget.renaming) {
      // Opening a comp is what a double-click means in every editor — so a comp
      // is renamed from its context menu or its settings dialogue instead,
      // never by a stray second click on the row.
      if (item case ItemReference_Composition(:final field0)) {
        Provider.of<LumitUiState>(context, listen: false)
            .setSelectedComp(field0);
        return;
      }
      // Footage has no window of its own to open, and the thing people want
      // from a clip they have just double-clicked is a comp to put it in —
      // already the size, rate and length of the media, because that dialogue
      // reads the selection (the longest item wins when there are several).
      if (item case ItemReference_Footage(:final field0)) {
        final selected = widget.selectedFootage();
        widget.onNewComposition(selected.isEmpty ? [field0] : selected);
        return;
      }
      // A folder opens and shuts, which is what opening one means. Renaming it
      // is on the row menu with the other two kinds'.
      if (item is ItemReference_Folder) {
        widget.onToggleFolder();
        return;
      }
      // Nothing for the other kinds: a second click used to rename them in
      // place (K-191), which meant a slow double-click and a deliberate click
      // on a selected row were the same gesture and names opened editors
      // under people's pointers. Renaming is `Enter` on the selection now
      // (K-321), with the row menu's Rename as the mouse path.
      return;
    }
    widget.onSelect(SelectMode.replace);
  }

  Future<void> _doRelink(FootageReference footage) async {
    final picker = widget.relinkPicker;
    final path = picker != null
        ? await picker()
        : await pickFootage()
            .then((paths) => paths.isEmpty ? null : paths.first);
    if (path == null) return;
    footage.relink(path: path);
    widget.onLocalEdit();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final row = MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      // A raw listener, not a gesture: down and up fire the instant they
      // happen, without waiting for the tap/drag/double-tap arena to resolve.
      child: Listener(
        onPointerDown: _handlePointerDown,
        onPointerMove: _handlePointerMove,
        onPointerUp: _handlePointerUp,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          // Registered but empty: it claims double-clicks on the row in the
          // gesture arena, so the panel's empty-area double-tap (import) never
          // fires for a double-click on an item. The rename those clicks mean
          // already happened on the raw pointer-up above.
          onDoubleTap: () {},
          onSecondaryTapDown: (d) {
            // A right-click on a row already in the selection keeps it: the menu
            // is about what is picked, and collapsing four rows to one because the
            // menu was opened would throw the selection away.
            if (!widget.selected) widget.onSelect(SelectMode.replace);
            showProjectMenuFrb(
              context: context,
              item: item,
              missing: widget.missing,
              position: d.globalPosition,
              onFindMissing: widget.onFindMissing,
              onLocalEdit: widget.onLocalEdit,
              onStartRename: widget.onStartRename,
              onSetLabel: widget.onSetLabel,
              label: widget.label,
              onRelink: item is ItemReference_Footage
                  ? () => _doRelink((item as ItemReference_Footage).field0)
                  : null,
              proxy: widget.proxy,
              // The same picker seam the relink uses — picking a proxy file
              // and picking a replacement are the same gesture over the same
              // dialogue, and tests inject one stub for both.
              proxyPicker: widget.relinkPicker,
              // Read here, not in build: raising the menu is a gesture.
              folders: widget.folderChoices(),
              onMoveToFolder: widget.onMoveToFolder,
            );
          },
          child: Container(
            height: projectRowHeight,
            // Three greys at rest (§2.1, K-439): the row's own selected fill is
            // the header grey, and `surface_3` appears only under the pointer.
            color: widget.selected
                ? t.surface2
                : _hover
                    ? t.surface3
                    : null,
            padding: EdgeInsets.only(
              left: projectRowPadding + widget.depth * projectIndentPerDepth,
              right: projectRowPadding,
            ),
            child: Row(
              children: [
                _twirl(t),
                const SizedBox(width: projectRowGap),
                _glyph(t),
                const SizedBox(width: projectRowGap),
                // The row's flexible slot, and so the Name column: everything
                // to its right is a fixed box and the last of those swallows
                // the panel's spare width, so what is left here settles at
                // [projectNameColumn] however wide the panel grows (owner,
                // desk test). The indent and the badges come out of it exactly
                // as they always did.
                Expanded(child: _nameOrEditor(t)),
                // Placed somewhere (§12A.3a). Before the missing badge, so a
                // broken file that is nonetheless *in* a comp reads left to
                // right as "used, and lost" — which is the order those two
                // facts matter in.
                if (widget.inUse) ...[
                  const SizedBox(width: projectRowGap),
                  ProjectBadge(
                    key: ValueKey<String>('in-use-${_idOf(item)}'),
                    label: l10n.projectItemInUse,
                    colour: t.success,
                  ),
                ],
                // Reading from a stand-in (K-501). **Quiet on purpose**: the
                // other two badges wear a state colour because they report
                // something that wants acting on — placed, or lost. A proxy is
                // neither; it is a fact about which file the item is being
                // read from, so it takes the badge family's shape at
                // `text_muted` and claims none of the palette.
                //
                // Drawn only while the tick is on: the badge says "this item
                // is being read from its proxy", so a proxy that is attached
                // and switched off has nothing to announce.
                if (widget.proxy?.inUse ?? false) ...[
                  const SizedBox(width: projectRowGap),
                  ProjectBadge(
                    key: ValueKey<String>('proxy-${_idOf(item)}'),
                    label: l10n.projectItemProxy,
                    colour: t.textMuted,
                  ),
                ],
                if (widget.missing) ...[
                  const SizedBox(width: projectRowGap),
                  // The badge *is* the relink control: the mockup gives a
                  // broken row a pill and no button, and the panel would
                  // otherwise lose the one-click relink the row has always had.
                  LumitTooltip(
                    message: l10n.relinkEllipsis,
                    child: GestureDetector(
                      key: ValueKey<String>('relink-${_idOf(item)}'),
                      behavior: HitTestBehavior.opaque,
                      onTap: () =>
                          _doRelink((item as ItemReference_Footage).field0),
                      child: ProjectBadge(
                        label: l10n.projectItemMissing,
                        colour: t.warning,
                      ),
                    ),
                  ),
                ],
                ...widget.columns.cells(
                  items: widget.cells.items,
                  size: widget.cells.size,
                  fps: widget.cells.fps,
                  path: widget.cells.path,
                  style: _metaStyle(t),
                  pathStyle: _metaStyle(t).copyWith(color: t.textDisabled),
                ),
              ],
            ),
          ),
        ),
      ),
    );

    // Footage drags onto the Timeline (or onto New composition) as
    // `FootageDragData`; a composition drags onto the Timeline alone, as
    // `CompDragData`, to nest as a Precomp layer. The payload types are the
    // contract — the drop targets consume exactly these.
    if (item case ItemReference_Footage(:final field0)) {
      final name = widget.name;
      // Dragging a row that is part of the selection brings the whole selection;
      // dragging an unselected row is about that row alone, which is what every
      // file list does and what stops a stale selection following the pointer.
      final selection = widget.selected ? widget.selectedFootage() : const [];
      final dragged = selection.length > 1
          ? List<FootageReference>.from(selection)
          : <FootageReference>[field0];
      return Draggable<FootageDragData>(
        data: FootageDragData(
          dragged,
          dragged.length > 1 ? '${dragged.length} items' : name,
        ),
        dragAnchorStrategy: pointerDragAnchorStrategy,
        feedback: _DragFeedbackFrb(
          name: dragged.length > 1 ? '${dragged.length} items' : name,
        ),
        child: row,
      );
    }
    if (item case ItemReference_Composition(:final field0)) {
      return Draggable<CompDragData>(
        data: CompDragData(field0, widget.name),
        dragAnchorStrategy: pointerDragAnchorStrategy,
        feedback: _DragFeedbackFrb(name: widget.name, icon: LumitIcon.comp),
        child: row,
      );
    }
    // A folder row takes what the other rows drag: dropping on it files the
    // items there (K-451). Two nested targets rather than one, because a
    // `DragTarget` is typed and the panel's rows carry two payload types — the
    // same two the Timeline consumes.
    final drop = widget.onDropItems;
    if (drop != null) {
      return DragTarget<FootageDragData>(
        onAcceptWithDetails: (d) =>
            drop([for (final f in d.data.footage) ItemReference.footage(f)]),
        builder: (context, footageOver, _) => DragTarget<CompDragData>(
          onAcceptWithDetails: (d) =>
              drop([ItemReference.composition(d.data.comp)]),
          builder: (context, compOver, _) => Container(
            // The drop-target treatment (§6.5), painted over the row rather
            // than behind it: the row draws its own fill, and a drop with no
            // feedback is indistinguishable from one that did nothing.
            foregroundDecoration: footageOver.isEmpty && compOver.isEmpty
                ? null
                : BoxDecoration(
                    border: Border.all(color: t.accent, width: 1.5),
                    color: t.accent.withValues(alpha: 0.1),
                  ),
            child: row,
          ),
        ),
      );
    }
    return row;
  }

  /// The twirl's slot. A shut folder has to say so, or it reads as an empty
  /// one; the caret is its own target as well, the way the Hierarchy's is —
  /// and every row keeps the slot whether or not it has one, so a child still
  /// lines up one indent step right of the folder holding it.
  Widget _twirl(LumitTheme t) => SizedBox(
        width: projectRowIconSize,
        child: item is! ItemReference_Folder
            ? null
            : GestureDetector(
                key: ValueKey<String>('project-twirl-${_idOf(item)}'),
                behavior: HitTestBehavior.opaque,
                onTap: widget.onToggleFolder,
                child: lumitIcon(
                  widget.folderOpen
                      ? LumitIcon.twirlOpen
                      : LumitIcon.twirlClosed,
                  size: projectRowIconSize,
                  color: t.textMuted,
                ),
              ),
      );

  /// The row's type glyph, tinted by what the item *is* — the mockup's own
  /// per-type tints, which are the label palette's own chips (K-188): azure for
  /// picture footage, indigo for sound, amber for solids. A folder and a
  /// composition stay muted, as the mockup draws them, and missing footage
  /// wears the warning-tinted unlink glyph. No thumbnail here — the preview
  /// card carries the picture, so the tree stays a tight list of names.
  ///
  /// **A colour tag takes the glyph over** (docs/07 §3.1, §12A.3a: the tag
  /// tints the icon's strokes rather than adding a dot). The per-type tint is
  /// what an *untagged* item wears — a default, not a fact — so a tag that
  /// left it alone would need a mark of its own, which is what the mockup
  /// deliberately does not draw. Missing still wins over both: a file that is
  /// not there is more urgent than what colour someone filed it under.
  Widget _glyph(LumitTheme t) {
    final (icon, tint) = _iconFor(item, t);
    return lumitIcon(
      widget.missing ? LumitIcon.unlink : icon,
      size: projectRowIconSize,
      color: widget.missing
          ? t.warning
          : widget.label != 0
              ? t.labelColour(widget.label)
              : tint,
    );
  }

  Widget _nameOrEditor(LumitTheme t) {
    final controller = _rename;
    if (widget.renaming && controller != null) {
      return Container(
        padding: const EdgeInsets.symmetric(horizontal: 4),
        decoration: BoxDecoration(
          color: t.surface0,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.accent),
        ),
        child: EditableText(
          key: const ValueKey('rename-field'),
          controller: controller,
          focusNode: _renameFocus,
          style: t.body,
          cursorColor: t.accent,
          backgroundCursorColor: t.surface2,
          selectionColor: t.accent.withValues(alpha: 0.5),
          onSubmitted: (_) => _commitRename(),
          onTapOutside: (_) => _commitRename(),
        ),
      );
    }
    // A folder names a group, and a picked row is the one being talked about:
    // both read at `text_primary`. A broken one drops to muted — the badge
    // beside it is what carries the news.
    final colour = widget.missing
        ? t.textMuted
        : (widget.selected || item is ItemReference_Folder)
            ? t.textPrimary
            : t.textSecondary;
    return Text(widget.name,
        style: t.body.copyWith(color: colour), overflow: TextOverflow.ellipsis);
  }

  (LumitIcon, Color) _iconFor(ItemReference item, LumitTheme t) =>
      switch (item) {
        // Sound has no picture, so the media probe's zero width is what tells
        // the two apart — and it is the engine's answer, not a guess at the
        // file name.
        ItemReference_Footage() => widget.audio
            ? (LumitIcon.audioFile, t.labelColour(6))
            : (LumitIcon.footage, t.labelColour(1)),
        ItemReference_Folder() => (LumitIcon.folder, t.textMuted),
        ItemReference_Composition() => (LumitIcon.comp, t.textMuted),
        ItemReference_Solid() => (LumitIcon.solid, t.labelColour(2)),
      };
}

/// The floating label shown under the pointer while a row is dragged.
class _DragFeedbackFrb extends StatelessWidget {
  final String name;
  final LumitIcon icon;
  const _DragFeedbackFrb({required this.name, this.icon = LumitIcon.footage});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            lumitIcon(icon,
                size: iconSize,
                color: icon == LumitIcon.comp ? t.textMuted : t.labelColour(1)),
            const SizedBox(width: 6),
            Text(name, style: t.small),
          ],
        ),
      ),
    );
  }
}

enum _ProjectMenuAction {
  compSettings,
  rename,
  relink,
  findMissing,
  addAudioOnly,
  setProxy,
  makeProxy,
  useProxy,
  clearProxy,
  moveToRoot,
  delete
}

/// The project context menu.
Future<void> showProjectMenuFrb({
  required BuildContext context,
  required ItemReference item,
  required bool missing,
  required Offset position,
  required VoidCallback onFindMissing,
  required VoidCallback onLocalEdit,
  Future<void> Function()? onRelink,

  /// Put the row into its in-place rename editor. Null where the menu is
  /// raised from somewhere with no row to edit.
  VoidCallback? onStartRename,

  /// Tag the item. Null where the menu is raised with no row to tag, which is
  /// what makes the Label row absent rather than dead.
  ValueChanged<int>? onSetLabel,

  /// The tag the item wears now, so the strip can mark it.
  int label = 0,

  /// This item's proxy (K-501), or null where it has none — which is what
  /// decides whether the menu offers *Use proxy* and *Clear proxy* at all.
  BridgeProxy? proxy,

  /// Where **Set proxy…** gets its path. The panel's own relink seam, so a
  /// test stubs one dialogue for both.
  Future<String?> Function()? proxyPicker,

  /// The folders **Move to folder** offers, name and handle, in the order the
  /// panel lists them. Empty — a project with no folders in it — leaves the
  /// entry off the menu rather than opening onto nothing.
  List<(String, ItemReference)> folders = const [],

  /// File the picked row, and the rest of the selection with it, into that
  /// folder.
  void Function(ItemReference folder)? onMoveToFolder,
}) async {
  final isFootage = item is ItemReference_Footage;
  final isComp = item is ItemReference_Composition;
  // The comp the sound would land in (K-435). Read once, here, rather than in
  // the row's build: the menu is a gesture, not a rebuild path.
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final openComp = ui.selectedComp;
  // The colour config's own space names, off the summary the shell holds, and
  // what this item is set to now (K-490). Both read here — raising a menu is a
  // gesture, and `colour_space` is a document read.
  final colourSpaces =
      ui.colourSummary.loaded ? ui.colourSummary.spaces : const <String>[];
  final colourSpace = switch (item) {
    ItemReference_Footage(:final field0) => field0.colourSpace(),
    _ => null,
  };
  // Read here rather than inside the popup's builder: the popup is raised in
  // its own route, so it has no ThemeScope of the panel's above it.
  final menuTheme = ThemeScope.of(context).theme;
  final action = await showLumitPopup<_ProjectMenuAction>(
    context: context,
    position: position,
    builder: (close) => FloatSurface(
      width: 210,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (isComp)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.compSettings),
              child: Text(l10n.compositionSettingsEllipsis),
            ),
          // Every kind can be renamed from here. It matters most for a comp,
          // whose second click opens it rather than renaming it.
          MenuRow(
            key: const ValueKey('project-menu-rename'),
            onPressed: () => close(_ProjectMenuAction.rename),
            child: Text(l10n.rename),
          ),
          // Relink is offered only on a row that is actually broken.
          if (isFootage && missing)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.relink),
              child: Text(l10n.relinkEllipsis),
            ),
          if (isFootage)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.findMissing),
              child: Text(l10n.findMissingFootage),
            ),
          // The sound of this clip, on its own row (K-435). Offered only with a
          // comp open to put it in — placing a layer nowhere is not an action.
          if (isFootage && openComp != null)
            MenuRow(
              key: const ValueKey('project-menu-add-audio-only'),
              onPressed: () => close(_ProjectMenuAction.addAudioOnly),
              child: Text(l10n.addAudioOnly),
            ),
          // **Proxies, on the item's own menu** (K-501, docs/07 §3.3). Four
          // commands and no dialogue: attach a file, make one, read from it or
          // not, forget it. Offered on footage alone — a comp and a folder
          // have no media reference for a stand-in to stand in for — and the
          // last two only once there is a proxy, so the menu never lists a
          // word that would do nothing.
          if (isFootage) ...[
            MenuRow(
              key: const ValueKey('project-menu-set-proxy'),
              onPressed: () => close(_ProjectMenuAction.setProxy),
              child: Text(l10n.setProxyEllipsis),
            ),
            MenuRow(
              key: const ValueKey('project-menu-make-proxy'),
              onPressed: () => close(_ProjectMenuAction.makeProxy),
              child: Text(l10n.makeProxy),
            ),
            if (proxy != null) ...[
              // Ticked, in the shape the layer menu's Accepts lights uses
              // (K-483): a word says what the tick means, where a glyph could
              // not.
              MenuRow(
                key: const ValueKey('project-menu-use-proxy'),
                onPressed: () => close(_ProjectMenuAction.useProxy),
                child: Row(
                  children: [
                    SizedBox(
                        width: 16,
                        child: proxy.enabled ? const Text('✓') : null),
                    Expanded(child: Text(l10n.useProxy)),
                  ],
                ),
              ),
              MenuRow(
                key: const ValueKey('project-menu-clear-proxy'),
                onPressed: () => close(_ProjectMenuAction.clearProxy),
                child: Text(l10n.clearProxy),
              ),
            ],
          ],
          // **What colour space this footage arrived in** (K-490,
          // docs/impl/ocio.md §6.5). A submenu rather than a row, because the
          // answer is a name out of the project's colour config and there may
          // be forty of them. This is the smallest honest surface until
          // *Interpret footage…* exists as drawn (docs/07 §3.2), and it is
          // replaced when that dialogue lands.
          //
          // The names are the config's own and cross verbatim (K-303). A name
          // assigned while a config that has since gone was loaded is still
          // listed, ticked, because it is the user's statement about the file
          // and the menu must not pretend it was never made.
          if (item case ItemReference_Footage(:final field0))
            SubmenuRow(
              key: const ValueKey('project-menu-colour-space'),
              closeParent: () => close(null),
              submenu: (dismiss) => FloatSurface(
                width: 210,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    // The built-in interpretation: what a file says about
                    // itself, which is where every footage item starts.
                    MenuRow(
                      key: const ValueKey('project-menu-colour-space-none'),
                      onPressed: () {
                        dismiss();
                        field0.setColourSpace();
                        onLocalEdit();
                      },
                      child: Row(children: [
                        SizedBox(
                            width: 16,
                            child:
                                colourSpace == null ? const Text('✓') : null),
                        Expanded(child: Text(l10n.colourSpaceFromFile)),
                      ]),
                    ),
                    if (colourSpaces.isNotEmpty ||
                        (colourSpace != null &&
                            !colourSpaces.contains(colourSpace)))
                      Padding(
                        padding: const EdgeInsets.fromLTRB(10, 6, 10, 2),
                        child: Text(l10n.colourSpaceFromConfig,
                            style: menuTheme.small
                                .copyWith(color: menuTheme.textMuted)),
                      ),
                    for (final space in [
                      if (colourSpace != null &&
                          !colourSpaces.contains(colourSpace))
                        colourSpace,
                      ...colourSpaces,
                    ])
                      MenuRow(
                        key: ValueKey<String>(
                            'project-menu-colour-space-$space'),
                        onPressed: () {
                          dismiss();
                          field0.setColourSpace(space: space);
                          onLocalEdit();
                        },
                        child: Row(children: [
                          SizedBox(
                              width: 16,
                              child: space == colourSpace
                                  ? const Text('✓')
                                  : null),
                          Expanded(child: Text(space)),
                        ]),
                      ),
                  ],
                ),
              ),
              child: Text(l10n.colourSpace),
            ),
          // The colour tag, as the strip itself rather than a submenu: the
          // chips ARE the choice, so putting them on the menu row costs one
          // click where a submenu costs two and a hover in between. The same
          // shape the Timeline's layer swatch offers, and the same palette.
          if (onSetLabel != null)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(l10n.label, style: menuTheme.small),
                  const SizedBox(width: 6),
                  for (var i = 0; i < LumitTheme.labelCount; i++)
                    GestureDetector(
                      key: ValueKey<String>('project-menu-label-$i'),
                      onTap: () {
                        onSetLabel(i);
                        close(null);
                      },
                      child: Container(
                        width: 10,
                        height: 10,
                        margin: const EdgeInsets.only(right: 2),
                        decoration: BoxDecoration(
                          color: menuTheme.labelColour(i),
                          shape: BoxShape.circle,
                          border: i == label
                              ? Border.all(color: menuTheme.textPrimary)
                              : null,
                        ),
                      ),
                    ),
                ],
              ),
            ),
          // Filing, the mouse way — the drag onto a folder row is the other
          // one. A submenu rather than a dialogue: the folders are a short
          // list the panel already knows, so picking one is one hover and one
          // click, the same shape Effects & presets offers its categories in.
          if (onMoveToFolder != null && folders.isNotEmpty)
            SubmenuRow(
              key: const ValueKey('project-menu-move-to-folder'),
              closeParent: () => close(null),
              submenu: (dismiss) => FloatSurface(
                width: 210,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (final (name, folder) in folders)
                      MenuRow(
                        key: ValueKey<String>(
                            'project-menu-folder-${_idOf(folder)}'),
                        onPressed: () {
                          dismiss();
                          onMoveToFolder(folder);
                        },
                        child: Text(name),
                      ),
                  ],
                ),
              ),
              child: Text(l10n.moveToFolder),
            ),
          MenuRow(
            onPressed: () => close(_ProjectMenuAction.moveToRoot),
            child: Text(l10n.moveToRoot),
          ),
          MenuRow(
            onPressed: () => close(_ProjectMenuAction.delete),
            child: Text(l10n.delete),
          ),
        ],
      ),
    ),
  );
  if (action == null) return;

  if (!context.mounted) return;
  switch (action) {
    case _ProjectMenuAction.compSettings:
      if (item case ItemReference_Composition(:final field0)) {
        // Reachable now that the dialog takes a CompositionReference rather than
        // an AppStateStub; the port had to drop this entry until it did.
        if (await showCompSettingsFrb(context: context, comp: field0)) {
          onLocalEdit();
        }
      }
    case _ProjectMenuAction.rename:
      onStartRename?.call();
    case _ProjectMenuAction.relink:
      await onRelink?.call();
    case _ProjectMenuAction.findMissing:
      onFindMissing();
    case _ProjectMenuAction.addAudioOnly:
      if (item case ItemReference_Footage(:final field0)) {
        openComp?.addAudioLayer(footage: field0);
        onLocalEdit();
      }
    case _ProjectMenuAction.setProxy:
      if (item case ItemReference_Footage(:final field0)) {
        final path = proxyPicker != null
            ? await proxyPicker()
            : await pickFootage()
                .then((paths) => paths.isEmpty ? null : paths.first);
        if (path == null) return;
        field0.setProxy(path: path);
        onLocalEdit();
      }
    case _ProjectMenuAction.makeProxy:
      if (item case ItemReference_Footage(:final field0)) {
        // The engine's own refusals — one transcode at a time, and nothing to
        // read from on this machine — reach the status line as its notice,
        // rather than as an exception out of a menu handler.
        try {
          field0.makeProxy();
        } catch (e) {
          if (context.mounted) {
            Provider.of<LumitState>(context, listen: false)
                .postNotice(l10n.proxyFailed('$e'), error: true);
          }
          return;
        }
        // The transcode reports on the status line, where every other piece of
        // background work does; this is the start signal that gets the strip
        // polling. The finished file attaches itself on the poll that sees it
        // land, and the item scope of that op is what brings this panel back.
        proxyJobChanged.value++;
      }
    case _ProjectMenuAction.useProxy:
      if (item case ItemReference_Footage(:final field0)) {
        field0.setUseProxy(on_: !(proxy?.enabled ?? false));
        onLocalEdit();
      }
    case _ProjectMenuAction.clearProxy:
      if (item case ItemReference_Footage(:final field0)) {
        field0.clearProxy();
        onLocalEdit();
      }
    case _ProjectMenuAction.moveToRoot:
      item.moveToRoot();
      onLocalEdit();
    case _ProjectMenuAction.delete:
      // No confirmation: it is one undo step, matching egui.
      item.delete();
      onLocalEdit();
  }
}
