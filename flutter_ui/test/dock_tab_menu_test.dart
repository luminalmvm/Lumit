// The right-click menu on a panel's tab (K-521): Close panel, and the pop-out
// that is honestly greyed out because real operating-system windows are not
// available to us yet (K-449, docs/impl/multi-window.md).

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  /// Two panels tabbed together beside a third, so there is a tab strip to
  /// right-click and something left over after one is closed.
  DockSplit layout() => DockSplit(
        DockAxis.horizontal,
        [
          DockTabs([DockPane(Panel.project), DockPane(Panel.hierarchy)]),
          DockPane(Panel.viewer),
        ],
        [0.5, 0.5],
      );

  Widget harness(DockSplit root, {required VoidCallback onLayoutChanged}) =>
      Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(
                builder: (context) => DockWidget(
                  root: root,
                  buildPanel: (context, panel) =>
                      SizedBox(key: ValueKey<String>('pane-${panel.name}')),
                  onLayoutChanged: onLayoutChanged,
                  activePanel: ValueNotifier<Panel?>(null),
                ),
              ),
            ],
          ),
        ),
      );

  /// Right-click the tab pill whose label reads [title].
  Future<void> rightClickTab(WidgetTester tester, String title) async {
    final at = tester.getCenter(find.text(title.toUpperCase()));
    final gesture = await tester.startGesture(at, buttons: kSecondaryButton);
    await gesture.up();
    await tester.pump();
  }

  tearDown(closeLumitPopups);

  testWidgets('the tab menu closes the panel', (tester) async {
    final root = layout();
    var layoutChanges = 0;
    await tester
        .pumpWidget(harness(root, onLayoutChanged: () => layoutChanges++));

    await rightClickTab(tester, Panel.hierarchy.title);
    expect(find.text(l10n.closePanel), findsOneWidget);
    expect(find.text(l10n.popOutPanel), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('tab-menu-close')));
    await tester.pump();
    expect(panelsIn(root), isNot(contains(Panel.hierarchy)),
        reason: 'Close panel drops it from the arrangement');
    expect(panelsIn(root), contains(Panel.project));
    expect(layoutChanges, 1, reason: 'and the arrangement is persisted');
    expect(lumitPopupOpen, isFalse);
  });

  testWidgets('Pop out is listed but cannot be pressed', (tester) async {
    final root = layout();
    var layoutChanges = 0;
    await tester
        .pumpWidget(harness(root, onLayoutChanged: () => layoutChanges++));

    await rightClickTab(tester, Panel.project.title);
    // Not a MenuRow at all: it is drawn disabled, so there is nothing to press
    // and nothing that could look pressable.
    expect(find.byKey(const ValueKey('tab-menu-pop-out')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('tab-menu-pop-out')));
    await tester.pump();
    expect(panelsIn(root), contains(Panel.project),
        reason: 'a disabled row does nothing at all');
    expect(layoutChanges, 0);
  });
}
