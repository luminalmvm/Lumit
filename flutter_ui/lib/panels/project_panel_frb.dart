// The Project panel, built to the approved redesign mockup.
//
// **In plain terms**: this is the shelf the project's things live on. Top to
// bottom the mockup lays it out as a preview card for whatever is picked, a
// search well, a row of column headings, the tree of items, a thin horizontal
// scrollbar, and a bottom bar carrying the new-item controls at the left and a
// factual count at the right.
//
// **Where the panel's parts live.** This file is the panel itself — the walk
// of the document, the caches that walk fills, the selection, and the
// commands. The furniture is `project_chrome_frb.dart` (preview card, search
// row, column headings, scrollbar, bottom bar), one tree line is
// `project_row_frb.dart`, the right-click menu is `project_menu_frb.dart`, and
// the measurements and column arithmetic every one of them reads are in
// `project_columns_frb.dart`. Those four are re-exported from here, so
// everything that imported this panel still sees one library.
//
// **Behaviour is unchanged from the panel this replaces.** A click selects the
// instant the button goes down; a click on the lone selected row *opens* it,
// which makes a double-click "select, then open" in one motion, and what
// opening means is the item's own answer: a comp fronts, footage
// raises New composition on it, a folder shows or hides its children. Renaming
// is `Enter` or the row menu. A right-click raises that menu; footage and comp
// rows drag onto the Timeline (a comp lands as a Precomp layer);
// double-clicking empty space imports, and files dragged in from the OS file
// manager import the same way — the panel lights up while they hover.
// Missing footage wears the mockup's
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
import 'dart:io';
import 'dart:ui' as ui;

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/scheduler.dart';
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

import '../l10n/strings.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'project_chrome_frb.dart';
import 'project_columns_frb.dart';
import 'project_row_frb.dart';

export 'project_chrome_frb.dart';
export 'project_columns_frb.dart';
export 'project_row_frb.dart';

/// The longer edge the preview card's thumbnail is decoded at: the card draws
/// it 96 logical px wide, so ~2× for crispness on a high-DPI display.
const int _thumbMaxEdge = 224;

/// Put what the OS file manager dropped on the Project panel down the road it
/// belongs on, and say whether anything was imported.
///
/// **In plain terms**: dropping files on the panel is the same command as
/// **File › Import footage**, so it goes through the same call — one undo step
/// for the whole batch, the probe worker started per file, image sequences
/// spotted by the engine rather than here.
///
/// Three shapes arrive and each is answered differently:
///
/// * a **folder** is read one level deep for the files the import filter
///   offers, sorted, and handed over as if they had been picked — which is how
///   a folder of numbered stills becomes one image-sequence item: the engine
///   sees the run and folds it, and folds every later frame of the same run
///   into the item it already made;
/// * a **`.lum`** or an After Effects **`.aep`/`.zip`** is *not* opened
///   quietly. Opening a project throws away the one on screen, and importing
///   an After Effects project is a long conversion with a report at the end;
///   neither should happen because something landed on a panel. The status
///   line names the menu road instead;
/// * anything else is handed to the engine as footage. The list above is the
///   *dialogue's* filter, not the engine's — a format the picker does not
///   think to offer still imports, and a file the engine cannot read wears the
///   panel's own missing badge, which is a truer answer than silence.
Future<bool> importDroppedPaths(LumitState state, List<String> dropped) async {
  final media = <String>[];
  var isProject = false;
  var isAe = false;
  for (final path in dropped) {
    final folder = Directory(path);
    if (folder.existsSync()) {
      media.addAll(folder
          .listSync()
          .whereType<File>()
          .map((f) => f.path)
          .where((p) => footageExtensions.contains(_extensionOf(p)))
          .toList()
        ..sort());
      continue;
    }
    switch (_extensionOf(path)) {
      case 'lum':
        isProject = true;
      case 'aep':
      case 'zip':
        isAe = true;
      default:
        media.add(path);
    }
  }
  if (isProject) state.postNotice(l10n.dropProjectFile);
  if (isAe) state.postNotice(l10n.dropAeProject);
  return state.importFootagePaths(media);
}

String _extensionOf(String path) {
  final dot = path.lastIndexOf('.');
  return dot < 0 ? '' : path.substring(dot + 1).toLowerCase();
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
    // Enter renames the lone selected item — the same key the
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
    // `Ctrl+A` selects every item this panel is showing, asked for the
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
  /// console, through [_publishSelection].
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
  final Map<String, ProjectCells> _compCells = {};

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

  /// Each item's name, remembered until the document changes. Reading one is
  /// the dearest question this panel asks - the engine clones the whole item to
  /// answer it - and it can only change when the document does.
  final Map<String, String> _names = {};

  /// A rebuild is already booked for the end of this frame.
  ///
  /// The three probes this panel fires - the poster frame, the container's
  /// facts, and whether the file is still on disc - each answer whenever they
  /// answer, and each answer used to call `setState` on its own. Opening a
  /// folder of twenty clips therefore booked up to sixty full rebuilds of the
  /// panel, which is exactly the gesture that felt slowest and exactly the
  /// frames a scroll wants for itself.
  ///
  /// They write into their caches and ask for **one** rebuild instead. Nothing
  /// is lost: every one of them is read during the next build, so the answers
  /// arrive together rather than one frame apart.
  bool _rebuildBooked = false;

  /// Each footage item's proxy, or null where it has none — for the
  /// `proxy` badge and for the row menu's four commands. A document read like
  /// every other entry here, so it is asked once per document change and never
  /// in a rebuild: the budget test expects a hover to cost nothing.
  final Map<String, BridgeProxy?> _proxies = {};

  /// The project-wide *use proxies* switch, cached with the rest: it is
  /// a document read, so a rebuild must never ask for it again.
  bool? _useProxies;

  /// The colour the swatch filter is holding, or null for "show everything"
  /// (§12A.3a). Session state, like the search text and the shut folders: a
  /// filter is where you are looking, not something about the document.
  int? _labelFilter;

  /// Whether the tree is being narrowed at all — by a search term or by a
  /// held colour. A narrowed tree opens every folder, because what is being
  /// looked for is usually inside one.
  bool get _filtering => _search.isNotEmpty || _labelFilter != null;

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

  /// Mirror the anchor item to the shell, where the FX console reads
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

  /// The folders the user has shut, by id. Closed rather than open, so
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

  /// True while a drag from the OS file manager is over the panel, so it can
  /// wear the drop-target treatment (§6.5) the folder rows already wear.
  bool _dropHover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DropTarget(
      onDragEntered: (_) => setState(() => _dropHover = true),
      onDragExited: (_) => setState(() => _dropHover = false),
      onDragDone: (details) {
        setState(() => _dropHover = false);
        _dropped([for (final f in details.files) f.path]);
      },
      // Painted over the panel rather than behind it, for the reason the row
      // target gives: the panel draws its own fill, and a drop with no
      // feedback is indistinguishable from one that did nothing.
      child: Container(
        foregroundDecoration: !_dropHover
            ? null
            : BoxDecoration(
                border: Border.all(color: t.accent, width: 1.5),
                color: t.accent.withValues(alpha: 0.1),
              ),
        child: LayoutBuilder(
          builder: (context, box) => _build(context, box.maxWidth),
        ),
      ),
    );
  }

  Future<void> _dropped(List<String> paths) async {
    final state = Provider.of<LumitState>(context, listen: false);
    if (await importDroppedPaths(state, paths)) _documentChanged();
  }

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
          projectColumnHeader(t, cols, onResize: _resizeColumn),
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
    // `inherited` is the colour of the nearest ancestor folder that carries
    // one, 0 where no ancestor does (A12). A folder's tag tints what it holds,
    // and an item with a tag of its own overrides it — so what walks down is
    // the effective colour, which makes the nearest tagged ancestor the one
    // that wins without the walk having to look up the tree.
    void walk(
        ItemReference item, int depth, bool ancestorMatched, int inherited) {
      final id = projectItemId(item);
      _itemById[id] = item;
      itemCount++;
      // Cached like every other row fact beside it: nothing asks the engine
      // twice for an answer that only a document change can alter.
      // The name was the one that was not, and it is the dearest of them:
      // `ProjectItem::name` clones the whole item across the seam to read one
      // string, so a composition's every layer was copied per row per build.
      final name = _names[id] ??= _nameOf(item);
      final ownMatch = _search.isEmpty || name.toLowerCase().contains(_search);
      final selfMatched = ancestorMatched || ownMatch;
      final isMissingFootage =
          item is ItemReference_Footage && (_missing[id] ?? false);
      final label = _labels[id] ??= _labelOf(item);
      final searchHit = selfMatched ||
          (item is ItemReference_Folder && _subtreeMatches(item));
      // Sound with no picture at all — the probe's own answer, not the zero
      // picture width the panel used to infer it from. A silent
      // still has no sound and a picture that does not run; the old guess
      // called it audio.
      final audio = _mediaInfo[id] != null && _mediaInfo[id]!.videoCodec == null;
      // Missing-only is matched on the row's own name alone (docs/07 §3.3).
      // The swatch filter narrows *with* whatever else is running, and on the
      // colour the row is actually **wearing** — its own tag where it has one,
      // the nearest tagged folder's where it has not, and the
      // kind's own default tint where it has neither. Filing a shoot into a
      // red folder colours the shoot, so picking red has to find it — and a
      // just-imported clip visibly wears azure, so picking azure has to find
      // *it*: the filter used to stop at the inherited tag and missed every
      // untagged item showing its per-type colour.
      final worn = label != 0
          ? label
          : inherited != 0
              ? inherited
              : projectDefaultLabel(item, audio: audio);
      final chipHit = _labelFilter == null || worn == _labelFilter;
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
        rows.add(ProjectRowFrb(
          key: ValueKey<String>('project-row-$id'),
          item: item,
          name: name,
          depth: depth,
          missing: isMissingFootage,
          audio: audio,
          label: label,
          inherited: inherited,
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
          folderOpen: _filtering || !_closedFolders.contains(id),
          onToggleFolder: () => setState(() {
            if (!_closedFolders.remove(id)) _closedFolders.add(id);
          }),
          onLocalEdit: _documentChanged,
          onSetLabel: (picked) => _setLabel(item, picked),
          onMoveToFolder: (folder) => _fileInto(folder, _targets(item)),
          menuTargets: () => _targets(item),
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
        //
        // For rows that are actually drawn, though: a filter hiding a folder's
        // contents used to decode a poster frame for every clip inside it, and
        // each answer landed as its own rebuild of the whole panel.
        if (show) {
          _refreshThumb(field0);
          _refreshMediaInfo(field0);
        }
      }
      // A closed folder keeps its children to itself — unless a filter is
      // running, which has to be able to find what is inside one. A colour
      // counts as much as a search term here: the items wearing a folder's
      // colour are the ones inside it, so a shut folder would hide exactly
      // what was asked for.
      if (_filtering || !_closedFolders.contains(id)) {
        for (final child in children) {
          walk(child, depth + 1, selfMatched, label != 0 ? label : inherited);
        }
      }
    }

    _footageById.clear();
    _itemById.clear();
    for (final item in roots) {
      walk(item, 0, false, 0);
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (width >= projectWidthForPreview) _previewCard(t),
        _searchRow(t),
        projectColumnHeader(t, cols, onResize: _resizeColumn),
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
        projectScrollStrip(t, _hScroll),
        _footer(t,
            items: itemCount,
            missing: missingCount,
            width: width,
            project: state.project),
      ],
    );
  }

  /// The search well and its colour chips, with this panel's own controller,
  /// focus and held filter handed to the chrome that draws them.
  Widget _searchRow(LumitTheme t) => projectSearchRow(
        t,
        controller: _searchController,
        focus: _searchFocus,
        labelFilter: _labelFilter,
        onFilter: (picked) => setState(() => _labelFilter = picked),
      );

  /// The bottom bar. The proxies switch is a document read, so it comes off
  /// this panel's cache rather than being asked for again per build.
  Widget _footer(LumitTheme t,
          {required int items,
          required int missing,
          required double width,
          ProjectReference? project}) =>
      projectFooter(
        t,
        items: items,
        missing: missing,
        width: width,
        hasProject: project != null,
        useProxies: project != null && (_useProxies ??= project.useProxies()),
        missingOnly: _missingOnly,
        onToggleMissing: () => setState(() => _missingOnly = !_missingOnly),
        onImport: _import,
        onNewFolder: _newFolder,
        onNewComposition: _newComposition,
        onToggleProxies: () {
          if (project == null) return;
          project.setUseProxies(useProxies: !(_useProxies ?? false));
          _documentChanged();
        },
      );

  /// The preview card, fed from the caches the walk fills: the anchor item,
  /// its poster frame and its media facts, all already in RAM.
  Widget _previewCard(LumitTheme t) {
    final id = _anchorId;
    final item = id != null && _selectedIds.contains(id) ? _itemById[id] : null;
    return projectPreviewCard(
      t,
      item: item,
      name: item == null ? '' : (_names[id!] ??= _nameOf(item)),
      missing: item is ItemReference_Footage && (_missing[id] ?? false),
      thumb: id == null ? null : _thumbs[id],
      info: id == null ? null : _mediaInfo[id],
    );
  }

  /// Whether anything under this folder matches the needle, so a folder that
  /// holds a hit stays visible as the path to it.
  bool _subtreeMatches(ItemReference_Folder folder) {
    if (_search.isEmpty) return true;
    for (final child in folder.field0.getChildren()) {
      if ((_names[projectItemId(child)] ??= _nameOf(child))
          .toLowerCase()
          .contains(_search)) {
        return true;
      }
      if (child is ItemReference_Folder && _subtreeMatches(child)) return true;
    }
    return false;
  }

  /// The Size, fps and Items values this row can truthfully state, off the
  /// caches the walk fills. A row never works these out itself — it is handed
  /// finished strings, which is what keeps a hover free at the bridge.
  ProjectCells _cellsFor(ItemReference item, String id, bool missing) {
    switch (item) {
      case ItemReference_Footage(:final field0):
        // The path is what the *project* records, so it is worth stating even
        // for a file that is not there — it is where the item is pointing,
        // which is exactly what a relink is about to change.
        final path = _paths[id] ??= _pathOf(field0);
        if (missing) {
          return ProjectCells(
              size: projectNoValue, fps: projectNoValue, path: path);
        }
        final info = _mediaInfo[id];
        if (info == null) return ProjectCells(path: path);
        if (info.videoCodec == null) {
          // A sound file's cells, as the mockup writes them: the rate where a
          // picture would state its size, and the channel layout — shortened
          // to fit the FPS column — where a picture would state its rate.
          if (info.audioCodec == null) return ProjectCells(path: path);
          return ProjectCells(
            size: projectSampleRateText(info.sampleRate),
            fps: switch (info.channels) {
              1 => l10n.audioMonoShort,
              2 => l10n.audioStereoShort,
              final n => l10n.audioChannels(n),
            },
            path: path,
          );
        }
        return ProjectCells(
          size: '${info.width}×${info.height}',
          // A still has no rate to state. It probes with a video
          // stream of one frame, so a number *is* there — and printing it
          // would say the picture runs when it does not.
          fps: info.isStill ? null : projectRateText(info.fpsNum, info.fpsDen),
          path: path,
        );
      case ItemReference_Composition(:final field0):
        return _compCells[id] ??= () {
          final s = field0.getSettings();
          return ProjectCells(
            size: '${s.width}×${s.height}',
            fps: projectRateText(s.fpsNum, s.fpsDen),
          );
        }();
      case ItemReference_Folder():
        final n = _childCounts[id];
        return ProjectCells(items: n?.toString());
      case ItemReference_Solid():
        return const ProjectCells();
    }
  }

  /// Double-clicking the panel's blank space imports, which is the gesture
  /// every editor has and the one people reach for before finding a menu.
  Widget _importOnDoubleTap({required Widget child}) => GestureDetector(
        key: const ValueKey('project-empty-area'),
        behavior: HitTestBehavior.opaque,
        onDoubleTap: _import,
        child: child,
      );

  /// Fill in a footage item's media facts, off the build.
  void _refreshMediaInfo(FootageReference footage) {
    final id = footage.internalid.toString();
    if (_mediaInfo.containsKey(id)) return;
    // Claim the slot first, so a rebuild mid-probe does not probe twice.
    _mediaInfo[id] = null;
    footage.mediaInfo().then((info) {
      if (!mounted || info == null) return;
      _mediaInfo[id] = info;
      _bookRebuild();
    });
  }

  /// Ask for one rebuild at the end of this frame, however many probes have
  /// answered during it. Safe to call from a probe's `then`: it takes no
  /// account of what changed, because every cache it serves is read afresh by
  /// the next build.
  void _bookRebuild() {
    if (_rebuildBooked || !mounted) return;
    _rebuildBooked = true;
    SchedulerBinding.instance.addPostFrameCallback((_) {
      _rebuildBooked = false;
      if (mounted) setState(() {});
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
  /// reads the path the project records and touches no disk for it.
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
      _selectedIds.contains(projectItemId(item))
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
    final skip = projectItemId(excluding);
    void walk(ItemReference item) {
      if (item is! ItemReference_Folder) return;
      if (projectItemId(item) == skip) return;
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
      _names.clear();
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
          _thumbs[id] = image;
          _bookRebuild();
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
        final id = projectItemId(item);
        if (!_missing.containsKey(id)) {
          // Claim the slot first, so a rebuild mid-probe does not probe twice.
          _missing[id] = false;
          item.field0.getStatus().then((status) {
            if (!mounted) return;
            final isMissing = status == LumitMediaStatus.missing;
            if (_missing[id] != isMissing) {
              _missing[id] = isMissing;
              _bookRebuild();
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
