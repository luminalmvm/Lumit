// File ▸ Scripts: a JavaScript or After Effects script run on the open
// project, picked, from the Scripts folder or dropped on the window.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/src/rust/api/script.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/file_dialogs.dart';
import '../state/ui_state.dart';
import '../widgets/controls.dart';

/// Run the `.js` or `.jsx` file at [path] on the open project, or one picked
/// when there is none.
///
/// The busy card covers the window until the script ends, with Stop on it.
/// The whole run is one undo step, and an edit made from the interface
/// meanwhile would land inside it.
///
/// [picker] and [chooser] stand in for the file dialogues in a test.
Future<void> runScriptFrb(
  LumitState app,
  LumitUiState ui, {
  String? path,
  Future<String?> Function()? picker,
  Future<String?> Function(List<String> extensions, String title)? chooser,
}) async {
  final project = app.project;
  if (project == null) return;
  path ??= await (picker ?? pickScript)();
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
    started = project.runScript(path: path, events: events);
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
        case BridgeScriptEvent_Finished():
          if (!ended.isCompleted) ended.complete(event);
      }
    }, onDone: () {
      if (!ended.isCompleted) ended.complete(null);
    });
  }
  final end = started ? await ended.future : null;
  await listening?.cancel();

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

/// The folder File ▸ Scripts lists: [chosen] in Settings, else Lumit's own,
/// made the first time it is asked for. Null when there is nowhere.
String? scriptsFolderIn(String chosen) {
  if (chosen.isNotEmpty) return chosen;
  try {
    return scriptsFolder();
  } catch (_) {
    return null;
  }
}

/// Whether [path] is a script, by its extension.
bool isScriptFile(String path) {
  final lower = path.toLowerCase();
  return lower.endsWith('.js') || lower.endsWith('.jsx');
}

/// Ask once for each script among [dropped], and run the ones agreed to.
Future<void> runDroppedScripts(
    BuildContext context, Iterable<String> dropped) async {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  for (final path in dropped.where(isScriptFile)) {
    if (!context.mounted || app.project == null || app.busy.value != null) {
      return;
    }
    final run = await showLumitModal<bool>(
      context: context,
      builder: (close) =>
          _RunQuestion(name: _scriptName(path), onChoose: close),
    );
    if (run == true) await runScriptFrb(app, ui, path: path);
  }
}

class _RunQuestion extends StatelessWidget {
  final String name;
  final ValueChanged<bool?> onChoose;

  const _RunQuestion({required this.name, required this.onChoose});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 320,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.all(10),
            child: Text(l10n.scriptRunQuestion(name), style: t.bodyPrimary),
          ),
          const SizedBox(height: 4),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('script-drop-cancel'),
                  small: true,
                  frameless: true,
                  onPressed: () => onChoose(false),
                  child: Text(l10n.cancel, style: t.small),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('script-drop-run'),
                  small: true,
                  primary: true,
                  autofocus: true,
                  onPressed: () => onChoose(true),
                  child: Text(l10n.scriptRun),
                ),
              ],
            ),
          ),
          const SizedBox(height: 10),
        ],
      ),
    );
  }
}
