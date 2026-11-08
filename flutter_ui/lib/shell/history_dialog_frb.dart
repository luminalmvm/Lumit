// The History window, on the flutter_rust_bridge API.
//
// Undo and redo have always walked a list nobody could see. This shows it: one
// row per edit, oldest at the top, and clicking a row takes the project to how
// it stood after that edit. The rows past it go grey rather than disappearing —
// they are exactly what redo would put back — and they only go when a fresh
// edit clears the forward history, which is what redo has always done.
//
// **The engine names the rows and the engine does the walking.** A row's phrase
// is `Op::name` on the far side, translated on arrival like every other engine
// word (`engine_labels.dart`); a jump is `jump_history`, which presses
// the ordinary undo or redo in a loop, so nothing here can reach a state the
// keyboard could not. This file holds the list and nothing else — no idea of
// what an op is, no arithmetic about what a jump would do.
//
// **The list is read when it opens and after each jump, never in a build.**
// Rebuilds are hot (docs/13): the rows live in state and are refreshed
// on purpose, so opening a menu over the window costs no bridge call.
//
// The frame is the dialog pattern's: title strip, body, footer.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'settings_account.dart' show askBefore;

/// The window's width and the height of its list.
///
/// Wide enough for the longest phrase the engine sends ("Set footage colour
/// space" at 11 sets to 176) with room for a translation to run half as long
/// again, and tall enough for a dozen rows — which is more than a Ctrl-Z run
/// anybody counts. The window is resizable from its corner and remembers where
/// it was left, so a long session can make it as tall as the screen.
const double historyDialogWidth = 320;
const double historyListHeight = 320;
const double _rowExtent = 24;

/// Open the History window.
Future<void> showHistoryFrb(BuildContext context, LumitState app) =>
    showLumitModal<void>(
      context: context,
      id: 'history',
      initialSize: const Size(historyDialogWidth, historyListHeight + 80),
      minSize: const Size(historyDialogWidth, 200),
      builder: (close) => _HistoryDialog(app: app, onClose: () => close(null)),
    );

class _HistoryDialog extends StatefulWidget {
  final LumitState app;
  final VoidCallback onClose;

  const _HistoryDialog({required this.app, required this.onClose});

  @override
  State<_HistoryDialog> createState() => _HistoryDialogState();
}

class _HistoryDialogState extends State<_HistoryDialog> {
  List<BridgeHistoryEntry> _rows = const [];

  /// How many rows are applied — where the document stands on the list. Row
  /// `i` of [_rows] is applied when `i < _applied`.
  int _applied = 0;

  late final ScrollController _scroll;

  @override
  void initState() {
    super.initState();
    // Straight into the fields rather than through `setState`: the first build
    // has not happened yet, so there is nothing to schedule.
    _read();
    // It opens where the document stands, not at the far end of a long list.
    _scroll = ScrollController(
        initialScrollOffset: (_applied - 6).clamp(0, 1 << 20) * _rowExtent);
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  void _read() {
    final project = widget.app.project;
    _rows = project?.historyEntries() ?? const [];
    _applied = project?.appliedSteps() ?? 0;
  }

  /// Take the document to the point where `applied` steps have been applied,
  /// then re-read: the jump is several undos, and the list has to say so.
  void _jump(int applied) {
    if (applied == _applied) return;
    widget.app.settleHistory();
    widget.app.project?.jumpHistory(applied: applied);
    widget.app.notifyDocumentChanged();
    setState(_read);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // No width of its own: the window around it has the size, and the corner
    // grip would lose to one set here.
    return DialogFrame(
      children: [
        dialogTitleBar(
          t,
          title: l10n.historyTitle,
          onClose: widget.onClose,
          keyPrefix: 'history',
        ),
        Expanded(
          // The ground under the rows takes the menu without a step to go to.
          child: GestureDetector(
            behavior: HitTestBehavior.translucent,
            onSecondaryTapUp: (up) => _menu(null, up.globalPosition),
            child: ListView.builder(
            controller: _scroll,
            itemExtent: _rowExtent,
            // One more than the steps: the row above them all is the state
            // the list begins at, so the window can walk back to where it
            // starts rather than leaving that one point to the keyboard.
            itemCount: _rows.length + 1,
            itemBuilder: (context, index) => index == 0
                // Never quiet: the state the list begins at is behind the
                // document wherever the document stands, so it is never one
                // of the steps waiting to be redone.
                ? _row(t,
                    id: 'origin',
                    label: l10n.historyOrigin,
                    undone: false,
                    selected: _applied == 0,
                    applied: 0)
                : _row(t,
                    id: '$index',
                    label: engineLabel(_rows[index - 1].name),
                    undone: _rows[index - 1].undone,
                    selected: _applied == index,
                    applied: index),
          )),
        ),
        dialogFooter(
          t,
          keyPrefix: 'history',
          actions: [
            HouseButton(
              key: const ValueKey('history-done'),
              onPressed: widget.onClose,
              child: Text(l10n.close),
            ),
          ],
        ),
      ],
    );
  }

  /// One row: its phrase, quiet if the step has been undone, and the place the
  /// document stands marked as the selected one.
  Widget _row(
    LumitTheme t, {
    required String id,
    required String label,
    required bool undone,
    required bool selected,
    required int applied,
  }) =>
      GestureDetector(
        onSecondaryTapDown: (d) => _menu(applied, d.globalPosition),
        child: MenuRow(
          key: ValueKey<String>('history-row-$id'),
          selected: selected,
          onPressed: () => _jump(applied),
          child: Text(
            label,
            overflow: TextOverflow.ellipsis,
            style: undone ? t.body.copyWith(color: t.textMuted) : t.body,
          ),
        ),
      );

  /// A row's right-click: go to it, a step either way from where the
  /// document stands, or clear the list.
  void _menu(int? applied, Offset at) {
    showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) {
        Widget row(String id, String label, int? to, {String? action}) =>
            MenuRow(
              key: ValueKey<String>('history-menu-$id'),
              enabled: to != null,
              onPressed: () {
                close(null);
                if (to != null) _jump(to);
              },
              child: MenuLabel(label, action: action, enabled: to != null),
            );
        return [
          if (applied != null) ...[
            row('go', l10n.historyGoToStep,
                applied == _applied ? null : applied),
            const MenuDivider(),
          ],
          row('undo', l10n.menuUndo, _applied > 0 ? _applied - 1 : null,
              action: 'edit.undo'),
          row('redo', l10n.menuRedo,
              _applied < _rows.length ? _applied + 1 : null,
              action: 'edit.redo'),
          const MenuDivider(),
          MenuRow(
            key: const ValueKey('history-menu-clear'),
            enabled: _rows.isNotEmpty,
            onPressed: () {
              close(null);
              _clear();
            },
            child: MenuLabel(l10n.historyClearEllipsis,
                enabled: _rows.isNotEmpty),
          ),
        ];
      },
    );
  }

  /// Drop every step, once asked: the project stays as it is, and the list
  /// starts again from here.
  Future<void> _clear() async {
    if (!await askBefore(context, l10n.historyClearAsk, l10n.clear)) return;
    if (!mounted) return;
    widget.app.settleHistory();
    widget.app.project?.clearHistory();
    setState(_read);
  }
}
