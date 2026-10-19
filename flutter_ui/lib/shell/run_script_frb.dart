// File ▸ Run script…: a JavaScript or After Effects script run on the open
// project.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/src/rust/api/script.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/file_dialogs.dart';
import '../state/ui_state.dart';
import 'script_dialog_frb.dart';

/// Pick a `.js` or `.jsx` file and run it on the open project.
///
/// The busy card covers the window until the script ends, with Stop on it.
/// The whole run is one undo step, and an edit made from the interface
/// meanwhile would land inside it.
///
/// [context] is where a window the script shows opens. [picker] and
/// [chooser] stand in for the file dialogues in a test.
Future<void> runScriptFrb(
  LumitState app,
  LumitUiState ui, {
  BuildContext? context,
  Future<String?> Function()? picker,
  Future<String?> Function(List<String> extensions, String title)? chooser,
}) async {
  final project = app.project;
  if (project == null) return;
  final dialogs = ScriptDialogs(context);
  final path = await (picker ?? pickScript)();
  if (path == null) return;
  final name = _scriptName(path);

  // Up before the script starts, so nothing gets in ahead of it.
  app.busyProgress.value = null;
  app.busyCancel.value = stopScript;
  app.busyCancelLabel.value = l10n.scriptStop;
  app.busy.value = l10n.scriptRunning(name);

  final events = RustStreamSink<BridgeScriptEvent>();
  final ended = Completer<BridgeScriptEvent_Finished?>();
  var started = false;
  try {
    started = project.runScript(
        path: path, comp: ui.selectedComp, events: events);
  } catch (_) {}
  StreamSubscription<BridgeScriptEvent>? listening;
  if (started) {
    listening = events.stream.listen((event) async {
      switch (event) {
        case BridgeScriptEvent_ChooseFile(:final title, :final extensions):
          String? picked;
          try {
            picked = await (chooser ?? pickEffectInputFile)(extensions, title);
          } catch (_) {}
          answerScriptFile(path: picked);
        case BridgeScriptEvent_Alert(:final message):
          app.postNotice(message);
        case BridgeScriptEvent_OpenComp(:final comp):
          ui.setSelectedComp(comp);
        case BridgeScriptEvent_ShowDialog(:final tree):
          dialogs.show(tree);
        case BridgeScriptEvent_CloseDialog():
          dialogs.close();
        case BridgeScriptEvent_Finished():
          if (!ended.isCompleted) ended.complete(event);
      }
    }, onDone: () {
      if (!ended.isCompleted) ended.complete(null);
    });
  }
  final end = started ? await ended.future : null;
  await listening?.cancel();
  dialogs.close();

  app.busy.value = null;
  app.busyCancel.value = null;
  app.busyCancelLabel.value = null;

  switch (end) {
    case null:
      app.postNotice(l10n.scriptCouldNotRun(name), error: true);
    case BridgeScriptEvent_Finished(stopped: true):
      app.postNotice(l10n.scriptStopped(name));
    case BridgeScriptEvent_Finished(error: null):
      app.postNotice(l10n.scriptRan(name));
    case BridgeScriptEvent_Finished(:final error?, line: final line?):
      app.postNotice(l10n.scriptFailedAt(name, line, error), error: true);
    case BridgeScriptEvent_Finished(:final error?):
      app.postNotice(l10n.scriptFailed(name, error), error: true);
  }
  app.notifyDocumentChanged();
}

/// The file's name without its folder or extension.
String _scriptName(String path) {
  final file = path.split(RegExp(r'[\\/]')).last;
  final dot = file.lastIndexOf('.');
  return dot > 0 ? file.substring(0, dot) : file;
}
