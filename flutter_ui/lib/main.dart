// Lumit's Flutter frontend (K-174, the frontend alternative experiment).
// The engine stays in the Rust crates; this application is the chrome —
// see docs/archive/flutter-port/ for the plan and the parity checklist.

import 'dart:async';

import 'package:flutter/foundation.dart' show kDebugMode;
import 'package:flutter/material.dart';
import 'package:lumit_flutter/data/expressions_metadata.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart'
    show BridgePluginScan, rescanPlugins;
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/install_site.dart';
import 'package:lumit_flutter/probe/perf_probe.dart';
import 'package:lumit_flutter/shell/app_shell.dart';
import 'package:lumit_flutter/shell/startup_failure.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/state/bridge_trace.dart';
import 'package:lumit_flutter/state/faults.dart';
import 'package:lumit_flutter/state/ui_state.dart';

// main.dart is the entry point and nothing else; the shell's parts live beside
// it and are re-exported here so every existing `main.dart` import still finds
// them.
export 'package:lumit_flutter/shell/app_shell.dart';
export 'package:lumit_flutter/state/app_state.dart';
export 'package:lumit_flutter/state/bridge_trace.dart';
export 'package:lumit_flutter/state/ui_state.dart';

Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();

  // Before anything that could fault (K-741). A widget whose build throws is
  // replaced by Flutter with a blank grey rectangle in a release build, and the
  // exception is printed to a console a windowed Windows build does not have —
  // so until this line existed, a broken panel and an empty one looked the
  // same and left nothing behind either way. See state/faults.dart.
  recordFaults();
  try {
    await _start(args);
  } catch (error, stack) {
    // The window is invisible until Flutter's first frame, and nothing before
    // `runApp` draws one — so anything thrown on the way up left Lumit as a
    // process in Task Manager and nothing on screen, with the message on a
    // standard error nobody has (reported against 0.3.0). Say what happened
    // instead: in a window, and — through the same recorder a faulting panel
    // uses — in the diagnostics file.
    recordFault('start-up failed: $error', stack);
    runApp(StartupFailureApp('$error'));
  }
}

/// Everything that has to happen before the shell can go on screen.
Future<void> _start(List<String> args) async {
  // Sweep up after an update before anything else happens (K-297): delete the
  // version we have just replaced, now that nothing is holding its files, and
  // put it back if a swap was cut in half. Never throws and never blocks — a
  // tidying problem is not a reason for an editor not to open.
  tidyAfterUpdate(InstallSite.detect());

  // The call tracer takes StackTrace.current on every bridge call, which is
  // debugging money a release build must not spend.
  // The parked measurement probe (docs/impl/ui-performance.md §6): when a
  // build is given LUMIT_PROBE_PROJECT, bridge calls are counted per gesture.
  final probeBridge =
      probeProjectPath.isEmpty ? null : CountingBridgeHandler();
  await BridgeLib.init(
      handler: kDebugMode ? CustomHandler() : probeBridge);
  await ExpressionsMetadata.load();
  await ExpressionTextEditingController.initSyntaxHighlighting();
  final state = LumitState();
  // Start with an empty project rather than nothing at all. Every document
  // command — import, new composition, save — is disabled while there is no
  // project, so booting without one left the whole File and Composition menu
  // dead and no way to make it live: the first thing a user does needs
  // something to do it *to*.
  state.newProject();
  // A document on the command line opens over the empty project. On failure
  // openProject posts its notice and the empty project stands — the same
  // degraded-but-alive behaviour as a failed File → Open.
  final fromArgs = projectPathFromArgs(args) ??
      (probeProjectPath.isEmpty ? null : probeProjectPath);
  if (fromArgs != null) state.openProject(fromArgs);
  // The one start-up plugin scan (docs/12 §2.6, K-594). Not awaited: opening
  // other people's bundles and spawning a broker apiece takes as long as it
  // takes, and the shell must come up whether the machine has eighty plugins on
  // it or none. The effects added arrive in the browser's next read, and a
  // bundle that would not load is a line in the report rather than anything the
  // user is stopped by.
  unawaited(rescanPlugins().catchError((_) => BridgePluginScan(
        registered: const [],
        skipped: const [],
      )));
  // Somebody who double-clicked a `.lum` has already answered the welcome
  // screen's question, so it is not put to them (K-464).
  final ui = LumitUiState(state);
  runApp(LumitAppNew(state, ui, welcome: fromArgs == null));
  // The probe drives the measured gestures and writes its table, only when
  // asked for by the define — an ordinary build compiles all of it out of reach.
  if (probeProjectPath.isNotEmpty) startPerfProbe(state, ui, probeBridge);
}
