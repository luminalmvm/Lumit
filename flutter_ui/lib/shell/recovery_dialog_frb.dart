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
// offered first and worded as the ordinary choice.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

/// What the user chose.
enum RecoveryChoice { journal, autosave, discard }

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
    builder: (close) => _RecoveryDialog(
      autosaves: autosaves,
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

class _RecoveryDialog extends StatelessWidget {
  final List<BridgeAutosave> autosaves;
  final ValueChanged<RecoveryChoice?> onChoose;

  const _RecoveryDialog({required this.autosaves, required this.onChoose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 420,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.all(10),
            child: Text(l10n.recoverTitle, style: t.bodyPrimary),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Text(
              l10n.recoverBlurb,
              style: t.small,
            ),
          ),
          const SizedBox(height: 10),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Text(
              '${autosaves.length} autosave'
              '${autosaves.length == 1 ? '' : 's'}, newest: '
              '${_leaf(autosaves.first.path)}',
              style: t.small.copyWith(color: t.textMuted),
            ),
          ),
          const SizedBox(height: 12),
          _choice(
            t,
            key: 'recover-journal',
            title: l10n.recoverJournal,
            body: l10n.recoverJournalHelp,
            onPressed: () => onChoose(RecoveryChoice.journal),
          ),
          _choice(
            t,
            key: 'recover-autosave',
            title: l10n.recoverAutosave,
            body: l10n.recoverAutosaveHelp,
            onPressed: () => onChoose(RecoveryChoice.autosave),
          ),
          _choice(
            t,
            key: 'recover-discard',
            title: l10n.recoverDiscard,
            body: l10n.recoverDiscardHelp,
            onPressed: () => onChoose(RecoveryChoice.discard),
          ),
          const SizedBox(height: 8),
        ],
      ),
    );
  }

  Widget _choice(
    LumitTheme t, {
    required String key,
    required String title,
    required String body,
    required VoidCallback onPressed,
  }) =>
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
        child: MenuRow(
          key: ValueKey<String>(key),
          onPressed: onPressed,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: t.bodyPrimary),
              Text(body, style: t.small.copyWith(color: t.textMuted)),
            ],
          ),
        ),
      );

  static String _leaf(String path) => path.split(RegExp(r'[/\\]')).last;
}
