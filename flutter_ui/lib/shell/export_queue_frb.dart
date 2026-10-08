// The export queue window: what is waiting, what is writing, what landed.
//
// **In plain terms.** Only one export can run at a time, so the rest wait in a
// list — and this is that list. Each row is one item: the composition it was
// made from, where it writes, and how far it has got. An item was photographed
// when it was added (docs/06 §7.1), so a row goes on writing what you queued
// however much the project changes afterwards.
//
// It is built to the shared dialog pattern like every other popup — the same
// title strip, rows and footer — because the approved mockups draw the export
// *dialog* and leave the queue to the pattern.
//
// **Nothing here decides anything.** The engine holds the queue and turns it
// over; this window asks what is in it a few times a second and draws the
// answer. The one bridge call in a rebuild path would be that ask, so it is not
// in one: the timer reads, and `build` draws what was read.
//
// **A waiting row is dragged to move it.** The order of the queue is the order
// the exports run in, and dragging the row is the gesture the rest of the
// application already uses to reorder a list — layers in the Timeline, items
// in the Project panel, effects in a stack. Only what is still waiting can be
// picked up: the engine refuses a row that is running, has run, or has gone,
// in its own words, and the refusal is nothing to draw because the next poll
// shows the list unchanged.
//
// **How the drag is drawn follows the Motion setting** (docs/15 §8.1). At Full
// the row is carried and the rows it passes step aside. At Minimal and None
// nothing moves: an accent line marks the place the row would take, the same
// line an effect stack draws.

import 'dart:async';

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show revealInFolder;

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'status_line_frb.dart';

/// The frame: wide enough for a name, a path and a status side by side.
const double exportQueueWidth = 560;

/// One item's row, and the tallest the list grows before it scrolls.
const double exportQueueRow = 28;
const double exportQueueListMax = 320;

/// How often the window asks the engine how the queue is getting on. The same
/// cadence the export dialog used to poll at: fast enough to feel live, slow
/// enough that asking is not itself work.
const Duration _pollInterval = Duration(milliseconds: 250);

Future<void> showExportQueueFrb({
  required BuildContext context,
  List<BridgeExportQueueItem> Function()? list,
  void Function({required int id, required int index})? move,
}) =>
    showLumitModal<void>(
      context: context,
      id: 'export-queue',
      builder: (close) => _ExportQueue(
        list: list,
        move: move,
        onClose: () => close(null),
      ),
    );

class _ExportQueue extends StatefulWidget {
  /// The read seam, injected by tests so no engine has to hold a queue.
  final List<BridgeExportQueueItem> Function()? list;

  /// The reorder seam, injected beside it for the same reason.
  final void Function({required int id, required int index})? move;
  final VoidCallback onClose;

  const _ExportQueue(
      {required this.list, required this.move, required this.onClose});

  @override
  State<_ExportQueue> createState() => _ExportQueueState();
}

class _ExportQueueState extends State<_ExportQueue> {
  List<BridgeExportQueueItem> _items = const [];
  Timer? _poll;

  /// A drag that is marked rather than carried: the item in hand, where the
  /// pointer went down, and the place it is aimed at. All null between drags,
  /// and never set while rows are carried.
  int? _markedId;
  double _markedFrom = 0;
  int? _markedTo;

  @override
  void initState() {
    super.initState();
    _refresh();
    _poll = Timer.periodic(_pollInterval, (_) => _refresh());
  }

  @override
  void dispose() {
    _poll?.cancel();
    super.dispose();
  }

  void _refresh() {
    if (!mounted) return;
    setState(() => _items = (widget.list ?? exportQueueList)());
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final carries = scope.motion.carries;
    final waiting = _items
        .where((item) => item.state is BridgeExportQueueState_Waiting)
        .length;
    final done = _items
        .where((item) => item.state is BridgeExportQueueState_Done)
        .length;

    return DialogFrame(
      width: exportQueueWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.exportQueueTitle,
          onClose: widget.onClose,
          keyPrefix: 'export-queue',
        ),
        Padding(
          padding: exportBodyPaddingForQueue,
          child: _items.isEmpty
              ? SizedBox(
                  height: exportQueueRow,
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: Text(l10n.exportQueueEmpty,
                        key: const ValueKey('export-queue-empty'),
                        style: t.body.copyWith(color: t.textMuted)),
                  ),
                )
              : ConstrainedBox(
                  constraints:
                      const BoxConstraints(maxHeight: exportQueueListMax),
                  child: CustomScrollView(
                    shrinkWrap: true,
                    slivers: [
                      if (carries)
                        SliverReorderableList(
                          itemCount: _items.length,
                          itemBuilder: (context, index) =>
                              _row(t, _items[index], index),
                          // `onReorderItem` gives the place the row lands in
                          // the list without it, which is the index the
                          // engine's own move takes.
                          onReorderItem: _move,
                          proxyDecorator: (child, index, animation) =>
                              _lifted(t, child),
                        )
                      else
                        SliverList.builder(
                          itemCount: _items.length,
                          itemBuilder: (context, index) =>
                              _markedRow(t, _items[index], index),
                        ),
                    ],
                  ),
                ),
        ),
        dialogFooter(
          t,
          summary: l10n.exportQueueSummary('$waiting', '$done'),
          keyPrefix: 'export-queue',
          actions: [
            HouseButton(
              key: const ValueKey('export-queue-dismiss'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: widget.onClose,
              child: Text(l10n.close),
            ),
            HouseButton(
              key: const ValueKey('export-queue-start'),
              primary: true,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              onPressed: waiting == 0 ? null : _start,
              child: Text(l10n.exportQueueStart),
            ),
          ],
        ),
      ],
    );
  }

  void _start() {
    exportQueueStart();
    // Wake the status line, which polls only while an export is live.
    statusLineExportStarted.value++;
    _refresh();
  }

  /// Move a waiting item to another place in the list. The engine refuses
  /// anything else — running, run, or gone — and its refusal needs no notice
  /// here, because the list that comes back is the answer.
  void _move(int from, int to) {
    final items = _items;
    if (from < 0 || from >= items.length) return;
    try {
      (widget.move ?? exportQueueMove)(id: items[from].id, index: to);
    } catch (_) {
      // The queue turned over under the drag; re-reading is the recovery.
    }
    _refresh();
  }

  /// One item's row, and — while it is waiting — the grip that moves it.
  ///
  /// The lift is Flutter's own reorderable machinery rather than a bare
  /// `Draggable`: a plain draggable inside a scrolling list loses the gesture
  /// to the list itself, and this is the mechanism that both wins it and
  /// scrolls the list when the row is carried past its edge. A row that is not
  /// waiting has no listener at all, so it can be read and dropped past but
  /// never picked up.
  Widget _row(LumitTheme t, BridgeExportQueueItem item, int index) {
    final row = _opensFolder(item, _rowFace(t, item));
    return KeyedSubtree(
      key: ValueKey<String>('export-queue-row-${item.id}'),
      child: item.state is BridgeExportQueueState_Waiting
          // The row is picked up anywhere along it, not only where there
          // happens to be a letter: a row of text and gaps hit-tests only on
          // its text, and half of it would be dead to the grip.
          ? ReorderableDragStartListener(
              index: index,
              child: Listener(behavior: HitTestBehavior.opaque, child: row),
            )
          : row,
    );
  }

  /// One item's row where a drag is marked and not carried: the same grip,
  /// and the line on whichever edge the row in hand would land against.
  Widget _markedRow(LumitTheme t, BridgeExportQueueItem item, int index) {
    final row = _opensFolder(item, _rowFace(t, item));
    final from = _items.indexWhere((i) => i.id == _markedId);
    final to = _markedTo;
    final marked = from >= 0 && to == index && to != from;
    final below = marked && index > from;
    final line = BorderSide(color: t.accent, width: 2);
    return KeyedSubtree(
      key: ValueKey<String>('export-queue-row-${item.id}'),
      child: DecoratedBox(
        key: marked ? const ValueKey('export-queue-drop-line') : null,
        // Foreground, so the line sits over the row rather than under it,
        // and always in the tree so a row is not rebuilt by being aimed at.
        position: DecorationPosition.foreground,
        decoration: BoxDecoration(
          border: !marked
              ? null
              : below
                  ? Border(bottom: line)
                  : Border(top: line),
        ),
        child: item.state is BridgeExportQueueState_Waiting
            ? GestureDetector(
                behavior: HitTestBehavior.opaque,
                // From where the pointer went down, not from where the drag
                // was recognised: the distance in between is part of how far
                // the row has been taken.
                dragStartBehavior: DragStartBehavior.down,
                onVerticalDragStart: (d) => setState(() {
                  _markedId = item.id;
                  _markedFrom = d.globalPosition.dy;
                  _markedTo = index;
                }),
                onVerticalDragUpdate: (d) {
                  final at = _items.indexWhere((i) => i.id == _markedId);
                  if (at < 0) return;
                  // The place whose row the pointer is past the middle of.
                  final travel = d.globalPosition.dy - _markedFrom;
                  final next = (at + (travel / exportQueueRow).round())
                      .clamp(0, _items.length - 1);
                  if (next != _markedTo) setState(() => _markedTo = next);
                },
                onVerticalDragEnd: (_) {
                  final at = _items.indexWhere((i) => i.id == _markedId);
                  final next = _markedTo;
                  _endMarked();
                  if (at >= 0 && next != null && next != at) _move(at, next);
                },
                onVerticalDragCancel: _endMarked,
                child: row,
              )
            : row,
      ),
    );
  }

  void _endMarked() {
    if (!mounted) return;
    setState(() {
      _markedId = null;
      _markedFrom = 0;
      _markedTo = null;
    });
  }

  /// The row while it is being carried: the panel's own surface under an
  /// accent edge, rather than the Material card the default lift draws.
  Widget _lifted(LumitTheme t, Widget child) => DecoratedBox(
        decoration: BoxDecoration(
          color: t.surface2,
          border: Border.all(color: t.accent),
        ),
        child: child,
      );

  /// A finished row opens the folder its file is in, with a tooltip saying so.
  /// Every other row is handed back as it is: there is no file to show yet.
  /// The remove mark still takes its own click, being the deeper of the two.
  Widget _opensFolder(BridgeExportQueueItem item, Widget row) =>
      item.state is! BridgeExportQueueState_Done
          ? row
          : LumitTooltip(
              message: l10n.tipOpenFolder,
              child: MouseRegion(
                cursor: SystemMouseCursors.click,
                child: GestureDetector(
                  key: ValueKey<String>('export-queue-open-${item.id}'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => revealInFolder(path: item.path),
                  child: row,
                ),
              ),
            );

  /// One item: what it is, where it goes, how it is getting on, and the mark
  /// that takes it off the list.
  Widget _rowFace(LumitTheme t, BridgeExportQueueItem item) => SizedBox(
        height: exportQueueRow,
        child: Row(
          key: ValueKey<String>('export-queue-item-${item.id}'),
          children: [
            SizedBox(
              width: 140,
              child: Text(item.compName,
                  style: t.body, overflow: TextOverflow.ellipsis),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                _leaf(item.path),
                style: dialogMono(t),
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(width: 10),
            Flexible(flex: 2, child: _statusFace(t, item)),
            const SizedBox(width: 8),
            LumitTooltip(
              message: item.state is BridgeExportQueueState_Running
                  ? l10n.exportCancel
                  : l10n.exportQueueRemove,
              child: GestureDetector(
                key: ValueKey<String>('export-queue-drop-${item.id}'),
                behavior: HitTestBehavior.opaque,
                onTap: () {
                  // Cancel while it writes, forget once it has stopped: one
                  // mark, because "take this off my list" is one wish.
                  if (item.state is BridgeExportQueueState_Running) {
                    exportQueueCancel(id: item.id);
                  } else {
                    exportQueueRemove(id: item.id);
                  }
                  _refresh();
                },
                child: SizedBox(
                  width: dialogCloseGlyph + 8,
                  height: exportQueueRow,
                  child: Center(
                    child: glyph.LumitIcon(
                      LumitIcons.close,
                      size: dialogCloseGlyph,
                      colour: t.textMuted,
                      semanticLabel: l10n.exportQueueRemove,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      );

  /// How one item is getting on. An error is longer than the column, so a
  /// failed item shows all of it on hover.
  Widget _statusFace(LumitTheme t, BridgeExportQueueItem item) {
    final failed = item.state is BridgeExportQueueState_Failed;
    final text = Text(
      _status(item),
      key: ValueKey<String>('export-queue-status-${item.id}'),
      style: dialogMono(t).copyWith(color: failed ? t.warning : t.textMuted),
      overflow: TextOverflow.ellipsis,
      textAlign: TextAlign.right,
    );
    return failed ? LumitTooltip(message: _status(item), child: text) : text;
  }

  /// What one item is doing, in the engine's own words where it has any.
  String _status(BridgeExportQueueItem item) => switch (item.state) {
        BridgeExportQueueState_Waiting() => l10n.exportQueueWaiting,
        BridgeExportQueueState_Running(:final total, :final encoder)
            when total == BigInt.zero =>
          l10n.exportPreparing(encoder),
        BridgeExportQueueState_Running(
          :final frame,
          :final total,
          :final encoder
        ) =>
          l10n.exportFrameOf('$frame', '$total', encoder),
        BridgeExportQueueState_Done() => l10n.exportQueueDone,
        BridgeExportQueueState_Failed(:final error) => error,
      };

  static String _leaf(String path) => path.split(RegExp(r'[/\\]')).last;
}

/// The list's own inset — the export dialog's body padding, which is the
/// pattern's (§12A.4).
const EdgeInsets exportBodyPaddingForQueue =
    EdgeInsets.fromLTRB(14, 10, 14, 12);
