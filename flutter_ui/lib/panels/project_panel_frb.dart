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
// The "show only missing" filter moved onto the bottom bar's count, where the
// mockup writes `10 items · 1 missing`.
//
// **What the panel reads and when.** The handles *are* the identity: a row
// holds an `ItemReference` and calls `rename`/`delete`/`moveToRoot` straight on
// it. Everything a row *draws* — its name, its column values, its badge — is
// handed to it by the panel's own walk, and every engine answer that walk needs
// (a status probe, a media probe, a comp's settings, a folder's child count) is
// cached until the document changes. That is what keeps a hover, which rebuilds
// one row, costing nothing at the bridge (the budget test expects zero).

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/drag_payloads.dart';
import '../shell/comp_settings_frb.dart';
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

/// The column-header row — a secondary row's 18, plus its own hairline.
const double projectColumnHeaderHeight = 19;

/// One item row. An outline row, and so 22 (§12A.6).
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

/// The gap between every element in a row, headers included.
const double projectRowGap = 8;

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
const double _widthForPreview = 300;
const double _widthForItems = 300;
const double _widthForFps = 230;
const double _widthForSize = 190;

/// Which optional columns a given panel width can carry.
class ProjectColumns {
  final bool items;
  final bool size;
  final bool fps;

  const ProjectColumns({
    required this.items,
    required this.size,
    required this.fps,
  });

  factory ProjectColumns.forWidth(double width) => ProjectColumns(
        items: width >= _widthForItems,
        size: width >= _widthForSize,
        fps: width >= _widthForFps,
      );

  /// The trailing cells of a row or of the header — the same widths, the same
  /// gaps, the same right edge, so a value lands under its heading. The owner
  /// corrected this alignment twice in the mockup rounds; building both sides
  /// from one function is what stops it drifting again.
  List<Widget> cells({
    String? items,
    String? size,
    String? fps,
    required TextStyle style,
  }) =>
      [
        if (this.items) ..._cell(projectItemsColumn, items, style),
        if (this.size) ..._cell(projectSizeColumn, size, style),
        if (this.fps) ..._cell(projectFpsColumn, fps, style),
      ];

  static List<Widget> _cell(double width, String? text, TextStyle style) => [
        const SizedBox(width: projectRowGap),
        SizedBox(
          width: width,
          child: text == null
              ? null
              : Text(text,
                  style: style,
                  textAlign: TextAlign.right,
                  maxLines: 1,
                  overflow: TextOverflow.clip),
        ),
      ];
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
  const _Cells({this.items, this.size, this.fps});
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
  }

  void _onSearchRequested() {
    if (!mounted) return;
    if (_boundUi?.searchRequestIsFor(Panel.project) ?? false) {
      _searchFocus.requestFocus();
    }
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _boundUi?.panelSearchRequest.removeListener(_onSearchRequested);
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
    final cols = ProjectColumns.forWidth(width);

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
          _footer(t, items: 0, missing: 0, labels: width >= _widthForItems),
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
      final searchHit = selfMatched ||
          (item is ItemReference_Folder && _subtreeMatches(item));
      // Missing-only is matched on the row's own name alone (docs/07 §3.3).
      final show = missingOnly ? isMissingFootage && ownMatch : searchHit;
      if (show) {
        _visibleIds.add(id);
        rows.add(_ProjectRowFrb(
          key: ValueKey<String>('project-row-$id'),
          item: item,
          name: name,
          depth: depth,
          missing: isMissingFootage,
          audio: _mediaInfo[id]?.width == 0,
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
      if (item is ItemReference_Folder) {
        final children = item.field0.getChildren();
        _childCounts[id] = children.length;
        if (_search.isNotEmpty || !_closedFolders.contains(id)) {
          for (final child in children) {
            walk(child, depth + 1, selfMatched);
          }
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
            labels: width >= _widthForItems),
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
      case ItemReference_Footage():
        if (missing) return const _Cells(size: _noValue, fps: _noValue);
        final info = _mediaInfo[id];
        if (info == null || info.width == 0) return const _Cells();
        return _Cells(
          size: '${info.width}×${info.height}',
          fps: _rateText(info.fpsNum, info.fpsDen),
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
            child: HouseTextField(
              key: const ValueKey('project-search'),
              controller: _searchController,
              focusNode: _searchFocus,
              width: double.infinity,
              // The well fills its row rather than floating inside it: the
              // mockup renders it exactly 20 tall, and the default 3px above
              // and below would burst that.
              padding: const EdgeInsets.symmetric(horizontal: 6),
              hint: l10n.searchProject,
            ),
          ),
        ),
      );

  /// The column headings. Kicker words, and the values below them are laid out
  /// by the same [ProjectColumns.cells] call, so they cannot come apart.
  Widget _columnHeader(LumitTheme t, ProjectColumns cols) => Container(
        key: const ValueKey('project-column-header'),
        height: projectColumnHeaderHeight,
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: t.hairline)),
        ),
        padding: const EdgeInsets.only(
            left: _headerPadLeft, right: projectRowPadding),
        child: Row(
          children: [
            Expanded(child: Text(l10n.name.toUpperCase(), style: t.kicker)),
            ...cols.cells(
              items: l10n.projectColumnItems.toUpperCase(),
              size: l10n.projectColumnSize.toUpperCase(),
              fps: l10n.unitFps.toUpperCase(),
              style: t.kicker,
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
  /// rather than being dropped. The count's `· n missing` half is the
  /// "show only missing" filter, which the mockup likewise has no row for.
  Widget _footer(LumitTheme t,
      {required int items, required int missing, required bool labels}) {
    final active = _missingOnly && missing > 0;
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
              label: labels ? l10n.projectFooterImport : null,
              onPressed: _import,
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
          const Spacer(),
          // The count never pushes the bar wider than the panel: it is the
          // flexible text the width ladder's first step truncates (§12A.6).
          Flexible(
            child: Text(l10n.projectItemCount(items),
                style: count, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
          if (missing > 0)
            LumitTooltip(
              message: active ? l10n.tipShowEverything : l10n.tipMissingOnly,
              child: GestureDetector(
                key: const ValueKey('missing-toggle'),
                behavior: HitTestBehavior.opaque,
                onTap: () => setState(() => _missingOnly = !_missingOnly),
                child: Text(
                  ' · ${l10n.projectMissingCount(missing)}',
                  style:
                      count.copyWith(color: active ? t.warning : t.textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
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
            lumitIcon(icon, size: iconSize, color: t.textMuted),
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
              // The card's second fact line. The mockup fills it with the
              // container and audio layout, which the engine does not report
              // yet; the kind of thing this is, is what the panel can say
              // truthfully in the same breath.
              Text(type, style: _metaStyle(t)),
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
          // Audio has no frames worth counting, so its last field is
          // milliseconds rather than a frame number.
          line = info.width > 0
              ? '${info.width}×${info.height} · '
                  '${fps.toStringAsFixed(2)} ${l10n.unitFps}'
                  ' · ${timecodeOfRate(frames, info.fpsNum, info.fpsDen)}'
              : '${l10n.projectInfoAudio} · ${timecodeOfSecondsMs(seconds)}';
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

  /// Sound with no picture — the media probe's own answer (zero width), which
  /// is what picks the speaker glyph over the film one.
  final bool audio;
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
  final Future<String?> Function()? relinkPicker;

  const _ProjectRowFrb({
    super.key,
    required this.item,
    required this.name,
    required this.depth,
    required this.missing,
    required this.audio,
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
              onRelink: item is ItemReference_Footage
                  ? () => _doRelink((item as ItemReference_Footage).field0)
                  : null,
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
                Expanded(child: _nameOrEditor(t)),
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
                  style: _metaStyle(t),
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
    return row;
  }

  /// The twirl's slot. A shut folder has to say so, or it reads as an empty
  /// one; the caret is its own target as well, the way the Hierarchy's is —
  /// and every row keeps the slot whether or not it has one, so a child still
  /// lines up one indent step right of the folder holding it.
  Widget _twirl(LumitTheme t) => SizedBox(
        width: iconSize,
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
                  size: iconSize,
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
  Widget _glyph(LumitTheme t) {
    final (icon, tint) = _iconFor(item, t);
    return lumitIcon(
      widget.missing ? LumitIcon.unlink : icon,
      size: iconSize,
      color: widget.missing ? t.warning : tint,
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
}) async {
  final isFootage = item is ItemReference_Footage;
  final isComp = item is ItemReference_Composition;
  // The comp the sound would land in (K-435). Read once, here, rather than in
  // the row's build: the menu is a gesture, not a rebuild path.
  final openComp =
      Provider.of<LumitUiState>(context, listen: false).selectedComp;
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
    case _ProjectMenuAction.moveToRoot:
      item.moveToRoot();
      onLocalEdit();
    case _ProjectMenuAction.delete:
      // No confirmation: it is one undo step, matching egui.
      item.delete();
      onLocalEdit();
  }
}
