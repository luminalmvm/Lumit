// The Data window: the rows of a file feeding a graphics template's controls.
//
// One window, bound to one template at a time. Opening it for another
// template points it there. It floats over the work, so a copy can be
// selected while it is up, and the row in hand is shown on that copy.
//
// The table, the feeds and the faults are the engine's. The window reads them
// once for each document change, never in `build`, and forwards what is
// pressed.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/cut.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/state.dart' show ScopedChange;
import 'package:lumit_flutter/src/rust/api/template.dart';
import 'package:lumit_flutter/src/rust/api/template/data.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../state/dock.dart' show WorkspacePreset;
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/middle_ellipsis.dart';
import 'dialog_frame.dart';
import 'export_queue_frb.dart' show exportQueueRow;
import 'template_rows_export.dart';

/// The row number column, a data column at its narrowest, and the header's
/// two lines.
const double _numberColumn = 40;
const double _cellWidth = 132;
const double _headerHeight = 40;
const double _toolBarHeight = 36;

/// How long a burst of edits settles before the data is read again.
const Duration _settle = Duration(milliseconds: 120);

/// Open the Data window on [comp], or point the open one at it.
Future<void> showTemplateDataWindowFor(
    BuildContext context, CompositionReference comp) async {
  final open = _DataWindowState._open;
  if (open != null) {
    open.switchTo(comp);
    return;
  }
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  await showLumitModal<void>(
    context: context,
    id: 'template-data',
    modeless: true,
    initialSize: const Size(760, 420),
    minSize: const Size(480, 220),
    builder: (close) =>
        _DataWindow(comp: comp, app: app, ui: ui, onClose: () => close(null)),
  );
}

/// What a column can feed, as its header's menu lists it.
class _Feed {
  final BridgeFeeds feeds;
  final String? control;
  final String label;
  const _Feed(this.feeds, this.control, this.label);

  bool matches(BridgeDataColumn column) =>
      column.feeds == feeds && column.control == control;
}

class _DataWindow extends StatefulWidget {
  final CompositionReference comp;
  final LumitState app;
  final LumitUiState ui;
  final VoidCallback onClose;

  const _DataWindow({
    required this.comp,
    required this.app,
    required this.ui,
    required this.onClose,
  });

  @override
  State<_DataWindow> createState() => _DataWindowState();
}

class _DataWindowState extends State<_DataWindow> {
  /// The one window up, so opening again points it rather than stacking.
  static _DataWindowState? _open;

  late CompositionReference _comp;
  String _name = '';
  BridgeData? _data;
  BridgeTemplate? _template;
  Map<String, BridgeControl> _controls = const {};
  Set<(int, int)> _faults = const {};

  /// The picked rows in the order they were picked, the one the keys move,
  /// and the one a Shift press reaches from.
  final Set<int> _picked = {};
  int? _head;
  int? _anchor;

  /// The selected copy of this template, if the app's selection is one.
  CopyHost? _host;

  /// What the Viewer was last asked to show, so it is asked only on a change.
  (int?, String?, String?)? _shown;

  bool _busy = false;
  StreamSubscription<ScopedChange>? _changes;
  Timer? _debounce;
  final ControlFocusNode _focus = ControlFocusNode(debugLabel: 'data');
  final ScrollController _rows = ScrollController();
  final ScrollController _across = ScrollController();

  @override
  void initState() {
    super.initState();
    _open = this;
    _comp = widget.comp;
    _changes = widget.app.onChange.listen(_changed);
    widget.ui.model.addListener(_selectionMoved);
    widget.ui.selectedLayer.addListener(_selectionMoved);
    widget.ui.cutClip.addListener(_selectionMoved);
    _read();
    _findHost();
    WidgetsBinding.instance.addPostFrameCallback((_) => _show());
  }

  @override
  void dispose() {
    if (identical(_open, this)) _open = null;
    _changes?.cancel();
    _debounce?.cancel();
    widget.ui.model.removeListener(_selectionMoved);
    widget.ui.selectedLayer.removeListener(_selectionMoved);
    widget.ui.cutClip.removeListener(_selectionMoved);
    _unshow();
    _focus.dispose();
    _rows.dispose();
    _across.dispose();
    super.dispose();
  }

  /// Point the window at [comp], which is what opening it again does.
  void switchTo(CompositionReference comp) {
    if (comp.internalid == _comp.internalid) return;
    _unshow();
    setState(() {
      _comp = comp;
      _picked.clear();
      _head = null;
      _anchor = null;
      _read();
      _findHost();
    });
    _show();
  }

  // ---- reading ---------------------------------------------------------------

  void _read() {
    try {
      _name = _comp.getSettings().name;
      _template = _comp.getTemplate();
      _data = _comp.getData();
    } catch (_) {
      // The composition went under the window, which then shows no data.
      _template = null;
      _data = null;
    }
    _controls = {for (final c in _template?.controls ?? const []) c.id: c};
    _faults = {for (final f in _data?.faults ?? const []) (f.row, f.column)};
    final count = _data?.rows.length ?? 0;
    _picked.removeWhere((row) => row >= count);
    if (_head != null && _head! >= count) _head = _picked.lastOrNull;
    if (_anchor != null && _anchor! >= count) _anchor = _head;
  }

  void _changed(ScopedChange change) {
    if (!mounted) return;
    _debounce?.cancel();
    _debounce = Timer(_settle, () {
      if (!mounted) return;
      setState(() {
        _read();
        _findHost();
      });
      _show();
    });
  }

  void _selectionMoved() {
    if (!mounted) return;
    final was = _host;
    _findHost();
    final same = was?.layer.internallayerId == _host?.layer.internallayerId &&
        was?.clip == _host?.clip &&
        was?.copy.row == _host?.copy.row;
    if (same) return;
    setState(() {});
    _show();
  }

  /// The selected copy of this template: the clip picked in the Cut
  /// timeline in the Cut workspace, else the selected layer.
  void _findHost() {
    final ui = widget.ui;
    final id = _comp.internalid;
    CopyHost? found;
    final clip = ui.cutClip.value;
    if (clip != null && ui.workspace.activePreset == WorkspacePreset.cut) {
      final entry = ui.model.byId(clip.layer.internallayerId);
      for (final c in entry?.info.clips ?? const <BridgeClip>[]) {
        final copy = c.copy;
        if (c.id == clip.clip && copy != null && copy.comp == id) {
          found = (layer: clip.layer, clip: clip.clip, copy: copy);
        }
      }
    }
    final layer = ui.selectedLayer.value;
    if (found == null && layer != null) {
      final copy = ui.model.byId(layer.internallayerId)?.info.copy;
      if (copy != null && copy.comp == id) {
        found = (layer: layer, clip: null, copy: copy);
      }
    }
    _host = found;
  }

  // ---- the Viewer ------------------------------------------------------------

  /// Show the row under the selection's head in the Viewer, on the selected
  /// copy or on the template itself.
  void _show() {
    if (!mounted) return;
    final host = _host;
    final row = _data == null ? null : _head;
    final key = (row, host?.layer.internallayerId.toString(),
        host?.clip?.toString());
    if (_shown == key) return;
    _shown = key;
    try {
      _comp.showDataRow(row: row, host: host?.layer, hostClip: host?.clip);
    } catch (_) {
      // The template or the copy went under the window.
    }
    widget.ui.requestFrame();
  }

  void _unshow() {
    _shown = null;
    try {
      _comp.showDataRow(row: null);
    } catch (_) {
      // The composition has gone, and with it what it showed.
    }
    widget.ui.requestFrame();
  }

  // ---- picking rows ----------------------------------------------------------

  int get _count => _data?.rows.length ?? 0;

  void _press(int row) {
    final keys = HardwareKeyboard.instance;
    final toggle = keys.isControlPressed || keys.isMetaPressed;
    setState(() {
      if (keys.isShiftPressed && _anchor != null) {
        if (!toggle) _picked.clear();
        _pickRange(_anchor!, row);
      } else if (toggle) {
        if (!_picked.remove(row)) _picked.add(row);
        _anchor = row;
      } else {
        _picked
          ..clear()
          ..add(row);
        _anchor = row;
      }
      _head = _picked.contains(row) ? row : _picked.lastOrNull;
    });
    _focus.requestFocus();
    _show();
  }

  void _pickRange(int from, int to) {
    final low = math.min(from, to);
    final high = math.max(from, to);
    for (var i = low; i <= high; i++) {
      _picked.add(i);
    }
  }

  /// Walk the rows with the arrow keys: one row, or with Shift a run from
  /// the anchor.
  void _walk(int by) {
    if (_count == 0) return;
    final from = _head ?? (by > 0 ? -1 : _count);
    final to = (from + by).clamp(0, _count - 1);
    setState(() {
      if (HardwareKeyboard.instance.isShiftPressed) {
        _anchor ??= to;
        _picked.clear();
        _pickRange(_anchor!, to);
      } else {
        _picked
          ..clear()
          ..add(to);
        _anchor = to;
      }
      _head = to;
    });
    _reveal(to);
    _show();
  }

  /// Scroll so [row] is in view.
  void _reveal(int row) {
    if (!_rows.hasClients) return;
    final top = row * exportQueueRow;
    final view = _rows.position.viewportDimension;
    final at = _rows.offset;
    if (top < at) {
      _rows.jumpTo(top);
    } else if (top + exportQueueRow > at + view) {
      _rows.jumpTo(top + exportQueueRow - view);
    }
  }

  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final keys = HardwareKeyboard.instance;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowUp:
        _walk(-1);
      case LogicalKeyboardKey.arrowDown:
        _walk(1);
      case LogicalKeyboardKey.keyA
          when keys.isControlPressed || keys.isMetaPressed:
        if (_count == 0) return KeyEventResult.ignored;
        setState(() {
          _picked.addAll([for (var i = 0; i < _count; i++) i]);
          _head ??= 0;
          _anchor ??= 0;
        });
        _show();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  /// The picked rows in order, or every row when none is picked.
  List<int> get _rowsAsked => _picked.isEmpty
      ? [for (var i = 0; i < _count; i++) i]
      : (_picked.toList()..sort());

  // ---- acts ------------------------------------------------------------------

  void _edited() {
    widget.app.notifyDocumentChanged();
    widget.ui.model.refresh();
    setState(() {
      _read();
      _findHost();
    });
    _shown = null;
    _show();
  }

  Future<void> _openFile() async {
    final path = await pickTemplateData();
    if (path == null || !mounted) return;
    await _run(() => _comp.linkData(path: path, folder: l10n.templateDataFolder),
        l10n.templateDataOpenFailed);
  }

  Future<void> _reload() => _run(
      () => _comp.reloadData(folder: l10n.templateDataFolder),
      l10n.templateDataOpenFailed);

  Future<void> _saveStarting() async {
    final path = await pickTemplateDataSaveLocation(_name);
    if (path == null || !mounted) return;
    await _run(() => _comp.saveStartingData(path: path),
        l10n.templateDataSaveFailed,
        edits: false);
  }

  Future<void> _run(Future<void> Function() act, String failed,
      {bool edits = true}) async {
    setState(() => _busy = true);
    try {
      await act();
    } catch (_) {
      widget.app.postNotice(failed, error: true);
    }
    if (!mounted) return;
    setState(() => _busy = false);
    if (edits) _edited();
  }

  void _unlink() {
    try {
      _comp.unlinkData();
    } catch (_) {
      widget.app.postNotice(l10n.templateEditFailed, error: true);
      return;
    }
    _picked.clear();
    _head = null;
    _anchor = null;
    _edited();
  }

  void _feed(int column, _Feed feed) {
    try {
      _comp.setDataFeed(
          column: column, feeds: feed.feeds, control: feed.control);
    } catch (_) {
      widget.app.postNotice(l10n.templateEditFailed, error: true);
      return;
    }
    _edited();
  }

  bool get _follows {
    final host = _host;
    return host != null && _head != null && host.copy.row == _head;
  }

  void _follow() {
    final host = _host;
    if (host == null || _head == null) return;
    try {
      host.layer.followDataRow(row: _follows ? null : _head, clip: host.clip);
    } catch (_) {
      widget.app.postNotice(l10n.templateEditFailed, error: true);
      return;
    }
    _edited();
  }

  /// Place a copy for each row asked into the composition the Timeline shows,
  /// or on a new track in Cut, from the playhead.
  void _place() {
    final ui = widget.ui;
    final target = ui.selectedComp;
    if (target == null) return;
    final rows = _rowsAsked;
    final at = ui.playheadFrame.value;
    if (ui.workspace.activePreset == WorkspacePreset.cut) {
      final BridgeCutResult result;
      try {
        result = target.cutPlaceRows(template: _comp, rows: rows, atFrame: at);
      } catch (_) {
        widget.app.postNotice(l10n.templateDataPlaceFailed, error: true);
        return;
      }
      ui.reportCut(result);
      if (result != BridgeCutResult.done) return;
    } else {
      try {
        target.placeDataRows(template: _comp, rows: rows, atFrame: at);
      } catch (_) {
        widget.app.postNotice(l10n.templateDataPlaceFailed, error: true);
        return;
      }
    }
    _edited();
  }

  void _exportRows() {
    final data = _data;
    if (data == null) return;
    String? hostComp;
    final host = _host;
    if (host != null) {
      for (final (comp, name) in widget.app.comps()) {
        if (comp.internalid == host.layer.internalcompId) hostComp = name;
      }
    }
    unawaited(showTemplateRowsExport(
      context: context,
      comp: _comp,
      name: _name,
      data: data,
      template: _template,
      rows: _rowsAsked,
      host: host,
      hostComp: hostComp,
    ));
  }

  // ---- the window ------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final data = _data;
    return Focus(
      focusNode: _focus,
      autofocus: true,
      onKeyEvent: _key,
      child: TapRegion(
        onTapOutside: (_) => _focus.unfocus(),
        onTapInside: (_) {
          final held = FocusManager.instance.primaryFocus;
          if (held == null || !held.ancestors.contains(_focus)) {
            _focus.requestFocus();
          }
        },
        child: DialogFrame(
          children: [
            dialogTitleBar(
              t,
              title: l10n.templateDataTitle,
              subject: _name,
              onClose: widget.onClose,
              keyPrefix: 'template-data',
            ),
            if (data == null)
              Expanded(child: _empty(t))
            else ...[
              _toolBar(context, t),
              Expanded(child: _table(context, t, data)),
              _foot(t, data),
            ],
          ],
        ),
      ),
    );
  }

  /// No data yet: the two ways to start, and nothing else.
  Widget _empty(LumitTheme t) => Center(
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            _button(t, 'template-data-open', l10n.templateDataOpen,
                _busy ? null : _openFile),
            const SizedBox(width: 8),
            _button(t, 'template-data-starting', l10n.templateDataSaveStarting,
                _busy ? null : _saveStarting),
          ],
        ),
      );

  Widget _button(
          LumitTheme t, String id, String label, VoidCallback? onPressed) =>
      SizedBox(
        height: dialogControlHeight,
        child: HouseButton(
          key: ValueKey<String>(id),
          padding: const EdgeInsets.symmetric(horizontal: 12),
          onPressed: onPressed,
          child: Text(label, style: t.body),
        ),
      );

  Widget _toolBar(BuildContext context, LumitTheme t) {
    final host = _host;
    final idle = !_busy;
    return Container(
      height: _toolBarHeight,
      padding: const EdgeInsets.symmetric(horizontal: dialogPadding),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      child: Row(
        children: [
          _button(t, 'template-data-open', l10n.templateDataOpen,
              idle ? _openFile : null),
          const SizedBox(width: 6),
          _button(t, 'template-data-reload', l10n.templateDataReload,
              idle ? _reload : null),
          const Spacer(),
          if (host != null) ...[
            _button(
                t,
                'template-data-follow',
                _follows ? l10n.templateDataStopFollowing : l10n.templateDataFollow,
                idle && _head != null ? _follow : null),
            const SizedBox(width: 6),
          ],
          _button(t, 'template-data-place', l10n.templateDataPlace,
              idle && _count > 0 && widget.ui.selectedComp != null
                  ? _place
                  : null),
          const SizedBox(width: 6),
          _button(t, 'template-data-export', l10n.templateDataExport,
              idle && _count > 0 ? _exportRows : null),
          const SizedBox(width: 6),
          _more(t),
        ],
      ),
    );
  }

  /// The window's menu: the starting file, and unlinking.
  Widget _more(LumitTheme t) => LumitTooltip(
        message: l10n.templateDataMore,
        child: Builder(
          builder: (context) => SizedBox(
            width: dialogControlHeight,
            height: dialogControlHeight,
            child: HouseButton(
              key: const ValueKey('template-data-more'),
              padding: EdgeInsets.zero,
              onPressed: () {
                final box = context.findRenderObject()! as RenderBox;
                showMenuAt<void>(
                  context: context,
                  position: box.localToGlobal(Offset(0, box.size.height + 2)),
                  rows: (close) => [
                    MenuRow(
                      key: const ValueKey('template-data-menu-starting'),
                      enabled: !_busy,
                      onPressed: () {
                        close(null);
                        unawaited(_saveStarting());
                      },
                      child: MenuLabel(l10n.templateDataSaveStarting,
                          enabled: !_busy),
                    ),
                    MenuRow(
                      key: const ValueKey('template-data-menu-unlink'),
                      enabled: !_busy,
                      onPressed: () {
                        close(null);
                        _unlink();
                      },
                      child:
                          MenuLabel(l10n.templateDataUnlink, enabled: !_busy),
                    ),
                  ],
                );
              },
              child: glyph.LumitIcon(LumitIcons.more,
                  size: 12, colour: t.textSecondary),
            ),
          ),
        ),
      );

  /// Where the file is, cut in the middle, and how many rows it has.
  Widget _foot(LumitTheme t, BridgeData data) => dialogFooter(
        t,
        keyPrefix: 'template-data',
        leading: Row(
          children: [
            Flexible(
              child: MiddleEllipsis(data.file, style: dialogMono(t)),
            ),
            Text(' · ${l10n.templateDataRows(data.rows.length)}',
                style: dialogMono(t)),
          ],
        ),
        actions: const [],
      );

  // ---- the table -------------------------------------------------------------

  /// What a column can feed: nothing, each control, a text control's font,
  /// size and colour where a copy may change them, its length and its file's
  /// name.
  List<_Feed> get _feeds => [
        _Feed(BridgeFeeds.nothing, null, l10n.templateDataNothing),
        for (final c in _template?.controls ?? const <BridgeControl>[])
          if (!c.gone) ...[
            _Feed(BridgeFeeds.control, c.id, c.label),
            if (c.kind == BridgeControlKind.text) ...[
              if (c.font)
                _Feed(BridgeFeeds.font, c.id, l10n.templateDataFont(c.label)),
              if (c.size)
                _Feed(BridgeFeeds.size, c.id, l10n.templateDataSize(c.label)),
              if (c.colour)
                _Feed(BridgeFeeds.colour, c.id,
                    l10n.templateDataColour(c.label)),
            ],
          ],
        _Feed(BridgeFeeds.length, null, l10n.templateDataLength),
        _Feed(BridgeFeeds.fileName, null, l10n.templateDataFileName),
      ];

  /// What the header says a column feeds, or nothing when it feeds nothing.
  String _feedLabel(BridgeDataColumn column) {
    final label = _controls[column.control]?.label ?? '';
    return switch (column.feeds) {
      BridgeFeeds.nothing => '',
      BridgeFeeds.control => label,
      BridgeFeeds.font => l10n.templateDataFont(label),
      BridgeFeeds.size => l10n.templateDataSize(label),
      BridgeFeeds.colour => l10n.templateDataColour(label),
      BridgeFeeds.length => l10n.templateDataLength,
      BridgeFeeds.fileName => l10n.templateDataFileName,
    };
  }

  Widget _table(BuildContext context, LumitTheme t, BridgeData data) =>
      LayoutBuilder(builder: (context, box) {
        final columns = data.columns.length;
        final cell = columns == 0
            ? _cellWidth
            : math.max(_cellWidth, (box.maxWidth - _numberColumn) / columns);
        final width = _numberColumn + cell * columns;
        return dialogScroller(
          context,
          t,
          _across,
          child: SingleChildScrollView(
            controller: _across,
            scrollDirection: Axis.horizontal,
            child: SizedBox(
              width: math.max(width, box.maxWidth),
              height: box.maxHeight,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _header(context, t, data, cell),
                  Expanded(
                    child: dialogScroller(
                      context,
                      t,
                      _rows,
                      child: ListView.builder(
                        controller: _rows,
                        itemExtent: exportQueueRow,
                        itemCount: data.rows.length,
                        itemBuilder: (context, row) =>
                            _row(t, data, row, cell),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
      });

  Widget _header(
          BuildContext context, LumitTheme t, BridgeData data, double cell) =>
      Container(
        height: _headerHeight,
        decoration: BoxDecoration(
          color: t.surface2,
          border: Border(bottom: BorderSide(color: t.hairline)),
        ),
        child: Row(
          children: [
            const SizedBox(width: _numberColumn),
            for (final (i, column) in data.columns.indexed)
              SizedBox(width: cell, child: _headerCell(context, t, i, column)),
          ],
        ),
      );

  /// A column's name and what it feeds. A press opens the menu choosing
  /// that.
  Widget _headerCell(
      BuildContext context, LumitTheme t, int index, BridgeDataColumn column) {
    final quiet = column.feeds == BridgeFeeds.nothing;
    final feeds = _feedLabel(column);
    return Builder(
      builder: (context) {
        void menu(Offset at) => showMenuAt<void>(
              context: context,
              position: at,
              rows: (close) => [
                for (final feed in _feeds)
                  MenuRow(
                    key: ValueKey<String>(
                        'template-data-feed-$index-${feed.feeds.name}-${feed.control}'),
                    onPressed: () {
                      close(null);
                      if (!feed.matches(column)) _feed(index, feed);
                    },
                    child: MenuLabel(feed.label, ticked: feed.matches(column)),
                  ),
              ],
            );
        return HoverFill(
          child: GestureDetector(
            key: ValueKey<String>('template-data-column-$index'),
            behavior: HitTestBehavior.opaque,
            onTap: () {
              final box = context.findRenderObject()! as RenderBox;
              menu(box.localToGlobal(Offset(0, box.size.height)));
            },
            onSecondaryTapUp: (up) => menu(up.globalPosition),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(column.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: quiet
                          ? t.body.copyWith(color: t.textMuted)
                          : t.bodyPrimary),
                  if (feeds.isNotEmpty)
                    Text(feeds,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: t.small.copyWith(color: t.textMuted)),
                ],
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _row(LumitTheme t, BridgeData data, int row, double cell) {
    final cells = data.rows[row].cells;
    final picked = _picked.contains(row);
    return HoverFill(
      faint: true,
      child: GestureDetector(
        key: ValueKey<String>('template-data-row-$row'),
        behavior: HitTestBehavior.opaque,
        onTapDown: (_) => _press(row),
        child: Container(
          color: picked ? t.selectionFill : null,
          child: Row(
            children: [
              SizedBox(
                width: _numberColumn,
                child: Padding(
                  padding: const EdgeInsetsDirectional.only(end: 8),
                  child: Text('${row + 1}',
                      textAlign: TextAlign.end,
                      style: t.mono.copyWith(color: t.textMuted)),
                ),
              ),
              for (final (i, column) in data.columns.indexed)
                SizedBox(
                  width: cell,
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    child: _cell(t, column, i < cells.length ? cells[i] : '',
                        _faults.contains((row, i))),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }

  /// One cell: its text, mono for a number, underlined in the error ink when
  /// it will not read. A colour shows a swatch before it, and a picture its
  /// file's name.
  Widget _cell(LumitTheme t, BridgeDataColumn column, String text, bool fault) {
    final control = _controls[column.control];
    final quiet = column.feeds == BridgeFeeds.nothing;
    final colour = column.feeds == BridgeFeeds.colour ||
        (column.feeds == BridgeFeeds.control &&
            control?.kind == BridgeControlKind.colour);
    final footage = column.feeds == BridgeFeeds.control &&
        control?.kind == BridgeControlKind.footage;
    final number = double.tryParse(text.trim().replaceAll(',', '.')) != null;
    var style = number ? t.mono.copyWith(color: t.textPrimary) : t.bodyPrimary;
    if (quiet) style = style.copyWith(color: t.textMuted);
    if (fault) {
      style = style.copyWith(
        decoration: TextDecoration.underline,
        decorationColor: t.error,
        decorationStyle: TextDecorationStyle.wavy,
      );
    }
    final shown = footage ? _leaf(text) : text;
    final swatch = colour && !fault ? _parseColour(text) : null;
    return Row(
      children: [
        if (swatch != null) ...[
          Container(
            width: 10,
            height: 10,
            decoration: BoxDecoration(
              color: swatch,
              border: Border.all(color: t.hairline),
            ),
          ),
          const SizedBox(width: 6),
        ],
        if (footage && fault && text.trim().isNotEmpty) ...[
          glyph.LumitIcon(LumitIcons.missing, size: 12, colour: t.error),
          const SizedBox(width: 4),
        ],
        Flexible(
          child: Text(shown,
              maxLines: 1, overflow: TextOverflow.ellipsis, style: style),
        ),
      ],
    );
  }

  static String _leaf(String path) => path.split(RegExp(r'[/\\]')).last;

  /// A cell's `#RGB`, `#RRGGBB` or `#RRGGBBAA`, as the swatch draws it. Null
  /// for anything else.
  static Color? _parseColour(String text) {
    var hex = text.trim();
    if (!hex.startsWith('#')) return null;
    hex = hex.substring(1);
    if (hex.length == 3) hex = hex.split('').map((c) => '$c$c').join();
    if (hex.length == 6) hex = '${hex}ff';
    if (hex.length != 8) return null;
    final value = int.tryParse(hex, radix: 16);
    if (value == null) return null;
    return Color.fromARGB(value & 0xff, (value >> 24) & 0xff,
        (value >> 16) & 0xff, (value >> 8) & 0xff);
  }
}
