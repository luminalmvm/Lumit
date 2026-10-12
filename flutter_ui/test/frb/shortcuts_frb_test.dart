// The shell's keyboard shortcuts, against the real engine.
//
// The port dropped the previous shell's key handler entirely, so nothing on the
// keyboard did anything — space did not play, and Ctrl+Z did not undo. These
// drive `LumitAppView` itself rather than a panel, because the handler is the
// shell's and a panel-level test would not prove it is reachable.

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/clipboard.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/viewer_view.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Shell shortcuts (frb)', () {
    Future<({LumitState state, LumitUiState uiState})> mount(
        WidgetTester tester) async {
      // A desktop-sized window. The whole shell is mounted here, and at the
      // 800x600 default several panel toolbars are narrower than their controls
      // and overflow — a real defect at that width, but a pre-existing one and
      // not what these tests are about (recorded in docs/TODO.md).
      tester.view.physicalSize = const Size(1800, 1100);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      await tester.pumpWidget(hostPanel(
        child: const LumitAppView(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      return p;
    }

    testWidgets('space asks the transport to toggle', (tester) async {
      final p = await mount(tester);
      var asked = 0;
      p.uiState.togglePlayRequest.addListener(() => asked++);

      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pump();
      expect(asked, 1, reason: 'space reached the transport');
    });

    /// With the console up, the keyboard is the console's: a
    /// keystroke aimed at its search box must never also run a shell command
    /// — the exact bug was typing over the open console renaming and adding
    /// layers underneath it.
    testWidgets('with the console open, typing cannot run shell commands',
        (tester) async {
      final p = await mount(tester);
      var play = 0;
      p.uiState.togglePlayRequest.addListener(() => play++);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();

      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pumpAndSettle();
      expect(play, 0, reason: 'the space bar is typing, not the transport');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pumpAndSettle();
      expect(play, 1, reason: 'closed, the keys are the shell again');
    });

    /// **The recurring space-bar funeral.** Menus, popups and the palette all
    /// live in the Overlay outside the shell's focus scope; any of them could
    /// walk focus away for good, and every shortcut died until something was
    /// clicked. Shortcuts are global now — they work with focus parked
    /// nowhere at all, which is exactly the broken state this reproduces.
    testWidgets('space still toggles when focus has wandered off',
        (tester) async {
      final p = await mount(tester);
      var asked = 0;
      p.uiState.togglePlayRequest.addListener(() => asked++);

      // The broken state: nothing in the app holds focus.
      FocusManager.instance.primaryFocus?.unfocus();
      await tester.pump();

      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pump();
      expect(asked, 1,
          reason: 'shortcuts must not depend on where focus is sitting');
    });

    /// `Mod`+arrow steps the playhead. The **bare** arrows do not: they
    /// belong to whatever has focus — a list moving its highlight, a field
    /// moving its cursor — which is the whole reason the step took a modifier.
    testWidgets('Ctrl and the arrows step the playhead within the comp',
        (tester) async {
      final p = await mount(tester);

      Future<void> step(LogicalKeyboardKey arrow) async {
        await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
        await tester.sendKeyEvent(arrow);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
        await tester.pump();
      }

      await step(LogicalKeyboardKey.arrowRight);
      expect(p.uiState.playheadFrame.value, 1);

      await step(LogicalKeyboardKey.arrowLeft);
      expect(p.uiState.playheadFrame.value, 0);

      // A frame before the comp is not a frame.
      await step(LogicalKeyboardKey.arrowLeft);
      expect(p.uiState.playheadFrame.value, 0);

      // And a bare arrow leaves the playhead where it is.
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 0,
          reason: 'the bare arrows are free for whatever has focus');
    });

    testWidgets('Ctrl+Z undoes and Ctrl+Shift+Z redoes', (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      comp.addSolidLayer();
      expect(comp.getLayers(), hasLength(1));

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
      await tester.pump();
      expect(comp.getLayers(), isEmpty, reason: 'Ctrl+Z undid the layer');

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
      await tester.pump();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      expect(comp.getLayers(), hasLength(1),
          reason: 'and Ctrl+Shift+Z put it back');
    });

    testWidgets('Delete removes the selected layer, and nothing without one',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      comp.addSolidLayer();

      // Nothing selected: the key must be inert rather than deleting something
      // the user did not point at.
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();
      expect(comp.getLayers(), hasLength(1));

      p.uiState.selectedLayer.value = comp.getLayers().single;
      p.uiState.activePane.value = Panel.timeline.pane();
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();
      expect(comp.getLayers(), isEmpty);
      expect(p.uiState.selectedLayer.value, isNull,
          reason: 'the selection cannot outlive the layer');
    });

    /// Delete in Effect controls never reaches the layer, picked or not.
    testWidgets('Delete in Effect controls leaves the layer', (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      final layer = comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      layer.addEffect(name: 'invert');
      p.uiState.selectedLayer.value = comp.getLayers().single;
      p.uiState.activePane.value = Panel.effectControls.pane();
      await tester.pump();

      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();
      expect(comp.getLayers(), hasLength(1),
          reason: 'nothing picked is not a reason to delete the layer');

      final stack = comp.getLayers().single.getEffects();
      p.uiState.setEffectSelection(comp.getLayers().single, [stack[0].id()]);
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();
      expect(comp.getLayers(), hasLength(1), reason: 'the layer stayed');
      expect([
        for (final e in comp.getLayers().single.getEffects()) e.name()
      ], [
        stack[1].name()
      ], reason: 'and the picked effect went');
    });

    /// **Ctrl+Alt+T is the Retime chord**: After Effects' own Time Remap
    /// chord, and one Windows cannot steal. On gives the layer a Retime; off
    /// removes the property rather than leaving a flattened curve behind.
    testWidgets('Ctrl+Alt+T toggles the selected layer\'s Retime',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      p.uiState.selectedLayer.value = layer;

      Future<void> press() async {
        await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
        await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
        await tester.sendKeyEvent(LogicalKeyboardKey.keyT);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
        await tester.pump();
      }

      await press();
      expect(layer.getRetimeProperty(), isNotNull);
      await press();
      expect(layer.getRetimeProperty(), isNull);
    });

    /// After Effects' Alt+Shift+P keys Position on the selected layer and a
    /// second press takes the key away again, and Shift+= and Shift+- step
    /// the layer's blend mode.
    testWidgets('Alt+Shift+P keys Position, Shift+= steps the blend mode',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      final layer = comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.playheadFrame.value = 12;
      p.uiState.model.refresh();
      await tester.pump();

      Future<void> press(LogicalKeyboardKey key, {bool alt = false}) async {
        if (alt) await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
        await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
        await tester.sendKeyEvent(key);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
        if (alt) await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
        await tester.pump();
      }

      await press(LogicalKeyboardKey.keyP, alt: true);
      final keyed = layer.getTransform().positionX;
      expect(keyed, isA<BridgeScalar_Keyframed>());
      expect(
          comp.frameAtTime(
              time: (keyed as BridgeScalar_Keyframed).field0.single.time),
          12,
          reason: 'one key, on the playhead');
      expect(layer.getTransform().positionY, isA<BridgeScalar_Keyframed>(),
          reason: 'both axes of the row');

      await press(LogicalKeyboardKey.keyP, alt: true);
      expect(layer.getTransform().positionX, isA<BridgeScalar_Static>(),
          reason: 'the second press takes the key away');

      expect(layer.getBlend(), 0);
      await press(LogicalKeyboardKey.equal);
      expect(layer.getBlend(), 1);
      await press(LogicalKeyboardKey.minus);
      expect(layer.getBlend(), 0);
    });

    /// Layer ▸ New from the keyboard, on After Effects' chords: Ctrl+Y makes
    /// a Solid and Ctrl+Alt+Y an Adjustment layer, in the fronted comp.
    testWidgets('Ctrl+Y and Ctrl+Alt+Y add a Solid and an Adjustment layer',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      expect(comp.getLayers(), isEmpty);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyY);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      expect(comp.getLayers().single.getKind(), BridgeLayerKind.solid);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyY);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      // With nothing selected a new layer goes on top of the stack, so the
      // Adjustment reads first.
      expect(comp.getLayers().map((l) => l.getKind()),
          [BridgeLayerKind.adjustment, BridgeLayerKind.solid]);

      // With a layer selected the next one lands directly above it, exactly as
      // the menu row does.
      p.uiState.setSelection([comp.getLayers().last]);
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyY);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      expect(comp.getLayers().map((l) => l.getKind()), [
        BridgeLayerKind.adjustment,
        BridgeLayerKind.solid,
        BridgeLayerKind.solid,
      ], reason: 'above the selected layer, not at the top');
    });

    /// Otherwise every letter typed into a layer name would also be a command.
    ///
    /// Driven through the Timeline's own search field, which lives inside the
    /// shell exactly as a rename field does — a field mounted *beside* the
    /// shell would not exercise the gate at all, since its keys never reach the
    /// shell's handler in the first place.
    testWidgets('a focused text field keeps its keys', (tester) async {
      final p = await mount(tester);
      var asked = 0;
      p.uiState.togglePlayRequest.addListener(() => asked++);

      final search = find.byKey(const ValueKey('tl-search'));
      expect(search, findsOneWidget, reason: 'the Timeline is in the shell');
      await tester.tap(search);
      await tester.pump();

      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pump();
      expect(asked, 0,
          reason: 'the space went into the field, not the transport');

      // And once the field gives focus back, the key is a command again.
      FocusManager.instance.primaryFocus?.unfocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pump();
      expect(asked, 1);
    });

    /// **Ctrl+S did nothing.** `file.save` was in the keymap from the day the
    /// keymap came back, but the shell's dispatch had no case for it — so the
    /// chord resolved to an action nobody ran and the status line went on
    /// saying "Unsaved changes". Saved to a path already, so no picker
    /// is involved: this is about the dispatch, not the dialogue.
    testWidgets('Ctrl+S saves the project', (tester) async {
      final p = await mount(tester);
      final dir = Directory.systemTemp.createTempSync('lumit-save');
      addTearDown(() => dir.deleteSync(recursive: true));
      // Off the fake clock: a bridge Future only completes on the real event
      // loop (which is also why the chord below is settled, not pumped).
      await tester.runAsync(
          () => p.state.project!.save(path: '${dir.path}/scene.lumit'));

      p.uiState.selectedComp!.addSolidLayer();
      p.state.notifyDocumentChanged();
      await tester.pump();
      expect(p.state.project!.isDirty(), isTrue, reason: 'there is work to lose');

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyS);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await settleFrb(tester, until: () => !p.state.project!.isDirty());

      expect(p.state.project!.isDirty(), isFalse,
          reason: 'the chord reached the same save the File menu runs');
    });

    /// B and N set the work area's ends from the playhead (docs/07 §15). They
    /// were bound but dispatched by nobody, which is why the work area read as
    /// unimplemented.
    testWidgets('B and N set the work area from the playhead', (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      expect(comp.getWorkArea(), isNull, reason: 'a new comp has none set');

      p.uiState.playheadFrame.value = 12;
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.keyB);
      await tester.pump();
      expect(comp.frameAtTime(time: comp.getWorkArea()!.inPoint), 12);

      p.uiState.playheadFrame.value = 30;
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.keyN);
      await tester.pump();
      final work = comp.getWorkArea()!;
      expect(comp.frameAtTime(time: work.inPoint), 12,
          reason: 'setting the end leaves the start alone');
      expect(comp.frameAtTime(time: work.outPoint), 30);
    });

    /// Numbered markers. The pairing is the whole feature: the chord
    /// that marks a moment is the key that goes back to it, so both halves are
    /// asserted together — a set that does not return is not the feature.
    testWidgets('Shift+1 sets marker 1 and the bare 1 returns to it',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;

      p.uiState.playheadFrame.value = 24;
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();

      final marker = comp.getMarkers().single;
      expect(marker.label, '1', reason: 'the digit is what the marker says');
      expect(comp.frameAtTime(time: marker.time), 24);

      p.uiState.playheadFrame.value = 0;
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 24,
          reason: 'the bare digit went back to the marker');
    });

    /// **`Ctrl+C` on a selected layer copied nothing.** Cut, copy and
    /// paste had menu rows and no chord in the keymap at all, and no case in
    /// the shell's handler either — so the three keys everyone reaches for
    /// first did nothing, and the only way to copy a layer was the Edit menu.
    testWidgets('Ctrl+C copies the selected layer, Ctrl+V pastes it',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      final layer = comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      await tester.pump();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyC);
      await tester.pump();
      expect(p.uiState.clipboard.kind, ClipboardKind.layer,
          reason: 'the chord reached the same call the Edit menu makes');

      await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();
      expect(comp.getLayers(), hasLength(2),
          reason: 'and Ctrl+V put the copy back into the composition');
    });

    /// `M` still reveals Masks in the Timeline, which is why the plain marker
    /// key is `Shift+M`.
    testWidgets('Shift+M drops a marker at the playhead', (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;

      p.uiState.playheadFrame.value = 9;
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyM);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();

      expect(comp.getMarkers(), hasLength(1));
      expect(comp.getMarkers().single.label, isEmpty);
      expect(comp.frameAtTime(time: comp.getMarkers().single.time), 9);
    });

    /// The Viewer's own chords (docs/07 §15). They are scoped to the Viewer
    /// context, so the panel has to be the active one for them to mean
    /// anything at all — which is the half of this that a Global-context test
    /// would not prove.
    testWidgets('Ctrl+J and its siblings set the preview resolution',
        (tester) async {
      final p = await mount(tester);
      p.uiState.activePane.value = Panel.viewer.pane();
      await tester.pump();

      Future<void> chord(List<LogicalKeyboardKey> modifiers,
          LogicalKeyboardKey key) async {
        for (final m in modifiers) {
          await tester.sendKeyDownEvent(m);
        }
        await tester.sendKeyEvent(key);
        for (final m in modifiers.reversed) {
          await tester.sendKeyUpEvent(m);
        }
        await tester.pump();
      }

      await chord(
        [LogicalKeyboardKey.controlLeft, LogicalKeyboardKey.shiftLeft],
        LogicalKeyboardKey.keyJ,
      );
      expect(p.uiState.previewResolution, PreviewResolution.half);

      await chord([LogicalKeyboardKey.controlLeft], LogicalKeyboardKey.keyJ);
      expect(p.uiState.previewResolution, PreviewResolution.full);
    });

    /// The magnification chords do not zoom here — they *ask* the Viewer to,
    /// because "fit" is a rule only the panel can resolve.
    testWidgets('Ctrl+= asks the Viewer for a magnification', (tester) async {
      final p = await mount(tester);
      p.uiState.activePane.value = Panel.viewer.pane();
      await tester.pump();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.equal);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();

      expect(p.uiState.viewerZoomRequest.value?.$2, ViewerZoomCommand.zoomIn);
    });

    /// **Slide and trim are two key pairs** (A8, docs/07 §4.4). `[` and `]`
    /// put the layer's in or out point on the playhead by **moving the whole
    /// layer**, and a layer's keyframes are timed against its own clock — they
    /// reach the composition's through the start offset, which travels
    /// with a move — so the animation goes with the bar. `Alt` makes it a trim:
    /// one edge moves over content that stays put, and every keyframe is
    /// exactly where it was.
    ///
    /// The pair reads as one claim and is asserted as one: a `[` that carried
    /// no keyframes, or an `Alt+[` that carried them, would each be the whole
    /// distinction gone.
    testWidgets('[ slides the layer with its keyframes, Alt+[ trims without',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      final layer = comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [10, 40])
            BridgeKeyframe(
              time: comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await tester.pump();

      List<int> keyFrames() => [
            for (final k
                in (layer.getTransform().opacity as BridgeScalar_Keyframed)
                    .field0)
              comp.frameAtTime(time: k.time)
          ];
      int inFrame() => comp.frameAtTime(time: layer.getSpan().inPoint);
      int outFrame() => comp.frameAtTime(time: layer.getSpan().outPoint);

      expect(inFrame(), 0);
      final was = outFrame();
      expect(keyFrames(), [10, 40]);

      // `[` — the whole layer moves so its head lands on the playhead, and the
      // animation comes with it.
      p.uiState.playheadFrame.value = 20;
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.bracketLeft);
      await tester.pump();
      expect(inFrame(), 20);
      expect(outFrame(), was + 20, reason: 'the tail travelled the same way');
      expect(keyFrames(), [30, 60], reason: 'the keyframes slid with the bar');

      // `Alt+[` — the head is cut back to the playhead over content that has
      // not moved, so the keys stay where they are on the comp's clock.
      p.uiState.playheadFrame.value = 25;
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.bracketLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pump();
      expect(inFrame(), 25, reason: 'the head was trimmed to the playhead');
      expect(outFrame(), was + 20, reason: 'and the tail was left alone');
      expect(keyFrames(), [30, 60], reason: 'a trim moves no keyframes');
    });

    /// Every one of these was in the shipped keymap with nothing answering it:
    /// the chord looked up an action the shell's switch had no case for, so it
    /// fell through and the key did nothing at all.
    group('the navigation chords the table shipped', () {
      Future<void> chord(WidgetTester tester, LogicalKeyboardKey key,
          {bool shift = false}) async {
        if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
        await tester.sendKeyEvent(key);
        if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
        await tester.pump();
      }

      testWidgets('X in the Timeline turns the eye off for the selection',
          (tester) async {
        final p = await mount(tester);
        final comp = p.uiState.selectedComp!;
        final a = comp.addSolidLayer();
        final b = comp.addSolidLayer();
        p.uiState.setSelection([a, b]);
        p.uiState.activePane.value = Panel.timeline.pane();
        p.uiState.model.refresh();
        await tester.pump();

        await chord(tester, LogicalKeyboardKey.keyX);
        expect(a.getSwitches().visible, isFalse);
        expect(b.getSwitches().visible, isFalse,
            reason: 'the whole selection, not the primary alone');
      });
    });
  }, skip: !engineAvailable);
}
