// The recovery dialogue, on the flutter_rust_bridge API.
//
// Offered when a project is opened and the engine finds work beside it that the
// saved file does not contain: rotating autosaves, and a crash journal of edits
// made since the last save.
//
// **The three buttons are the choice**. The dialogue used to ask twice:
// pick a source in the body, then press *Recover* in the footer. It asks once
// now — one sentence, and three ways to answer it along the bottom, in the
// order they lose the most first: *Don't restore changes* opens the saved file
// as it is, *Restore latest autosave* opens the last timed copy, and *Restore
// all changes* replays the journal onto the saved file. The last is the filled
// action and the focused one, because it is the answer that loses nothing.
//
// **An autosave and the journal are not the same thing.** An autosave is a
// whole document written on a timer — opening one loses whatever happened after
// it. The journal is the edits themselves, replayed onto the saved file, so it
// recovers everything up to the moment things stopped. That is the whole reason
// *Restore all changes* is the default, and the tooltips carry the difference
// for anyone who wants it in words.
//
// **The frame is the dialog pattern's** at a narrow width: the
// title strip, the sentence, the footer. There are no label-left rows here at
// all any more, so this dialogue has no row measurements of its own — only its
// width, and the footer's actions **stack** rather than sit in a line, which is
// §12A.6's ladder step 2 and the only way three of the owner's phrases fit a
// narrow window. `dialog_frame.dart` holds the pieces; this file holds the
// question.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// What the user chose.
enum RecoveryChoice { journal, autosave, discard }

/// The frame this dialogue takes.
///
/// **Measured, not chosen**. The row was tried first, as the owner
/// asked. In Hanken Grotesk at 11 the three labels are 108.5, 116.5 and — the
/// filled one being a 9px mono kicker tracked 0.12em (§12A.4) — 123.1, which
/// with 12 either side of the two outlined buttons and 16 either side of the
/// filled one comes to 428.1 of button. Add the footer's 14 insets and the 12
/// before each button and one line needs **492**: 95% of the 520 this dialogue
/// already was, which is not a narrow window by any reading.
///
/// So the actions stack (§12A.6 step 2 — a run that will not fit drops to
/// another line rather than eliding its words), and the width falls to the
/// owner's own ≈350. At 350 the body has 322: the sentence sets in two lines,
/// and each full-width button has more than twice the room its label needs, so
/// no translation of these three phrases can clip.
const double recoveryDialogWidth = 350;

/// Offer recovery for `projectPath`. Returns null when there is nothing to
/// recover, so the caller can open the project normally without a dialogue
/// nobody needed.
Future<RecoveryChoice?> showRecoveryDialogFrb({
  required BuildContext context,
  required LumitState state,
  required String projectPath,
}) async {
  final autosaves = listAutosaves(project: projectPath);
  if (autosaves.isEmpty) return null;

  final choice = await showLumitModal<RecoveryChoice>(
    context: context,
    id: 'recovery',
    builder: (close) => _RecoveryDialog(onChoose: close),
  );
  if (choice == null) return null;

  switch (choice) {
    case RecoveryChoice.journal:
      state.project?.restoreJournal(projectPath: projectPath);
    case RecoveryChoice.autosave:
      await state.openProject(autosaves.first.path);
    case RecoveryChoice.discard:
      break;
  }
  state.notifyDocumentChanged();
  return choice;
}

class _RecoveryDialog extends StatelessWidget {
  final ValueChanged<RecoveryChoice?> onChoose;

  const _RecoveryDialog({required this.onChoose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: recoveryDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.recoveryTitle,
          onClose: () => onChoose(null),
          keyPrefix: 'recover',
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(
              dialogPadding, dialogPadding, dialogPadding, dialogPadding),
          child: Text(l10n.recoveryQuestion, style: t.body),
        ),
        dialogFooter(
          t,
          keyPrefix: 'recover',
          stacked: true,
          actions: [
            _button(t, 'recover-discard', l10n.recoveryRestoreNone,
                l10n.tipRecoverNone, RecoveryChoice.discard),
            _button(t, 'recover-autosave', l10n.recoveryRestoreAutosave,
                l10n.tipRecoverAutosave, RecoveryChoice.autosave),
            // The window's default action: focused on open, so Enter
            // restores everything — the answer that loses nothing.
            _button(t, 'recover-journal', l10n.recoveryRestoreAll,
                l10n.tipRecoverAll, RecoveryChoice.journal,
                primary: true),
          ],
        ),
      ],
    );
  }

  /// One way to answer, with what it actually does on hover: the labels say
  /// what happens, the tooltips say what it costs.
  Widget _button(
    LumitTheme t,
    String key,
    String label,
    String tip,
    RecoveryChoice choice, {
    bool primary = false,
  }) =>
      LumitTooltip(
        message: tip,
        child: HouseButton(
          key: ValueKey<String>(key),
          primary: primary,
          autofocus: primary,
          padding: EdgeInsets.symmetric(horizontal: primary ? 16 : 12),
          onPressed: () => onChoose(choice),
          child: Text(label),
        ),
      );
}
