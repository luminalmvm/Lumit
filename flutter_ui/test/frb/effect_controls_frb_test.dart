// The Effect controls panel on frb, tested against the real engine.
//
// The panel that existed before this was a float-only sketch in panels_frb.dart
// with a `TODO: commit the value` where the commit should be, so there is
// nothing to migrate here — v0's own panel could only *edit* scalars and colours
// ("every other kind shows its value read-only… since the matching edit op is
// not in the bridge yet"), which this one improves on rather than matches.
//
// Every document operation is genuine; see frb_test_support.dart.

import 'package:flutter/gestures.dart' show kSecondaryButton;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/state/clipboard.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show effectLabelOf, EffectParamRowFrb;
import 'package:lumit_flutter/widgets/angle_dial.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:uuid/uuid.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Effect controls (frb)', () {
    /// A project with one comp, one layer in it, and that layer selected — the
    /// state the panel needs before it draws anything at all.
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    Future<void> mount(
      WidgetTester tester,
      ({LumitState state, LumitUiState uiState, LayerReference layer}) p, {
      // The Transform card is off by default (K-193); the rows it holds are
      // still this panel's to test, so the tests that want them ask for it
      // exactly as a user would.
      bool transform = true,
    }) async {
      p.uiState.workspace.interface.transformInEffectControls = transform;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
    }

    testWidgets('without a layer selected it says so rather than drawing empty',
        (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.textContaining('Select a composition'), findsOneWidget);

      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      await tester.pump();
      expect(find.textContaining('Select a layer'), findsOneWidget);
    });

    testWidgets('deselecting a layer keeps the last one on the panel',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);
      expect(find.textContaining('No effects'), findsOneWidget);

      p.uiState.selectedLayer.value = null;
      await tester.pump();
      // Still the same layer's stack: clicking away in the Timeline is not a
      // request to lose your place.
      expect(find.textContaining('Select a layer'), findsNothing);
      expect(find.textContaining('No effects'), findsOneWidget);
    });

    testWidgets('Add effect commits one, and it appears as a card',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(find.textContaining('No effects'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('fx-add')));
      await tester.pumpAndSettle();
      // The menu lists categories, each opening onto its effects by their
      // sentence-case label — the raw match_name never reaches the user
      // (K-194: Add effect → Blur & sharpen → Gaussian blur).
      expect(find.text('Gaussian blur'), findsNothing,
          reason: 'the effects wait behind their category');
      await tester.tap(find.byKey(const ValueKey('fx-category-blur_sharpen')));
      await tester.pumpAndSettle();
      expect(find.text('Gaussian blur'), findsOneWidget);
      await tester.tap(find.text('Gaussian blur'));
      await tester.pumpAndSettle();

      expect(p.layer.getEffects(), hasLength(1),
          reason: 'the menu reached the document');
      expect(find.text('Gaussian blur'), findsOneWidget,
          reason: 'the card is titled by label, not by match name');
      expect(find.text('Radius'), findsOneWidget,
          reason: 'a row per declared parameter, labelled from the schema');
    });

    testWidgets(
        'a null layer says its effects change no picture, and keeps their values',
        (tester) async {
      // K-274: effects on a null are ACCEPTED and labelled inert rather than
      // refused. A null draws nothing, so nothing here changes a picture — but
      // the parameters are real, animatable values, which is the whole point
      // of putting a control on a null.
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final nul = comp.addNullLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = nul;

      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-null-inert')), findsNothing,
          reason: 'nothing to say about a stack that is empty');

      nul.addEffect(name: 'blur');
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-null-inert')), findsOneWidget,
          reason: 'the drop is accepted, and the panel says what it does');

      // And the effect is genuinely on the layer, with a readable value — the
      // difference between "inert" and "refused". (That those values stay
      // live and animatable is pinned engine-side, where the commit is:
      // `an_effect_on_a_null_layer_keeps_its_animated_value`.)
      expect(nul.getEffects().length, 1);
      expect(find.text('Gaussian blur'), findsOneWidget,
          reason: 'the stack draws as it does on any other layer');
    });

    /// **Copying one effect** (K-275). The engine has taken one or a whole
    /// stack since copy/paste landed — `copy_effects(Some(id))` — and the Edit
    /// menu's Copy takes the *layer*, so until this row existed there was no
    /// way to pick a single effect and no way to reach the call.
    testWidgets('an effect heading copies that one effect', (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      // Not the Invert effect: since K-395 every effect draws an "Invert"
      // beside its Matte picker, so an effect NAMED Invert makes the heading
      // ambiguous to find by text. Nothing here is about which effect it is.
      p.layer.addEffect(name: 'vignette');
      await mount(tester, p);

      expect(p.uiState.clipboard.kind, isNull, reason: 'nothing copied yet');

      final second = p.layer.getEffects()[1];
      await tester.tapAt(
        tester.getCenter(find.text(effectLabelOf(second.name()))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(ValueKey<String>('fx-menu-copy-${second.id()}')));
      await tester.pumpAndSettle();

      expect(p.uiState.clipboard.kind, ClipboardKind.effects,
          reason: 'it goes on the same clipboard a whole stack does — both are '
              '.lumfx, so Paste needs no idea which it holds');
      // And it is *one* effect, not the stack: pasting onto a bare layer adds
      // exactly one.
      final bare = p.uiState.selectedComp!.addSolidLayer();
      bare.pasteEffects(
        text: p.uiState.clipboard.text!,
        atFrame: 0,
      );
      expect(bare.getEffects(), hasLength(1),
          reason: 'the picked effect alone, not the two on the layer');
      expect(bare.getEffects().single.name(), second.name());
    });

    /// **An effect's name picks it** (K-300). Clicking a heading only twirled
    /// it before, so an effect could not be selected here at all — and Copy,
    /// which acts on the selection, had nothing to take but the whole layer.
    /// Shift takes the run between, the way it does in every other list here.
    testWidgets('clicking an effect name picks it, and Shift takes the run',
        (tester) async {
      final p = withLayer();
      for (final name in ['blur', 'invert', 'vignette']) {
        p.layer.addEffect(name: name);
      }
      await mount(tester, p);
      final stack = p.layer.getEffects();

      await tester.tap(find.text(effectLabelOf(stack.first.name())));
      await tester.pumpAndSettle();
      expect(p.uiState.selectedEffects.value, [stack.first.id()]);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tap(find.text(effectLabelOf(stack[2].name())));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pumpAndSettle();
      expect(p.uiState.selectedEffects.value, [for (final e in stack) e.id()],
          reason: 'Shift extended the pick down the stack, in stack order');

      // And that is what Copy takes: three effects, one .lumfx document.
      expect(copySelectionFrb(p.uiState), isTrue);
      expect(p.uiState.clipboard.kind, ClipboardKind.effects);
      final bare = p.uiState.selectedComp!.addSolidLayer();
      bare.pasteEffects(text: p.uiState.clipboard.text!, atFrame: 0);
      expect(bare.getEffects(), hasLength(3));
    });

    testWidgets('a selection made in the Viewer switches the panel to it',
        (tester) async {
      // The Viewer picks a layer by calling `setSelection` on the shell — it
      // never goes through the Timeline — so this panel must follow the shell,
      // not the panel that happens to be next to it (K-275).
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final other = p.uiState.selectedComp!.addSolidLayer();
      // Vignette, not Invert: every matte row draws the word "Invert" since
      // K-395, so the effect of that name is no longer a unique bit of text.
      other.addEffect(name: 'vignette');
      await mount(tester, p);
      expect(find.text('Gaussian blur'), findsOneWidget);

      p.uiState.setSelection([other]);
      await tester.pump();

      expect(find.text('Vignette'), findsOneWidget,
          reason: "the panel shows the newly selected layer's stack");
      expect(find.text('Gaussian blur'), findsNothing,
          reason: 'and not the one it was showing before');
    });

    testWidgets('a parameter edit commits, and reading it back is exact',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p);

      // By key, not `.first`: the Transform card is drawn above the stack, so
      // the first DragValueField on screen is an anchor-point cell.
      final id = p.layer.getEffects().single.id();
      await tester.tap(find.byKey(ValueKey<String>('fx-float-$id-radius')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText).first, '12.5');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      final radius = p.layer.getEffects().single.getValue(id: 'radius');
      expect(
        radius,
        isA<BridgeEffectValue_Float>().having(
          (v) => (v.field0 as BridgeScalar_Static).field0,
          'radius',
          12.5,
        ),
        reason: 'the typed value reached the document as a static scalar',
      );
    });

    /// **The drag regression.** A parameter could be typed into but not dragged:
    /// the panel held the stack of effect handles across the whole gesture, and
    /// a `BridgeEffectInstance` passed to `renderFrameWithPreview` is *moved* —
    /// frb disposes the Dart side of it — so the first preview tick killed the
    /// handles and every tick after it threw `DroppableDisposedException`. What
    /// is staged now is the edit, not the handles.
    testWidgets('a parameter can be dragged, not only typed into',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p);

      final id = p.layer.getEffects().single.id();
      double radius() => ((p.layer.getEffects().single.getValue(id: 'radius')
                  as BridgeEffectValue_Float)
              .field0 as BridgeScalar_Static)
          .field0;
      final before = radius();

      await tester.drag(
        find.byKey(ValueKey<String>('fx-float-$id-radius')),
        const Offset(60, 0),
      );
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull,
          reason: 'no handle was used after it had been handed to Rust');
      expect(radius(), greaterThan(before),
          reason: 'the drag reached the document');
    });

    testWidgets('the enable switch, reorder and remove all reach the document',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      p.layer.addEffect(name: 'sharpen');
      await mount(tester, p);

      final first = p.layer.getEffects().first;
      expect(first.enabled(), isTrue);

      await tester
          .tap(find.byKey(ValueKey<String>('fx-enabled-${first.id()}')));
      await tester.pump();
      expect(p.layer.getEffects().first.enabled(), isFalse,
          reason: 'bypassing an effect is a document edit, not a view state');

      // Reorder: right-click the second card's heading and move it up (K-276
      // put the two arrows' rare job in a menu and gave their space to the
      // render time, which is read constantly).
      final before = p.layer.getEffects().map((e) => e.name()).toList();
      final second = p.layer.getEffects()[1];
      await tester.tapAt(
        tester.getCenter(find.text(effectLabelOf(second.name()))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(ValueKey<String>('fx-menu-up-${second.id()}')));
      await tester.pumpAndSettle();
      expect(p.layer.getEffects().map((e) => e.name()).toList(),
          before.reversed.toList());

      // Remove: the stack shortens by exactly one.
      final top = p.layer.getEffects().first;
      await tester.tap(find.byKey(ValueKey<String>('fx-remove-${top.id()}')));
      await tester.pump();
      expect(p.layer.getEffects(), hasLength(1));
    });

    /// Dragging an effect's name to another effect's name moves it there — the
    /// gesture the owner asked for and the one every other list in the
    /// application already uses (docs/07 §6).
    testWidgets('an effect is reordered by dragging its heading',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      p.layer.addEffect(name: 'sharpen');
      await mount(tester, p);

      final before = p.layer.getEffects().map((e) => e.name()).toList();
      expect(before, ['blur', 'sharpen']);

      // The second heading onto the first: sharpen takes blur's place.
      final from = find.text(effectLabelOf('sharpen'));
      final onto = find.text(effectLabelOf('blur'));
      final drag = await tester.startGesture(tester.getCenter(from));
      // Past the drag threshold in steps, so the Draggable starts and the
      // target under the pointer is entered before the release.
      await tester.pump(const Duration(milliseconds: 20));
      await drag.moveTo(tester.getCenter(onto));
      await tester.pump(const Duration(milliseconds: 20));
      await drag.up();
      await tester.pumpAndSettle();

      expect(p.layer.getEffects().map((e) => e.name()).toList(),
          ['sharpen', 'blur']);
    });

    testWidgets('the top card is offered no way up, and the bottom none down',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      p.layer.addEffect(name: 'sharpen');
      await mount(tester, p);

      final effects = p.layer.getEffects();

      // The topmost effect's menu offers the moves it can make and not the
      // ones it cannot — a dead row tells you what you cannot do, which is not
      // what a menu is for.
      await tester.tapAt(
        tester.getCenter(find.text(effectLabelOf(effects[0].name()))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      expect(find.byKey(ValueKey<String>('fx-menu-up-${effects[0].id()}')),
          findsNothing);
      expect(find.byKey(ValueKey<String>('fx-menu-top-${effects[0].id()}')),
          findsNothing);
      expect(find.byKey(ValueKey<String>('fx-menu-down-${effects[0].id()}')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-menu-bottom-${effects[0].id()}')),
          findsOneWidget);
    });

    testWidgets('an effect twirls shut, and its rows go with it',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      expect(find.text('Radius'), findsOneWidget,
          reason: 'a newly applied effect arrives open');

      // **Only the twirl folds it** (K-300). The name picks the effect, and a
      // click that also collapsed the card took the parameters away at the
      // moment you said which effect you meant.
      await tester.tap(find.text('Gaussian blur'));
      await tester.pump();
      expect(find.text('Radius'), findsOneWidget,
          reason: 'picking an effect does not shut it');

      await tester.tap(find.byKey(ValueKey<String>('fx-twirl-$id')));
      await tester.pump();
      expect(find.text('Radius'), findsNothing);
      expect(find.byKey(ValueKey<String>('fx-enabled-$id')), findsOneWidget,
          reason: 'a shut effect still shows its heading and its switch');

      await tester.tap(find.byKey(ValueKey<String>('fx-twirl-$id')));
      await tester.pump();
      expect(find.text('Radius'), findsOneWidget);
    });

    testWidgets('Reset puts every parameter back and drops its keyframes',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final before = p.layer.getEffects().single.getValue(id: 'radius');
      await mount(tester, p, transform: false);

      // Animate it and move it away from its default, so Reset has both a
      // changed value and a curve to undo.
      final id = p.layer.getEffects().single.id();
      final stack = p.layer.getEffects();
      stack.single.setValue(
        id: 'radius',
        value: BridgeEffectValue.float(BridgeScalar.keyframed([
          BridgeKeyframe(
            time: const BridgeRational(num: 0, den: 1),
            value: 40,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ])),
      );
      p.layer.setEffects(effects: stack);
      p.uiState.model.refresh();
      await tester.pump();

      await tester.tap(find.byKey(ValueKey<String>('fx-reset-$id')));
      await tester.pump();

      expect(p.layer.getEffects().single.getValue(id: 'radius'), before,
          reason: 'the schema default is written back, curve and all');
    });

    testWidgets(
        'the Transform rows draw every property and commit one at a time',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(find.text('Transform'), findsOneWidget);
      for (final row in [
        'Anchor point',
        'Position',
        'Scale',
        'Rotation',
        'Opacity'
      ]) {
        expect(find.text(row), findsOneWidget, reason: row);
      }

      final before = p.layer.getTransform();
      await tester.tap(find.byKey(const ValueKey('tf-opacity')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText).first, '40');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      final after = p.layer.getTransform();
      expect((after.opacity as BridgeScalar_Static).field0, 40);
      expect(after.positionX, before.positionX,
          reason: 'one property per op — nothing else moved');
    });

    /// **The stale-value regression.** A row only ever changed when it wrote
    /// the value itself. So an undo moved the picture and left the number
    /// behind, and the same property edited in the Timeline's fold-out never
    /// reached this panel — one miss, two symptoms: nothing here listened to
    /// the engine. Fails without the read model's change subscription and its
    /// revision check (K-184).
    testWidgets('an edit made elsewhere, and an undo, both reach the rows',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);
      expect(find.text('100%'), findsOneWidget, reason: 'opacity as it starts');

      // What the Timeline's fold-out does when the same row is dragged there.
      p.layer.setTransform(
          prop: BridgeTransformProp.opacity, value: BridgeScalar.static_(40));
      await settleFrb(tester,
          until: () => find.text('40%').evaluate().isNotEmpty);
      expect(find.text('40%'), findsOneWidget,
          reason: 'an edit made in the other panel shows here');

      p.state.project!.undo();
      await settleFrb(tester,
          until: () => find.text('100%').evaluate().isNotEmpty);
      expect(find.text('100%'), findsOneWidget,
          reason: 'undo puts the number back, not only the picture');
    });

    /// A 2D layer showing 3D controls that cannot do anything is worse than not
    /// showing them, so the z and x/y-rotation rows are gated on the switch.
    testWidgets('the 3D rows appear only on a 3D layer', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(find.text('Rotation x'), findsNothing);
      expect(find.text('Rotation y'), findsNothing);
      // Position draws two cells, not three, when the layer is flat.
      expect(find.byKey(const ValueKey('tf-positionZ')), findsNothing);
      expect(find.byKey(const ValueKey('tf-positionX')), findsOneWidget);
      expect(find.byKey(const ValueKey('tf-positionY')), findsOneWidget);
    });

    /// A camera is 3D by construction whatever its switch says (K-023): it
    /// positions in z and looks somewhere, so hiding its z and rotation rows
    /// would gate away the only controls that mean anything on it. The rule
    /// used to live in an engine reader nothing called; now the panel decides
    /// it from the model, and this is what pins that a camera never lost it.
    testWidgets('a camera gets its 3D rows without its switch', (tester) async {
      final p = withLayer();
      final comp = p.uiState.selectedComp!;
      final camera = comp.addCameraLayer();
      p.uiState.selectedLayer.value = camera;
      await mount(tester, p);

      expect(find.text('Rotation x'), findsOneWidget);
      expect(find.text('Rotation y'), findsOneWidget);
      expect(find.byKey(const ValueKey('tf-positionZ')), findsOneWidget);
    });

    /// An animated parameter stays a field (docs/07 §4.3): editing it writes
    /// the key under the playhead — never a static value over the curve,
    /// which would delete every key in one step that looks like nudging a
    /// number.
    testWidgets('editing an animated parameter edits the key, not the curve',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');

      final staged = p.layer.getEffects();
      staged.single.setValue(
        id: 'radius',
        value: BridgeEffectValue.float(BridgeScalar.keyframed([
          BridgeKeyframe(
            time: const BridgeRational(num: 0, den: 1),
            value: 4,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
          BridgeKeyframe(
            time: const BridgeRational(num: 1, den: 1),
            value: 40,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ])),
      );
      p.layer.setEffects(effects: staged);

      await mount(tester, p);

      final id = p.layer.getEffects().single.id();
      final field = find.byKey(ValueKey<String>('fx-float-$id-radius'));
      expect(field, findsOneWidget,
          reason: 'an animated parameter keeps its field');

      // The playhead sits on the first key: the drag edits that key.
      await tester.drag(field, const Offset(40, 0));
      await tester.pumpAndSettle();

      final after = p.layer.getEffects().single.getValue(id: 'radius');
      final scalar = (after as BridgeEffectValue_Float).field0;
      expect(scalar, isA<BridgeScalar_Keyframed>(),
          reason: 'the curve survives the edit');
      final keys = (scalar as BridgeScalar_Keyframed).field0;
      expect(keys, hasLength(2), reason: 'no key added or lost at a key');
      expect(keys.first.value, greaterThan(4),
          reason: 'the edit landed in the key under the playhead');
      expect(keys.last.value, 40, reason: 'the other key is untouched');
    });
    testWidgets(
        'the lens flare panel folds: point pair, groups, conditional matte rows',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'lens_flare');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      // The light x/y pair is ONE row (docs/07 SS6.1) with a shared stem
      // label, not two rows.
      expect(
        find.byWidgetPredicate((w) {
          final key = w.key;
          return key is ValueKey<String> &&
              key.value.startsWith('fx-row-') &&
              key.value.endsWith('-light_x-pair');
        }),
        findsOneWidget,
      );
      expect(find.text('Light'), findsOneWidget);
      expect(find.text('Light y'), findsNothing);

      // The collapsed groups show their headers, not their members.
      expect(find.text('Lens options'), findsOneWidget);
      expect(find.text('Flare options'), findsOneWidget);
      expect(find.text('Blades'), findsNothing);

      // Twirling Lens options open reveals the Int-kind Blades row.
      await tester.tap(find.text('Lens options'));
      await tester.pump();
      expect(find.text('Blades'), findsOneWidget);

      // The matte rows are hidden while Source is Manual...
      // ("Matte" is this row's label since K-395 — the uniform word. In Manual
      // the Source dropdown reads "Manual light", so nothing says it at all.)
      expect(find.text('Matte'), findsNothing);
      expect(find.text('Threshold'), findsNothing);

      // ...and appear when Source type switches to Matte.
      final effects = p.layer.getEffects();
      final fx = effects.single;
      fx.setValue(id: 'source_type', value: const BridgeEffectValue.choice(1));
      p.layer.setEffects(effects: effects);
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.text('Matte'), findsNWidgets(2),
          reason: 'the row label, and the Source dropdown now reading Matte');
      expect(find.text('Threshold'), findsOneWidget);
      expect(find.text('Threshold softness'), findsOneWidget);

      // The Matte starts pointed at the layer the effect is ON
      // (K-288), and the picker says so. Before this it defaulted to None
      // and the effect sat there detecting nothing until you went hunting
      // for another layer — which on an adjustment layer, whose only
      // picture is the composite below, was always the wrong one.
      expect(find.textContaining('(this layer)'), findsOneWidget);

      // Light tint is a source-mode-independent row (K-259); Use source
      // colour appears with Matte and would with Lights.
      expect(find.text('Light tint'), findsOneWidget);
      expect(find.text('Use source colour'), findsOneWidget);

      // Back to Manual: the tint stays, the source-colour toggle and the
      // matte rows go.
      final again = p.layer.getEffects();
      again.single.setValue(
          id: 'source_type', value: const BridgeEffectValue.choice(0));
      p.layer.setEffects(effects: again);
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.text('Light tint'), findsOneWidget);
      expect(find.text('Use source colour'), findsNothing);
      expect(find.text('Matte'), findsNothing);
    });

    // Blend (K-289): the Transparent/Black Background pair became a blend
    // menu, defaulting to Add — the behaviour every flare already had.
    testWidgets('the lens flare offers a blend menu, defaulting to Add',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'lens_flare');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      expect(find.text('Background'), findsNothing,
          reason: 'the two-option Background choice is gone');
      expect(find.text('Blend'), findsOneWidget);
      expect(find.text('Add'), findsWidgets,
          reason: 'a fresh flare adds its light, as it always did');
    });

    // The Lens picker (K-262, curated K-264). Twenty entries sit well
    // under the searchable threshold, so the row is the PLAIN dropdown —
    // the searchable picker's laziness is pinned in
    // test/search_dropdown_test.dart against synthetic options. What the
    // panel owes here: the curated default shows, and the custom Lens file
    // row (K-264) is present for the prescriptions the palette leaves out.
    testWidgets('the lens picker shows the curated default and the file row',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'lens_flare');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      expect(find.byType(BareSearchDropdown), findsNothing,
          reason: 'twenty entries is a dropdown, not a search problem');
      expect(find.text('Zeiss · Arri Master Prime T1.3 50mm'), findsOneWidget,
          reason: 'the curated default is the reference cine prime');
      expect(find.text('Lens file'), findsOneWidget,
          reason: 'a user .lens file covers everything the palette leaves out');
    });

    /// **The uniform Matte row** (K-395). Every effect can be driven by a
    /// matte, and the way you say so is the same row everywhere: a layer
    /// picker with an **Invert** beside it, on ONE row. The effect under test
    /// is deliberately an arbitrary one — a plain Gaussian blur, which has no
    /// idea what a matte is — because the point of injecting the pair is that
    /// no effect had to be told.
    testWidgets('every effect gets a Matte row, and binding a layer sticks',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final other = p.uiState.selectedComp!.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      final picker = find.byKey(ValueKey<String>('fx-layer-$id-matte'));
      final invert = find.byKey(ValueKey<String>('fx-bool-$id-matte_invert'));
      expect(picker, findsOneWidget,
          reason: 'a blur declares no matte and gets one anyway');
      expect(find.text('Matte'), findsOneWidget);
      expect(find.text('Invert'), findsOneWidget);

      // ONE row, not two adjacent ones: the switch is drawn *inside* the
      // picker's row, and never gets a row of its own.
      expect(
        find.descendant(
          of: find.byKey(ValueKey<String>('fx-row-$id-matte')),
          matching: invert,
        ),
        findsOneWidget,
        reason: 'the Invert sits beside its picker, on the same row',
      );
      expect(
          find.byKey(ValueKey<String>('fx-row-$id-matte_invert')), findsNothing,
          reason: 'and so has no row of its own to sit on');

      // The switch writes, from where it now lives.
      await tester.tap(invert);
      await tester.pumpAndSettle();
      expect(
        p.layer.getEffects().single.getValue(id: 'matte_invert'),
        isA<BridgeEffectValue_Bool>().having((v) => v.field0, 'invert', isTrue),
        reason: 'ticking Invert reached the document',
      );

      // And the picker binds a layer, which reads back as that layer.
      await tester.tap(picker);
      await tester.pumpAndSettle();
      await tester.tap(find.text(other.getInfo().name).last);
      await tester.pumpAndSettle();
      expect(
        p.layer.getEffects().single.getValue(id: 'matte'),
        isA<BridgeEffectValue_Layer>()
            .having((v) => v.field0, 'matte', other.internallayerId),
        reason: 'the bound matte round-trips through the document',
      );
    });

    /// **The Matte row picks a channel and the Mix row a blend** (K-425). The
    /// engine injects `matte_channel` beside the matte pair and `blend` beside
    /// `mix`; the panel draws each on its parent's row, never on one of its own.
    testWidgets(
        'the Channel sits on the Matte row and the Blend on the Mix row',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      final channel =
          find.byKey(ValueKey<String>('fx-choice-$id-matte_channel'));
      final blend = find.byKey(ValueKey<String>('fx-choice-$id-blend'));
      expect(
        find.descendant(
            of: find.byKey(ValueKey<String>('fx-row-$id-matte')),
            matching: channel),
        findsOneWidget,
        reason: 'the Channel choice rides on the Matte row',
      );
      expect(find.byKey(ValueKey<String>('fx-row-$id-matte_channel')),
          findsNothing);
      expect(
        find.descendant(
            of: find.byKey(ValueKey<String>('fx-row-$id-mix')),
            matching: blend),
        findsOneWidget,
        reason: 'the Blend choice rides on the Mix row',
      );
      expect(find.byKey(ValueKey<String>('fx-row-$id-blend')), findsNothing);

      // A rider writes, from where it lives.
      await tester.tap(blend);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Add').last);
      await tester.pumpAndSettle();
      expect(
        p.layer.getEffects().single.getValue(id: 'blend'),
        isA<BridgeEffectValue_Choice>()
            .having((v) => v.field0, 'blend', isNot(0)),
        reason: 'picking a blend mode reached the document',
      );
    });

    /// An effect that picks its matte's channel itself takes no Channel rider.
    testWidgets('depth of field has no Channel on its Matte row',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'dof');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      expect(find.byKey(ValueKey<String>('fx-row-$id-depth')), findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-choice-$id-matte_channel')),
          findsNothing);
      expect(find.byKey(ValueKey<String>('fx-choice-$id-depth_channel')),
          findsNothing);
    });

    /// Depth of field owned the idea first, under its own ids (K-065 keeps
    /// them). K-395 gives it the shared row and the shared words: `depth` is
    /// labelled **Matte**, `depth_invert` is labelled **Invert**, and the two
    /// draw as one row rather than one at the top and one three twirls down.
    testWidgets('depth of field wears the same Matte row, ids unchanged',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'dof');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      expect(find.text('Matte'), findsOneWidget);
      expect(find.text('Depth layer'), findsNothing,
          reason: 'the private synonym is gone');
      expect(find.text('Depth invert'), findsNothing);
      expect(
        find.descendant(
          of: find.byKey(ValueKey<String>('fx-row-$id-depth')),
          matching: find.byKey(ValueKey<String>('fx-bool-$id-depth_invert')),
        ),
        findsOneWidget,
        reason: 'the same one-row treatment, under the same words',
      );

      await tester
          .tap(find.byKey(ValueKey<String>('fx-bool-$id-depth_invert')));
      await tester.pumpAndSettle();
      expect(
        p.layer.getEffects().single.getValue(id: 'depth_invert'),
        isA<BridgeEffectValue_Bool>().having((v) => v.field0, 'invert', isTrue),
        reason: 'the stored id did not move with the label',
      );
    });

    testWidgets('Enter renames the selected effect, and the name persists',
        (tester) async {
      // K-321: an effect instance can carry the user's own name. Enter on the
      // selected effect opens the heading's inline editor; the committed name
      // shows in place of the label and reaches the document.
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p);
      p.uiState.activePanel.value = Panel.effectControls;

      final stack = p.layer.getEffects();
      await tester.tap(find.text(effectLabelOf(stack.single.name())));
      await tester.pumpAndSettle();
      expect(p.uiState.selectedEffects.value, [stack.single.id()]);

      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-rename-field')), findsOneWidget,
          reason: 'Enter on the selected effect opens the inline rename');

      await tester.enterText(
          find.byKey(const ValueKey('fx-rename-field')), 'Blur the sign');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(find.text('Blur the sign'), findsOneWidget,
          reason: 'the heading shows the given name');
      expect(p.layer.getEffects().single.getInfo().customName, 'Blur the sign',
          reason: 'the name reached the document');

      // An empty rename clears back to the label.
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const ValueKey('fx-rename-field')), '');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(p.layer.getEffects().single.getInfo().customName, isNull);
      expect(find.text(effectLabelOf('blur')), findsOneWidget,
          reason: 'a cleared name falls back to the effect label');

      // Escape throws the edit away (K-323). Enter, clicking away and an
      // empty commit all *write*; without this there is no way out that does
      // not, and Escape fell through to a modal dismissal with no modal.
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('fx-rename-field')), 'Regretted');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-rename-field')), findsNothing,
          reason: 'Escape closes the rename editor');
      expect(p.layer.getEffects().single.getInfo().customName, isNull,
          reason: 'and writes nothing to the document');
    });

    // Depth of field's folded aperture (K-313): the twirls, the greyed rows and
    // the angle dial all arrive on the panel. This is the front half of the
    // fold — the back half (that the shipped defaults render the historical
    // disc bit for bit) is pinned in the engine tests.
    testWidgets('depth of field folds its aperture behind twirls, and greys',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'dof');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      // The three twirls show their headers, not their members.
      for (final label in ['Iris', 'Highlights', 'Depth map']) {
        expect(find.text(label), findsOneWidget);
      }
      expect(find.text('Roundness'), findsNothing,
          reason: 'the aperture arrives collapsed behind its twirl');

      // Twirling Iris open reveals the shape controls, the dial among them.
      await tester.tap(find.text('Iris'));
      await tester.pump();
      expect(find.text('Roundness'), findsOneWidget);
      expect(find.text('Blades'), findsOneWidget);
      expect(find.byType(AngleDial), findsOneWidget,
          reason: 'Rotation is a dial (docs/07 SS6), not a slider');

      // The focus point is one row over an _x/_y pair, with its own crosshair.
      await tester.tap(find.text('Depth map'));
      await tester.pump();
      expect(find.text('Focus point'), findsOneWidget);
      expect(find.text('Focus point y'), findsNothing);

      // Greying: no depth layer is picked, so everything that reads one is
      // disabled, while Focus distance — which does not — stays live.
      final greyed = tester
          .widgetList<EffectParamRowFrb>(find.byType(EffectParamRowFrb))
          .where((r) => !r.enabled)
          .map((r) => r.param.id)
          .toSet();
      expect(greyed, contains('depth_channel'));
      expect(greyed, contains('use_focus_point'));
      expect(greyed, contains('remove_edge_leak'));
      expect(greyed, isNot(contains('focus')));
      expect(greyed, isNot(contains('roundness')));
    });

    /// **The mask-path row** (K-408): one of *this layer's* masks, by name,
    /// with **First mask** as the unset entry.
    ///
    /// The row is mounted directly with a synthetic parameter rather than
    /// through one of the three built-ins that now declare one (Scribble,
    /// Stroke and Vegas's Mask/Path source, K-409), because what is under test
    /// is the **control**, not any effect: the entries it offers, the words it
    /// uses, and that picking one reaches the document as a `MaskPath` value
    /// rather than something else — against a real layer with real masks in a
    /// real document. Which built-ins declare the row is asserted engine-side,
    /// in `a_mask_path_row_declares_itself_and_defaults_to_the_first_mask`.
    testWidgets('a mask-path row lists this layer’s masks, First mask first',
        (tester) async {
      final p = withLayer();
      BridgeMask maskNamed(String name, double x) => BridgeMask(
            id: UuidValue.fromString(const Uuid().v4()),
            name: name,
            vertices: [
              BridgeVertex(
                  x: x, y: 0, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
              BridgeVertex(
                  x: x + 10,
                  y: 0,
                  tanInX: 0,
                  tanInY: 0,
                  tanOutX: 0,
                  tanOutY: 0),
              BridgeVertex(
                  x: x + 10,
                  y: 8,
                  tanInX: 0,
                  tanInY: 0,
                  tanOutX: 0,
                  tanOutY: 0),
            ],
            closed: true,
            inverted: false,
            opacity: const BridgeScalar.static_(100),
            mode: BridgeMaskMode.add,
            feather: const BridgeScalar.static_(0),
            expansion: const BridgeScalar.static_(0),
            pathKeys: const [],
          );
      p.layer.addMask(mask: maskNamed('Outline', 0));
      p.layer.addMask(mask: maskNamed('Highlight', 40));
      p.uiState.model.refresh();

      // Somewhere to write to, and something to read back from: an ordinary
      // effect instance, whose value map the row's writes land in.
      p.layer.addEffect(name: 'blur');
      final fx = p.layer.getEffects().single;
      final id = fx.id();
      BridgeEffectValue? written;
      const param = BridgeParamInfo(
        id: 'path',
        label: 'Path',
        kind: BridgeParamKind.maskPath(),
      );

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: EffectParamRowFrb(
          effectId: id,
          param: param,
          value: const BridgeEffectValue.maskPath(),
          comp: p.uiState.selectedComp!,
          playheadFrame: 0,
          onSeek: (_) {},
          onWrite: (_, __, v) => written = v,
          onLive: (_, __, ___) {},
          ownerLayerId: p.layer.internallayerId,
          ownerLayers: p.uiState.model.layers,
        ),
      ));
      await tester.pumpAndSettle();

      final picker = find.byKey(ValueKey<String>('fx-mask-$id-path'));
      expect(picker, findsOneWidget);
      expect(find.text('First mask'), findsOneWidget,
          reason: 'an unset row means the layer’s first mask, not "None"');

      // Open it: the entry, then this layer’s masks by their own names — and
      // nothing from any other layer.
      await tester.tap(picker);
      await tester.pumpAndSettle();
      expect(find.text('Outline'), findsOneWidget);
      expect(find.text('Highlight'), findsOneWidget);

      await tester.tap(find.text('Highlight').last);
      await tester.pumpAndSettle();
      expect(
        written,
        isA<BridgeEffectValue_MaskPath>().having(
          (v) => v.field0,
          'mask',
          p.layer.getMasks()[1].id,
        ),
        reason: 'picking a mask writes that mask, as a MaskPath value',
      );
    });

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
  }, skip: !engineAvailable);
}
