// The Scopes panel's own chrome, and when it asks the engine for a trace.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/scopes_panel_frb.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/workspace.dart';

import 'frb_test_support.dart';

/// Counts the trace requests that cross the bridge. The name is the generated
/// one for `CompositionReference.renderScope`, so nothing else is counted.
class _ScopeCounter extends BaseHandler {
  int renders = 0;

  void _tick(String name) {
    if (name == 'composition_reference_render_scope') renders++;
  }

  @override
  Future<S> executeNormal<S, E extends Object>(NormalTask<S, E> task) {
    _tick(task.constMeta.debugName);
    return super.executeNormal(task);
  }

  @override
  S executeSync<S, E extends Object, WireSyncType>(
      SyncTask<S, E, WireSyncType> task) {
    _tick(task.constMeta.debugName);
    return super.executeSync(task);
  }
}

void main() {
  final counter = _ScopeCounter();

  setUpAll(() async {
    final stem = Platform.isWindows
        ? 'lumit_bridge.dll'
        : Platform.isMacOS
            ? 'liblumit_bridge.dylib'
            : 'liblumit_bridge.so';
    await BridgeLib.init(
      externalLibrary: ExternalLibrary.open('../target/debug/$stem'),
      handler: counter,
    );
    // Never the developer's own settings file (see `initEngineForTests`).
    Workspace.storeOverride =
        '${Directory.systemTemp.createTempSync('lumit-ws').path}/workspace.json';
  });

  group('Scopes (frb)', () {
    /// The toolbar names the trace and nothing else. It used to carry
    /// a frame readout beside the picker, which is the Timeline's and the
    /// Viewer's to state — three places saying the same number, and one of
    /// them competing with the trace it sits above.
    testWidgets('the toolbar carries no frame readout', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      p.uiState.playheadFrame.value = 7;

      await tester.pumpWidget(hostPanel(
        child: const ScopesPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.byKey(const ValueKey('scope-kind')), findsOneWidget,
          reason: 'the picker is still there');
      expect(find.textContaining('frame'), findsNothing);
      expect(find.textContaining('7'), findsNothing);
    });

    /// **An edit at a stationary playhead refreshes the trace — once.**
    ///
    /// The regression: dropping the throttle left "same frame, nothing to ask
    /// for" as the whole test, so a value drag — which changes the picture
    /// without moving the playhead — showed the trace from before the edit
    /// until the playhead next moved. The frame reaching the Viewer is what
    /// says the picture changed, so the memo counts arrivals as well as frame
    /// numbers: one request per arrival, and none for the rebuilds between.
    testWidgets('a new picture of the same frame is traced exactly once',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);
      p.uiState.playheadFrame.value = 4;

      final host = hostPanel(
        child: const ScopesPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      );
      await tester.pumpWidget(host);
      await tester.pump();
      final settled = counter.renders;
      expect(settled, greaterThan(0), reason: 'the panel traces on mount');

      // Rebuilds that change neither the frame nor the picture ask for nothing.
      await tester.pumpWidget(host);
      await tester.pump();
      expect(counter.renders, settled,
          reason: 'a plain rebuild must not cost a trace');

      // The edit's frame reaches the Viewer: the playhead has not moved, but
      // the picture under it has.
      p.uiState.frameArrived.value++;
      await tester.pump();
      expect(counter.renders, settled + 1,
          reason: 'the arrival is worth exactly one trace');

      // And the rebuilds that follow it — the trace landing, a hover — do not
      // ask again.
      await tester.pumpWidget(host);
      await tester.pump();
      expect(counter.renders, settled + 1,
          reason: 'once per arrival, never once per rebuild');
    });
  }, skip: !engineAvailable);
}
