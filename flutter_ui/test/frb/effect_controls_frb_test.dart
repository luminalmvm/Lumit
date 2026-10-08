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
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/state/clipboard.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show
        effectLabelOf,
        logSliderTravel,
        logSliderUsable,
        logSliderValue,
        EffectParamRowFrb;
import 'package:lumit_flutter/state/dropper.dart' show DropperSample;
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/dashed_outline.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart';
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
    /// Every container label in the panel is a kicker (docs/15 §7.1) and a
    /// kicker capitalises **on the way to the screen**, so the schema label
    /// and the arb string both stay sentence case and only the finder knows
    /// about the capitals.
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
      // The Transform card is off by default; the rows it holds are
      // still this panel's to test, so the tests that want them ask for it
      // exactly as a user would.
      bool transform = true,
      DensityTokens density = DensityTokens.regular,
      ThemeShape shape = ThemeShape.studio,
    }) async {
      p.uiState.workspace.interface.transformInEffectControls = transform;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        density: density,
        shape: shape,
      ));
      await tester.pump();
    }

    testWidgets('Add effect commits one, and it appears as a card',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(find.textContaining('No effects'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('fx-add')));
      await tester.pumpAndSettle();
      // The menu lists categories, each opening onto its effects by their
      // sentence-case label — the raw match_name never reaches the user
      // (Add effect → Blur & sharpen → Gaussian blur).
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
      // not the panel that happens to be next to it.
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final other = p.uiState.selectedComp!.addSolidLayer();
      // Vignette, not Invert: every matte row draws the word "Invert", so the
      // effect of that name is no longer a unique bit of text.
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

      // Reorder: right-click the second card's heading and move it up. The
      // two arrows' rare job moved into a menu, and their space went to the
      // render time, which is read constantly.
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

    /// Folds are kept with the project, so a panel closed and opened again
    /// finds its effects and groups as it left them.
    testWidgets('effect and group folds outlive the panel', (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'lens_flare');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      await tester.tap(heading('Lens options'));
      await tester.pump();
      expect(find.text('Blades'), findsOneWidget);

      await tester.pumpWidget(const SizedBox());
      await mount(tester, p, transform: false);
      expect(find.text('Blades'), findsOneWidget,
          reason: 'the group came back open');

      final id = p.layer.getEffects().single.id();
      await tester.tap(find.byKey(ValueKey<String>('fx-twirl-$id')));
      await tester.pump();
      await tester.pumpWidget(const SizedBox());
      await mount(tester, p, transform: false);
      expect(heading('Lens options'), findsNothing,
          reason: 'the effect came back shut');
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
    /// revision check.
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

    /// A camera is 3D by construction whatever its switch says: it
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
      // ("Matte" is this row's label — the uniform word. In Manual the Source
      // dropdown reads "Manual light", so nothing says it at all.)
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

      // The Matte row carries its Invert, like every other one: drawn
      // inside the picker's row and never given one of its own, folded by the
      // same id convention the injected rows use — and it belongs to the
      // Matte-only group, so the rows under it stay conditional.
      final fxId = p.layer.getEffects().single.id();
      expect(find.text('Invert'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(ValueKey<String>('fx-row-$fxId-matte')),
          matching: find.byKey(ValueKey<String>('fx-bool-$fxId-matte_invert')),
        ),
        findsOneWidget,
        reason: 'the flare Invert sits beside its picker, on the same row',
      );
      expect(find.byKey(ValueKey<String>('fx-row-$fxId-matte_invert')),
          findsNothing,
          reason: 'and so has no row of its own to sit on');

      // The Matte starts pointed at the layer the effect is ON, and the
      // picker says so. Before this it defaulted to None and the effect sat
      // there detecting nothing until you went hunting for another layer —
      // which on an adjustment layer, whose only picture is the composite
      // below, was always the wrong one.
      expect(find.textContaining('(this layer)'), findsOneWidget);

      // Light tint is a source-mode-independent row; Use source
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
      expect(find.text('Invert'), findsNothing,
          reason:
              'the Invert is part of the Matte-only group, not a stray row');
    });

    // Blend: the Transparent/Black Background pair became a blend
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

    /// **The uniform Matte row**. Every effect can be driven by a
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
      // The Mix and Matte rows sit under the Compositing fold, shut by default.
      await tester.tap(find.byKey(ValueKey<String>('fx-compositing-$id')));
      await tester.pumpAndSettle();
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
      // Numbered by place in the composition since item 6.13, so the entry
      // is "1. Solid" rather than the bare name.
      await tester.tap(find.textContaining(other.getInfo().name).last);
      await tester.pumpAndSettle();
      expect(
        p.layer.getEffects().single.getValue(id: 'matte'),
        isA<BridgeEffectValue_Layer>()
            .having((v) => v.field0, 'matte', other.internallayerId),
        reason: 'the bound matte round-trips through the document',
      );
    });

    /// **The Matte row picks a channel and the Mix row a blend**. The
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
      // Both rows sit under the Compositing fold, shut by default.
      await tester.tap(find.byKey(ValueKey<String>('fx-compositing-$id')));
      await tester.pumpAndSettle();
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

    /// **P0 — copying an effect and pasting it did nothing** (owner, desk
    /// test). The chord had no handler on this panel at all, so it went to the
    /// shell, where `copySelectionFrb` offers it first to whichever panel has
    /// *claimed* copy for keyframes — and a paste that answers the claim puts
    /// keys back, not the effect. The panel now claims both chords while it is
    /// the active one, chaining onto whatever held them, which is what makes
    /// the round trip land.
    testWidgets('Copy and Paste carry an effect, onto another layer too',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      p.layer.addEffect(name: 'vignette');

      // Anything at all could have claimed the chord already — this is exactly
      // what the Timeline does with a property row picked — and the effect
      // must still win while this panel is the one being used. Set before the
      // panel mounts, because chaining is what mounting does.
      p.uiState.copyClaim = () => true;
      p.uiState.pasteClaim = () => true;

      await mount(tester, p);
      p.uiState.activePane.value = Panel.effectControls.pane();

      final second = p.layer.getEffects()[1];
      await tester.tap(heading(effectLabelOf(second.name())));
      await tester.pumpAndSettle();
      expect(p.uiState.selectedEffects.value, [second.id()]);

      // Through the shell's own Copy, which is the only route the chord takes.
      expect(copySelectionFrb(p.uiState), isTrue);
      expect(p.uiState.clipboard.kind, ClipboardKind.effects,
          reason: 'the picked effect went on the clipboard, not the keys a '
              'panel elsewhere had claimed');

      // Paste onto ANOTHER layer: select it, and the panel follows.
      final other = p.uiState.selectedComp!.addSolidLayer();
      p.uiState.setSelection([other]);
      await tester.pumpAndSettle();

      Future<void> paste() async {
        await pasteSelectionFrb(
            p.state, p.uiState, p.uiState.selectedComp, other);
        await tester.pumpAndSettle();
      }

      await paste();
      expect(other.getEffects(), hasLength(1),
          reason: 'one effect, not the whole stack it was picked out of');
      expect(other.getEffects().single.name(), second.name());
      expect(other.getEffects().single.id(), isNot(second.id()),
          reason: 'a pasted effect is a fresh instance, never a shared id');

      // And it is on screen, not only in the document.
      expect(heading(effectLabelOf(second.name())), findsOneWidget);

      // Pasting again onto the same layer stacks a second copy — the paste is
      // an append, exactly as loading a preset is.
      await paste();
      expect(other.getEffects(), hasLength(2));
    });

    /// **Rename is on the heading's menu** (owner, desk test). `Enter` on the
    /// selected effect was the only way in, and a keyboard-only act is one
    /// nobody finds. It is the menu rather than a double-click on the name
    /// because that is the pattern the application already settled on:
    /// renaming came off a list row's second click and went on the row menu
    /// instead. An effect heading is a list row.
    testWidgets('the heading menu renames the effect, in one undo step',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p);

      final effect = p.layer.getEffects().single;
      Future<void> openMenu() async {
        await tester.tapAt(
          tester.getCenter(heading(effectLabelOf(effect.name()))),
          buttons: kSecondaryButton,
        );
        await tester.pumpAndSettle();
      }

      await openMenu();
      await tester
          .tap(find.byKey(ValueKey<String>('fx-menu-rename-${effect.id()}')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-rename-field')), findsOneWidget,
          reason: 'the menu opened the heading\'s own inline editor');

      await tester.enterText(
          find.byKey(const ValueKey('fx-rename-field')), 'Soften the sign');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(heading('Soften the sign'), findsOneWidget);
      expect(
          p.layer.getEffects().single.getInfo().customName, 'Soften the sign');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect(p.layer.getEffects().single.getInfo().customName, isNull,
          reason: 'one rename, one undo step');

      // And an empty name clears back to the effect's own label, the same way
      // the keyboard path's does — one editor, one contract.
      await openMenu();
      await tester
          .tap(find.byKey(ValueKey<String>('fx-menu-rename-${effect.id()}')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const ValueKey('fx-rename-field')), '');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(p.layer.getEffects().single.getInfo().customName, isNull);
      expect(heading(effectLabelOf('blur')), findsOneWidget);
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
            vertexFeather: const [],
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
        derived: false,
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

    /// A sequenced row cut in two, a second sequenced row beside it, and an
    /// effect to hang a clip picker off. The clips are what the picker lists;
    /// the second row is there to be left out of it.
    ({
      LumitState state,
      LumitUiState uiState,
      LayerReference layer,
      UuidValue effect,
    }) withClips() {
      final p = withLayer();
      final comp = p.uiState.selectedComp!;
      p.layer.convertToSequenced();
      p.layer.cutClipAt(frame: 40);
      final other = p.state.project!.importFootage(path: 'C:/clips/other.mov');
      comp.addFootageLayer(footage: other, asSequence: true);
      p.uiState.model.refresh();
      // Somewhere to write to: an ordinary instance, whose values the row's
      // writes land in.
      p.layer.addEffect(name: 'blur');
      return (
        state: p.state,
        uiState: p.uiState,
        layer: p.layer,
        effect: p.layer.getEffects().single.id(),
      );
    }

    /// The clip row under test, with `siblings` standing in for the rest of
    /// the effect: on Audio level that is the Audio row above it.
    Widget clipRow(
      ({
        LumitState state,
        LumitUiState uiState,
        LayerReference layer,
        UuidValue effect,
      }) p,
      Map<String, BridgeEffectValue> siblings,
      void Function(BridgeEffectValue) onWrite,
    ) =>
        hostPanel(
          state: p.state,
          uiState: p.uiState,
          child: EffectParamRowFrb(
            effectId: p.effect,
            param: const BridgeParamInfo(
              id: 'clip',
              label: 'Clip',
              kind: BridgeParamKind.clip(),
              unit: BridgeUnit.raw,
              derived: false,
            ),
            value: const BridgeEffectValue.clip(),
            comp: p.uiState.selectedComp!,
            playheadFrame: 0,
            onSeek: (_) {},
            onWrite: (_, __, v) => onWrite(v),
            onLive: (_, __, ___) {},
            ownerLayerId: p.layer.internallayerId,
            ownerLayers: p.uiState.model.layers,
            siblings: siblings,
          ),
        );

    /// **A clip picker offers the clips of the layer its own Layer row names,
    /// and nobody else's** (docs/impl/audio-nodes.md §3, plan 4).
    ///
    /// A clip belongs to a layer, so a row cannot reach across and name a clip
    /// on another one. Two cuts of the same file are two entries, because an
    /// entry is what it plays *and* where it starts.
    testWidgets('a clip row lists only the named layer\'s clips',
        (tester) async {
      final p = withClips();
      BridgeEffectValue? written;
      await tester.pumpWidget(clipRow(
        p,
        {'audio': BridgeEffectValue.layer(p.layer.internallayerId)},
        (v) => written = v,
      ));
      await tester.pumpAndSettle();

      await tester
          .tap(find.byKey(ValueKey<String>('fx-clip-${p.effect}-clip')));
      await tester.pumpAndSettle();
      expect(find.text('shot.mov · 0'), findsOneWidget);
      expect(find.text('shot.mov · 40'), findsOneWidget);
      expect(find.textContaining('other.mov'), findsNothing,
          reason: 'a clip on another layer is not this row\'s to name');

      await tester.tap(find.text('shot.mov · 40').last);
      await tester.pumpAndSettle();
      final clips = p.layer.getClips()
        ..sort((a, b) => a.startFrame.compareTo(b.startFrame));
      expect(
        written,
        isA<BridgeEffectValue_Clip>()
            .having((v) => v.field0, 'clip', clips[1].id),
        reason: 'picking a clip writes that clip, as a Clip value',
      );
    });

    // -----------------------------------------------------------------------
    // A pick is a typed value, not a reset.
    // -----------------------------------------------------------------------

    /// **Lifting a number off the picture must not throw the curve away.**
    ///
    /// The colour swatch already had this right: a picked channel goes
    /// through `scalarWithValueAt`, so a keyed colour takes a key at the
    /// playhead. The two *number* pickers beside it did not — the focal-point
    /// dropper and the x/y crosshair both stated a bare static — so picking a
    /// focus distance on an animated depth-of-field deleted every keyframe it
    /// had, which is the opposite of the gesture's whole meaning.
    ///
    /// Typing the same number keeps the curve, so picking it must too.
    testWidgets('picking a focus distance keys it rather than flattening it',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final id = p.layer.getEffects().single.id();

      // Two keys, neither of them under the playhead: a pick at frame 0 has to
      // leave both standing and plant a third.
      BridgeKeyframe key(int frame, double value) => BridgeKeyframe(
            time: p.uiState.selectedComp!.timeOfFrame(frame: frame),
            value: value,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          );
      final animated = BridgeScalar.keyframed([key(10, 0.2), key(20, 0.8)]);

      BridgeEffectValue? written;
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: EffectParamRowFrb(
          effectId: id,
          // The focal point of a depth-of-field: the one row that offers a
          // depth dropper, and it offers it only beside a `depth` layer.
          param: const BridgeParamInfo(
            id: 'focus',
            label: 'Focus',
            kind: BridgeParamKind.float(
                default_: 0.5,
                sliderMin: 0,
                sliderMax: 1,
                hardMin: 0,
                hardMax: 1),
            unit: BridgeUnit.raw,
            derived: false,
          ),
          value: BridgeEffectValue.float(animated),
          siblings: {
            'depth': BridgeEffectValue.layer(p.layer.internallayerId),
          },
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

      // Arm it the way a hand does, then hand it the sample the Viewer would.
      await tester.tap(find.byKey(ValueKey<String>('dropper-fx-$id-focus')));
      await tester.pumpAndSettle();
      final arm = p.uiState.dropper.value;
      expect(arm, isNotNull, reason: 'the tap armed the dropper');
      arm!.onPick(const DropperSample(
          r: 0, g: 0, b: 0, depth: 0.4, x: 4, y: 4, region: 1));
      await tester.pumpAndSettle();

      final value = written;
      expect(value, isA<BridgeEffectValue_Float>());
      final scalar = (value as BridgeEffectValue_Float).field0;
      expect(scalar, isA<BridgeScalar_Keyframed>(),
          reason: 'a pick on a keyed property stays keyed — it is a typed '
              'value, not a reset');
      final keys = (scalar as BridgeScalar_Keyframed).field0;
      expect(keys.length, 3,
          reason: 'the two keys that were there survive, and the pick plants '
              'a third under the playhead');
      expect(keys.first.value, closeTo(0.4, 1e-9),
          reason: 'the new key at frame 0 carries the sampled depth');
      expect([keys[1].value, keys[2].value], [0.2, 0.8],
          reason: 'the keys away from the playhead are untouched');
    });

    // -----------------------------------------------------------------------
    // The unit rider and the vector-pair chain (docs/15 §12A.3).
    // -----------------------------------------------------------------------

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

    /// A **keyed** other half of a static well scales whole: every key's
    /// value times the factor, every key's time, interpolation and eased
    /// shape held. Scaling only the number under the playhead would plant
    /// keys nobody made.
    testWidgets('a chained pair scales a keyed half key by key',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'lens_flare');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      BridgeScalar scalarOf(String param) => (p.layer
              .getInfo()
              .effects
              .single
              .values
              .firstWhere((e) => e.id == param)
              .value as BridgeEffectValue_Float)
          .field0;

      // x static, y a two-key curve with an eased side worth keeping.
      const eased =
          BridgeSideInterp.bezier(BridgeBezierSide(speed: 12, influence: 0.4));
      final keys = [
        const BridgeKeyframe(
          time: BridgeRational(num: 0, den: 1),
          value: 50,
          interpIn: BridgeSideInterp.linear(),
          interpOut: eased,
        ),
        const BridgeKeyframe(
          time: BridgeRational(num: 1, den: 1),
          value: -20,
          interpIn: eased,
          interpOut: BridgeSideInterp.hold(),
        ),
      ];
      final stack = p.layer.getEffects();
      stack.single
        ..setValue(
            id: 'light_x',
            value: const BridgeEffectValue.float(BridgeScalar.static_(100)))
        ..setValue(
            id: 'light_y',
            value: BridgeEffectValue.float(BridgeScalar.keyframed(keys)));
      p.layer.setEffects(effects: stack);
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      Future<void> typeX(String value) async {
        await tester.tap(find.byKey(ValueKey<String>('fx-float-$id-light_x')));
        await tester.pump();
        await tester.enterText(find.byType(EditableText).first, value);
        await tester.testTextInput.receiveAction(TextInputAction.done);
        await tester.pumpAndSettle();
      }

      // Unchained, the curve is not touched at all.
      await typeX('200');
      expect(scalarOf('light_y'), BridgeScalar.keyframed(keys),
          reason: 'a separate pair moves alone, curve and all');

      await tester
          .tap(find.byKey(ValueKey<String>('fx-pair-link-$id-light_x')));
      await tester.pumpAndSettle();
      await typeX('400');

      final scaled = scalarOf('light_y') as BridgeScalar_Keyframed;
      expect(scaled.field0.length, 2, reason: 'no key is added or dropped');
      expect([for (final k in scaled.field0) k.value], [100.0, -40.0],
          reason: 'x doubled, so every key doubled');
      expect(
          [for (final k in scaled.field0) k.time], [keys[0].time, keys[1].time],
          reason: 'times are the other axis');
      expect(scaled.field0.first.interpIn, const BridgeSideInterp.linear());
      expect(scaled.field0.last.interpOut, const BridgeSideInterp.hold());
      expect(
          scaled.field0.first.interpOut,
          const BridgeSideInterp.bezier(
              BridgeBezierSide(speed: 24, influence: 0.4)),
          reason: 'speed lives on the value axis and scales with it; '
              'influence is the shape and does not');

      // One undo step for the whole gesture, both halves together.
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect(scalarOf('light_y'), BridgeScalar.keyframed(keys));
      expect(scalarOf('light_x'), const BridgeScalar.static_(200),
          reason: 'the pair is one op, so one undo puts both back');

      // Nought has no factor: a pair dragged off zero separates rather than
      // multiplying a whole curve by nothing.
      final zeroed = p.layer.getEffects();
      zeroed.single.setValue(
          id: 'light_x',
          value: const BridgeEffectValue.float(BridgeScalar.static_(0)));
      p.layer.setEffects(effects: zeroed);
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      await typeX('50');
      expect(scalarOf('light_y'), BridgeScalar.keyframed(keys),
          reason: 'every number is nought times something');
    });

    /// **A driven parameter says so in the stopwatch's column**: a driver
    /// wired to it wins over its keyframes, so the hollow ring and the word
    /// *driven* stand where the stopwatch and the key navigator were — neither
    /// means anything on a row with no keys of its own — and the value field
    /// keeps drawing the number while refusing every gesture on it.
    testWidgets('a driven parameter marks the left of the row and goes deaf',
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
          groups: const [],
          outUnwired: false,
        ),
      );
      await mount(tester, p);

      final mark = find.byKey(ValueKey<String>('fx-driven-$effect-radius'));
      final field = find.byKey(ValueKey<String>('fx-float-$effect-radius'));
      expect(mark, findsOneWidget);
      expect(find.text('driven'), findsOneWidget);
      expect(find.byKey(ValueKey<String>('kf-stopwatch-$effect-radius')),
          findsNothing,
          reason: 'a driven row has no keys of its own to switch on');
      expect(find.byKey(ValueKey<String>('kf-toggle-$effect-radius')),
          findsNothing,
          reason: 'nor any to step between');
      expect(tester.getTopLeft(mark).dx, lessThan(tester.getTopLeft(field).dx),
          reason: 'the mark takes the column the stopwatch had, on the left');
      expect(field, findsOneWidget,
          reason: 'the number the row holds is still worth reading');
      expect(find.ancestor(of: field, matching: find.byType(IgnorePointer)),
          findsWidgets,
          reason: 'but the wire decides the value, so the field takes no '
              'gesture');

      // Unwire it and the ordinary control comes straight back. The staged
      // instance above was consumed by its own commit, so this reads a fresh
      // one — the same rule every staged handle on this seam follows.
      p.layer.setGraph(
        drivers: p.layer.getGraphDrivers(),
        wiring: const BridgeGraphWiring(
            outUnwired: false, edges: [], layout: [], exposed: [], groups: []),
      );
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.byKey(ValueKey<String>('fx-driven-$effect-radius')),
          findsNothing);
    });

    // -------------------------------------------------------------------
    // A command on a picked run acts on the whole run.
    //
    // `_withHandle` matched one effect id and returned after the first hit, so
    // the enable switch, the × and the menu's Remove and Move commands were
    // all singular while Copy - two rows away in the same menu - already took
    // the picked run. They ask the same question now: `effectsToCopy`.
    // -------------------------------------------------------------------

    /// **Where a picked run lands when it is moved**. Each effect is
    /// taken out and put back at the target index, so the run has to be walked
    /// from the far end - otherwise it arrives inside out.
    testWidgets('Move to top takes the picked run, in its own order',
        (tester) async {
      final p = withLayer();
      for (final name in ['blur', 'vignette', 'invert', 'tint']) {
        p.layer.addEffect(name: name);
      }
      await mount(tester, p);
      final stack = p.layer.getEffects();
      // The bottom two, moved to the top together.
      p.uiState.setEffectSelection(p.layer, [stack[2].id(), stack[3].id()]);
      await tester.pump();

      await tester.tapAt(
        tester.getCenter(heading(effectLabelOf(stack[2].name()))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(ValueKey<String>('fx-menu-top-${stack[2].id()}')));
      await tester.pumpAndSettle();

      expect([
        for (final e in p.layer.getEffects()) e.name()
      ], [
        stack[2].name(),
        stack[3].name(),
        stack[0].name(),
        stack[1].name(),
      ]);
    });

    // -------------------------------------------------------------------
    // The panel as a whole: a drag across the switches, the picked run's
    // twirl, Delete, and what survives a comp being fronted.
    // -------------------------------------------------------------------

    /// **Delete removes the picked effects** (item 6.6) — claimed rather than
    /// handled on the keyboard, because the shell's own Delete removes the
    /// *layer* and every hardware-keyboard handler runs on every key. The
    /// shell asks the claim first; this is that call.
    testWidgets('Delete removes the picked effects, and nothing else',
        (tester) async {
      final p = withLayer();
      for (final name in ['blur', 'vignette', 'invert']) {
        p.layer.addEffect(name: name);
      }
      await mount(tester, p, transform: false);
      final stack = p.layer.getEffects();
      p.uiState.activePane.value = Panel.effectControls.pane();

      expect(p.uiState.deleteClaim, isNotNull,
          reason: 'the panel claims Delete while it is mounted');
      expect(p.uiState.deleteClaim!(), isFalse,
          reason: 'nothing picked is not this panel’s Delete — the layer '
              'selection is what the shell falls back to');

      p.uiState.setEffectSelection(p.layer, [stack[0].id(), stack[2].id()]);
      await tester.pump();
      expect(p.uiState.deleteClaim!(), isTrue);
      await tester.pump();

      expect(
          [for (final e in p.layer.getEffects()) e.name()], [stack[1].name()],
          reason: 'the picked run went and the unpicked effect stayed');
      expect(p.uiState.selectedEffects.value, isEmpty,
          reason: 'nothing is picked once it no longer exists');
      expect(p.layer.getInfo().name, isNotEmpty,
          reason: 'the layer itself is untouched');
    });

    /// **A Custom shader's rows are the ones its own source declares**
    /// (docs/impl/custom-shader.md §1.5, CS2). Every other effect's controls are
    /// the same on every layer they are dropped on; this one's come from the
    /// shader *this copy of it* holds, and they have to be ordinary rows once
    /// they get here — same widgets, same labels, same everything.
    testWidgets("a Custom shader's rows come from the shader it holds",
        (tester) async {
      const twoRows = r"""
struct Params {
    /// @slider(0, 200) @default(25) @unit(px) Ripple radius
    radius: f32,
    /// @colour @default(1, 0.5, 0.2, 1) Ripple tint
    tint: vec4<f32>,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    return lumit_sample(uv) * p.tint * p.radius;
}
""";

      final p = withLayer();
      p.layer.addEffect(name: 'custom_shader');
      await mount(tester, p, transform: false);

      // A fresh instance draws its declared rows and nothing else: an effect
      // the user has not filled in yet is a passthrough, not a failure.
      expect(heading('Custom shader'), findsOneWidget);
      expect(find.text('Edit shader…'), findsOneWidget,
          reason: 'the two Action rows are declared, so they draw');
      expect(find.text('Load from file…'), findsOneWidget);
      expect(find.text('Ripple radius'), findsNothing);

      // Load a shader the way `Load from file…` does: staged on one handle,
      // committed with the stack.
      final stack = p.layer.getEffects();
      stack.single.setShaderSource(source: twoRows, origin: null);
      p.layer.setEffects(effects: stack);
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      expect(find.text('Ripple radius'), findsOneWidget,
          reason: "the source's own uniforms are rows in the panel");
      expect(find.text('Ripple tint'), findsOneWidget);

      // A shader that will not compile wears the calm badge, with the
      // compiler's own sentence beneath it and its line numbers moved onto the
      // text the user typed — and the rows below it stay live.
      final broken = p.layer.getEffects();
      broken.single.setShaderSource(
        source: 'fn shade(uv: vec2<f32>) -> vec4<f32> {\n'
            '    let a = 1.0;\n'
            '    return nonesuch(uv);\n}\n',
        origin: null,
      );
      p.layer.setEffects(effects: broken);
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      expect(find.textContaining('wgsl:3:'), findsOneWidget,
          reason: 'the compiler names line 3 of the three lines they wrote');
    });

    /// An OCIO name row lists the project's config from the summary the
    /// interface already holds, and a pick writes the config's own spelling.
    testWidgets('an OCIO name row lists the config and writes the name',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'ocio_colour_space');
      p.uiState.colourSummary = const BridgeColourSummary(
        path: 'config.ocio',
        loaded: true,
        problem: '',
        problemArgs: [],
        problemEnglish: '',
        spaces: ['lin', 'srgb_texture'],
        displays: [],
        looks: [],
        name: 'test config',
        workingFromConfig: false,
        workingSpace: '',
        problems: [],
      );
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      final row = find
          .byKey(ValueKey<String>('fx-colour-name-$id-output_colour_space'));
      expect(row, findsOneWidget);
      await tester.tap(row);
      await tester.pumpAndSettle();
      await tester.tap(find.text('srgb_texture').last);
      await tester.pumpAndSettle();

      expect(
        p.layer.getEffects().single.getValue(id: 'output_colour_space'),
        isA<BridgeEffectValue_Text>()
            .having((v) => v.field0, 'name', 'srgb_texture'),
        reason: 'the pick reached the document as the config spells it',
      );
    });

    /// **The Node graph card's Open graph row** (docs/impl/node-graph-comp.md
    /// §4.4): it fronts the composition the effect applies, the way the Custom
    /// shader's Edit enters its inner graph. Fronting a comp is not an event
    /// the engine could answer, so the panel takes this row itself.
    testWidgets('a Node graph effect opens the graph it applies',
        (tester) async {
      final p = withLayer();
      final graph = p.state.project!.newNodeGraph(name: 'Wires');
      p.layer.addNodeGraphEffect(graph: graph);
      p.uiState.model.refresh();
      await mount(tester, p);

      final id = p.layer.getEffects().single.id();
      await tester.tap(find.byKey(ValueKey<String>('fx-action-$id-open')));
      await tester.pumpAndSettle();

      expect(p.uiState.selectedComp?.internalid, graph.internalid,
          reason: 'the row fronted the bound graph');
    });

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
  }, skip: !engineAvailable);

  // The logarithmic slider's own arithmetic, which needs no engine: a
  // frequency row spends half its travel in the bottom decade or the control
  // is useless (docs/impl/audio-effects.md §2).
  group('Logarithmic slider travel', () {
    test('travel and value are inverses of one another', () {
      for (final hz in [20.0, 100.0, 1000.0, 4400.0, 20000.0]) {
        final t = logSliderTravel(hz, 20, 20000);
        expect(logSliderValue(t, 20, 20000), closeTo(hz, 1e-6),
            reason: 'the thumb comes back to \$hz');
      }
    });

    test('a range through zero falls back to linear rather than a NaN', () {
      expect(logSliderUsable(0, 100), isFalse);
      expect(logSliderValue(0.5, 0, 100), closeTo(50, 1e-9));
      expect(logSliderTravel(50, 0, 100), closeTo(0.5, 1e-9));
      expect(logSliderValue(0.25, -1, 1), closeTo(-0.5, 1e-9));
    });
  });
}
