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
import 'package:lumit_flutter/icons/icons.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/state/timeline_columns.dart';
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
        {double height = 600,
        DensityTokens density = DensityTokens.regular}) async {
      tester.view.physicalSize = Size(1280, height);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: Size(1280, height),
        density: density,
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
          reason: 'and reaches its floor, which the cache bar is drawn on');

      // The cache bar is *on* the band's row, at the ruler's floor — not a
      // strip of its own beneath it (§12A.1).
      final cache = tester.getRect(find.byType(TimelineCacheBar).first);
      expect(cache.bottom, closeTo(ruler.bottom, 0.5));
      expect(cache.top, greaterThan(band.top),
          reason: 'the band paints behind it');

      final flag = tester.getRect(find
          .byKey(ValueKey<String>('tl-marker-${markersOf(p.comp).single.id}')));
      expect(flag.top, greaterThanOrEqualTo(waist - 0.5),
          reason: 'a marker lives in the lower half with the band');
      expect(flag.bottom, closeTo(cache.top, 0.5),
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

    /// **And it stops where the composition does** (owner, 2026-08-24). The
    /// wash's *widget* is the whole lane area, which is [TimelineAxis.pad]
    /// wider than the comp at each end — that pad is grab room for a handle on
    /// the first or last frame, not composition. What it paints has to be the
    /// comp: the band used to run from the lane area's own left edge, six
    /// pixels before frame zero, while the ruler's band was laid out from
    /// `xOf(start)` and started in the right place. On a comp with no work
    /// area set — which is most of them — that put the band's own colour in
    /// the pad at both ends and made the strip look longer than the comp.
    ///
    /// Pinned at the painter, because the widget still fills the area: what
    /// changed is the clip, and the clip is where frame zero is.
    testWidgets('the work-area band starts at frame zero, not at the edge',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final wash = find.byWidgetPredicate(
          (w) => w is CustomPaint && w.painter is WorkAreaGroundPainter);
      final paint = tester.widgetList<CustomPaint>(wash).first;
      final painter = paint.painter! as WorkAreaGroundPainter;
      final area = tester.getRect(wash.first);

      // Frame zero's x *is* the pad, in the area's own pixels — that is what
      // `TimelineAxis.xOf(0)` returns at any zoom, so this needs no frame
      // count to state: the band begins one pad in, and ends one pad short.
      expect(painter.compStartX, closeTo(TimelineAxis.pad, 0.01),
          reason: 'the band begins at frame zero, one pad in from the edge');
      expect(painter.compEndX, closeTo(area.width - TimelineAxis.pad, 0.01),
          reason: 'and ends at the comp\'s end, a pad short of the far edge');

      // The clip is real, not just recorded: nothing is painted in the pad.
      expect(
          wash.first,
          paints
            ..clipRect(
                rect: Rect.fromLTRB(
                    painter.compStartX, 0, painter.compEndX, area.height)));
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
    /// of them, so each one is measured here rather than trusted — and against
    /// the density's own tokens rather than against numbers copied out of the
    /// table, so the pin cannot drift from what the app reads (K-454).
    testWidgets('the panel is built to K-451 heights', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      const d = DensityTokens.regular;

      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final cache = tester.getRect(find.byType(TimelineCacheBar).first);
      expect(ruler.height, closeTo(d.ruler, 0.5),
          reason: 'the ruler is the density\'s, cache bar counted inside it');
      // And the cache bar is *on* the ruler's floor, not a strip beneath it
      // (§12A.1): the work-area band is what it is drawn over.
      expect(cache.bottom, closeTo(ruler.bottom, 0.5),
          reason: 'the cache bar sits on the ruler floor, inside it');

      // The outline's two secondary rows — timecode/search/mode, then the
      // column header — stand between the top of the panel's table and its
      // first row, and the ruler starts at that same top on the lane side.
      // **That equality is the ruler's whole derivation** (§12A.6): the lane
      // side spends on its ruler exactly what the outline spends on its two
      // rows, so the two halves meet.
      expect(outlineRow(tester, layer).top - ruler.top,
          closeTo(d.secondaryRow * 2, 0.5),
          reason: 'two secondary rows sit above the first layer row');
      expect(outlineRow(tester, layer).height, closeTo(d.laneRow, 0.5),
          reason: 'an outline row is the density\'s lane row');
      expect(laneBar(tester, layer).height, closeTo(d.laneRow, 0.5),
          reason: 'and so is its bar');

      // The cache bar's own 3, at the bottom of the ruler's lower row.
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
    /// them** (K-451): a bar is 16 whatever the row measures and centred in it,
    /// the layer's label colour is a 6px dot, and the number beside it stands
    /// in an 18px column.
    testWidgets('a lane row draws a 16px bar centred in it', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final row = laneBar(tester, layer);
      final bar = tester
          .getRect(find.byKey(ValueKey<String>('tl-bar-body-${idOf(layer)}')));
      expect(bar.height, closeTo(clipBarHeight, 0.5),
          reason: 'a clip bar within a lane row is 16 under either '
              'density (§12A.6)');
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

    /// 10c. **The pickers inside a row wear the in-row face** (§12A.6's table,
    /// K-451): matte, blend and parent are cells in a layer's row, not dialog
    /// controls, and the mockup draws all three at the shorter face with a
    /// 10px label.
    testWidgets('the matte, blend and parent pickers are the density\'s',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final row = outlineRow(tester, layer);
      for (final cell in ['matte', 'blend', 'parent']) {
        final picker =
            find.byKey(ValueKey<String>('tl-$cell-${layer.internallayerId}'));
        expect(tester.getRect(picker).height,
            closeTo(DensityTokens.regular.inRowPicker, 0.5),
            reason: 'the $cell picker is the mockup\'s 18 under Regular, '
                'against the 20 a dropdown elsewhere in a panel stands at '
                '(K-454)');
        expect(tester.getRect(picker).center.dy, closeTo(row.center.dy, 0.5),
            reason: 'and is centred in its row');
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

    /// 10c-bis. **The layer-search well is the drawing's 16**, in a secondary
    /// row of 19 — ground above and below it, rather than a field filling its
    /// row edge to edge. Measured against the artboard, 2026-08-24: the well
    /// had sized itself to its own 16px glyph plus its hairline and come out
    /// at 18. (The in-row pickers were measured in the same pass and already
    /// matched the drawing's 18, so only this one moved.)
    testWidgets('the layer search sits in the drawing\'s 16px well',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      expect(tester.getRect(find.byKey(const ValueKey('tl-search'))).height,
          closeTo(layerSearchWellHeight, 0.5));
      expect(layerSearchWellHeight, 16,
          reason: 'which is the drawing\'s own value');
      expect(layerSearchWellHeight,
          lessThan(DensityTokens.compact.secondaryRow.toDouble()),
          reason: 'and leaves ground in the row at either density');
    });

    /// 10d. **Compact is the same panel, a pixel tighter** (K-454, §12A.6's
    /// second column). The setting reaches the rows that matter — the layer
    /// rows and the panel's own chrome — and it reaches them through the
    /// theme, so the outline and the lanes move together: a table whose two
    /// halves agreed at one density and not at the other would be worse than
    /// no setting at all.
    testWidgets('Compact draws the table\'s tighter column', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p, density: DensityTokens.compact);
      const d = DensityTokens.compact;

      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final row = outlineRow(tester, layer);
      expect(row.height, closeTo(22, 0.5), reason: 'a lane row is 22');
      expect(ruler.height, closeTo(36, 0.5), reason: 'the ruler is 36');
      // The ruler is still exactly the two secondary rows the outline spends
      // opposite it, which is what holds the halves level at either density.
      expect(row.top - ruler.top, closeTo(36, 0.5),
          reason: 'two 18px secondary rows stand above the first layer row');
      expect(
          tester
              .getRect(find.byKey(const ValueKey('tl-lane-bottom-bar')))
              .height,
          closeTo(18, 0.5),
          reason: 'a panel bottom bar is a secondary row, and so 18');
      expect(d.secondaryRow, 18, reason: 'which is the token\'s own value');

      for (final cell in ['matte', 'blend', 'parent']) {
        expect(
            tester
                .getRect(find.byKey(
                    ValueKey<String>('tl-$cell-${layer.internallayerId}')))
                .height,
            closeTo(16, 0.5),
            reason: 'an in-row picker is 16 under Compact');
      }

      // And the halves still meet, which is the whole point of the derivation.
      expectLevel(tester, layer, why: 'under Compact');
    });

    /// 10d-bis. **The identity cluster is the drawing's** (K-461): twirl,
    /// then the layer number, then the label dot, then the name — and 8 px of
    /// air between each of the three marks, where the outline used to run the
    /// twirl hard against the dot. Measured as edges rather than as widget
    /// classes, because the claim is about what a reader sees in what order.
    testWidgets(
        'the outline reads twirl, number, dot, name at the drawing\'s '
        'gaps', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      final id = idOf(layer);

      final row = find.byKey(ValueKey<String>('tl-row-$id'));
      final twirl =
          tester.getRect(find.byKey(ValueKey<String>('tl-twirl-$id')));
      final number = tester
          .getRect(find.descendant(of: row, matching: find.text('1')).first);
      final dot = tester.getRect(find.byKey(ValueKey<String>('tl-label-$id')));
      final name = tester.getRect(find.byKey(ValueKey<String>('tl-name-$id')));

      expect(twirl.right, lessThanOrEqualTo(number.left),
          reason: 'the twirl opens the cluster');
      expect(number.right, lessThanOrEqualTo(dot.left),
          reason: 'the number is the row\'s address and comes before the dot '
              '(K-461 — they used to stand the other way round)');
      expect(dot.right, lessThanOrEqualTo(name.left),
          reason: 'and the dot belongs to the name it colours');

      // The number's cell is 18 wide, so its glyph does not fill it: the gaps
      // are measured off the cells, which is what the drawing dimensions.
      expect(number.left - twirl.right, closeTo(identityGap, 0.5),
          reason: 'the drawing sets 8 between the twirl and the number');
      expect(dot.left - (number.left + 18), closeTo(identityGap, 0.5),
          reason: 'and 8 between the number\'s 18px cell and the dot\'s');
      expect(identityGap, 8, reason: 'which is the constant\'s own value');
    });

    /// 10d-ter. **The compose columns start at their content** (K-461): the
    /// drawing's 84 / 84 / 64 faces, and — with no matte set anywhere in the
    /// comp — nothing else (K-463). They had been 118 / 112 / 96, which is
    /// slack no picker ever used; then the matte column kept the mode toggles'
    /// 28 whether or not they were drawn, which read as a hole between the
    /// matte and the blend on every row of every comp without a matte.
    testWidgets('matte, blend and parent start at the drawing\'s widths',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      expect((matteFaceWidth, blendCellWidth, parentCellWidth), (84.0, 84, 64),
          reason: 'the drawing\'s dropdown faces');
      expect(composeGroupWidth, 84 + cellGap + 84 + cellGap + 64,
          reason: 'and the group at rest is the three of them and the gaps '
              'between — no room held back for toggles nobody has asked for');
      expect(composeGroupWidth, lessThan(334),
          reason: 'which is narrower than the 334 that shipped');

      // **The faces themselves are 84 / 84 / 64** (owner, 2026-08-24): a
      // dropdown never swells past the width the drawing gives it.
      for (final (cell, width) in [
        ('matte', matteFaceWidth),
        ('blend', blendCellWidth),
        ('parent', parentCellWidth),
      ]) {
        expect(
            tester
                .getRect(find.byKey(
                    ValueKey<String>('tl-$cell-${layer.internallayerId}')))
                .width,
            closeTo(width, 1.0),
            reason: 'the $cell face is the drawing\'s width at rest');
      }
      expect(minGroupWidth(TimelineGroup.compose),
          lessThanOrEqualTo(composeGroupWidth),
          reason: 'and the group can still be dragged narrower than it starts');
    });

    /// 10d-sexies. **The matte column widens for the toggles, and only while a
    /// matte is set** (owner, K-463). With one set, the two mode toggles have
    /// to fit between the matte face and the blend column — on the row that
    /// has the matte *and* on the rows that do not, or the blend column stops
    /// being a column.
    testWidgets('a matte set carries its toggles without crowding the blend',
        (tester) async {
      final p = withComp();
      final source = p.comp.addSolidLayer();
      final gated = p.comp.addSolidLayer();
      gated.setMatte(
          matte: BridgeMatte(
              layer: source.internallayerId, luma: false, inverted: false));
      p.uiState.model.refresh();
      await mount(tester, p);

      Rect at(String key) => tester.getRect(find.byKey(ValueKey<String>(key)));

      for (final (what, id) in [
        ('the row with the matte', gated.internallayerId),
        ('the row without one', source.internallayerId),
      ]) {
        expect(at('tl-matte-$id').width, closeTo(matteFaceWidth, 1.0),
            reason: 'the face is still the drawing\'s 84 on $what');
        expect(at('tl-blend-$id').left - at('tl-matte-$id').right,
            closeTo(outlineGap + matteToggleWidth, 1.0),
            reason: 'and the toggles\' room stands before the blend on $what, '
                'so the two rows keep the same columns');
      }

      // The toggles themselves, on the row that has them: inside the room, and
      // clear of both neighbours.
      final id = gated.internallayerId;
      expect(at('tl-matte-luma-$id').left,
          greaterThanOrEqualTo(at('tl-matte-$id').right - 0.5),
          reason: 'the luma toggle starts where the face ends');
      expect(at('tl-matte-invert-$id').right,
          lessThanOrEqualTo(at('tl-blend-$id').left + 0.5),
          reason: 'and the invert toggle ends before the blend picker');
    });

    /// 10d-quinquies. **One gap, everywhere in an outline row: 8** (owner,
    /// 2026-08-24). The drawing's rows are a single flex line with `gap: 8`
    /// and 8 of padding at each end — one even space between every mark in
    /// the row. The outline had three values: 8 inside the identity cluster,
    /// 4 between the compose pickers, and 7 for a cluster seam.
    ///
    /// Measured across the row rather than read off the constants, because
    /// the claim is about what the eye walks past.
    testWidgets('every gap in an outline row is the drawing\'s 8',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      final id = layer.internallayerId;

      expect((outlineGap, identityGap, cellGap, groupDividerWidth),
          (8.0, 8.0, 8.0, 8.0),
          reason: 'the gap inside a cluster, between its cells, and at a '
              'cluster seam are one number');

      Rect at(String key) =>
          tester.getRect(find.byKey(ValueKey<String>('tl-$key-$id')));

      // Inside a switch cluster the drawing is tighter — its switches div sets
      // `gap: 6` — and so is this: the cells abut, and a cell of
      // `switchCellWidth` around a 16px glyph stands them 6 apart while the
      // whole cell stays the click target (§7.2).
      expect(at('locked').left - at('solo').left, closeTo(switchCellWidth, 0.5),
          reason: 'switch cells abut');
      expect(switchCellWidth - iconSize, closeTo(6, 0.5),
          reason: 'and the glyphs inside a cluster stand at the drawing\'s 6');

      // Across the row: the two cluster seams a solid layer draws cells on
      // either side of, and then picker to picker.
      expect(at('twirl').left - at('shy').right, closeTo(outlineGap, 0.5),
          reason: 'the seam between the switches and the identity cluster is '
              'one gap, where it had been 7');
      expect(at('matte').left - at('3d').right, closeTo(outlineGap, 0.5),
          reason: 'as is the seam between the modes and the pickers — the '
              'name\'s own trailing 4 is gone with it');
      expect(at('blend').left - at('matte').right, closeTo(outlineGap, 0.5),
          reason: 'and the matte face is one gap from the blend on a comp with '
              'no matte set — the toggles\' room appears with the first matte '
              'and not before (K-463)');
      expect(at('parent').left - at('blend').right, closeTo(outlineGap, 0.5),
          reason: 'as the blend is from the parent');
    });

    /// 10d-quater. **A lane key is the drawing's 11 in Layers mode too**
    /// (K-459), and its mark is **split at its vertical centre** — the left
    /// half drawn from the interpolation coming in, the right half from the
    /// one going out (K-457).
    ///
    /// Two claims, asked two ways: the size off the rendered lane, because
    /// that is what a reader aims at; the shapes off the geometry itself,
    /// because a triangle's tip is a fact about the path and reading it back
    /// out of painted pixels would only be measuring the renderer.
    testWidgets('a lane key stands 11 tall in Layers mode', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 40),
            value: 0,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.hold(),
          ),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-${idOf(layer)}')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Transform'));
      await tester.pumpAndSettle();

      // The lane is there and drawing keys — the size claim below is about
      // the constant both modes now share.
      expect(
          find.byKey(const ValueKey<String>('tl-lane-diamonds')), findsWidgets,
          reason: 'a twirled-open property draws its own lane of keys');
      expect(laneKeyHalf * 2, 11,
          reason: 'a key stands 11 point to point in Layers mode as it does '
              'in Keys — Layers used to draw them at 8 (K-459)');
      expect(
          tester
              .getRect(
                  find.byKey(const ValueKey<String>('tl-lane-diamonds')).first)
              .height,
          greaterThanOrEqualTo(laneKeyHalf * 2),
          reason: 'and the lane it is drawn in has the room for it');
    });

    /// 10d-quinquies. **The mark is split at its vertical centre** (K-457):
    /// each half is its own side's shape, all three shapes stand the same
    /// height, and a bezier side is the **hourglass** — two triangles tip to
    /// tip — which supersedes the rounded shape Keys mode first drew.
    test('a key\'s two halves are drawn from their own sides', () {
      const x = 100.0, mid = 10.0, half = laneKeyHalf;
      Path pathOf(KeyShape s, {required bool left}) =>
          keyHalfPath(s, x, mid, half, left: left);

      // One height, every shape, either side — the whole of "all at one
      // height", and the thing that keeps a mixed lane reading as one row.
      for (final shape in KeyShape.values) {
        for (final left in [true, false]) {
          final box = pathOf(shape, left: left).getBounds();
          expect(box.height, closeTo(half * 2, 0.01),
              reason: '$shape stands the same height as the rest');
          expect(left ? box.right : box.left, closeTo(x, 0.01),
              reason: '$shape is split on the centre line, not beside it');
        }
      }

      // The diamond comes to a point at top and bottom; the square does not.
      // Sampled a whisker inside the top edge, on the half's own side.
      final justInside = Offset(x - half + 0.5, mid - half + 0.5);
      expect(pathOf(KeyShape.diamond, left: true).contains(justInside), isFalse,
          reason: 'a diamond is a point at the top, so its corner is empty');
      expect(
          pathOf(KeyShape.square, left: true)
              .contains(Offset(x - 1, mid - half + 0.5)),
          isTrue,
          reason: 'a square is at full width there');

      // The hourglass: wide at top and bottom, pinched to nothing at the
      // centre — which is what "two triangles tip to tip" means and what a
      // circle could never say.
      final hourglass = pathOf(KeyShape.hourglass, left: true);
      expect(hourglass.contains(Offset(x - 1, mid - half + 0.5)), isTrue,
          reason: 'wide where the value is furthest from the key');
      expect(hourglass.contains(Offset(x - 1, mid - 0.5)), isFalse,
          reason: 'and pinched to the centre point the mark is split on');
      expect(hourglass.contains(Offset(x - 1, mid + half - 0.5)), isTrue,
          reason: 'the second triangle stands under the first');
    });

    /// 10e. **Keys mode is the same table** (K-455): the dope sheet swaps the
    /// body under the shared ruler, so its rows take the density's lane row
    /// on both sides and its own filter row stands exactly where the column
    /// header did. Measured under Compact, because a mode that derived its
    /// own heights would drift here first.
    testWidgets('the Keys sheet follows the density like every other row',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [0, 60])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p, density: DensityTokens.compact);
      const d = DensityTokens.compact;

      await tester.tap(find.byKey(const ValueKey('tl-view-keys')));
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(ValueKey<String>('tl-keys-twirl-${idOf(layer)}')));
      await tester.pumpAndSettle();

      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final row = tester
          .getRect(find.byKey(ValueKey<String>('tl-keys-row-${idOf(layer)}')));
      final lane = tester.getRect(
          find.byKey(ValueKey<String>('tl-keys-layer-${idOf(layer)}')));
      final prop = tester.getRect(find.byKey(
          ValueKey<String>('tl-keys-prop-${idOf(layer)}/transform/opacity')));

      expect(row.height, closeTo(d.laneRow, 0.5));
      expect(prop.height, closeTo(d.laneRow, 0.5));
      expect(row.top - ruler.top, closeTo(d.secondaryRow * 2, 0.5),
          reason: 'the timecode row and the filter row stand above the first '
              'layer, exactly the ruler opposite them');
      expect(lane.top, closeTo(row.top, 0.5),
          reason: 'the dope sheet\'s halves are level too');
      expect(lane.height, closeTo(row.height, 0.5));
    });

    /// 11. **Neither column of switches ever stretches** (§12A.1, K-448; Modes
    /// joined Switches on the owner's word, 2026-08-24): the seam is not a
    /// handle, and a resize asked for anyway leaves it where it was. Both are
    /// rows of icons — a wider column buys blank space and nothing else.
    testWidgets('the switch columns are pinned to their minimum width',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      // Modes is exactly its switch cells and no more. Four of them since the
      // accepts-lights switch left the column for the row menu (owner).
      expect(renderGroupWidth, 4 * switchCellWidth);

      for (final group in [TimelineGroup.switches, TimelineGroup.render]) {
        final key = ValueKey<String>('tl-seam-${group.name}');
        final seam = find.byKey(key);
        expect(seam, findsOneWidget,
            reason: '${group.name}: the rule is still drawn');
        expect(
          find.descendant(of: seam, matching: find.byType(GestureDetector)),
          findsNothing,
          reason: '${group.name}: but there is nothing to take hold of',
        );

        final before = tester.getRect(seam);
        await tester.drag(seam, const Offset(60, 0));
        await tester.pump();
        expect(tester.getRect(find.byKey(key)).left, closeTo(before.left, 0.5),
            reason: '${group.name}: a drag on it widens nothing');
      }
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
/// less than the view is tall (K-248). That row is the table's, and so the
/// density's (K-454), not one of the view's own.
final double _seqOwnRow = DensityTokens.regular.laneRow;
