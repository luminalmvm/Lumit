// The Timeline's chrome on frb: comp tabs, cache bar, search, the parent
// picker, markers, the work area and the razor.
//
// Driven through the panel rather than in isolation, for the same reason as
// everywhere else here: what matters is that a click reaches the document.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/icons.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Timeline chrome (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      // The outline alone is 800 px of columns; the default 800×600 test
      // surface would push its right edge (and the lanes) off screen.
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
    }

    /// Open the toolbar's ⋯ menu, where the layer/work-area/marker commands
    /// live now that the toolbar row belongs to the readouts and the search.
    Future<void> openMore(WidgetTester tester) async {
      await tester.tap(find.byKey(const ValueKey('tl-more')));
      await tester.pumpAndSettle();
    }

    /// The panel header strip (§12A.1): the panel's kicker, the comp tabs, and
    /// the single filled Export at the far right — which runs the File menu's
    /// own command rather than a second route to the same dialog.
    testWidgets('the header names the panel and carries Export',
        (tester) async {
      final p = withComp();
      await mount(tester, p);

      final header = tester.getRect(find.byType(CompTabsFrb));
      final kicker = tester.getRect(find.text('TIMELINE'));
      final export = tester.getRect(find.byKey(const ValueKey('tl-export')));
      final tab = tester
          .getRect(find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}')));

      expect(kicker.right, lessThanOrEqualTo(tab.left + 0.5),
          reason: 'the kicker comes before the tabs');
      expect(export.left, greaterThan(tab.right),
          reason: 'and Export is at the far right of the strip');
      expect(export.right, lessThanOrEqualTo(header.right + 0.5));

      await tester.tap(find.byKey(const ValueKey('tl-export')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('export-close')), findsOneWidget,
          reason: 'it opens the export dialogue the File menu opens');
      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// **A hovered comp tab wears the value well's own hover outline**
    /// (K-640). A tab that is not the open one used to answer a hover with
    /// nothing at all: the pointer crossed it and the strip did not admit it
    /// was a control. It now takes the same face a well takes — one pixel of
    /// `hairlineStrong` — in a *foreground* decoration, so nothing about the
    /// strip's layout moves as the pointer travels along it.
    testWidgets('a hovered comp tab outlines, and does not move',
        (tester) async {
      final p = withComp();
      final second = p.state.project!.newComposition(name: 'Other');
      p.uiState.setSelectedComp(second);
      await mount(tester, p);

      final key = ValueKey<String>('tl-tab-${p.comp.internalid}');
      Container box() => tester.widget<Container>(find
          .descendant(of: find.byKey(key), matching: find.byType(Container))
          .first);

      expect(box().foregroundDecoration, isNull,
          reason: 'an untouched tab wears no outline');
      final before = tester.getSize(find.byKey(key));

      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);
      await gesture.moveTo(tester.getCenter(find.byKey(key)));
      await tester.pumpAndSettle();

      final t = LumitTheme.dark();
      final outline =
          (box().foregroundDecoration! as BoxDecoration).border! as Border;
      expect(outline.top.color, t.hairlineStrong,
          reason: "the well's own hover edge, and no other grey");
      expect(outline.top.width, 1);
      expect(tester.getSize(find.byKey(key)), before,
          reason: 'and the outline costs the strip no layout');

      // The open tab already says so with its seated surface, so it stays as
      // it is under the pointer.
      final open = ValueKey<String>('tl-tab-${second.internalid}');
      await gesture.moveTo(tester.getCenter(find.byKey(open)));
      await tester.pumpAndSettle();
      expect(
          tester
              .widget<Container>(find
                  .descendant(
                      of: find.byKey(open), matching: find.byType(Container))
                  .first)
              .foregroundDecoration,
          isNull,
          reason: 'the open tab is already marked; hover adds nothing');
    });

    /// The open composition is marked by the seated surface alone (§12A.1) —
    /// **no accent tick**. The accent's "active tab" job is the workspace
    /// tabs', not these.
    testWidgets('the open comp tab carries no accent tick', (tester) async {
      final p = withComp();
      await mount(tester, p);

      final t = LumitTheme.dark();
      final tab = tester.widget<Container>(find
          .descendant(
            of: find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}')),
            matching: find.byType(Container),
          )
          .first);
      final decoration = tab.decoration! as BoxDecoration;
      expect(decoration.color, t.surface1,
          reason: 'the fronted tab is seated in the panel\'s own surface');
      final border = decoration.border! as Border;
      expect(border.bottom.width, 0,
          reason: 'and wears nothing along its bottom edge');
      expect([border.left.color, border.right.color, border.bottom.color],
          isNot(contains(t.accent)),
          reason: 'nothing on this tab is drawn in the accent');
      // And no seams either: the manifest computes no border on any comp tab.
      // The sides are still reserved so a tab keeps its width in both shapes,
      // but they are drawn in nothing.
      expect([border.left.color.a, border.right.color.a], [0.0, 0.0],
          reason: 'the seams are reserved, not drawn');
    });

    testWidgets('the comp tabs show the open comps and front one',
        (tester) async {
      final p = withComp();
      final second = p.state.project!.newComposition(name: 'Titles');
      await mount(tester, p);

      expect(find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('tl-tab-${second.internalid}')),
          findsNothing,
          reason: 'a comp nobody has fronted is not an open tab');

      p.uiState.setSelectedComp(second);
      await tester.pump();
      final tab = find.byKey(ValueKey<String>('tl-tab-${second.internalid}'));
      expect(tab, findsOneWidget, reason: 'fronting a comp opens its tab');

      await tester
          .tap(find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}')));
      await tester.pump();
      expect(p.uiState.selectedComp?.internalid, p.comp.internalid);
      expect(tab, findsOneWidget, reason: 'switching away keeps the tab open');
    });

    /// The × closes only the tab: the comp stays in the project, and closing
    /// the fronted tab fronts its nearest remaining neighbour.
    testWidgets('closing a comp tab keeps the comp and fronts a neighbour',
        (tester) async {
      final p = withComp();
      final second = p.state.project!.newComposition(name: 'Titles');
      p.uiState.setSelectedComp(second);
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-tab-close-${second.internalid}')));
      await tester.pump();

      expect(find.byKey(ValueKey<String>('tl-tab-${second.internalid}')),
          findsNothing);
      expect(p.uiState.selectedComp?.internalid, p.comp.internalid,
          reason: 'the neighbour fronted');
      expect(p.state.comps().map((c) => c.$2), contains('Titles'),
          reason: 'closing a tab never deletes the comp');

      // Closing the last tab leaves no comp fronted, and the panel says so.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-tab-close-${p.comp.internalid}')));
      await tester.pump();
      expect(p.uiState.selectedComp, isNull);
      expect(find.textContaining('Open a composition'), findsOneWidget);
    });

    /// The strip is the user's order, not the project's: a tab dragged onto
    /// another takes its place, and the fronted comp comes along unchanged.
    testWidgets('dragging a comp tab reorders the strip', (tester) async {
      final p = withComp();
      final second = p.state.project!.newComposition(name: 'Titles');
      p.uiState.setSelectedComp(second);
      await mount(tester, p);

      final first = find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}'));
      expect(
          tester.getCenter(first).dx,
          lessThan(tester
              .getCenter(
                  find.byKey(ValueKey<String>('tl-tab-${second.internalid}')))
              .dx));

      // Onto the tab to its left, which is where it lands.
      await tester.drag(
          find.byKey(ValueKey<String>('tl-tab-${second.internalid}')),
          tester.getCenter(first) -
              tester.getCenter(
                  find.byKey(ValueKey<String>('tl-tab-${second.internalid}'))));
      await tester.pumpAndSettle();

      expect(p.uiState.openComps, [second.internalid, p.comp.internalid]);
      expect(p.uiState.selectedComp?.internalid, second.internalid,
          reason: 'reordering the strip fronts nothing new');
      expect(
          tester
              .getCenter(
                  find.byKey(ValueKey<String>('tl-tab-${second.internalid}')))
              .dx,
          lessThan(tester.getCenter(first).dx),
          reason: 'and the strip is drawn in the new order');
    });

    /// Right-clicking a tab reaches the comp's settings, so the comp being
    /// worked in can be edited without going back to the Project panel.
    testWidgets('a comp tab offers Composition settings on right click',
        (tester) async {
      final p = withComp();
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      expect(
          find.byKey(const ValueKey('tl-tab-menu-settings')), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('tl-tab-menu-settings')));
      await tester.pumpAndSettle();
      // The dialog frame sets its title as a capitals kicker (§12A.4), so
      // the phrase on screen is the upper-cased form; the menu entry that got
      // us here wears the trailing ellipsis and neither matches by accident.
      expect(find.text('COMPOSITION SETTINGS'), findsOneWidget,
          reason: 'the dialog the Project panel opens, from the tab');
      expect(tester.takeException(), isNull);
    });

    /// **The bridge error where the Timeline should be.** Pre-compose, step
    /// into the new comp, undo: the layers come back and the comp they were
    /// packed into stops existing — with the Timeline still fronting it, every
    /// panel read a comp the engine had never heard of. What has gone cannot
    /// stay fronted, so the user goes back where they came from.
    testWidgets('undoing away the fronted comp goes back to the previous one',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      final packed = p.comp.precompose(
        layerIds: [p.comp.getLayers().single.internallayerId],
        name: 'Packed',
        leaveAttributes: false,
        adjustDuration: false,
      );
      final inner = switch (packed.getSourceItem()!) {
        ItemReference_Composition(:final field0) => field0,
        _ => throw StateError('a Precomp layer draws from a composition'),
      };
      p.uiState.setSelectedComp(inner);
      await mount(tester, p);
      expect(p.uiState.selectedComp?.internalid, inner.internalid);

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();

      expect(p.uiState.selectedComp?.internalid, p.comp.internalid,
          reason: 'the comp the user came from fronts again');
      expect(find.byKey(ValueKey<String>('tl-tab-${inner.internalid}')),
          findsNothing,
          reason: 'and the tab it had goes with it');
      expect(tester.takeException(), isNull);
    });

    /// With nowhere to go back to — the comp the user came from has gone too
    /// — the nearest open tab takes over, looking left before right.
    testWidgets('a vanished comp falls back to the nearest open tab',
        (tester) async {
      final p = withComp();
      final second = p.state.project!.newComposition(name: 'Titles');
      p.uiState.setSelectedComp(second);
      final third = p.state.project!.newComposition(name: 'Doomed');
      p.uiState.setSelectedComp(third);
      await mount(tester, p);

      // Both the fronted comp and the one the user came from go.
      for (final comp in [second, third]) {
        ItemReference.composition(comp).delete();
      }
      p.uiState.model.refresh();
      await tester.pump();

      expect(p.uiState.selectedComp?.internalid, p.comp.internalid,
          reason: 'the nearest tab still standing, to the left');
      expect(tester.takeException(), isNull);
    });

    testWidgets('search narrows the outline to matching rows', (tester) async {
      final p = withComp();
      p.comp.addTextLayer();
      p.comp.addCameraLayer();
      await mount(tester, p);

      // Once each: the outline names the layer, and its bar carries no label
      // unless the setting asks for one (K-514).
      expect(find.text('Text'), findsOneWidget);
      expect(find.text('Camera'), findsOneWidget);

      await tester.enterText(find.byKey(const ValueKey('tl-search')), 'cam');
      await tester.pump();

      expect(find.text('Camera'), findsOneWidget);
      expect(find.text('Text'), findsNothing,
          reason: 'search hides the rows that do not match');
    });

    testWidgets('the parent picker parents a layer and refuses a cycle',
        (tester) async {
      final p = withComp();
      final parent = p.comp.addAdjustmentLayer();
      final child = p.comp.addCameraLayer();
      await mount(tester, p);

      expect(child.getParent(), isNull);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-parent-${child.internallayerId}')));
      await tester.pumpAndSettle();
      // Numbered by place in the composition since item 6.13, so the entry
      // reads "1. Adjustment" rather than the bare name.
      await tester.tap(find.textContaining(parent.getName()).last);
      await tester.pumpAndSettle();

      expect(child.getParent(), parent.internallayerId);

      // Clearing it is a first-class choice, not an error state.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-parent-${child.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('None').last);
      await tester.pumpAndSettle();
      expect(child.getParent(), isNull);
    });

    testWidgets('Set in and Set out move the work area, Clear removes it',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      expect(p.comp.getWorkArea(), isNull);

      p.uiState.playheadFrame.value = 20;
      await tester.pump();
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-work-in')));
      await tester.pumpAndSettle();

      var area = p.comp.getWorkArea();
      expect(area, isNotNull);
      expect(p.comp.frameAtTime(time: area!.inPoint), 20);

      p.uiState.playheadFrame.value = 60;
      await tester.pump();
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-work-out')));
      await tester.pumpAndSettle();

      area = p.comp.getWorkArea();
      expect(p.comp.frameAtTime(time: area!.outPoint), 60);
      expect(p.comp.frameAtTime(time: area.inPoint), 20,
          reason: 'setting the out point leaves the in point alone');

      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-clear-work-area')));
      await tester.pumpAndSettle();
      expect(p.comp.getWorkArea(), isNull);
    });

    /// A work area with no length is not a work area, so the opposite edge gives
    /// way rather than the click being ignored.
    testWidgets('setting the out point before the in point still leaves length',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      p.uiState.playheadFrame.value = 40;
      await tester.pump();
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-work-in')));
      await tester.pumpAndSettle();

      p.uiState.playheadFrame.value = 10;
      await tester.pump();
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-work-out')));
      await tester.pumpAndSettle();

      final area = p.comp.getWorkArea()!;
      final start = p.comp.frameAtTime(time: area.inPoint);
      final end = p.comp.frameAtTime(time: area.outPoint);
      expect(end, greaterThan(start), reason: 'it always has length');
    });

    /// **Dragging an edge cannot leave the comp.** A pointer past either end
    /// gave a frame outside it, and a negative in point took the render worker
    /// down: cast unsigned for the cache fill it became a first frame of
    /// eighteen quintillion, `clamp` panicked on the crossed bounds, and every
    /// later frame request came back a send error. The helper the drag commits
    /// through clamps, so the handle stops at the edge.
    testWidgets('a work-area edge dragged past the comp stops at its end',
        (tester) async {
      final p = withComp();
      await mount(tester, p);
      final frames = p.comp.durationFrames();

      // Well past the end, then well before the start.
      p.comp.setWorkArea(
        span: workAreaWith(
          comp: p.comp,
          current: null,
          wanted: frames + 500,
          isStart: false,
        ),
      );
      expect(p.comp.frameAtTime(time: p.comp.getWorkArea()!.outPoint), frames,
          reason: 'the out point stops at the end of the comp');

      p.comp.setWorkArea(
        span: workAreaWith(
          comp: p.comp,
          current: p.comp.getWorkArea(),
          wanted: -500,
          isStart: true,
        ),
      );
      final area = p.comp.getWorkArea()!;
      expect(p.comp.frameAtTime(time: area.inPoint), 0,
          reason: 'and the in point at frame zero');
      expect(p.comp.frameAtTime(time: area.outPoint),
          greaterThan(p.comp.frameAtTime(time: area.inPoint)));
    });

    testWidgets('the marker editor adds at the playhead and removes',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      p.uiState.playheadFrame.value = 33;
      await tester.pump();
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-markers')));
      await tester.pumpAndSettle();

      expect(find.text('No markers yet'), findsOneWidget);
      await tester.enterText(
          find.byKey(const ValueKey('marker-label')), 'Chorus');
      await tester.tap(find.byKey(const ValueKey('marker-add')));
      await tester.pumpAndSettle();

      final markers = p.comp.getMarkers();
      expect(markers, hasLength(1));
      expect(markers.single.label, 'Chorus');
      expect(p.comp.frameAtTime(time: markers.single.time), 33,
          reason: 'the marker landed on the playhead');

      await tester.tap(
          find.byKey(ValueKey<String>('marker-remove-${markers.single.id}')));
      await tester.pumpAndSettle();
      expect(p.comp.getMarkers(), isEmpty);

      await tester.tap(find.byKey(const ValueKey('marker-close')));
      await tester.pumpAndSettle();
    });

    /// Scrubbing during playback used to be unwinnable: the engine handed back
    /// a frame every tick and each one put the playhead straight back where the
    /// transport wanted it. Taking hold of the playhead takes it off the
    /// transport (K-254), and it stays where the drag left it — the
    /// return-to-start of a normal stop would undo the very gesture.
    testWidgets('dragging the ruler during playback stops it and holds',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      p.uiState.play();
      await tester.pump();
      expect(p.uiState.playing.value, isTrue);

      await tester.drag(
          find.byKey(const ValueKey('tl-ruler')), const Offset(120, 0));
      await tester.pumpAndSettle();

      expect(p.uiState.playing.value, isFalse,
          reason: 'taking hold of the playhead stops the transport');
      expect(p.uiState.playheadFrame.value, greaterThan(0),
          reason: 'and it stays where the drag left it, not back at the start');
    });

    /// Markers on the ruler are direct manipulation now (K-254): a flag can be
    /// dragged to another moment, and its text changed from its own menu. The
    /// dialogue in the ⋯ menu is still there for adding one by hand.
    testWidgets('a marker flag drags along the ruler', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 10, label: 'Chorus');
      await mount(tester, p);

      final id = p.comp.getMarkers().single.id;
      final flag = find.byKey(ValueKey<String>('tl-marker-$id'));
      expect(flag, findsOneWidget, reason: 'the marker draws on the ruler');

      await tester.drag(flag, const Offset(80, 0));
      await tester.pumpAndSettle();

      final moved = p.comp.getMarkers().single;
      expect(p.comp.frameAtTime(time: moved.time), greaterThan(10),
          reason: 'the drag moved the marker later in the comp');
      expect(moved.label, 'Chorus', reason: 'and left what it says alone');
      expect(moved.id, id, reason: 'it is the same marker, not a new one');
    });

    /// A marker can carry a span (K-441, docs/15 §12A.1): the ruler draws a bar
    /// running from its frame for its duration, and a moment draws none.
    testWidgets('a spanning marker draws a bar, a moment draws none',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 10, label: 'Chorus');

      final moment = p.comp.getMarkers().single;
      expect(moment.durationFrames, isNull,
          reason: 'a marker made on the ruler is a moment');
      await mount(tester, p);
      expect(find.byKey(ValueKey<String>('tl-marker-span-${moment.id}')),
          findsNothing);

      // Give it a span. The panel has no control for one yet — the seam
      // carries it so the ruler can DRAW it — so this writes what a marker
      // imported or detected with a duration would arrive as.
      writeMarkers(p.comp, [
        BridgeMarker(
          id: moment.id,
          time: moment.time,
          label: moment.label,
          durationFrames: 20,
        ),
      ]);
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      final bar = find.byKey(ValueKey<String>('tl-marker-span-${moment.id}'));
      expect(bar, findsOneWidget, reason: 'the span draws as a bar');
      final flag = tester
          .getRect(find.byKey(ValueKey<String>('tl-marker-${moment.id}')));
      final span = tester.getRect(bar);
      expect(span.left, closeTo(flag.left + MarkerFlag.width / 2, 0.5),
          reason: 'the bar starts under the point, which is what says where');
      expect(span.width, greaterThan(0));
      expect(span.height, MarkerFlag.spanHeight);

      // And it survives an edit that never touched it — the same promise
      // K-270 made for a beat's kind.
      await tester.tap(find.byKey(ValueKey<String>('tl-marker-${moment.id}')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('marker-menu-edit')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('marker-edit-label')), 'Drop');
      await tester.tap(find.byKey(const ValueKey('marker-edit-ok')));
      await tester.pumpAndSettle();

      final renamed = p.comp.getMarkers().single;
      expect(renamed.label, 'Drop');
      expect(renamed.durationFrames, 20,
          reason: 'renaming a spanning marker did not flatten it to a moment');
    });

    testWidgets('right-clicking a marker edits what it says', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 10, label: 'Chorus');
      await mount(tester, p);

      final id = p.comp.getMarkers().single.id;
      await tester.tap(find.byKey(ValueKey<String>('tl-marker-$id')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('marker-menu-edit')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('marker-edit-label')), 'Drop');
      await tester.tap(find.byKey(const ValueKey('marker-edit-ok')));
      await tester.pumpAndSettle();

      final edited = p.comp.getMarkers().single;
      expect(edited.label, 'Drop');
      expect(p.comp.frameAtTime(time: edited.time), 10,
          reason: 'renaming did not move it');
    });

    testWidgets('a marker can be deleted from its own menu', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 10, label: 'Chorus');
      await mount(tester, p);

      await tester.tap(
          find.byKey(
              ValueKey<String>('tl-marker-${p.comp.getMarkers().single.id}')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('marker-menu-delete')));
      await tester.pumpAndSettle();

      expect(p.comp.getMarkers(), isEmpty);
    });

    /// Markers do not stack. Two flags on one frame are two things to click and
    /// one place, and the second hides the first exactly — so the newcomer wins,
    /// whether it arrives by shortcut or by being dragged on top.
    test('a marker added to an occupied frame replaces what is there', () {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      addMarkerFrb(comp, frame: 20, label: 'Chorus');
      addMarkerFrb(comp, frame: 20, label: 'Drop');
      expect(comp.getMarkers(), hasLength(1));
      expect(comp.getMarkers().single.label, 'Drop');
    });

    /// The drop half of the same rule: a flag dragged onto another takes its
    /// place, and keeps its own identity rather than being deleted and remade.
    test('a marker dragged onto another replaces it', () {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      addMarkerFrb(comp, frame: 10, label: 'Chorus');
      addMarkerFrb(comp, frame: 60, label: 'Drop');
      final drop = markersOf(comp).firstWhere((m) => m.label == 'Drop');

      writeMarkers(comp,
          markersWithFrb(comp, frame: 10, label: drop.label, id: drop.id));

      final left = comp.getMarkers();
      expect(left, hasLength(1), reason: 'the two did not stack');
      expect(left.single.label, 'Drop', reason: 'the dragged one won');
      expect(left.single.id, drop.id, reason: 'and it is the same marker');
      expect(comp.frameAtTime(time: left.single.time), 10);
    });

    /// The flag's **point** is what says which frame, so it sits on the
    /// playhead rather than beside it — the whole reason the flag is centred on
    /// its frame instead of hung off to the right of it.
    testWidgets('a marker flag points at the frame it marks', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 40, label: 'Chorus');
      p.uiState.playheadFrame.value = 40;
      await mount(tester, p);

      final flag = tester.getTopLeft(find
          .byKey(ValueKey<String>('tl-marker-${markersOf(p.comp).single.id}')));
      final playhead = tester.getCenter(find.byType(PlayheadMarker).first);
      expect(flag.dx + MarkerFlag.width / 2, closeTo(playhead.dx, 1.5),
          reason: 'the point and the playhead are on the same frame');
    });

    /// And the shape is the redesign's (docs/15 §12A.1): an upward triangle
    /// standing on the cache bar, half of it outside the backdrop pill that
    /// carries the label — the pill starting at the point, so what is written
    /// reads as hanging off *this* moment.
    testWidgets('the marker pill starts at the point of the triangle',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 40, label: 'Chorus');
      await mount(tester, p);

      final flagFinder = find
          .byKey(ValueKey<String>('tl-marker-${markersOf(p.comp).single.id}'));
      final flag = tester.getRect(flagFinder);
      final pill = tester.getRect(
          find.descendant(of: flagFinder, matching: find.byType(Container)));

      expect(pill.left, closeTo(flag.left + MarkerFlag.width / 2, 0.5),
          reason: 'the pill begins where the point is');
      expect(pill.right, greaterThan(flag.left + MarkerFlag.width),
          reason: 'and runs away to the right, clear of the triangle');
      expect(pill.height, MarkerFlag.height);
      expect(pill.bottom, closeTo(flag.bottom, 0.5),
          reason: 'both stand on the same floor');
    });

    /// A numbered marker names one place, so setting it again moves it rather
    /// than leaving two for the bare digit to choose between. Unlabelled cues
    /// are dropped as freely as you like.
    test('a numbered marker is replaced, an unlabelled one is not', () {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      addMarkerFrb(comp, frame: 10, label: '1');
      addMarkerFrb(comp, frame: 40, label: '1');
      expect(comp.getMarkers(), hasLength(1));
      expect(markerFrameFrb(comp, '1'), 40);

      addMarkerFrb(comp, frame: 5);
      addMarkerFrb(comp, frame: 7);
      expect(comp.getMarkers(), hasLength(3));
      expect(markerFrameFrb(comp, '2'), isNull,
          reason: 'a digit with no marker is nothing to jump to');
    });

    // --- layer markers (K-254) -------------------------------------------

    /// A composition dropped into another brings its markers along as the
    /// layer's own. Copies, with ids of their own: from here the two lists are
    /// unrelated, so editing the layer's never reaches into the comp it came
    /// from — or into anywhere else that comp is used.
    test('a comp dropped in brings its markers along as the layer\'s', () {
      final p = freshProject();
      final source = p.state.project!.newComposition(name: 'Beats');
      addMarkerFrb(source, frame: 12, label: 'Drop');
      final into = p.state.project!.newComposition(name: 'Scene');

      final layer = into.addPrecompLayer(comp: source);
      final onLayer = layer.getMarkers();
      expect(onLayer, hasLength(1));
      expect(onLayer.single.label, 'Drop');
      expect(onLayer.single.id, isNot(source.getMarkers().single.id),
          reason: 'a copy, not the same marker');

      // And the copy is genuinely independent.
      layer.setMarkers(markers: const []);
      expect(layer.getMarkers(), isEmpty);
      expect(source.getMarkers(), hasLength(1),
          reason: 'clearing the layer left the composition alone');
    });

    /// Pre-composing carries the comp's markers into the new comp, and leaves
    /// the Precomp layer without any: the same cues are on the ruler above, and
    /// drawing them again on the layer would say it twice.
    test('pre-composing carries markers in and leaves the layer bare', () {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final solid = comp.addSolidLayer();
      addMarkerFrb(comp, frame: 30, label: 'Chorus');

      final precomp = comp.precompose(
        layerIds: [solid.internallayerId],
        name: 'Packed',
        leaveAttributes: false,
        adjustDuration: false,
      );
      expect(precomp.getMarkers(), isEmpty,
          reason: 'the Precomp layer draws no markers of its own');
      expect(comp.getMarkers(), hasLength(1),
          reason: 'the outer comp keeps its own');

      final inner = precomp.getSourceItem();
      expect(inner, isA<ItemReference_Composition>());
      final packed = (inner as ItemReference_Composition).field0;
      expect(packed.getMarkers(), hasLength(1),
          reason: 'and the packed comp got a copy');
      expect(packed.getMarkers().single.label, 'Chorus');
      expect(packed.frameAtTime(time: packed.getMarkers().single.time), 30);
    });

    testWidgets('a layer marker draws on the bar and deletes from its menu',
        (tester) async {
      final p = withComp();
      final source = p.state.project!.newComposition(name: 'Beats');
      addMarkerFrb(source, frame: 20, label: 'Drop');
      final layer = p.comp.addPrecompLayer(comp: source);
      await mount(tester, p);

      final id = layer.getMarkers().single.id;
      final flag = find.byKey(ValueKey<String>('tl-layer-marker-$id'));
      expect(flag, findsOneWidget, reason: 'it draws on the layer\'s bar');

      await tester.tap(flag, buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tl-layer-marker-delete')));
      await tester.pumpAndSettle();

      expect(layer.getMarkers(), isEmpty);
      expect(source.getMarkers(), hasLength(1),
          reason: 'the composition it came from is untouched');
    });

    // --- the sequence view (K-248) --------------------------------------

    /// A Sequence layer, ready to open. Added layers land at the top of the
    /// stack, so it is always the first — which lets a test put something
    /// underneath it first.
    Future<LayerReference> sequencedLayer(dynamic p) async {
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.comp.addFootageLayer(footage: footage, asSequence: false);
      p.comp.getLayers().first.convertToSequenced();
      return p.comp.getLayers().first as LayerReference;
    }

    testWidgets('double-clicking a Sequence layer opens its clips in its row',
        (tester) async {
      final p = withComp();
      final layer = await sequencedLayer(p);
      await mount(tester, p);
      await tester.pump();

      final clip = layer.getClips().single;
      expect(find.byKey(ValueKey<String>('seq-clip-${clip.id}')), findsNothing,
          reason: 'shut until it is opened');

      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      expect(
          find.byKey(ValueKey<String>('seq-clip-${clip.id}')), findsOneWidget,
          reason: 'the clip is on screen');
      expect(find.byKey(const ValueKey('seq-envelope')), findsOneWidget,
          reason: 'and so is the speed envelope beneath it');

      // Double-clicking again shuts it.
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();
      expect(find.byKey(ValueKey<String>('seq-clip-${clip.id}')), findsNothing);
    });

    /// A trimmed clip draws the outline of the material trimmed away (K-441,
    /// docs/15 §12A.1) — the clip-level twin of the layer bar's own ghost.
    ///
    /// The source's length is the one thing the model cannot look up, so this
    /// needs a file that genuinely probes: a two-second WAV, which the probe
    /// answers a real duration for without needing a video encoder on the
    /// machine.
    testWidgets('a trimmed clip outlines the source it was cut from',
        (tester) async {
      final p = withComp();
      final footage =
          p.state.project!.importFootage(path: _oneSecondY4m('reach.y4m'));
      p.comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = p.comp.getLayers().first;
      layer.convertToSequenced();
      await mount(tester, p);

      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      final whole = layer.getClips().single;
      expect(
          find.byKey(ValueKey<String>('seq-clip-${whole.id}')), findsOneWidget,
          reason: 'the sequence view is open');
      expect(whole.reachStartFrame, isNotNull,
          reason: 'a probeable source has a knowable reach');
      expect(find.byKey(ValueKey<String>('seq-clip-ghost-${whole.id}')),
          findsNothing,
          reason: 'a clip showing all of its source has nothing to outline');

      // Trim the tail off, and what was cut away becomes the outline.
      layer.trimClip(
        clip: whole.id,
        startFrame: whole.startFrame,
        endFrame: whole.startFrame + 10,
      );
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      final trimmed = layer.getClips().single;
      final ghost =
          find.byKey(ValueKey<String>('seq-clip-ghost-${trimmed.id}'));
      expect(ghost, findsOneWidget);
      final box = tester
          .getRect(find.byKey(ValueKey<String>('seq-clip-${trimmed.id}')));
      expect(tester.getRect(ghost).right, greaterThan(box.right),
          reason: 'the outline reaches past the clip by what was trimmed off');

      // A retimed clip has no reach: its map decides which source moment each
      // of its frames shows, so its length stops being the source's business.
      layer.setClipSpeed(clip: trimmed.id, percent: 50, endPercent: 50);
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect(layer.getClips().single.reachStartFrame, isNull,
          reason: 'a retimed clip has no reach to draw');
      expect(find.byKey(ValueKey<String>('seq-clip-ghost-${trimmed.id}')),
          findsNothing);
    });

    /// A clip fills the way its layer's bar does (§12A.1): the label colour
    /// thinned, with the solid leading edge carrying it whole, so a run of
    /// cuts reads as a run of beginnings rather than a row of bright slabs.
    testWidgets('a clip fills desaturated with a solid leading edge',
        (tester) async {
      final p = withComp();
      final layer = await sequencedLayer(p);
      await mount(tester, p);
      await tester.pump();

      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      final clip = layer.getClips().single;
      final t = LumitTheme.dark();
      final label = t.labelColour(layer.getInfo().label);
      final deco = tester
          .widget<Container>(find.descendant(
            of: find.byKey(ValueKey<String>('seq-clip-${clip.id}')),
            matching: find.byType(Container),
          ))
          .decoration as BoxDecoration;
      expect(deco.color, label.withValues(alpha: clipFillAlpha));

      final edge = tester
          .widget<ColoredBox>(find.descendant(
            of: find.byKey(ValueKey<String>('seq-clip-edge-${clip.id}')),
            matching: find.byType(ColoredBox),
          ))
          .color;
      expect(edge, label, reason: 'the edge is the colour at full strength');
      expect(
          tester
              .getSize(find.byKey(ValueKey<String>('seq-clip-edge-${clip.id}')))
              .width,
          clipEdgeWidth);
    });

    /// The two halves of the Timeline must agree about how tall every row is.
    ///
    /// The outline has nothing of its own to draw for an open sequence view —
    /// the clips and their envelope are the lane's — so it has to reserve the
    /// room anyway. It did not, which put every row below the opened layer at
    /// a different height on each side: names stopped lining up with bars.
    testWidgets('opening a view moves the outline down with the lanes',
        (tester) async {
      final p = withComp();
      // The solid goes in *first*: a new layer lands at the top of the stack,
      // so this is the one that ends up below the sequenced layer — and below
      // is where the misalignment showed.
      final below = p.comp.addSolidLayer();
      final layer = await sequencedLayer(p);
      await mount(tester, p);
      await tester.pump();
      Rect nameOf() => tester.getRect(
          find.byKey(ValueKey<String>('tl-row-${below.internallayerId}')));
      Rect barOf() => tester.getRect(
          find.byKey(ValueKey<String>('tl-bar-${below.internallayerId}')));

      final gapBefore = nameOf().top - barOf().top;

      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      expect(nameOf().top - barOf().top, closeTo(gapBefore, 0.5),
          reason: 'the row below moved by the same amount on both sides');
    });

    /// A drag must survive the readout appearing.
    ///
    /// The readout is a `Stack` child that only exists while a drag runs, and
    /// unkeyed children are matched by position — so it took the gesture
    /// detector's slot, Flutter rebuilt that element, and the recogniser
    /// holding the drag went with it. The gesture ended the instant the
    /// readout showed up, one frame in. Driven here one step at a time,
    /// because a single synthetic move never reproduces it (the same reason
    /// K-212's first round of tests all passed).
    /// The velocity drag must stay linear.
    ///
    /// The axis grows to hold whatever a point reaches, so a point dragged
    /// past the floor widened it, which stretched what the next pixel of
    /// travel was worth, which pushed the point further still — a steady hand
    /// sent the value off exponentially. The axis is frozen for the length of
    /// a gesture now, so equal travel is equal change.
    testWidgets('an envelope drag stays linear', (tester) async {
      final p = withComp();
      final layer = await sequencedLayer(p);
      await mount(tester, p);
      await tester.pump();
      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      final clip = layer.getClips().single;
      final strip = tester.getRect(find.byKey(const ValueKey('seq-envelope')));
      final clipBox =
          tester.getRect(find.byKey(ValueKey<String>('seq-clip-${clip.id}')));
      // Near the clip's own start, so the point grabbed is its first — the
      // one whose speed is read back below.
      final from = Offset(clipBox.left + 3, strip.top + strip.height * 0.3);

      /// The first point's speed after dragging down by [by] pixels, in equal
      /// steps. Read off the map rather than `speedPercent`, which is null the
      /// moment the two ends differ — which is exactly what dragging one does.
      Future<double> after(double by) async {
        layer.setClipSpeed(clip: clip.id, percent: 100, endPercent: 100);
        // The panel draws from the read model, so a write behind its back has
        // to be published or the next drag starts from the last one's result.
        p.uiState.model.refresh();
        await tester.pumpAndSettle();
        final gesture = await tester.startGesture(from);
        for (var i = 0; i < 6; i++) {
          await gesture.moveBy(Offset(0, by / 6));
          await tester.pump();
        }
        await gesture.up();
        await tester.pumpAndSettle();
        final map = layer.getClips().single.retime;
        return envelopeSpeeds(keysOf(map)).first;
      }

      final near = 100 - await after(20);
      final far = 100 - await after(40);
      // Twice the travel, twice the change — not four times, or forty.
      expect(far, closeTo(near * 2, near * 0.25),
          reason: 'equal pixels are equal per cent, however far it has gone');
    });

    testWidgets('a drag keeps going once the readout appears', (tester) async {
      final p = withComp();
      final layer = await sequencedLayer(p);
      await mount(tester, p);
      await tester.pump();
      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      final before = layer.getClips().single;
      final strip = tester.getRect(find.byKey(const ValueKey('seq-envelope')));
      final clipBox =
          tester.getRect(find.byKey(ValueKey<String>('seq-clip-${before.id}')));
      final from = Offset(clipBox.center.dx, strip.top + strip.height * 0.3);

      // One event at a time, because a single synthetic move never
      // reproduces a drag that dies part way (the same reason K-212's first
      // round of tests all passed).
      final gesture = await tester.startGesture(from);
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(0, 8));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final after = layer.getClips().single;
      expect(after.speedPercent, isNotNull);
      // The whole 48px of travel, not the first step of it.
      final oneStepOnly = 100 - (8 / strip.height) * 161;
      expect(after.speedPercent!, lessThan(oneStepOnly - 20),
          reason: 'every move counted, not just the one before the readout '
              'appeared');
    });

    testWidgets('dragging an envelope point re-speeds only that clip',
        (tester) async {
      final p = withComp();
      final layer = await sequencedLayer(p);
      await mount(tester, p);
      await tester.pump();
      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      final before = layer.getClips().single;
      expect(before.retimed, isFalse, reason: 'plays at source rate to start');

      // Downwards is slower: the envelope runs fast at the top to backwards at
      // the bottom, so a drag down the strip lowers the speed. Started over
      // the clip's own span, which is what picks the clip whose line it is.
      final strip = tester.getRect(find.byKey(const ValueKey('seq-envelope')));
      final clipBox =
          tester.getRect(find.byKey(ValueKey<String>('seq-clip-${before.id}')));
      await tester.dragFrom(
        Offset(clipBox.center.dx, strip.top + strip.height * 0.3),
        const Offset(0, 30),
      );
      await tester.pumpAndSettle();

      final after = layer.getClips().single;
      expect(after.retimed, isTrue);
      expect(after.speedPercent, isNotNull);
      expect(after.speedPercent!, lessThan(100),
          reason: 'dragging down the strip slows the clip');
      // The covenant: the clip is still exactly where it was on the row.
      expect(after.startFrame, before.startFrame);
      expect(after.endFrame, before.endFrame);
    });

    testWidgets('the razor cuts a sequence clip where it is clicked',
        (tester) async {
      final p = withComp();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = p.comp.getLayers().single;
      layer.convertToSequenced();
      final sequenced = p.comp.getLayers().single;
      expect(sequenced.getClips(), hasLength(1));

      await mount(tester, p);
      await tester.pump();

      // Unarmed, a click on the bar does not cut.
      final bar = find
          .byKey(ValueKey<String>('tl-bar-body-${sequenced.internallayerId}'));
      expect(bar, findsOneWidget);
      final box = tester.getRect(bar);
      // **The middle of the clip, worked out in frames.** A Sequence layer's
      // own span is the comp's, but the clip inside it is only as long as its
      // (unreadable) media makes it, so a point a third of the way along the
      // *bar* can be past the end of the clip — where there is nothing to cut.
      // This used to be a flat `left + 8`, which is a pixel count standing in
      // for a frame: the day the outline narrowed by one column the lane grew
      // by the same amount, those 8 pixels bought fewer frames, and the razor
      // landed on the clip's first frame, where a cut is a no-op. Frames do not
      // move when a column does.
      final clip = sequenced.getClips().single;
      final middle = (clip.startFrame.toInt() + clip.endFrame.toInt()) / 2;
      final inside = Offset(
        box.left + box.width * middle / p.comp.durationFrames(),
        box.center.dy,
      );
      await tester.tapAt(inside);
      await tester.pump();
      expect(p.comp.getLayers().single.getClips(), hasLength(1),
          reason: 'the razor is a mode, not the default click');

      // The Timeline's menu item arms the toolbar's Razor tool (K-220) —
      // one razor, two doors.
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-razor')));
      await tester.pumpAndSettle();
      expect(p.uiState.tools.tool, ToolMode.razor);

      await tester.tapAt(inside);
      await tester.pumpAndSettle();

      expect(p.comp.getLayers().single.getClips(), hasLength(2),
          reason: 'the armed razor cut the clip under the pointer');
    });

    testWidgets('the cache meter reads the engine and clears on click',
        (tester) async {
      // The meter lives on the shell's status line now, so it is mounted
      // directly rather than through the Timeline.
      final p = withComp();
      await tester.pumpWidget(hostPanel(
        child: const CacheMeterFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.byKey(const ValueKey('cache-meter')), findsOneWidget);
      // One bar per tier, each with its own megabytes: a merged number cannot
      // answer "what is cached" for any of the three.
      expect(find.text('RAM'), findsOneWidget);
      expect(find.text('VRAM'), findsOneWidget);
      expect(find.text('Disk'), findsOneWidget);
      expect(find.textContaining('MB'), findsNWidgets(3),
          reason: 'the megabytes held read out beside each bar');
      // Clicking a tier empties that tier; the readout is live, so this must
      // not throw with no project rendered yet. The disk tier asks first rather
      // than deleting on a click — with nothing parked there is nothing to ask
      // about, so no dialogue appears and nothing happens.
      await tester.tap(find.byKey(const ValueKey('cache-meter-ram')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('cache-meter-vram')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('cache-meter-disk')));
      await tester.pump();
      expect(find.byKey(const ValueKey('cache-meter')), findsOneWidget);
    });
    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
    testWidgets('Detect beats is offered and is calm without audio',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      await openMore(tester);
      expect(find.byKey(const ValueKey('tl-detect-beats')), findsOneWidget);

      // No audio in this comp — and on CI no pipeline either. Either way the
      // command does nothing rather than raising, and no markers appear.
      await tester.tap(find.byKey(const ValueKey('tl-detect-beats')));
      await tester.pumpAndSettle();
      expect(p.comp.getMarkers(), isEmpty);
    });

    /// **An Adjustment layer wears the set's own Adjustment glyph**, not the
    /// Solid's fill-colour mark. It borrowed the solid's for as long as the set
    /// was thought to owe a drawing here; the drawing was already in the set,
    /// unused. A solid is one flat colour and an adjustment layer has no colour
    /// of its own at all, so the two must not read as the same kind of row.
    testWidgets('an adjustment layer draws the Adjustment glyph',
        (tester) async {
      expect(iconForKind(BridgeLayerKind.adjustment), LumitIcon.adjustment);
      expect(iconForKind(BridgeLayerKind.solid), LumitIcon.solid,
          reason: 'and a solid keeps its own');

      // And the glyph actually resolves: an icon the own-set switch misses
      // falls through to an empty box rather than raising, so the mapping
      // above is only half the claim.
      await tester.pumpWidget(Directionality(
        textDirection: TextDirection.ltr,
        child: lumitIcon(LumitIcon.adjustment,
            size: iconSize, color: const Color(0xffffffff)),
      ));
      final drawn =
          tester.widget<glyph.LumitIcon>(find.byType(glyph.LumitIcon));
      expect(drawn.glyph, LumitIcons.adjustment);
    });

    // -----------------------------------------------------------------------
    // 6.43 — the Animated filter (K-441).
    // -----------------------------------------------------------------------

    /// **Animated lists what is keyed, All brings the twirls back.** The filter
    /// reaches past the twirl set entirely: a layer that has never been opened
    /// shows its keyed rows the moment the filter is on, and a keyed row's
    /// headings come with it while the ones with nothing under them go.
    testWidgets('the Animated filter lists only the rows that carry keys',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [10, 40])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p);

      final id = layer.internallayerId;
      Finder row(String path) =>
          find.byKey(ValueKey<String>('tl-keys-prop-$id/$path'));

      // Shut by default, so nothing of the fold-out is drawn at all.
      expect(row('transform'), findsNothing);

      final filter = find.byKey(const ValueKey('tl-filter-animated'));
      expect(filter, findsOneWidget);
      await tester.tap(filter);
      await tester.pumpAndSettle();

      expect(row('transform'), findsOneWidget,
          reason: 'the heading that leads to the keyed row came with it');
      expect(row('transform/opacity'), findsOneWidget);
      expect(row('transform/position'), findsNothing,
          reason: 'a transform row with nothing keyed is not listed');

      // All: the twirl set is back in charge, and it says shut.
      await tester.tap(filter);
      await tester.pumpAndSettle();
      expect(row('transform'), findsNothing);
      expect(row('transform/opacity'), findsNothing);
    });

    /// A layer with nothing keyed anywhere lists nothing under it — but it is
    /// still a layer in the stack, so its own row stays. Hiding layers is the
    /// shy switch's job, not a property filter's.
    testWidgets('a layer with nothing keyed keeps its row and shows no rows',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('tl-filter-animated')));
      await tester.pumpAndSettle();

      final id = layer.internallayerId;
      expect(find.byKey(ValueKey<String>('tl-row-$id')), findsOneWidget);
      expect(find.byKey(ValueKey<String>('tl-keys-prop-$id/transform')),
          findsNothing);
    });
  }, skip: !engineAvailable);
}

/// A real one-second moving picture on disk, so a clip's source has a length
/// the probe can actually read.
///
/// **Y4M**, not a WAV and not a still. A sound file becomes an Audio layer,
/// which has no clips at all; a still has a picture but no length to be
/// trimmed out of. Y4M is the one moving-picture container that can be written
/// here byte by byte — a text header and raw planes, no encoder — so this
/// needs no ffmpeg CLI on the machine, which a widget test must not depend on.
///
/// 2x2 at 25 fps for 25 frames: with 4:2:0 chroma each frame is four luma
/// samples and one of each chroma, so the whole file is under three hundred
/// bytes.
String _oneSecondY4m(String name) {
  const width = 2;
  const height = 2;
  const frames = 25;
  final out = BytesBuilder();
  void ascii(String s) => out.add(s.codeUnits);

  ascii('YUV4MPEG2 W$width H$height F25:1 Ip A1:1 C420\n');
  for (var f = 0; f < frames; f++) {
    ascii('FRAME\n');
    // A mid-grey frame: four luma samples, then one sample of each chroma
    // plane at half resolution.
    out.add(Uint8List(width * height)..fillRange(0, width * height, 128));
    out.add(Uint8List(2)..fillRange(0, 2, 128));
  }

  final dir = Directory.systemTemp.createTempSync('lumit-reach');
  final file = File('${dir.path}/$name');
  file.writeAsBytesSync(out.takeBytes());
  return file.path;
}
