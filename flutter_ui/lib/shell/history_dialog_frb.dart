// The History window (K-688), on the flutter_rust_bridge API.
//
// Undo and redo have always walked a list nobody could see. This shows it: one
// row per edit, oldest at the top, and clicking a row takes the project to how
// it stood after that edit. The rows past it go grey rather than disappearing —
// they are exactly what redo would put back — and they only go when a fresh
// edit clears the forward history, which is what redo has always done.
//
// **The engine names the rows and the engine does the walking.** A row's phrase
// is `Op::name` on the far side, translated on arrival like every other engine
// word (K-303, `engine_labels.dart`); a jump is `jump_history`, which presses
// the ordinary undo or redo in a loop, so nothing here can reach a state the
// keyboard could not. This file holds the list and nothing else — no idea of
// what an op is, no arithmetic about what a jump would do.
//
// **The list is read when it opens and after each jump, never in a build.**
// Rebuilds are hot (docs/13, K-681): the rows live in state and are refreshed
// on purpose, so opening a menu over the window costs no bridge call.
//
// The frame is the dialog pattern's (K-444): title strip, body, footer.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// The window's width and the height of its list.
///
/// Wide enough for the longest phrase the engine sends ("Set footage colour
/// space" at 11 sets to 176) with room for a translation to run half as long
/// again, and tall enough for a dozen rows — which is more than a Ctrl-Z run
/// anybody counts. The window is resizable from its corner and remembers where
/// it was left (K-242), so a long session can make it as tall as the screen.
const double historyDialogWidth = 320;
const double historyListHeight = 320;

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

  @override
  void initState() {
    super.initState();
    // Straight into the fields rather than through `setState`: the first build
    // has not happened yet, so there is nothing to schedule.
    _read();
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
    widget.app.project?.jumpHistory(applied: applied);
    widget.app.notifyDocumentChanged();
    setState(_read);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: historyDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.menuHistory,
          onClose: widget.onClose,
          keyPrefix: 'history',
        ),
        Flexible(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: historyListHeight),
            child: ListView.builder(
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
            ),
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'history',
          actions: [
            HouseButton(
              key: const ValueKey('history-close'),
              small: true,
              primary: true,
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
      MenuRow(
        key: ValueKey<String>('history-row-$id'),
        selected: selected,
        onPressed: () => _jump(applied),
        child: Text(
          label,
          overflow: TextOverflow.ellipsis,
          style: undone ? t.body.copyWith(color: t.textMuted) : t.body,
        ),
      );
}
