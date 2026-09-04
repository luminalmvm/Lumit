// Reopening a project puts the user back where they were: the comps that were
// on the tab strip, the one that was fronted, the frame the playhead sat on and
// the layer that was selected.
//
// None of that is in the `.lum` (docs/10 §1.1 keeps the document free of one
// machine's habits) — it lives in the workspace store, keyed by the project's
// path. So the round trip worth testing is store-out, store-in across a real
// save and a real open, with the engine handing back a genuinely reloaded
// document whose references are new objects carrying the old ids.
//
// `openProject` clears the engine's project registry, which is why this file
// stands alone: every reference an earlier test held would die in it.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/workspace.dart';

import 'frb_test_support.dart';

/// The panel arrangement as one comparable string.
String layoutOf(Workspace workspace) => jsonEncode(workspace.dock.toJson());

void main() {
  setUpAll(initEngineForTests);

  testWidgets('a reopened project comes back where it was left',
      (tester) async {
    final dir = Directory.systemTemp.createTempSync('lumit-session');
    final path = '${dir.path}/session.lum';
    final workspace = Workspace();

    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: workspace);
    final project = state.project!;
    final scene = project.newComposition(name: 'Scene');
    final titles = project.newComposition(name: 'Titles');
    final layer = scene.addSolidLayer();

    project.save(path: path);
    await settleFrb(tester, until: () => File(path).existsSync());
    expect(File(path).existsSync(), isTrue, reason: 'nothing to reopen');

    // Where the user got to: both comps open, Scene fronted, playhead at 12,
    // the solid selected, and the panels dragged to a shape of their own.
    ui.setSelectedComp(titles);
    ui.setSelectedComp(scene);
    ui.setSelection([layer]);
    ui.playheadFrame.value = 12;
    workspace.dock.shares[0] = 0.31;
    final arranged = layoutOf(workspace);
    ui.rememberSession();

    // A different document, then the saved one opened over the top of it. The
    // layout is dragged somewhere else in between, standing in for the user
    // having been in another project with an arrangement of its own.
    state.newProject();
    expect(ui.openComps, isEmpty, reason: 'a new project starts on nothing');
    expect(ui.selectedComp, isNull);
    workspace.dock.shares[0] = 0.5;
    expect(layoutOf(workspace), isNot(arranged));

    // **Load-bearing.** Every panel that draws a comp tab or a menu of comps
    // reads this, so in a running application the cached list is always warm
    // when a project is opened — and a restore that asks it which comps exist
    // gets the *previous* project's answer unless adopting one clears it.
    // Without this line the test opens with a cold cache, which is the one
    // state the application is never in.
    state.comps();

    // Not awaited: reading a document is an async frb call whose continuation
    // only lands on the real event-loop turns settleFrb provides. What is
    // waited for is the *adoption* — `opening` also covers the first frame,
    // which a widget test with no Viewer mounted never receives.
    final adopted = state.project;
    state.openProject(path);
    await settleFrb(tester, until: () => !identical(state.project, adopted));

    expect(layoutOf(workspace), arranged,
        reason: 'the panels came back where this project had them');
    expect(ui.openComps, hasLength(2));
    expect(ui.selectedComp?.internalid, scene.internalid);
    expect(ui.playheadFrame.value, 12);
    expect(ui.selectedLayer.value?.internallayerId, layer.internallayerId,
        reason: 'the selected layer came back with the comp');
    expect(workspace.recentProjects, contains(path));
  });

  /// **Each comp remembers where you were**. Coming back to a comp
  /// through the tab strip is a return, not a fresh start: the playhead goes
  /// back to the frame it was left on rather than to zero, and it survives the
  /// project being closed and opened again.
  testWidgets('every comp comes back on the frame it was left on',
      (tester) async {
    final dir = Directory.systemTemp.createTempSync('lumit-comp-views');
    final path = '${dir.path}/views.lum';
    final workspace = Workspace();

    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: workspace);
    final project = state.project!;
    final scene = project.newComposition(name: 'Scene');
    final titles = project.newComposition(name: 'Titles');

    project.save(path: path);
    await settleFrb(tester, until: () => File(path).existsSync());

    ui.setSelectedComp(scene);
    ui.playheadFrame.value = 40;
    ui.setSelectedComp(titles);
    expect(ui.playheadFrame.value, 0,
        reason: 'a comp nobody has been in opens at its start');
    ui.playheadFrame.value = 7;

    ui.setSelectedComp(scene);
    expect(ui.playheadFrame.value, 40, reason: 'Scene was left on 40');
    ui.setSelectedComp(titles);
    expect(ui.playheadFrame.value, 7, reason: 'Titles was left on 7');

    // The Timeline's own half of the record, written where the panel writes it.
    ui.rememberCompView(scene.internalid.toString(), zoom: 4, scroll: 0.5);
    ui.rememberSession();

    final adopted = state.project;
    state.newProject();
    state.comps();
    state.openProject(path);
    await settleFrb(tester, until: () => !identical(state.project, adopted));

    expect(ui.selectedComp?.internalid, titles.internalid);
    expect(ui.playheadFrame.value, 7);
    final view = ui.compViews[scene.internalid.toString()];
    expect(view?.frame, 40, reason: 'the comp not fronted kept its frame too');
    expect(view?.zoom, 4);
    expect(view?.scroll, 0.5);
  });

  /// Opening a **Precomp layer** is the exception: it enters the nested comp
  /// at the moment that layer is showing, which the engine maps through the
  /// layer's start offset and Retime.
  testWidgets('a precomp opens on the frame the layer is showing',
      (tester) async {
    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: Workspace());
    final project = state.project!;
    final outer = project.newComposition(name: 'Outer');
    final inner = project.newComposition(name: 'Inner');
    final layer = outer.addPrecompLayer(comp: inner);

    ui.setSelectedComp(outer);
    ui.playheadFrame.value = 30;
    ui.openNestedComp(layer, inner);
    expect(ui.selectedComp?.internalid, inner.internalid);
    expect(ui.playheadFrame.value, 30,
        reason: 'an unmoved, unretimed precomp maps frame for frame');

    // Standing past the layer's end opens the nested comp at its own end.
    ui.setSelectedComp(outer);
    ui.playheadFrame.value = outer.durationFrames() - 1;
    ui.openNestedComp(layer, inner);
    expect(ui.playheadFrame.value, inner.durationFrames() - 1);
  });

  testWidgets('a session naming things that have gone falls back quietly',
      (tester) async {
    final dir = Directory.systemTemp.createTempSync('lumit-session-stale');
    final path = '${dir.path}/stale.lum';
    final workspace = Workspace();

    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: workspace);
    state.project!.newComposition(name: 'Scene');
    state.project!.save(path: path);
    await settleFrb(tester, until: () => File(path).existsSync());

    // A session written by an older sitting, naming a comp and a layer that
    // the saved document does not contain.
    workspace.rememberSession(
      path,
      const SavedSession(
        openComps: ['00000000-0000-0000-0000-0000000000aa'],
        activeComp: '00000000-0000-0000-0000-0000000000aa',
        frame: 7,
        selectedLayer: '00000000-0000-0000-0000-0000000000bb',
      ),
    );

    final adopted = state.project;
    state.openProject(path);
    await settleFrb(tester, until: () => !identical(state.project, adopted));

    expect(ui.openComps, isEmpty, reason: 'a comp that is gone opens no tab');
    expect(ui.selectedComp, isNull);
    expect(ui.selectedLayer.value, isNull);
    expect(ui.playheadFrame.value, 7, reason: 'the frame is still the frame');
  });

  /// **The arrangement travels with the file**. The second half of this
  /// stands in for another person's machine: a fresh workspace store that has
  /// never seen the project, so nothing local can answer and the only account
  /// of how the interface was arranged is the one inside the `.lum`.
  testWidgets('a shared project opens arranged the way its author left it',
      (tester) async {
    final dir = Directory.systemTemp.createTempSync('lumit-session-shared');
    final path = '${dir.path}/shared.lum';

    final author = Workspace();
    final authorState = LumitState()..newProject();
    final authorUi = LumitUiState(authorState, workspace: author);
    final scene = authorState.project!.newComposition(name: 'Scene');
    authorUi.setSelectedComp(scene);
    author.dock.shares[0] = 0.27;
    final arranged = layoutOf(author);

    // Not awaited: the save is an async frb call whose continuation only lands
    // on the real event-loop turns settleFrb provides.
    saveProjectFrb(authorState, authorUi, picker: () async => path);
    await settleFrb(tester, until: () => File(path).existsSync());
    expect(File(path).existsSync(), isTrue);

    // Somebody else's machine: their own arrangement, and no record of this
    // project at all.
    final other = Workspace();
    final otherState = LumitState()..newProject();
    final otherUi = LumitUiState(otherState, workspace: other);
    expect(other.sessionFor(path), isNull, reason: 'never opened here');
    expect(layoutOf(other), isNot(arranged));

    final adopted = otherState.project;
    otherState.openProject(path);
    await settleFrb(tester,
        until: () => !identical(otherState.project, adopted));

    expect(layoutOf(other), arranged,
        reason: 'the arrangement came out of the file');
    expect(otherUi.selectedComp?.internalid, scene.internalid,
        reason: 'and so did the comp that was open in it');
  });

  /// **A progress timer must not outlive the state that started it.**
  ///
  /// `PreviewProgressTracker` waits a moment before drawing a bar, so that a
  /// frame which arrives quickly never flashes one. That wait is a timer, and
  /// a timer nobody cancels keeps running after the thing that set it has gone
  /// — in the application a small leak per project session, and in this suite
  /// a failure that lands somewhere else entirely: the tracker of a discarded
  /// UI state fires inside whatever test happens to be running, which then
  /// fails on a pending timer it never created. `cache_bar_frb_test` went red
  /// on main exactly that way while passing on the identical tree elsewhere.
  ///
  /// The test framework fails any test that ends with a timer outstanding, so
  /// starting one and then disposing the state is the whole assertion: without
  /// the `previewProgress.dispose()` in `LumitUiState.dispose`, this reports a
  /// pending timer.
  testWidgets('a preview-progress timer does not outlive its UI state',
      (tester) async {
    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: Workspace());

    // A frame worth waiting for, which is what arms the delay. Not `done`:
    // a finished frame cancels it again and would prove nothing.
    ui.previewProgress.report(BridgeRenderProgress(
      frame: BigInt.from(3),
      stage: 1,
      fraction: 0.25,
      done: false,
    ));

    ui.dispose();
  });

  /// The other half, and the one the fix above cannot reach. A test that keeps
  /// its UI state to the end — which is every test using `freshProject`, since
  /// that disposes on tear-down — can still finish inside the tracker's delay.
  /// `addTearDown` runs *after* `flutter_test` unmounts the tree, pumps, and
  /// asserts that no timer is pending, so disposing there is too late: the
  /// assertion has already fired.
  ///
  /// `hostPanel` therefore stops the tracker as the tree comes down, which
  /// happens inside that unmount. Reporting progress and simply ending is the
  /// whole assertion — without that, this reports a pending timer, which is how
  /// `cache_bar_frb_test` failed on the Linux runner while passing on Windows.
  testWidgets('a mounted panel leaves no progress timer behind',
      (tester) async {
    final p = freshProject();
    await tester.pumpWidget(hostPanel(
      state: p.state,
      uiState: p.uiState,
      child: const SizedBox(),
    ));
    await tester.pump();

    // Arms the 150 ms delay and leaves it armed: not `done`, and nothing here
    // waits for `previewProgress.idle`.
    p.uiState.previewProgress.report(BridgeRenderProgress(
      frame: BigInt.from(7),
      stage: 2,
      fraction: 0.5,
      done: false,
    ));
  });
}
