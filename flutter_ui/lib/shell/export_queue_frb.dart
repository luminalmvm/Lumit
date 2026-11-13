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
// **Several comps at once.** *Add compositions…* opens on the comp in front,
// and its + adds any other, each with its own preset, to queue together.
// A waiting row's settings mark opens the export dialogue on that item, so
// each one can have its own settings before it runs.
//
// **A graphics template's rows fold.** The rows queued together from one
// template share a group, and are drawn under one heading, shut until it is
// opened: how many are done, a bar across the run, and the failed ones. A
// shut run is dragged as one block.
//
// **How the drag is drawn follows the Motion setting** (docs/15 §8.1). At Full
// the row is carried and the rows it passes step aside. At Minimal and None
// nothing moves: an accent line marks the place the row would take, the same
// line an effect stack draws.

import 'dart:async';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show revealInFolder;
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/middle_ellipsis.dart';
import 'dialog_frame.dart';
import 'export_dialog_frb.dart';
import 'export_pick_frb.dart';
import 'settings_rows.dart' show byteSize;
import 'status_line_frb.dart';

/// The frame: wide enough for a name, its preset, a path and how it is
/// getting on side by side.
const double exportQueueWidth = 800;

/// One item's row, and the tallest the list grows before it scrolls.
const double exportQueueRow = 28;
const double exportQueueListMax = 320;

/// How far a template's rows are set in under the heading of their run,
/// which is the width of its twirl.
const double _runIndent = 18;

/// One line of the list: an item, or the heading of a run of a template's
/// rows queued together.
class _Line {
  const _Line({this.item, this.inRun = false, this.group, this.run = const []});

  final BridgeExportQueueItem? item;

  /// The item is one of a run's rows, drawn under its open heading.
  final bool inRun;

  /// A heading's group and its rows, in the queue's order.
  final int? group;
  final List<BridgeExportQueueItem> run;

  /// What the line is known by across reads of the queue.
  String get key => item != null ? 'item-${item!.id}' : 'run-$group';
}

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

  /// The open project's compositions by id, so a waiting row can open its
  /// settings. An item from a comp since deleted, or from another project,
  /// finds nothing here and keeps the settings it has.
  Map<String, CompositionReference> _comps = const {};

  /// What the list draws: each item, with a template's rows under the
  /// heading of their run, and the runs opened to show their rows.
  List<_Line> _lines = const [];
  final Set<int> _openRuns = {};

  /// A drag that is marked rather than carried: the line in hand, where the
  /// pointer went down, and the place it is aimed at. All null between drags,
  /// and never set while rows are carried.
  String? _markedKey;
  double _markedFrom = 0;
  int? _markedTo;

  /// When the window first saw each item running, and the frame it was on,
  /// for the time it has left.
  final Map<int, (DateTime, int)> _seenRunning = {};

  /// Whether the queue will start the next item by itself: false before
  /// Start, and after Stop after this one.
  bool _goesOn = false;

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
    try {
      _comps = {
        for (final (comp, _) in context.read<LumitState>().comps())
          comp.internalid.toString(): comp,
      };
    } catch (_) {
      // No project to ask, as in a widget test.
    }
    try {
      _goesOn = exportQueueGoesOn();
    } catch (_) {
      // No engine to ask, as in a widget test.
    }
    setState(() {
      _items = (widget.list ?? exportQueueList)();
      _lines = _fold(_items);
      // A row that started exporting while it was in hand has lost the
      // gesture that would have ended the drag, so its mark is taken down
      // here.
      final marked = _lines.where((l) => l.key == _markedKey).firstOrNull;
      if (_markedKey != null && (marked == null || _blockOf(marked).isEmpty)) {
        _markedKey = null;
        _markedFrom = 0;
        _markedTo = null;
      }
    });
  }

  /// The lines the list draws: an item where it stands, and each run of a
  /// template's rows under a heading where its first row stands, its rows
  /// after the heading while it is open.
  List<_Line> _fold(List<BridgeExportQueueItem> items) {
    final runs = <int, List<BridgeExportQueueItem>>{};
    for (final item in items) {
      if (item.group case final group?) (runs[group] ??= []).add(item);
    }
    _openRuns.retainAll(runs.keys);
    final lines = <_Line>[];
    for (final item in items) {
      final group = item.group;
      if (group == null) {
        lines.add(_Line(item: item));
        continue;
      }
      final run = runs.remove(group);
      if (run == null) continue;
      lines.add(_Line(group: group, run: run));
      if (_openRuns.contains(group)) {
        for (final row in run) {
          lines.add(_Line(item: row, inRun: true));
        }
      }
    }
    return lines;
  }

  /// The items a drag of [line] moves: a waiting item, or the waiting rows of
  /// a shut run. Empty for a line that cannot be picked up.
  List<int> _blockOf(_Line line) {
    if (line.item case final item?) {
      return item.state is BridgeExportQueueState_Waiting ? [item.id] : const [];
    }
    if (_openRuns.contains(line.group)) return const [];
    return [
      for (final item in line.run)
        if (item.state is BridgeExportQueueState_Waiting) item.id,
    ];
  }

  /// Move the line at [from] to the place [to] in the list without it, the
  /// index `onReorderItem` gives. Its items land before the first item of the
  /// line now at [to], or last. Each is moved on its own, in the order that
  /// keeps the block's own order.
  void _moveLines(int from, int to) {
    final lines = _lines;
    if (from < 0 || from >= lines.length) return;
    final block = _blockOf(lines[from]);
    if (block.isEmpty) return;
    final rest = [...lines]..removeAt(from);
    final order = [for (final item in _items) item.id];
    final after = [
      for (final id in order)
        if (!block.contains(id)) id,
    ];
    int? anchor;
    if (to < rest.length) {
      final line = rest[to];
      anchor = line.item?.id ??
          line.run.map((i) => i.id).where((id) => !block.contains(id)).firstOrNull;
    }
    var at = anchor == null ? after.length : after.indexOf(anchor);
    if (at < 0) at = after.length;
    after.insertAll(at, block);
    final down = after.indexOf(block.first) > order.indexOf(block.first);
    try {
      for (final id in down ? block.reversed : block) {
        (widget.move ?? exportQueueMove)(id: id, index: after.indexOf(id));
      }
    } catch (_) {
      // The queue turned over under the drag; re-reading is the recovery.
    }
    _refresh();
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
        // The list's ground is the queue's own right-click; a row's is its own.
        GestureDetector(
          behavior: HitTestBehavior.translucent,
          onSecondaryTapUp: (up) => _queueMenu(up.globalPosition),
          child: Padding(
          padding: exportBodyPaddingForQueue,
          child: _items.isEmpty
              ? SizedBox(
                  height: exportQueueRow,
                  child: Align(
                    alignment: AlignmentDirectional.centerStart,
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
                          itemCount: _lines.length,
                          itemBuilder: (context, index) =>
                              _row(t, _lines[index], index),
                          // `onReorderItem` gives the place the row lands in
                          // the list without it.
                          onReorderItem: _moveLines,
                          proxyDecorator: (child, index, animation) =>
                              _lifted(t, child),
                        )
                      else
                        SliverList.builder(
                          itemCount: _lines.length,
                          itemBuilder: (context, index) =>
                              _markedRow(t, _lines[index], index),
                        ),
                    ],
                  ),
                ),
        )),
        // The footer's count is the queue's too.
        GestureDetector(
          behavior: HitTestBehavior.translucent,
          onSecondaryTapUp: (up) => _queueMenu(up.globalPosition),
          child: dialogFooter(
          t,
          summary: _items.isEmpty
              ? ''
              : l10n.exportQueueSummary('$waiting', '$done'),
          keyPrefix: 'export-queue',
          actions: [
            HouseButton(
              key: const ValueKey('export-queue-add'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: () => showExportPickFrb(context: context),
              child: Text(l10n.exportQueueAddComps),
            ),
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
              // Nothing to do while the queue is already working through.
              onPressed: waiting == 0 || _goesOn ? null : _start,
              child: Text(l10n.exportQueueStart),
            ),
          ],
        )),
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
  Widget _row(LumitTheme t, _Line line, int index) {
    final row = _lineFace(t, line);
    return KeyedSubtree(
      key: ValueKey<String>('export-queue-row-${line.key}'),
      child: _blockOf(line).isNotEmpty
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
  Widget _markedRow(LumitTheme t, _Line entry, int index) {
    final row = _lineFace(t, entry);
    final from = _lines.indexWhere((l) => l.key == _markedKey);
    final to = _markedTo;
    final marked = from >= 0 && to == index && to != from;
    final below = marked && index > from;
    final line = BorderSide(color: t.accent, width: 2);
    return KeyedSubtree(
      key: ValueKey<String>('export-queue-row-${entry.key}'),
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
        child: _blockOf(entry).isNotEmpty
            ? GestureDetector(
                behavior: HitTestBehavior.opaque,
                // From where the pointer went down, not from where the drag
                // was recognised: the distance in between is part of how far
                // the row has been taken.
                dragStartBehavior: DragStartBehavior.down,
                onVerticalDragStart: (d) => setState(() {
                  _markedKey = entry.key;
                  _markedFrom = d.localPosition.dy;
                  _markedTo = index;
                }),
                onVerticalDragUpdate: (d) {
                  final at = _lines.indexWhere((l) => l.key == _markedKey);
                  if (at < 0) return;
                  // The place whose row the pointer is past the middle of.
                  final travel = d.localPosition.dy - _markedFrom;
                  final next = (at + (travel / exportQueueRow).round())
                      .clamp(0, _lines.length - 1);
                  if (next != _markedTo) setState(() => _markedTo = next);
                },
                onVerticalDragEnd: (_) {
                  final at = _lines.indexWhere((l) => l.key == _markedKey);
                  final next = _markedTo;
                  _endMarked();
                  if (at >= 0 && next != null && next != at) {
                    _moveLines(at, next);
                  }
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
      _markedKey = null;
      _markedFrom = 0;
      _markedTo = null;
    });
  }

  /// A line's face: an item's row, set in under its run's heading when it is
  /// one of a template's rows, or the heading of a run.
  Widget _lineFace(LumitTheme t, _Line line) {
    final item = line.item;
    if (item == null) return _runFace(t, line);
    final row = _opensFolder(item, _rowFace(t, item));
    return line.inRun
        ? Padding(
            padding: const EdgeInsetsDirectional.only(start: _runIndent),
            child: row)
        : row;
  }

  /// The heading of a run of a template's rows: the twirl, the template's
  /// name and how many rows, how many are done of how many, a bar across the
  /// run, and the failed rows in the error ink.
  Widget _runFace(LumitTheme t, _Line line) {
    final group = line.group!;
    final run = line.run;
    final open = _openRuns.contains(group);
    var done = 0;
    var failed = 0;
    var progress = 0.0;
    for (final item in run) {
      switch (item.state) {
        case BridgeExportQueueState_Done():
          done++;
          progress += 1;
        case BridgeExportQueueState_Failed():
          failed++;
        case BridgeExportQueueState_Running(:final frame, :final total)
            when total > 0:
          progress += frame / total;
        default:
      }
    }
    return GestureDetector(
      key: ValueKey<String>('export-queue-run-$group'),
      behavior: HitTestBehavior.opaque,
      onTap: () => setState(() {
        if (!_openRuns.remove(group)) _openRuns.add(group);
        _lines = _fold(_items);
      }),
      onSecondaryTapDown: (press) => _runMenu(line, press.globalPosition),
      child: HoverFill(
        child: SizedBox(
          height: exportQueueRow,
          child: Row(
            children: [
              SizedBox(
                width: _runIndent,
                child: Align(
                  alignment: AlignmentDirectional.centerStart,
                  child: TwirlTurn(
                    open: open,
                    child: glyph.LumitIcon(
                      open ? LumitIcons.collapse : LumitIcons.expand,
                      size: 12,
                      colour: t.textMuted,
                    ),
                  ),
                ),
              ),
              Expanded(
                child: FittedText(
                    l10n.exportQueueRun(run.first.groupName, run.length),
                    t.bodyPrimary),
              ),
              const SizedBox(width: 10),
              SizedBox(
                width: 260,
                child: Row(
                  key: ValueKey<String>('export-queue-run-status-$group'),
                  children: [
                    Text(l10n.exportQueueRunDone(done, run.length),
                        style: dialogMono(t)),
                    const SizedBox(width: 8),
                    Expanded(
                      child: HouseProgressBar(
                          fraction: run.isEmpty ? 0 : progress / run.length),
                    ),
                    if (failed > 0) ...[
                      const SizedBox(width: 8),
                      Text(l10n.exportQueueRunFailed(failed),
                          style: dialogMono(t).copyWith(color: t.error)),
                    ],
                  ],
                ),
              ),
              // Where an item's settings and remove marks stand.
              const SizedBox(width: 8 + 2 * (dialogCloseGlyph + 8)),
            ],
          ),
        ),
      ),
    );
  }

  /// A run's menu: stop after the row in hand, cancel what still waits,
  /// clear what has landed, queue the failed rows again, and the folder.
  void _runMenu(_Line line, Offset at) {
    final run = line.run;
    final running =
        run.any((item) => item.state is BridgeExportQueueState_Running);
    final waiting = [
      for (final item in run)
        if (item.state is BridgeExportQueueState_Waiting) item.id,
    ];
    final done = [
      for (final item in run)
        if (item.state is BridgeExportQueueState_Done) item,
    ];
    final failed = [
      for (final item in run)
        if (item.state is BridgeExportQueueState_Failed) item.id,
    ];
    final shown = done.lastOrNull?.path ?? run.first.path;
    unawaited(showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) {
        Widget row(String id, String label, VoidCallback? press) => MenuRow(
              key: ValueKey<String>('export-queue-run-menu-$id'),
              enabled: press != null,
              onPressed: () {
                close(null);
                press?.call();
                _refresh();
              },
              child: MenuLabel(label,
                  enabled: press != null, ticked: running ? false : null),
            );
        return [
          if (running) _stopAfterRow(() => close(null)),
          row(
              'cancel',
              l10n.exportQueueCancelWaiting,
              waiting.isEmpty
                  ? null
                  : () {
                      for (final id in waiting) {
                        exportQueueRemove(id: id);
                      }
                    }),
          row(
              'clear',
              l10n.exportQueueRemoveFinished,
              done.isEmpty
                  ? null
                  : () {
                      for (final item in done) {
                        exportQueueRemove(id: item.id);
                      }
                    }),
          row(
              'again',
              l10n.exportQueueAgainFailed,
              failed.isEmpty
                  ? null
                  : () {
                      for (final id in failed) {
                        if (exportQueueAgain(id: id) != null) {
                          exportQueueRemove(id: id);
                        }
                      }
                    }),
          if (!kIsWeb)
            row('folder', l10n.projectShowInFolder, () {
              // The last file written, else the folder the run writes into.
              if (revealInFolder(path: shown)) return;
              final cut = shown.lastIndexOf(RegExp(r'[/\\]'));
              if (cut > 0) revealInFolder(path: shown.substring(0, cut));
            }),
        ];
      },
    ));
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
      item.state is! BridgeExportQueueState_Done || kIsWeb
          ? row
          : LumitTooltip(
              message: l10n.projectShowInFolder,
              child: HoverFill(
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
  Widget _rowFace(LumitTheme t, BridgeExportQueueItem item) => GestureDetector(
      onSecondaryTapDown: (press) => _showMenu(t, item, press.globalPosition),
      // The row's marks come up with the pointer. A row that is doing nothing
      // only says what it is.
      child: HoverMark(
        cursor: MouseCursor.defer,
        builder: (context, lit) => SizedBox(
        height: exportQueueRow,
        child: Row(
          key: ValueKey<String>('export-queue-item-${item.id}'),
          children: [
            SizedBox(
              width: 150,
              child: FittedText(item.compName, t.body),
            ),
            const SizedBox(width: 10),
            // The preset it was queued on, or its format where it was set
            // by hand, and the format on hover.
            SizedBox(
              width: 120,
              child: LumitTooltip(
                message: exportFormatLabel(item.codec),
                child: FittedText(
                    item.preset.isEmpty
                        ? exportFormatLabel(item.codec)
                        : item.preset,
                    t.body.copyWith(color: t.textMuted)),
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: MiddleEllipsis(
                _leaf(item.path),
                style: dialogMono(t),
                tip: item.path,
              ),
            ),
            const SizedBox(width: 10),
            SizedBox(width: 260, child: _statusFace(t, item)),
            const SizedBox(width: 8),
            // Hidden marks take no clicks either.
            IgnorePointer(
              ignoring: lit <= 0,
              child: Opacity(
              opacity: lit > 0 ? 1 : 0,
              child: Row(mainAxisSize: MainAxisSize.min, children: [
            _settingsMark(t, item),
            LumitTooltip(
              message: item.state is BridgeExportQueueState_Running
                  ? l10n.exportCancel
                  : l10n.exportQueueRemove,
              child: HoverMark(
                builder: (context, lit) => GestureDetector(
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
                        colour: Color.lerp(t.textMuted, t.textPrimary, lit),
                        semanticLabel: l10n.exportQueueRemove,
                      ),
                    ),
                  ),
                ),
              ),
            ),
              ]),
              ),
            ),
          ],
        ),
      ),
      ),
    );

  /// A row's menu: what can be done with it as it stands, Remove last.
  void _showMenu(LumitTheme t, BridgeExportQueueItem item, Offset at) {
    final comp = _comps[item.comp.toString()];
    final running = item.state is BridgeExportQueueState_Running;
    final waiting = item.state is BridgeExportQueueState_Waiting;
    final done = item.state is BridgeExportQueueState_Done;
    final index = _items.indexOf(item);
    Widget row(String id, String label, VoidCallback? press,
            void Function(VoidCallback) run) =>
        MenuRow(
          key: ValueKey<String>('export-queue-menu-$id'),
          enabled: press != null,
          onPressed: () {
            if (press != null) run(press);
          },
          // In line with Stop after this one's tick, where it shows.
          child: MenuLabel(label,
              enabled: press != null, ticked: running ? false : null),
        );
    unawaited(showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) {
        void run(VoidCallback act) {
          close(null);
          act();
          _refresh();
        }

        return [
          if (running) _stopAfterRow(() => close(null)),
          if (item.state case BridgeExportQueueState_Running(:final paused))
            row(
                'pause',
                paused ? l10n.remoteGoOn : l10n.remotePause,
                () => exportQueuePause(id: item.id, paused: !paused),
                run),
          if (done && !kIsWeb)
            row('open', l10n.projectShowInFolder,
                () => revealInFolder(path: item.path), run),
          // A failed row too, where its settings are what has to change.
          if (waiting || item.state is BridgeExportQueueState_Failed)
            row(
                'settings',
                l10n.exportQueueSettingsEllipsis,
                comp == null
                    ? null
                    : () => showExportDialogFrb(
                        context: context, comp: comp, queued: item.id),
                run),
          // Asks the composition as it is now, so a comp since deleted cannot.
          // A template's row is queued again as it was, so it keeps its row.
          if (!running)
            row(
                'duplicate',
                l10n.menuDuplicate,
                item.group != null
                    ? () => _duplicateRow(item)
                    : comp == null
                        ? null
                        : () => _duplicate(comp, item),
                run),
          if (done || item.state is BridgeExportQueueState_Failed)
            row(
                'again',
                l10n.exportQueueAgain,
                item.group != null
                    ? () {
                        if (exportQueueAgain(id: item.id) != null) {
                          exportQueueRemove(id: item.id);
                        }
                      }
                    : comp == null
                        ? null
                        : () => _again(comp, item),
                run),
          if (waiting && _items.length > 1) ...[
            const MenuDivider(),
            row('top', l10n.moveToTop, index == 0 ? null : () => _move(index, 0),
                run),
            row(
                'bottom',
                l10n.moveToBottom,
                index == _items.length - 1
                    ? null
                    : () => _move(index, _items.length - 1),
                run),
          ],
          const MenuDivider(),
          row(
              'remove',
              running ? l10n.exportCancel : l10n.exportQueueRemove,
              () => running
                  ? exportQueueCancel(id: item.id)
                  : exportQueueRemove(id: item.id),
              run),
        ];
      },
    ));
  }

  /// Stop after this one, ticked while the queue will hold once the running
  /// export lands. Unticked, it runs on.
  Widget _stopAfterRow(VoidCallback close) => MenuRow(
        key: const ValueKey('export-queue-menu-stop-after'),
        onPressed: () {
          close();
          _goesOn ? exportQueueStopAfter() : exportQueueStart();
          _refresh();
        },
        child: MenuLabel(l10n.exportQueueStopAfter, ticked: !_goesOn),
      );

  /// The queue's own menu: run it, hold it after the export in flight, add
  /// to it, find the last file it wrote, and clear away what has landed.
  void _queueMenu(Offset at) {
    final running =
        _items.any((item) => item.state is BridgeExportQueueState_Running);
    final waiting =
        _items.any((item) => item.state is BridgeExportQueueState_Waiting);
    final done = [
      for (final item in _items)
        if (item.state is BridgeExportQueueState_Done) item.id,
    ];
    final last = _items
        .where((item) => item.state is BridgeExportQueueState_Done)
        .lastOrNull;
    unawaited(showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) {
        Widget row(String id, String label, VoidCallback? press) => MenuRow(
              key: ValueKey<String>('export-queue-menu-$id'),
              enabled: press != null,
              onPressed: () {
                close(null);
                press?.call();
              },
              child: MenuLabel(label,
                  enabled: press != null, ticked: running ? false : null),
            );
        return [
          row('start', l10n.exportQueueStart,
              waiting && !_goesOn ? _start : null),
          if (running) _stopAfterRow(() => close(null)),
          const MenuDivider(),
          row('add', l10n.exportQueueAddComps,
              () => showExportPickFrb(context: context)),
          if (!kIsWeb)
            row('folder', l10n.projectShowInFolder,
                last == null ? null : () => revealInFolder(path: last.path)),
          row(
              'clear',
              l10n.exportQueueRemoveFinished,
              done.isEmpty
                  ? null
                  : () {
                      for (final id in done) {
                        exportQueueRemove(id: id);
                      }
                      _refresh();
                    }),
        ];
      },
    ));
  }

  /// Another row with the same settings, writing to a file of its own.
  void _duplicate(CompositionReference comp, BridgeExportQueueItem item) {
    final spec = exportQueueSpec(id: item.id);
    if (spec == null) return;
    comp.queueExport(
        spec: spec,
        path: exportFreePath(
            path: item.path, codec: item.codec, captions: spec.captions),
        start: false);
  }

  /// A template's row queued again as it was, so it keeps its row, written
  /// to a file of its own beside the original.
  void _duplicateRow(BridgeExportQueueItem item) {
    final spec = exportQueueSpec(id: item.id);
    final copy = exportQueueAgain(id: item.id);
    if (spec == null || copy == null) return;
    exportQueueUpdate(
        id: copy,
        spec: spec,
        path: exportFreePath(
            path: item.path, codec: item.codec, captions: spec.captions));
  }

  /// A finished or failed row, queued again over its own file.
  void _again(CompositionReference comp, BridgeExportQueueItem item) {
    final spec = exportQueueSpec(id: item.id);
    if (spec == null) return;
    comp.queueExport(spec: spec, path: item.path, start: false);
    exportQueueRemove(id: item.id);
  }

  /// The mark that opens a waiting item's settings in the export dialogue.
  /// Kept as an empty space on every other row, so the columns stay put.
  Widget _settingsMark(LumitTheme t, BridgeExportQueueItem item) {
    final comp = _comps[item.comp.toString()];
    const size = Size(dialogCloseGlyph + 8, exportQueueRow);
    final open = item.state is BridgeExportQueueState_Waiting ||
        item.state is BridgeExportQueueState_Failed;
    if (!open || comp == null) return SizedBox.fromSize(size: size);
    return LumitTooltip(
      message: l10n.exportQueueSettings,
      child: HoverMark(
        builder: (context, lit) => GestureDetector(
          key: ValueKey<String>('export-queue-settings-${item.id}'),
          behavior: HitTestBehavior.opaque,
          onTap: () => showExportDialogFrb(
              context: context, comp: comp, queued: item.id),
          child: SizedBox.fromSize(
            size: size,
            child: Center(
              child: glyph.LumitIcon(
                LumitIcons.settings,
                size: dialogCloseGlyph,
                colour: Color.lerp(t.textMuted, t.textPrimary, lit),
                semanticLabel: l10n.exportQueueSettings,
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// How one item is getting on. A running one is a bar with how far it has
  /// got and how long it has left, its encoder on hover. An error is longer
  /// than the column, so a failed item shows all of it on hover.
  Widget _statusFace(LumitTheme t, BridgeExportQueueItem item) {
    final key = ValueKey<String>('export-queue-status-${item.id}');
    if (item.state case BridgeExportQueueState_Running(paused: true)) {
      return Text(l10n.remotePaused,
          key: key,
          style: dialogMono(t).copyWith(color: t.textMuted),
          textAlign: TextAlign.right);
    }
    if (item.state
        case BridgeExportQueueState_Running(
          :final frame,
          :final total,
          :final encoder,
          :final paused
        ) when total > 0) {
      final seen = _seenRunning[item.id] ??= (DateTime.now(), frame);
      // A held export has no time left to tell.
      final left = paused
          ? null
          : exportTimeLeft(
              done: frame - seen.$2,
              left: total - frame,
              taken: DateTime.now().difference(seen.$1));
      return LumitTooltip(
        message: encoder,
        child: Row(
          key: key,
          children: [
            Expanded(child: HouseProgressBar(fraction: frame / total)),
            const SizedBox(width: 8),
            Text(l10n.exportQueuePercent((frame * 100 / total).floor()),
                style: dialogMono(t)),
            if (left != null) ...[
              const SizedBox(width: 8),
              Flexible(
                child: Text(left,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: t.small.copyWith(color: t.textMuted)),
              ),
            ],
          ],
        ),
      );
    }
    final failed = item.state is BridgeExportQueueState_Failed;
    final text = Text(
      _status(item),
      key: key,
      style: dialogMono(t).copyWith(color: failed ? t.warning : t.textMuted),
      overflow: TextOverflow.ellipsis,
      textAlign: TextAlign.right,
    );
    return failed ? LumitTooltip(message: _status(item), child: text) : text;
  }

  /// What one item is doing, in words, where it is not drawn as a bar.
  String _status(BridgeExportQueueItem item) => switch (item.state) {
        BridgeExportQueueState_Waiting() => l10n.exportQueueWaiting,
        BridgeExportQueueState_Running() => l10n.exportQueuePreparing,
        // How long it took and how big the file is, where those are known.
        BridgeExportQueueState_Done(:final seconds, :final bytes) => [
            l10n.exportQueueDone,
            if (seconds > 0) clockTime(seconds),
            if (bytes > 0) fileSize(bytes),
          ].join(' · '),
        BridgeExportQueueState_Failed(:final error) => error,
      };

  static String _leaf(String path) => path.split(RegExp(r'[/\\]')).last;
}

/// [seconds] as a clock reads it: `2:31`, or `1:02:31` past the hour.
String clockTime(int seconds) {
  String two(int n) => n.toString().padLeft(2, '0');
  final h = seconds ~/ 3600;
  final m = seconds ~/ 60 % 60;
  final s = two(seconds % 60);
  return h > 0 ? '$h:${two(m)}:$s' : '$m:$s';
}

/// A file's size, in kilobytes under one megabyte.
String fileSize(int bytes) => bytes < 1 << 20
    ? '${(bytes / 1024).ceil()} ${l10n.unitKb}'
    : byteSize(bytes);

/// The list's own inset — the export dialog's body padding, which is the
/// pattern's (§12A.4).
const EdgeInsets exportBodyPaddingForQueue =
    EdgeInsets.fromLTRB(14, 10, 14, 12);

/// How long a running export has left, in words for the strip, or null while
/// there is too little to go on.
///
/// It is frames done over time taken since the strip first saw the export, so
/// it follows the average speed and one slow frame does not move it. Nothing
/// is said for the first five seconds, when that average is mostly noise.
String? exportTimeLeft(
    {required int done, required int left, required Duration taken}) {
  if (done <= 0 || left <= 0 || taken < const Duration(seconds: 5)) return null;
  final minutes = (taken.inMilliseconds / 60000 * left / done).round();
  if (minutes < 1) return l10n.exportLeftUnderMinute;
  return minutes < 120
      ? l10n.exportLeftMinutes('$minutes')
      : l10n.exportLeftHours('${(minutes / 60).round()}');
}
