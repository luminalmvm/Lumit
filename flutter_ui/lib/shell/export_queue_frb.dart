// The export queue window: what is waiting, what is writing, what landed.
//
// **In plain terms.** Only one export can run at a time, so the rest wait in a
// list — and this is that list. Each row is one item: the composition it was
// made from, where it writes, and how far it has got. An item was photographed
// when it was added (docs/06 §7.1), so a row goes on writing what you queued
// however much the project changes afterwards.
//
// It is built to the K-444 dialog pattern like every other popup — the same
// title strip, rows and footer — because the approved mockups draw the export
// *dialog* and leave the queue to the pattern.
//
// **Nothing here decides anything.** The engine holds the queue and turns it
// over; this window asks what is in it a few times a second and draws the
// answer. The one bridge call in a rebuild path would be that ask, so it is not
// in one: the timer reads, and `build` draws what was read.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';

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
}) =>
    showLumitModal<void>(
      context: context,
      id: 'export-queue',
      builder: (close) => _ExportQueue(
        list: list,
        onClose: () => close(null),
      ),
    );

class _ExportQueue extends StatefulWidget {
  /// The read seam, injected by tests so no engine has to hold a queue.
  final List<BridgeExportQueueItem> Function()? list;
  final VoidCallback onClose;

  const _ExportQueue({required this.list, required this.onClose});

  @override
  State<_ExportQueue> createState() => _ExportQueueState();
}

class _ExportQueueState extends State<_ExportQueue> {
  List<BridgeExportQueueItem> _items = const [];
  Timer? _poll;

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
    final t = ThemeScope.of(context).theme;
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
                  child: SingleChildScrollView(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [for (final item in _items) _row(t, item)],
                    ),
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

  /// One item: what it is, where it goes, how it is getting on, and the mark
  /// that takes it off the list.
  Widget _row(LumitTheme t, BridgeExportQueueItem item) => SizedBox(
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
            Flexible(
              flex: 2,
              child: Text(
                _status(item),
                key: ValueKey<String>('export-queue-status-${item.id}'),
                style: dialogMono(t).copyWith(
                    color: item.state is BridgeExportQueueState_Failed
                        ? t.warning
                        : t.textMuted),
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.right,
              ),
            ),
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
