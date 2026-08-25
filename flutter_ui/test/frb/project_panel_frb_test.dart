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

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart'
    show FootageReference, LumitMediaStatus;
import 'package:lumit_flutter/src/rust/api/project_item.dart'
    show ItemReference_Composition, ItemReference_Footage;
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeLayerKind;
import 'package:lumit_flutter/src/rust/api/state.dart' show ScopedChange;
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/drag_payloads.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:lumit_flutter/theme/theme.dart';

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

    /// A genuine double-click on a row — the gesture that **opens** it
    /// (K-534).
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

    testWidgets('an empty project shows the quiet hint', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(
        hostPanel(
          child: const ProjectPanelFrb(),
          state: p.state,
          uiState: p.uiState,
        ),
      );
      await tester.pump();

      expect(
        find.textContaining('No items yet'),
        findsOneWidget,
        reason: 'an empty document must say so rather than showing nothing',
      );
    });

    testWidgets('items appear as rows, each with a stable key', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.text('Scene'), findsOneWidget);
      expect(find.text('shot.mov'), findsOneWidget);
      // The auto-folder a new composition is filed into.
      expect(find.text('Compositions'), findsOneWidget);

      expect(
        find.byKey(ValueKey<String>('project-row-${comp.internalid}')),
        findsOneWidget,
      );
      expect(
        find.byKey(ValueKey<String>('project-row-${footage.internalid}')),
        findsOneWidget,
      );
    });

    /// v0 never tested nesting at all — its `walk` had no assertions. A new
    /// composition is filed into the Compositions auto-folder, so it must appear
    /// once, indented one level, not twice and not at the root.
    testWidgets('a folder nests its children, indented one level per depth',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.text('Scene'), findsOneWidget,
          reason: 'a filed comp is drawn once, under its folder — not twice');

      // The folder sits at depth 0, the comp inside it at depth 1.
      final folderRow = find.ancestor(
        of: find.text('Compositions'),
        matching: find.byType(Container),
      );
      final compRow =
          find.byKey(ValueKey<String>('project-row-${comp.internalid}'));
      final folderLeft = tester.getTopLeft(find.text('Compositions')).dx;
      final compLeft = tester.getTopLeft(find.text('Scene')).dx;
      expect(folderRow, findsWidgets);
      expect(compRow, findsOneWidget);
      expect(
        compLeft - folderLeft,
        closeTo(projectIndentPerDepth, 0.01),
        reason: 'one nesting level indents by the mockup\'s 16px',
      );
    });

    /// **Double-clicking** footage opens New composition on it (K-243):
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

    /// **Clicking a row that is already selected does nothing** (K-534, owner
    /// desk test: "if I click a selected item it brings up the new composition
    /// menu").
    ///
    /// Opening used to be decided on the raw pointer-up, which cannot tell the
    /// second click of a double-click from a click on a row selected a minute
    /// ago — so every ordinary click on a chosen clip raised New composition.
    /// It is the double-tap's business now.
    testWidgets('a click on an already-selected row opens nothing',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      var asked = 0;
      await tester.pumpWidget(hostPanel(
        child: ProjectPanelFrb(importPicker: () async {
          asked++;
          return [];
        }),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tap(rowText('shot.mov'));
      await tester.pump(const Duration(milliseconds: 500));
      expect(p.uiState.selectedProjectItem.value, isA<ItemReference_Footage>());

      // The click the owner made: the same row again, at any pace at all.
      await tester.tap(rowText('shot.mov'));
      await tester.pump(const Duration(milliseconds: 500));

      expect(find.text('NEW COMPOSITION'), findsNothing,
          reason: 'clicking a chosen row is not a command');
      expect(find.byKey(const ValueKey('comp-apply')), findsNothing);
      expect(find.byKey(const ValueKey('rename-field')), findsNothing);
      expect(asked, 0, reason: 'and it is not an import either');
      expect(p.uiState.selectedProjectItem.value, isA<ItemReference_Footage>(),
          reason: 'the row is simply still selected');
    });

    testWidgets('a click publishes the picked item for the FX console',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.state.project!.newComposition(name: 'Scene');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(p.uiState.selectedProjectItem.value, isNull,
          reason: 'nothing picked, nothing published');
      // The pumps ride out the rows' double-tap window, which arms a timer on
      // every tap.
      await tester.tap(rowText('shot.mov'));
      await tester.pump(const Duration(milliseconds: 500));
      expect(p.uiState.selectedProjectItem.value, isA<ItemReference_Footage>(),
          reason: 'the anchor item is mirrored to the shell (K-327)');
      await tester.tap(rowText('Scene'));
      await tester.pump(const Duration(milliseconds: 500));
      expect(
          p.uiState.selectedProjectItem.value, isA<ItemReference_Composition>(),
          reason: 'and follows the click');
    });

    /// Opening a folder is showing what is in it, so a double-click shuts it
    /// and another opens it again (K-243). The Compositions auto-folder is one.
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

    /// A search has to be able to find what is inside a shut folder, or
    /// searching would depend on where the twirls happen to be left.
    testWidgets('a search looks inside a shut folder', (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await doubleClick(tester, rowText('Compositions'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(rowText('Scene'), findsNothing);

      await tester.enterText(
          find.byKey(const ValueKey('project-search')), 'Scene');
      await tester.pumpAndSettle();
      expect(rowText('Scene'), findsOneWidget);
    });

    /// Renaming a folder moved to the row menu with the other two kinds'.
    testWidgets('a folder renames from its row menu', (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tapAt(
        tester.getCenter(rowText('Compositions')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('project-menu-rename')));
      await tester.pumpAndSettle();

      await tester.enterText(
          find.byKey(const ValueKey('rename-field')), 'Shots');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      expect(rowText('Shots'), findsOneWidget);
      expect(find.text('Compositions'), findsNothing,
          reason: 'the rename reached the document, not just the field');
    });

    /// **Add audio only (K-435):** the sound of a clip, as its own layer in the
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

    testWidgets('a blank rename is refused and the old name survives',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // From the row menu, which is where renaming footage lives (K-243).
      await tester.tapAt(
        tester.getCenter(rowText('shot.mov')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('project-menu-rename')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const ValueKey('rename-field')), '   ');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      expect(rowText('shot.mov'), findsOneWidget,
          reason: 'a row must never be able to lose its label');
    });

    /// Double-clicking a row is "select, then open" in one motion (owner
    /// request): the first click selects on its down stroke, the second opens
    /// immediately — no double-tap window to wait out. It must also never fall
    /// through to the empty-area import.
    testWidgets('double-clicking footage opens New composition, immediately',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      var asked = 0;
      await tester.pumpWidget(hostPanel(
        child: ProjectPanelFrb(importPicker: () async {
          asked++;
          return [];
        }),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // Two quick clicks, `kDoubleTapMinTime` apart and no more: any reliance
      // on the 300ms window *after* the second click would fail here.
      await doubleClick(tester, rowText('shot.mov'));

      await settleFrb(
        tester,
        until: () =>
            find.byKey(const ValueKey('comp-apply')).evaluate().isNotEmpty,
      );
      expect(find.text('NEW COMPOSITION'), findsWidgets,
          reason: 'the second click opens the dialogue with no arena delay');
      expect(asked, 0,
          reason: 'a double-click on a row is never an empty-area import');
      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();

      // Let the row's arena-absorbing double-tap recogniser time out before
      // the test tears down.
      await tester.pump(const Duration(milliseconds: 400));
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

    /// The same, starting from an already-selected row: the **double-click**
    /// still opens it. Being selected already is not what makes a click mean
    /// something (K-534) — the double-click is.
    testWidgets('double-clicking an already-selected row opens it',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // Select it first, as its own settled gesture.
      await tester.tap(rowText('shot.mov'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(find.byKey(const ValueKey('comp-apply')), findsNothing,
          reason: 'the first click only selects');

      await doubleClick(tester, rowText('shot.mov'));
      await settleFrb(
        tester,
        until: () =>
            find.byKey(const ValueKey('comp-apply')).evaluate().isNotEmpty,
      );
      expect(find.text('NEW COMPOSITION'), findsWidgets,
          reason: 'a double-click opens it, selected or not');
      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();
      await tester.pump(const Duration(milliseconds: 400));
    });

    testWidgets('footage rows are draggable, carrying FootageDragData',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // The Timeline's drop target consumes exactly this type and nothing else
      // produces it, so the payload type is load-bearing.
      expect(find.byType(Draggable<FootageDragData>), findsOneWidget);
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

    /// Dropping footage on New composition opens the same dialogue the button
    /// opens, and every dropped item lands in the finished comp as a layer
    /// (docs/07 §3.1).
    testWidgets('footage dropped on New composition makes a comp of it',
        (tester) async {
      final p = freshProject();
      final a = p.state.project!.importFootage(path: 'C:/clips/a.mov');
      final b = p.state.project!.importFootage(path: 'C:/clips/b.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // The drop is delivered straight to the target's callback rather than
      // simulated with a pointer: the Draggable and the target live in the same
      // panel, and a real drag would need the row and the footer on screen at
      // once at a size these tests do not fix.
      final target = find.byType(DragTarget<FootageDragData>);
      tester
          .widget<DragTarget<FootageDragData>>(target)
          .onAcceptWithDetails!(DragTargetDetails<FootageDragData>(
        data: FootageDragData([a, b], '2 items'),
        offset: tester.getCenter(target),
      ));
      // The dialogue opens only after every dropped item has been probed, which
      // is a real trip into FFmpeg — `settleFrb` waits on the engine rather than
      // on a frame count.
      await settleFrb(
        tester,
        until: () =>
            find.byKey(const ValueKey('comp-apply')).evaluate().isNotEmpty,
      );

      expect(find.text('NEW COMPOSITION'), findsWidgets,
          reason: 'a drop asks for the settings, exactly as the click does');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      final comp = p.uiState.selectedComp;
      expect(comp, isNotNull);
      expect(comp!.getLayers(), hasLength(2),
          reason: 'both dropped clips are in the comp');
    });

    testWidgets('the context menu deletes an item', (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tapAt(
        tester.getCenter(find.text('shot.mov')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();

      expect(find.text('Move to root'), findsOneWidget);
      expect(find.text('Find missing footage'), findsOneWidget);

      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();

      expect(find.text('shot.mov'), findsNothing);
      expect(p.state.project!.getItems(), isEmpty);
    });

    testWidgets('the context menu moves a filed item back to the root',
        (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // 'Scene' starts filed inside Compositions, so it is indented.
      final indentedBefore = tester.getTopLeft(rowText('Scene')).dx;

      await tester.tapAt(
        tester.getCenter(rowText('Scene')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Move to root'));
      await tester.pumpAndSettle();

      expect(
        tester.getTopLeft(rowText('Scene')).dx,
        lessThan(indentedBefore),
        reason: 'unfiled, so it is no longer indented under the folder',
      );
      expect(rowText('Scene'), findsOneWidget, reason: 'moved, not deleted');
    });

    /// The other direction, and the panel's own filing gesture (K-451): a row
    /// dropped on a folder row lands in that folder, and the folder's Items
    /// count says so at once.
    testWidgets('a folder row takes a dropped item and files it',
        (tester) async {
      final p = freshProject();
      final folder = p.state.project!.newFolder(name: 'Footage');
      final shot = p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      Finder countCell(String text) => find.descendant(
            of: find
                .byKey(ValueKey<String>('project-row-${folder.internalid}')),
            matching: find.text(text),
          );
      expect(countCell('0'), findsOneWidget, reason: 'an empty folder');

      // Delivered straight to the target's callback, as the New composition
      // drop test does: the row and the folder are both on screen, but a real
      // pointer drag would race the double-tap recogniser the rows also use.
      final target = find.descendant(
        of: find.byType(ListView),
        matching: find.byType(DragTarget<FootageDragData>),
      );
      tester
          .widget<DragTarget<FootageDragData>>(target)
          .onAcceptWithDetails!(DragTargetDetails<FootageDragData>(
        data: FootageDragData([shot], 'shot.mov'),
        offset: tester.getCenter(target),
      ));
      await tester.pumpAndSettle();

      expect(folder.getChildren(), hasLength(1),
          reason: 'the drop reached the document');
      expect(countCell('1'), findsOneWidget,
          reason: 'the Items count is re-read after the edit');
      expect(
        tester.getTopLeft(rowText('shot.mov')).dx,
        greaterThan(tester.getTopLeft(rowText('Footage')).dx),
        reason: 'filed, so it draws indented under the folder',
      );
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

    /// **Delete takes the whole selection** (K-523). It read the clicked row
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

    /// And the other half of the rule: a right-click on a row that is not part
    /// of the selection is about that row. (The row replaces the selection on
    /// the way into the menu, which is what makes this true rather than a
    /// second rule.)
    testWidgets('the context menu on an unpicked row deletes that row alone',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/a.mov');
      p.state.project!.importFootage(path: 'C:/clips/b.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await _clickRow(tester, 'a.mov');
      await tester.tapAt(
        tester.getCenter(rowText('b.mov')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();

      expect(rowText('a.mov'), findsOneWidget);
      expect(find.text('b.mov'), findsNothing);
    });

    /// A folder cannot be filed inside itself: the engine refuses it, so the
    /// menu never offers it — a dead entry is worse than a missing one.
    testWidgets('Move to folder never offers a folder its own subtree',
        (tester) async {
      final p = freshProject();
      final outer = p.state.project!.newFolder(name: 'Shoots');
      p.state.project!.newFolder(name: 'Day one', parent: outer.internalid);

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tapAt(
        tester.getCenter(rowText('Shoots')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      expect(find.text('Move to folder'), findsNothing,
          reason: 'itself and its own child are all there is to offer');
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

    /// The menu offers a different set per item kind, and offering the wrong one
    /// is how a user ends up with a Relink that cannot mean anything.
    ///
    /// Migrated from the v0 suite (project_placement_test.dart), which is the
    /// only place this was asserted before.
    testWidgets('the context menu shows the item set for the row it opened on',
        (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // A composition: settings, move, delete. Relink and Find missing are
      // footage-only (egui panels.rs).
      await tester.tapAt(tester.getCenter(rowText('Scene')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      expect(find.text('Composition settings…'), findsOneWidget);
      expect(find.text('Move to root'), findsOneWidget);
      expect(find.text('Delete'), findsOneWidget);
      expect(find.text('Relink…'), findsNothing);
      expect(find.text('Find missing footage'), findsNothing);
      await tester.tapAt(const Offset(400, 560));
      await tester.pumpAndSettle();

      // Present footage: no settings, and no Relink — that appears only on a
      // row that is actually broken.
      await tester.tapAt(tester.getCenter(rowText('shot.mov')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      expect(find.text('Composition settings…'), findsNothing);
      expect(find.text('Relink…'), findsNothing);
      expect(find.text('Find missing footage'), findsOneWidget);
      expect(find.text('Move to root'), findsOneWidget);
      expect(find.text('Delete'), findsOneWidget);
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
      // crosses (K-451). "footage" was what it could say before that, and is
      // still the fallback for an item with no container to name.
      expect(find.text('footage'), findsNothing);
      final codec =
          tester.widget<Text>(find.byKey(const ValueKey('project-info-codec')));
      expect(codec.data, isNotEmpty,
          reason: 'the header names the container the picture came out of');
    });

    /// The header's second line: the media's own vital statistics, from
    /// `mediaInfo`. A BMP has one frame and no rate worth speaking of, so the
    /// line asserts presence of the dimensions rather than exact wording.
    testWidgets('the info header reads out the media facts', (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: _probeableImageFile('still.bmp'));

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(tester);
      await tester.tap(rowText('still.bmp'));
      // The single tap only wins the arena once the double-tap window closes.
      await tester.pump(const Duration(milliseconds: 350));
      await settleFrb(
        tester,
        until: () => find
            .byKey(const ValueKey('project-info-line'))
            .evaluate()
            .isNotEmpty,
      );

      expect(find.byKey(const ValueKey('project-info-line')), findsOneWidget,
          reason: 'the probe answered and the line drew');
    });

    /// Selection must land the instant the button goes down — waiting out the
    /// double-click window read as the panel lagging behind the mouse. Fails
    /// without the row's pointer-down listener.
    testWidgets('selection lands on pointer down, before the tap resolves',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final gesture =
          await tester.startGesture(tester.getCenter(rowText('shot.mov')));
      await tester.pump();
      // The header names the item while the button is still held down.
      expect(find.byKey(const ValueKey('project-info-header')), findsOneWidget,
          reason: 'selection must not wait for the gesture arena');
      await gesture.up();
      await tester.pumpAndSettle();
    });

    /// The header is always there at one height, so selecting an item must
    /// never shove the tree downward.
    testWidgets('the info header keeps its height so rows never jump',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final before = tester.getTopLeft(rowText('shot.mov'));
      final gesture =
          await tester.startGesture(tester.getCenter(rowText('shot.mov')));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      expect(tester.getTopLeft(rowText('shot.mov')), before,
          reason: 'the header filling in must not move the rows');
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

    /// The panel used to rebuild on *every* document change, so tweaking a layer
    /// dropped the whole missing-media cache and re-probed every footage file on
    /// disk. `ScopedChange.items` is the separation; `op_scope` in api/state.rs
    /// classifies each op, and its unit tests cover the full table.
    testWidgets('a layer edit is not an item-list change; a rename is',
        (tester) async {
      final p = freshProject();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addFootageLayer(footage: footage, asSequence: false);

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      // Drain the setup's own changes first: the engine's stream only delivers
      // on real event-loop turns, so without this they arrive after we subscribe.
      await settleFrb(tester);

      final scopes = <ScopedChange>[];
      final sub = p.state.onChange.listen(scopes.add);
      addTearDown(sub.cancel);

      comp.getLayers().single.rename(name: 'Hero');
      await settleFrb(tester, until: () => scopes.isNotEmpty);

      expect(scopes.single.items, isFalse,
          reason: 'a layer rename must not make the panel re-probe every file');
      expect(scopes.single.layer, isNotNull,
          reason: 'it scopes to the layer that changed');

      // An item rename is the panel's business, and reaches it from outside.
      scopes.clear();
      final item =
          p.state.project!.getItems().whereType<ItemReference_Footage>().single;
      item.rename(name: 'hero.mov');
      await settleFrb(tester, until: () => scopes.isNotEmpty);

      expect(scopes.single.items, isTrue);
      expect(find.text('hero.mov'), findsOneWidget,
          reason: 'an edit made elsewhere still redraws the row');
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
      // The button asks before it commits (K-180): nothing exists until Create.
      expect(find.text('NEW COMPOSITION'), findsWidgets);
      expect(p.state.project!.getItems(), isEmpty);

      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      expect(p.state.project!.getItems(), hasLength(1));
      expect(p.uiState.selectedComp, isNotNull,
          reason: 'a comp you just made is the one you want to work on');
    });

    /// Cancelling has to leave the project exactly as it was — a dialogue that
    /// commits on the way out is worse than no dialogue.
    testWidgets('cancelling New composition makes nothing', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('project-new-comp')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();

      expect(p.state.project!.getItems(), isEmpty);
      expect(p.uiState.selectedComp, isNull);
    });

    /// Double-clicking empty space is the gesture people reach for before they
    /// find a menu, and it has to keep working once the panel has rows in it.
    testWidgets('double-clicking the empty area imports', (tester) async {
      final p = freshProject();
      var asked = 0;
      await tester.pumpWidget(hostPanel(
        child: ProjectPanelFrb(
          importPicker: () async {
            asked++;
            return ['C:/clips/shot.mov'];
          },
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final area = find.byKey(const ValueKey('project-empty-area'));
      await tester.tap(area);
      // A second tap is only a double tap if it lands inside the window, and
      // only a double *tap* if the first has passed kDoubleTapMinTime.
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tap(area);
      await tester.pumpAndSettle();

      expect(asked, 1, reason: 'the double-click opened the picker');
      expect(p.state.project!.getItems(), hasLength(1));

      // And again, now that the panel is drawing rows rather than the hint.
      await tester.tap(area, warnIfMissed: false);
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tap(area, warnIfMissed: false);
      await tester.pumpAndSettle();
      expect(asked, 2,
          reason: 'the blank space below the rows takes the gesture too');
    });

    /// Enter renames the lone selected item (K-321) — the keyboard path that
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
      p.uiState.activePanel.value = Panel.project;

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

      // Escape throws the edit away (K-323): the editor closes and the item
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
      p.uiState.activePanel.value = Panel.timeline;
      await tester.tap(rowText('Hero shot'));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      expect(find.byKey(const ValueKey('rename-field')), findsNothing,
          reason: 'a per-panel binding is live in the focused panel only');
    });

    // -----------------------------------------------------------------------
    // The five the mockup drew and the engine could not answer until now
    // (K-451, docs/07 §3.1, docs/15 §12A.3a).
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

    testWidgets('the Path column carries the folder, never the name again',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      tester.view.physicalSize = const Size(480, 760);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(480, 760),
      ));
      await settleFrb(tester, minRounds: 6);

      expect(find.text('PATH'), findsOneWidget,
          reason: 'the column has a kicker heading like the rest');
      // Importing records the file by its bare name, so there is no folder to
      // state yet — and the cell says nothing rather than repeating the Name
      // column, which is the whole reason it carries the folder and not the
      // path (the engine's own `file_path` is pinned in the Rust tests).
      expect(rowText('shot.mov'), findsOneWidget,
          reason: 'the name appears once in the row, not once per column');

      // Narrow enough and the column goes, with the preview card and Items.
      tester.view.physicalSize = const Size(260, 760);
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(260, 760),
      ));
      await settleFrb(tester, minRounds: 6);
      expect(find.text('PATH'), findsNothing,
          reason: 'the docked mockup at 260 draws no Path column');
    });

    testWidgets('a placed item wears the in use badge, and only a placed one',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final used = p.state.project!.importFootage(path: 'C:/clips/used.mov');
      p.state.project!.importFootage(path: 'C:/clips/spare.mov');
      comp.addFootageLayer(footage: used, asSequence: false);

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(tester, minRounds: 6);

      expect(find.byKey(ValueKey<String>('in-use-${used.internalid}')),
          findsOneWidget);
      expect(find.text('in use'), findsOneWidget,
          reason: 'the spare clip is not placed, so it says nothing');
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

      // The chip filter narrows to that colour, and the neutral chip clears it.
      await tester.tap(find.byKey(const ValueKey('project-label-chip-4')));
      await tester.pump();
      expect(rowText('shot.mov'), findsOneWidget);
      expect(rowText('other.mov'), findsNothing,
          reason: 'an untagged item is not this colour');

      await tester.tap(find.byKey(const ValueKey('project-label-chip-none')));
      await tester.pump();
      expect(rowText('other.mov'), findsOneWidget,
          reason: 'the neutral chip is the way back out');
    });

    testWidgets('the preview card states the codec and the sound it found',
        (tester) async {
      final p = freshProject();
      // A real file, so the probe has something to answer about.
      final path = _probeableImageFile('still.bmp');
      p.state.project!.importFootage(path: path);
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

      await tester.tap(rowText('still.bmp'));
      await tester.pump(kDoubleTapTimeout + const Duration(milliseconds: 50));
      await settleFrb(tester, minRounds: 8);

      // A BMP is a picture that does not run: no rate, no length, and the
      // second line names the codec rather than the kind of item.
      final line =
          tester.widget<Text>(find.byKey(const ValueKey('project-info-line')));
      expect(line.data, contains('still'),
          reason: 'a still says so where a rate and a length would be');
      expect(line.data, isNot(contains('fps')));

      final codec =
          tester.widget<Text>(find.byKey(const ValueKey('project-info-codec')));
      expect(codec.data, isNot('footage'),
          reason: 'the codec line replaced the kind-of-item fallback');
    });

    /// **Proxies on the row menu (K-501).** Four commands and one badge, all
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

    /// A comp row has no media reference, so it is offered none of the four.
    testWidgets('the proxy commands are footage-only', (tester) async {
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
      expect(
          find.byKey(const ValueKey('project-menu-set-proxy')), findsNothing);
      expect(
          find.byKey(const ValueKey('project-menu-make-proxy')), findsNothing);
    });

    /// **The project-wide switch (K-501)** lives on the bottom bar, after the
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
/// libavformat, so four arbitrary bytes read as missing just like a path that is
/// not there — the relink would appear to do nothing. This writes a genuinely
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
