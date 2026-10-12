// An extension's page, on a REAL window, on whichever platform runs this.
//
// The page is drawn by the system's web view, and that is different code on
// each platform: the webview_windows plugin, macos/Runner/ExtensionViews.swift
// and linux/runner/extension_views.cc. None of it can be reached by `flutter
// test`, which has no window and no runner. Here the application's own
// runner is up, a small extension is written to a temporary folder, and its
// page reports what it found with `app.notice`.
//
// One notice covers the whole path: the folder was served under the
// extension's own name, the page was given `window.lumit` before its own
// script ran, a call reached Lumit, and its answer reached the page. The
// rest of the line is what an extension is promised on every platform: a
// page the web trusts, storage of its own, and its own files to fetch.
//
// Run, from flutter_ui/:
//
//   flutter test integration_test/extension_page_test.dart -d windows

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/extension_panel.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/workspace.dart';

import '../test/frb/frb_test_support.dart' show hostPanel;

const _page = '''
<!doctype html>
<meta charset="utf-8">
<title>Probe</title>
<script src="probe.js"></script>
''';

const _script = r'''
async function probe() {
  const info = await lumit.call("app.info");
  const project = await lumit.call("project.info");
  localStorage.setItem("probe", "yes");
  let file = "none";
  try {
    file = (await (await fetch("note.txt")).text()).trim();
  } catch (error) {
    file = "refused";
  }
  let share = "answered";
  try {
    await lumit.call("share.state");
  } catch (error) {
    share = error.code;
  }
  await lumit.call("app.notice", {
    text: `api ${info.api}, trusted ${isSecureContext}, `
      + `subtle ${typeof crypto.subtle}, kept ${localStorage.getItem("probe")}, `
      + `file ${file}, project ${project.open}, share ${share}`,
  });
}
if (window.lumit) probe(); else window.addEventListener("lumitready", probe);
''';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  // Frames as the application asks for them: the Linux page is placed after
  // each frame, as it is in the application.
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.fullyLive;

  testWidgets('an extension page loads, is trusted, and talks both ways',
      (tester) async {
    final scratch = Directory.systemTemp.createTempSync('lumit-extension-page');
    // Never the developer's own settings file.
    Workspace.storeOverride = '${scratch.path}/workspace.json';
    final folder = Directory('${scratch.path}/probe')..createSync();
    File('${folder.path}/index.html').writeAsStringSync(_page);
    File('${folder.path}/probe.js').writeAsStringSync(_script);
    File('${folder.path}/note.txt').writeAsStringSync('served\n');

    await BridgeLib.init();
    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: Workspace());
    final said = <String>[];
    state.notice.addListener(() {
      final notice = state.notice.value;
      if (notice != null) said.add(notice.message);
    });

    await tester.pumpWidget(hostPanel(
      state: state,
      uiState: ui,
      child: ExtensionView(
        extension: BridgeExtension(
          id: 'probe',
          name: 'Probe',
          version: '1.0.0',
          summary: '',
          author: '',
          homepage: '',
          folder: folder.path,
          entry: 'index.html',
          // Not `share`, so the page's one share call is refused.
          permissions: const [BridgeExtensionPermission.project],
          hosts: const [],
          broken: false,
        ),
      ),
    ));

    // A web view starts a process of its own, which takes its time on a
    // shared machine.
    final deadline = DateTime.now().add(const Duration(seconds: 90));
    while (said.isEmpty && DateTime.now().isBefore(deadline)) {
      await tester.pump(const Duration(milliseconds: 100));
    }

    expect(said, [
      'Probe: api 1, trusted true, subtle object, kept yes, '
          'file served, project true, share not-allowed',
    ]);
  });
}
