// `Ctrl+A` is the focused panel's, not the composition's (K-522).
//
// `edit.select.all` used to mean "every layer" wherever it was pressed, so in
// the Project panel it selected things that were not on screen. The shell now
// routes the chord to whichever panel is focused — the same arrangement
// `Ctrl+F` uses for the search boxes — and falls back to every layer only when
// no panel claims it.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/drag_payloads.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Select all (frb)', () {
    /// The routing itself, with no panel mounted: which panels claim the chord
    /// and which leave it to mean "every layer".
    test('only the panels that keep a selection claim the chord', () {
      final p = freshProject();
      for (final panel in [Panel.project, Panel.effectControls]) {
        p.uiState.activePanel.value = panel;
        expect(p.uiState.requestSelectAll(), isTrue,
            reason: '${panel.name} answers Ctrl+A itself');
      }
      // The Node graph is not here yet: a single-node selection has nothing to
      // select all of, so the chord must not be swallowed there.
      for (final panel in [Panel.timeline, Panel.viewer, Panel.graph, null]) {
        p.uiState.activePanel.value = panel;
        expect(p.uiState.requestSelectAll(), isFalse,
            reason: 'the shell still means every layer in ${panel?.name}');
      }
    });

    testWidgets('the Project panel takes every row it is showing',
        (tester) async {
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

      p.uiState.activePanel.value = Panel.project;
      expect(p.uiState.requestSelectAll(), isTrue);
      await tester.pump();

      expect(_selectionOn(tester, 'a.mov'), 3,
          reason: 'every listed row is picked');
    });

    /// And only the rows it is *showing*: a search narrows the list, and select
    /// all means what is in front of you rather than what is filed away.
    testWidgets('a filtered Project panel takes only what is listed',
        (tester) async {
      final p = freshProject();
      for (final name in ['alpha.mov', 'beta.mov', 'alpine.mov']) {
        p.state.project!.importFootage(path: 'C:/clips/$name');
      }
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      await tester.enterText(find.byType(EditableText).first, 'alp');
      await tester.pump();

      p.uiState.activePanel.value = Panel.project;
      expect(p.uiState.requestSelectAll(), isTrue);
      await tester.pump();

      expect(_selectionOn(tester, 'alpha.mov'), 2,
          reason: 'alpha and alpine, not beta');
    });

    testWidgets('the Effect controls panel takes the whole stack',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      layer.addEffect(name: 'vignette');
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;

      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      p.uiState.activePanel.value = Panel.effectControls;
      expect(p.uiState.requestSelectAll(), isTrue);
      await tester.pump();

      expect(p.uiState.selectedEffects.value, hasLength(2),
          reason: 'both effects on the layer, not just one');
      expect(p.uiState.selectedEffectsLayer, layer);
    });
  }, skip: !engineAvailable);
}

/// How many items a drag started on the row reading [label] would carry — the
/// panel keeps its selection to itself and this is what it publishes, so it is
/// also how the existing selection tests read the set.
int _selectionOn(WidgetTester tester, String label) => tester
    .widget<Draggable<FootageDragData>>(
      find.ancestor(
        of: find.text(label),
        matching: find.byType(Draggable<FootageDragData>),
      ),
    )
    .data!
    .footage
    .length;
