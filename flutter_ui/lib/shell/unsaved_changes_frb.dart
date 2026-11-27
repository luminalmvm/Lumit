// The question asked before a project with unsaved changes is closed, replaced
// or quit out of: save it, discard the changes, or stay where you are.
//
// LumitState asks it (`askBeforeLeaving`), so New, Open, Close, an import and
// quitting all get the same question from one place.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/remote.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/background.dart'
    show applyBackground, closeWindow, showWindow;
import '../state/remote_exports_frb.dart' show remoteEnded;
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'menu_bar_frb.dart' show saveProjectFrb;
import 'question_window.dart';
import 'recovery_dialog_frb.dart';
import 'settings_account.dart' show askBefore;
import 'tour_frb.dart' show onlyTourDemo;

const double unsavedDialogWidth = 350;

/// Give LumitState somewhere to ask. Called once, from the widget that is up
/// for the whole life of the window.
void installUnsavedQuestion(BuildContext context) {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  app.askUnsaved = () async =>
      onlyTourDemo(app) || await askUnsavedChangesFrb(context, app, ui);
  // Quitting stops an export, so closing in the middle of one asks first.
  // Quit in the tray can come with no window up, so it is shown first.
  app.askQuitExporting = () async {
    // Another computer is waiting on an export done here, so staying in the
    // tray is offered first, and it is the only question.
    final job = remoteJobs()
        .where((j) => !j.sent && !remoteEnded(j.stage))
        .firstOrNull;
    if (job != null) {
      if (ui.workspace.keepRunning) await showWindow();
      if (!context.mounted) return true;
      final keep = await showLumitModal<bool>(
        context: context,
        builder: (close) => QuestionWindow(
          title: l10n.remoteNewsArrived(job.compName, job.otherName),
          keyPrefix: 'quit-remote',
          onClose: () => close(null),
          actions: [
            questionPlain('quit-remote-quit', l10n.exportQuit,
                () => close(false)),
            questionFilled('quit-remote-keep', l10n.remoteKeepRunning,
                () => close(true)),
          ],
        ),
      );
      if (keep == null) return false;
      if (!keep) return true;
      ui.workspace.setKeepRunning(true);
      await applyBackground(ui.workspace);
      await closeWindow();
      return false;
    }
    if (exportPoll() is! BridgeExportState_Running) return true;
    if (ui.workspace.keepRunning) await showWindow();
    if (!context.mounted) return true;
    return askBefore(context, l10n.exportQuitAsk, l10n.exportQuit);
  };
  app.offerRecovery = (path) => showRecoveryDialogFrb(
      context: context, state: app, projectPath: path, crashed: true);
}

/// Ask, and answer whether the project may go.
///
/// Save runs the ordinary save, which asks for a location when the project has
/// never had one. If that is cancelled, or the write fails, the project still
/// has unsaved changes and the answer is no: nothing is lost and nothing
/// closes. Cancel, Escape and a click outside are all no. The Save and Discard
/// keys are the letters underlined on the buttons.
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
    }
    return true;
  }
  await saveProjectFrb(app, ui, picker: savePicker, leaving: true);
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
    // The dialogue holds no text field, so a letter is never someone typing.
    // It sits above the buttons, so it hears the keys the focused Save does.
    return Focus(
      canRequestFocus: false,
      skipTraversal: true,
      onKeyEvent: _onKey,
      child: DialogFrame(
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
                child: _keyed(l10n.discard, l10n.unsavedKeyDiscard),
              ),
              HouseButton(
                key: const ValueKey('unsaved-save'),
                primary: true,
                autofocus: true,
                onPressed: () => onChoose(true),
                child: _keyed(l10n.save, l10n.unsavedKeySave),
              ),
            ],
          ),
        ],
      ),
    );
  }

  /// The Save and Discard letters, on their own: with Ctrl, Alt or Cmd held a
  /// press is a shortcut, not an answer.
  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isAltPressed || keys.isMetaPressed) {
      return KeyEventResult.ignored;
    }
    // The character typed, so a letter on another layout still answers.
    final typed = (event.character ?? event.logicalKey.keyLabel).toLowerCase();
    if (typed.isEmpty) return KeyEventResult.ignored;
    if (typed == l10n.unsavedKeySave.toLowerCase()) {
      onChoose(true);
      return KeyEventResult.handled;
    }
    if (typed == l10n.unsavedKeyDiscard.toLowerCase()) {
      onChoose(false);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// [label] with the first [key] in it underlined. A label that has no such
  /// letter is shown as it is.
  Widget _keyed(String label, String key) {
    final at = key.isEmpty ? -1 : label.toLowerCase().indexOf(key.toLowerCase());
    if (at < 0) return Text(label);
    return Text.rich(TextSpan(children: [
      TextSpan(text: label.substring(0, at)),
      TextSpan(
        text: label.substring(at, at + key.length),
        style: const TextStyle(decoration: TextDecoration.underline),
      ),
      TextSpan(text: label.substring(at + key.length)),
    ]));
  }
}
