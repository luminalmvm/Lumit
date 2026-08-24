// The recovery dialogue, on the flutter_rust_bridge API.
//
// Offered when a project is opened and the engine finds work beside it that the
// saved file does not contain: rotating autosaves, and a crash journal of edits
// made since the last save.
//
// **The two are not the same thing and the wording matters.** An autosave is a
// whole document written on a timer — opening one loses whatever happened after
// it. The journal is the edits themselves, replayed onto the saved file, so it
// recovers everything up to the moment things stopped. The journal is therefore
// the choice the dialogue opens on.
//
// **The shape is the dialog pattern's** (K-444, K-469): the frame, the kicker
// title strip with the project's own name beside it, label-left rows, and a
// footer carrying the factual line, the outlined way out and the single filled
// action. `dialog_frame.dart` holds the pieces; this file holds the question.
// There is no drawing of its own for this dialogue, so it wears the one the
// Export and New composition drawings share rather than inventing a third.
//
// **What can be recovered is a choice, and what to do about it is an action.**
// The two sources sit in the body as rows to pick between; the footer carries
// what happens next — *Open the saved file as it is*, which touches nothing,
// and *Recover*, which applies whichever source is picked. Escape and the close
// mark do neither, and the project opens as it was saved.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// What the user chose.
enum RecoveryChoice { journal, autosave, discard }

/// The frame this dialogue takes, and its row — a 160px name column with 12
/// after it, in rows of 30. Its own numbers, not the Export dialog's (K-458:
/// each dialogue measures what it holds).
const double recoveryDialogWidth = 520;
const double recoveryLabelColumn = 160;
const double recoveryRowGap = 12;
const double recoveryRowHeight = 30;

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
    builder: (close) => _RecoveryDialog(
      autosaves: autosaves,
      subject: _leaf(projectPath),
      onChoose: close,
    ),
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

class _RecoveryDialog extends StatefulWidget {
  final List<BridgeAutosave> autosaves;
  final String subject;
  final ValueChanged<RecoveryChoice?> onChoose;

  const _RecoveryDialog({
    required this.autosaves,
    required this.subject,
    required this.onChoose,
  });

  @override
  State<_RecoveryDialog> createState() => _RecoveryDialogState();
}

class _RecoveryDialogState extends State<_RecoveryDialog> {
  /// The journal opens picked: it is the choice that loses nothing, and a
  /// dialogue that opens on nothing makes the user answer a question twice.
  RecoveryChoice _source = RecoveryChoice.journal;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: recoveryDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.recoverTitle,
          subject: widget.subject,
          onClose: () => widget.onChoose(null),
          keyPrefix: 'recover',
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(
              dialogPadding, dialogPadding, dialogPadding, 12),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(l10n.recoverBlurb, style: t.body),
              const SizedBox(height: 10),
              _option(
                t,
                key: 'recover-journal',
                choice: RecoveryChoice.journal,
                title: l10n.recoverJournal,
                body: l10n.recoverJournalHelp,
              ),
              _option(
                t,
                key: 'recover-autosave',
                choice: RecoveryChoice.autosave,
                title: l10n.recoverAutosave,
                body: l10n.recoverAutosaveHelp,
                // Which copy "the newest" is, said in the row that offers it
                // rather than in a line of its own: a file name is a fact
                // about this choice, not about the dialogue.
                fact: _leaf(widget.autosaves.first.path),
              ),
            ],
          ),
        ),
        dialogFooter(
          t,
          summary: l10n.recoverFound(widget.autosaves.length),
          keyPrefix: 'recover',
          actions: [
            LumitTooltip(
              message: l10n.recoverDiscardHelp,
              child: HouseButton(
                key: const ValueKey('recover-discard'),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                onPressed: () => widget.onChoose(RecoveryChoice.discard),
                child: Text(l10n.recoverDiscard),
              ),
            ),
            HouseButton(
              key: const ValueKey('recover-apply'),
              // The window's default action (K-319): focused on open, so Enter
              // recovers whichever source is picked above.
              primary: true,
              autofocus: true,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              onPressed: () => widget.onChoose(_source),
              child: Text(l10n.recoverAction),
            ),
          ],
        ),
      ],
    );
  }

  /// One source to recover from: its name in the row's name column, what it
  /// means beside it, the picked one filled the way a chosen row is.
  ///
  /// [fact] is a mono line under the sentence for a row that has something to
  /// report — the newest autosave's file name — never for help, which is what
  /// the sentence already is (§12A.4).
  Widget _option(
    LumitTheme t, {
    required String key,
    required RecoveryChoice choice,
    required String title,
    required String body,
    String fact = '',
  }) =>
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 1),
        child: MenuRow(
          key: ValueKey<String>(key),
          selected: _source == choice,
          onPressed: () => setState(() => _source = choice),
          child: dialogRow(
            t,
            title,
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(body, style: t.small.copyWith(color: t.textMuted)),
                if (fact.isNotEmpty)
                  Text(fact,
                      style: dialogMono(t), overflow: TextOverflow.ellipsis),
              ],
            ),
            labelColumn: recoveryLabelColumn,
            gap: recoveryRowGap,
            minHeight: recoveryRowHeight,
          ),
        ),
      );
}

String _leaf(String path) => path.split(RegExp(r'[/\\]')).last;
