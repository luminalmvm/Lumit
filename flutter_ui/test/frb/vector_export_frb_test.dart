// The Lottie and SVG window on the real engine: it opens on a composition,
// the check lists what the file leaves out with a place for each, the footer
// says how big the file is and the player plays it; and a place, clicked,
// selects its layer behind the window.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/vector_export_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Lottie and SVG window (frb)', () {
    /// A comp with one solid wearing a Glow, which no Lottie carries, and the
    /// window open on it with its first export back.
    Future<({LumitState state, LumitUiState uiState, LayerReference solid})>
        open(WidgetTester tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final solid = comp.addSolidLayer()..addEffect(name: 'glow');
      p.uiState.setSelectedComp(comp);
      await tester.pumpWidget(hostPanel(
        size: const Size(1200, 800),
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey<String>('open'),
            onPressed: () => showVectorExportFrb(context),
            child: const Text('open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.tap(find.byKey(const ValueKey<String>('open')));
      await tester.pump();
      expect(find.text(l10n.vectorTitle), findsOneWidget);
      // The first export of a project stands up its worker, so this waits as
      // long as a first frame does.
      await settleFrb(tester,
          maxRounds: coldWorkerRounds,
          until: () =>
              find.byKey(const ValueKey<String>('vector-left-out')).evaluate().isNotEmpty);
      return (state: p.state, uiState: p.uiState, solid: solid);
    }

    testWidgets('the window shows the check, the size and plays the file',
        (tester) async {
      await open(tester);

      expect(find.text(l10n.vectorOutcomeLeftOut), findsOneWidget);
      expect(find.text(l10n.vectorReasonEffect('Glow')), findsOneWidget,
          reason: 'the glow is named as left out, in the reader\'s words');
      final summary = tester.widget<Text>(
          find.byKey(const ValueKey<String>('vector-summary')));
      expect(summary.data, contains(l10n.unitKb),
          reason: 'the footer says how big the file is');

      num frame() => tester
          .widget<DragValueField>(
              find.byKey(const ValueKey<String>('vector-frame')))
          .value;
      expect(frame(), 0);
      await tester.tap(find.byKey(const ValueKey<String>('vector-play')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));
      expect(frame(), isNot(0),
          reason: 'play moves the frame, which is the player\'s own clock');
    });

    testWidgets('a place selects its layer', (tester) async {
      final p = await open(tester);
      expect(p.uiState.selectedLayer.value, isNull);

      final place = find.byWidgetPredicate(
          (w) => w is Text && (w.data ?? '').startsWith('Scene › '));
      expect(place, findsWidgets);
      await tester.tap(place.first);
      await tester.pump();

      expect(p.uiState.selectedLayer.value?.internallayerId,
          p.solid.internallayerId);
      expect(find.text(l10n.vectorTitle), findsOneWidget,
          reason: 'the window stays up: it is modeless');
    });
  }, skip: !engineAvailable);
}
