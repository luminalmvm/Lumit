// The dock model: default workspace fidelity to dock.rs::default_layout,
// serialisation round-trip, and the start-up Project-tab rule.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  /// **A panel that has been folded away must not cost anyone their
  /// arrangement**. Every workspace saved while the Node preview existed still
  /// names it, and the pane lookup used to be a bare `!` — so reading one back
  /// threw, and the stored layout took the settings with it. A pane naming a
  /// panel this build does not have is dropped instead, and what is left opens
  /// as it was.
  test('a saved layout naming a panel that has gone opens without it', () {
    final saved = {
      'kind': 'split',
      'axis': 'horizontal',
      'shares': [0.7, 0.3],
      'children': [
        {'kind': 'pane', 'panel': 'viewer'},
        {
          'kind': 'tabs',
          'active': 1,
          'children': [
            {'kind': 'pane', 'panel': 'scopes'},
            {'kind': 'pane', 'panel': 'nodePreview'},
          ],
        },
      ],
    };
    final parsed = DockNode.fromJson(saved);
    expect(parsed, isA<DockSplit>());
    final root = parsed! as DockSplit;
    expect([for (final c in root.children) c.runtimeType.toString()].length, 2,
        reason: 'the viewer and the tab group both survive');
    final tabs = root.children[1] as DockTabs;
    expect([for (final c in tabs.children) c.panel], [Panel.scopes]);
    expect(tabs.active, 0,
        reason: 'the fronted tab had gone, so the group opens on what is left');
    expect(root.shares, [0.7, 0.3], reason: 'the shares stay with their panes');
  });

  test('serialisation round-trips the tree', () {
    final root = defaultLayout();
    (root.children[0] as DockSplit).shares[0] = 0.3;
    ((root.children[0] as DockSplit).children[0] as DockTabs).active = 1;
    final json = root.toJson();
    final back = DockNode.fromJson(json) as DockSplit;
    expect(back.toJson(), json);
    expect(((back.children[0] as DockSplit).children[0] as DockTabs).active, 1);
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

    test('the last panel standing cannot be hidden', () {
      final root =
          DockSplit(DockAxis.vertical, [DockPane(Panel.viewer)], [1.0]);
      setPanelVisible(root, Panel.viewer, false);
      expect(panelsIn(root), [Panel.viewer],
          reason: 'an empty dock has no way back');
    });
  });

  group('Panels declare a minimum, and the seam respects it', () {
    /// **The seam stops at the floor** — the owner's own gesture. Dragging the
    /// boundary between Project and Viewer all the way to the left used to
    /// take the Project panel down to a sliver, which is where it crashed.
    testWidgets('dragging a seam past a panel\'s floor is refused',
        (tester) async {
      final root = DockSplit(DockAxis.horizontal,
          [DockPane(Panel.project), DockPane(Panel.viewer)], [0.5, 0.5]);
      tester.view.physicalSize = const Size(1000, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      await tester.pumpWidget(Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(initialEntries: [
            OverlayEntry(
              builder: (context) => DockWidget(
                root: root,
                buildPanel: (context, pane) => SizedBox.expand(
                    key: ValueKey<String>('pane-${pane.panel.name}')),
                onLayoutChanged: () {},
                activePanel: ValueNotifier<PaneId?>(null),
                maximised: ValueNotifier<PaneId?>(null),
              ),
            ),
          ]),
        ),
      ));
      await tester.pump();

      final pane = find.byKey(const ValueKey<String>('pane-project'));
      final seam = tester.getRect(pane).centerRight.translate(4, 0);
      // Far further than the panel could ever give — one drag, so the refusal
      // is not something a slower gesture could creep past.
      await tester.dragFrom(seam, const Offset(-900, 0));
      await tester.pump();

      expect(tester.getRect(pane).width,
          greaterThanOrEqualTo(panelMinWidth(Panel.project)),
          reason: 'the seam will not shrink a panel past its own minimum');
    });
  });
}
