// The re-dock model ops (dock.dart movePanel/simplify): stacking, same-axis
// and cross-axis splits, unwrapping, joining, the every-panel-once invariant,
// self-drop no-ops, and the root staying a DockSplit.

import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/dock.dart';

/// A simple three-pane layout: a horizontal split of Viewer, Timeline, Scopes.
DockSplit threeAcross() => DockSplit(
      DockAxis.horizontal,
      [
        DockPane(Panel.viewer),
        DockPane(Panel.timeline),
        DockPane(Panel.scopes),
      ],
      [0.4, 0.3, 0.3],
    );

void checkInvariants(DockSplit root) {
  final panels = panelsIn(root);
  expect(panels.toSet().length, panels.length,
      reason: 'every panel appears at most once');

  void walk(DockNode node) {
    switch (node) {
      case DockPane():
        break;
      case DockTabs():
        expect(node.children, isNotEmpty);
        expect(node.active, inInclusiveRange(0, node.children.length - 1));
        // A tab group never has a single child — it would be unwrapped.
        expect(node.children.length, greaterThanOrEqualTo(2));
      case DockSplit():
        expect(node.children.length, node.shares.length,
            reason: 'shares match children');
        for (final s in node.shares) {
          expect(s, greaterThan(0), reason: 'all shares positive');
        }
        for (final c in node.children) {
          walk(c);
        }
    }
  }

  walk(root);
}

void main() {
  test('stack onto a solo pane makes a two-tab group, dragged active', () {
    final root = threeAcross();
    movePanel(root, Panel.scopes.pane(), Panel.viewer.pane(), DropPosition.stack);

    final tabs = root.children[0] as DockTabs;
    expect(
        [for (final c in tabs.children) c.panel], [Panel.viewer, Panel.scopes]);
    expect(tabs.active, 1, reason: 'the dragged tab is fronted');
    // Scopes left its old slot; the split is now Viewer-group + Timeline.
    expect(root.children.length, 2);
    checkInvariants(root);
  });

  test('same-axis split inserts adjacent with halved shares', () {
    // Scopes sits outside the horizontal split, so dragging it in leaves the
    // target's neighbours untouched and the halving is exact.
    final root = DockSplit(
      DockAxis.vertical,
      [
        DockSplit(
          DockAxis.horizontal,
          [DockPane(Panel.viewer), DockPane(Panel.timeline)],
          [0.6, 0.4],
        ),
        DockPane(Panel.scopes),
      ],
      [0.75, 0.25],
    );
    // Timeline holds 0.4; splitting Scopes off its right halves it.
    movePanel(root, Panel.scopes.pane(), Panel.timeline.pane(), DropPosition.right);

    // The vertical root collapses to a single horizontal child.
    expect(root.children.length, 1);
    final row = root.children.single as DockSplit;
    expect(row.axis, DockAxis.horizontal);
    expect([for (final c in row.children) (c as DockPane).panel],
        [Panel.viewer, Panel.timeline, Panel.scopes]);
    expect(row.shares[0], closeTo(0.6, 1e-9));
    expect(row.shares[1], closeTo(0.2, 1e-9));
    expect(row.shares[2], closeTo(0.2, 1e-9));
    checkInvariants(root);
  });

  /// Dragging panels about never loses or duplicates one.
  ///
  /// Measured against the arrangement's *own* inventory rather than
  /// `Panel.values`: the default layout no longer holds every panel (Easing is
  /// Retiming's), and what this test is actually about is that a move
  /// conserves whatever was there. Seeded with the Easing panel present, so the
  /// newest panel is one of the ones being thrown around.
  test('every panel appears once across a randomised sequence of moves', () {
    final root = defaultLayout();
    setPanelVisible(root, Panel.easing, true);
    final inventory = panelsIn(root).toSet();
    final rng = Random(20260721);
    const positions = DropPosition.values;

    for (var i = 0; i < 50; i++) {
      final panels = panesIn(root);
      final dragged = panels[rng.nextInt(panels.length)];
      var target = panels[rng.nextInt(panels.length)];
      if (dragged == target) continue;
      final pos = positions[rng.nextInt(positions.length)];
      movePanel(root, dragged, target, pos);

      checkInvariants(root);
      expect(panelsIn(root).toSet(), inventory,
          reason: 'no panel is lost or duplicated by move $i');
    }
  });
}
