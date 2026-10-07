// Tab in the shell, against the real engine: it opens the flowchart for the
// fronted composition.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  final chart = find.byKey(const ValueKey('flowchart'));

  /// Film places Shot and Shot places Plate, with Shot fronted.
  ({
    LumitState state,
    LumitUiState uiState,
    CompositionReference film,
    CompositionReference shot,
    CompositionReference plate,
  }) nested(WidgetTester tester) {
    tester.view.physicalSize = const Size(1800, 1100);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    final p = freshProject();
    final project = p.state.project!;
    final film = project.newComposition(name: 'Film');
    final shot = project.newComposition(name: 'Shot');
    final plate = project.newComposition(name: 'Plate');
    film.addPrecompLayer(comp: shot);
    shot.addPrecompLayer(comp: plate);
    p.uiState.setSelectedComp(shot);
    return (
      state: p.state,
      uiState: p.uiState,
      film: film,
      shot: shot,
      plate: plate,
    );
  }

  Future<void> mountShell(
      WidgetTester tester, LumitState state, LumitUiState uiState) async {
    await tester.pumpWidget(hostPanel(
      child: const LumitAppView(),
      state: state,
      uiState: uiState,
    ));
    await tester.pump();
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pump();
    await tester.pump();
  }

  group('The flowchart (frb)', () {
    testWidgets(
        'Tab draws the fronted comp and its neighbours, and Enter '
        'fronts the one chosen', (tester) async {
      final p = nested(tester);
      await mountShell(tester, p.state, p.uiState);

      await press(tester, LogicalKeyboardKey.tab);
      expect(chart, findsOneWidget);
      for (final name in ['Film', 'Shot', 'Plate']) {
        expect(find.descendant(of: chart, matching: find.text(name)),
            findsOneWidget);
      }

      await press(tester, LogicalKeyboardKey.arrowLeft);
      await press(tester, LogicalKeyboardKey.enter);
      expect(chart, findsNothing);
      expect(p.uiState.selectedComp, p.film);

      // From Film the way back is to the right.
      await press(tester, LogicalKeyboardKey.tab);
      await press(tester, LogicalKeyboardKey.arrowRight);
      await press(tester, LogicalKeyboardKey.enter);
      expect(p.uiState.selectedComp, p.shot);
    });

    testWidgets('Tab again shuts it and opens nothing', (tester) async {
      final p = nested(tester);
      await mountShell(tester, p.state, p.uiState);
      await press(tester, LogicalKeyboardKey.tab);
      expect(chart, findsOneWidget);
      await press(tester, LogicalKeyboardKey.tab);
      expect(chart, findsNothing);
      expect(p.uiState.selectedComp, p.shot);
    });

    testWidgets('with no composition fronted Tab draws nothing',
        (tester) async {
      tester.view.physicalSize = const Size(1800, 1100);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      await mountShell(tester, p.state, p.uiState);
      await press(tester, LogicalKeyboardKey.tab);
      expect(chart, findsNothing);
    });
  });
}
