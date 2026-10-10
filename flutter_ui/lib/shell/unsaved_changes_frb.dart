// The question asked before a project with unsaved changes is closed, replaced
// or quit out of: save it, discard the changes, or stay where you are.
//
// LumitState asks it (`askBeforeLeaving`), so New, Open, Close, an import and
// quitting all get the same question from one place.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'menu_bar_frb.dart' show saveProjectFrb;
import 'recovery_dialog_frb.dart';
import 'tour_frb.dart' show onlyTourDemo;

/// Narrow, like the recovery dialogue: one sentence and three short answers.
const double unsavedDialogWidth = 350;

/// Give LumitState somewhere to ask. Called once, from the widget that is up
/// for the whole life of the window.
void installUnsavedQuestion(BuildContext context) {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  app.askUnsaved = () async =>
      onlyTourDemo(app) || await askUnsavedChangesFrb(context, app, ui);
  // The same window is where a crash's edits are offered back.
  app.offerRecovery = (path) => showRecoveryDialogFrb(
      context: context, state: app, projectPath: path, crashed: true);
}

/// Ask, and answer whether the project may go.
///
/// Save runs the ordinary save, which asks for a location when the project has
/// never had one. If that is cancelled, or the write fails, the project still
/// has unsaved changes and the answer is no: nothing is lost and nothing
/// closes. Cancel, Escape and a click outside are all no.
///
/// [savePicker] is the seam `saveProjectFrb` takes, for a test.
Future<bool> askUnsavedChangesFrb(
  BuildContext context,
  LumitState app,
  LumitUiState ui, {
  Future<String?> Function()? savePicker,
}) async {
  // No window left to ask in, so there is nobody to answer.
  if (!context.mounted) return true;
  final save = await showLumitModal<bool>(
    context: context,
    builder: (close) => _UnsavedDialog(onChoose: close),
  );
  if (save == null) return false;
  if (!save) {
    // A guest's copy whose host is away keeps only what its file holds.
    try {
      app.project?.shareDiscardAway();
    } catch (_) {
      // Closed already.
    }
    return true;
  }
  await saveProjectFrb(app, ui, picker: savePicker);
  try {
    return !(app.project?.isDirty() ?? false);
  } catch (_) {
    return false;
  }
}

class _UnsavedDialog extends StatelessWidget {
  /// True to save, false to discard, null to stay.
  final ValueChanged<bool?> onChoose;

  const _UnsavedDialog({required this.onChoose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: unsavedDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.unsavedChanges,
          onClose: () => onChoose(null),
          keyPrefix: 'unsaved',
        ),
        Padding(
          padding: const EdgeInsets.all(dialogPadding),
          child: Text(l10n.unsavedQuestion, style: t.body),
        ),
        dialogFooter(
          t,
          keyPrefix: 'unsaved',
          actions: [
            HouseButton(
              key: const ValueKey('unsaved-cancel'),
              onPressed: () => onChoose(null),
              child: Text(l10n.cancel),
            ),
            HouseButton(
              key: const ValueKey('unsaved-discard'),
              onPressed: () => onChoose(false),
              child: Text(l10n.discard),
            ),
            // The default: focused on open, so Enter saves.
            HouseButton(
              key: const ValueKey('unsaved-save'),
              primary: true,
              autofocus: true,
              onPressed: () => onChoose(true),
              child: Text(l10n.save),
            ),
          ],
        ),
      ],
    );
  }
}
