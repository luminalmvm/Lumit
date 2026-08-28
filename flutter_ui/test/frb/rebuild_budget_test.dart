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
  }, skip: !engineAvailable);
}
