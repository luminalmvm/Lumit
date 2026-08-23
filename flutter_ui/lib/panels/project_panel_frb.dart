// The Project panel, on the flutter_rust_bridge API — the first full panel port.
//
// Top to bottom: a search field that filters the tree live, an info header
// reading out the selected item (thumbnail, dimensions, rate, length), the
// Import / New composition glyph buttons, then one row per document item with
// folders nesting their children. A click selects the instant the button goes
// down; a click on the lone selected row *opens* it immediately — which makes a
// double-click "select, then open" in one motion, and what opening means is the
// item's own answer (K-243): a comp fronts, footage raises New composition on
// it, a folder shows or hides its children. Renaming is on the row menu. A
// right-click raises that menu; footage and comp rows drag onto the Timeline (a
// comp lands as a Precomp layer); double-clicking empty space imports. Missing footage wears a badge with an inline Relink… button, and a
// "show only missing" toggle appears while anything is missing. Rows carry
// their type glyph; the decoded thumbnail lives in the info header.
//
// **What changed from the v0 panel, and why it is shorter.** v0 read one big
// snapshot, mirrored it into `BridgeItem` trees, and addressed every edit by UUID
// string through `AppStateStub`. Here the handles *are* the identity: a row holds
// an `ItemReference` and calls `rename`/`delete`/`moveToRoot` straight on it, so
// there is no snapshot to diff, no mirror class to keep in step, and no id
// lookup. The thumbnail is the clearest case — v0 needed an isolate, a wire
// protocol and a generation map to keep a cold FFmpeg decode off the UI thread;
// `FootageReference.thumbnail` is simply async, decoded once per item into a
// RAM cache the info header draws from.

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

/// The longer edge the info header's thumbnail is decoded at: ~64 logical px
/// at 2× for crispness on a high-DPI display.
const int _thumbMaxEdge = 128;

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

/// How far each nesting level indents a row.
const double _indentPerDepth = 14;

class ProjectPanelFrb extends StatefulWidget {
  /// The relink file picker seam (chosen path, or null when cancelled). Defaults
  /// to the real footage picker; tests inject their own so no plugin channel
  /// opens.
  final Future<String?> Function()? relinkPicker;

  /// The import picker seam, for the footer button and the double-click. Same
  /// reason: a widget test must never open a plugin channel.
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

  /// Every item drawn this build, by id — what the info header looks the
  /// anchor up in. Rebuilt with the rows.
  final Map<String, ItemReference> _itemById = {};

  /// Decoded media facts per footage id, for the info header's readout.
  /// Cached because `mediaInfo` probes the file; cleared with the epoch.
  final Map<String, BridgeMediaInfo?> _mediaInfo = {};

  /// Decoded poster frames by footage id, held in RAM for the session so the
  /// info header never re-decodes for a selection change. A null entry claims
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
  /// info header describes one thing. Deselected (a toggle off) or unknown
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
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final state = Provider.of<LumitState>(context);
    final roots = state.project?.getItems() ?? const <ItemReference>[];

    if (roots.isEmpty) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _searchBar(t),
          _infoHeader(t),
          _toolbar(t),
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
        ],
      );
    }

    _refreshMissing(roots);

    // The filter only bites while something is missing, so a healthy project can
    // never trap the user behind an empty "missing only" view.
    final anyMissing = _missing.values.any((m) => m);
    final missingOnly = _missingOnly && anyMissing;

    final rows = <Widget>[];
    _visibleIds.clear();

    // A row shows when its own name matches, or an ancestor folder's did —
    // searching a folder finds what it holds (docs/07 §3.1). Missing-only is
    // stricter: it is never widened by a folder name, so every visible row is
    // something to fix (docs/07 §3.3).
    void walk(ItemReference item, int depth, bool ancestorMatched) {
      final id = _idOf(item);
      _itemById[id] = item;
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
          selected: _selectedIds.contains(id),
          renaming: _renamingId == id,
          selectionCount: _selectedIds.length,
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
        // Decoded ahead of selection and held in RAM, so the info header
        // shows the picture and the facts the instant a row is clicked.
        // Poster frames are ~48 px, so even a large project holds
        // kilobytes, not megabytes.
        _refreshThumb(field0);
        _refreshMediaInfo(field0);
      }
      // A closed folder keeps its children to itself — unless a search is
      // running, which has to be able to find what is inside one.
      if (item is ItemReference_Folder &&
          (_search.isNotEmpty || !_closedFolders.contains(id))) {
        for (final child in item.field0.getChildren()) {
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
        _searchBar(t),
        _infoHeader(t),
        if (anyMissing)
          _MissingHeaderFrb(
            count: _missing.values.where((m) => m).length,
            active: missingOnly,
            onToggle: () => setState(() => _missingOnly = !_missingOnly),
          ),
        _toolbar(t),
        Expanded(
          // Wrapping the list rather than sitting behind it: a sibling under a
          // ListView never sees a pointer, because the list is opaque across
          // its whole extent. As the parent it gets what the rows leave — and
          // a row's own double-tap wins the arena on the row itself.
          child: _importOnDoubleTap(
            child: ListView(
              padding: const EdgeInsets.symmetric(vertical: 4),
              children: rows,
            ),
          ),
        ),
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

  Widget _searchBar(LumitTheme t) => Padding(
        padding: const EdgeInsets.all(6),
        child: HouseTextField(
          key: const ValueKey('project-search'),
          controller: _searchController,
          focusNode: _searchFocus,
          width: double.infinity,
          hint: l10n.searchProject,
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

  /// Import and New composition as glyph buttons above the tree, where the
  /// egui panel kept them.
  ///
  /// They are on the menu bar too, and that is not duplication worth removing:
  /// the panel is where you are looking when you want them, and a panel that
  /// can only show what someone else put in it is a dead end.
  Widget _toolbar(LumitTheme t) => Container(
        height: 24,
        padding: const EdgeInsets.symmetric(horizontal: 6),
        decoration: BoxDecoration(
          border: Border(top: BorderSide(color: t.hairline)),
        ),
        child: Row(
          children: [
            LumitTooltip(
              message: l10n.importFootage,
              child: HouseButton(
                key: const ValueKey('project-import'),
                small: true,
                frameless: true,
                onPressed: _import,
                child: lumitIcon(LumitIcon.folder,
                    size: iconSize, color: t.textMuted),
              ),
            ),
            const SizedBox(width: 4),
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
                  child: HouseButton(
                    key: const ValueKey('project-new-comp'),
                    small: true,
                    frameless: true,
                    onPressed: _newComposition,
                    child: lumitIcon(LumitIcon.comp,
                        size: iconSize, color: t.textMuted),
                  ),
                ),
              ),
            ),
          ],
        ),
      );

  /// The height the info header always occupies: a 36px thumbnail plus its
  /// padding. Constant whether or not anything is selected, so the tree below
  /// never jumps when the selection changes.
  static const double _infoHeaderHeight = 48;

  /// The selected item's readout (docs/07 §3.1): thumbnail, name, type, and
  /// the item's own vital statistics. Always present at a fixed height; with
  /// nothing selected it is simply quiet.
  Widget _infoHeader(LumitTheme t) {
    final id = _anchorId;
    final item = id != null && _selectedIds.contains(id) ? _itemById[id] : null;

    return SizedBox(
      height: _infoHeaderHeight,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(8, 2, 8, 8),
        child: item == null || id == null
            ? const SizedBox.expand()
            : _infoHeaderContent(t, item, id),
      ),
    );
  }

  Widget _infoHeaderContent(LumitTheme t, ItemReference item, String id) {
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
        width: 64,
        height: 36,
        child: image == null
            ? Center(
                child: lumitIcon(LumitIcon.footage,
                    size: iconSize, color: t.layer.footage))
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
      children: [
        if (thumb != null) ...[thumb, const SizedBox(width: 8)],
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(
                children: [
                  Flexible(
                    child: Text(_nameOf(item),
                        style: t.bodyPrimary, overflow: TextOverflow.ellipsis),
                  ),
                  const SizedBox(width: 6),
                  Text(type, style: t.small.copyWith(color: t.textMuted)),
                ],
              ),
              _infoLine(t, item, id, missing),
            ],
          ),
        ),
      ],
    );
  }

  /// The header's second line: the facts this item can truthfully state. The
  /// length reads as `HH:MM:SS:FF` timecode at the item's own rate — the same
  /// clock face the Viewer shows — never as a bare frame count.
  Widget _infoLine(LumitTheme t, ItemReference item, String id, bool missing) {
    String? line;
    switch (item) {
      case ItemReference_Footage():
        if (missing) {
          return Text(l10n.projectItemMissing,
              style: t.small.copyWith(color: t.warning));
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
    return Padding(
      padding: const EdgeInsets.only(top: 1),
      child: Text(line,
          key: const ValueKey('project-info-line'),
          style: t.small.copyWith(color: t.textMuted),
          overflow: TextOverflow.ellipsis),
    );
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

/// The header shown while the project has missing footage: a count and a
/// "show only missing" toggle.
class _MissingHeaderFrb extends StatelessWidget {
  final int count;
  final bool active;
  final VoidCallback onToggle;
  const _MissingHeaderFrb({
    required this.count,
    required this.active,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: active ? l10n.tipShowEverything : l10n.tipMissingOnly,
      child: GestureDetector(
        key: const ValueKey('missing-toggle'),
        behavior: HitTestBehavior.opaque,
        onTap: onToggle,
        child: Container(
          height: 24,
          color: active ? t.accent.withValues(alpha: 0.12) : t.surface1,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: Row(
            children: [
              lumitIcon(LumitIcon.unlink, size: iconSize, color: t.warning),
              const SizedBox(width: 6),
              Text(
                l10n.missingFileCount(count),
                style: t.small.copyWith(color: t.warning),
              ),
            ],
          ),
        ),
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
  final bool selected;
  final bool renaming;

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
    required this.selected,
    required this.renaming,
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
            constraints: const BoxConstraints(minHeight: 22),
            color: widget.selected
                ? t.surface2
                : _hover
                    ? t.surface4
                    : null,
            padding: EdgeInsets.only(
              left: 6 + widget.depth * _indentPerDepth,
              right: 6,
            ),
            child: Row(
              children: [
                _leading(t),
                const SizedBox(width: 6),
                Expanded(child: _nameOrEditor(t)),
                if (widget.missing) ...[
                  const SizedBox(width: 6),
                  Text(l10n.projectItemMissing,
                      style: t.small.copyWith(color: t.warning)),
                  const SizedBox(width: 6),
                  LumitTooltip(
                    message: l10n.relink,
                    child: HouseButton(
                      key: ValueKey<String>('relink-${_idOf(item)}'),
                      small: true,
                      onPressed: () =>
                          _doRelink((item as ItemReference_Footage).field0),
                      child: Text(l10n.relinkEllipsis, style: t.small),
                    ),
                  ),
                ],
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

  /// The row's type glyph. Missing footage wears the warning-tinted unlink
  /// glyph. No thumbnail here — the info header carries the picture, so the
  /// tree stays a tight list of names.
  Widget _leading(LumitTheme t) {
    final (icon, tint) = _iconFor(item, t);
    final glyph = lumitIcon(
      widget.missing ? LumitIcon.unlink : icon,
      size: iconSize,
      color: widget.missing ? t.warning : tint,
    );
    // A shut folder has to say so, or it reads as an empty one. The caret is
    // its own target as well, the way the Hierarchy's is — and every row keeps
    // the slot whether or not it has one, so a child still lines up one indent
    // step right of the folder holding it.
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
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
        ),
        const SizedBox(width: 4),
        glyph,
      ],
    );
  }

  Widget _nameOrEditor(LumitTheme t) {
    final controller = _rename;
    if (widget.renaming && controller != null) {
      return Container(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
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
    return Text(widget.name, style: t.body, overflow: TextOverflow.ellipsis);
  }

  (LumitIcon, Color) _iconFor(ItemReference item, LumitTheme t) =>
      switch (item) {
        ItemReference_Footage() => (LumitIcon.footage, t.layer.footage),
        ItemReference_Folder() => (LumitIcon.folder, t.textMuted),
        ItemReference_Composition() => (LumitIcon.comp, t.layer.precomp),
        ItemReference_Solid() => (LumitIcon.solid, t.layer.solid),
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
                color:
                    icon == LumitIcon.comp ? t.layer.precomp : t.layer.footage),
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
