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
import 'package:lumit_flutter/shell/app_shell.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/state/bridge_trace.dart';
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

  // Sweep up after an update before anything else happens (K-297): delete the
  // version we have just replaced, now that nothing is holding its files, and
  // put it back if a swap was cut in half. Never throws and never blocks — a
  // tidying problem is not a reason for an editor not to open.
  tidyAfterUpdate(InstallSite.detect());

  // The call tracer takes StackTrace.current on every bridge call, which is
  // debugging money a release build must not spend.
  await BridgeLib.init(handler: kDebugMode ? CustomHandler() : null);
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
  final fromArgs = projectPathFromArgs(args);
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
  runApp(LumitAppNew(state, LumitUiState(state), welcome: fromArgs == null));
}
