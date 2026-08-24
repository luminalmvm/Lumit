// The Timeline's two halves, as a user sees them: one table.
//
// **Why this file exists.** `timeline_panel_frb.dart` builds the outline and the
// lane area as two separate trees inside two separate vertical scrollables, kept
// in step by a shared `blockHeights` list and a scroll mirror. A refactor is
// planned that merges them into one row-per-layer tree in a single scrollable.
// Nothing currently fails if that merge quietly breaks the alignment, because no
// test builds the panel and *measures* it.
//
// So every claim here is about geometry a user could point at — a name and its
// bar sharing a top edge, both sides moving together when the table scrolls, the
// playhead standing where the clock says — and never about which widget class or
// which controller produces it. These must pass before the refactor and after
// it; a test that names `_Outline`, `_vOutline`, or "the second scroll view"
// would only be measuring today's implementation.

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Timeline alignment (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    // The outline alone is 800 px of columns; the default 800×600 surface would
    // push its right edge (and the lanes) off screen. Height is a parameter
    // because the scrolling claims need a viewport shorter than the stack.
    Future<void> mount(WidgetTester tester, dynamic p,
        {double height = 600}) async {
      tester.view.physicalSize = Size(1280, height);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: Size(1280, height),
      ));
      await tester.pump();
    }

    String idOf(LayerReference l) => l.internallayerId.toString();

    /// The layer's row in the outline — its name, number and switches.
    Rect outlineRow(WidgetTester tester, LayerReference l) =>
        tester.getRect(find.byKey(ValueKey<String>('tl-row-${idOf(l)}')));

    /// The same layer's bar in the lane area.
    Rect laneBar(WidgetTester tester, LayerReference l) =>
        tester.getRect(find.byKey(ValueKey<String>('tl-bar-${idOf(l)}')));

    /// Both halves of one layer's first row occupy the same band of the screen.
    void expectLevel(WidgetTester tester, LayerReference l, {String? why}) {
      final row = outlineRow(tester, l);
      final bar = laneBar(tester, l);
      expect(bar.top, closeTo(row.top, 0.5),
          reason: why ?? 'the name and the bar share a top edge');
      expect(bar.height, closeTo(row.height, 0.5),
          reason: why ?? 'and are the same height');
    }

    /// A mouse wheel notch over [at]. The panel hands a plain wheel to the
    /// scrollable — this is how the table is scrolled, on either side.
    Future<void> wheel(WidgetTester tester, Offset at, double dy) async {
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(pointer.hover(at));
      await tester.sendEventToBinding(pointer.scroll(Offset(0, dy)));
      await tester.pump();
    }

    /// The same wheel with a modifier held: Ctrl zooms time, Shift scrolls the
    /// lanes sideways (docs/07 §4.6).
    Future<void> modifiedWheel(WidgetTester tester, LogicalKeyboardKey key,
        Offset at, double dy) async {
      await tester.sendKeyDownEvent(key);
      await wheel(tester, at, dy);
      await tester.sendKeyUpEvent(key);
      await tester.pump();
    }

    /// 1. **At rest, every layer's two halves are one row.** The cheapest thing
    /// the merge can break, and the thing the whole table rests on.
    testWidgets('a layer\'s outline row and its lane bar share a row',
        (tester) async {
      final p = withComp();
      final layers = [
        for (var i = 0; i < 4; i++) p.comp.addSolidLayer(),
      ];
      p.uiState.model.refresh();
      await mount(tester, p);

      for (final l in layers) {
        expectLevel(tester, l);
      }
      // And they stack in the same order on both sides: the outline's tops
      // ascend exactly as the lanes' do.
      final rowTops = [for (final l in layers) outlineRow(tester, l).top];
      final barTops = [for (final l in layers) laneBar(tester, l).top];
      expect(
          barTops, orderedEquals([for (final t in rowTops) closeTo(t, 0.5)]));
      // Rows are stacked, not overlapping: consecutive tops differ by a row.
      final sorted = [...rowTops]..sort();
      for (var i = 1; i < sorted.length; i++) {
        expect(sorted[i] - sorted[i - 1], greaterThan(1),
            reason: 'four layers occupy four distinct bands');
      }
    });

    /// 2. **A fold-out opens on both sides at once.** This is what
    /// `blockHeights` exists to guarantee: the lane side must reserve exactly
    /// the room the outline's property rows take, or every layer below the
    /// opened one sits at a different height on the two sides.
    testWidgets('opening a layer\'s properties moves both halves equally',
        (tester) async {
      final p = withComp();
      // Bottom to top: [top, middle, bottom] as drawn.
      final bottom = p.comp.addSolidLayer();
      final middle = p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final beforeRow = outlineRow(tester, bottom).top;
      final beforeBar = laneBar(tester, bottom).top;
      expect(find.text('Transform'), findsNothing);

      await tester
          .tap(find.byKey(ValueKey<String>('tl-twirl-${idOf(middle)}')));
      await tester.pumpAndSettle();

      // The outline grew property rows.
      expect(find.text('Transform'), findsOneWidget,
          reason: 'the fold-out opened');
      // The lanes grew the matching block of lane rows.
      final lanes = find.byKey(ValueKey<String>('tl-lanes-${idOf(middle)}'));
      expect(lanes, findsOneWidget,
          reason: 'the lane side has a block for the fold-out rows');

      // Both sides pushed the layer below down, by the same distance.
      final shiftRow = outlineRow(tester, bottom).top - beforeRow;
      final shiftBar = laneBar(tester, bottom).top - beforeBar;
      expect(shiftRow, greaterThan(0), reason: 'the layer below moved down');
      expect(shiftBar, closeTo(shiftRow, 0.5),
          reason: 'and moved down by exactly as much on the lane side');

      // The lane block sits in the gap the outline opened, to the pixel.
      final laneRect = tester.getRect(lanes);
      expect(laneRect.top, closeTo(laneBar(tester, middle).bottom, 0.5),
          reason: 'the lanes start where the layer\'s own bar ends');
      expect(laneRect.height, closeTo(shiftRow, 0.5),
          reason: 'the lanes are exactly as tall as the outline\'s new rows');

      // And every layer is still level, opened one included.
      for (final l in [top, middle, bottom]) {
        expectLevel(tester, l, why: 'still level after the twirl');
      }
    });

    /// 3. **A Sequence layer's open view keeps the stack level.** Its outline
    /// side is a blank spacer (`sequenceExtra`) while its lane side draws real
    /// clips: two different widgets that must agree on one height, which is
    /// exactly the sort of pairing a merge is liable to drop.
    testWidgets('an open Sequence view keeps the layers below it level',
        (tester) async {
      final p = withComp();
      final below = p.comp.addSolidLayer();
      final seq = p.comp.addSequenceLayer();
      final above = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final beforeRow = outlineRow(tester, below).top;

      // Double-clicking a Sequence layer's bar opens its view. Retried,
      // because the first tap selects and can rebuild the row under the
      // second tap - on a loaded runner the rebuild lands between the two
      // and the gesture reads as two singles. The retry re-issues the
      // GESTURE; every assertion about what the open view looks like below
      // is as strict as it ever was.
      final bar = find.byKey(ValueKey<String>('tl-bar-body-${idOf(seq)}'));
      final room = find.byKey(ValueKey<String>('tl-seq-room-${idOf(seq)}'));
      for (var attempt = 0; attempt < 3; attempt++) {
        await tester.tap(bar);
        await tester.pump(const Duration(milliseconds: 30));
        await tester.tap(bar);
        await tester.pumpAndSettle();
        await settleFrb(tester, until: () => room.evaluate().isNotEmpty);
        if (room.evaluate().isNotEmpty) break;
        await tester.pump(const Duration(milliseconds: 400));
      }
      final view = find.byKey(ValueKey<String>('tl-seq-${idOf(seq)}'));
      expect(room, findsOneWidget,
          reason: 'the outline leaves the view its room');
      expect(view, findsOneWidget, reason: 'the lanes draw the view');
      expect(tester.getRect(room).height,
          closeTo(tester.getRect(view).height - _seqOwnRow, 0.5),
          reason:
              'the spacer holds everything the view added below the bar row');

      // The layer under it moved down, and its two halves moved together.
      expect(outlineRow(tester, below).top, greaterThan(beforeRow));
      expectLevel(tester, below, why: 'below an open Sequence view');
      expectLevel(tester, above, why: 'above an open Sequence view');
    });

    /// 4. **Scrolling moves both halves together, from either side.** Today two
    /// controllers mirror each other; after the merge there is one scrollable.
    /// The claim is the same either way: the row you were looking at stays one
    /// row.
    testWidgets('scrolling one side carries the other with it', (tester) async {
      final p = withComp();
      final layers = [
        for (var i = 0; i < 20; i++) p.comp.addSolidLayer(),
      ];
      p.uiState.model.refresh();
      // Short enough that twenty layers do not fit.
      await mount(tester, p, height: 300);

      final probe = layers.first; // the bottom-most row on screen
      final laneAt = laneBar(tester, layers.last).center;
      final outlineAt = outlineRow(tester, layers.last).center;

      final startRow = outlineRow(tester, probe).top;
      await wheel(tester, laneAt, 120);
      final afterLaneRow = outlineRow(tester, probe).top;
      expect(afterLaneRow, lessThan(startRow - 1),
          reason: 'a wheel over the lanes scrolled the outline too');
      expectLevel(tester, probe, why: 'after scrolling from the lane side');

      await wheel(tester, outlineAt, 120);
      final afterOutlineRow = outlineRow(tester, probe).top;
      expect(afterOutlineRow, lessThan(afterLaneRow - 1),
          reason: 'a wheel over the outline scrolled the lanes too');
      expectLevel(tester, probe, why: 'after scrolling from the outline side');

      // Everything still on screen is still level.
      for (final l in layers) {
        expectLevel(tester, l, why: 'after scrolling');
      }
    });

    /// 5. **Horizontal scroll belongs to the lanes alone**, and the ruler and
    /// cache bar ride with them because they share that viewport — while
    /// neither follows the rows vertically.
    testWidgets(
        'scrolling sideways moves the lanes, the ruler and the cache'
        ' bar, and never the outline', (tester) async {
      final p = withComp();
      final layers = [
        for (var i = 0; i < 20; i++) p.comp.addSolidLayer(),
      ];
      p.uiState.model.refresh();
      await mount(tester, p, height: 300);

      final ruler = find.byKey(const ValueKey('tl-ruler'));
      final cache = find.byKey(const ValueKey('tl-cache-bar'));
      final probe = layers.last;

      // Zoom in, or there is nothing to scroll sideways: at zoom 1 the whole
      // comp fits the lane viewport.
      final laneAt = laneBar(tester, probe).center;
      for (var i = 0; i < 8; i++) {
        await modifiedWheel(tester, LogicalKeyboardKey.controlLeft, laneAt, -1);
      }
      await tester.pumpAndSettle();

      final rowBefore = outlineRow(tester, probe);
      final barBefore = laneBar(tester, probe);
      final rulerBefore = tester.getRect(ruler);
      final cacheBefore = tester.getRect(cache);

      await modifiedWheel(tester, LogicalKeyboardKey.shiftLeft, laneAt, 120);
      await tester.pumpAndSettle();

      final barAfter = laneBar(tester, probe);
      expect(barAfter.left, lessThan(barBefore.left - 1),
          reason: 'the lanes scrolled sideways');
      expect(outlineRow(tester, probe).left, closeTo(rowBefore.left, 0.5),
          reason: 'the outline did not budge');
      expect(outlineRow(tester, probe).top, closeTo(rowBefore.top, 0.5),
          reason: 'and did not move vertically either');
      expect(tester.getRect(ruler).left - rulerBefore.left,
          closeTo(barAfter.left - barBefore.left, 0.5),
          reason: 'the ruler shares the lanes\' horizontal viewport');
      expect(tester.getRect(cache).left - cacheBefore.left,
          closeTo(barAfter.left - barBefore.left, 0.5),
          reason: 'so does the cache bar');
      expectLevel(tester, probe, why: 'after a sideways scroll');

      // Now scroll the rows: the ruler and cache bar are pinned above them.
      await wheel(tester, laneAt, 120);
      expect(tester.getRect(ruler).top, closeTo(rulerBefore.top, 0.5),
          reason: 'the ruler does not scroll with the rows');
      expect(tester.getRect(cache).top, closeTo(cacheBefore.top, 0.5),
          reason: 'nor does the cache bar');
    });

    /// 6. **The cross-row overlays span the table.** The playhead and the
    /// work-area wash are drawn over every row rather than inside any of them,
    /// which is exactly the arrangement a row-per-layer tree has to be careful
    /// to keep.
    testWidgets('the playhead spans the lanes and stands at the current time',
        (tester) async {
      final p = withComp();
      final layers = [
        for (var i = 0; i < 3; i++) p.comp.addSolidLayer(),
      ];
      p.uiState.playheadFrame.value = 0;
      p.uiState.model.refresh();
      await mount(tester, p);

      final frames = p.comp.durationFrames();
      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      Rect playhead() => tester.getRect(find.byType(PlayheadMarker));

      // The marker is centred on its frame, and the axis is the ruler's own
      // span *inside its padding* (docs/15 §12A.1) — so the clock and the
      // picture agree, and frame zero has a few pixels to its left to hang a
      // handle in.
      const pad = TimelineAxis.pad;
      final span = ruler.width - pad * 2;
      expect(playhead().center.dx, closeTo(ruler.left + pad, 1.0),
          reason: 'frame zero stands a padding in from the edge of the ruler');
      expect(playhead().left, greaterThan(ruler.left),
          reason: 'and the whole head is inside the ruler, so it can be seen');

      p.uiState.playheadFrame.value = frames ~/ 2;
      await tester.pump();
      expect(playhead().center.dx,
          closeTo(ruler.left + pad + span * (frames ~/ 2) / frames, 1.0),
          reason: 'half way along the comp is half way along the axis');

      // And it runs the full height of the lane side: from the ruler down past
      // the last layer's bar.
      final marker = playhead();
      expect(marker.top, lessThan(ruler.bottom),
          reason: 'the playhead starts up in the ruler');
      expect(marker.bottom, greaterThan(laneBar(tester, layers.first).bottom),
          reason: 'and carries on past the last row');
    });

    /// 6b. **The ruler is two rows, not one** (docs/15 §12A.1). The clock owns
    /// the upper half — the labels, the ticks and the playhead's head — and the
    /// lower half carries the markers and the work-area band, so a flag never
    /// sits on a tick and the band never tints the time.
    testWidgets('the ruler is double height, clock above, band below',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 40, label: 'Chorus');
      p.comp.setWorkArea(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 10),
          outPoint: p.comp.timeOfFrame(frame: 60),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      p.uiState.playheadFrame.value = 0;
      p.uiState.model.refresh();
      await mount(tester, p);

      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final waist = ruler.top + ruler.height / 2;

      final band = tester.getRect(find.byKey(const ValueKey('tl-work-area')));
      expect(band.top, closeTo(waist, 0.5),
          reason: 'the band hangs from the waist of the ruler');
      expect(band.bottom, closeTo(ruler.bottom, 0.5),
          reason: 'and reaches its floor, where the cache bar takes it on');

      final flag = tester.getRect(find
          .byKey(ValueKey<String>('tl-marker-${markersOf(p.comp).single.id}')));
      expect(flag.top, greaterThanOrEqualTo(waist - 0.5),
          reason: 'a marker lives in the lower half with the band');
      expect(flag.bottom, closeTo(ruler.bottom, 0.5),
          reason: 'standing on the cache bar');

      final head = tester.getRect(find.byType(PlayheadMarker).first);
      expect(head.top, closeTo(ruler.top, 0.5),
          reason: 'the playhead head is in the upper half, with the clock');
    });

    /// 6c. **The axis pads its two ends, and both halves of the table share the
    /// padding** (docs/15 §12A.1) — a handle on the first frame has room to be
    /// drawn and grabbed, and the lanes stay lined up with the ruler while it
    /// does.
    testWidgets('the axis pads both ends, ruler and lanes alike',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      p.comp.setWorkArea(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 0),
          outPoint: p.comp.timeOfFrame(frame: 40),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      p.uiState.model.refresh();
      await mount(tester, p);

      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final row =
          tester.getRect(find.byKey(ValueKey<String>('tl-bar-${idOf(layer)}')));
      final bar = tester
          .getRect(find.byKey(ValueKey<String>('tl-bar-body-${idOf(layer)}')));

      // A layer running the whole comp starts where frame zero is, and frame
      // zero is a padding in from the edge — in the lanes exactly as in the
      // ruler, because there is one axis.
      expect(row.left, closeTo(ruler.left, 0.5),
          reason: 'the lane row and the ruler are the same width');
      expect(bar.left, closeTo(row.left + TimelineAxis.pad, 0.5),
          reason: 'the lanes carry the same padding the ruler has');
      expect(bar.right, closeTo(row.right - TimelineAxis.pad, 0.5),
          reason: 'and the far end is padded too');

      // Which is what the padding is for: the work-area handle on frame zero
      // is inside the ruler, whole, rather than half off its edge.
      final handle =
          tester.getRect(find.byKey(const ValueKey('tl-work-start')));
      expect(handle.left, greaterThanOrEqualTo(ruler.left - 0.5),
          reason: 'the handle on the first frame is grabbable');
      expect(handle.center.dx, closeTo(ruler.left + TimelineAxis.pad, 0.5),
          reason: 'and centred on frame zero');
    });

    testWidgets('the work-area wash spans every row', (tester) async {
      final p = withComp();
      final layers = [
        for (var i = 0; i < 3; i++) p.comp.addSolidLayer(),
      ];
      p.uiState.model.refresh();
      await mount(tester, p);

      final wash = find.byWidgetPredicate(
          (w) => w is CustomPaint && w.painter is WorkAreaGroundPainter);
      expect(wash, findsWidgets,
          reason: 'the lanes are washed by the work area');
      final rect = tester.getRect(wash.first);
      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));

      expect(rect.width, closeTo(ruler.width, 0.5),
          reason: 'the wash covers the whole time axis, not one row');
      // It starts at the top of the rows and reaches past the last of them:
      // this is a cross-row overlay, not a decoration on any single row.
      expect(rect.top, lessThan(laneBar(tester, layers.last).top + 0.5));
      expect(
          rect.bottom, greaterThan(laneBar(tester, layers.first).bottom - 0.5));
    });

    /// 7. **A reorder drag lands the layer where the drop said it would.** The
    /// arithmetic has its own tests (`timeline_drag_test.dart`); what is
    /// untested is that the widget honours it — and that the row lands level
    /// afterwards rather than merely being renumbered.
    testWidgets('dragging a layer\'s name down reorders the stack',
        (tester) async {
      final p = withComp();
      final bottom = p.comp.addSolidLayer();
      final middle = p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      expect([
        for (final l in p.comp.getLayers()) l.internallayerId
      ], [
        top.internallayerId,
        middle.internallayerId,
        bottom.internallayerId
      ]);

      // Two full rows down: past the midpoint of both blocks below it, which is
      // the slot rule `layerDragTarget` uses. The first move only buys the
      // touch slop — the drag has not begun until it is crossed, so the travel
      // that decides the slot is what comes after it.
      final rowHeight = outlineRow(tester, top).height;
      final g = await tester.startGesture(tester
          .getCenter(find.byKey(ValueKey<String>('tl-name-${idOf(top)}'))));
      await g.moveBy(const Offset(0, kTouchSlop + 2));
      await tester.pump();
      await g.moveBy(Offset(0, rowHeight * 2));
      await tester.pump();
      await g.up();
      await tester.pumpAndSettle();

      expect([
        for (final l in p.comp.getLayers()) l.internallayerId
      ], [
        middle.internallayerId,
        bottom.internallayerId,
        top.internallayerId
      ], reason: 'the dragged layer landed at the bottom of the stack');

      // And the moved row is a row again: both halves level, in its new place.
      for (final l in [top, middle, bottom]) {
        expectLevel(tester, l, why: 'after a reorder drag');
      }
      expect(outlineRow(tester, top).top,
          greaterThan(outlineRow(tester, bottom).top),
          reason: 'it is drawn where the document says it is');
    });

    /// 9. **The mockups' heights are canonical** (K-451, docs/15 §12A.6). The
    /// panel's chrome is built to those logical pixels, not to approximations
    /// of them, so each one is measured here rather than trusted: a secondary
    /// row is 18, a lane row 22, and the ruler — counting the cache bar under
    /// it, which the table counts — is 36.
    testWidgets('the panel is built to K-451 heights', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final cache = tester.getRect(find.byType(CacheStrip).first);
      expect(ruler.height + cache.height, closeTo(36, 0.5),
          reason: 'the ruler is 36 with the cache bar counted inside it');

      // The outline's two secondary rows — timecode/search/mode, then the
      // column header — stand between the top of the panel's table and its
      // first row, and the ruler starts at that same top on the lane side.
      expect(outlineRow(tester, layer).top - ruler.top, closeTo(36, 0.5),
          reason: 'two 18px secondary rows sit above the first layer row');
      expect(outlineRow(tester, layer).height, closeTo(22, 0.5),
          reason: 'an outline row is 22');
      expect(laneBar(tester, layer).height, closeTo(22, 0.5),
          reason: 'and so is its bar');

      // The cache bar's own 3, which is what makes the clock above it 33.
      expect(cache.height, closeTo(TimelineCacheBar.height, 0.5));
      expect(TimelineCacheBar.height, 3,
          reason: 'the cache bar is the mockup\'s 3px stripe');

      // The panel header strip: the kicker, the comp tabs and Export, all at
      // 22 (§12A.6).
      expect(tester.getRect(find.byType(CompTabsFrb)).height, closeTo(22, 0.5),
          reason: 'the panel header strip is 22');
      expect(tester.getRect(find.byKey(const ValueKey('tl-export'))).height,
          lessThanOrEqualTo(22.5),
          reason: 'and the filled Export action fits inside it');
    });

    /// 10. **The pieces inside a row are the mockup's, not approximations of
    /// them** (K-451): a bar is 16 in a 22 row and centred in it, the layer's
    /// label colour is a 6px dot, and the number beside it stands in an 18px
    /// column.
    testWidgets('a lane row draws a 16px bar centred in its 22',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final row = laneBar(tester, layer);
      final bar = tester
          .getRect(find.byKey(ValueKey<String>('tl-bar-body-${idOf(layer)}')));
      expect(bar.height, closeTo(clipBarHeight, 0.5),
          reason: 'a clip bar within a lane row is 16 (§12A.6)');
      expect(bar.center.dy, closeTo(row.center.dy, 0.5),
          reason: 'and is centred in the row, ground above and below');

      // The label colour is a bullet, not a swatch.
      final dot = tester.getRect(find.descendant(
        of: find.byKey(ValueKey<String>('tl-label-${idOf(layer)}')),
        matching: find.byType(DecoratedBox),
      ));
      expect(dot.width, closeTo(6, 0.5));
      expect(dot.height, closeTo(6, 0.5));
    });

    /// 10b. **The bar's label is Hanken at 10** (§7.1, K-451) — the mockup's
    /// own size, and the face §7.1 gives everything the *user* named. It was
    /// mono at 11, which is the row the axis numbers keep.
    testWidgets('a bar\'s label is set in Hanken at 10', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final label = tester
          .renderObject<RenderParagraph>(
              find.byKey(ValueKey<String>('tl-bar-name-${idOf(layer)}')))
          .text
          .style!;
      expect(label.fontSize, 10);
      expect(label.fontFamily, LumitTheme.fontFamily,
          reason: 'a layer\'s own name is sentence-case Hanken, not mono');
    });

    /// 10c. **The pickers inside a row are 16 with a 10px label** (§12A.6's
    /// table, K-451): matte, blend and parent are cells in a 22px row, not
    /// dialog controls, and the mockup draws all three at the shorter face.
    testWidgets('the matte, blend and parent pickers are 16 tall',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final row = outlineRow(tester, layer);
      for (final cell in ['matte', 'blend', 'parent']) {
        final picker =
            find.byKey(ValueKey<String>('tl-$cell-${layer.internallayerId}'));
        expect(tester.getRect(picker).height, closeTo(inRowDropdownHeight, 0.5),
            reason: 'the $cell picker is the mockup\'s 16, not the 18 a '
                'dialog\'s dropdown stands at');
        expect(tester.getRect(picker).center.dy, closeTo(row.center.dy, 0.5),
            reason: 'and is centred in its 22px row');
        expect(
            tester
                .renderObject<RenderParagraph>(
                    find.descendant(of: picker, matching: find.byType(Text)))
                .text
                .style!
                .fontSize,
            inRowDropdownTextSize,
            reason: 'with the mockup\'s 10px label');
      }
    });

    /// 11. **The switches column never stretches** (§12A.1, K-448): its seam
    /// is not a handle, and a resize asked for anyway leaves it where it was.
    testWidgets('the switches column is pinned to its minimum width',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final seam = find.byKey(const ValueKey('tl-seam-switches'));
      expect(seam, findsOneWidget, reason: 'the rule is still drawn');
      expect(
        find.descendant(of: seam, matching: find.byType(GestureDetector)),
        findsNothing,
        reason: 'but there is nothing to take hold of',
      );

      final before =
          tester.getRect(find.byKey(const ValueKey('tl-seam-switches')));
      await tester.drag(seam, const Offset(60, 0));
      await tester.pump();
      expect(
          tester.getRect(find.byKey(const ValueKey('tl-seam-switches'))).left,
          closeTo(before.left, 0.5),
          reason: 'a drag on it widens nothing');
    });

    /// 12. **The comp-wide switches live in the bottom bar** (§12A.1), after
    /// the column toggles and behind a divider — and every command they carry
    /// is the one it always was.
    testWidgets('shy, motion blur and the overflow sit in the bottom bar',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final bar =
          tester.getRect(find.byKey(const ValueKey('tl-column-compose')));
      for (final key in ['tl-hide-shy', 'tl-mb-master', 'tl-more']) {
        final found = tester.getRect(find.byKey(ValueKey<String>(key)));
        expect(found.center.dy, closeTo(bar.center.dy, 1),
            reason: '$key rides in the bottom bar with the column toggles');
        expect(found.left, greaterThan(bar.right),
            reason: '$key sits after the Parent toggle');
      }
    });
  });
}

/// The Sequence view takes the layer's own bar row as the top of its clip
/// strip, so the outline's spacer holds everything *below* that row — one row
/// less than the view is tall (K-248).
const double _seqOwnRow = 22;
