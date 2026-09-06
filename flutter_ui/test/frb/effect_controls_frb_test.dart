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
import 'package:lumit_flutter/panels/fx_section.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show effectLabelOf, EffectParamRowFrb, EffectPointRowFrb;
import 'package:lumit_flutter/state/dropper.dart' show DropperSample;
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
    /// Every container label in the panel is a kicker (docs/15 §7.1) and a
    /// kicker capitalises **on the way to the screen**, so the schema label
    /// and the arb string both stay sentence case and only the finder knows
    /// about the capitals.
    Finder heading(String label) => find.text(label.toUpperCase());

    /// A chord as the hardware keyboard delivers it, modifier held across the
    /// letter the way a person presses them.
    Future<void> chord(WidgetTester tester, LogicalKeyboardKey key) async {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(key);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();
    }

    Future<void> pasteChord(WidgetTester tester) =>
        chord(tester, LogicalKeyboardKey.keyV);

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

    /// **Add lands on every selected layer**, the way the Effect menu
    /// and the effects console already do. This button reached for the layer
    /// the panel was showing alone, so the same effect on the same selection
    /// landed on three layers from the menu and on one from here.
    testWidgets('Add effect commits to every selected layer', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final first = comp.addAdjustmentLayer();
      final second = comp.addAdjustmentLayer();
      p.uiState.setSelectedComp(comp);
      p.uiState.setSelection([first, second]);
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('fx-add')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('fx-category-blur_sharpen')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Gaussian blur'));
      await tester.pumpAndSettle();

      expect(first.getEffects(), hasLength(1));
      expect(second.getEffects(), hasLength(1),
          reason: 'the second selected layer must get the effect too');
    });

    /// **And a stack nobody has selected still takes one.** The panel keeps the
    /// last layer up after a deselect on purpose, so an add made against those
    /// rows means those rows rather than nothing at all.
    testWidgets('Add effect still reaches a deselected panel layer',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);
      p.uiState.selectedLayer.value = null;
      p.uiState.setSelection(const []);
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('fx-add')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('fx-category-blur_sharpen')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Gaussian blur'));
      await tester.pumpAndSettle();

      expect(p.layer.getEffects(), hasLength(1));
    });

    testWidgets(
        'a null layer says its effects change no picture, and keeps their values',
        (tester) async {
      // Effects on a null are ACCEPTED and labelled inert rather than refused.
      // A null draws nothing, so nothing here changes a picture — but the
      // parameters are real, animatable values, which is the whole point
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

    /// **Copying one effect**. The engine has taken one or a whole
    /// stack since copy/paste landed — `copy_effects(Some(id))` — and the Edit
    /// menu's Copy takes the *layer*, so until this row existed there was no
    /// way to pick a single effect and no way to reach the call.
    testWidgets('an effect heading copies that one effect', (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      // Not the Invert effect: every effect draws an "Invert" beside its
      // Matte picker, so an effect NAMED Invert makes the heading ambiguous
      // to find by text. Nothing here is about which effect it is.
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

    /// **An effect's name picks it**. Clicking a heading only twirled
    /// it before, so an effect could not be selected here at all — and Copy,
    /// which acts on the selection, had nothing to take but the whole layer.
    /// Shift takes the run between, the way it does in every other list here.
    /// And a click on **nothing** puts it down again (owner, desk test). The
    /// panel's floor was inert, so a pick made here outlived the moment it was
    /// made in and pointed the next Delete or Copy at an effect nobody was
    /// looking at any more.
    testWidgets('clicking an empty spot clears the pick', (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p, transform: false);
      final stack = p.layer.getEffects();

      await tester.tap(heading(effectLabelOf(stack.single.name())));
      await tester.pumpAndSettle();
      expect(p.uiState.selectedEffects.value, [stack.single.id()]);

      // The floor: below the one card, where nothing is drawn.
      final ground = tester.getRect(find.byKey(const ValueKey('fx-ground')));
      final card = tester.getRect(find.byKey(const ValueKey('fx-card-0')));
      await tester.tapAt(Offset(ground.center.dx, card.bottom + 20));
      await tester.pumpAndSettle();

      expect(p.uiState.selectedEffects.value, isEmpty,
          reason: 'a click on nothing is a click on nothing');
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

    /// **The drop indicator** (owner, desk test). A reorder drag drew a line
    /// along the top of whatever heading was under the pointer, whichever way
    /// the drag was travelling — so half the time it marked a gap the effect
    /// was not going into. The line now draws on the edge the effect lands
    /// against, which is what an insertion point means.
    testWidgets('a reorder drag marks the gap the effect will land in',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      p.layer.addEffect(name: 'sharpen');
      await mount(tester, p);

      /// The drop line's border while [from] is held over [onto], or null.
      Future<Border?> lineDragging(Finder from, Finder onto) async {
        final drag = await tester.startGesture(tester.getCenter(from));
        await tester.pump(const Duration(milliseconds: 20));
        await drag.moveTo(tester.getCenter(onto));
        await tester.pump(const Duration(milliseconds: 20));
        final found = find.byKey(const ValueKey('fx-drop-line'));
        final border = found.evaluate().isEmpty
            ? null
            : ((tester.widget<DecoratedBox>(found).decoration as BoxDecoration)
                .border as Border?);
        // Back over its own heading before letting go: a section refuses a
        // drop on itself, so the stack is left exactly as it was and the two
        // directions can be read off one arrangement.
        await drag.moveTo(tester.getCenter(from));
        await tester.pump(const Duration(milliseconds: 20));
        await drag.up();
        await tester.pumpAndSettle();
        return border;
      }

      final top = heading(effectLabelOf('blur'));
      final bottom = heading(effectLabelOf('sharpen'));
      final t = LumitTheme.dark();

      // Travelling DOWN the stack: the effect lands after the heading it is
      // over, so the line is under that heading.
      final down = await lineDragging(top, bottom);
      expect(down, isNotNull, reason: 'the drag says where it will land');
      expect(down!.bottom.color, t.accent);
      expect(down.bottom.width, fxDropLineWidth);
      expect(down.top.width, 0, reason: 'and only on the one edge');

      // Travelling UP: it lands before, so the line is over the heading.
      final up = await lineDragging(bottom, top);
      expect(up, isNotNull);
      expect(up!.top.color, t.accent);
      expect(up.bottom.width, 0);
    });

    /// **The enable switch is a target you can hit** (owner, desk test). The
    /// mark keeps its small box; the area that answers to a click is the
    /// whole stopwatch column for the whole height of the heading, because
    /// this is the control the panel is poked at most and it was being missed.
    testWidgets('the enable switch takes clicks across the whole column',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();

      final hit = find.byKey(ValueKey<String>('fx-enabled-hit-$id'));
      final box = tester.getSize(hit);
      expect(box.width, fxEnableHitWidth);
      expect(box.height, fxEnableHitHeight);
      expect(box.width * box.height, greaterThan(14 * 14),
          reason: "bigger than the checkbox's own 14px box, which is what was "
              'being missed');

      // The very corner of that block — well outside the drawn mark — still
      // switches the effect, and switches it exactly once.
      final corner = tester.getTopLeft(hit) + const Offset(1, 1);
      await tester.tapAt(corner);
      await tester.pump();
      expect(p.layer.getEffects().single.enabled(), isFalse);

      // And the mark itself is not a second switch that fires as well: a tap
      // dead centre toggles once, not twice.
      await tester.tap(find.byKey(ValueKey<String>('fx-enabled-$id')));
      await tester.pump();
      expect(p.layer.getEffects().single.enabled(), isTrue);
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

      // **Only the twirl folds it**. The name picks the effect, and a
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
      // The two over-life curves fold into one editor with a tab each,
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
      // The dial that became a slider: the jitter the note left silent about.
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
      // Numbered by place in the composition since item 6.13, so the entry
      // is "1. Solid" rather than the bare name.
      await tester.tap(find.textContaining(other.getInfo().name).last);
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

    // The Lens picker is curated. Twenty entries sit well
    // under the searchable threshold, so the row is the PLAIN dropdown —
    // the searchable picker's laziness is pinned in
    // test/search_dropdown_test.dart against synthetic options. What the
    // panel owes here: the curated default shows, and the custom Lens file
    // row is present for the prescriptions the palette leaves out.
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

    /// Depth of field owned the idea first, under its own ids, and it keeps
    /// them. It now takes the shared row and the shared words: `depth` is
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
      p.uiState.activePanel.value = Panel.effectControls;

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

    /// **P0 — `Ctrl+V` pasted twice** (owner, desk test). The panel answered
    /// the chord on the hardware keyboard *and* the shell answered it, and
    /// every hardware-keyboard handler runs on every key: one press put the
    /// effects on the layer twice. The panel claims the chord now, exactly as
    /// it already claimed Delete, and a claim is asked once.
    testWidgets('the paste chord is claimed, never answered on the keyboard',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p);
      p.uiState.activePanel.value = Panel.effectControls;

      final source = p.layer.getEffects().single;
      p.uiState
          .copyEffectsToClipboard(p.layer.copyEffects(effects: [source.id()]));

      final other = p.uiState.selectedComp!.addSolidLayer();
      p.uiState.setSelection([other]);
      await tester.pumpAndSettle();

      await pasteChord(tester);
      expect(other.getEffects(), isEmpty,
          reason: 'the panel does not act on the chord itself — acting here '
              'and at the shell is what pasted twice');

      await pasteSelectionFrb(
          p.state, p.uiState, p.uiState.selectedComp, other);
      await tester.pumpAndSettle();
      expect(other.getEffects(), hasLength(1),
          reason: 'the shell asks the claim, and once asked is once pasted');
    });

    /// With nothing picked out of a stack the chord is **not** claimed: the
    /// shell copies the layer whole, as it always did.
    testWidgets('with no effect picked the copy chord falls through',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');

      var claimed = false;
      p.uiState.copyClaim = () {
        claimed = true;
        return true;
      };

      await mount(tester, p);
      p.uiState.activePanel.value = Panel.effectControls;

      expect(copySelectionFrb(p.uiState), isTrue,
          reason: 'the shell is still the one that answers');
      expect(claimed, isTrue);
      expect(p.uiState.clipboard.kind, isNull,
          reason: 'the panel took nothing it had not been asked for');
    });

    testWidgets('Enter renames the selected effect, and the name persists',
        (tester) async {
      // An effect instance can carry the user's own name. Enter on the
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

      // Escape throws the edit away. Enter, clicking away and an
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

    // Depth of field's folded aperture: the twirls, the greyed rows and
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

    /// **The mask-path row**: one of *this layer's* masks, by name,
    /// with **First mask** as the unset entry.
    ///
    /// The row is mounted directly with a synthetic parameter rather than
    /// through one of the three built-ins that now declare one (Scribble,
    /// Stroke and Vegas's Mask/Path source), because what is under test
    /// is the **control**, not any effect: the entries it offers, the words it
    /// uses, and that picking one reaches the document as a `MaskPath` value
    /// rather than something else — against a real layer with real masks in a
    /// real document. Which built-ins declare the row is asserted engine-side,
    /// in `a_mask_path_row_declares_itself_and_defaults_to_the_first_mask`.
    /// **The fixed columns** (docs/15 §12A.3). Every row lays out on the
    /// same x positions, and the keyframe-navigation slot is reserved whether or
    /// not the property is animated — so a stopwatch being switched on adds
    /// three buttons without shifting the label under them.
    ///
    /// This is the shape the panel did NOT have: the navigator used to appear
    /// inside the name column and shove the label sideways, so twirling a
    /// stack open and keying one property re-ragged the whole list.
    group('the fixed columns', () {
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

      /// **The mockups' heights are canonical** (docs/15 §12A.6). A
      /// parameter row occupies 26 whatever control it carries, a section
      /// heading 24, and a value well 20 — measured rather than trusted,
      /// because a stack whose rows step in and out is exactly the fault the
      /// fixed content box was introduced to settle.
      testWidgets('rows, headings and wells are built to the mockup heights',
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
            reason: 'a parameter row occupies 27 under Regular');

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

      /// The other density column. Compact takes a pixel off the row pitch and
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

    /// The same rule on the crosshair that picks a *position*: an animated
    /// Centre keeps its path, and the pick keys both axes at the playhead.
    testWidgets('picking a point keys both axes rather than flattening them',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final id = p.layer.getEffects().single.id();

      BridgeKeyframe key(int frame, double value) => BridgeKeyframe(
            time: p.uiState.selectedComp!.timeOfFrame(frame: frame),
            value: value,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          );
      BridgeParamInfo axis(String id, String label) => BridgeParamInfo(
            id: id,
            label: label,
            kind: const BridgeParamKind.float(
                default_: 0,
                sliderMin: -1000,
                sliderMax: 1000,
                hardMin: null,
                hardMax: null),
            unit: BridgeUnit.px,
          );

      final written = <String, BridgeEffectValue>{};
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: EffectPointRowFrb(
          effectId: id,
          xParam: axis('centre_x', 'Centre x'),
          yParam: axis('centre_y', 'Centre y'),
          xValue: BridgeEffectValue.float(
              BridgeScalar.keyframed([key(10, 100), key(20, 300)])),
          yValue: BridgeEffectValue.float(
              BridgeScalar.keyframed([key(10, 50), key(20, 150)])),
          comp: p.uiState.selectedComp!,
          playheadFrame: 0,
          onSeek: (_) {},
          onWrite: (_, param, v) => written[param] = v,
          onLive: (_, __, ___) {},
        ),
      ));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(ValueKey<String>('dropper-fx-$id-centre_x')));
      await tester.pumpAndSettle();
      final arm = p.uiState.dropper.value;
      expect(arm, isNotNull, reason: 'the crosshair armed');
      arm!.onPick(const DropperSample(
          r: 0,
          g: 0,
          b: 0,
          depth: 0,
          x: 8,
          y: 8,
          xFrac: 0.5,
          yFrac: 0.25,
          region: 1));
      await tester.pumpAndSettle();

      for (final id in ['centre_x', 'centre_y']) {
        final value = written[id];
        expect(value, isA<BridgeEffectValue_Float>(),
            reason: '$id was written');
        final scalar = (value as BridgeEffectValue_Float).field0;
        expect(scalar, isA<BridgeScalar_Keyframed>(),
            reason: '$id keeps its keyframes through a pick');
        expect((scalar as BridgeScalar_Keyframed).field0.length, 3,
            reason: '$id keeps both keys and gains one at the playhead');
      }
    });

    // -----------------------------------------------------------------------
    // The unit rider and the vector-pair chain (docs/15 §12A.3).
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
    /// map got wrong. Radial blur's Centre carried this test until every
    /// centre became px@comp; Amplitude is the case that remains (Shake's is a
    /// distance, Lightning's a share of its own bolt).
    testWidgets('amplitude reads px on one effect and % on another',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'shake');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);
      final shakeAmp = find.ancestor(
        of: find.text('Amplitude'),
        matching: find.byType(EffectParamRowFrb),
      );
      expect(
        find.descendant(of: shakeAmp, matching: find.text('px')),
        findsOneWidget,
        reason: "Shake's Amplitude is px@comp",
      );

      final second = withLayer();
      second.layer.addEffect(name: 'lightning');
      second.uiState.model.refresh();
      await mount(tester, second, transform: false);
      final boltAmp = find.ancestor(
        of: find.text('Amplitude'),
        matching: find.byType(EffectParamRowFrb),
      );
      expect(
        find.descendant(of: boltAmp, matching: find.text('%')),
        findsOneWidget,
        reason: "Lightning's Amplitude is a share of its own bolt",
      );
      expect(
        find.descendant(of: boltAmp, matching: find.text('px')),
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

    /// Both halves keyed, the well's edit is a key at the playhead, so the
    /// other half takes one there too, at the ratio the pair reads on that
    /// frame. Stretching its whole curve instead left the two halves agreeing
    /// at the playhead and nowhere else.
    testWidgets(
        'a chained pair keyed on both halves plants the other key at the playhead',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'lens_flare');
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      List<BridgeKeyframe> keysOf(String param) => ((p.layer
                  .getInfo()
                  .effects
                  .single
                  .values
                  .firstWhere((e) => e.id == param)
                  .value as BridgeEffectValue_Float)
              .field0 as BridgeScalar_Keyframed)
          .field0;
      BridgeKeyframe key(int seconds, double value) => BridgeKeyframe(
            time: BridgeRational(num: seconds, den: 1),
            value: value,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          );

      // x and y both ramp, at 2:1.
      final stack = p.layer.getEffects();
      stack.single
        ..setValue(
            id: 'light_x',
            value: BridgeEffectValue.float(
                BridgeScalar.keyframed([key(0, 100), key(2, 300)])))
        ..setValue(
            id: 'light_y',
            value: BridgeEffectValue.float(
                BridgeScalar.keyframed([key(0, 50), key(2, 150)])));
      p.layer.setEffects(effects: stack);
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(ValueKey<String>('fx-pair-link-$id-light_x')));
      await tester.pumpAndSettle();

      // Halfway along, where neither half has a key: x reads 200, y 100.
      p.uiState.playheadFrame.value = p.uiState.selectedComp!
          .frameAtTime(time: const BridgeRational(num: 1, den: 1));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey<String>('fx-float-$id-light_x')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText).first, '400');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      final x = keysOf('light_x');
      final y = keysOf('light_y');
      expect([for (final k in x) k.value], [100.0, 400.0, 300.0],
          reason: 'the well took a key at the playhead');
      expect([for (final k in y) k.value], [50.0, 200.0, 150.0],
          reason: 'so did the other half, at the 2:1 the pair reads there');
      expect([for (final k in y) k.time], [for (final k in x) k.time]);
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

    testWidgets('the enable switch bypasses every picked effect',
        (tester) async {
      final p = withLayer();
      for (final name in ['blur', 'vignette']) {
        p.layer.addEffect(name: name);
      }
      await mount(tester, p);
      final stack = p.layer.getEffects();
      p.uiState.setEffectSelection(p.layer, [for (final e in stack) e.id()]);
      await tester.pump();

      await tester.tap(
          find.byKey(ValueKey<String>('fx-enabled-hit-${stack.first.id()}')));
      await tester.pump();

      expect([
        for (final e in p.layer.getEffects()) e.getInfo().enabled
      ], [
        false,
        false
      ], reason: 'both took the clicked card\'s new state');
    });

    testWidgets('the × removes every picked effect', (tester) async {
      final p = withLayer();
      for (final name in ['blur', 'vignette', 'invert']) {
        p.layer.addEffect(name: name);
      }
      await mount(tester, p);
      final stack = p.layer.getEffects();
      p.uiState.setEffectSelection(p.layer, [stack[0].id(), stack[1].id()]);
      await tester.pump();

      await tester
          .tap(find.byKey(ValueKey<String>('fx-remove-${stack[0].id()}')));
      await tester.pump();

      expect(
          [for (final e in p.layer.getEffects()) e.name()], [stack[2].name()],
          reason: 'the two picked went; the unpicked one stayed');
    });

    /// The other half of the rule: a card that is **not** in the picked run is
    /// about itself, exactly as Copy already treated it.
    testWidgets('a command on an unpicked card acts on that card alone',
        (tester) async {
      final p = withLayer();
      for (final name in ['blur', 'vignette']) {
        p.layer.addEffect(name: name);
      }
      await mount(tester, p);
      final stack = p.layer.getEffects();
      p.uiState.setEffectSelection(p.layer, [stack[0].id()]);
      await tester.pump();

      await tester
          .tap(find.byKey(ValueKey<String>('fx-remove-${stack[1].id()}')));
      await tester.pump();

      expect(
          [for (final e in p.layer.getEffects()) e.name()], [stack[0].name()]);
    });

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

    /// **Click and drag across the switches** (item 6.2). The first switch
    /// decides what the run becomes; every switch the pointer crosses takes
    /// that state rather than flipping its own, so a mixed run comes out even
    /// instead of coming out mixed the other way round.
    testWidgets('a drag across the enable switches sets them all to the first',
        (tester) async {
      final p = withLayer();
      for (final name in ['blur', 'vignette', 'invert']) {
        p.layer.addEffect(name: name);
      }
      // The middle one already off: the run the drag meets is mixed.
      final staged = p.layer.getEffects();
      p.layer.setEffectEnabled(effect: staged[1], enabled: false);
      await mount(tester, p, transform: false);
      final stack = p.layer.getEffects();
      expect([for (final e in stack) e.getInfo().enabled], [true, false, true]);

      Finder hit(int i) =>
          find.byKey(ValueKey<String>('fx-enabled-hit-${stack[i].id()}'));
      final drag = await tester.startGesture(tester.getCenter(hit(0)));
      await tester.pump();
      await drag.moveTo(tester.getCenter(hit(1)));
      await tester.pump();
      await drag.moveTo(tester.getCenter(hit(2)));
      await tester.pump();
      await drag.up();
      await tester.pumpAndSettle();

      expect([for (final e in p.layer.getEffects()) e.getInfo().enabled],
          [false, false, false],
          reason: 'the one already off stayed off — the drag never flips '
              'anything to the opposite of what the first switch became');
    });

    /// **A selected run twirls together** (item 6.3). Having said that five
    /// effects are what you are working on, opening them one twirl at a time
    /// is five clicks to reach a state already asked for.
    testWidgets('twirling one picked effect twirls every picked effect',
        (tester) async {
      final p = withLayer();
      for (final name in ['blur', 'vignette', 'invert']) {
        p.layer.addEffect(name: name);
      }
      await mount(tester, p, transform: false);
      final stack = p.layer.getEffects();
      // The bottom two picked; the top one left out of it, and it is the one
      // that must not move.
      p.uiState.setEffectSelection(p.layer, [stack[1].id(), stack[2].id()]);
      await tester.pump();

      /// Whether that card is drawing rows — which is what open means. By
      /// card rather than by label: three effects share parameter names.
      Finder rowsIn(int card) => find.descendant(
            of: find.byKey(ValueKey<String>('fx-card-$card')),
            matching: find.byType(EffectParamRowFrb),
          );
      expect(rowsIn(0), findsWidgets, reason: 'each arrives open');
      expect(rowsIn(1), findsWidgets);

      await tester
          .tap(find.byKey(ValueKey<String>('fx-twirl-${stack[1].id()}')));
      await tester.pump();
      expect(rowsIn(1), findsNothing, reason: 'the one that was clicked shut');
      expect(rowsIn(2), findsNothing,
          reason: 'and so did the other picked one');
      expect(rowsIn(0), findsWidgets,
          reason: 'the unpicked effect kept its rows');

      // Opening again takes the run with it too, from either end of it.
      await tester
          .tap(find.byKey(ValueKey<String>('fx-twirl-${stack[2].id()}')));
      await tester.pump();
      expect(rowsIn(1), findsWidgets);
      expect(rowsIn(2), findsWidgets);
    });

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
      p.uiState.activePanel.value = Panel.effectControls;

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

    /// **Fronting another comp does not lose your place** (item 6.28). The
    /// read model rebinds to the new comp's layers, so the layer this panel
    /// is showing stops being in it while still existing perfectly well in
    /// the comp it belongs to. Its rows stay up until a layer is selected in
    /// the new comp; a layer that has genuinely gone is still the placeholder.
    testWidgets('fronting another comp keeps the stack that was on the panel',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p, transform: false);
      expect(heading('Gaussian blur'), findsOneWidget);

      final other = p.state.project!.newComposition(name: 'Other');
      p.uiState.setSelectedComp(other);
      p.uiState.model.refresh();
      await tester.pump();
      expect(heading('Gaussian blur'), findsOneWidget,
          reason: 'the layer is in the comp you came from, not gone');
      expect(find.textContaining('Select a layer'), findsNothing);

      // A layer selected in the new comp is what replaces it.
      final fresh = other.addSolidLayer();
      p.uiState.model.refresh();
      p.uiState.setSelection([fresh]);
      await tester.pump();
      expect(heading('Gaussian blur'), findsNothing);
      expect(find.textContaining('No effects'), findsOneWidget);

      // And a layer deleted out of its own comp is the placeholder, which is
      // the case this hold must not swallow.
      fresh.delete();
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.textContaining('Select a layer'), findsOneWidget);
    });

    /// **Layer pickers number their entries** (item 6.13): "2. Sky" — the
    /// layer's own place in the composition, so two layers sharing a name are
    /// still two entries you can tell apart. The number is data, not a
    /// phrase, so no string file knows about it.
    testWidgets('a layer picker lists entries by their place in the comp',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final comp = p.uiState.selectedComp!;
      comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p, transform: false);

      final id = p.layer.getEffects().single.id();
      await tester.tap(find.byKey(ValueKey<String>('fx-layer-$id-matte')));
      await tester.pumpAndSettle();

      final names = [for (final e in p.uiState.model.layers) e.info.name];
      expect(names, hasLength(2));
      // Entry one is the top layer of the comp, entry two the next — the
      // layer the effect sits on says so as well, which is why this
      // reads the prefix rather than the whole entry.
      expect(find.textContaining('1. ${names[0]}'), findsOneWidget);
      expect(find.textContaining('2. ${names[1]}'), findsOneWidget);
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

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
  }, skip: !engineAvailable);
}
