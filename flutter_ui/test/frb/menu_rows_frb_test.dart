// The Layer and Animation menu rows that stopped being "(Not implemented)",
// tested against the real engine.
//
// Each of these is a second door onto a call the Timeline already makes, so
// what is worth asserting is not the call — it is that the row reaches it, that
// it reaches it for *every* selected layer, and that a row whose
// precondition is missing greys out rather than failing when pressed.
//
// The bar is mounted the way menu_bar_frb_test mounts it, because the enablement
// of half these rows is about the selection, and the selection lives in a
// notifier the bar does not subscribe to itself.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/menu_animation_frb.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/state/file_dialogs.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/l10n/strings.dart';
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

    /// Re-read the model and rebuild the bar.
    ///
    /// `tester.pump()` with no duration does not move the fake clock, and the
    /// read model groups its re-reads by frame timestamp — so between
    /// two menu gestures in one test the bar would otherwise draw from the
    /// document as it stood before the first. The application never sees this:
    /// its frames really do advance, and the engine's change stream refreshes
    /// the model as well.
    Future<void> settle(WidgetTester tester, dynamic p) async {
      (p.uiState as LumitUiState).model.refresh();
      (p.state as LumitState).notifyDocumentChanged();
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

    /// A row invoked on a selection runs on every layer in it.
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

    testWidgets('Animation ▸ Set keyframe plants one on the picked row',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      // A keyed property, with the playhead between its two keys.
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
      expect(after.field1, BridgeExpressionLanguage.rhai,
          reason: 'a new expression starts in Rhai until Settings says not');
    });

    /// The language is the dropdown's to say, never the text's: the same
    /// line is written once as JavaScript and runs as JavaScript. The
    /// default a new expression opens in is the setting's.
    testWidgets('Add expression writes the language its dropdown names',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.selectedProperties.value = [
        '${layer.internallayerId}/transform/positionX',
      ];
      p.uiState.model.refresh();
      await mount(tester, p);
      final picker = find.byKey(const ValueKey('expression-language'));

      await choose(tester, 'Animation', 'Add expression');
      await tester.pumpAndSettle();
      expect(find.descendant(of: picker, matching: find.text('Rhai')),
          findsOneWidget);
      await tester.enterText(
          find.byKey(const ValueKey('expression-text')), 'Math.round(7 / 2)');
      await tester.tap(picker);
      await tester.pumpAndSettle();
      await tester.tap(find.text('JavaScript').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('expression-confirm')));
      await tester.pumpAndSettle();

      final after = layer.getTransform().positionX as BridgeScalar_Expression;
      expect(after.field0, 'Math.round(7 / 2)');
      expect(after.field1, BridgeExpressionLanguage.javaScript);
      expect(
          sampleScalarWithContext(
              scalar: after,
              time: const BridgeRational(num: 0, den: 1),
              layer: layer),
          4,
          reason: 'JavaScript halves 7 to 3.5; Rhai has no Math at all');

      // Reopened on a row that has one, the dialogue shows that row's own.
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      await choose(tester, 'Animation', 'Add expression');
      await tester.pumpAndSettle();
      expect(find.descendant(of: picker, matching: find.text('JavaScript')),
          findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('expression-cancel')));
      await tester.pumpAndSettle();

      // A saved expression keeps its language, through a file and back.
      final ws = p.uiState.workspace;
      ws.saveExpression(
          'Wobble', 'wiggle(2, 30)', BridgeExpressionLanguage.javaScript);
      ws.saveExpression('Spin', 'time * 90');
      final file = ws.encodeExpressions();
      ws.deleteExpression('Wobble');
      expect(ws.importExpressions(file), 2);
      expect(ws.savedExpressionLanguage('Wobble'),
          BridgeExpressionLanguage.javaScript);
      expect(ws.savedExpressionLanguage('Spin'), BridgeExpressionLanguage.rhai);
      // The same name and text in another language is another expression.
      ws.saveExpression('Wobble', 'wiggle(2, 30)');
      ws.importExpressions(file);
      expect(ws.savedExpressionLanguage('Wobble'), BridgeExpressionLanguage.rhai);
      expect(ws.savedExpressionLanguage('Wobble 2'),
          BridgeExpressionLanguage.javaScript);
      for (final name in ['Wobble', 'Wobble 2', 'Spin']) {
        ws.deleteExpression(name);
      }
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

    /// The two preset rows are a second door onto the `.lumfx` the Effects &
    /// presets panel writes and reads, so what is worth asserting is
    /// that the menu reaches the same document — and that Apply reaches every
    /// selected layer.
    group('Animation ▸ preset rows', () {
      late Directory dir;

      setUp(() {
        dir = Directory.systemTemp.createTempSync('lumit-menu-preset');
      });
      tearDown(() {
        animationPresetSavePicker = (suggested) => pickPresetSaveLocation(
            suggested,
            initialDirectory: presetsDirPath());
        animationPresetOpenPicker = pickPresetToOpen;
        try {
          dir.deleteSync(recursive: true);
        } catch (_) {}
      });

      testWidgets('Save writes the selected layer\'s stack as a .lumfx',
          (tester) async {
        final p = withComp();
        final layer = p.comp.addSolidLayer();
        layer.addEffect(name: 'blur');
        p.uiState.setSelection([layer]);
        p.uiState.model.refresh();
        await mount(tester, p);

        final path = '${dir.path}/Soft edges.lumfx';
        animationPresetSavePicker = (_) async => path;

        await choose(tester, 'Animation', l10n.menuSaveAnimationPreset);
        await tester.pump();

        final written = File(path);
        expect(written.existsSync(), isTrue);
        final text = written.readAsStringSync();
        expect(text, contains('blur'));
        expect(text, contains('Soft edges'),
            reason:
                'the preset is named after its file, as the panel names it');
      });

      testWidgets('Apply lands the preset on the primary layer only',
          (tester) async {
        final p = withComp();
        final source = p.comp.addSolidLayer();
        source.addEffect(name: 'blur');
        final path = '${dir.path}/one.lumfx';
        File(path).writeAsStringSync(source.savePreset(name: 'one'));

        final a = p.comp.addSolidLayer();
        final b = p.comp.addSolidLayer();
        p.uiState.setSelection([a, b]);
        p.uiState.model.refresh();
        await mount(tester, p);

        animationPresetOpenPicker = () async => path;

        await choose(tester, 'Animation', l10n.menuApplyAnimationPreset);
        // The file is read off the interface's thread.
        await settleFrb(tester, until: () => a.getEffects().isNotEmpty);

        expect(a.getEffects().length, 1);
        expect(b.getEffects(), isEmpty,
            reason: 'the primary layer alone, not every selected layer');
      });
    });
  });
}
