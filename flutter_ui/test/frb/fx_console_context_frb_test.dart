// The FX console's Keyframe ring (K-326), against the real engine: a slice
// plants a key at the playhead, never moves the picture, and asks the
// Timeline to show the row it keyed.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/shell/fx_console_context.dart';
import 'package:lumit_flutter/shell/fx_console_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  ({
    LumitState state,
    LumitUiState uiState,
    CompositionReference comp,
    LayerReference layer,
  }) withLayer() {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    p.uiState.setSelectedComp(comp);
    final layer = comp.addSolidLayer();
    p.uiState.model.refresh();
    return (state: p.state, uiState: p.uiState, comp: comp, layer: layer);
  }

  /// The keys the layer's Position x carries right now, by frame.
  List<int> positionKeyFrames(
      LumitUiState ui, LayerReference layer, CompositionReference comp) {
    ui.model.refresh();
    final entry = ui.model.byId(layer.internallayerId)!;
    return switch (entry.info.transform.positionX) {
      BridgeScalar_Keyframed(:final field0) => [
          for (final k in field0) comp.frameAtTime(time: k.time)
        ],
      _ => const [],
    };
  }

  group('FX console keyframe ring (frb)', () {
    test('one slice per everyday transform row, and never the 3D extras', () {
      final p = withLayer();
      final ring =
          fxConsoleKeyframeRing(p.state, p.uiState, p.layer, p.comp);
      expect(ring.map((e) => e.label).toList(),
          ['Anchor point', 'Position', 'Scale', 'Rotation', 'Opacity'],
          reason: 'five rows — a ring is capped at six (docs/07 §12.2)');
    });

    test('a slice plants a key at the playhead and shows the row', () {
      final p = withLayer();
      p.uiState.playheadFrame.value = 12;
      final ring =
          fxConsoleKeyframeRing(p.state, p.uiState, p.layer, p.comp);
      ring.firstWhere((e) => e.label == 'Position').run!();

      expect(positionKeyFrames(p.uiState, p.layer, p.comp), [12],
          reason: 'one key, where the playhead stood');
      expect(p.uiState.revealPropertyRequest.value,
          (p.layer.internallayerId, 'reveal.position'),
          reason: 'the Timeline is asked to show the row just keyed');
      expect(p.uiState.activePanel.value, Panel.timeline,
          reason: 'and fronted, so the key is on screen');
    });

    test('a second frame keys again; the same frame never duplicates', () {
      final p = withLayer();
      p.uiState.playheadFrame.value = 12;
      RadialEntry position() =>
          fxConsoleKeyframeRing(p.state, p.uiState, p.layer, p.comp)
              .firstWhere((e) => e.label == 'Position');
      position().run!();
      position().run!();
      expect(positionKeyFrames(p.uiState, p.layer, p.comp), [12],
          reason: 'keying an already-keyed frame only reveals');

      p.uiState.playheadFrame.value = 30;
      position().run!();
      expect(positionKeyFrames(p.uiState, p.layer, p.comp), [12, 30],
          reason: 'a new frame inserts in order');
    });

    test('a row driven by an expression is dimmed, not keyed over', () {
      final p = withLayer();
      p.layer.setTransform(
          prop: BridgeTransformProp.positionX,
          value: const BridgeScalar.expression('time'));
      p.uiState.model.refresh();
      final ring =
          fxConsoleKeyframeRing(p.state, p.uiState, p.layer, p.comp);
      expect(ring.firstWhere((e) => e.label == 'Position').enabled, isFalse,
          reason: 'writing keys over an expression would delete it');
      expect(ring.firstWhere((e) => e.label == 'Scale').enabled, isTrue,
          reason: 'only the expressed row dims');
    });
  });

  group('a Project panel item in the ring (frb)', () {
    // `fxConsoleRadial` takes a BuildContext for the closures other contexts
    // capture; the project-item slice never touches it, so any mounted
    // context serves.
    Future<BuildContext> anyContext(WidgetTester tester) async {
      late BuildContext ctx;
      await tester.pumpWidget(Builder(builder: (c) {
        ctx = c;
        return const SizedBox();
      }));
      return ctx;
    }

    testWidgets('footage offers Add to comp, and choosing it places a layer',
        (tester) async {
      final p = withLayer();
      final footage =
          p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.uiState.selectedProjectItem.value = ItemReference.footage(footage);
      p.uiState.activePanel.value = Panel.project;

      final ring =
          fxConsoleRadial(await anyContext(tester), p.state, p.uiState);
      expect(ring.map((e) => e.label).toList(), ['Add to comp'],
          reason: 'one slice — never the new-layer ring this fell through to');
      expect(ring.single.enabled, isTrue);
      final before = p.comp.getLayers().length;
      ring.single.run!();
      expect(p.comp.getLayers().length, before + 1,
          reason: 'the footage landed in the open comp');
    });

    testWidgets('a comp nests, but never into itself', (tester) async {
      final p = withLayer();
      final other = p.state.project!.newComposition(name: 'Titles');
      p.uiState.activePanel.value = Panel.project;
      final ctx = await anyContext(tester);

      p.uiState.selectedProjectItem.value = ItemReference.composition(other);
      final nests = fxConsoleRadial(ctx, p.state, p.uiState).single;
      expect(nests.enabled, isTrue);
      final before = p.comp.getLayers().length;
      nests.run!();
      expect(p.comp.getLayers().length, before + 1,
          reason: 'the comp nested as a precomp layer');

      p.uiState.selectedProjectItem.value = ItemReference.composition(p.comp);
      final self = fxConsoleRadial(ctx, p.state, p.uiState).single;
      expect(self.enabled, isFalse,
          reason: 'a comp cannot nest into itself — dimmed, not a no-op');
    });

    testWidgets('the slice keeps its place, dimmed, when it cannot run',
        (tester) async {
      final p = freshProject();
      final footage =
          p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.uiState.selectedProjectItem.value = ItemReference.footage(footage);
      p.uiState.activePanel.value = Panel.project;

      // No composition open: the slice is offered but dimmed, so the
      // direction is learned before it is ever usable.
      final ring =
          fxConsoleRadial(await anyContext(tester), p.state, p.uiState);
      expect(ring.map((e) => e.label).toList(), ['Add to comp']);
      expect(ring.single.enabled, isFalse);
    });

    testWidgets('the item counts only while the Project panel is where you '
        'stand', (tester) async {
      final p = withLayer();
      final footage =
          p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.uiState.selectedProjectItem.value = ItemReference.footage(footage);
      p.uiState.selectedLayer.value = p.layer;
      p.uiState.activePanel.value = Panel.timeline;

      final ring =
          fxConsoleRadial(await anyContext(tester), p.state, p.uiState);
      expect(ring.map((e) => e.label), isNot(contains('Add to comp')),
          reason: 'in the Timeline, the layer is still the context');
    });
  });

  group('the Timeline answers the reveal request (frb)', () {
    testWidgets('the keyed row is open in the fold-out after the ask',
        (tester) async {
      final p = withLayer();
      p.uiState.selectedLayer.value = p.layer;
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
      expect(find.text('Position'), findsNothing,
          reason: 'the layer starts folded shut');

      p.uiState
          .requestRevealProperty(p.layer.internallayerId, 'reveal.position');
      await tester.pump();
      expect(find.text('Position'), findsOneWidget,
          reason: 'the ask opens the layer and exactly that row');
      expect(p.uiState.revealPropertyRequest.value, isNull,
          reason: 'the request is consumed, not left to re-fire');

      // Asking again must never hide it — ensure-open, not the reveal keys'
      // toggle.
      p.uiState
          .requestRevealProperty(p.layer.internallayerId, 'reveal.position');
      await tester.pump();
      expect(find.text('Position'), findsOneWidget);
    });
  });
}
