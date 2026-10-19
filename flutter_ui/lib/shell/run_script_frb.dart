// File ▸ Scripts: a JavaScript or After Effects script run on the open
// project, picked, from the Scripts folder or dropped on the window.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/src/rust/api/script.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/file_dialogs.dart';
import '../state/ui_state.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// Run the `.js` or `.jsx` file at [path] on the open project, or one picked
/// when there is none.
///
/// The busy card covers the window until the script ends, with Stop on it.
/// The whole run is one undo step, and an edit made from the interface
/// meanwhile would land inside it.
///
/// What it printed is a press away on the notice after.
///
/// [picker] and [chooser] stand in for the file dialogues in a test.
Future<void> runScriptFrb(
  BuildContext context,
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
  // Its alerts, for the log, which has only what it printed.
  final alerts = <String>[];
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
          alerts.add(message);
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

  final log = [...?end?.log, ...alerts];
  void say(String message, {bool error = false}) => app.postNotice(message,
      error: error,
      action: log.isEmpty ? null : l10n.scriptShowLog,
      onTap: log.isEmpty
          ? null
          : () {
              if (context.mounted) showScriptLog(context, name, log);
            });
  switch (end) {
    case null:
      say(l10n.scriptCouldNotRun(name), error: true);
    case BridgeScriptEvent_Finished(stopped: true):
      say(l10n.scriptStopped(name));
    case BridgeScriptEvent_Finished(error: null):
      say(l10n.scriptRan(name));
    case BridgeScriptEvent_Finished(:final error?, line: final line?):
      say(l10n.scriptFailedAt(name, line, error), error: true);
    case BridgeScriptEvent_Finished(:final error?):
      say(l10n.scriptFailed(name, error), error: true);
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
    if (run == true && context.mounted) {
      await runScriptFrb(context, app, ui, path: path);
    }
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

/// What the script [name] printed, in a window of its own, with Copy.
Future<void> showScriptLog(
        BuildContext context, String name, List<String> log) =>
    showLumitModal<void>(
      context: context,
      id: 'script-log',
      initialSize: const Size(520, 360),
      builder: (close) =>
          _ScriptLog(name: name, log: log, onClose: () => close(null)),
    );

class _ScriptLog extends StatefulWidget {
  final String name;
  final List<String> log;
  final VoidCallback onClose;

  const _ScriptLog(
      {required this.name, required this.log, required this.onClose});

  @override
  State<_ScriptLog> createState() => _ScriptLogState();
}

class _ScriptLogState extends State<_ScriptLog> {
  final _scroll = ScrollController();

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      children: [
        dialogTitleBar(
          t,
          title: l10n.scriptLog,
          subject: widget.name,
          onClose: widget.onClose,
          keyPrefix: 'script-log',
        ),
        Flexible(
          child: dialogScroller(
            context,
            t,
            _scroll,
            child: ListView.builder(
              key: const ValueKey('script-log-lines'),
              controller: _scroll,
              padding: const EdgeInsets.all(dialogPadding),
              itemCount: widget.log.length,
              itemBuilder: (_, i) => Text(widget.log[i], style: t.mono),
            ),
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'script-log',
          actions: [
            HouseButton(
              key: const ValueKey('script-log-copy'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: () => unawaited(Clipboard.setData(
                  ClipboardData(text: widget.log.join('\n')))),
              child: Text(l10n.scriptLogCopy),
            ),
          ],
        ),
      ],
    );
  }
}
