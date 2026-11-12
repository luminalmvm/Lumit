// The Library window: everything a person keeps in Lumit to use again, in
// one place. Presets, styles, themes, fonts, shortcut sets, lenses, flares,
// export presets and LUTs, each kind a folder on this computer. Free. With
// Lumit Pro the same library is on every computer of the account, and the
// window says which computer has what.
//
// The shape is the Settings window's: a title strip with the status and a
// search, the kinds down the left, the items in the middle grouped by folder,
// and the chosen item on the right with where it is, its history and its
// actions. Nothing here is lost by a slip: a delete, a rename, a restore or
// a settled conflict can be undone, and what cannot be undone asks first.

import 'dart:async';
import 'dart:math' as math;

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show revealInFolder;
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../state/app_state.dart';
import '../state/file_dialogs.dart';
import '../state/library.dart';
import '../state/ui_state.dart';
import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'dialog_frame.dart';
import 'library_parts.dart';
import 'pro_window.dart';
import 'profile_chip.dart' show ProBadge, SyncRing;
import 'project_history_window.dart' show backupWhen;
import 'settings_account.dart' show askBefore;

const Size libraryWindowSize = Size(980, 620);
const Size libraryMinSize = Size(760, 480);

const double _strip = 30;
const double _sidebarWidth = 172;
const double _detailWidth = 272;
const double _itemRow = 36;
const double _headerRow = 22;
const double _pad = dialogPadding;

/// A sidebar row and the accent tick down the one in force, as Settings
/// draws them.
const double _navHeight = 24;
const double _navTick = 2;

/// Open the Library window: on [kind] when one is named, with the item
/// whose key is [select] chosen and its details showing, or on Needs you
/// with [conflicts].
Future<void> showLibrary(BuildContext context,
    {LibraryKind? kind, String? select, bool conflicts = false}) {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  return showLumitModal<void>(
    context: context,
    id: 'library',
    initialSize: libraryWindowSize,
    minSize: libraryMinSize,
    builder: (close) => LibraryWindow(
      app: app,
      library: ui.library,
      kind: kind,
      select: select,
      conflicts: conflicts,
      close: () => close(null),
    ),
  );
}

/// Which list the middle shows: everything, one kind, the conflicts, or
/// what was recently deleted.
class _Page {
  final LibraryKind? kind;
  final bool needsYou, deleted;
  const _Page.all()
      : kind = null,
        needsYou = false,
        deleted = false;
  const _Page.kind(this.kind)
      : needsYou = false,
        deleted = false;
  const _Page.needsYou()
      : kind = null,
        needsYou = true,
        deleted = false;
  const _Page.deleted()
      : kind = null,
        needsYou = false,
        deleted = true;

  bool get all => kind == null && !needsYou && !deleted;

  @override
  bool operator ==(Object other) =>
      other is _Page &&
      other.kind == kind &&
      other.needsYou == needsYou &&
      other.deleted == deleted;

  @override
  int get hashCode => Object.hash(kind, needsYou, deleted);
}

/// A line of the middle: a folder's name, or an item.
class _Row {
  final String? header;
  final LibraryEntry? entry;
  const _Row.header(this.header) : entry = null;
  const _Row.item(this.entry) : header = null;
  double get height => entry == null ? _headerRow : _itemRow;
}

/// One thing that can be done to an item, for its buttons and its menu.
class _Act {
  final String id, label;
  final Future<void> Function() run;
  const _Act(this.id, this.label, this.run);
}

class LibraryWindow extends StatefulWidget {
  /// Where notices go. Null in a test that pumps the window bare.
  final LumitState? app;
  final LibraryState library;
  final LibraryKind? kind;

  /// The key of an item to open with chosen, and whether to open on the
  /// conflicts.
  final String? select;
  final bool conflicts;
  final VoidCallback close;

  const LibraryWindow({
    super.key,
    required this.app,
    required this.library,
    this.kind,
    this.select,
    this.conflicts = false,
    required this.close,
  });

  @override
  State<LibraryWindow> createState() => _LibraryWindowState();
}

class _LibraryWindowState extends State<LibraryWindow> {
  late _Page _page = _opening();
  int _pageTurns = 0;

  /// The page the window opens on: the conflicts, the chosen item's kind,
  /// the kind named, or everything.
  _Page _opening() {
    if (widget.conflicts) return const _Page.needsYou();
    final chosen = _entry(widget.select);
    if (chosen != null) {
      return chosen.gone != null
          ? const _Page.deleted()
          : _Page.kind(chosen.kind);
    }
    return widget.kind == null ? const _Page.all() : _Page.kind(widget.kind);
  }

  final _search = TextEditingController();
  final _searchFocus = FocusNode(debugLabel: 'library search');
  final _focus = FocusNode(debugLabel: 'library');
  final _scroll = ScrollController();

  /// The chosen items by key, the end a Shift range grows from, and the one
  /// the keyboard stands on.
  final Set<String> _selected = {};
  String? _anchor, _cursor;

  /// Whether the arrow keys walk the kinds rather than the list.
  bool _inSidebar = false;

  /// The item being renamed in its row, and the field it is typed in.
  String? _renaming;
  final _renameText = TextEditingController();
  final _renameFocus = FocusNode(debugLabel: 'library rename');

  /// Files held over the window.
  bool _dragging = false;

  /// Conflicts settled a moment ago, kept while their cards leave.
  final Map<String, LibraryConflict> _leaving = {};
  Set<String> _conflictKeys = {};

  VoidCallback? _escape;

  LibraryState get _library => widget.library;

  @override
  void initState() {
    super.initState();
    _library.addListener(_changed);
    _search.addListener(_changed);
    _conflictKeys = {for (final c in _library.conflicts) c.entry.key};
    _lastConflicts = _library.conflicts;
    if (widget.select case final key? when _entry(key) != null) {
      _selected.add(key);
      _anchor = _cursor = key;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        final e = _entry(key);
        if (e != null) _reveal(e, _rows(_listed()));
      });
    }
    // Escape clears the search, or drops a rename, before it closes the
    // window: the finest thing on screen goes first.
    _escape = EscapeLadder.register(EscapeRung.gesture, () {
      if (_renaming != null) {
        setState(() => _renaming = null);
        return true;
      }
      if (_search.text.isNotEmpty) {
        _search.clear();
        return true;
      }
      return false;
    });
  }

  @override
  void dispose() {
    _escape?.call();
    _library.removeListener(_changed);
    _search.dispose();
    _searchFocus.dispose();
    _focus.dispose();
    _scroll.dispose();
    _renameText.dispose();
    _renameFocus.dispose();
    super.dispose();
  }

  void _changed() {
    if (!mounted) return;
    setState(() {
      final now = {for (final c in _library.conflicts) c.entry.key};
      // A conflict that went leaves its card behind for a beat.
      for (final was in _conflictKeys.difference(now)) {
        final gone = _leavingOf(was);
        if (gone != null) _leaving[was] = gone;
      }
      _conflictKeys = now;
      _lastConflicts = _library.conflicts;
      if (_page.needsYou && now.isEmpty && _leaving.isEmpty) {
        WidgetsBinding.instance.addPostFrameCallback((_) => _afterLeaving());
      }
      // What is chosen keeps to what is still there.
      final keys = {
        for (final e in _library.entries) e.key,
        for (final e in _library.recentlyDeleted) e.key,
      };
      _selected.removeWhere((k) => !keys.contains(k));
      if (_cursor != null && !keys.contains(_cursor)) _cursor = null;
      if (_renaming != null && !keys.contains(_renaming)) _renaming = null;
    });
  }

  List<LibraryConflict> _lastConflicts = const [];
  LibraryConflict? _leavingOf(String key) {
    for (final c in _lastConflicts) {
      if (c.entry.key == key) return c;
    }
    return null;
  }

  // --- What is shown ---------------------------------------------------------

  bool _wanted(LibraryEntry e) {
    final q = _search.text.trim().toLowerCase();
    if (q.isEmpty) return true;
    return e.name.toLowerCase().contains(q) ||
        libraryDetail(e).toLowerCase().contains(q) ||
        e.path.toLowerCase().contains(q) ||
        libraryKindLabel(e.kind).toLowerCase().contains(q);
  }

  int _byName(LibraryEntry a, LibraryEntry b) {
    final g = (a.group ?? '').compareTo(b.group ?? '');
    if (g != 0) return g;
    return a.name.toLowerCase().compareTo(b.name.toLowerCase());
  }

  /// The items of the page in force, in the order they are listed.
  List<LibraryEntry> _listed() {
    if (_page.deleted) {
      return _library.recentlyDeleted.where(_wanted).toList()
        ..sort((a, b) => (b.gone ?? DateTime(0)).compareTo(a.gone ?? DateTime(0)));
    }
    final kind = _page.kind;
    return [
      for (final e in _library.entries)
        if ((kind == null || e.kind == kind) && _wanted(e)) e
    ]..sort((a, b) {
        final k = a.kind.index - b.kind.index;
        return k != 0 ? k : _byName(a, b);
      });
  }

  /// The lines of the middle: in All, a header per kind; in a kind, a
  /// header per folder, with the loose items first.
  List<_Row> _rows(List<LibraryEntry> items) {
    final rows = <_Row>[];
    String? last;
    var first = true;
    for (final e in items) {
      final head = _page.all ? libraryKindLabel(e.kind) : e.group;
      if (head != last || first) {
        if (head != null && (first || head != last)) rows.add(_Row.header(head));
        last = head;
        first = false;
      }
      rows.add(_Row.item(e));
    }
    return rows;
  }

  LibraryEntry? _entry(String? key) {
    if (key == null) return null;
    for (final e in _library.entries) {
      if (e.key == key) return e;
    }
    for (final e in _library.recentlyDeleted) {
      if (e.key == key) return e;
    }
    return null;
  }

  List<LibraryEntry> get _chosen =>
      [for (final k in _selected) if (_entry(k) case final e?) e];

  // --- Choosing -------------------------------------------------------------

  void _open(_Page page) {
    if (page == _page) return;
    setState(() {
      _page = page;
      _pageTurns++;
      _selected.clear();
      _anchor = _cursor = null;
      _renaming = null;
      if (_scroll.hasClients) _scroll.jumpTo(0);
    });
  }

  void _pick(LibraryEntry e, List<LibraryEntry> items,
      {bool extend = false, bool toggle = false}) {
    setState(() {
      _inSidebar = false;
      if (extend && _anchor != null) {
        final from = items.indexWhere((x) => x.key == _anchor);
        final to = items.indexWhere((x) => x.key == e.key);
        if (from >= 0 && to >= 0) {
          _selected
            ..clear()
            ..addAll([
              for (var i = math.min(from, to); i <= math.max(from, to); i++)
                items[i].key
            ]);
        }
      } else if (toggle) {
        _selected.contains(e.key)
            ? _selected.remove(e.key)
            : _selected.add(e.key);
        _anchor = e.key;
      } else {
        _selected
          ..clear()
          ..add(e.key);
        _anchor = e.key;
      }
      _cursor = e.key;
    });
    _focus.requestFocus();
  }

  /// Scroll the list so [e]'s row is in view.
  void _reveal(LibraryEntry e, List<_Row> rows) {
    if (!_scroll.hasClients) return;
    var top = 0.0;
    for (final r in rows) {
      if (r.entry?.key == e.key) break;
      top += r.height;
    }
    final position = _scroll.position;
    final bottom = top + _itemRow;
    double? to;
    if (top < position.pixels) {
      to = top;
    } else if (bottom > position.pixels + position.viewportDimension) {
      to = bottom - position.viewportDimension;
    }
    if (to == null) return;
    final spec = ThemeScope.of(context).motion.scroll;
    final at = to.clamp(0.0, position.maxScrollExtent);
    spec.isStill
        ? _scroll.jumpTo(at)
        : _scroll.animateTo(at, duration: spec.duration, curve: spec.curve);
  }

  // --- Keys -----------------------------------------------------------------

  List<_Page> get _pages => [
        const _Page.all(),
        for (final k in LibraryKind.values)
          if (libraryKindListed(
              k, _library.entries.where((e) => e.kind == k).length))
            _Page.kind(k),
        if (_library.conflicts.isNotEmpty) const _Page.needsYou(),
        const _Page.deleted(),
      ];

  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    final key = event.logicalKey;
    final mod = HardwareKeyboard.instance.isControlPressed ||
        HardwareKeyboard.instance.isMetaPressed;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    // Chords first, which work from anywhere in the window.
    if (mod) {
      if (key == LogicalKeyboardKey.keyF) {
        _searchFocus.requestFocus();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyI || key == LogicalKeyboardKey.keyO) {
        unawaited(_addFiles());
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyZ && !_typing) {
        unawaited(shift ? _library.redo() : _library.undo());
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyY && !_typing) {
        unawaited(_library.redo());
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyA && !_typing && !_page.needsYou) {
        setState(() => _selected
          ..clear()
          ..addAll([for (final e in _listed()) e.key]));
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    }
    // A field keeps its own keys.
    if (_typing) return KeyEventResult.ignored;
    if (key == LogicalKeyboardKey.arrowLeft) {
      setState(() => _inSidebar = true);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.arrowRight) {
      setState(() => _inSidebar = false);
      return KeyEventResult.handled;
    }
    if (_inSidebar) return _sidebarKey(key);
    if (_page.needsYou) return KeyEventResult.ignored;
    final items = _listed();
    if (key == LogicalKeyboardKey.f2) {
      final e = _entry(_cursor);
      if (e != null && e.here && !_page.deleted) _startRename(e);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.delete || key == LogicalKeyboardKey.backspace) {
      final chosen = _chosen;
      if (chosen.isNotEmpty && !_page.deleted) unawaited(_delete(chosen));
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.enter || key == LogicalKeyboardKey.numpadEnter) {
      final e = _entry(_cursor);
      if (e != null) unawaited(_main(e));
      return KeyEventResult.handled;
    }
    if (items.isEmpty) return KeyEventResult.ignored;
    final at = items.indexWhere((e) => e.key == _cursor);
    final pageBy = _scroll.hasClients
        ? math.max(1, (_scroll.position.viewportDimension / _itemRow).floor() - 1)
        : 8;
    final to = switch (key) {
      LogicalKeyboardKey.arrowUp => math.max(0, at - 1),
      LogicalKeyboardKey.arrowDown => math.min(items.length - 1, at + 1),
      LogicalKeyboardKey.pageUp => math.max(0, at - pageBy),
      LogicalKeyboardKey.pageDown => math.min(items.length - 1, at + pageBy),
      LogicalKeyboardKey.home => 0,
      LogicalKeyboardKey.end => items.length - 1,
      _ => null,
    };
    if (to == null) return KeyEventResult.ignored;
    _pick(items[to], items, extend: shift);
    _reveal(items[to], _rows(items));
    return KeyEventResult.handled;
  }

  /// Whether a text field has the keyboard.
  bool get _typing => _searchFocus.hasFocus || _renameFocus.hasFocus;

  KeyEventResult _sidebarKey(LogicalKeyboardKey key) {
    final pages = _pages;
    final at = pages.indexOf(_page);
    final to = switch (key) {
      LogicalKeyboardKey.arrowUp => math.max(0, at - 1),
      LogicalKeyboardKey.arrowDown => math.min(pages.length - 1, at + 1),
      LogicalKeyboardKey.home => 0,
      LogicalKeyboardKey.end => pages.length - 1,
      _ => null,
    };
    if (to == null) return KeyEventResult.ignored;
    _open(pages[to]);
    return KeyEventResult.handled;
  }

  // --- Doing ------------------------------------------------------------------

  /// A notice on the status line, with Undo beside it when the library can
  /// take the thing back.
  void _say(String message, {bool undo = true}) {
    widget.app?.postNotice(message,
        action: undo ? l10n.libraryUndo : null,
        onTap: undo ? () => unawaited(_library.undo()) : null);
  }

  /// What a row does on Enter or a double click: fetch what is not here,
  /// go to the conflict, or show the file.
  Future<void> _main(LibraryEntry e) async {
    if (_page.deleted) return _undelete(e);
    switch (e.state) {
      case LibraryItemState.notHere:
        await _library.get(e);
      case LibraryItemState.conflict:
        _open(const _Page.needsYou());
      default:
        if (e.pathHere case final at?) revealInFolder(path: at);
    }
  }

  Future<void> _delete(List<LibraryEntry> items) async {
    await _library.delete(items);
    setState(() {
      _selected.clear();
      _cursor = null;
    });
    _say(items.length == 1
        ? l10n.libraryDeleted(items.single.name)
        : l10n.libraryDeletedMany(items.length));
  }

  void _startRename(LibraryEntry e) {
    setState(() {
      _renaming = e.key;
      _renameText
        ..text = e.name
        ..selection = TextSelection(baseOffset: 0, extentOffset: e.name.length);
    });
  }

  Future<void> _finishRename(LibraryEntry e, String name) async {
    setState(() => _renaming = null);
    final to = name.trim();
    if (to.isEmpty || to == e.name) return;
    await _library.rename(e, to);
    _say(l10n.libraryRenamed(to));
    _focus.requestFocus();
  }

  Future<void> _keepOnlyHere(LibraryEntry e) async {
    if (!e.key.startsWith('local:')) {
      final sure = await askBefore(context, l10n.libraryKeepOnlyHereAsk(e.name),
          l10n.libraryKeepOnlyHereYes);
      if (!sure) return;
    }
    await _library.keepOnlyHere(e);
    _say(l10n.libraryKeptOnlyHere(e.name));
  }

  Future<void> _removeHere(LibraryEntry e) async {
    await _library.removeHere(e);
    _say(l10n.libraryRemovedHere(e.name));
  }

  Future<void> _restore(LibraryEntry e, LibraryVersion v) async {
    await _library.restore(e, v);
    _say(l10n.libraryRestored(e.name));
  }

  Future<void> _undelete(LibraryEntry e) async {
    await _library.undelete(e);
    _say(l10n.libraryRestored(e.name), undo: false);
  }

  Future<void> _deleteForever(LibraryEntry e) async {
    final sure = await askBefore(
        context, l10n.libraryDeleteForeverAsk(e.name), l10n.libraryDeleteForever);
    if (sure) await _library.deleteForever(e);
  }

  Future<void> _resolve(LibraryConflict c, LibraryResolution r) async {
    _settled = c.entry;
    await _library.resolve(c, r);
    _say(l10n.libraryResolved(c.entry.name));
  }

  /// The item settled here last, for where the window goes once Needs you
  /// has nothing left.
  LibraryEntry? _settled;

  /// Once the last card has left, back to the kind of what was settled with
  /// it chosen, or to everything, rather than an empty page the sidebar no
  /// longer lists.
  void _afterLeaving() {
    if (!mounted ||
        !_page.needsYou ||
        _leaving.isNotEmpty ||
        _library.conflicts.isNotEmpty) {
      return;
    }
    final settled = _settled;
    _open(settled == null ? const _Page.all() : _Page.kind(settled.kind));
    if (_entry(settled?.key) case final kept?) {
      setState(() {
        _selected
          ..clear()
          ..add(kept.key);
        _anchor = _cursor = kept.key;
      });
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _reveal(kept, _rows(_listed()));
      });
    }
  }

  Future<void> _addFiles() async {
    final paths = await pickLibraryFiles();
    if (paths.isEmpty) return;
    await _add(paths);
  }

  Future<void> _add(List<String> paths) async {
    final added = await _library.add(paths);
    if (added.isEmpty) return;
    _say(l10n.libraryAdded(added.length), undo: false);
    if (added.length == 1 && mounted) {
      final e = added.single;
      _open(_Page.kind(e.kind));
      setState(() {
        _selected
          ..clear()
          ..add(e.key);
        _cursor = _anchor = e.key;
      });
    }
  }

  Future<void> _import() async {
    final file = await pickLibraryToOpen();
    if (file == null) return;
    final n = await _library.importFrom(file);
    _say(l10n.libraryAdded(n), undo: false);
  }

  Future<void> _export() async {
    final file = await pickLibrarySaveLocation();
    if (file == null) return;
    final n = await _library.exportTo(file, _chosen);
    _say(l10n.libraryExported(n), undo: false);
  }

  /// Everything that can be done to [e], in the order the detail pane and the
  /// menu show it.
  List<_Act> _acts(LibraryEntry e) {
    final sync = _library.syncing;
    final inAccount = !e.key.startsWith('local:');
    // Two versions wait for a choice: that comes first, and nothing that
    // would move or rename the item meanwhile is offered.
    if (e.state == LibraryItemState.conflict) {
      return [
        _Act('resolve', l10n.libraryNeedsYou,
            () async => _open(const _Page.needsYou())),
        if (e.pathHere case final at?)
          _Act('reveal', l10n.projectShowInFolder,
              () async => revealInFolder(path: at)),
        _Act('delete', l10n.delete, () => _delete([e])),
      ];
    }
    return [
      if (e.pathHere case final at?)
        _Act('reveal', l10n.projectShowInFolder,
            () async => revealInFolder(path: at)),
      if (sync && e.state == LibraryItemState.notHere && !e.cannotWrite)
        _Act('get', l10n.libraryGet, () => _library.get(e)),
      if (sync && e.here && inAccount && e.state != LibraryItemState.onlyHere)
        _Act('remove-here', l10n.libraryRemoveHere, () => _removeHere(e)),
      if (sync && e.here && e.state != LibraryItemState.onlyHere)
        _Act('keep-here', l10n.libraryKeepOnlyHere, () => _keepOnlyHere(e)),
      if (sync && e.state == LibraryItemState.onlyHere)
        _Act('share-again', l10n.libraryShareAgain,
            () => _library.shareAgain(e)),
      if (e.here) _Act('rename', l10n.rename, () async => _startRename(e)),
      _Act('delete', l10n.delete, () => _delete([e])),
    ];
  }

  void _menu(LibraryEntry e, Offset at) {
    final many = _selected.length > 1 && _selected.contains(e.key);
    unawaited(showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) => [
        ..._undoRows(close),
        if (_page.deleted) ...[
          _menuRow('undelete', l10n.libraryRestore, close, () => _undelete(e)),
          const MenuDivider(),
          _menuRow('forever', l10n.libraryDeleteForever, close,
              () => _deleteForever(e)),
        ] else if (many) ...[
          _menuRow('export', l10n.libraryExport, close, _export),
          const MenuDivider(),
          _menuRow('delete', l10n.delete, close, () => _delete(_chosen)),
        ] else
          for (final (i, act) in _acts(e).indexed) ...[
            if (act.id == 'delete' && i > 0) const MenuDivider(),
            _menuRow(act.id, act.label, close, act.run),
          ],
      ],
    ));
  }

  /// Undo and Redo, named after what they would do, over a row's own rows.
  List<Widget> _undoRows(void Function(void) close) {
    final undo = _library.undoing, redo = _library.redoing;
    if (undo == null && redo == null) return const [];
    return [
      if (undo != null)
        _menuRow('undo', libraryUndoWords(undo, redo: false), close,
            () => _library.undo()),
      if (redo != null)
        _menuRow('redo', libraryUndoWords(redo, redo: true), close,
            () => _library.redo()),
      const MenuDivider(),
    ];
  }

  Widget _menuRow(String id, String label, void Function(void) close,
          Future<void> Function() then) =>
      MenuRow(
        key: ValueKey<String>('library-menu-$id'),
        onPressed: () {
          close(null);
          unawaited(then());
        },
        child: MenuLabel(label),
      );

  // --- Drawing ----------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final items = _page.needsYou ? const <LibraryEntry>[] : _listed();
    final chosen = _chosen;
    final detail = !_page.needsYou && chosen.isNotEmpty;
    return dialogSurface(
      context,
      t,
      Entrance.content(
        spec: scope.motion.content,
        rise: scope.motion.contentRise,
        blur: scope.motion.contentBlur,
        child: Focus(
          focusNode: _focus,
          autofocus: true,
          onKeyEvent: _key,
          child: DropTarget(
            onDragEntered: (_) => setState(() => _dragging = true),
            onDragExited: (_) => setState(() => _dragging = false),
            onDragDone: (details) async {
              setState(() => _dragging = false);
              final paths = await droppedPaths(details.files);
              if (mounted) await _add(paths);
            },
            child: Stack(
              children: [
                Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Builder(
                      builder: (context) => GestureDetector(
                        onSecondaryTapUp: (d) => showWindowMenu(
                            context, d.globalPosition,
                            onClose: widget.close),
                        child: _titleStrip(t),
                      ),
                    ),
                    Expanded(
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          SizedBox(width: _sidebarWidth, child: _sidebar(t)),
                          Container(width: 1, color: t.hairline),
                          Expanded(
                            child: Entrance(
                              key: ValueKey<Object>((_page, _pageTurns)),
                              spec: _pageTurns == 0
                                  ? MotionSpec.still
                                  : scope.motion.reveal,
                              rise: scope.motion.revealRise,
                              child: _middle(t, items),
                            ),
                          ),
                          if (detail) ...[
                            Container(width: 1, color: t.hairline),
                            SizedBox(
                              width: _detailWidth,
                              child: chosen.length == 1
                                  ? _Detail(
                                      key: ValueKey<String>(chosen.single.key),
                                      entry: chosen.single,
                                      library: _library,
                                      deleted: _page.deleted,
                                      acts: _page.deleted
                                          ? [
                                              _Act('undelete', l10n.libraryRestore,
                                                  () => _undelete(chosen.single)),
                                              _Act(
                                                  'forever',
                                                  l10n.libraryDeleteForever,
                                                  () => _deleteForever(
                                                      chosen.single)),
                                            ]
                                          : _acts(chosen.single),
                                      onRestore: (v) =>
                                          _restore(chosen.single, v),
                                      onSaveCopy: (v) =>
                                          _library.saveCopy(chosen.single, v),
                                    )
                                  : _many(t, chosen),
                            ),
                          ],
                        ],
                      ),
                    ),
                    _footer(t),
                  ],
                ),
                // Files held over the window light its edge.
                Positioned.fill(
                  child: IgnorePointer(
                    child: AnimatedContainer(
                      duration: scope.motion.dropZone.duration,
                      curve: scope.motion.dropZone.curve,
                      decoration: BoxDecoration(
                        borderRadius:
                            BorderRadius.circular(t.tokens.floatRadius),
                        border: Border.all(
                            color: t.accent.withValues(alpha: _dragging ? 1 : 0),
                            width: 1.5),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _titleStrip(LumitTheme t) => Container(
        key: const ValueKey('library-title-strip'),
        height: _strip + 1,
        decoration: BoxDecoration(
          color: t.surface2,
          border: Border(bottom: BorderSide(color: t.hairline)),
        ),
        padding: const EdgeInsets.symmetric(horizontal: _pad),
        child: Row(
          children: [
            Text(t.kickerCase(l10n.libraryTitle), style: t.kickerOn),
            const Spacer(),
            _status(t),
            const SizedBox(width: 12),
            SizedBox(
              width: 174,
              height: 20,
              child: HouseTextField(
                key: const ValueKey('library-search'),
                controller: _search,
                focusNode: _searchFocus,
                width: double.infinity,
                padding: const EdgeInsets.symmetric(horizontal: 6),
                fill: t.surface2,
                hint: l10n.librarySearch,
                leading: glyph.LumitIcon(LumitIcons.search,
                    size: 12, colour: t.textMuted),
              ),
            ),
            const SizedBox(width: 12),
            LumitTooltip(
              message: l10n.close,
              child: HoverMark(
                swells: true,
                builder: (context, lit) => GestureDetector(
                  key: const ValueKey('library-close-mark'),
                  behavior: HitTestBehavior.opaque,
                  onTap: widget.close,
                  child: SizedBox(
                    width: dialogCloseGlyph + 8,
                    height: _strip,
                    child: Center(
                      child: glyph.LumitIcon(LumitIcons.close,
                          size: dialogCloseGlyph,
                          colour: Color.lerp(t.textMuted, t.textPrimary, lit),
                          semanticLabel: l10n.close),
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      );

  /// One calm line: what the library is doing. A conflict count is pressable
  /// and goes to Needs you.
  Widget _status(LumitTheme t) {
    final (words, tone) = libraryStatusWords(_library);
    final colour = libraryToneColour(t, tone);
    final Widget? mark = switch (tone) {
      LibraryTone.good =>
        glyph.LumitIcon(LumitIcons.tick, size: 12, colour: t.success),
      LibraryTone.busy => const SizedBox(width: 11, height: 11, child: SyncRing()),
      LibraryTone.warn => Container(
          width: 6,
          height: 6,
          decoration: BoxDecoration(color: t.warning, shape: BoxShape.circle)),
      LibraryTone.quiet => null,
    };
    final line = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (mark != null) ...[mark, const SizedBox(width: 6)],
        Text(words,
            key: const ValueKey('library-status'),
            style: t.small.copyWith(color: colour)),
      ],
    );
    // A press goes to the conflicts when there are any, and otherwise syncs
    // now, which is also the way to try again offline or after a failure.
    final conflicts = _library.conflicts.isNotEmpty;
    if (!_library.syncing ||
        (_library.status == LibraryStatus.syncing && !conflicts)) {
      return Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4), child: line);
    }
    final pressable = HoverFill(
      cursor: SystemMouseCursors.click,
      radius: BorderRadius.circular(t.tokens.controlRadius),
      child: GestureDetector(
        key: const ValueKey('library-status-press'),
        behavior: HitTestBehavior.opaque,
        onTap: conflicts
            ? () => _open(const _Page.needsYou())
            : () => unawaited(_library.syncNow()),
        child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 3),
            child: line),
      ),
    );
    return conflicts
        ? pressable
        : LumitTooltip(message: l10n.settingsSyncNow, child: pressable);
  }

  // --- The sidebar ------------------------------------------------------------

  Widget _sidebar(LumitTheme t) {
    final counts = <LibraryKind, int>{};
    for (final e in _library.entries) {
      counts[e.kind] = (counts[e.kind] ?? 0) + 1;
    }
    final conflicts = _library.conflicts.length;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: SingleChildScrollView(
            padding: const EdgeInsets.only(top: 8, bottom: 8),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _navRow(t, const _Page.all(), l10n.libraryAll,
                    _library.entries.length),
                for (final k in LibraryKind.values)
                  if (libraryKindListed(k, counts[k] ?? 0))
                    _navRow(t, _Page.kind(k), libraryKindLabel(k),
                        counts[k] ?? 0),
                const SizedBox(height: 8),
                Container(height: 1, color: t.hairline),
                const SizedBox(height: 8),
                if (conflicts > 0)
                  _navRow(t, const _Page.needsYou(), l10n.libraryNeedsYou,
                      conflicts,
                      warn: true),
                _navRow(t, const _Page.deleted(), l10n.libraryRecentlyDeleted,
                    _library.recentlyDeleted.length),
              ],
            ),
          ),
        ),
        if (_library.syncing && _library.machines.isNotEmpty) _computers(t),
      ],
    );
  }

  Widget _navRow(LumitTheme t, _Page page, String label, int count,
      {bool warn = false}) {
    final on = page == _page;
    final scope = ThemeScope.of(context);
    return HoverFill(
      child: GestureDetector(
        key: ValueKey<String>(page.kind == null
            ? 'library-page-${_pageId(page)}'
            : 'library-kind-${page.kind!.name}'),
        behavior: HitTestBehavior.opaque,
        onTap: () => _open(page),
        child: AnimatedContainer(
          duration: scope.motion.mark.duration,
          curve: scope.motion.mark.curve,
          height: _navHeight,
          padding: EdgeInsetsDirectional.only(
              start: on ? _pad - _navTick : _pad, end: _pad),
          decoration: BoxDecoration(
            color: on ? t.surface2 : null,
            border: on
                ? Border(
                    left: BorderSide(color: t.accent, width: _navTick))
                : null,
          ),
          child: Row(
            children: [
              Expanded(
                child: Text(label,
                    style: on ? t.bodyPrimary : t.body,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
              ),
              Text('$count',
                  style: t.mono.copyWith(
                      fontSize: 10,
                      color: warn
                          ? t.warning
                          : on
                              ? t.textSecondary
                              : t.textMuted)),
            ],
          ),
        ),
      ),
    );
  }

  String _pageId(_Page page) => page.all
      ? 'all'
      : page.needsYou
          ? 'needs-you'
          : page.deleted
              ? 'deleted'
              : page.kind!.name;

  /// The account's computers under the kinds: a disc, the name, and when it
  /// was last seen.
  Widget _computers(LumitTheme t) => Padding(
        padding: const EdgeInsets.fromLTRB(_pad, 8, _pad, 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(t.kickerCase(l10n.libraryComputers), style: t.kicker),
            const SizedBox(height: 6),
            for (final m in _library.machines)
              SizedBox(
                height: 22,
                child: Row(
                  children: [
                    LibraryDisc(
                      hold: LibraryHold.newest,
                      initial: m.name.trim().isEmpty
                          ? ''
                          : String.fromCharCode(m.name.trim().runes.first)
                              .toUpperCase(),
                      size: 12,
                      theme: t,
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(libraryMachineName(m),
                          style: t.body, maxLines: 1, overflow: TextOverflow.ellipsis),
                    ),
                    if (m.online && !m.me)
                      Container(
                        width: 5,
                        height: 5,
                        margin: const EdgeInsetsDirectional.only(end: 6),
                        decoration: BoxDecoration(
                            color: t.success, shape: BoxShape.circle),
                      ),
                    Text(librarySeen(m),
                        style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
                  ],
                ),
              ),
          ],
        ),
      );

  // --- The middle ------------------------------------------------------------

  Widget _middle(LumitTheme t, List<LibraryEntry> items) {
    if (_page.needsYou) return _needsYou(t);
    final rows = _rows(items);
    // Names two listed items share, whose rows then say their files.
    final seen = <String>{};
    final twins = <String>{
      for (final e in items)
        if (!seen.add(e.name.toLowerCase())) e.name.toLowerCase()
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _listHeader(t, items),
        Expanded(
          child: items.isEmpty
              ? _empty(t)
              : dialogScroller(
                  context,
                  t,
                  _scroll,
                  child: ListView.builder(
                    key: ValueKey<String>('library-list-${_pageId(_page)}'),
                    controller: _scroll,
                    padding: const EdgeInsets.only(bottom: 8),
                    itemCount: rows.length,
                    itemBuilder: (context, i) {
                      final row = rows[i];
                      if (row.header case final head?) return _groupHeader(t, head);
                      final e = row.entry!;
                      return _page.deleted
                          ? _DeletedRow(
                              key: ValueKey<String>('library-row-${e.key}'),
                              entry: e,
                              selected: _selected.contains(e.key),
                              onTap: () => _pick(e, items),
                              onMenu: (at) => _menu(e, at),
                              onRestore: () => _undelete(e),
                              onForever: () => _deleteForever(e),
                            )
                          : _ItemRow(
                              key: ValueKey<String>('library-row-${e.key}'),
                              entry: e,
                              showFile: twins.contains(e.name.toLowerCase()),
                              machines: _library.syncing
                                  ? _library.machines
                                  : const [],
                              selected: _selected.contains(e.key),
                              renaming: _renaming == e.key,
                              renameText: _renameText,
                              renameFocus: _renameFocus,
                              onRenamed: (name) => _finishRename(e, name),
                              onRenameCancelled: () =>
                                  setState(() => _renaming = null),
                              onTap: (extend, toggle) =>
                                  _pick(e, items, extend: extend, toggle: toggle),
                              onOpen: () => _main(e),
                              onMenu: (at) {
                                if (!_selected.contains(e.key)) _pick(e, items);
                                _menu(e, at);
                              },
                              onGet: _library.syncing
                                  ? () => unawaited(_library.get(e))
                                  : null,
                            );
                    },
                  ),
                ),
        ),
      ],
    );
  }

  /// The list's own header: what is listed, how many and how much, and for
  /// a kind the switch that keeps it on this computer.
  Widget _listHeader(LumitTheme t, List<LibraryEntry> items) {
    final kind = _page.kind;
    final title = _page.all
        ? l10n.libraryAll
        : _page.deleted
            ? l10n.libraryRecentlyDeleted
            : libraryKindLabel(kind!);
    var bytes = 0;
    for (final e in items) {
      bytes += e.size;
    }
    return SizedBox(
      height: 32,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: _pad),
        child: Row(
          children: [
            Text(title, style: t.bodyPrimary),
            const SizedBox(width: 8),
            Text(
              [
                '${items.length}',
                if (bytes > 0 && !_page.deleted) writtenBytes(bytes),
              ].join(' · '),
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
            ),
            const Spacer(),
            if (kind != null && _library.syncing) ...[
              Text(l10n.libraryOnThisComputer,
                  style: t.small.copyWith(color: t.textSecondary)),
              const SizedBox(width: 8),
              HouseToggle(
                key: const ValueKey('library-kind-on'),
                value: _library.kindOn(kind),
                onChanged: (on) => _library.setKindOn(kind, on),
              ),
            ],
          ],
        ),
      ),
    );
  }

  Widget _groupHeader(LumitTheme t, String name) => Container(
        height: _headerRow,
        padding: const EdgeInsets.symmetric(horizontal: _pad),
        alignment: AlignmentDirectional.centerStart,
        child: Text(t.kickerCase(name),
            style: t.kicker, maxLines: 1, overflow: TextOverflow.ellipsis),
      );

  Widget _empty(LumitTheme t) => Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(l10n.libraryEmpty,
                key: const ValueKey('library-empty'),
                style: t.body.copyWith(color: t.textSecondary)),
            if (!_page.deleted) ...[
              const SizedBox(height: 10),
              HouseButton(
                key: const ValueKey('library-empty-add'),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                onPressed: () => unawaited(_addFiles()),
                child: Text(l10n.libraryAdd),
              ),
            ],
          ],
        ),
      );

  /// The conflicts, each as two cards side by side and the three ways out.
  /// One that was settled leaves calmly rather than vanishing.
  Widget _needsYou(LumitTheme t) {
    final scope = ThemeScope.of(context);
    final live = _library.conflicts;
    final shown = <Widget>[
      for (final (i, c) in live.indexed)
        _ConflictBlock(
          key: ValueKey<String>('library-conflict-${c.entry.key}'),
          conflict: c,
          machines: _library.machines,
          first: i == 0,
          onResolve: (r) => _resolve(c, r),
        ),
      for (final MapEntry(:key, value: c) in _leaving.entries)
        _Leave(
          key: ValueKey<String>('library-conflict-leaving-$key'),
          spec: scope.motion.reveal,
          onDone: () {
            setState(() => _leaving.remove(key));
            _afterLeaving();
          },
          child: _ConflictBlock(
              conflict: c, machines: _library.machines, onResolve: null),
        ),
    ];
    if (shown.isEmpty) {
      return Center(
          child: Text(l10n.libraryEmpty,
              style: t.body.copyWith(color: t.textSecondary)));
    }
    return dialogScroller(
      context,
      t,
      _scroll,
      child: ListView(
        key: const ValueKey('library-needs-you'),
        controller: _scroll,
        padding: const EdgeInsets.fromLTRB(_pad, 10, _pad, 10),
        children: shown,
      ),
    );
  }

  /// The right pane for more than one chosen item.
  Widget _many(LumitTheme t, List<LibraryEntry> chosen) {
    var bytes = 0;
    for (final e in chosen) {
      bytes += e.size;
    }
    return Padding(
      padding: const EdgeInsets.all(_pad),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(l10n.libraryChosen(chosen.length),
              style: t.bodyStrong.copyWith(color: t.textPrimary, fontSize: 13)),
          const SizedBox(height: 2),
          Text(writtenBytes(bytes),
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
          const SizedBox(height: 14),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              HouseButton(
                key: const ValueKey('library-many-export'),
                small: true,
                onPressed: () => unawaited(_export()),
                child: Text(l10n.libraryExport, style: t.small),
              ),
              if (!_page.deleted)
                HouseButton(
                  key: const ValueKey('library-many-delete'),
                  small: true,
                  onPressed: () => unawaited(_delete(chosen)),
                  child: Text(l10n.delete, style: t.small),
                ),
            ],
          ),
        ],
      ),
    );
  }

  // --- The footer -------------------------------------------------------------

  Widget _footer(LumitTheme t) {
    final usage = _library.usage;
    final pro = _library.account.pro;
    final Widget? leading = _library.syncing && usage != null
        ? LibraryStorage(usage)
        : !pro
            ? HouseButton(
                key: const ValueKey('library-pro'),
                frameless: true,
                small: true,
                padding: const EdgeInsets.symmetric(horizontal: 4),
                onPressed: () => showProWindow(context),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(l10n.librarySyncWithPro,
                        style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
                    const SizedBox(width: 6),
                    const ProBadge(),
                  ],
                ),
              )
            : null;
    Widget action(String id, String label, VoidCallback? onPressed) =>
        HouseButton(
          key: ValueKey<String>('library-$id'),
          padding: const EdgeInsets.symmetric(horizontal: 12),
          onPressed: onPressed,
          child: Text(label),
        );
    return dialogFooter(
      t,
      keyPrefix: 'library',
      leading: leading,
      actions: [
        action('import', l10n.libraryImport, () => unawaited(_import())),
        action('export', l10n.libraryExport,
            _library.entries.isEmpty ? null : () => unawaited(_export())),
        action('add', l10n.libraryAdd, () => unawaited(_addFiles())),
        action('close', l10n.close, widget.close),
      ],
    );
  }
}

// --- Rows ---------------------------------------------------------------------

/// One item: its tile, name and detail, size, a disc per computer, and the
/// mark it wears when it is not simply in sync.
class _ItemRow extends StatelessWidget {
  final LibraryEntry entry;

  /// Whether the second line leads with the file's name, as another listed
  /// item has the same name.
  final bool showFile;
  final List<LibraryMachine> machines;
  final bool selected, renaming;
  final TextEditingController renameText;
  final FocusNode renameFocus;
  final ValueChanged<String> onRenamed;
  final VoidCallback onRenameCancelled;
  final void Function(bool extend, bool toggle) onTap;
  final VoidCallback onOpen;
  final void Function(Offset at) onMenu;
  final VoidCallback? onGet;

  const _ItemRow({
    super.key,
    required this.entry,
    this.showFile = false,
    required this.machines,
    required this.selected,
    required this.renaming,
    required this.renameText,
    required this.renameFocus,
    required this.onRenamed,
    required this.onRenameCancelled,
    required this.onTap,
    required this.onOpen,
    required this.onMenu,
    required this.onGet,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final e = entry;
    final mark = LibraryMark(e, onGet: onGet, getKey: 'library-get-${e.key}');
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTapDown: (_) => onTap(HardwareKeyboard.instance.isShiftPressed,
          HardwareKeyboard.instance.isControlPressed ||
              HardwareKeyboard.instance.isMetaPressed),
      onDoubleTap: onOpen,
      onSecondaryTapDown: (d) => onMenu(d.globalPosition),
      child: HoverFill(
        faint: true,
        child: AnimatedContainer(
          duration: scope.motion.mark.duration,
          curve: scope.motion.mark.curve,
          height: _itemRow,
          color: selected ? t.surface2 : t.surface2.withValues(alpha: 0),
          padding: const EdgeInsets.symmetric(horizontal: _pad),
          child: Row(
            children: [
              LibraryTile(e),
              const SizedBox(width: 10),
              Expanded(
                child: renaming
                    ? SizedBox(
                        height: 20,
                        child: HouseTextField(
                          key: ValueKey<String>('library-rename-${e.key}'),
                          controller: renameText,
                          focusNode: renameFocus,
                          width: double.infinity,
                          autofocus: true,
                          onSubmitted: onRenamed,
                          onTapOutside: () => onRenamed(renameText.text),
                          onCancelled: onRenameCancelled,
                        ),
                      )
                    : Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(e.name,
                              style: t.bodyPrimary,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis),
                          if (e.detail.isNotEmpty || showFile)
                            Text(
                                [
                                  if (showFile) libraryFileName(e),
                                  if (e.detail.isNotEmpty) libraryDetail(e),
                                ].join(' · '),
                                style: t.small.copyWith(color: t.textMuted),
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis),
                        ],
                      ),
              ),
              const SizedBox(width: 10),
              SizedBox(
                width: 56,
                child: Text(
                  e.size > 0 ? writtenBytes(e.size) : '',
                  textAlign: TextAlign.end,
                  style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
                ),
              ),
              if (machines.isNotEmpty) ...[
                const SizedBox(width: 12),
                LibraryDiscs(e, machines),
              ],
              const SizedBox(width: 10),
              SizedBox(
                width: 44,
                child: Align(
                  alignment: AlignmentDirectional.centerEnd,
                  // A change of state fades from one mark to the next.
                  child: AnimatedSwitcher(
                    duration: scope.motion.mark.duration,
                    switchInCurve: scope.motion.mark.curve,
                    switchOutCurve: scope.motion.mark.curve,
                    child: KeyedSubtree(
                        key: ValueKey<Object>((e.state, e.cannotWrite)),
                        child: mark),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// A recently deleted item: when and where it went, and the two ways on.
class _DeletedRow extends StatelessWidget {
  final LibraryEntry entry;
  final bool selected;
  final VoidCallback onTap, onRestore, onForever;
  final void Function(Offset at) onMenu;

  const _DeletedRow({
    super.key,
    required this.entry,
    required this.selected,
    required this.onTap,
    required this.onMenu,
    required this.onRestore,
    required this.onForever,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final e = entry;
    final where = e.head?.deviceName ?? '';
    final when = e.gone == null ? '' : backupWhen(e.gone!);
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTapDown: (_) => onTap(),
      onSecondaryTapDown: (d) => onMenu(d.globalPosition),
      child: HoverFill(
        faint: true,
        child: AnimatedContainer(
          duration: scope.motion.mark.duration,
          curve: scope.motion.mark.curve,
          height: _itemRow,
          color: selected ? t.surface2 : t.surface2.withValues(alpha: 0),
          padding: const EdgeInsets.symmetric(horizontal: _pad),
          child: Row(
            children: [
              LibraryTile(e),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(e.name,
                        style: t.bodyPrimary,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis),
                    Text(
                      [
                        libraryKindLabel(e.kind),
                        if (e.group case final g?) g,
                      ].join(' · '),
                      style: t.small.copyWith(color: t.textMuted),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 10),
              Text(
                [if (where.isNotEmpty) where, if (when.isNotEmpty) when]
                    .join(' · '),
                style: t.small.copyWith(color: t.textMuted),
              ),
              const SizedBox(width: 12),
              HouseButton(
                key: ValueKey<String>('library-undelete-${e.key}'),
                small: true,
                padding: const EdgeInsets.symmetric(horizontal: 8),
                onPressed: onRestore,
                child: Text(l10n.libraryRestore, style: t.small),
              ),
              const SizedBox(width: 6),
              HouseButton(
                key: ValueKey<String>('library-forever-${e.key}'),
                small: true,
                frameless: true,
                padding: const EdgeInsets.symmetric(horizontal: 6),
                onPressed: onForever,
                child: Text(l10n.libraryDeleteForever, style: t.small),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// --- The detail pane ----------------------------------------------------------

/// The chosen item: its preview, what it is, where it is, its history and
/// its actions. The history is asked for as the pane opens and kept while
/// the same item is chosen.
class _Detail extends StatefulWidget {
  final LibraryEntry entry;
  final LibraryState library;
  final bool deleted;
  final List<_Act> acts;
  final Future<void> Function(LibraryVersion v) onRestore, onSaveCopy;

  const _Detail({
    super.key,
    required this.entry,
    required this.library,
    required this.deleted,
    required this.acts,
    required this.onRestore,
    required this.onSaveCopy,
  });

  @override
  State<_Detail> createState() => _DetailState();
}

class _DetailState extends State<_Detail> {
  List<LibraryVersion>? _history;
  final _scroll = ScrollController();

  @override
  void initState() {
    super.initState();
    unawaited(_read());
  }

  @override
  void didUpdateWidget(_Detail old) {
    super.didUpdateWidget(old);
    if (old.entry.head?.n != widget.entry.head?.n) unawaited(_read());
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _read() async {
    final e = widget.entry;
    if (!widget.library.syncing || e.key.startsWith('local:')) {
      setState(() => _history = const []);
      return;
    }
    try {
      final versions = await widget.library.history(e);
      if (mounted) setState(() => _history = versions);
    } catch (_) {
      // Offline, or the server would not say: the head is still known.
      if (mounted) setState(() => _history = [if (e.head case final h?) h]);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final e = widget.entry;
    final machines = widget.library.syncing ? widget.library.machines : const <LibraryMachine>[];
    final history = _history ?? [if (e.head case final h?) h];
    final rule = Container(height: 1, color: t.hairline);
    return dialogScroller(
      context,
      t,
      _scroll,
      child: ListView(
        controller: _scroll,
        padding: EdgeInsets.zero,
        children: [
          _preview(t, e),
          Padding(
            padding: const EdgeInsets.fromLTRB(_pad, 12, _pad, 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(e.name,
                    key: const ValueKey('library-detail-name'),
                    style: t.bodyStrong.copyWith(color: t.textPrimary, fontSize: 13),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis),
                const SizedBox(height: 3),
                Text(
                  [
                    libraryKindLabel(e.kind),
                    if (e.group case final g?) g,
                    if (libraryFileName(e) case final f
                        when f != e.name && _stemOf(f) != e.name)
                      f,
                    if (e.size > 0) writtenBytes(e.size),
                    if (e.detail.isNotEmpty) libraryDetail(e),
                  ].join(' · '),
                  style: t.small.copyWith(color: t.textMuted),
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                ),
                if (e.cannotWrite) ...[
                  const SizedBox(height: 6),
                  Text(l10n.libraryCannotWrite,
                      key: const ValueKey('library-detail-reason'),
                      style: t.small.copyWith(color: t.warning)),
                ],
              ],
            ),
          ),
          if (machines.isNotEmpty) ...[
            rule,
            _section(t, l10n.libraryWhere, [
              for (final m in machines)
                SizedBox(
                  height: 22,
                  child: Row(
                    children: [
                      LibraryDisc(
                          hold: libraryHoldOf(e, m),
                          initial: m.name.trim().isEmpty
                              ? ''
                              : String.fromCharCode(m.name.trim().runes.first)
                                  .toUpperCase(),
                          size: 12,
                          theme: t),
                      const SizedBox(width: 8),
                      Expanded(
                          child: Text(libraryMachineName(m),
                              style: t.body,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis)),
                      Text(libraryWhereWords(e, m),
                          style: t.small.copyWith(
                              color: m.me &&
                                      (e.cannotWrite ||
                                          e.state == LibraryItemState.conflict ||
                                          e.state == LibraryItemState.tooBig ||
                                          e.state == LibraryItemState.noRoom)
                                  ? t.warning
                                  : t.textMuted)),
                    ],
                  ),
                ),
            ]),
          ],
          if (history.isNotEmpty) ...[
            rule,
            _section(t, l10n.libraryHistory, [
              for (final (i, v) in history.indexed) _version(t, v, newest: i == 0),
            ]),
          ],
          rule,
          Padding(
            padding: const EdgeInsets.fromLTRB(_pad, 12, _pad, 12),
            child: Wrap(
              spacing: 6,
              runSpacing: 6,
              children: [
                for (final act in widget.acts)
                  HouseButton(
                    key: ValueKey<String>('library-${act.id}'),
                    small: true,
                    frameless: act.id == 'delete' || act.id == 'forever',
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    onPressed: () => unawaited(act.run()),
                    child: Text(act.label, style: t.small),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _section(LumitTheme t, String title, List<Widget> rows) => Padding(
        padding: const EdgeInsets.fromLTRB(_pad, 10, _pad, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(t.kickerCase(title), style: t.kicker),
            const SizedBox(height: 4),
            ...rows,
          ],
        ),
      );

  /// The preview over the pane: a font's own face at size, a theme's or
  /// style's colours, or the kind's glyph.
  Widget _preview(LumitTheme t, LibraryEntry e) {
    final Widget face;
    if (e.kind == LibraryKind.fonts) {
      face = Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          LibraryFontSample(
              path: e.pathHere,
              modified: e.modified,
              sample: 'Aa Bb Cc',
              style: t.heading.copyWith(fontSize: 30, height: 1.1)),
          const SizedBox(height: 6),
          LibraryFontSample(
              path: e.pathHere,
              modified: e.modified,
              sample: l10n.libraryFontSample,
              style: t.body.copyWith(color: t.textSecondary)),
        ],
      );
    } else if (e.colours.isNotEmpty) {
      // All eight, outlined once, as Settings ▸ Appearance shows a theme.
      face = Container(
        height: 28,
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.hairlineStrong),
        ),
        foregroundDecoration: BoxDecoration(
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.hairlineStrong),
        ),
        clipBehavior: Clip.antiAlias,
        child: LibrarySwatches(e.colours, count: 8),
      );
    } else if (e.kind == LibraryKind.templates && e.pathHere != null) {
      face = ClipRRect(
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        child: SizedBox(
          height: 72,
          child: AspectRatio(
            aspectRatio: 16 / 9,
            child: LibraryPoster(
              path: e.pathHere!,
              modified: e.modified,
              fallback: Center(
                child: glyph.LumitIcon(libraryGlyph(e),
                    size: 28, colour: t.textSecondary),
              ),
            ),
          ),
        ),
      );
    } else if (e.kind == LibraryKind.presets && e.detail.isNotEmpty) {
      // A preset shows its stack, top to bottom, as Effect controls lists it.
      final names = libraryDetail(e).split(', ');
      final shown = names.take(names.length > 4 ? 3 : 4).toList();
      face = Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final name in shown)
            Container(
              height: 18,
              margin: const EdgeInsets.only(bottom: 2),
              padding: const EdgeInsets.symmetric(horizontal: 6),
              alignment: AlignmentDirectional.centerStart,
              decoration: BoxDecoration(
                color: t.surface3,
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              ),
              child: Text(name,
                  style: t.small.copyWith(color: t.textPrimary),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis),
            ),
          if (names.length > shown.length)
            Text('+${names.length - shown.length}',
                style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
        ],
      );
    } else {
      face = Align(
        alignment: AlignmentDirectional.centerStart,
        child: glyph.LumitIcon(libraryGlyph(e),
            size: 28, colour: t.textSecondary),
      );
    }
    return Container(
      height: 96,
      padding: const EdgeInsets.symmetric(horizontal: _pad),
      alignment: AlignmentDirectional.centerStart,
      decoration: BoxDecoration(
        color: t.surface2,
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      child: face,
    );
  }

  /// One version: when, which computer and how big, and Restore unless it
  /// is the current one. Its right-click also keeps it as a copy.
  Widget _version(LumitTheme t, LibraryVersion v, {required bool newest}) {
    final row = Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(backupWhen(v.created),
                    style: t.bodyPrimary, maxLines: 1, overflow: TextOverflow.ellipsis),
                Text(
                  [if (v.deviceName.isNotEmpty) v.deviceName, writtenBytes(v.size)]
                      .join(' · '),
                  style: t.small.copyWith(color: t.textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          if (newest)
            Text(l10n.libraryCurrent, style: t.small.copyWith(color: t.textMuted))
          else
            HouseButton(
              key: ValueKey<String>('library-restore-${v.n}'),
              small: true,
              padding: const EdgeInsets.symmetric(horizontal: 8),
              onPressed: () => unawaited(widget.onRestore(v)),
              child: Text(l10n.libraryRestore, style: t.small),
            ),
        ],
      ),
    );
    return Builder(
      key: ValueKey<String>('library-version-${v.n}'),
      builder: (context) => GestureDetector(
        behavior: HitTestBehavior.translucent,
        onSecondaryTapUp: (d) => showMenuAt<void>(
          context: context,
          position: d.globalPosition,
          rows: (close) => [
            if (!newest)
              MenuRow(
                key: ValueKey<String>('library-version-restore-${v.n}'),
                onPressed: () {
                  close(null);
                  unawaited(widget.onRestore(v));
                },
                child: MenuLabel(l10n.libraryRestore),
              ),
            MenuRow(
              key: ValueKey<String>('library-version-copy-${v.n}'),
              onPressed: () {
                close(null);
                unawaited(widget.onSaveCopy(v));
              },
              child: MenuLabel(l10n.librarySaveCopy),
            ),
          ],
        ),
        child: row,
      ),
    );
  }
}

// --- Conflicts -----------------------------------------------------------------

/// One item changed on two computers: this computer's version and the
/// other's side by side, with what each is and when, and the three ways out.
/// With no [onResolve] it is a card on its way out.
class _ConflictBlock extends StatelessWidget {
  final LibraryConflict conflict;
  final List<LibraryMachine> machines;
  final void Function(LibraryResolution r)? onResolve;

  /// Whether it is the first on the page, which has the plain keys.
  final bool first;

  const _ConflictBlock(
      {super.key,
      required this.conflict,
      required this.machines,
      required this.onResolve,
      this.first = false});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final c = conflict;
    final e = c.entry;
    final theirs = c.theirs;
    final me = machines.where((m) => m.me).firstOrNull;
    final other = machines.where((m) => m.id == theirs.device).firstOrNull;
    final mineName = me == null ? l10n.libraryThisComputer : libraryMachineName(me);
    final theirName = theirs.deviceName.isNotEmpty
        ? theirs.deviceName
        : other == null
            ? ''
            : libraryMachineName(other);
    // What each side holds, a line each: a preset's effects one by one,
    // else its one line, else its name.
    List<String> lines(String detail) {
      final said = libraryDetailOf(e.kind, detail);
      if (said.isEmpty) return [e.name];
      return e.kind == LibraryKind.presets ? said.split(', ') : [said];
    }

    final mine = lines(e.detail);
    final their = lines(c.theirsDetail);
    return Padding(
      padding: const EdgeInsets.only(bottom: 18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.baseline,
            textBaseline: TextBaseline.alphabetic,
            children: [
              Flexible(
                child: Text(e.name,
                    style: t.bodyStrong.copyWith(color: t.textPrimary, fontSize: 13),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
              ),
              const SizedBox(width: 8),
              Text(
                [libraryKindLabel(e.kind), if (e.group case final g?) g].join(' · '),
                style: t.small.copyWith(color: t.textMuted),
              ),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(
                child: _card(
                  t,
                  who: mineName,
                  initial: me?.name ?? mineName,
                  when: c.mineChanged ?? e.modified,
                  size: e.size,
                  rows: mine,
                  other: their,
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: _card(
                  t,
                  who: theirName,
                  initial: theirName,
                  when: theirs.created,
                  size: theirs.size,
                  rows: their,
                  other: mine,
                ),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Row(
            children: [
              _choice(t, 'mine', l10n.libraryKeepThisOne, LibraryResolution.keepMine),
              const SizedBox(width: 6),
              _choice(t, 'theirs', l10n.libraryKeepTheOther, LibraryResolution.keepTheirs),
              const SizedBox(width: 6),
              _choice(t, 'both', l10n.libraryKeepBoth, LibraryResolution.keepBoth),
            ],
          ),
        ],
      ),
    );
  }

  Widget _choice(LumitTheme t, String id, String label, LibraryResolution r) {
    final resolve = onResolve;
    return HouseButton(
      key: ValueKey<String>(first
          ? 'library-keep-$id'
          : 'library-keep-$id-${conflict.entry.key}'),
      small: true,
      padding: const EdgeInsets.symmetric(horizontal: 10),
      onPressed: resolve == null ? null : () => resolve(r),
      child: Text(label, style: t.small),
    );
  }

  /// One side: whose it is, when it changed and how big it is, then what is
  /// in it. A line the other side lacks is outlined in the accent.
  Widget _card(LumitTheme t,
      {required String who,
      required String initial,
      required DateTime? when,
      required int size,
      required List<String> rows,
      required List<String> other}) {
    final letter = initial.trim().isEmpty
        ? ''
        : String.fromCharCode(initial.trim().runes.first).toUpperCase();
    final differs = rows.length != other.length;
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: t.hairline),
        borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
      ),
      padding: const EdgeInsets.fromLTRB(10, 8, 10, 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              LibraryDisc(hold: LibraryHold.newest, initial: letter, size: 12, theme: t),
              const SizedBox(width: 8),
              Expanded(
                child: Text(who,
                    style: t.bodyPrimary, maxLines: 1, overflow: TextOverflow.ellipsis),
              ),
              Text(
                  [
                    if (when != null) backupWhen(when),
                    if (size > 0) writtenBytes(size),
                  ].join(' · '),
                  style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
            ],
          ),
          const SizedBox(height: 8),
          for (final (i, line) in rows.indexed)
            Container(
              margin: const EdgeInsets.only(bottom: 4),
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              decoration: BoxDecoration(
                border: Border.all(
                    color: !other.contains(line) ||
                            (!differs && other.indexOf(line) != i)
                        ? t.accent
                        : t.hairline),
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              ),
              child: Text(line,
                  style: t.body, maxLines: 1, overflow: TextOverflow.ellipsis),
            ),
        ],
      ),
    );
  }
}

/// [child] fading out and folding up over [spec], then gone. Under a still
/// spec it goes at once.
class _Leave extends StatefulWidget {
  final MotionSpec spec;
  final VoidCallback onDone;
  final Widget child;
  const _Leave(
      {super.key, required this.spec, required this.onDone, required this.child});

  @override
  State<_Leave> createState() => _LeaveState();
}

class _LeaveState extends State<_Leave> {
  bool _gone = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      if (widget.spec.isStill) {
        widget.onDone();
        return;
      }
      setState(() => _gone = true);
      Future<void>.delayed(widget.spec.duration, () {
        if (mounted) widget.onDone();
      });
    });
  }

  @override
  Widget build(BuildContext context) => IgnorePointer(
        child: AnimatedOpacity(
          opacity: _gone ? 0 : 1,
          duration: widget.spec.duration,
          curve: widget.spec.curve,
          child: AnimatedSize(
            duration: widget.spec.duration,
            curve: widget.spec.curve,
            alignment: Alignment.topCenter,
            child: _gone ? const SizedBox(width: double.infinity) : widget.child,
          ),
        ),
      );
}

/// A file's name without its extension.
String _stemOf(String file) {
  final dot = file.lastIndexOf('.');
  return dot <= 0 ? file : file.substring(0, dot);
}
