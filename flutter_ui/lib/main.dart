// Lumit's Flutter frontend (the frontend alternative experiment).
// The engine stays in the Rust crates; this application is the chrome —
// see docs/archive/flutter-port/ for the plan and the parity checklist.

import 'dart:async';

import 'package:flutter/foundation.dart' show kDebugMode;
import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show RustStreamSink;
import 'package:lumit_flutter/data/expressions_metadata.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart'
    show clearCache, clearVramCache;
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/about_window_frb.dart' show lumitVersion;
import 'package:lumit_flutter/shell/splash_window.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart'
    show BridgeScanProgress, BridgeScanStage, rescanPlugins;
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/install_site.dart';
import 'package:lumit_flutter/probe/perf_probe.dart';
import 'package:lumit_flutter/shell/app_shell.dart';
import 'package:lumit_flutter/shell/startup_failure.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/state/background.dart';
import 'package:lumit_flutter/state/bridge_trace.dart';
import 'package:lumit_flutter/state/faults.dart';
import 'package:lumit_flutter/state/ui_state.dart';
import 'package:lumit_flutter/state/updates.dart' show versionFromBootLine;

// main.dart is the entry point and nothing else; the shell's parts live beside
// it and are re-exported here so every existing `main.dart` import still finds
// them.
export 'package:lumit_flutter/shell/app_shell.dart';
export 'package:lumit_flutter/state/app_state.dart';
export 'package:lumit_flutter/state/bridge_trace.dart';
export 'package:lumit_flutter/state/ui_state.dart';

Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();

  // Before anything that could fault. A widget whose build throws is
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
    await BootSplash.handOver();
    runApp(StartupFailureApp('$error'));
  }
}

/// The boot splash, while it is up.
BootSplash? _splash;

/// Everything that has to happen before the shell can go on screen.
Future<void> _start(List<String> args) async {
  // Sweep up after an update before anything else happens: delete the
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
  // macOS says what a double-click or a clicked link asked for in an event,
  // not on the command line. What has come so far joins the arguments.
  args = [...args, ...await takeOpenRequests()];
  final fromArgs = projectPathFromArgs(args) ??
      (probeProjectPath.isEmpty ? null : probeProjectPath);
  // An invite link clicked in a browser starts Lumit with the link, and the
  // Shared project window opens on it once there is a window to open it in.
  state.launchInvite = inviteFromArgs(args);
  // And a double-clicked extension package asks to be installed.
  state.launchPackage = packagePathFromArgs(args);
  final ui = LumitUiState(state);
  // Up once the settings are read, so it speaks the person's language.
  _splash = await BootSplash.open(
      version: versionFromBootLine(lumitVersion()) ?? '',
      step: l10n.bootLoadingExtensions,
      accent: ui.workspace.themedSplash ? ui.workspace.theme.accent : null);
  // Every plugin is in before anything opens, so a project never draws one as
  // missing because the scan had not reached it yet.
  await _scanPlugins();
  _splash?.say(l10n.bootStartingUp, fraction: 1);
  // A document on the command line opens over the empty project. On failure
  // openProject posts its notice and the empty project stands — the same
  // degraded-but-alive behaviour as a failed File → Open.
  if (fromArgs != null) state.openProject(fromArgs, launch: true);
  // Where a new project's template is, which is the workspace's to say. The
  // project Lumit starts on is a new one too, unless something else was asked
  // for.
  state.projectTemplate = () =>
      ui.workspace.projectTemplate.isEmpty ? null : ui.workspace.projectTemplate;
  if (fromArgs == null && ui.workspace.projectTemplate.isNotEmpty) {
    unawaited(state.newProject(ask: false));
  }
  // The account and its sync, which a test never starts: nothing else at
  // start-up speaks to a server of Lumit's.
  unawaited(ui.profiles.start());
  ui.reviews.start();
  ui.phones.start();
  // Closing the window into the tray, when Settings asks for it. The frames
  // held for the preview are given back meanwhile, nobody is looking at them.
  listenToRunner(hidden: () {
    clearCache();
    clearVramCache();
  });
  unawaited(applyBackground(ui.workspace));
  // The main window takes over from the splash once it has something on it.
  WidgetsBinding.instance.addPostFrameCallback((_) => BootSplash.handOver());
  // Somebody who double-clicked a `.lum` has already answered the welcome
  // screen's question, so it is not put to them.
  runApp(LumitAppNew(state, ui,
      welcome: fromArgs == null &&
          state.launchInvite == null &&
          state.launchPackage == null));
  // The probe drives the measured gestures and writes its table, only when
  // asked for by the define — an ordinary build compiles all of it out of reach.
  if (probeProjectPath.isNotEmpty) startPerfProbe(state, ui, probeBridge);
}

/// The one start-up plugin scan (docs/12 §2.6), saying on the splash which
/// bundle it is opening. A bundle that would not load is a line in the report
/// rather than anything the user is stopped by.
Future<void> _scanPlugins() async {
  final progress = RustStreamSink<BridgeScanProgress>();
  StreamSubscription<BridgeScanProgress>? listening;
  try {
    // The sink has a stream only once the call has taken it.
    final scan =
        effectSchema.scanned(() => rescanPlugins(progress: progress));
    listening = progress.stream.listen(_sayScan, onError: (_) {});
    await scan;
  } catch (_) {
    // A scan that failed found nothing, and Lumit runs without plugins.
  } finally {
    await listening?.cancel();
  }
}

/// One step of the scan, as a line on the splash. The OFX bundles are most
/// of the wait, so they get most of the bar.
void _sayScan(BridgeScanProgress p) {
  final through =
      p.total == 0 ? 0.0 : (p.done + (p.effect.isEmpty ? 0 : 1)) / p.total;
  final detail = p.effect.isEmpty ? p.bundle : '${p.bundle} · ${p.effect}';
  switch (p.stage) {
    case BridgeScanStage.extensions:
      _splash?.say(l10n.bootLoadingExtensions, fraction: 0.05);
    case BridgeScanStage.effects:
      _splash?.say(l10n.bootLoadingPlugins,
          detail: detail, fraction: 0.1 + 0.75 * through);
    case BridgeScanStage.audio:
      _splash?.say(l10n.bootLoadingAudioPlugins,
          detail: detail, fraction: 0.85 + 0.15 * through);
  }
}
