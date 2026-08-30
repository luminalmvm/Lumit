// The Layer and Animation menu rows that stopped being "(Not implemented)"
// (K-244), tested against the real engine.
//
// Each of these is a second door onto a call the Timeline already makes, so
// what is worth asserting is not the call — it is that the row reaches it, that
// it reaches it for *every* selected layer (K-523), and that a row whose
// precondition is missing greys out rather than failing when pressed.
//
// The bar is mounted the way menu_bar_frb_test mounts it, because the enablement
// of half these rows is about the selection, and the selection lives in a
// notifier the bar does not subscribe to itself.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:provider/provider.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Menu rows (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      tester.view.physicalSize = const Size(1000, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: Align(
          alignment: Alignment.topLeft,
          child: Builder(builder: (context) {
            final state = context.watch<LumitState>();
            context.watch<LumitUiState>();
            return LumitMenuBarFrb(app: state);
          }),
        ),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1000, 900),
      ));
      await tester.pump();
    }

    /// Open [menu], step through [under] when there is one, and click [item].
    Future<void> choose(WidgetTester tester, String menu, String item,
        {String? under}) async {
      await tester.tap(find.byKey(ValueKey<String>('menu-$menu')));
      await tester.pump();
      if (under != null) {
        await tester.tap(find.text(under));
        await tester.pump();
      }
      await tester.ensureVisible(find.text(item).first);
      await tester.pump();
      await tester.tap(find.text(item).first);
      await tester.pump();
    }

    /// Open a menu without choosing anything, and put it away again.
    Future<void> open(WidgetTester tester, String menu, {String? under}) async {
      await tester.tap(find.byKey(ValueKey<String>('menu-$menu')));
      await tester.pump();
      if (under != null) {
        await tester.tap(find.text(under));
        await tester.pump();
      }
    }

    /// Re-read the model and rebuild the bar.
    ///
    /// `tester.pump()` with no duration does not move the fake clock, and the
    /// read model groups its re-reads by frame timestamp (K-184) — so between
    /// two menu gestures in one test the bar would otherwise draw from the
    /// document as it stood before the first. The application never sees this:
    /// its frames really do advance, and the engine's change stream refreshes
    /// the model as well.
    Future<void> settle(WidgetTester tester, dynamic p) async {
      (p.uiState as LumitUiState).model.refresh();
      (p.state as LumitState).notifyDocumentChanged();
      await tester.pump();
    }

    Future<void> dismiss(WidgetTester tester) async {
      await tester.tapAt(const Offset(500, 800));
      await tester.pump();
    }

    double staticOf(BridgeScalar scalar) =>
        scalar is BridgeScalar_Static ? scalar.field0 : double.nan;

    testWidgets('Layer ▸ Transform ▸ Reset puts every property back',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
          prop: BridgeTransformProp.positionX,
          value: const BridgeScalar.static_(400));
      layer.setTransform(
          prop: BridgeTransformProp.scaleX,
          value: const BridgeScalar.static_(37));
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Layer', 'Reset', under: 'Transform');

      final after = layer.getTransform();
      expect(staticOf(after.positionX), 0);
      expect(staticOf(after.scaleX), 100,
          reason: 'a fresh layer is at full size, not at nothing');
      expect(staticOf(after.opacity), 100);
    });

    /// K-523: a row invoked on a selection runs on every layer in it.
    testWidgets('Layer ▸ Transform ▸ Flip horizontally flips all of them',
        (tester) async {
      final p = withComp();
      final a = p.comp.addSolidLayer();
      final b = p.comp.addSolidLayer();
      p.uiState.setSelection([a, b]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Layer', 'Flip horizontally', under: 'Transform');

      expect(staticOf(a.getTransform().scaleX), -100);
      expect(staticOf(b.getTransform().scaleX), -100);
      expect(staticOf(a.getTransform().scaleY), 100,
          reason: 'a horizontal flip leaves the other axis alone');
    });

    testWidgets('Layer ▸ Transform ▸ Centre in view puts it in the middle',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Layer', 'Centre in view', under: 'Transform');

      final settings = p.comp.getSettings();
      expect(staticOf(layer.getTransform().positionX), settings.width / 2);
      expect(staticOf(layer.getTransform().positionY), settings.height / 2);
    });

    testWidgets('Layer ▸ Transform separates a pair and offers to combine it',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Layer', 'Separate Position axes',
          under: 'Transform');
      expect(layer.getInfo().axisModes.position, BridgeAxisMode.separated);

      // The row now says the opposite, because it says what pressing it does.
      await settle(tester, p);
      await choose(tester, 'Layer', 'Combine Position axes', under: 'Transform');
      expect(layer.getInfo().axisModes.position, BridgeAxisMode.combined);
    });

    testWidgets('Layer ▸ 3D layer turns the switch on for every selected layer',
        (tester) async {
      final p = withComp();
      final a = p.comp.addSolidLayer();
      final b = p.comp.addSolidLayer();
      p.uiState.setSelection([a, b]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Layer', '3D layer');

      expect(a.getSwitches().threeD, isTrue);
      expect(b.getSwitches().threeD, isTrue);
    });

    testWidgets('Layer ▸ Blending mode sets the mode, and the steps walk it',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      final modes = listBlendModes();
      await choose(tester, 'Layer', modes[2], under: 'Blending mode');
      expect(layer.getBlend(), 2);

      await settle(tester, p);
      await choose(tester, 'Layer', 'Next blending mode');
      expect(layer.getBlend(), 3);

      await settle(tester, p);
      await choose(tester, 'Layer', 'Previous blending mode');
      expect(layer.getBlend(), 2);
    });

    testWidgets('Layer ▸ Matte gates with the layer above, and takes it off',
        (tester) async {
      final p = withComp();
      // The second solid lands on top, so it is the one the row means.
      final under = p.comp.addSolidLayer();
      p.comp.addSolidLayer();
      p.uiState.setSelection([under]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Layer', 'Luma inverted matte', under: 'Matte');
      final matte = under.getMatte();
      expect(matte, isNotNull);
      expect(matte!.luma, isTrue);
      expect(matte.inverted, isTrue);

      await settle(tester, p);
      await choose(tester, 'Layer', 'No matte', under: 'Matte');
      expect(under.getMatte(), isNull);
    });

    testWidgets('Layer ▸ Markers marks the layer and clears it again',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.playheadFrame.value = 12;
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Layer', 'Add at playhead', under: 'Markers');
      expect(layer.getMarkers().length, 1);

      await settle(tester, p);
      await choose(tester, 'Layer', 'Delete all markers', under: 'Markers');
      expect(layer.getMarkers(), isEmpty);
    });

    /// The Mask submenu means the mask whose row the Timeline has picked, so
    /// with none picked every row in it is dead.
    testWidgets('Layer ▸ Mask is dead until a mask row is picked',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);
      await open(tester, 'Layer', under: 'Mask');
      expect(tester.widget<Text>(find.text('Subtract')).style?.color,
          t.textDisabled);
      await dismiss(tester);
    });

    testWidgets('Layer ▸ Flow is dead on a kind with no frames to interpolate',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);
      await open(tester, 'Layer');
      expect(tester.widget<Text>(find.text('Flow')).style?.color,
          t.textDisabled,
          reason: 'a solid has no source frames to make in-betweens from');
      await dismiss(tester);
    });

    testWidgets('Animation ▸ Set keyframe plants one on the picked row',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      // A keyed property: Set keyframe adds to a curve, it does not start one
      // (K-447 — the stopwatch is the whole model).
      layer.setTransform(
        prop: BridgeTransformProp.positionX,
        value: BridgeScalar.keyframed([
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 0),
            value: 0,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 20),
            value: 100,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ]),
      );
      p.uiState.setSelection([layer]);
      p.uiState.selectedProperties.value = [
        '${layer.internallayerId}/transform/positionX',
      ];
      p.uiState.playheadFrame.value = 10;
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Animation', 'Set keyframe');

      final after = layer.getTransform().positionX;
      expect(after, isA<BridgeScalar_Keyframed>());
      expect((after as BridgeScalar_Keyframed).field0.length, 3,
          reason: 'the playhead sat between the two, so a third lands there');
    });

    testWidgets('Animation ▸ Toggle hold keyframe holds the key at the playhead',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.positionX,
        value: BridgeScalar.keyframed([
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 5),
            value: 0,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 25),
            value: 100,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ]),
      );
      p.uiState.setSelection([layer]);
      p.uiState.selectedProperties.value = [
        '${layer.internallayerId}/transform/positionX',
      ];
      p.uiState.playheadFrame.value = 5;
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Animation', 'Toggle hold keyframe');

      final keys =
          (layer.getTransform().positionX as BridgeScalar_Keyframed).field0;
      expect(keys.first.interpOut, isA<BridgeSideInterp_Hold>());
      expect(keys.last.interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'only the key under the playhead was asked about');

      // And back again — the row is one key, not two.
      await settle(tester, p);
      await choose(tester, 'Animation', 'Toggle hold keyframe');
      expect(
        (layer.getTransform().positionX as BridgeScalar_Keyframed)
            .field0
            .first
            .interpOut,
        isA<BridgeSideInterp_Linear>(),
      );
    });

    testWidgets('the keyframe dialogues are dead with no key at the playhead',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);
      await open(tester, 'Animation');
      for (final row in ['Keyframe interpolation…', 'Keyframe speed…']) {
        expect(tester.widget<Text>(find.text(row)).style?.color, t.textDisabled,
            reason: '$row has nothing to act on');
      }
      await dismiss(tester);
    });

    testWidgets('Animation ▸ Keyframe interpolation… writes both sides',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.positionX,
        value: BridgeScalar.keyframed([
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 0),
            value: 0,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 20),
            value: 100,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ]),
      );
      p.uiState.setSelection([layer]);
      p.uiState.selectedProperties.value = [
        '${layer.internallayerId}/transform/positionX',
      ];
      p.uiState.playheadFrame.value = 0;
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Animation', 'Keyframe interpolation…');
      await tester.pumpAndSettle();
      // The Out side becomes a hold; the In side is left as it opened.
      await tester.tap(find.byKey(const ValueKey('key-interp-out')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Hold').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('keyframe-confirm')));
      await tester.pumpAndSettle();

      final keys =
          (layer.getTransform().positionX as BridgeScalar_Keyframed).field0;
      expect(keys.first.interpOut, isA<BridgeSideInterp_Hold>());
    });

    testWidgets('Animation ▸ Animate text gives a Type layer an animator',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addTextLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      expect(layer.getText()!.animators, isEmpty);
      await choose(tester, 'Animation', 'Animate text');
      expect(layer.getText()!.animators.length, 1);
    });

    testWidgets('Animation ▸ Animate text is dead on a layer with no words',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);
      await open(tester, 'Animation');
      expect(tester.widget<Text>(find.text('Animate text')).style?.color,
          t.textDisabled);
      await dismiss(tester);
    });

    testWidgets('Animation ▸ Add expression puts one on the picked row',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.selectedProperties.value = [
        '${layer.internallayerId}/transform/positionX',
      ];
      p.uiState.model.refresh();
      await mount(tester, p);

      await choose(tester, 'Animation', 'Add expression');
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('expression-text')), 'time * 2');
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('expression-confirm')));
      await tester.pumpAndSettle();

      final after = layer.getTransform().positionX;
      expect(after, isA<BridgeScalar_Expression>());
      expect((after as BridgeScalar_Expression).field0, 'time * 2');
    });

    testWidgets('File ▸ Close project leaves an empty one in its place',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);
      final was = p.state.project!.internalid;

      await choose(tester, 'File', 'Close project');

      expect(p.state.project, isNotNull,
          reason: 'the shell always has a document');
      expect(p.state.project!.internalid, isNot(was),
          reason: 'the one that was open has gone');
      expect(p.state.project!.getItems(), isEmpty);
    });
  });
}
