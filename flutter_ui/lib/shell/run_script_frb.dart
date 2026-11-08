// File ▸ Scripts: a JavaScript or After Effects script run on the open
// project, picked, from the Scripts folder or dropped on the window.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/src/rust/api/script.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show revealInFolder;
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/file_dialogs.dart';
import '../state/settings.dart' show ScriptEvent;
import '../state/ui_state.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'menu_bar_frb.dart' show MenuEntry;
import 'question_window.dart';
import 'review_link_parts.dart' show reviewRowMenu;
import 'script_dialog_frb.dart';
import 'package:lumit_flutter/state/platform.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/platform_web.dart';

/// Run the `.js` or `.jsx` file at [path] on the open project, or one picked
/// when there is none.
///
/// The busy card covers the window until the script ends, with Stop on it.
/// The whole run is one undo step, and an edit made from the interface
/// meanwhile would land inside it.
///
/// What it printed is a press away on the notice after.
///
/// [after] is the event an event script runs after, which it reads with the
/// file's name in `lumit.args`.
///
/// [context] is where a window the script shows opens. [picker] and
/// [chooser] stand in for the file dialogues in a test.
Future<void> runScriptFrb(
  BuildContext context,
  LumitState app,
  LumitUiState ui, {
  String? path,
  (ScriptEvent, String)? after,
  Future<String?> Function()? picker,
  Future<String?> Function(List<String> extensions, String title)? chooser,
}) async {
  final project = app.project;
  if (project == null) return;
  final dialogs = ScriptDialogs(context);
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
    started = project.runScript(
        path: path,
        comp: ui.selectedComp,
        args: {
          if (after != null) ...{'event': after.$1.name, 'file': after.$2},
        },
        events: events);
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
      say(switch (after?.$1) {
        null => l10n.scriptRan(name),
        ScriptEvent.export => l10n.scriptRanAfterExport(name),
        ScriptEvent.open => l10n.scriptRanAfterOpen(name),
        ScriptEvent.save => l10n.scriptRanAfterSave(name),
      });
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

/// Give LumitState somewhere to run the scripts chosen in Settings. Called
/// once, from the widget that is up for the whole life of the window.
void installEventScripts(BuildContext context) {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  app.runEventScript = (event, path) {
    final script = ui.workspace.eventScripts[event];
    final folder = scriptsFolderIn(ui.workspace.scriptsFolder);
    if (script == null || folder == null || app.project == null) return;
    final sep = Platform.pathSeparator;
    // Never on top of another script, or anything else the card is up for.
    // A script's own saves and exports never get here, so none can loop.
    if (app.busy.value != null) {
      app.postNotice(l10n.scriptEventSkipped(_scriptName(script)));
      return;
    }
    unawaited(runScriptFrb(context, app, ui,
        path: '$folder$sep${script.replaceAll('/', sep)}',
        after: (event, path.split(RegExp(r'[\\/]')).last)));
  };
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
      builder: (close) => QuestionWindow(
        title: l10n.scriptRunQuestion(_scriptName(path)),
        keyPrefix: 'script-drop',
        onClose: () => close(false),
        actions: [
          questionPlain('script-drop-cancel', l10n.cancel, () => close(false)),
          questionFilled('script-drop-run', l10n.scriptRun, () => close(true)),
        ],
      ),
    );
    if (run == true && context.mounted) {
      await runScriptFrb(context, app, ui, path: path);
    }
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

  void _copy(String text) =>
      unawaited(Clipboard.setData(ClipboardData(text: text)));

  void _menu(Offset at, String line) => reviewRowMenu(context, at, [
        ('script-copy-line', l10n.copyLine, () => _copy(line)),
        ('script-copy-all', l10n.scriptLogCopy, () => _copy(widget.log.join('\n'))),
      ]);

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
        Expanded(
          child: dialogScroller(
            context,
            t,
            _scroll,
            child: ListView.builder(
              key: const ValueKey('script-log-lines'),
              controller: _scroll,
              padding: const EdgeInsets.all(dialogPadding),
              itemCount: widget.log.length,
              itemBuilder: (_, i) => GestureDetector(
                behavior: HitTestBehavior.opaque,
                onSecondaryTapDown: (press) =>
                    _menu(press.globalPosition, widget.log[i]),
                child: Text(widget.log[i], style: t.mono),
              ),
            ),
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'script-log',
          actions: [
            questionPlain('script-log-copy', l10n.scriptLogCopy,
                () => _copy(widget.log.join('\n'))),
            questionFilled('script-log-close', l10n.close, widget.onClose),
          ],
        ),
      ],
    );
  }
}

/// File ▸ Scripts: each script in the Scripts folder, a submenu for each
/// folder inside it, then Run script… and the folder itself. Read from disk
/// as the menu opens.
List<MenuEntry> scriptRows(
    BuildContext context, LumitState app, LumitUiState ui) {
  final folder = scriptsFolderIn(ui.workspace.scriptsFolder);
  var scripts = const <String>[];
  try {
    if (folder != null) scripts = listScripts(folder: folder);
  } catch (_) {}
  VoidCallback? run(String path) => app.project == null
      ? null
      : () => runScriptFrb(context, app, ui,
          path: '$folder${Platform.pathSeparator}'
              '${path.replaceAll('/', Platform.pathSeparator)}');

  // A folder's scripts come together in the list, so each run of paths
  // under one name is that folder's submenu.
  List<MenuEntry> rows(List<String> paths, String under) {
    final out = <MenuEntry>[];
    for (var i = 0; i < paths.length;) {
      final slash = paths[i].indexOf('/');
      if (slash < 0) {
        out.add(MenuEntry(paths[i], run('$under${paths[i]}')));
        i++;
        continue;
      }
      final name = paths[i].substring(0, slash + 1);
      final inside = <String>[];
      for (; i < paths.length && paths[i].startsWith(name); i++) {
        inside.add(paths[i].substring(name.length));
      }
      out.add(MenuEntry.submenu(
          name.substring(0, slash), rows(inside, '$under$name')));
    }
    return out;
  }

  return [
    ...rows(scripts, ''),
    if (scripts.isNotEmpty) MenuEntry.divider(),
    MenuEntry(l10n.menuRunScript,
        app.project == null ? null : () => runScriptFrb(context, app, ui)),
    MenuEntry(l10n.menuOpenScriptsFolder,
        folder == null ? null : () => revealInFolder(path: folder)),
  ];
}
