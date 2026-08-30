// How many *widgets* one interaction rebuilds — the regression trap for the
// other kind of chatter.
//
// `bridge_call_budget_test` counts what crosses the seam. This counts what the
// framework redraws on this side of it, which is where the owner's "the
// playhead lags the pointer" actually lived: no bridge call at all, just the
// Effect controls panel listening to the playhead at its root and rebuilding
// every card, every row and every button on each frame of a scrub. Measured at
// 327 widgets per playhead move on a three-layer, three-effect project — and it
// grows with the document, so a real project paid it many times over.
//
// The rule the numbers below pin: **a scrub redraws the rows that follow the
// playhead, and nothing else.** A row whose channels are all static cannot
// change under a scrub, so it does not listen; a keyed one does, and the second
// test is what stops the first being "satisfied" by a panel that has simply
// stopped following the playhead at all.
//
// Counting is done by reading the framework's own dirty-widget log
// (`debugPrintRebuildDirtyWidgets`), which names every element it rebuilds.

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

/// Counts widget rebuilds by name, from the framework's own log.
class _Rebuilds {
  final Map<String, int> byName = {};
  bool counting = false;
  DebugPrintCallback? _previous;

  void install() {
    _previous = debugPrint;
    debugPrint = (String? message, {int? wrapWidth}) {
      if (!counting || message == null) return;
      // `Rebuilding (dirty, …): Widget` and `Building Widget` both end in the
      // widget's own description.
      var line = message;
      final tail = line.lastIndexOf('): ');
      if (tail >= 0) line = line.substring(tail + 3);
      line = line.replaceFirst(RegExp(r'^(Building|Rebuilding)\s+'), '');
      final name = line.trim().split(RegExp(r'[\s(<{-]')).first;
      byName[name] = (byName[name] ?? 0) + 1;
    };
    debugPrintRebuildDirtyWidgets = true;
  }

  /// Both globals back where they were — `flutter_test` fails the test if a
  /// foundation debug variable is left set.
  void remove() {
    debugPrintRebuildDirtyWidgets = false;
    if (_previous != null) debugPrint = _previous!;
  }

  int get total => byName.values.fold(0, (a, b) => a + b);
  void reset() => byName.clear();

  String ranking() {
    final entries = byName.entries.toList()
      ..sort((a, b) => b.value.compareTo(a.value));
    return entries.take(15).map((e) => '${e.value}x ${e.key}').join('\n');
  }
}

void main() {
  setUpAll(initEngineForTests);

  group('Rebuild budget', () {
    late _Rebuilds rebuilds;

    setUp(() {
      rebuilds = _Rebuilds()..install();
    });
    tearDown(() => rebuilds.remove());

    /// Mount the two panels a scrub is felt in, over a small but honest
    /// document: two layers, three effects, every parameter still.
    Future<({dynamic ui, dynamic comp})> mount(WidgetTester tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final solid = comp.addSolidLayer()..addEffect(name: 'blur');
      solid.addEffect(name: 'sharpen');
      comp.addTextLayer().addEffect(name: 'blur');
      p.uiState.setSelectedComp(comp);
      p.uiState.selectedLayer.value = comp.getLayers().first;
      // The layer cards off, so the effect stack is at the top of the panel and
      // its rows are inside the viewport — a `ListView` builds no row it cannot
      // show, and a row that was never built cannot be counted.
      p.uiState.workspace.interface.transformInEffectControls = false;
      p.uiState.model.refresh();

      tester.view.physicalSize = const Size(1600, 800);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(1600, 800),
        child: const Row(children: [
          SizedBox(width: 900, height: 800, child: TimelinePanelFrb()),
          Expanded(child: EffectControlsPanelFrb()),
        ]),
      ));
      await settleFrb(tester, minRounds: 8);
      return (ui: p.uiState, comp: comp);
    }

    testWidgets('a scrub redraws the playhead and little else', (tester) async {
      final p = await mount(tester);

      rebuilds
        ..reset()
        ..counting = true;
      for (var frame = 1; frame <= 20; frame++) {
        p.ui.playheadFrame.value = frame;
        // Real frames with time on the clock, as in the bridge budgets: work
        // grouped "once per frame" sees one frame for a whole run of bare
        // pumps, and the count would be a fiction.
        await tester.pump(const Duration(milliseconds: 16));
      }
      rebuilds
        ..counting = false
        ..remove();

      // ignore: avoid_print
      print('SCRUB REBUILDS ${rebuilds.total}\n${rebuilds.ranking()}');
      // **The panel does not follow the playhead.** Every card and every row in
      // Effect controls used to, which is what made a flick of the playhead lag
      // the pointer.
      expect(
        rebuilds.byName['FxSection'] ?? 0,
        0,
        reason: 'an effect card redrew on a scrub:\n${rebuilds.ranking()}',
      );
      // Nor does a still row, which has no diamond to fill and no curve to
      // sample. Counted at its stopwatch: a parameter row's own listener sits
      // *inside* its build, so what a scrub redraws is the subtree under that
      // listener, and this is the first widget in it that has a name worth
      // asserting on. This is the count that grows with the document.
      expect(
        rebuilds.byName['KeyframeControlsFrb'] ?? 0,
        0,
        reason: 'a still parameter row redrew on a scrub:\n'
            '${rebuilds.ranking()}',
      );
      // Measured at 300 for twenty moves — the playhead line and the time
      // readout, which are exactly the two things a scrub changes. The cap is
      // roughly 2x that, in the house style, so honest growth does not trip it;
      // what must never come back is a count in the thousands.
      expect(
        rebuilds.total,
        lessThan(700),
        reason: 'a scrub redrew far too much:\n${rebuilds.ranking()}',
      );
    });

    /// **What a scrub *paints*, which is not the same question.**
    ///
    /// Rebuilding little is no help if the framework still redraws everything:
    /// the playhead was a `Positioned` child of the same `Stack` the lanes sit
    /// in, so moving it relaid that stack out — and repainting a stack repaints
    /// every child of it that has no layer of its own. The bars and lanes were
    /// redrawn for a vertical line moving over them, and the bill grew with
    /// every row a `U` opened. That is the owner's "laggy even over cached
    /// frames".
    ///
    /// Counted off the boundaries themselves: a `RenderRepaintBoundary` records
    /// each time it is actually repainted, so this asks the render tree what it
    /// did rather than inferring it.
    testWidgets('a scrub repaints the playhead and not the lanes',
        (tester) async {
      final p = await mount(tester);
      // Twirled open, so the lanes carry rows and not only bars — the state the
      // complaint is about.
      for (final layer in p.comp.getLayers()) {
        final id = layer.internallayerId.toString();
        await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
        await tester.pump();
      }
      await settleFrb(tester, minRounds: 4);
      // This one counts paints, not rebuilds, and the framework refuses to end
      // a test with a foundation debug flag still set.
      rebuilds.remove();

      int paints(String key) {
        final boundary = tester.renderObject<RenderRepaintBoundary>(
            find.byKey(ValueKey<String>(key)).first);
        return boundary.debugSymmetricPaintCount +
            boundary.debugAsymmetricPaintCount;
      }

      final lanesBefore = paints('tl-lane-blocks');
      final headBefore = paints('tl-playhead-layer');
      for (var frame = 1; frame <= 20; frame++) {
        p.ui.playheadFrame.value = frame;
        await tester.pump(const Duration(milliseconds: 16));
      }

      expect(paints('tl-lane-blocks'), lanesBefore,
          reason: 'the lanes were redrawn for a playhead that moved over them');
      // The guard, without which a lane area that had stopped drawing at all
      // would pass: the line itself must have been redrawn, twenty times.
      expect(paints('tl-playhead-layer'), greaterThan(headBefore),
          reason: 'the playhead did not redraw, so nothing was measured');
    });

    /// **The same question for the work area's band** (the owner's "dragging a
    /// work-area edge on a large timeline is incredibly laggy").
    ///
    /// The band spans the ruler and every lane under it, so an edge drag has
    /// three washes to move: the lanes' ground, the wash over the bars, and the
    /// graph's. The panel used to move them by holding the staged span in its
    /// own state and calling `setState` per pointer move — which rebuilt the
    /// whole Timeline, outline and rows and key counts included. Measured at
    /// 59,129 widgets for twenty pointer moves on a twenty-layer comp twirled
    /// open (~2,950 a move), and it grows with the document.
    ///
    /// The rule this pins: **an edge drag repaints the band and rebuilds the
    /// ruler, and nothing else.**
    testWidgets('a work-area edge drag repaints the band and not the lanes',
        (tester) async {
      final p = await mount(tester);
      for (final layer in p.comp.getLayers()) {
        final id = layer.internallayerId.toString();
        await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
        await tester.pump();
      }
      await settleFrb(tester, minRounds: 4);

      int paints(String key) {
        final boundary = tester.renderObject<RenderRepaintBoundary>(
            find.byKey(ValueKey<String>(key)).first);
        return boundary.debugSymmetricPaintCount +
            boundary.debugAsymmetricPaintCount;
      }

      final handle = find.byKey(const ValueKey<String>('tl-work-end'));
      expect(handle, findsOneWidget);
      final gesture = await tester.startGesture(tester.getCenter(handle));
      await tester.pump();

      final lanesBefore = paints('tl-lane-blocks');
      final bandBefore = paints('tl-lane-ground');
      rebuilds
        ..reset()
        ..counting = true;
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(-6, 0));
        await tester.pump(const Duration(milliseconds: 16));
      }
      rebuilds
        ..counting = false
        ..remove();
      final lanesAfter = paints('tl-lane-blocks');
      final bandAfter = paints('tl-lane-ground');
      // Let go before asserting, so a failure does not leave a drag standing.
      await gesture.up();
      await tester.pump();

      // ignore: avoid_print
      print('WORK-AREA DRAG REBUILDS ${rebuilds.total} band '
          '${bandAfter - bandBefore} lanes ${lanesAfter - lanesBefore}\n'
          '${rebuilds.ranking()}');
      expect(lanesAfter, lanesBefore,
          reason: 'the lanes were redrawn for a band moving over them');
      // The guard: a panel that had simply stopped following the hand would
      // satisfy the line above. The band must be shown to have moved.
      expect(bandAfter, greaterThan(bandBefore),
          reason: 'the work-area band did not redraw, so nothing was measured');
      // Measured at 254 for twenty moves — the ruler, its two handles and the
      // band it draws itself. The cap is roughly 2x, in the house style; what
      // must never come back is a count in the thousands, which is the panel
      // rebuilding whole.
      expect(rebuilds.total, lessThan(700),
          reason: 'an edge drag redrew far too much:\n${rebuilds.ranking()}');
      // And the signature of that regression, named: an outline row and a lane
      // bar have nothing to do with where the work area ends.
      for (final name in ['OutlineRow', 'Bar']) {
        expect(rebuilds.byName[name] ?? 0, 0,
            reason: 'a $name redrew on a work-area drag:\n'
                '${rebuilds.ranking()}');
      }
    });

    /// The other half of the rule, and the reason the first test cannot be
    /// passed by a panel that has simply gone deaf: a **keyed** parameter shows
    /// the value under the playhead, so its row must redraw on every move.
    testWidgets('a keyed parameter row still follows the playhead',
        (tester) async {
      final p = await mount(tester);
      final layer = p.comp.getLayers().first;
      final stack = layer.getEffects();
      stack.first.setValue(
        id: 'radius',
        value: BridgeEffectValue.float(BridgeScalar.keyframed([
          for (final (frame, value) in [(0, 0.0), (40, 60.0)])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: frame),
              value: value,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ])),
      );
      layer.setEffects(effects: stack);
      p.ui.model.refresh();
      await settleFrb(tester, minRounds: 4);

      rebuilds
        ..reset()
        ..counting = true;
      for (var frame = 1; frame <= 20; frame++) {
        p.ui.playheadFrame.value = frame;
        await tester.pump(const Duration(milliseconds: 16));
      }
      rebuilds
        ..counting = false
        ..remove();

      // ignore: avoid_print
      print('KEYED SCRUB REBUILDS ${rebuilds.total}\n${rebuilds.ranking()}');
      expect(find.text('Radius'), findsWidgets,
          reason: 'the keyed row is actually on screen to be counted');
      expect(
        rebuilds.byName['KeyframeControlsFrb'] ?? 0,
        greaterThan(0),
        reason: 'the keyed row stopped showing the value under the playhead:\n'
            '${rebuilds.ranking()}',
      );
    });

    /// The second interaction the same rule is about: **picking an effect**.
    ///
    /// It is felt in two panels at once — the heading lights in Effect
    /// controls, the row lights in the Timeline — and both used to redraw
    /// themselves whole to do it. Measured at 1167 widgets for one click on a
    /// two-layer, three-effect project: 858 of it the Timeline's `setState`,
    /// 306 the Effect controls panel listening at its root, and the rest the
    /// shell-wide `notifyListeners` that fanned out to every other panel too.
    /// Nothing about a pick reaches the engine, so the whole of it was Dart
    /// redrawing things a selection cannot change.
    ///
    /// The layer whose effect is picked really does redraw — that is the next
    /// test, and it is what stops this one being passed by a panel that has
    /// gone deaf.
    Future<({dynamic ui, dynamic comp, LayerReference layer, UuidValue effect})>
        pickAnEffect(WidgetTester tester) async {
      final p = await mount(tester);
      // The layer Effect controls is showing, so the heading that lights is on
      // screen to be counted as well as the Timeline row.
      final LayerReference layer = p.ui.selectedLayer.value;
      final effect = layer.getEffects().first.id();
      rebuilds
        ..reset()
        ..counting = true;
      p.ui.setEffectSelection(layer, <UuidValue>[effect]);
      await tester.pump(const Duration(milliseconds: 16));
      rebuilds
        ..counting = false
        ..remove();
      return (ui: p.ui, comp: p.comp, layer: layer, effect: effect);
    }

    testWidgets('picking an effect redraws the rows it lights and little else',
        (tester) async {
      await pickAnEffect(tester);

      // ignore: avoid_print
      print('EFFECT SELECT REBUILDS ${rebuilds.total}\n${rebuilds.ranking()}');
      // The Timeline's chrome cannot say anything about which effect is
      // picked, so it must not redraw for one. This is the widget that catches
      // a panel-wide `setState` coming back: it is drawn once, above the rows,
      // and only a rebuild of the whole panel reaches it.
      expect(
        rebuilds.byName['ColumnHeader'] ?? 0,
        0,
        reason: 'the whole Timeline redrew for one pick:\n'
            '${rebuilds.ranking()}',
      );
      // Nor do the lane bars: a bar draws a layer's span, which a pick does
      // not move.
      expect(
        rebuilds.byName['Bar'] ?? 0,
        0,
        reason: 'the lane bars redrew for one pick:\n${rebuilds.ranking()}',
      );
      // Measured at 372, against 1169 before: one outline row, one effect
      // card, and the widgets inside those two. The cap is roughly 2x, in the
      // house style, so honest growth does not trip it; what must never come
      // back is the four-figure count of a panel-wide redraw.
      expect(
        rebuilds.total,
        lessThan(750),
        reason: 'picking an effect redrew far too much:\n'
            '${rebuilds.ranking()}',
      );
    });

    /// The other half of the rule: the rows that a pick *does* change must
    /// still change. A budget met by a panel that stopped listening would be
    /// an editor in which nothing ever lights up.
    testWidgets('the picked effect still lights its row and its heading',
        (tester) async {
      final picked = await pickAnEffect(tester);

      // The Timeline: the layer the effect belongs to is marked, and it was
      // redrawn to say so.
      final row = tester.widgetList<OutlineRow>(find.byType(OutlineRow)).where(
          (r) => r.entry.layer.internallayerId == picked.layer.internallayerId);
      expect(row, isNotEmpty, reason: 'the layer has a row on screen');
      expect(
        row.first.highlighted,
        isTrue,
        reason: "the picked effect's layer did not light up — the Timeline "
            'stopped following the effect selection',
      );
      expect(
        rebuilds.byName['OutlineRow'] ?? 0,
        greaterThan(0),
        reason: 'no outline row redrew, so nothing on screen changed:\n'
            '${rebuilds.ranking()}',
      );
      // Effect controls: exactly the headings whose answer flipped redrew —
      // the heading of the effect just picked, and no more than a couple.
      expect(
        rebuilds.byName['_WhenPicked'] ?? 0,
        greaterThan(0),
        reason: 'no effect heading redrew, so none of them lit:\n'
            '${rebuilds.ranking()}',
      );
    });

    /// The same interaction again, made **in the Timeline itself**: clicking a
    /// property's name.
    ///
    /// It is the last of the three selection paths, and the one that was left
    /// out of the first pass because it changes two things rather than one —
    /// the picked rows *and* the lane keyframes that come with them (K-500
    /// §2.1). Both halves of the table draw from it, so it stayed a `setState`
    /// on the whole panel: measured at 1144 widgets for one click on a
    /// two-layer project, the toolbar, the column headers, every bar and every
    /// row redrawn so that one row could go from unlit to lit.
    ///
    /// It is *two* redraws to be rid of, not one, and that is why it looked
    /// bigger than the outline's share: the press picks the row before the tap
    /// does (K-334, `_selectOnEdit`), so a plain click on a name went through
    /// the panel-wide `setState` twice over.
    Future<({dynamic ui, dynamic comp, String path})> pickAProperty(
        WidgetTester tester) async {
      final p = await mount(tester);
      final layer = p.comp.getLayers().first;
      final id = layer.internallayerId.toString();
      // Open the layer, then its Transform group, so a property row with a
      // name to click is on screen. Only the first layer: the second is left
      // shut, which is what makes it the layer that must *not* redraw.
      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Transform'));
      await tester.pumpAndSettle();
      // Read off the row itself rather than spelled out here, so the test says
      // "the row that was clicked" and not "the path I expect it to have".
      final path = tester
          .widget<FoldRow>(find.ancestor(
              of: find.text('Opacity'), matching: find.byType(FoldRow)))
          .path;

      rebuilds
        ..reset()
        ..counting = true;
      await tester.tap(find.text('Opacity'));
      await tester.pump(const Duration(milliseconds: 16));
      rebuilds
        ..counting = false
        ..remove();
      return (ui: p.ui, comp: p.comp, path: path);
    }

    testWidgets('clicking a property row lights it without redrawing the panel',
        (tester) async {
      await pickAProperty(tester);

      // ignore: avoid_print
      print('PROPERTY CLICK REBUILDS ${rebuilds.total}\n${rebuilds.ranking()}');
      // The chrome above the rows knows nothing about which row is picked, so
      // only a panel-wide `setState` reaches it. This is the trap for one
      // coming back.
      expect(
        rebuilds.byName['ColumnHeader'] ?? 0,
        0,
        reason: 'the whole Timeline redrew for one property click:\n'
            '${rebuilds.ranking()}',
      );
      // Nor do the lane bars: a bar draws a layer's span, which a pick does
      // not move.
      expect(
        rebuilds.byName['Bar'] ?? 0,
        0,
        reason: 'the lane bars redrew for one property click:\n'
            '${rebuilds.ranking()}',
      );
      // Measured at 426, against 1144 before: the one open layer's outline
      // block — its row and every fold row under it — and that layer's lanes.
      // The cap is roughly 2x, in the house style, so honest growth does not
      // trip it; what must never come back is the four-figure count of a
      // panel-wide redraw.
      expect(
        rebuilds.total,
        lessThan(850),
        reason: 'clicking a property redrew far too much:\n'
            '${rebuilds.ranking()}',
      );
    });

    /// The other half of the rule, again: the row that was clicked must
    /// actually come up lit, or the budget above is met by a panel that has
    /// gone deaf.
    testWidgets('the clicked property row still shows as selected',
        (tester) async {
      final picked = await pickAProperty(tester);

      final rows = tester
          .widgetList<FoldRow>(find.byType(FoldRow))
          .where((r) => r.path == picked.path);
      expect(rows, isNotEmpty, reason: 'the Opacity row is on screen');
      expect(
        rows.first.selectedProperties,
        contains(picked.path),
        reason: 'the clicked row did not draw itself selected — the outline '
            'stopped following the property selection',
      );
      expect(
        rebuilds.byName['FoldRow'] ?? 0,
        greaterThan(0),
        reason: 'no property row redrew, so nothing on screen changed:\n'
            '${rebuilds.ranking()}',
      );
      // And the shell heard it too (K-341): the Viewer outlines the layer a
      // picked row belongs to.
      expect(picked.ui.selectedProperties.value, contains(picked.path));
    });

    /// And the fourth selection path: clicking a **layer's** name.
    ///
    /// It was the one left holding the panel-wide `setState` after the other
    /// three stopped needing one, because a row and a bar took their lit state
    /// as a build-time flag — so the only way to move the light was to rebuild
    /// the panel that hands the flag down. On the owner's 64-layer comp that
    /// cost one **69 ms** build frame for a click that changes the shading of
    /// two blocks (docs/impl/ui-performance.md §3.1, WP-2). The layer ids ride
    /// in [TimelineSelection] now and each block reads its own slice.
    Future<({dynamic ui, LayerReference layer})> clickALayer(
        WidgetTester tester) async {
      final p = await mount(tester);
      // The layer that is *not* already selected: a click that changes nothing
      // would meet any budget going.
      final LayerReference layer = p.comp.getLayers().last;
      // On the name, not the row's centre — the centre lands on the blend
      // dropdown (the same reason `bridge_call_budget_test` names this cell).
      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      rebuilds
        ..reset()
        ..counting = true;
      await tester.tapAt(tester.getTopLeft(name) + const Offset(5, 8));
      await tester.pump(const Duration(milliseconds: 16));
      rebuilds
        ..counting = false
        ..remove();
      // The row's own double-click window (K-243's rename) is a 40 ms timer,
      // and the binding refuses to end a test with one pending. Off the count,
      // because a click that has already lit its row is what is being measured.
      await tester.pump(const Duration(milliseconds: 100));
      return (ui: p.ui, layer: layer);
    }

    testWidgets('clicking a layer lights it without redrawing the panel',
        (tester) async {
      await clickALayer(tester);

      // ignore: avoid_print
      print('LAYER CLICK REBUILDS ${rebuilds.total}\n${rebuilds.ranking()}');
      // None of the Timeline's chrome can say which layer is picked, so only a
      // rebuild of the whole panel reaches these. They are the trap for one
      // coming back — by `setState` here, or by a shell-wide `notifyListeners`
      // from anything the click touches on its way through.
      for (final name in [
        'ColumnHeader',
        'Toolbar',
        'Outline',
        'LayerArea',
        'TimelineRuler',
      ]) {
        expect(
          rebuilds.byName[name] ?? 0,
          0,
          reason: 'the whole Timeline redrew for one layer click ($name):\n'
              '${rebuilds.ranking()}',
        );
      }
      // Measured at 914, against 1506 before: two outline rows, two bars,
      // and — most of it — the Effect controls panel showing the layer's own
      // stack, which is the one thing a layer click genuinely gives a new
      // picture to (§4.4). The cap is roughly 2x, in the house style.
      expect(
        rebuilds.total,
        lessThan(1800),
        reason: 'clicking a layer redrew far too much:\n${rebuilds.ranking()}',
      );
    });

    /// The honest half: the row and the bar the click lights must actually
    /// redraw, or the budget above is met by an outline that has gone deaf.
    testWidgets('the clicked layer still lights its row and its bar',
        (tester) async {
      final picked = await clickALayer(tester);

      final rows = tester.widgetList<OutlineRow>(find.byType(OutlineRow)).where(
          (r) => r.entry.layer.internallayerId == picked.layer.internallayerId);
      expect(rows, isNotEmpty, reason: 'the clicked layer has a row on screen');
      expect(rows.first.selected, isTrue,
          reason: 'the clicked row did not draw itself selected — the outline '
              'stopped following the layer selection');
      // Both halves of the table draw the same answer (K-217), and the bar is
      // the half that reads it through the lanes' own blocks.
      final bars = tester.widgetList<Bar>(find.byType(Bar)).where(
          (b) => b.entry.layer.internallayerId == picked.layer.internallayerId);
      expect(bars, isNotEmpty, reason: 'the clicked layer has a bar on screen');
      expect(bars.first.selected, isTrue,
          reason: 'the bar did not outline — the lanes stopped following the '
              'layer selection');
      expect(rebuilds.byName['OutlineRow'] ?? 0, greaterThan(0),
          reason: 'no row redrew, so nothing on screen changed:\n'
              '${rebuilds.ranking()}');
      // And the shell holds it, so the Viewer and Delete act on it.
      expect(picked.ui.selectedLayer.value?.equals(layer: picked.layer), isTrue,
          reason: 'the click did not reach the shell selection');
    });

    /// The **scale** rule, and the one the three budgets above cannot state:
    /// what an interaction costs must follow the rows **on screen**, not the
    /// rows the composition has.
    ///
    /// Measured on the owner's `songcutfull` precomp, 2026-08-28: both halves
    /// of the Timeline built every layer's block in a `Column` inside a scroll
    /// view, so a select-all walked thousands of widgets, a twirl and a `U`
    /// walked them again, and a delete rebuilt 2330. Nothing above catches it,
    /// because every fixture above is small enough that "every layer" and
    /// "every layer on screen" are the same list.
    ///
    /// So the fixture here is deliberately far taller than its panel: two
    /// hundred layers in a 300px Timeline, which shows a couple of dozen rows.
    /// The claim is that the count is a function of the panel's height.
    testWidgets(
        'a select-all costs the rows on screen, not the rows in '
        'the comp', (tester) async {
      const layers = 200;
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      for (var i = 0; i < layers; i++) {
        comp.addSolidLayer();
      }
      p.uiState.setSelectedComp(comp);
      p.uiState.model.refresh();

      // Short, so the stack cannot fit: this is the whole point of the
      // fixture. Wide, so the outline's columns are not the thing being
      // measured.
      const size = Size(1600, 300);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: size,
        child: const TimelinePanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      rebuilds
        ..reset()
        ..counting = true;
      p.uiState.setSelection(comp.getLayers());
      await tester.pump(const Duration(milliseconds: 16));
      rebuilds
        ..counting = false
        ..remove();

      // ignore: avoid_print
      print('SELECT ALL REBUILDS ${rebuilds.total} over '
          '${rebuilds.byName['OutlineRow'] ?? 0} rows and '
          '${rebuilds.byName['Bar'] ?? 0} bars\n${rebuilds.ranking()}');
      // **This is the assertion.** Measured at 28 rows and 28 bars: a
      // screenful is roughly nine rows at this height and the window holds a
      // screenful either side of it, so a drag has real widgets to slide. The
      // cap is roughly 2x, in the house style. What must never come back is a
      // count that tracks the *comp* — two hundred of each, whatever the panel
      // is showing.
      for (final name in ['OutlineRow', 'Bar']) {
        expect(
          rebuilds.byName[name] ?? 0,
          lessThan(60),
          reason: 'the $name count is following the comp rather than the '
              'viewport:\n${rebuilds.ranking()}',
        );
      }
      // And the same claim as a total, since a row's own subtree is most of
      // what a rebuild costs: measured at 5033 for those 28 blocks, against
      // the ~36,000 the same click cost when every layer was built.
      expect(
        rebuilds.total,
        lessThan(10000),
        reason: 'a select-all redrew the whole comp:\n${rebuilds.ranking()}',
      );

      // And the honest half: the rows that *are* on screen were redrawn, and
      // they came up selected. A budget met by a panel that had simply stopped
      // building rows would be an outline with nothing in it.
      final rows = tester.widgetList<OutlineRow>(find.byType(OutlineRow));
      expect(rows, isNotEmpty, reason: 'the outline still has rows on screen');
      expect(rows.every((r) => r.selected), isTrue,
          reason: 'every row on screen came up selected');
      expect(rebuilds.byName['OutlineRow'] ?? 0, greaterThan(0),
          reason: 'no row redrew, so nothing on screen changed:\n'
              '${rebuilds.ranking()}');
    });

    /// The **incremental scroll** rule (K-678, docs/impl/ui-performance.md
    /// §4.3): a wheel notch that slides the window by a row or two builds the
    /// rows it brings in, and leaves the rest of the window alone.
    ///
    /// The select-all test above pins the window's *size*; this pins what a
    /// slide inside it costs. Measured before the fix: every block in the
    /// window rebuilt on each slide frame — 28 rows and 28 bars here, ~57 of
    /// each at the owner's maximised window, a ~75 ms build frame and 8.6 fps.
    testWidgets('a scroll builds the rows it brings in, not the whole window',
        (tester) async {
      const layers = 200;
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      for (var i = 0; i < layers; i++) {
        comp.addSolidLayer();
      }
      p.uiState.setSelectedComp(comp);
      p.uiState.model.refresh();

      const size = Size(1600, 300);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: size,
        child: const TimelinePanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      final vertical = tester
          .stateList<ScrollableState>(find.byType(Scrollable))
          .map((s) => s.position)
          .where((s) => s.axis == Axis.vertical && s.maxScrollExtent > 0)
          .toList();
      expect(vertical, isNotEmpty, reason: 'the stack has somewhere to scroll');

      // Into the middle of the stack first. At the very top the window is
      // pinned against the start of the content, so a short scroll slides
      // nothing and the count below would be met by a panel that had simply
      // not moved.
      vertical.first.jumpTo(1000);
      await tester.pump(const Duration(milliseconds: 16));

      rebuilds
        ..reset()
        ..counting = true;
      // A wheel notch's worth: a couple of rows.
      vertical.first.jumpTo(1050);
      await tester.pump(const Duration(milliseconds: 16));
      rebuilds
        ..counting = false
        ..remove();

      // ignore: avoid_print
      print('SCROLL SLIDE REBUILDS ${rebuilds.total} over '
          '${rebuilds.byName['OutlineRow'] ?? 0} rows and '
          '${rebuilds.byName['Bar'] ?? 0} bars\n${rebuilds.ranking()}');
      // **This is the assertion.** Two rows enter each half; the cap is roughly
      // 2x that, in the house style. What must never come back is a count that
      // tracks the *window* — every block rebuilt to show one new row.
      for (final name in ['OutlineRow', 'Bar']) {
        expect(
          rebuilds.byName[name] ?? 0,
          lessThan(8),
          reason: 'a scroll slide rebuilt the whole window of $name, not the '
              'blocks entering it:\n${rebuilds.ranking()}',
        );
      }
      // And the honest half: the rows the slide brought in were built, so the
      // budget is not met by a stack that has stopped following the scroll.
      expect(rebuilds.byName['OutlineRow'] ?? 0, greaterThan(0),
          reason: 'nothing was built, so no row entered the window:\n'
              '${rebuilds.ranking()}');
    });

    // ------------------------------------------------------------------
    // **§4.2's repaint matrix, as gates** (WP-6, docs/impl/ui-performance.md).
    //
    // The tests above pin what an interaction *builds*; the matrix pins what
    // it *re-records*, which is the half the owner's 20 fps actually lived in.
    // Every row of the table gets one, in the table's own order: idle, select,
    // scroll, zoom, playhead drag, work-area drag, edit. The playhead and
    // work-area rows already have theirs above (they were written first, for
    // K-626 and K-649); the five here are the rest.
    //
    // What cannot be tested headless is the raster thread's own milliseconds —
    // a widget test has no compositor and no window. Those stay the probe's
    // (§6, run in the owner's conditions); what is pinned here is the count
    // that causes them: how many blocks re-recorded.
    // ------------------------------------------------------------------

    /// A tall stack in a short panel — the fixture every window-sized rule
    /// needs, because in a fixture that fits, "the window" and "the comp" are
    /// the same list and no budget can tell them apart.
    Future<({dynamic state, dynamic ui, dynamic comp})> mountTall(
      WidgetTester tester, {
      int layers = 200,
      Size size = const Size(1600, 300),
    }) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      for (var i = 0; i < layers; i++) {
        comp.addSolidLayer();
      }
      p.uiState.setSelectedComp(comp);
      // One layer already in hand before the panel is mounted. An empty
      // selection is not the state a select gesture is measured from: going
      // from nothing to something is a different question (and a rarer one)
      // than moving the light from one row to the next.
      p.uiState.selectedLayer.value = comp.getLayers().first;
      p.uiState.model.refresh();

      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: size,
        child: const TimelinePanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);
      return (state: p.state, ui: p.uiState, comp: comp);
    }

    /// A band's own paint counts and those of the repaint boundaries **directly
    /// under** it — one per block (K-678, §4.3). The walk stops at the first
    /// boundary down each branch, so the answer is about blocks and not about
    /// the widgets inside them.
    ///
    /// **What the two counters mean**, because the arithmetic below depends on
    /// it and the framework's names do not say it. A boundary's *symmetric*
    /// count rises when it re-recorded during its parent's own paint. Its
    /// *asymmetric* count rises in two different situations: it re-recorded
    /// **alone** (the parent did not paint), and — the trap — the parent
    /// painted and this child's existing layer was **reused** without
    /// re-recording, which is the saving these boundaries exist for. So a
    /// child's total is not a count of re-records, and reading it as one makes
    /// every block on screen look dirty on any gesture that moves the band.
    ({int band, Map<RenderRepaintBoundary, (int, int)> blocks}) bandPaints(
        WidgetTester tester, String key) {
      final root = tester.renderObject<RenderRepaintBoundary>(
          find.byKey(ValueKey<String>(key)));
      final out = <RenderRepaintBoundary, (int, int)>{};
      void walk(RenderObject o) {
        if (o is RenderRepaintBoundary && !identical(o, root)) {
          out[o] = (o.debugSymmetricPaintCount, o.debugAsymmetricPaintCount);
          return;
        }
        o.visitChildren(walk);
      }

      walk(root);
      return (
        band: root.debugSymmetricPaintCount + root.debugAsymmetricPaintCount,
        blocks: out,
      );
    }

    /// How many blocks **re-recorded** between two samples of [bandPaints].
    ///
    /// A block that entered the window since `before` counts: it was recorded
    /// for the first time. Otherwise a rise in the symmetric count is a
    /// re-record, and a rise in the asymmetric one is a re-record only where
    /// the band itself did not paint — where it did, that rise is the reuse
    /// described above.
    ///
    /// The one thing this cannot see, and the reason §4.2's raster rows stay
    /// the probe's: while the band is painting, a block that re-recorded
    /// *alone* on some other frame of the same gesture is indistinguishable
    /// from one whose layer was reused. So on a band-moving gesture this
    /// number is a floor, and the ceiling is the probe's raster millisecond in
    /// the owner's conditions (§6).
    Set<RenderRepaintBoundary> recorded(
      ({int band, Map<RenderRepaintBoundary, (int, int)> blocks}) before,
      ({int band, Map<RenderRepaintBoundary, (int, int)> blocks}) after,
    ) {
      final bandPainted = after.band > before.band;
      return after.blocks.entries
          .where((e) {
            final was = before.blocks[e.key];
            if (was == null) return true;
            if (e.value.$1 > was.$1) return true;
            return !bandPainted && e.value.$2 > was.$2;
          })
          .map((e) => e.key)
          .toSet();
    }

    /// The same question over a gesture that takes several frames: the bands
    /// are sampled after **each** one, so the reuse-versus-re-record decision
    /// above is made against that frame's own band paint rather than against
    /// the whole flight's. Returns how many distinct blocks re-recorded at
    /// least once in each half.
    Future<({int lanes, int outline})> recordedOver(
        WidgetTester tester, int frames) async {
      final lanes = <RenderRepaintBoundary>{};
      final outline = <RenderRepaintBoundary>{};
      var lanesWere = bandPaints(tester, 'tl-lane-blocks');
      var outlineWere = bandPaints(tester, 'tl-outline-blocks');
      for (var i = 0; i < frames; i++) {
        await tester.pump(const Duration(milliseconds: 16));
        final l = bandPaints(tester, 'tl-lane-blocks');
        final o = bandPaints(tester, 'tl-outline-blocks');
        lanes.addAll(recorded(lanesWere, l));
        outline.addAll(recorded(outlineWere, o));
        lanesWere = l;
        outlineWere = o;
      }
      return (lanes: lanes.length, outline: outline.length);
    }

    /// **Idle.** The matrix's first row, and the one everything else rests on:
    /// an editor nobody is touching draws nothing. Measured true in all four
    /// of the probe's conditions (§2.3) — this is the headless guard for it,
    /// where the raster thread's silence cannot be observed but the render
    /// tree's can.
    testWidgets('idle: nothing rebuilds and no block repaints', (tester) async {
      await mountTall(tester);
      final blocks = bandPaints(tester, 'tl-lane-blocks').blocks.length;

      rebuilds
        ..reset()
        ..counting = true;
      final paints = await recordedOver(tester, 20);
      rebuilds
        ..counting = false
        ..remove();

      // ignore: avoid_print
      print('IDLE REBUILDS ${rebuilds.total} paints lanes ${paints.lanes} '
          'outline ${paints.outline} of $blocks blocks\n${rebuilds.ranking()}');
      expect(paints.lanes, 0,
          reason: 'a lane block re-recorded with nothing happening to it');
      expect(paints.outline, 0,
          reason: 'an outline block re-recorded with nothing happening to it');
      expect(rebuilds.total, 0,
          reason: 'something rebuilt itself at rest — a polling listener, or a '
              'ticker nobody stopped:\n${rebuilds.ranking()}');
    });

    /// **Select.** The matrix allows "the blocks whose slice changed"; before
    /// WP-2 the band was one boundary and a click re-recorded fifty-seven rows
    /// to move the light on one (§4.3, 9.8–15.3 ms of the click frame).
    testWidgets('select: a layer click repaints the block it lights',
        (tester) async {
      final p = await mountTall(tester);
      // A layer that is not lit already: a click that changes nothing would
      // meet any paint budget going.
      final layer = p.comp.getLayers()[3];
      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      expect(name, findsOneWidget, reason: 'the layer has a row on screen');

      final lanesBefore = bandPaints(tester, 'tl-lane-blocks');
      final outlineBefore = bandPaints(tester, 'tl-outline-blocks');
      rebuilds
        ..reset()
        ..counting = true;
      await tester.tapAt(tester.getTopLeft(name) + const Offset(5, 8));
      await tester.pump(const Duration(milliseconds: 16));
      rebuilds
        ..counting = false
        ..remove();
      final lanes =
          recorded(lanesBefore, bandPaints(tester, 'tl-lane-blocks')).length;
      final outline =
          recorded(outlineBefore, bandPaints(tester, 'tl-outline-blocks'))
              .length;
      // The row's own double-click window (K-243's rename) is a 40 ms timer,
      // and the binding refuses to end a test with one pending.
      await tester.pump(const Duration(milliseconds: 100));

      // ignore: avoid_print
      print('LAYER CLICK PAINTS lanes $lanes outline $outline of '
          '${lanesBefore.blocks.length} blocks, ${rebuilds.total} rebuilds\n'
          '${rebuilds.ranking()}');
      // One block each half changes its answer, and the cap is roughly 2x in
      // the house style — a previously-lit row unlighting is the honest second.
      // What must never come back is a number that tracks the window.
      expect(lanes, lessThan(5),
          reason: 'a click re-recorded the lane band rather than the block it '
              'lit ($lanes of ${lanesBefore.blocks.length})');
      expect(outline, lessThan(5),
          reason: 'a click re-recorded the outline band rather than the block '
              'it lit ($outline of ${outlineBefore.blocks.length})');
      // The honest half: a budget met by a band that has stopped drawing the
      // selection at all is an editor where nothing lights up.
      expect(outline, greaterThan(0),
          reason: 'no outline block re-recorded, so no row lit');
    });

    /// **Scroll.** The build half of this is pinned above (K-678); this is the
    /// paint half of the same rule — the entering block records, the rest of
    /// the window translates on its own layer.
    testWidgets('scroll: a slide repaints the blocks entering the window',
        (tester) async {
      await mountTall(tester);
      rebuilds.remove();
      final vertical = tester
          .stateList<ScrollableState>(find.byType(Scrollable))
          .map((s) => s.position)
          .where((s) => s.axis == Axis.vertical && s.maxScrollExtent > 0)
          .toList();
      expect(vertical, isNotEmpty, reason: 'the stack has somewhere to scroll');
      // Into the middle first: at the top the window is pinned against the
      // start of the content and a short scroll slides nothing.
      vertical.first.jumpTo(1000);
      await tester.pump(const Duration(milliseconds: 16));

      final lanesBefore = bandPaints(tester, 'tl-lane-blocks');
      final outlineBefore = bandPaints(tester, 'tl-outline-blocks');
      // A wheel notch's worth: a couple of rows.
      vertical.first.jumpTo(1050);
      await tester.pump(const Duration(milliseconds: 16));
      final lanes =
          recorded(lanesBefore, bandPaints(tester, 'tl-lane-blocks')).length;
      final outline =
          recorded(outlineBefore, bandPaints(tester, 'tl-outline-blocks'))
              .length;

      // ignore: avoid_print
      print('SCROLL SLIDE PAINTS lanes $lanes outline $outline of '
          '${lanesBefore.blocks.length} blocks');
      for (final (half, count, total) in [
        ('lane', lanes, lanesBefore.blocks.length),
        ('outline', outline, outlineBefore.blocks.length),
      ]) {
        expect(count, lessThan(8),
            reason: 'a slide re-recorded the whole $half window rather than '
                'the blocks entering it ($count of $total)');
      }
      expect(lanes, greaterThan(0),
          reason: 'nothing re-recorded, so no block entered the window');
    });

    /// **Zoom.** K-293's seam, as a paint count: only the lane half listens to
    /// the zoom, so the outline must not draw at all for one — and the zoom
    /// flies (`SmoothZoom`), so the flight's frames are counted with it.
    ///
    /// The gate is stated of the outline **band** as well as of its blocks,
    /// which is what makes it airtight: a band that did not paint cannot have
    /// painted a child, and with the band still, a block's asymmetric count can
    /// only have risen by re-recording alone. The lane half is left to the
    /// probe for the reason `recorded` gives — while a band is painting every
    /// frame, its blocks' reuse and their re-records are the same counter.
    testWidgets('zoom: a zoom tick redraws the lanes and never the outline',
        (tester) async {
      await mountTall(tester);
      rebuilds.remove();
      final blocks = bandPaints(tester, 'tl-outline-blocks').blocks.length;
      final bandBefore = bandPaints(tester, 'tl-outline-blocks').band;
      final barBefore = tester.getRect(find.byType(Bar).first);

      // Ctrl+wheel — the real gesture, through the real pointer-signal path.
      // On a bar, not on the band's own centre: the band is as tall as the
      // whole stack, so its centre is a couple of thousand pixels below the
      // window and the wheel would land on nothing.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(pointer.hover(barBefore.center));
      await tester.sendEventToBinding(pointer.scroll(const Offset(0, -1)));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      // The flight, frame by frame.
      final paints = await recordedOver(tester, 20);
      final bandAfter = bandPaints(tester, 'tl-outline-blocks').band;
      await tester.pumpAndSettle();

      // ignore: avoid_print
      print('ZOOM FLY PAINTS outline ${paints.outline} of $blocks blocks, '
          'band ${bandAfter - bandBefore}');
      // **This is the assertion.** A zoom moves time, and a name, a switch and
      // a parent picker are not drawn against time.
      expect(bandAfter, bandBefore,
          reason: 'the outline band painted for a zoom, which changes nothing '
              'in it');
      expect(paints.outline, 0,
          reason: 'an outline block re-recorded for a zoom '
              '(${paints.outline} of $blocks blocks)');
      // The honest half: a gate met by a panel that ignored the wheel would be
      // a Timeline that cannot zoom. The bars really did move.
      expect(tester.getRect(find.byType(Bar).first).width,
          greaterThan(barBefore.width + 1),
          reason: 'the wheel did not zoom the lanes, so nothing was measured');
    });

    /// **Edit.** The matrix's last row: one model-refresh wave, and what the
    /// edit touched. The wave itself is `bridge_call_budget_test`'s ("an edit
    /// refreshes the model once and walks no layer", WP-5); this is what it
    /// costs on this side of the seam.
    testWidgets('edit: a switch toggle redraws the row it changed',
        (tester) async {
      final p = await mountTall(tester);
      final layer = p.comp.getLayers().first;

      final lanesBefore = bandPaints(tester, 'tl-lane-blocks');
      final outlineBefore = bandPaints(tester, 'tl-outline-blocks');
      rebuilds
        ..reset()
        ..counting = true;
      // Exactly what a click on a switch cell does: the op, the committing
      // panel's own refresh, and — a turn later — the engine's own report of
      // the same change, which is the second half of the wave.
      layer.setSwitch(switch_: BridgeLayerSwitch.locked, on_: true);
      p.ui.model.refresh();
      p.state.handleChange(ScopedChange(
        project: p.state.project!,
        item: ItemReference.composition(p.comp),
        layer: layer,
        items: false,
      ));
      await tester.pump();
      await settleFrb(tester, minRounds: 4, maxRounds: 8);
      rebuilds
        ..counting = false
        ..remove();
      final lanes =
          recorded(lanesBefore, bandPaints(tester, 'tl-lane-blocks')).length;
      final outline =
          recorded(outlineBefore, bandPaints(tester, 'tl-outline-blocks'))
              .length;

      // ignore: avoid_print
      print('EDIT REBUILDS ${rebuilds.total} paints lanes $lanes outline '
          '$outline of ${lanesBefore.blocks.length} blocks\n${rebuilds.ranking()}');
      // The rows on screen rebuild once for the new model — that is the wave,
      // and it is allowed. What is not is the wave arriving twice, which is
      // what the count catches: the cap is roughly 2x one pass over the window
      // in the house style.
      for (final name in ['OutlineRow', 'Bar']) {
        expect(rebuilds.byName[name] ?? 0, lessThan(2 * lanesBefore.blocks.length),
            reason: 'an edit rebuilt every $name more than once — the '
                'follow-on is more than one wave:\n${rebuilds.ranking()}');
      }
      expect(outline, greaterThan(0),
          reason: 'nothing re-recorded, so the locked switch never showed');
      // And the shell heard it, so the budget is not met by a panel that has
      // stopped following the document.
      expect(layer.getSwitches().locked, isTrue);
    });
  }, skip: !engineAvailable);
}
