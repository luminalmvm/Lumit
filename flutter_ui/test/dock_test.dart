// The dock model: default workspace fidelity to dock.rs::default_layout,
// serialisation round-trip, and the start-up Project-tab rule.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/dock.dart';

void main() {
  test('default layout matches default_layout() structure and shares', () {
    final root = defaultLayout();
    expect(root.axis, DockAxis.vertical);
    expect(root.shares, [0.68, 0.32]);
    expect(root.children.length, 2);

    final upper = root.children[0] as DockSplit;
    expect(upper.axis, DockAxis.horizontal);
    expect(upper.shares, [0.22, 0.58, 0.20]);

    final left = upper.children[0] as DockTabs;
    expect(
      [for (final c in left.children) c.panel],
      [
        Panel.project,
        Panel.effectControls,
        Panel.hierarchy,
      ],
    );
    expect(left.active, 0, reason: 'the left group opens on Project');

    expect((upper.children[1] as DockPane).panel, Panel.viewer);
    // The right column carries Effects & presets fronted (docs/07 §1.6's
    // Edit workspace), with Scopes and Debug tabbed behind it.
    final right = upper.children[2] as DockTabs;
    expect(
      [for (final c in right.children) c.panel],
      [Panel.effectsAndPresets, Panel.scopes, Panel.debug],
    );
    expect(right.active, 0,
        reason: 'the right group opens on Effects & presets');
    expect((root.children[1] as DockPane).panel, Panel.timeline);
  });

  /// Every panel appears at most once, and all but three appear.
  ///
  /// This used to read "every panel, exactly once", and the exceptions are
  /// named one by one on purpose: a fourth wanting the same exemption must be
  /// added here rather than this loosening to "some panels are missing".
  ///
  /// Three panels are deliberately not in the default arrangement, and all for
  /// the same reason (docs/07 §1.6): a panel nobody asked for should not
  /// appear in an arrangement they already know. **Easing** belongs to
  /// Retiming (K-349); the **Graph** and **Node** panels to Nodes (K-445,
  /// K-471). All three are one tick away in the Window menu.
  test(
      'no panel appears twice in the default workspace, and only Easing, '
      'Graph and Node are absent', () {
    final panels = panelsIn(defaultLayout());
    expect(panels.toSet().length, panels.length);
    expect(
        panels.toSet(),
        Panel.values.toSet()
          ..removeAll([Panel.easing, Panel.graph, Panel.node]));
  });

  test('serialisation round-trips the tree', () {
    final root = defaultLayout();
    (root.children[0] as DockSplit).shares[0] = 0.3;
    ((root.children[0] as DockSplit).children[0] as DockTabs).active = 2;
    final json = root.toJson();
    final back = DockNode.fromJson(json) as DockSplit;
    expect(back.toJson(), json);
    expect(((back.children[0] as DockSplit).children[0] as DockTabs).active, 2);
  });

  test('activatePanelTab fronts the tab that holds the panel', () {
    final root = defaultLayout();
    final left = (root.children[0] as DockSplit).children[0] as DockTabs;
    left.active = 2;
    activatePanelTab(root, Panel.project);
    expect(left.active, 0);
    // A panel not in any tab group is a no-op.
    activatePanelTab(root, Panel.viewer);
    expect(left.active, 0);
  });

  test('panel titles are the glossary names', () {
    expect(Panel.project.title, 'Project');
    expect(Panel.effectControls.title, 'Effect controls');
    expect(Panel.effectsAndPresets.title, 'Effects & presets');
  });

  group('panel visibility (the Window menu tick list)', () {
    test('hiding drops the panel and showing puts it back, fronted', () {
      final root = defaultLayout();
      expect(panelVisible(root, Panel.scopes), isTrue);

      setPanelVisible(root, Panel.scopes, false);
      expect(panelVisible(root, Panel.scopes), isFalse);
      expect(panelsIn(root).toSet().length, panelsIn(root).length,
          reason: 'no panel appears twice after a removal');

      setPanelVisible(root, Panel.scopes, true);
      expect(panelVisible(root, Panel.scopes), isTrue);
      // It went into a tab group, fronted — a panel you just asked for is the
      // one you want to look at.
      final tabs = panelsIn(root);
      expect(tabs.where((p) => p == Panel.scopes), hasLength(1));
    });

    test('asking for what is already so changes nothing', () {
      final root = defaultLayout();
      final before = root.toJson();
      setPanelVisible(root, Panel.viewer, true);
      setPanelVisible(root, Panel.debug, true);
      expect(root.toJson(), before);
    });

    test('the last panel standing cannot be hidden', () {
      final root =
          DockSplit(DockAxis.vertical, [DockPane(Panel.viewer)], [1.0]);
      setPanelVisible(root, Panel.viewer, false);
      expect(panelsIn(root), [Panel.viewer],
          reason: 'an empty dock has no way back');
    });

    test('a tree of bare panes still finds somewhere to put one', () {
      final root = DockSplit(
        DockAxis.vertical,
        [DockPane(Panel.viewer), DockPane(Panel.timeline)],
        [0.5, 0.5],
      );
      setPanelVisible(root, Panel.scopes, true);
      expect(panelVisible(root, Panel.scopes), isTrue);
      expect(root.shares.length, root.children.length);
    });
  });

  /// The Easing panel is Retiming's alone (K-349): a new panel that appeared in
  /// the four arrangements people already know would be a rearrangement nobody
  /// asked for. Anywhere else it is opened deliberately.
  group('the Retiming preset', () {
    test('gives the Easing panel the right-hand column, untabbed', () {
      final root = presetLayout(WorkspacePreset.retiming);
      final upper = root.children[0] as DockSplit;
      final right = upper.children.last;
      expect(right, isA<DockPane>());
      expect((right as DockPane).panel, Panel.easing,
          reason: 'a panel behind a tab is a panel you have to keep fetching');
      expect(root.shares, [0.55, 0.45],
          reason: 'retiming is timeline work, so the Timeline is as tall as '
              "Audio's");
      expect(upper.shares.length, upper.children.length);
    });

    test('is the only shipped arrangement holding it', () {
      for (final preset in WorkspacePreset.values) {
        expect(
          panelVisible(presetLayout(preset), Panel.easing),
          preset == WorkspacePreset.retiming,
          reason: '${preset.name} should '
              '${preset == WorkspacePreset.retiming ? '' : 'not '}hold Easing',
        );
      }
      expect(panelVisible(defaultLayout(), Panel.easing), isFalse,
          reason: 'first run is unchanged by the panel existing');
    });

    test('every panel is still reachable from the Window menu', () {
      // The menu ticks `Panel.values`, so a panel in no arrangement must still
      // be one `setPanelVisible` can place.
      final root = defaultLayout();
      setPanelVisible(root, Panel.easing, true);
      expect(panelVisible(root, Panel.easing), isTrue);
    });
  });

  /// The Nodes preset (K-445, K-471) to the approved Nodes-workspace drawing:
  /// the Graph panel large with a short Timeline under it, a small Viewer
  /// upper right and the Node panel beneath that. The shares are the drawing's
  /// own proportions, pinned here because "roughly like the picture" is not a
  /// test.
  group('the Nodes preset', () {
    test('splits across, not down: the Timeline is under the graph only', () {
      final root = presetLayout(WorkspacePreset.nodes);
      expect(root.axis, DockAxis.horizontal,
          reason: 'the small viewer keeps its full height beside the graph '
              'column, so the Timeline cannot span the window');
      expect(root.shares, [0.76, 0.24]);

      final graphColumn = root.children[0] as DockSplit;
      expect(graphColumn.axis, DockAxis.vertical);
      expect([for (final c in graphColumn.children) (c as DockPane).panel],
          [Panel.graph, Panel.timeline]);
      expect(graphColumn.shares, [0.82, 0.18],
          reason: 'the drawing shows a short Timeline strip, not a tall one');

      final rightColumn = root.children[1] as DockSplit;
      expect(rightColumn.axis, DockAxis.vertical);
      expect([for (final c in rightColumn.children) (c as DockPane).panel],
          [Panel.viewer, Panel.node]);
      expect(rightColumn.shares, [0.80, 0.20]);
    });

    test('the Timeline is the ordinary one, simply given less room', () {
      // A pane, not a tab group and not some second widget: the panel is
      // shared with every other arrangement, and only its share differs.
      final nodes = presetLayout(WorkspacePreset.nodes);
      final short = (nodes.children[0] as DockSplit).children[1];
      expect(short, isA<DockPane>());
      expect((short as DockPane).panel, Panel.timeline);
      expect(
        (nodes.children[0] as DockSplit).shares[1],
        lessThan(presetLayout(WorkspacePreset.edit).shares[1]),
        reason: "shorter than Edit's, which is what 'short' means here",
      );
    });

    test('is the only shipped arrangement holding the Graph and Node panels',
        () {
      for (final preset in WorkspacePreset.values) {
        for (final panel in [Panel.graph, Panel.node]) {
          expect(
            panelVisible(presetLayout(preset), panel),
            preset == WorkspacePreset.nodes,
            reason: '${preset.name} should '
                '${preset == WorkspacePreset.nodes ? '' : 'not '}'
                'hold ${panel.name}',
          );
        }
      }
      expect(panelVisible(defaultLayout(), Panel.node), isFalse,
          reason: 'first run is unchanged by the panel existing');
    });

    test('sits third on the strip, beside Effects', () {
      // The strip is generated from the enum's order, and the drawing puts
      // Nodes between Effects and Colour.
      expect(WorkspacePreset.values.map((p) => p.name), [
        'edit',
        'effects',
        'nodes',
        'colour',
        'audio',
        'retiming',
      ]);
    });

    test('the Node panel is still reachable from the Window menu', () {
      // The menu ticks `Panel.values`, so a panel in no arrangement must still
      // be one `setPanelVisible` can place.
      final root = defaultLayout();
      setPanelVisible(root, Panel.node, true);
      expect(panelVisible(root, Panel.node), isTrue);
    });
  });
}
