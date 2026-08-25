// Which tab the dock fronts when the document moves under it (items 6.28,
// 6.35).
//
// The rule is small and easy to state: what you just asked for should be the
// panel you are looking at. Selecting a layer is asking for its controls;
// opening a project — or closing one, which leaves an empty project behind —
// is asking for what is in it. Neither takes the *keyboard* away from the
// panel you are working in: fronting a tab shows something, it does not claim
// the keys, which is why `activePanel` is asserted to stand still here.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Fronting a panel (frb)', () {
    /// The tab group holding [panel], or null when it sits alone.
    DockTabs? groupOf(DockNode node, Panel panel) {
      switch (node) {
        case DockPane():
          return null;
        case DockTabs(:final children):
          return children.any((c) => c.panel == panel) ? node : null;
        case DockSplit(:final children):
          for (final child in children) {
            final found = groupOf(child, panel);
            if (found != null) return found;
          }
          return null;
      }
    }

    testWidgets('selecting a layer fronts the Effect controls tab',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      p.uiState.setSelectedComp(comp);

      final group = groupOf(p.uiState.split, Panel.effectControls)!;
      expect(group.activePane.panel, Panel.project,
          reason: 'the default arrangement opens on the Project panel');

      p.uiState.setSelection([comp.getLayers().single]);
      expect(group.activePane.panel, Panel.effectControls);
      expect(p.uiState.activePanel.value, isNull,
          reason: 'the tab is fronted; the keyboard is not moved with it');
    });

    testWidgets('a project arriving fronts the Project panel', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      p.uiState.setSelectedComp(comp);
      p.uiState.setSelection([comp.getLayers().single]);

      final group = groupOf(p.uiState.split, Panel.project)!;
      expect(group.activePane.panel, Panel.effectControls);

      // Closing a project is opening the empty one that replaces it, which is
      // the same adoption and the same rule.
      p.state.newProject();
      expect(group.activePane.panel, Panel.project);
    });
  }, skip: !engineAvailable);
}
