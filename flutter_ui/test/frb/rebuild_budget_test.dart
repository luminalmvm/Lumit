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
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
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
  }, skip: !engineAvailable);
}
