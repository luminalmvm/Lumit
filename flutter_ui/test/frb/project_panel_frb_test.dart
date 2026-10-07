// The Project panel on frb, tested against the real engine.
//
// These are the ported equivalents of the 12 v0 tests that lived in
// project_placement_test.dart, section_d_test.dart and final_sweep_test.dart,
// plus coverage for three things v0 never asserted at all: the folder tree, the
// per-depth indent, and the row keys.
//
// Every document operation here is genuine — see frb_test_support.dart for why
// these are integration tests rather than fake-bridge unit tests.

import 'dart:io';
import 'dart:typed_data';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart'
    show FootageReference, LumitMediaStatus;
import 'package:lumit_flutter/src/rust/api/project_item.dart'
    show ItemReference_Folder, ItemReference_Footage;
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeLayerKind;
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/drag_payloads.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart'
    show HouseTextField, LumitTooltip;

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Project panel (frb)', () {
    /// A tree row's name text, as distinct from the info header's copy of it:
    /// selecting an item mirrors its name into the header, so a bare
    /// `find.text` goes ambiguous the moment anything is selected. The rows
    /// live in the panel's ListView; the header does not.
    Finder rowText(String name) =>
        find.descendant(of: find.byType(ListView), matching: find.text(name));

    /// A genuine double-click on a row — the gesture that **opens** it.
    ///
    /// [kDoubleTapMinTime] between the two, which is Flutter's own floor for
    /// calling a pair of taps a double tap, and nothing more: the open must
    /// land on the second click's own release, with none of the 300ms window
    /// waited out afterwards.
    Future<void> doubleClick(WidgetTester tester, Finder target) async {
      final centre = tester.getCenter(target);
      await tester.tapAt(centre);
      await tester.pump(kDoubleTapMinTime);
      await tester.tapAt(centre);
      await tester.pump();
    }

    /// Hold [colour] in the swatch filter, the way the pointer does it:
    /// the well's one square opens the eight-colour picker, and the
    /// picked chip is the filter. `null` picks the neutral chip, which shows
    /// everything again.
    Future<void> pickFilterColour(WidgetTester tester, int? colour) async {
      await tester.tap(find.byKey(const ValueKey('project-label-filter')));
      await tester.pumpAndSettle();
      await tester.tap(
          find.byKey(ValueKey<String>('project-filter-chip-${colour ?? 0}')));
      await tester.pumpAndSettle();
    }

    /// **Double-clicking** footage opens New composition on it:
    /// footage has no window of its own, and the thing wanted from a clip just
    /// double-clicked is a comp to put it in, already its size, rate and
    /// length. Renaming footage moved to the row menu with this.
    testWidgets(
        'clicking footage selects it, and a double-click makes a comp of it',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // The first click selects on its down stroke; the double-click opens the
      // dialogue, after the media has been probed.
      await doubleClick(tester, rowText('shot.mov'));
      await settleFrb(
        tester,
        until: () =>
            find.byKey(const ValueKey('comp-apply')).evaluate().isNotEmpty,
      );

      expect(find.text('NEW COMPOSITION'), findsWidgets);
      expect(find.byKey(const ValueKey('rename-field')), findsNothing,
          reason: 'a double-click on footage is not a rename any more');

      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      final comp = p.uiState.selectedComp;
      expect(comp, isNotNull, reason: 'the new comp is fronted');
      expect(comp!.getLayers(), hasLength(1),
          reason: 'the clip it was made from is in it');
    });

    /// Opening a folder is showing what is in it, so a double-click shuts it
    /// and another opens it again. The Compositions auto-folder is one.
    testWidgets('a double-click on a folder opens and shuts it',
        (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(rowText('Scene'), findsOneWidget,
          reason: 'a folder starts open, showing what it holds');

      await doubleClick(tester, rowText('Compositions'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(rowText('Scene'), findsNothing, reason: 'the folder shut');
      expect(find.byKey(const ValueKey('rename-field')), findsNothing,
          reason: 'and it is not a rename any more');

      await doubleClick(tester, rowText('Compositions'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(rowText('Scene'), findsOneWidget, reason: 'and opened again');
    });

    /// **A run of stills takes its speed from the item menu** (docs/07 §3.1).
    /// Stills carry no rate of their own, so the field beside Relink is the
    /// only place the speed of an imported run can be said: it opens on the 25
    /// the import gave it, writes the exact pair the engine stores, and undoes
    /// in one step. A file that is not a run is offered no field at all.
    testWidgets('an image sequence takes a new rate from its menu',
        (tester) async {
      final dir = Directory.systemTemp.createTempSync('lumit-sequence-rate');
      for (var n = 1; n <= 8; n++) {
        File('${dir.path}/frame${n.toString().padLeft(4, '0')}.png')
            .writeAsBytesSync(const [0]);
      }
      final p = freshProject();
      final run =
          p.state.project!.importFootage(path: '${dir.path}/frame0001.png');
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      Future<void> openMenu(String name) async {
        await tester.tapAt(
          tester.getCenter(rowText(name)),
          buttons: kSecondaryButton,
        );
        await tester.pumpAndSettle();
      }

      await openMenu('frame[0001-0008].png');
      const field = ValueKey('project-menu-sequence-rate-field');
      expect(find.byKey(const ValueKey('project-menu-sequence-rate')),
          findsOneWidget);
      expect(
        tester.widget<HouseTextField>(find.byKey(field)).controller.text,
        '25',
        reason: 'the rate the import gave it, in the hand the dialogs write in',
      );

      await tester.enterText(find.byKey(field), '23.976');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(
        (run.sequenceRate()!.fpsNum, run.sequenceRate()!.fpsDen),
        (24000, 1001),
        reason: 'a decimal in the field, the exact pair in the document',
      );

      p.state.project!.undo();
      expect((run.sequenceRate()!.fpsNum, run.sequenceRate()!.fpsDen), (25, 1),
          reason: 'one gesture, one op, one undo step');

      // One file is not a run, so there is no speed of its own to correct.
      await openMenu('shot.mov');
      expect(find.byKey(const ValueKey('project-menu-sequence-rate')),
          findsNothing);
      dir.deleteSync(recursive: true);
    });

    /// **New node graph on the panel's own menu** (§4.4). It goes through the
    /// same settings dialogue New composition does, and fronts what it made.
    testWidgets('New node graph from the row menu makes one and fronts it',
        (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tapAt(
        tester.getCenter(rowText('Scene')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(const ValueKey('project-menu-new-node-graph')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      final made = p.uiState.selectedComp;
      expect(made, isNotNull, reason: 'a graph you just made is fronted');
      expect(made!.getModel().isNodeGraph, isTrue);
      // Blank through the dialogue, so the engine names it and counts the
      // node graphs apart from the comps.
      expect(made.getSettings().name, 'Node graph 1');
      expect(rowText('Node graph 1'), findsOneWidget,
          reason: 'the panel re-read after its own edit');
    });

    /// **Add audio only:** the sound of a clip, as its own layer in the
    /// open composition. Offered only where there is a composition to put it
    /// in — a layer placed nowhere is not an action.
    testWidgets('Add audio only puts a clip\'s sound in the open comp',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      Future<void> openMenu() async {
        await tester.tapAt(
          tester.getCenter(rowText('shot.mov')),
          buttons: kSecondaryButton,
        );
        await tester.pumpAndSettle();
      }

      // No comp open: the entry is not there to be clicked.
      await openMenu();
      expect(find.byKey(const ValueKey('project-menu-add-audio-only')),
          findsNothing,
          reason: 'nowhere to put a layer, so nothing is offered');
      await tester.tapAt(const Offset(5, 5));
      await tester.pumpAndSettle();

      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      await tester.pumpAndSettle();

      await openMenu();
      await tester
          .tap(find.byKey(const ValueKey('project-menu-add-audio-only')));
      await tester.pumpAndSettle();

      final layers = comp.getLayers();
      expect(layers, hasLength(1));
      expect(layers.first.getKind(), BridgeLayerKind.audio,
          reason: 'the sound arrived as an Audio layer, not a footage layer');
      expect(layers.first.hasPicture(), isFalse);

      // One op: one undo takes it away again.
      p.state.project!.undo();
      expect(comp.getLayers(), isEmpty);
    });

    /// A *composition* double-clicks open instead — what it means in every
    /// editor — so its second click must front it in the Timeline and never
    /// drop into a rename. Renaming a comp lives in its context menu.
    testWidgets('double-clicking a composition opens it in the Timeline',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(p.uiState.selectedComp, isNull);

      await doubleClick(tester, rowText('Scene'));

      expect(p.uiState.selectedComp?.internalid, comp.internalid,
          reason: 'the second click fronted the comp');
      expect(find.byKey(const ValueKey('rename-field')), findsNothing,
          reason: 'opening a comp is not renaming it');

      // The rename it gave up is still reachable from the row menu.
      await tester.tapAt(tester.getCenter(rowText('Scene')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('project-menu-rename')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('rename-field')), findsOneWidget);

      await tester.pump(const Duration(milliseconds: 400));
    });

    /// Selecting several rows is what makes "drop four clips on the Timeline",
    /// or on New composition, a single gesture. Ctrl adds one at a time, Shift
    /// takes the run between, and a plain click goes back to just one.
    testWidgets('Ctrl and Shift select more than one row', (tester) async {
      final p = freshProject();
      for (final name in ['a.mov', 'b.mov', 'c.mov']) {
        p.state.project!.importFootage(path: 'C:/clips/$name');
      }
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      List<FootageReference> dragged() => tester
          .widget<Draggable<FootageDragData>>(
            find.ancestor(
              of: find.text('a.mov'),
              matching: find.byType(Draggable<FootageDragData>),
            ),
          )
          .data!
          .footage;

      await _clickRow(tester, 'a.mov');
      expect(dragged(), hasLength(1), reason: 'one click, one row');

      await _clickRow(tester, 'c.mov', held: LogicalKeyboardKey.controlLeft);
      expect(dragged(), hasLength(2),
          reason: 'Ctrl adds a row without dropping the first');

      await _clickRow(tester, 'a.mov');
      await _clickRow(tester, 'c.mov', held: LogicalKeyboardKey.shiftLeft);
      expect(dragged(), hasLength(3),
          reason: 'Shift takes the whole run between the two clicks');

      await _clickRow(tester, 'b.mov');
      expect(dragged(), hasLength(1),
          reason: 'a plain click goes back to just that row');
    });

    /// **Move to folder** files everything picked, in one undo step — the
    /// gesture for the rows that do not drag, and for a selection spanning
    /// kinds.
    testWidgets('the context menu files the whole selection into a folder',
        (tester) async {
      final p = freshProject();
      final folder = p.state.project!.newFolder(name: 'Footage');
      p.state.project!.importFootage(path: 'C:/clips/a.mov');
      p.state.project!.importFootage(path: 'C:/clips/b.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await _clickRow(tester, 'a.mov');
      await _clickRow(tester, 'b.mov', held: LogicalKeyboardKey.controlLeft);

      await tester.tapAt(
        tester.getCenter(rowText('b.mov')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      // The folders wait behind the entry, exactly as the effect categories do.
      await tester.tap(find.text('Move to folder'));
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(
            ValueKey<String>('project-menu-folder-${folder.internalid}')),
      );
      await tester.pumpAndSettle();

      expect(folder.getChildren(), hasLength(2),
          reason: 'both picked rows were filed, not just the one clicked');

      // One undo step for the pair: the group is what makes the gesture whole.
      p.state.project!.undo();
      expect(folder.getChildren(), isEmpty);
      expect(p.state.project!.getItems(), hasLength(3),
          reason: 'the folder and both clips, back at the root');
    });

    /// **Delete takes the whole selection**. It read the clicked row
    /// alone while Move to folder, two entries away in the same menu, already
    /// took `_targets` — the shape this ruling exists to stamp out.
    testWidgets('the context menu deletes the whole selection', (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/a.mov');
      p.state.project!.importFootage(path: 'C:/clips/b.mov');
      p.state.project!.importFootage(path: 'C:/clips/c.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await _clickRow(tester, 'a.mov');
      await _clickRow(tester, 'b.mov', held: LogicalKeyboardKey.controlLeft);

      await tester.tapAt(
        tester.getCenter(rowText('b.mov')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();

      expect(find.text('a.mov'), findsNothing);
      expect(find.text('b.mov'), findsNothing);
      expect(rowText('c.mov'), findsOneWidget,
          reason: 'the unpicked row stayed');
    });

    /// Missing-media rows and the filter. The imported path does not exist, so the
    /// engine's probe genuinely fails — no fake status is injected anywhere.
    ///
    /// `settleFrb` rather than a plain `pump`: the status probe is an async frb
    /// call, and only a real event-loop turn can deliver its answer. See
    /// `frb_test_support.dart` for the full account of that seam — and note that
    /// pumping *inside* `runAsync` is not the fix, because the panel's own
    /// `.then` continuation lives in the fake-async queue.
    testWidgets(
        'missing footage wears a badge, a Relink button, and can be '
        'filtered to', (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');
      final gone = p.state.project!.importFootage(path: 'C:/nowhere/gone.mp4');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(
        tester,
        until: () => find.text('missing').evaluate().isNotEmpty,
      );

      expect(find.text('missing'), findsOneWidget,
          reason: 'the engine probed the path, found nothing, and said so');
      expect(
        find.byKey(ValueKey<String>('relink-${gone.internalid}')),
        findsOneWidget,
      );

      // The header appears only while something is missing, and filters to it.
      expect(find.byKey(const ValueKey('missing-toggle')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('missing-toggle')));
      await tester.pumpAndSettle();

      expect(find.text('gone.mp4'), findsOneWidget);
      expect(find.text('Scene'), findsNothing,
          reason: 'filtered: every visible row is now something to fix');
    });

    testWidgets('relink routes the picked path to the engine', (tester) async {
      final p = freshProject();
      final gone = p.state.project!.importFootage(path: 'C:/nowhere/gone.mp4');

      // A file the engine's probe genuinely accepts, for the relink to land on.
      final target = _probeableMediaFile('relinked.wav');

      await tester.pumpWidget(hostPanel(
        child: ProjectPanelFrb(relinkPicker: () async => target),
        state: p.state,
        uiState: p.uiState,
      ));
      final relink = find.byKey(ValueKey<String>('relink-${gone.internalid}'));
      await settleFrb(
        tester,
        until: () => relink.evaluate().isNotEmpty,
      );
      expect(relink, findsOneWidget,
          reason: 'the missing badge is the inline relink control (the '
              'mockup gives a broken row a pill and no button)');

      // The tap itself is ordinary fake-async work, but it does not fire on the
      // pointer-up: the *row* under the button offers `onDoubleTap`, and a
      // `DoubleTapGestureRecognizer` holds the gesture arena for
      // `kDoubleTapTimeout` so a second tap can still arrive. Until that hold is
      // released the arena is never swept, so the button's own tap recognizer
      // never wins and `onPressed` never runs. Fake time has to be advanced past
      // it — `settleFrb` deliberately elapses none, so this pump is the one that
      // presses the button.
      await tester.tap(relink);
      await tester.pump(kDoubleTapTimeout + const Duration(milliseconds: 50));
      // `_doRelink` then awaits the injected picker (a fake-zone future, already
      // resolved by that pump) and calls the synchronous `relink`, which clears
      // the panel's status cache — so the row re-probes, and that needs real
      // event-loop turns again.
      await settleFrb(tester);

      expect(find.text('missing'), findsNothing,
          reason: 'the item resolves now, so the badge is gone');
      // …and the engine, not just the widget, agrees. Started inside `runAsync`,
      // so both the call and its continuation are real async — the one shape
      // that may be awaited there without deadlocking.
      final status = await tester.runAsync(() => gone.getStatus());
      expect(status, LumitMediaStatus.ready,
          reason: 'the picked path reached the engine, not just the panel');
    });

    /// The decoded picture lives in the info header now, not on the row: the
    /// tree stays a tight list of names, and selecting an item is what asks
    /// for its readout (docs/07 §3.1).
    testWidgets('selecting footage shows its thumbnail in the info header',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: _probeableImageFile('still.bmp'));

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(tester);

      expect(find.byType(RawImage), findsNothing,
          reason: 'rows carry glyphs; nothing is selected yet');

      await tester.tap(rowText('still.bmp'));
      // The single tap only wins the arena once the double-tap window closes.
      await tester.pump(const Duration(milliseconds: 350));
      await settleFrb(
        tester,
        until: () => find.byType(RawImage).evaluate().isNotEmpty,
      );

      expect(find.byKey(const ValueKey('project-info-header')), findsOneWidget);
      expect(find.byType(RawImage), findsOneWidget,
          reason: 'the header drew the decoded picture');
      // The card's second line names what the file is MADE OF now the codec
      // crosses. "footage" was what it could say before that, and is
      // still the fallback for an item with no container to name.
      expect(find.text('footage'), findsNothing);
      final codec =
          tester.widget<Text>(find.byKey(const ValueKey('project-info-codec')));
      expect(codec.data, isNotEmpty,
          reason: 'the header names the container the picture came out of');
    });

    /// The persistent search field (docs/07 §3.1): the tree narrows live to
    /// names that match, and a folder whose own name matches keeps its
    /// children visible as the path to them.
    testWidgets('the search field filters the tree live', (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.state.project!.importFootage(path: 'C:/clips/other.avi');
      p.state.project!.newComposition(name: 'Scene');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(tester);

      expect(find.text('shot.mov'), findsOneWidget);
      expect(find.text('other.avi'), findsOneWidget);

      await tester.enterText(
          find.byKey(const ValueKey('project-search')), 'shot');
      await tester.pump();

      expect(find.text('shot.mov'), findsOneWidget);
      expect(find.text('other.avi'), findsNothing,
          reason: 'the needle narrowed the tree');
      expect(find.text('Scene'), findsNothing);

      // A folder name matches: its children show as the path to them.
      await tester.enterText(
          find.byKey(const ValueKey('project-search')), 'compositions');
      await tester.pump();
      expect(find.text('Compositions'), findsOneWidget);
      expect(find.text('Scene'), findsOneWidget,
          reason: 'a matching folder keeps what it holds visible');

      await tester.enterText(find.byKey(const ValueKey('project-search')), '');
      await tester.pump();
      expect(find.text('other.avi'), findsOneWidget,
          reason: 'clearing the needle widens back to everything');
    });

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
    /// The reason these exist: the panel used to show "import footage or
    /// create a composition" and offer no way to do either, so an empty
    /// project was a dead end unless you found the menu bar.
    testWidgets('the footer imports footage into the project', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: ProjectPanelFrb(
          importPicker: () async => ['C:/clips/shot.mov'],
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(p.state.project!.getItems(), isEmpty);

      await tester.tap(find.byKey(const ValueKey('project-import')));
      await tester.pump();

      expect(p.state.project!.getItems(), hasLength(1),
          reason: 'the import reached the document');
      expect(find.textContaining('No items yet'), findsNothing,
          reason: 'and the panel is showing it');
    });

    testWidgets('the footer asks for settings, then makes a composition',
        (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('project-new-comp')));
      await tester.pump();
      // The button asks before it commits: nothing exists until Create.
      expect(find.text('NEW COMPOSITION'), findsWidgets);
      expect(p.state.project!.getItems(), isEmpty);

      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      expect(p.state.project!.getItems(), hasLength(1));
      expect(p.uiState.selectedComp, isNotNull,
          reason: 'a comp you just made is the one you want to work on');
    });

    /// A layered document is not footage: it arrives as a composition of its
    /// layers, and the engine is the one that says which files those are.
    testWidgets('a Photoshop document imports as a composition of its layers',
        (tester) async {
      final p = freshProject();
      final dir = Directory.systemTemp.createTempSync('lumit-psd');
      addTearDown(() => dir.deleteSync(recursive: true));
      final file = File('${dir.path}/poster.psd')
        ..writeAsBytesSync(_layeredPsd());

      expect(await p.state.importFootagePaths([file.path]), isTrue);

      // The roots: the Compositions folder and the folder of layer items.
      final roots = p.state.project!.getItems();
      expect(roots, hasLength(2));
      expect(roots, everyElement(isA<ItemReference_Folder>()),
          reason: 'the document itself is not filed as a footage item');
      expect(p.state.notice.value?.message, l10n.importLayersLeftOut(1),
          reason: 'the layer with no picture is counted, not dropped quietly');

      p.state.project!.undo();
      expect(p.state.project!.getItems(), isEmpty,
          reason: 'one import, one undo step');
    });

    /// Enter renames the lone selected item — the keyboard path that
    /// replaced the old second-click rename, live for every item kind.
    testWidgets('Enter renames the selected item', (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      p.uiState.activePane.value = Panel.project.pane();

      await tester.tap(rowText('shot.mov'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(find.byKey(const ValueKey('rename-field')), findsNothing);

      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('rename-field')), findsOneWidget,
          reason: 'Enter on the selection opens the inline rename');

      await tester.enterText(
          find.byKey(const ValueKey('rename-field')), 'Hero shot');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(rowText('Hero shot'), findsOneWidget,
          reason: 'the rename reached the document');

      // Escape throws the edit away: the editor closes and the item
      // keeps the name it had. Every other way out of an inline rename
      // commits, so without this there is no way to change your mind.
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('rename-field')), 'Typed then regretted');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('rename-field')), findsNothing,
          reason: 'Escape closes the editor');
      expect(rowText('Hero shot'), findsOneWidget,
          reason: 'and writes nothing: the old name stands');

      // While another panel is the active one, the key is not this panel's.
      p.uiState.activePane.value = Panel.timeline.pane();
      await tester.tap(rowText('Hero shot'));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      expect(find.byKey(const ValueKey('rename-field')), findsNothing,
          reason: 'a per-panel binding is live in the focused panel only');
    });

    // -----------------------------------------------------------------------
    // The five the mockup drew and the engine could not answer until now
    // (docs/07 §3.1, docs/15 §12A.3a).
    // -----------------------------------------------------------------------

    testWidgets('the bottom bar makes a folder, filed into the picked one',
        (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('project-new-folder')));
      await tester.pump();
      expect(rowText('Folder 1'), findsOneWidget,
          reason: 'a blank name takes the next unused "Folder N"');

      // Picking it and pressing again files the second one inside it, which
      // is what "the folder you are looking at" means.
      await tester.tap(rowText('Folder 1'));
      await tester.pump(kDoubleTapTimeout + const Duration(milliseconds: 50));
      await tester.tap(find.byKey(const ValueKey('project-new-folder')));
      await tester.pump();

      final children = p.state.project!.getItems();
      expect(children, hasLength(1),
          reason: 'the second folder is filed, not left at the root');
      expect(rowText('Folder 2'), findsOneWidget);
    });

    testWidgets('a colour tag tints the row glyph, filters the tree and undoes',
        (tester) async {
      final p = freshProject();
      final shot = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.state.project!.importFootage(path: 'C:/clips/other.mov');
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(tester, minRounds: 6);

      // Tagged through the row menu's chip strip — one click, no submenu.
      await tester.tap(rowText('shot.mov'), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('project-menu-label-4')));
      await tester.pumpAndSettle();

      final item = p.state.project!.getItems().firstWhere((i) =>
          i is ItemReference_Footage && i.field0.internalid == shot.internalid);
      expect(item.label(), 4, reason: 'the engine holds the tag');

      // The swatch filter narrows to that colour, and the picker's neutral
      // chip clears it.
      await pickFilterColour(tester, 4);
      expect(rowText('shot.mov'), findsOneWidget);
      expect(rowText('other.mov'), findsNothing,
          reason: 'an untagged item is not this colour');

      await pickFilterColour(tester, null);
      expect(rowText('other.mov'), findsOneWidget,
          reason: 'the neutral chip is the way back out');
    });

    /// **A sound file gets a play button where a picture would be** — the
    /// owner's answer to the footage-previews request (2026-09-07).
    testWidgets('a sound file is played from the preview card', (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: _probeableMediaFile('take.wav'));
      tester.view.physicalSize = const Size(480, 760);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(480, 760),
      ));
      await settleFrb(tester, minRounds: 8);

      final button = find.byKey(const ValueKey('project-preview-sound'));
      expect(button, findsNothing, reason: 'nothing is picked yet');

      await tester.tap(rowText('take.wav'));
      await tester.pump(kDoubleTapTimeout + const Duration(milliseconds: 50));
      await settleFrb(tester, minRounds: 8);

      expect(button, findsOneWidget,
          reason: 'a file with sound and no picture offers the play button');
      // And nothing to scrub: there are no frames behind it.
      expect(find.byKey(const ValueKey('project-preview-scrub')), findsNothing);

      // What the button says it will do, which is also what it says the
      // preview is doing.
      String says() => tester
          .widget<LumitTooltip>(
              find.ancestor(of: button, matching: find.byType(LumitTooltip)))
          .message;

      final idle = says();
      await tester.tap(button);
      await settleFrb(tester, minRounds: 4);
      expect(says(), isNot(idle),
          reason: 'the button turned round: it stops the preview now');

      // Pressing again silences it and the button goes back to offering a play.
      await tester.tap(button);
      await settleFrb(tester, minRounds: 4);
      expect(says(), idle);
    });

    /// **Proxies on the row menu.** Four commands and one badge, all
    /// over the seam: attach a file, read from it or not, forget it.
    testWidgets('the proxy commands round-trip from the row menu',
        (tester) async {
      final p = freshProject();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: ProjectPanelFrb(
          // The Set proxy… picker, stubbed: the same seam the relink uses.
          relinkPicker: () async => 'C:/clips/shot_proxy.mov',
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      Future<void> openMenu() async {
        await tester.tapAt(
          tester.getCenter(rowText('shot.mov')),
          buttons: kSecondaryButton,
        );
        await tester.pumpAndSettle();
      }

      final badge = find.byKey(ValueKey<String>('proxy-${footage.internalid}'));
      expect(badge, findsNothing, reason: 'nothing attached, nothing to say');

      // Nothing attached: the two commands that need a proxy are absent
      // rather than dead.
      await openMenu();
      expect(
          find.byKey(const ValueKey('project-menu-set-proxy')), findsOneWidget);
      expect(find.byKey(const ValueKey('project-menu-make-proxy')),
          findsOneWidget);
      expect(
          find.byKey(const ValueKey('project-menu-use-proxy')), findsNothing);
      expect(
          find.byKey(const ValueKey('project-menu-clear-proxy')), findsNothing);

      // Set proxy… attaches the picked file, switched on.
      await tester.tap(find.byKey(const ValueKey('project-menu-set-proxy')));
      await tester.pumpAndSettle();
      expect(footage.getProxy()?.path, contains('shot_proxy.mov'));
      expect(footage.getProxy()?.enabled, isTrue);
      expect(badge, findsOneWidget,
          reason: 'a row reading from its proxy says so');

      // Use proxy is the tick, and it writes both ways.
      await openMenu();
      await tester.tap(find.byKey(const ValueKey('project-menu-use-proxy')));
      await tester.pumpAndSettle();
      expect(footage.getProxy()?.enabled, isFalse);
      expect(badge, findsNothing,
          reason: 'attached but switched off has nothing to announce');

      await openMenu();
      await tester.tap(find.byKey(const ValueKey('project-menu-use-proxy')));
      await tester.pumpAndSettle();
      expect(footage.getProxy()?.enabled, isTrue);

      // Clear proxy detaches it, and the two commands go with it.
      await openMenu();
      await tester.tap(find.byKey(const ValueKey('project-menu-clear-proxy')));
      await tester.pumpAndSettle();
      expect(footage.getProxy(), isNull);
      expect(badge, findsNothing);

      await openMenu();
      expect(
          find.byKey(const ValueKey('project-menu-use-proxy')), findsNothing);
    });

    /// **The project-wide switch** lives on the bottom bar, after the
    /// new-item controls: it lights at `text_primary` while it is on, rests at
    /// `text_muted`, and writes the document both ways.
    testWidgets('the bottom bar carries the project-wide proxies switch',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final t = LumitTheme.dark();
      const key = ValueKey('project-use-proxies');
      // The mark, not the word: the word sheds on a narrow panel (§12A.6) and
      // the mark is what is always there to read the state off.
      ColorFilter? ink() => tester
          .widget<SvgPicture>(find.descendant(
              of: find.byKey(key), matching: find.byType(SvgPicture)))
          .colorFilter;

      expect(p.state.project!.useProxies(), isTrue,
          reason: 'a project reads from its proxies by default');
      expect(ink(), ColorFilter.mode(t.textPrimary, BlendMode.srcIn));

      await tester.tap(find.byKey(key));
      await tester.pumpAndSettle();
      expect(p.state.project!.useProxies(), isFalse,
          reason: 'the click reached the document');
      expect(ink(), ColorFilter.mode(t.textMuted, BlendMode.srcIn),
          reason: 'two strengths, never the accent');

      await tester.tap(find.byKey(key));
      await tester.pumpAndSettle();
      expect(p.state.project!.useProxies(), isTrue);
    });

    /// **Files dragged in from the OS file manager.** Flutter 3.47
    /// has no drop API of its own, so the event arrives on the `desktop_drop`
    /// plugin's method channel; these drive that channel directly, which is
    /// the whole mechanism minus the operating system.
    group('a drop from the file manager', () {
      /// One `desktop_drop` platform message, delivered the way the plugin's
      /// native side delivers it.
      Future<void> dropEvent(String method, Object? arguments) =>
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
              .handlePlatformMessage(
            'desktop_drop',
            const StandardMethodCodec()
                .encodeMethodCall(MethodCall(method, arguments)),
            (_) {},
          );

      /// A hover over the middle of a 480×760 panel, in the physical pixels
      /// the plugin reports. The host's device pixel ratio is 1, so these are
      /// the logical coordinates too.
      Future<void> hover() => dropEvent('entered', <double>[240, 380]);

      testWidgets('lights the panel while it hovers, and imports on release',
          (tester) async {
        final p = freshProject();
        await tester.pumpWidget(hostPanel(
          child: const ProjectPanelFrb(),
          state: p.state,
          uiState: p.uiState,
        ));
        await tester.pump();

        BoxDecoration? highlight() => tester
            .widget<Container>(find
                .descendant(
                  of: find.byType(DropTarget),
                  matching: find.byType(Container),
                )
                .first)
            .foregroundDecoration as BoxDecoration?;

        expect(highlight(), isNull, reason: 'nothing is being dragged');

        await hover();
        await tester.pump();
        expect(highlight()?.border,
            Border.all(color: LumitTheme.dark().accent, width: 1.5),
            reason: 'the drop-target treatment, as the folder rows wear it');

        await dropEvent('performOperation', <String>['C:/clips/shot.mov']);
        await tester.pump();

        expect(highlight(), isNull, reason: 'the drag is over');
        expect(find.text('shot.mov'), findsOneWidget);
      });
    });

    /// The import road itself, driven without the plugin: what each shape of
    /// dropped path turns into.
    group('what a dropped path becomes', () {
      testWidgets('a batch of files is one import and one undo step',
          (tester) async {
        final p = freshProject();
        await tester.pumpWidget(hostPanel(
          child: const SizedBox.shrink(),
          state: p.state,
          uiState: p.uiState,
        ));

        expect(
          await importDroppedPaths(
              p.state, ['C:/clips/a.mov', 'C:/clips/b.mov']),
          isTrue,
        );
        expect(p.state.project!.getItems().length, 2);

        p.state.project!.undo();
        expect(p.state.project!.getItems(), isEmpty,
            reason: 'one drop is one Ctrl-Z, however many files it carried');
      });
    });
  }, skip: !engineAvailable);
}

/// Click a row, optionally with a modifier held.
///
/// Two things this has to get right. The modifier is *held on the keyboard*
/// rather than carried on the tap, because `GestureDetector.onTap` does not
/// report one. And the pump is a full double-tap timeout: the rows also handle
/// double-taps, so a single tap is not delivered until the recogniser gives up
/// waiting for a second one — pumping a single frame leaves the click pending
/// and the test asserting against a selection that has not happened yet.
Future<void> _clickRow(
  WidgetTester tester,
  String name, {
  LogicalKeyboardKey? held,
}) async {
  if (held != null) await tester.sendKeyDownEvent(held);
  await tester.tap(find.text(name));
  await tester.pump(kDoubleTapTimeout);
  if (held != null) await tester.sendKeyUpEvent(held);
}

/// A temp file the engine's probe accepts, written **synchronously**.
///
/// Two traps are baked into this one small function.
///
/// *Synchronous `dart:io` is not a style choice.* An awaited async `dart:io` call
/// in a `testWidgets` body hangs the test outright. The I/O completes on the real
/// event loop, but its continuation was registered in the fake-async zone, and by
/// then `runTest` has done its one `flushMicrotasks` and is merely awaiting the
/// body — so nothing ever drains that queue. This is the same deadlock described
/// under `settleFrb`, and it is what made this test run for minutes instead of
/// failing: it never even reached the widget. `createTempSync`/`writeAsBytesSync`
/// sidestep it entirely.
///
/// *Existing is not the same as resolving.* `get_status` probes the file with
/// libavformat, so four arbitrary bytes do not resolve any more than a path that
/// is not there, so the relink would appear to do nothing. This writes a genuinely
/// valid 8-bit mono PCM WAV, which libavformat opens and reports one audio stream
/// for, so the item really does resolve afterwards. A WAV rather than a video
/// because it can be built here byte by byte; a real video would need an ffmpeg
/// CLI on the machine, which a widget test must not depend on.
String _probeableMediaFile(String name) {
  final dir = Directory.systemTemp.createTempSync('lumit-relink');
  final file = File('${dir.path}/$name');
  file.writeAsBytesSync(_silentWav());
  return file.path;
}

/// 0.1 s of 8-bit mono silence, as a WAV byte for byte.
Uint8List _silentWav() {
  const sampleRate = 8000;
  final samples = Uint8List(sampleRate ~/ 10)
    ..fillRange(0, sampleRate ~/ 10, 128);
  final out = BytesBuilder();
  void ascii(String s) => out.add(s.codeUnits);
  void u16(int v) => out.add([v & 0xff, (v >> 8) & 0xff]);
  void u32(int v) =>
      out.add([v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff]);

  ascii('RIFF');
  u32(36 + samples.length); // everything after this field
  ascii('WAVE');
  ascii('fmt ');
  u32(16); // fmt chunk size
  u16(1); // PCM, uncompressed
  u16(1); // mono
  u32(sampleRate);
  u32(sampleRate); // byte rate: 1 channel × 1 byte × rate
  u16(1); // block align
  u16(8); // bits per sample
  ascii('data');
  u32(samples.length);
  out.add(samples);
  return out.takeBytes();
}

/// A file with a genuinely decodable picture in it, for the thumbnail path.
///
/// A 2×2 24-bit BMP rather than a video: it can be built here byte by byte,
/// where a real video would need an ffmpeg CLI on the machine — which a widget
/// test must not depend on. libavformat opens it as a one-frame video stream,
/// which is all `thumbnail` asks for. The WAV that [_probeableMediaFile] writes
/// will not do: it resolves, but has no picture to decode.
String _probeableImageFile(String name) {
  final dir = Directory.systemTemp.createTempSync('lumit-thumb');
  final file = File('${dir.path}/$name');
  file.writeAsBytesSync(_tinyBmp());
  return file.path;
}

/// A 2×2 24-bit BMP, bottom-up, rows padded to a 4-byte boundary.
Uint8List _tinyBmp() {
  final out = BytesBuilder();
  void ascii(String s) => out.add(s.codeUnits);
  void u16(int v) => out.add([v & 0xff, (v >> 8) & 0xff]);
  void u32(int v) =>
      out.add([v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff]);

  // Two pixels per row is 6 bytes, padded to 8; two rows.
  const pixelBytes = 16;
  ascii('BM');
  u32(14 + 40 + pixelBytes); // file size
  u32(0); // reserved
  u32(14 + 40); // offset to the pixel array

  u32(40); // BITMAPINFOHEADER
  u32(2); // width
  u32(2); // height
  u16(1); // planes
  u16(24); // bits per pixel
  u32(0); // BI_RGB, uncompressed
  u32(pixelBytes);
  u32(2835); // ~72 dpi
  u32(2835);
  u32(0); // palette colours used
  u32(0); // all colours important

  // BGR triples: two rows of orange/blue, each padded to four bytes.
  for (var row = 0; row < 2; row++) {
    out.add([20, 120, 220, 220, 120, 20]);
    out.add([0, 0]); // row padding
  }
  return out.takeBytes();
}

/// A 4 by 4 Photoshop document, bottom layer first: two layers that hold a
/// picture and, between them, one that holds none, which is what an adjustment
/// layer looks like on disk.
Uint8List _layeredPsd() {
  const size = 4;
  void u16(BytesBuilder b, int v) => b.add([(v >> 8) & 0xff, v & 0xff]);
  void u32(BytesBuilder b, int v) {
    u16(b, (v >> 16) & 0xffff);
    u16(b, v & 0xffff);
  }

  final records = BytesBuilder();
  final data = BytesBuilder();
  void layer(String name, int side) {
    // Top, left, bottom, right.
    for (final edge in [0, 0, side, side]) {
      u32(records, edge);
    }
    // Transparency (-1), then red, green and blue, each stored raw.
    final channels = side == 0 ? const <int>[] : const [0xffff, 0, 1, 2];
    u16(records, channels.length);
    for (final id in channels) {
      u16(records, id);
      u32(records, 2 + side * side);
      u16(data, 0);
      data.add(List.filled(side * side, 255));
    }
    records.add('8BIMnorm'.codeUnits);
    records.add([255, 0, 0, 0]); // opacity, clipping, flags, filler
    // The name is a length byte and the text, padded to four bytes.
    final padded = (name.length + 4) & ~3;
    u32(records, 4 + 4 + padded);
    u32(records, 0); // no mask
    u32(records, 0); // no blending ranges
    records.add([name.length, ...name.codeUnits]);
    records.add(List.filled(padded - 1 - name.length, 0));
  }

  layer('Background', size);
  layer('Levels', 0);
  layer('Title', 2);
  final info = BytesBuilder();
  u16(info, 3);
  info.add(records.takeBytes());
  info.add(data.takeBytes());
  if (info.length.isOdd) info.addByte(0);
  final layers = info.takeBytes();

  final out = BytesBuilder();
  out.add('8BPS'.codeUnits);
  u16(out, 1);
  out.add(List.filled(6, 0));
  u16(out, 3); // channels
  u32(out, size);
  u32(out, size);
  u16(out, 8); // bits a channel
  u16(out, 3); // RGB
  u32(out, 0); // colour mode data
  u32(out, 0); // image resources
  u32(out, 4 + layers.length + 4);
  u32(out, layers.length);
  out.add(layers);
  u32(out, 0); // global mask
  // The flattened picture, raw, mid grey.
  u16(out, 0);
  out.add(List.filled(size * size * 3, 128));
  return out.takeBytes();
}
