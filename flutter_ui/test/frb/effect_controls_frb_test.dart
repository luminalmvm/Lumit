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
    show effectLabelOf, EffectParamRowFrb, EffectPointRowFrb;
import 'package:lumit_flutter/icons/lumit_icon.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/angle_dial.dart';
import 'package:lumit_flutter/widgets/dashed_outline.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:uuid/uuid.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Effect controls (frb)', () {
    /// A section or parameter-group heading, by the words in it.
    ///
    /// Since K-443 every container label in the panel is a kicker (docs/15
    /// §7.1) and a kicker capitalises **on the way to the screen**, so the
    /// schema label and the arb string both stay sentence case and only the
    /// finder knows about the capitals.
    Finder heading(String label) => find.text(label.toUpperCase());

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
      DensityTokens density = DensityTokens.regular,
    }) async {
      p.uiState.workspace.interface.transformInEffectControls = transform;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        density: density,
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
      expect(heading('Gaussian blur'), findsOneWidget,
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
      expect(heading('Gaussian blur'), findsOneWidget,
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
        tester.getCenter(heading(effectLabelOf(second.name()))),
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

      await tester.tap(heading(effectLabelOf(stack.first.name())));
      await tester.pumpAndSettle();
      expect(p.uiState.selectedEffects.value, [stack.first.id()]);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tap(heading(effectLabelOf(stack[2].name())));
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
      expect(heading('Gaussian blur'), findsOneWidget);

      p.uiState.setSelection([other]);
      await tester.pump();

      expect(heading('Vignette'), findsOneWidget,
          reason: "the panel shows the newly selected layer's stack");
      expect(heading('Gaussian blur'), findsNothing,
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

      // **Bypassed draws as a dashed outline, not a dimmed row** (docs/15 §5).
      // The rows stop answering the pointer, but nothing fades: the reason to
      // look at a bypassed effect is to read what it is set to.
      expect(find.byType(DashedOutline), findsOneWidget,
          reason: 'the bypassed heading wears the outline; the live one does '
              'not');
      expect(
        find.descendant(
          of: find.byType(EffectParamRowFrb),
          matching: find.byWidgetPredicate((w) => w is Opacity && w.opacity < 1,
              description: 'a dimmed row'),
        ),
        findsNothing,
        reason: 'the 40% dim is gone — the outline carries the state',
      );

      // Reorder: right-click the second card's heading and move it up (K-276
      // put the two arrows' rare job in a menu and gave their space to the
      // render time, which is read constantly).
      final before = p.layer.getEffects().map((e) => e.name()).toList();
      final second = p.layer.getEffects()[1];
      await tester.tapAt(
        tester.getCenter(heading(effectLabelOf(second.name()))),
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
      final from = heading(effectLabelOf('sharpen'));
      final onto = heading(effectLabelOf('blur'));
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
        tester.getCenter(heading(effectLabelOf(effects[0].name()))),
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
      await tester.tap(heading('Gaussian blur'));
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

      expect(heading('Transform'), findsOneWidget);
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
      expect(heading('Lens options'), findsOneWidget);
      expect(heading('Flare options'), findsOneWidget);
      expect(find.text('Blades'), findsNothing);

      // Twirling Lens options open reveals the Int-kind Blades row.
      await tester.tap(heading('Lens options'));
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
    // --- Particulate's surface (particulate.md §2, points-stream.md §4.3) ---
    //
    // PS6 is the *verification* that the effect's controls arrive from the
    // schema with no new row kind: four kickers, the two over-life curves, the
    // seed with its reseed, the mask-path reference, the layer reference and
    // the Mix row. What is asserted here is what would break silently — the
    // Render group's three modes and the rows each of them owns, and that the
    // sprite reference actually *binds*, since an unset one draws discs and
    // says nothing about why.

    testWidgets('Particulate draws its four groups and both over-life curves',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'particulate');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      for (final label in ['Emitter', 'Particle', 'Forces', 'Render']) {
        expect(heading(label), findsOneWidget, reason: label);
      }
      final id = p.layer.getEffects().single.id();
      // The two over-life curves fold into one editor with a tab each (K-412),
      // never one plot per row.
      expect(find.byKey(ValueKey<String>('fx-curves-$id')), findsOneWidget);
      expect(find.text('Size over life'), findsOneWidget);
      expect(find.text('Opacity over life'), findsOneWidget);
      // The seed's reseed, the mask-path reference and the sprite reference.
      expect(find.byKey(ValueKey<String>('fx-seed-$id-seed')), findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-mask-$id-mask_path')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-layer-$id-sprite_layer')),
          findsOneWidget);
      // K-507's dial-turned-slider: the jitter the note left silent about.
      expect(find.text('Rotation jitter'), findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-float-$id-rotation_jitter')),
          findsOneWidget);
    });

    /// **Each render mode's own control, live only in that mode.** Three
    /// `EnabledWhen` rules, and nothing else in the panel knows the mode
    /// exists — which is what makes them worth pinning.
    testWidgets('Particulate greys the rows the render mode does not use',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'particulate');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      Set<String> greyed() => tester
          .widgetList<EffectParamRowFrb>(find.byType(EffectParamRowFrb))
          .where((r) => !r.enabled)
          .map((r) => r.param.id)
          .toSet();

      // Disc, the default: Feather is its own control; the other two are not.
      expect(greyed(), isNot(contains('feather')));
      expect(greyed(), contains('sprite_layer'));
      expect(greyed(), contains('streak_length'));

      // Move to Sprite and the greying moves with it. The mode is set on the
      // document rather than through its dropdown: what is under test is the
      // panel's reading of `EnabledWhen`, and the dropdown is the same control
      // every other choice row draws.
      final staged = p.layer.getEffects();
      staged.single
          .setValue(id: 'mode', value: const BridgeEffectValue.choice(1));
      p.layer.setEffects(effects: staged);
      p.uiState.model.refresh();
      await tester.pump();

      expect(greyed(), contains('feather'));
      expect(greyed(), isNot(contains('sprite_layer')));
      expect(greyed(), contains('streak_length'));
    });

    /// **The sprite reference binds** — the one thing on this card that fails
    /// quietly if it does not. An unset Sprite layer resolves to Disc
    /// host-side, so a picker that looked right and wrote nothing would leave
    /// the user staring at discs with no explanation.
    testWidgets('Particulate\'s sprite layer picker reaches the document',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'particulate');
      final other = p.uiState.selectedComp!.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);
      final id = p.layer.getEffects().single.id();
      // Sprite mode, or the row is greyed and deaf by design.
      final staged = p.layer.getEffects();
      staged.single
          .setValue(id: 'mode', value: const BridgeEffectValue.choice(1));
      p.layer.setEffects(effects: staged);
      p.uiState.model.refresh();
      await tester.pump();

      // The Render group is the last of the four, so the row is below the
      // fold of a 760-tall panel: scroll to it as a user would.
      final picker = find.byKey(ValueKey<String>('fx-layer-$id-sprite_layer'));
      await tester.ensureVisible(picker);
      await tester.pumpAndSettle();
      await tester.tap(picker);
      await tester.pumpAndSettle();
      await tester.tap(find.text(other.getInfo().name).last);
      await tester.pumpAndSettle();

      expect(
        p.layer.getEffects().single.getValue(id: 'sprite_layer'),
        isA<BridgeEffectValue_Layer>()
            .having((v) => v.field0, 'sprite layer', other.internallayerId),
        reason: 'the picked layer round-trips through the document; unset '
            'would have drawn discs and said nothing',
      );
    });

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
      await tester.tap(heading(effectLabelOf(stack.single.name())));
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

      expect(heading('Blur the sign'), findsOneWidget,
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
      expect(heading(effectLabelOf('blur')), findsOneWidget,
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
        expect(heading(label), findsOneWidget, reason: label);
      }
      expect(find.text('Roundness'), findsNothing,
          reason: 'the aperture arrives collapsed behind its twirl');

      // Twirling Iris open reveals the shape controls, the dial among them.
      await tester.tap(heading('Iris'));
      await tester.pump();
      expect(find.text('Roundness'), findsOneWidget);
      expect(find.text('Blades'), findsOneWidget);
      expect(find.byType(AngleDial), findsOneWidget,
          reason: 'Rotation is a dial (docs/07 SS6), not a slider');

      // The focus point is one row over an _x/_y pair, with its own crosshair.
      await tester.tap(heading('Depth map'));
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
    /// **The fixed columns** (K-443, docs/15 §12A.3). Every row lays out on the
    /// same x positions, and the keyframe-navigation slot is reserved whether or
    /// not the property is animated — so a stopwatch being switched on adds
    /// three buttons without shifting the label under them.
    ///
    /// This is the shape the panel did NOT have: the navigator used to appear
    /// inside the name column and shove the label sideways, so twirling a
    /// stack open and keying one property re-ragged the whole list.
    group('the fixed columns (K-443)', () {
      /// The panel with Gaussian blur applied, Radius optionally keyed, and
      /// the effect's id in hand.
      Future<UuidValue> mountBlur(
        WidgetTester tester,
        ({LumitState state, LumitUiState uiState, LayerReference layer}) p, {
        required bool animated,
        DensityTokens density = DensityTokens.regular,
      }) async {
        p.layer.addEffect(name: 'blur');
        if (animated) {
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
        }
        await mount(tester, p, transform: false, density: density);
        return p.layer.getEffects().single.id();
      }

      /// The glyph inside one of the row's keyframe buttons.
      LumitIcon glyphIn(WidgetTester tester, String keyName) =>
          tester.widget<LumitIcon>(find.descendant(
            of: find.byKey(ValueKey<String>(keyName)),
            matching: find.byType(LumitIcon),
          ));

      testWidgets('the label sits at the same x animated or not',
          (tester) async {
        await mountBlur(tester, withLayer(), animated: false);
        final still = tester.getTopLeft(find.text('Radius')).dx;

        await mountBlur(tester, withLayer(), animated: true);
        expect(tester.getTopLeft(find.text('Radius')).dx, still,
            reason: 'the navigator has a slot of its own; it never borrows '
                'the label column');
      });

      testWidgets('the control column starts at the same x too',
          (tester) async {
        await mountBlur(tester, withLayer(), animated: false);
        final still = tester.getTopLeft(find.byType(DragValueField).first).dx;

        await mountBlur(tester, withLayer(), animated: true);
        expect(tester.getTopLeft(find.byType(DragValueField).first).dx, still,
            reason: 'the wells stack into one column down the panel');
      });

      testWidgets('the navigator is there only while the property is animated',
          (tester) async {
        final id = await mountBlur(tester, withLayer(), animated: false);
        for (final button in ['prev', 'toggle', 'next']) {
          expect(find.byKey(ValueKey<String>('kf-$button-$id-radius')),
              findsNothing,
              reason: 'nothing to navigate on a static value');
        }
        expect(find.byKey(ValueKey<String>('kf-stopwatch-$id-radius')),
            findsOneWidget,
            reason:
                'the stopwatch is how animation begins, so it is always there');

        final keyed = await mountBlur(tester, withLayer(), animated: true);
        for (final button in ['prev', 'toggle', 'next']) {
          expect(find.byKey(ValueKey<String>('kf-$button-$keyed-radius')),
              findsOneWidget,
              reason: button);
        }
      });

      /// **The mockups' heights are canonical** (K-451, docs/15 §12A.6). A
      /// parameter row occupies 26 whatever control it carries, a section
      /// heading 24, and a value well 20 — measured rather than trusted,
      /// because a stack whose rows step in and out is exactly the fault the
      /// fixed content box was introduced to settle.
      testWidgets('rows, headings and wells are built to K-451 heights',
          (tester) async {
        await mountBlur(tester, withLayer(), animated: false);

        // The pitch from one row to the next is what a row actually spends on
        // the panel: its content box, the section's 2px either side, and the
        // hairline under it.
        final rows = tester
            .widgetList<EffectParamRowFrb>(find.byType(EffectParamRowFrb));
        expect(rows.length, greaterThan(1),
            reason: 'Gaussian blur has a Radius and a Mix to measure between');
        final tops = [
          for (var i = 0; i < rows.length; i++)
            tester.getRect(find.byType(EffectParamRowFrb).at(i)).top,
        ]..sort();
        expect(tops[1] - tops[0], closeTo(27, 0.5),
            reason: 'a parameter row occupies 27 under Regular (K-454)');

        // The heading's own box: the nearest Container above its kicker.
        expect(
          tester
              .getRect(find
                  .ancestor(
                    of: heading('Gaussian blur'),
                    matching: find.byType(Container),
                  )
                  .first)
              .height,
          closeTo(24, 0.5),
          reason: 'an effect section heading is 24',
        );

        expect(tester.getRect(find.byType(DragValueField).first).height,
            closeTo(20, 0.5),
            reason: 'a value well in a panel is 20');
      });

      /// K-454's other column. Compact takes a pixel off the row pitch and
      /// nothing else: the heading and the well measure the same, because
      /// §12A.6's two columns agree about both.
      testWidgets('Compact takes a pixel off the row and leaves the rest',
          (tester) async {
        await mountBlur(tester, withLayer(),
            animated: false, density: DensityTokens.compact);

        final tops = [
          for (var i = 0;
              i < tester.widgetList(find.byType(EffectParamRowFrb)).length;
              i++)
            tester.getRect(find.byType(EffectParamRowFrb).at(i)).top,
        ]..sort();
        expect(tops[1] - tops[0], closeTo(26, 0.5),
            reason: 'a parameter row occupies 26 under Compact');

        expect(
          tester
              .getRect(find
                  .ancestor(
                    of: heading('Gaussian blur'),
                    matching: find.byType(Container),
                  )
                  .first)
              .height,
          closeTo(24, 0.5),
          reason: 'a heading is 24 under both densities',
        );
        expect(tester.getRect(find.byType(DragValueField).first).height,
            closeTo(20, 0.5),
            reason: 'a well is 20 under both densities');
      });

      /// The stopwatch is one of `animated`'s closed job list (§3.1) — never
      /// the accent, which the redesign spends on the filled action and the
      /// playhead.
      testWidgets('the stopwatch is muted at rest and animated when keyed',
          (tester) async {
        final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

        final id = await mountBlur(tester, withLayer(), animated: false);
        expect(glyphIn(tester, 'kf-stopwatch-$id-radius').colour, t.textMuted);

        final keyed = await mountBlur(tester, withLayer(), animated: true);
        expect(glyphIn(tester, 'kf-stopwatch-$keyed-radius').colour, t.animated,
            reason: 'a keyed property says so in amber, not in the accent');
        // And so does the number in its well: the playhead sits on the first
        // key, so the diamond is amber too.
        expect(glyphIn(tester, 'kf-toggle-$keyed-radius').colour, t.animated,
            reason: 'the playhead is on a key');
      });

      testWidgets('a keyed value rests animated in its well', (tester) async {
        final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);
        final id = await mountBlur(tester, withLayer(), animated: true);
        final number = tester.widget<Text>(find.descendant(
          of: find.byKey(ValueKey<String>('fx-float-$id-radius')),
          matching: find.byType(Text),
        ));
        expect(number.style!.color, t.animated,
            reason: 'the well is where a keyframed value says it is keyed');
      });
    });

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
        unit: BridgeUnit.raw,
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

    // -----------------------------------------------------------------------
    // The unit rider and the vector-pair chain (K-443, docs/15 §12A.3).
    // -----------------------------------------------------------------------

    /// Every numeric row says what its number *is*, in the mockup's own plain
    /// mono — and it says it off the **declaration**, which is the whole point:
    /// an id-keyed table could not tell Radial blur's per-cent `centre_x` from
    /// the dozen effects whose `centre_x` is px@comp, and it got them all
    /// wrong in the same direction.
    testWidgets('a numeric row draws its declared unit beside the value',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final t =
          ThemeScope.of(tester.element(find.byType(EffectControlsPanelFrb)))
              .theme;
      final px = tester.widget<Text>(find.text('px'));
      expect(px.style!.fontFamily, LumitTheme.monoFontFamily,
          reason: 'the rider is plain mono, not a kicker');
      expect(px.style!.fontSize, 10);
      expect(px.style!.color, t.textMuted);
      expect(px.style!.letterSpacing, isNull,
          reason: 'it states a fact; it does not name a container');

      // Radius is px@comp, Mix is a per cent, Blend is a dropdown with no
      // number and therefore no unit at all.
      expect(find.text('px'), findsOneWidget);
      expect(find.text('%'), findsOneWidget);
    });

    /// The same parameter id, two units, two effects — the case the deleted
    /// map got wrong.
    testWidgets('centre_x reads px on one effect and % on another',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'mirror');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);
      expect(find.text('px'), findsWidgets,
          reason: "Mirror's Centre is px@comp");

      final second = withLayer();
      second.layer.addEffect(name: 'radial_blur');
      second.uiState.model.refresh();
      await mount(tester, second, transform: false);
      final centre = find.ancestor(
        of: find.text('Centre'),
        matching: find.byType(EffectPointRowFrb),
      );
      expect(
        find.descendant(of: centre, matching: find.text('%')),
        findsOneWidget,
        reason: "Radial blur's Centre is a per cent of the frame",
      );
      expect(
        find.descendant(of: centre, matching: find.text('px')),
        findsNothing,
      );
    });

    /// A point is two wells with a chain between them, and the chain is a real
    /// undoable edit on the instance — not a Dart-side flag.
    testWidgets('a point pair chains and unchains, undoably', (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'lens_flare');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      final chain = find.byKey(ValueKey<String>('fx-pair-link-$id-light_x'));
      expect(chain, findsOneWidget, reason: 'the pair draws a chain');
      expect(p.layer.getInfo().effects.single.linkedPairs, isEmpty,
          reason: 'a pair starts separate, which is every older project');

      await tester.tap(chain);
      await tester.pumpAndSettle();
      expect(p.layer.getInfo().effects.single.linkedPairs, ['light']);

      // One undo step, like every other effect-stack edit.
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect(p.layer.getInfo().effects.single.linkedPairs, isEmpty);
    });

    /// Linked means proportional: dragging one well scales the other by the
    /// same factor, and typing does too — the chain is about the two numbers,
    /// not about which gesture moved one of them.
    testWidgets('a chained pair scales its other half by the same factor',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'lens_flare');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      double valueOf(String param) {
        final v = p.layer
            .getInfo()
            .effects
            .single
            .values
            .firstWhere((e) => e.id == param)
            .value;
        return ((v as BridgeEffectValue_Float).field0 as BridgeScalar_Static)
            .field0;
      }

      // Put both halves somewhere with a ratio worth keeping.
      final stack = p.layer.getEffects();
      stack.single
        ..setValue(
            id: 'light_x',
            value: const BridgeEffectValue.float(BridgeScalar.static_(100)))
        ..setValue(
            id: 'light_y',
            value: const BridgeEffectValue.float(BridgeScalar.static_(50)));
      p.layer.setEffects(effects: stack);
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      // A value well types when it is clicked into: the field is a scrub
      // control until then.
      Future<void> typeX(String value) async {
        await tester.tap(find.byKey(ValueKey<String>('fx-float-$id-light_x')));
        await tester.pump();
        await tester.enterText(find.byType(EditableText).first, value);
        await tester.testTextInput.receiveAction(TextInputAction.done);
        await tester.pumpAndSettle();
      }

      // Unchained, x moves alone.
      await typeX('200');
      expect(valueOf('light_x'), 200);
      expect(valueOf('light_y'), 50, reason: 'a separate pair moves alone');

      // Chained, y follows by the factor x moved by.
      await tester
          .tap(find.byKey(ValueKey<String>('fx-pair-link-$id-light_x')));
      await tester.pumpAndSettle();
      await typeX('400');
      expect(valueOf('light_x'), 400);
      expect(valueOf('light_y'), 100,
          reason: 'x doubled, so y doubled — the ratio is what is kept');
    });

    /// **A driven parameter says so** (K-471): a driver wired to it in the
    /// Graph panel wins over its keyframes, so the row draws a hollow ring in
    /// the wire's own colour, the word *driven*, and the driver's name in the
    /// well — never a control you could drag while the wire decides the value.
    testWidgets('a driven parameter names its driver instead of a control',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final effect = p.layer.getEffects().single.id();
      final made = p.layer.newDriver(name: 'wiggle');
      p.layer.setGraph(
        drivers: [made],
        wiring: BridgeGraphWiring(
          edges: [
            BridgeGraphEdge(
              from: BridgeOutputRef.driver(node: made.id(), port: 'value'),
              to: BridgeInputRef.param(
                  node: BridgeNodeRef.effect(effect), port: 'radius'),
            ),
          ],
          layout: const [],
          exposed: const [],
        ),
      );
      await mount(tester, p);

      expect(find.byKey(ValueKey<String>('fx-driven-$effect-radius')),
          findsOneWidget);
      expect(find.text('driven'), findsOneWidget);
      expect(find.text('Wiggle'), findsWidgets,
          reason: 'the well names the driver the parameter is following');
      expect(
          find.byKey(ValueKey<String>('fx-float-$effect-radius')), findsNothing,
          reason: 'the stored number is not what the picture uses any more, '
              'so there is nothing here to drag');

      // Unwire it and the ordinary control comes straight back. The staged
      // instance above was consumed by its own commit, so this reads a fresh
      // one — the same rule every staged handle on this seam follows.
      p.layer.setGraph(
        drivers: p.layer.getGraphDrivers(),
        wiring: const BridgeGraphWiring(edges: [], layout: [], exposed: []),
      );
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.byKey(ValueKey<String>('fx-driven-$effect-radius')),
          findsNothing);
    });

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
  }, skip: !engineAvailable);
}
