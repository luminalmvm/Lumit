// The Viewer's "at effect" chip (K-528), against the real engine.
//
// The chip replaced a whole panel, so what it has to prove is what the panel
// proved and one thing more:
//
//  * it appears from **both** selection surfaces — an effect picked in the
//    Effect controls stack and a box picked on the graph are one selection
//    (K-300), so they must be one chip and not two behaviours;
//  * it names the effect it would stop at, the user's own name included;
//  * it clears when the selection can no longer name a single point, and the
//    picture goes back with it — a chip outliving its selection would leave
//    the Viewer showing a truncated composition with nothing saying why;
//  * toggling it costs **one** render request, which is the render a playhead
//    step already costs. The old panel's whole reason for being bounded was
//    that a second viewport is expensive; the chip's is that it is not a
//    second viewport at all.
//
// The prefix render itself — that the picture really differs, and that a cut
// stack names its own frame — is proved engine-side, where the pixels are:
// `lumit-bridge`'s `a_prefix_point_cuts_the_stack_and_names_its_own_frame` and
// `lumit-render/tests/node_prefix_preview.rs`.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show effectLabelOf;
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/panels/viewer_prefix_chip.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';

import 'frb_test_support.dart';

/// Counts render requests alone: the chip's whole cost claim is "one render per
/// toggle", and counting every call would drown that in the read model's own
/// traffic.
class _RenderCounter extends BaseHandler {
  int renders = 0;
  bool counting = false;

  void _tick(String name) {
    // Exactly this one: the nine `render_frame_with_*` drag paths share
    // its prefix and are not what a toggle asks for.
    if (counting && name == 'composition_reference_render_frame') renders++;
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
  TestWidgetsFlutterBinding.ensureInitialized();
  final counter = _RenderCounter();

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
  });

  group('The Viewer\'s "at effect" chip', () {
    /// A comp with one solid carrying a blur and an exposure, selected.
    ({LumitState state, LumitUiState uiState, LayerReference layer}) withTwo() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      layer.addEffect(name: 'exposure');
      p.uiState.setSelectedComp(comp);
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    /// The chip returns a `Positioned`, so it mounts in a Stack — the Viewer's
    /// stage stack, and here a bare one. Nothing else of the Viewer is needed:
    /// the chip reads the selection and the read model and nothing on screen.
    Widget chipOnly(dynamic p) => hostPanel(
          state: p.state as LumitState,
          uiState: p.uiState as LumitUiState,
          child: Stack(
            children: [ViewerPrefixChip(uiState: p.uiState as LumitUiState)],
          ),
        );

    Finder chip() => find.byKey(const ValueKey('viewer-at-effect'));

    testWidgets('is absent until one effect is picked', (tester) async {
      final p = withTwo();
      await tester.pumpWidget(chipOnly(p));
      await tester.pump();
      expect(chip(), findsNothing, reason: 'nothing picked names no point');

      final effects = p.layer.getEffects();
      p.uiState.setEffectSelection(p.layer, [for (final e in effects) e.id()]);
      await tester.pump();
      expect(chip(), findsNothing,
          reason: 'a run of effects names no single point either');

      p.uiState.setEffectSelection(p.layer, [effects.first.id()]);
      await tester.pump();
      expect(chip(), findsOneWidget);
      expect(find.text('at ${effectLabelOf('blur')}'), findsOneWidget);
    });

    /// The heading in the Effect controls stack is one of the two surfaces, and
    /// clicking it is an ordinary pick — no chip-specific plumbing anywhere in
    /// that panel.
    testWidgets('appears from a pick in the Effect controls stack',
        (tester) async {
      final p = withTwo();
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(420, 700),
        child: Stack(children: [
          const EffectControlsPanelFrb(),
          ViewerPrefixChip(uiState: p.uiState),
        ]),
      ));
      await settleFrb(tester, minRounds: 4);

      await tester.tap(find.text(effectLabelOf('exposure').toUpperCase()));
      await tester.pump();

      expect(chip(), findsOneWidget);
      expect(find.text('at ${effectLabelOf('exposure')}'), findsOneWidget);
    });

    /// And the box on the graph canvas is the other. The two are one selection
    /// (K-300), which is the whole reason the chip is one chip.
    testWidgets('appears from a pick on the graph canvas', (tester) async {
      final p = withTwo();
      const size = Size(900, 600);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: size,
        child: Stack(children: [
          const GraphPanelFrb(),
          ViewerPrefixChip(uiState: p.uiState),
        ]),
      ));
      await tester.pump();

      final key = graphNodeKey(p.layer
          .getGraph()
          .nodes
          .firstWhere((n) => n.matchName == 'blur')
          .node);
      await tester.tapAt(
          tester.getCenter(find.byKey(ValueKey<String>('graph-node-$key'))));
      await tester.pump();

      expect(chip(), findsOneWidget);
      expect(find.text('at ${effectLabelOf('blur')}'), findsOneWidget);
    });

    /// The user's own name for an instance (K-321) is what the stack shows, so
    /// it is what the chip shows: two Exposures called "Key" and "Fill" must
    /// not both read "at Exposure".
    testWidgets('names a renamed effect by its own name', (tester) async {
      final p = withTwo();
      // Staged on a handle and committed with the stack, which is how the
      // panel's own rename lands.
      final stack = p.layer.getEffects();
      stack[1].setCustomName(name: 'Key light');
      p.layer.setEffects(effects: stack);
      p.uiState.model.refresh();
      p.uiState.setEffectSelection(p.layer, [p.layer.getEffects()[1].id()]);

      await tester.pumpWidget(chipOnly(p));
      await settleFrb(tester, minRounds: 4);
      expect(find.text('at Key light'), findsOneWidget);
    });

    /// The point is derived from the selection rather than stored beside it, so
    /// there is no second copy to go stale — and the chip cannot survive a
    /// selection it can no longer name.
    testWidgets('clears with the selection, and the picture goes back',
        (tester) async {
      final p = withTwo();
      final first = p.layer.getEffects().first.id();
      p.uiState.setEffectSelection(p.layer, [first]);
      await tester.pumpWidget(chipOnly(p));
      await tester.pump();

      await tester.tap(chip());
      await tester.pump();
      expect(p.uiState.atSelectedEffect.value, isTrue);
      expect(p.uiState.viewerPrefix, isNotNull);
      expect(p.uiState.viewerPrefix!.effect, first);

      p.uiState.clearEffectSelection();
      await tester.pump();
      expect(chip(), findsNothing);
      expect(p.uiState.atSelectedEffect.value, isFalse,
          reason: 'no selection, no point to stop at');
      expect(p.uiState.viewerPrefix, isNull,
          reason: 'and so the Viewer is back to the finished picture');
    });

    /// Walking down a stack is the gesture this exists for, so a **different**
    /// single effect keeps it engaged and moves the point.
    testWidgets('follows a pick that moves to another effect', (tester) async {
      final p = withTwo();
      final effects = p.layer.getEffects();
      p.uiState.setEffectSelection(p.layer, [effects.first.id()]);
      await tester.pumpWidget(chipOnly(p));
      await tester.pump();
      await tester.tap(chip());
      await tester.pump();

      p.uiState.setEffectSelection(p.layer, [effects[1].id()]);
      await tester.pump();
      expect(p.uiState.atSelectedEffect.value, isTrue);
      expect(p.uiState.viewerPrefix!.effect, effects[1].id());
      expect(find.text('at ${effectLabelOf('exposure')}'), findsOneWidget);

      // But a run does not, because a run names no single point.
      p.uiState.setEffectSelection(p.layer, [for (final e in effects) e.id()]);
      await tester.pump();
      expect(p.uiState.atSelectedEffect.value, isFalse);
      expect(chip(), findsNothing);
    });

    /// **The cost claim.** A toggle is one render request — the same one a
    /// playhead step makes — because the point rides the render the Viewer was
    /// going to ask for anyway. The old panel needed a bound because it was a
    /// second render; this needs one to show that it is not.
    testWidgets('a toggle costs one render request, and a hover none',
        (tester) async {
      final p = withTwo();
      p.uiState.setEffectSelection(p.layer, [p.layer.getEffects().first.id()]);
      await tester.pumpWidget(chipOnly(p));
      await settleFrb(tester, minRounds: 4);

      counter
        ..renders = 0
        ..counting = true;
      await tester.tap(chip());
      await tester.pump();
      final onCost = counter.renders;
      await tester.tap(chip());
      await tester.pump();
      final total = counter.renders;
      counter.counting = false;

      expect(onCost, 1, reason: 'turning it on is one render');
      expect(total, 2, reason: 'and turning it off is one more');

      // A rebuild that changes nothing asks for nothing: the chip draws from
      // the read model and the selection, both already in Dart.
      counter
        ..renders = 0
        ..counting = true;
      for (var i = 0; i < 3; i++) {
        p.uiState.model.notifyListeners();
        await tester.pump();
      }
      counter.counting = false;
      expect(counter.renders, 0, reason: 'a redraw is not a render');
    });
  });
}
