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
import 'package:lumit_flutter/state/clipboard.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/widgets/controls.dart';

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

    /// `Ctrl+Shift+P` was bound with nothing answering it. It asks the menu bar
    /// for the palette rather than building a list of commands of its own.
    testWidgets('Ctrl+Shift+P asks for the command palette', (tester) async {
      final p = await mount(tester);
      var asked = 0;
      p.uiState.paletteRequest.addListener(() => asked++);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyP);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();

      expect(asked, 1);
    });

    /// `Ctrl+Space` asks the same bar for the FX console (K-324) — and the
    /// bare space bar must still reach the transport, which is the thing a
    /// modified space bar is most likely to have broken.
    testWidgets('Ctrl+Space asks for the FX console, and space still plays',
        (tester) async {
      final p = await mount(tester);
      var console = 0;
      var play = 0;
      p.uiState.consoleRequest.addListener(() => console++);
      p.uiState.togglePlayRequest.addListener(() => play++);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();
      expect(console, 1);
      expect(play, 0, reason: 'the modified chord is not the transport');

      // The console that just opened owns the keyboard (K-328), so it is
      // closed before the bare space bar can mean the transport again.
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pumpAndSettle();
      expect(play, 1, reason: 'the bare space bar still plays');
      expect(console, 1);
    });

    /// With the console up, the keyboard is the console's (K-328): a
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

    /// `Mod`+arrow steps the playhead (K-282). The **bare** arrows do not: they
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

    testWidgets('Home and End go to the ends of the comp', (tester) async {
      final p = await mount(tester);
      final last = p.uiState.selectedComp!.durationFrames() - 1;

      await tester.sendKeyEvent(LogicalKeyboardKey.end);
      await tester.pump();
      expect(p.uiState.playheadFrame.value, last);

      await tester.sendKeyEvent(LogicalKeyboardKey.home);
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 0);
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
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();
      expect(comp.getLayers(), isEmpty);
      expect(p.uiState.selectedLayer.value, isNull,
          reason: 'the selection cannot outlive the layer');
    });

    /// **A finer selection gets Delete first (K-234).** A selected mask row is
    /// what the key is about, not the layer it sits on — and every key handler
    /// runs on every key, so the Timeline cannot claim the chord merely by
    /// handling it. The shell asks, and stands down when the answer is yes.
    testWidgets('Delete stands down when a panel claims it', (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      comp.addSolidLayer();
      p.uiState.selectedLayer.value = comp.getLayers().single;

      var claimed = 0;
      p.uiState.deleteClaim = () {
        claimed++;
        return true;
      };
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();
      expect(claimed, 1, reason: 'the shell asked before deleting');
      expect(comp.getLayers(), hasLength(1),
          reason: 'and left the layer alone');

      // A claim that declines gives the key back.
      p.uiState.deleteClaim = () => false;
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();
      expect(comp.getLayers(), isEmpty);
    });

    /// Alt+Shift+T does nothing now (K-200): it was a misremembering of AE's
    /// chord, and on Windows the OS steals it for the input-language switch
    /// anyway. It is unbound rather than kept as a second chord — Retime is
    /// not special — and anyone who wants it can bind it in Settings → Keymap.
    testWidgets('Alt+Shift+T is unbound and leaves the layer alone',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      p.uiState.selectedLayer.value = layer;

      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyT);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pump();

      expect(layer.getRetimeProperty(), isNull,
          reason: 'no binding, no Retime — the chord means nothing');
    });

    /// **Ctrl+Alt+T is the Retime chord** (K-197, narrowed to one by K-200):
    /// After Effects' own Time Remap chord, and one Windows cannot steal. On
    /// gives the layer a Retime; off removes the property rather than leaving
    /// a flattened curve behind.
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

    /// **The dialogue-kills-the-keyboard regression.** A modal is an *overlay*
    /// entry, so it sits outside the shell's `FocusScope` rather than inside it.
    /// A text field in one therefore takes focus out of the shell's subtree
    /// altogether, and when the entry is removed the focus it held dies with it
    /// — leaving the primary focus somewhere that is not under the shell, so the
    /// shell's key handler was never called again and *every* shortcut was dead
    /// until something inside the shell was clicked.
    ///
    /// It only became reachable when New composition grew a dialogue (K-180):
    /// make a comp, press space, nothing plays.
    testWidgets('the keyboard still works after a dialogue has been used',
        (tester) async {
      final p = await mount(tester);
      var asked = 0;
      p.uiState.togglePlayRequest.addListener(() => asked++);

      await tester.tap(find.byKey(const ValueKey<String>('menu-Composition')));
      await tester.pump();
      await tester.tap(find.text('Composition settings…'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('comp-name')), findsOneWidget);

      // Type into it, which is what moves focus into the overlay.
      await tester.tap(find.byKey(const ValueKey('comp-name')));
      await tester.pump();
      await tester.enterText(find.byKey(const ValueKey('comp-name')), 'Scene');
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await tester.pump();
      expect(asked, 1,
          reason: 'the shell has the keyboard back once the dialogue is gone');
    });

    /// **Ctrl+S did nothing.** `file.save` was in the keymap from the day the
    /// keymap came back, but the shell's dispatch had no case for it — so the
    /// chord resolved to an action nobody ran and the status line went on
    /// saying "Unsaved changes" (K-203). Saved to a path already, so no picker
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

    /// B and N set the work area's ends from the playhead (docs/07 §15). Bound
    /// since K-199 and dispatched by nobody until K-203, which is why the work
    /// area read as unimplemented.
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

    /// Numbered markers (K-254). The pairing is the whole feature: the chord
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

    /// A digit with no marker behind it is a key without a meaning yet, not an
    /// error — and it must not move the playhead anywhere.
    testWidgets('a digit with no marker does nothing', (tester) async {
      final p = await mount(tester);
      p.uiState.playheadFrame.value = 15;
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.digit7);
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 15);
    });

    /// **`Ctrl+C` on a selected layer copied nothing** (K-300). Cut, copy and
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

    /// **A copy has to leave a trace the machine can see** (K-302). The layer
    /// and effect clipboard was in-app only, so copying a layer and pasting
    /// into a text editor produced nothing — which reads exactly like Copy
    /// having done nothing at all, and was the first thing the owner tried.
    testWidgets('a copied layer is on the system clipboard too',
        (tester) async {
      // The plugin channel is not wired in a widget test; stand in for the
      // platform's clipboard so what Lumit writes can be read back.
      String? written;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'Clipboard.setData') {
            written = (call.arguments as Map)['text'] as String?;
          }
          if (call.method == 'Clipboard.getData') return {'text': written};
          return null;
        },
      );
      addTearDown(() => tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null));

      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      p.uiState.setSelection([comp.addSolidLayer()]);
      await tester.pump();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyC);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();

      expect(written, isNotNull,
          reason: 'something reached the system clipboard');
      expect(lumitDocumentKind(written!), ClipboardKind.layer,
          reason: 'and it is the layer document, which another Lumit window '
              'can take straight back off it');

      // The round trip: an empty tray, a document on the system clipboard —
      // the state a second Lumit window is in — still pastes.
      p.uiState.clipboard.clear();
      expect(await p.uiState.adoptSystemClipboard(), isTrue);
      expect(p.uiState.clipboard.kind, ClipboardKind.layer);
    });

    /// With an effect picked out of a stack, the chord takes *that*, not the
    /// layer under it (K-300) — the finest selection wins, exactly as Delete
    /// has done since K-234.
    testWidgets('Ctrl+C takes the picked effect, not the layer it sits on',
        (tester) async {
      final p = await mount(tester);
      final comp = p.uiState.selectedComp!;
      final layer = comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      layer.addEffect(name: 'invert');
      p.uiState.setSelection([layer]);
      final second = layer.getEffects()[1];
      p.uiState.setEffectSelection(layer, [second.id()]);
      await tester.pump();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyC);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();

      expect(p.uiState.clipboard.kind, ClipboardKind.effects);
      final bare = comp.addSolidLayer();
      bare.pasteEffects(text: p.uiState.clipboard.text!, atFrame: 0);
      expect(bare.getEffects(), hasLength(1));
      expect(bare.getEffects().single.name(), second.name(),
          reason: 'the effect that was picked, and only it');
    });

    /// `M` still reveals Masks in the Timeline, which is why the plain marker
    /// key is `Shift+M` (K-254).
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
      p.uiState.activePanel.value = Panel.viewer;
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
      p.uiState.activePanel.value = Panel.viewer;
      await tester.pump();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.equal);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();

      expect(p.uiState.viewerZoomRequest.value?.$2, ViewerZoomCommand.zoomIn);
    });

    /// Moving between panels without the mouse (docs/07 §15, "Panels"). These
    /// three bindings were in the shipped keymap with nothing behind them —
    /// and could not have reached anything, because the Panels context is one
    /// no panel *is*, so the focused-panel lookup never asked for it.
    testWidgets('Ctrl+F6 walks the focus ring round the arrangement',
        (tester) async {
      final p = await mount(tester);
      final order = panelsIn(p.uiState.split);
      expect(order.length, greaterThan(2), reason: 'a ring needs somewhere to go');
      expect(p.uiState.activePanel.value, isNull);

      Future<void> cycle({bool back = false}) async {
        await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
        if (back) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
        await tester.sendKeyEvent(LogicalKeyboardKey.f6);
        if (back) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
        await tester.pump();
      }

      await cycle();
      expect(p.uiState.activePanel.value, order.first,
          reason: 'with nothing focused, a cycle begins at the beginning');
      await cycle();
      expect(p.uiState.activePanel.value, order[1]);
      await cycle(back: true);
      expect(p.uiState.activePanel.value, order.first);
      // Past the beginning it wraps rather than stopping.
      await cycle(back: true);
      expect(p.uiState.activePanel.value, order.last);
    });

    /// `Ctrl+F` is only meaningful where there is a field to put the cursor
    /// in, and it must never focus two at once.
    testWidgets('Ctrl+F focuses the search box of the panel that has one',
        (tester) async {
      final p = await mount(tester);

      Future<void> pressCtrlF() async {
        await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
        await tester.sendKeyEvent(LogicalKeyboardKey.keyF);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
        await tester.pump();
      }

      // A panel with no search box leaves the chord alone.
      p.uiState.activePanel.value = Panel.timeline;
      await tester.pump();
      await pressCtrlF();
      expect(p.uiState.panelSearchRequest.value, 0);

      p.uiState.activePanel.value = Panel.project;
      await tester.pump();
      await pressCtrlF();
      expect(p.uiState.panelSearchRequest.value, 1);

      final field = tester.widget<HouseTextField>(
          find.byKey(const ValueKey('project-search')));
      expect(field.focusNode?.hasFocus, isTrue,
          reason: 'the cursor is in the Project panel\'s search field');
    });
  }, skip: !engineAvailable);
}
