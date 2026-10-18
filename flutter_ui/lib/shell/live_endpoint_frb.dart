// Settings ▸ Let other programs drive Lumit: the interface's side of the live
// endpoint. A program connected shows on the status line with Disconnect,
// and a script it runs puts up the busy card File ▸ Run script does.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/src/rust/api/script.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/ui_state.dart';

/// The program driving Lumit: what it calls itself, empty until it says, or
/// null when none is connected.
final ValueNotifier<String?> liveProgram = ValueNotifier(null);

StreamSubscription<BridgeLiveEvent>? _listening;

/// Whether the busy card up now is a program's script.
bool _scriptShown = false;

/// Starts and stops, one after another, so a quick off and on lands in the
/// order it was made.
Future<void> _calls = Future.value();

/// Counts calls to [applyLiveEndpointFrb], so a start that a later call
/// has overtaken is left for that call to settle.
int _applied = 0;

/// Start or stop the endpoint, as [on] says. Off lets go of the program
/// connected.
void applyLiveEndpointFrb(bool on, LumitState app, LumitUiState ui) {
  final applied = ++_applied;
  unawaited(_listening?.cancel());
  _listening = null;
  liveProgram.value = null;
  _endScript(app);
  _calls = _calls.then((_) async {
    if (!on) {
      try {
        await stopLiveEndpoint();
      } catch (_) {}
      return;
    }
    if (applied != _applied) return;
    final events = RustStreamSink<BridgeLiveEvent>();
    var started = false;
    try {
      started = await startLiveEndpoint(events: events);
    } catch (_) {}
    if (applied != _applied) return;
    if (!started) {
      app.postNotice(l10n.liveCouldNotStart, error: true);
      return;
    }
    _listen(events, app, ui);
  });
}

/// Stop the endpoint after whatever was asked before, as Lumit quits.
Future<void> stopLiveEndpointFrb() {
  _applied++;
  return _calls = _calls.then((_) async {
    try {
      await stopLiveEndpoint();
    } catch (_) {}
  });
}

void _listen(
    RustStreamSink<BridgeLiveEvent> events, LumitState app, LumitUiState ui) {
  _listening = events.stream.listen((event) {
    switch (event) {
      case BridgeLiveEvent_Attached(:final name):
        liveProgram.value = name;
      case BridgeLiveEvent_Detached():
        liveProgram.value = null;
        _endScript(app);
      case BridgeLiveEvent_ScriptStarted(:final name):
        // The whole script is one undo step, so the window waits behind
        // the card, as it does for File ▸ Run script.
        _scriptShown = true;
        app.busyProgress.value = null;
        app.busyCancel.value = stopScript;
        app.busyCancelLabel.value = l10n.scriptStop;
        app.busy.value =
            l10n.scriptRunning(name.isEmpty ? l10n.liveScript : name);
        // The script waits for the card to be drawn, or a click meanwhile
        // would land in its undo step. A window that draws nothing, such as
        // a minimised one, takes no clicks either.
        unawaited(Future.any([
          SchedulerBinding.instance.endOfFrame,
          Future<void>.delayed(const Duration(milliseconds: 250)),
        ]).then((_) => liveScriptShown()));
      case BridgeLiveEvent_ScriptEnded():
        _endScript(app);
        app.notifyDocumentChanged();
      case BridgeLiveEvent_OpenComp(:final comp):
        ui.setSelectedComp(comp);
      case BridgeLiveEvent_Alert(:final message):
        app.postNotice(message);
    }
  });
}

void _endScript(LumitState app) {
  if (!_scriptShown) return;
  _scriptShown = false;
  app.busy.value = null;
  app.busyCancel.value = null;
  app.busyCancelLabel.value = null;
}
