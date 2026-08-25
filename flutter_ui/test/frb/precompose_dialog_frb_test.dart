// The Pre-compose dialogue (docs/07 §13.4).
//
// The dialogue's job is to turn two questions into the four arguments the
// engine call takes, so what is worth testing is that each answer reaches the
// document: attributes left behind or moved, the new comp trimmed to the
// selection or as long as this one, and the remembered answers coming back the
// next time it opens. The engine's own behaviour is covered on the Rust side;
// these drive the real bridge, so a wrong argument shows up as a wrong
// document rather than a passing mock.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/precompose_dialog_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  /// Open the dialogue over a comp with `layers` selected, and hand back
  /// everything the assertions need.
  Future<
      ({
        CompositionReference comp,
        List<LayerReference> layers,
        Workspace workspace,
      })> open(
    WidgetTester tester, {
    int layerCount = 1,
  }) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    for (var i = 0; i < layerCount; i++) {
      comp.addSolidLayer();
    }
    final layers = comp.getLayers();
    p.uiState.setSelection(layers);

    await tester.pumpWidget(hostPanel(
      child: Builder(
        builder: (context) => GestureDetector(
          key: const ValueKey('open'),
          behavior: HitTestBehavior.opaque,
          onTap: () => showPrecomposeDialogFrb(
            context: context,
            comp: comp,
            selectedLayers: layers,
            ui: p.uiState,
            workspace: p.uiState.workspace,
          ),
          child: const SizedBox(width: 200, height: 40),
        ),
      ),
      state: p.state,
      uiState: p.uiState,
    ));
    await tester.tap(find.byKey(const ValueKey('open')));
    await tester.pumpAndSettle();
    return (comp: comp, layers: layers, workspace: p.uiState.workspace);
  }

  /// The composition a Precomp layer draws from.
  CompositionReference sourceComp(LayerReference layer) {
    final item = layer.getSourceItem();
    if (item case ItemReference_Composition(:final field0)) return field0;
    throw StateError('a Precomp layer draws from a composition');
  }

  /// Pre-compose is the dialogue's default action (K-243): it takes focus when
  /// the window opens, so `Enter` presses it without the pointer having to find
  /// it. It must also not reach the Timeline behind the window, which is where
  /// `Enter` renames the selected layer.
  testWidgets('Enter presses Pre-compose', (tester) async {
    final it = await open(tester);
    final before = it.layers.single.getName();

    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    final after = it.comp.getLayers();
    expect(after.length, 1, reason: 'the layer was packed into a new comp');
    expect(sourceComp(after.single).getLayers().single.getName(), before);
    expect(find.byKey(const ValueKey('precompose-confirm')), findsNothing,
        reason: 'and the dialogue closed');
  });

  testWidgets('moving the attributes packs the layers whole', (tester) async {
    final it = await open(tester, layerCount: 2);

    await tester.tap(find.byKey(const ValueKey('precompose-move')));
    await tester.enterText(
        find.byKey(const ValueKey('precompose-name')), 'Packed');
    await tester.tap(find.byKey(const ValueKey('precompose-confirm')));
    await tester.pumpAndSettle();

    final after = it.comp.getLayers();
    expect(after.length, 1, reason: 'both layers went into the new comp');
    expect(after.single.getName(), 'Packed');
    expect(sourceComp(after.single).getLayers().length, 2);
  });

  /// The choice is only offered for one layer, and it is the whole point of the
  /// option: the attributes stay on the layer left standing here.
  testWidgets('leaving the attributes keeps the effect on the Precomp layer',
      (tester) async {
    final it = await open(tester);
    it.layers.single.addEffect(name: 'blur');

    await tester.tap(find.byKey(const ValueKey('precompose-leave')));
    await tester.tap(find.byKey(const ValueKey('precompose-confirm')));
    await tester.pumpAndSettle();

    final precomp = it.comp.getLayers().single;
    expect(precomp.getEffects().length, 1, reason: 'the effect stayed behind');
    expect(sourceComp(precomp).getLayers().single.getEffects(), isEmpty,
        reason: 'and did not travel too, which would apply it twice');
  });

  testWidgets('a stack cannot leave its attributes behind', (tester) async {
    await open(tester, layerCount: 2);

    // Move is the answer, and the other choice is shown disabled rather than
    // hidden — pressing it does nothing at all.
    final leave = tester.widget<HouseRadio>(
        find.byKey(const ValueKey('precompose-leave')));
    expect(leave.enabled, isFalse);

    await tester.tap(find.byKey(const ValueKey('precompose-leave')));
    await tester.pumpAndSettle();
    final move = tester
        .widget<HouseRadio>(find.byKey(const ValueKey('precompose-move')));
    expect(move.selected, isTrue, reason: 'the choice did not move');
  });

  testWidgets('adjusting the duration trims the new comp to the selection',
      (tester) async {
    final it = await open(tester);
    // Two seconds of a thirty-second comp.
    it.layers.single.setSpan(
      span: BridgeSpan(
        inPoint: const BridgeRational(num: 0, den: 1),
        outPoint: const BridgeRational(num: 2, den: 1),
        startOffset: const BridgeRational(num: 0, den: 1),
      ),
    );

    // Adjust is on by default, so this is the plain confirm.
    await tester.tap(find.byKey(const ValueKey('precompose-confirm')));
    await tester.pumpAndSettle();

    final inner = sourceComp(it.comp.getLayers().single);
    expect(inner.durationFrames(), 120, reason: 'two seconds at 60 fps');
  });

  testWidgets('the answers are remembered for next time', (tester) async {
    final it = await open(tester);
    expect(it.workspace.precomposeOpenNewComp, isFalse);

    await tester.tap(find.byKey(const ValueKey('precompose-open-new-comp')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('precompose-confirm')));
    await tester.pumpAndSettle();

    expect(it.workspace.precomposeOpenNewComp, isTrue);
  });

  /// The name label sits on one line beside its field, and the duration
  /// checkbox is indented under the choices it qualifies rather than sitting
  /// flush with them.
  testWidgets('the name asks on one line and the duration sits under the '
      'choices', (tester) async {
    await open(tester);

    final label = find.text('New composition name');
    final labelBox = tester.getSize(label);
    final oneLine = tester.renderObject<RenderBox>(label).getMaxIntrinsicHeight(
          double.infinity,
        );
    expect(labelBox.height, oneLine, reason: 'the label does not wrap');

    final adjust = tester.getTopLeft(
        find.byKey(const ValueKey('precompose-adjust-duration')));
    final move =
        tester.getTopLeft(find.byKey(const ValueKey('precompose-move')));
    expect(adjust.dx, greaterThan(move.dx),
        reason: 'indented under the attribute choices');

    // A label that will not wrap can still overflow its row, which Flutter
    // reports as an exception rather than by drawing anything wrong.
    expect(tester.takeException(), isNull);
  });

  testWidgets('cancelling changes nothing', (tester) async {
    final it = await open(tester);

    await tester.tap(find.byKey(const ValueKey('precompose-cancel')));
    await tester.pumpAndSettle();

    expect(it.comp.getLayers().single.internallayerId,
        it.layers.single.internallayerId);
  });
}
