// Widget smoke tests: the boot splash runs and gives way, the shell renders
// the default workspace, the Window menu opens Settings, the Settings window
// shows its pages, and clicking a pane moves the active-panel accent edge.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/splash.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';

Future<void> pumpApp(WidgetTester tester) async {
  await tester.binding.setSurfaceSize(const Size(1280, 800));
  await tester.pumpWidget(LumitApp(workspace: Workspace()));
  // Let the boot splash play out (it is animation-driven, so settling
  // carries it to completion).
  await tester.pumpAndSettle();
}

/// Boot the app with a caller-owned workspace, so a test can read the dock
/// tree back after a drag.
Future<void> pumpWith(WidgetTester tester, Workspace ws) async {
  await tester.binding.setSurfaceSize(const Size(1280, 800));
  await tester.pumpWidget(LumitApp(workspace: ws));
  await tester.pumpAndSettle();
}

/// Whether some tab group holds both panels.
bool _inSameTabs(DockNode node, Panel a, Panel b) => switch (node) {
      DockPane() => false,
      DockTabs(:final children) =>
        {for (final c in children) c.panel}.containsAll({a, b}),
      DockSplit(:final children) =>
        children.any((c) => _inSameTabs(c, a, b)),
    };

/// The split directly holding `panel` as a bare pane, if any.
DockSplit? _parentSplitOf(DockNode node, Panel panel) {
  if (node is! DockSplit) return null;
  for (final c in node.children) {
    if (c is DockPane && c.panel == panel) return node;
  }
  for (final c in node.children) {
    final found = _parentSplitOf(c, panel);
    if (found != null) return found;
  }
  return null;
}

void main() {
  testWidgets('the boot splash lists its lines, then gives way', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    await tester.pumpWidget(LumitApp(workspace: Workspace()));
    await tester.pump();
    expect(find.text('Lumit'), findsOneWidget);

    // Half-way through, some boot lines are up.
    await tester.pump(const Duration(milliseconds: 500));
    expect(find.text(bootLines.first), findsOneWidget);

    await tester.pumpAndSettle();
    expect(find.text('Lumit'), findsNothing, reason: 'the splash gives way');
  });

  testWidgets('the splash absorbs clicks instead of letting them through',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    await tester.pumpWidget(LumitApp(workspace: Workspace()));
    await tester.pump();
    // A click during boot must reach nothing — not the splash (no skip),
    // not the app behind it (owner feedback, 2026-07-21).
    await tester.tap(find.text('Lumit'), warnIfMissed: false);
    await tester.pump();
    expect(find.text('Lumit'), findsOneWidget, reason: 'no click-to-skip');
    await tester.pumpAndSettle();
    expect(find.text('Lumit'), findsNothing);
  });

  testWidgets('the shell renders the menu bar, tabs and status line',
      (tester) async {
    await pumpApp(tester);

    for (final label in ['File', 'Edit', 'Composition', 'Window']) {
      expect(find.text(label), findsOneWidget);
    }
    // The left tab group's pills ('Project' also titles the fronted panel's
    // placeholder body, so it can match more than once).
    expect(find.text('Project'), findsWidgets);
    expect(find.text('Effect controls'), findsOneWidget);
    expect(find.text('Effects & presets'), findsOneWidget);
    expect(find.text('Hierarchy'), findsOneWidget);
    expect(find.text('Flutter frontend — phase F0'), findsOneWidget);
  });

  testWidgets('Window → Settings… opens the Settings window on General',
      (tester) async {
    await pumpApp(tester);

    await tester.tap(find.text('Window'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Settings…'));
    await tester.pumpAndSettle();

    // Opens on General (owner request 2026-07-21; the egui window opened on
    // Appearance — a recorded deviation).
    expect(find.text('Settings'), findsOneWidget);
    expect(find.text('Autosave'), findsOneWidget);
    await tester.tap(find.text('Appearance').first);
    await tester.pumpAndSettle();
    expect(find.text('Colour scheme'), findsOneWidget);
    expect(find.text('Panel shape'), findsOneWidget);

    // Switch to the Performance page.
    await tester.tap(find.text('Performance').first);
    await tester.pumpAndSettle();
    expect(find.text('Memory budget'), findsOneWidget);

    // Done closes it.
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    expect(find.text('Memory budget'), findsNothing);
  });

  testWidgets('tab pills switch the left group', (tester) async {
    await pumpApp(tester);

    // The pill strip scrolls; bring the last pill into view before tapping.
    await tester.ensureVisible(find.text('Hierarchy'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Hierarchy'));
    await tester.pumpAndSettle();
    expect(
      find.text('The composition tree arrives in phase F4.'),
      findsOneWidget,
    );
  });

  testWidgets("a bare pane's grip drags it to stack onto another pane",
      (tester) async {
    final ws = Workspace();
    await pumpWith(tester, ws);

    // The Viewer stub fills its pane on the neutral surround, so its rect is
    // the pane rect (sharp mode: no padding). The grip sits at the top-right.
    final surround = ws.theme.viewerSurround;
    final viewerRect = tester.getRect(
      find
          .byWidgetPredicate((w) => w is Container && w.color == surround)
          .first,
    );
    final gripCentre = viewerRect.topRight + const Offset(-8, 8);
    final timelineCentre = tester.getCenter(
      find.text('Layer rows, lanes and the graph lens arrive in phase F3.'),
    );

    expect(_inSameTabs(ws.dock, Panel.viewer, Panel.timeline), isFalse);

    // No pump between down and move: the pane's press claims the active-panel
    // accent, and rebuilding for it mid-gesture would drop the pointer.
    final gesture = await tester.startGesture(gripCentre);
    await gesture.moveTo(timelineCentre);
    await gesture.up();
    await tester.pumpAndSettle();

    expect(_inSameTabs(ws.dock, Panel.viewer, Panel.timeline), isTrue,
        reason: 'the Viewer stacked onto the Timeline into one tab group');
  });

  testWidgets("a tab pill drags to split off a pane's left edge",
      (tester) async {
    final ws = Workspace();
    await pumpWith(tester, ws);

    final surround = ws.theme.viewerSurround;
    final viewerRect = tester.getRect(
      find
          .byWidgetPredicate((w) => w is Container && w.color == surround)
          .first,
    );
    // A point just inside the Viewer's left edge resolves to a left split.
    final leftEdge = viewerRect.centerLeft + const Offset(6, 0);

    await tester.ensureVisible(find.text('Hierarchy'));
    await tester.pumpAndSettle();
    final pillCentre = tester.getCenter(find.text('Hierarchy'));

    final gesture = await tester.startGesture(pillCentre);
    await gesture.moveTo(leftEdge);
    await gesture.up();
    await tester.pumpAndSettle();

    // Hierarchy left the tab group and now sits as a bare pane just left of
    // the Viewer in a horizontal split.
    expect(_inSameTabs(ws.dock, Panel.hierarchy, Panel.project), isFalse);
    final split = _parentSplitOf(ws.dock, Panel.hierarchy);
    expect(split, isNotNull);
    expect(split!.axis, DockAxis.horizontal);
    final idxH = split.children.indexWhere(
        (c) => c is DockPane && c.panel == Panel.hierarchy);
    final idxV = split.children.indexWhere(
        (c) => c is DockPane && c.panel == Panel.viewer);
    expect(idxV, idxH + 1, reason: 'Hierarchy sits immediately left of Viewer');
  });

  testWidgets('clicking a pane gives it the accent boundary', (tester) async {
    await pumpApp(tester);

    // No panel is active until something takes a click.
    Iterable<Container> accentEdged() =>
        tester.widgetList<Container>(find.byType(Container)).where((c) {
      final fg = c.foregroundDecoration;
      return fg is BoxDecoration && fg.border != null;
    });
    expect(accentEdged(), isEmpty);

    // Click inside the Viewer pane.
    await tester.tap(
      find.text(
          'The composited frame arrives with the shared-texture path (phase F2)'),
      warnIfMissed: false,
    );
    await tester.pump();
    expect(accentEdged().length, 1,
        reason: 'exactly one pane wears the accent edge after a click');
  });

  testWidgets('the accent picker opens, picks a colour and applies it on OK',
      (tester) async {
    final ws = Workspace();
    await pumpWith(tester, ws);
    expect(ws.accentOverride, isNull);

    await tester.tap(find.text('Window'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Settings…'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Appearance').first);
    await tester.pumpAndSettle();

    // Open the picker from the accent swatch.
    await tester.tap(find.byKey(const Key('accent-swatch')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('colour-picker-square')), findsOneWidget);

    // Type an exact hex, then commit with OK.
    await tester.enterText(find.byType(EditableText), '3366CC');
    await tester.pump();
    await tester.tap(find.text('OK'));
    await tester.pumpAndSettle();

    expect(ws.accentOverride, isNotNull);
    expect((ws.accentOverride!.r * 255).round(), 0x33);
    expect((ws.accentOverride!.g * 255).round(), 0x66);
    expect((ws.accentOverride!.b * 255).round(), 0xcc);
    // The live theme's accent tracks the override exactly (withAccent).
    expect(ws.theme.accent, ws.accentOverride);
  });

  testWidgets('tapping the picker square changes the pick before OK',
      (tester) async {
    final ws = Workspace();
    await pumpWith(tester, ws);

    await tester.tap(find.text('Window'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Settings…'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Appearance').first);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('accent-swatch')));
    await tester.pumpAndSettle();

    // Move the hue, then tap the middle of the square: a definite, non-grey
    // colour that differs from the seeded accent.
    await tester.tap(find.byKey(const Key('colour-picker-strip')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('colour-picker-square')));
    await tester.pump();
    await tester.tap(find.text('OK'));
    await tester.pumpAndSettle();

    expect(ws.accentOverride, isNotNull);
    expect(ws.theme.accent, ws.accentOverride);
  });

  testWidgets('dismissing the accent picker leaves the accent untouched',
      (tester) async {
    final ws = Workspace();
    await pumpWith(tester, ws);
    expect(ws.accentOverride, isNull);

    await tester.tap(find.text('Window'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Settings…'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Appearance').first);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('accent-swatch')));
    await tester.pumpAndSettle();

    // Change the pick, then cancel — nothing should apply.
    await tester.enterText(find.byType(EditableText), '3366CC');
    await tester.pump();
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(ws.accentOverride, isNull);
    expect(find.byKey(const Key('colour-picker-square')), findsNothing);
  });

  testWidgets('the settings dropdowns open and apply their pick',
      (tester) async {
    // Regression (owner report, 2026-07-21): opening a BareDropdown inside
    // the Settings window forced an infinite width, and every later click
    // failed with "Cannot hit test a render box with no size".
    final ws = Workspace();
    await pumpWith(tester, ws);

    await tester.tap(find.text('Window'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Settings…'));
    await tester.pumpAndSettle();

    // The window opens on General (owner request); go to Appearance.
    expect(find.text('Autosave'), findsOneWidget);
    await tester.tap(find.text('Appearance').first);
    await tester.pumpAndSettle();

    // Colour scheme: open the dropdown, pick Gruvbox dark, watch it apply.
    await tester.tap(find.text('Dark').first);
    await tester.pumpAndSettle();
    expect(find.text('Gruvbox dark'), findsOneWidget);
    await tester.tap(find.text('Gruvbox dark'));
    await tester.pumpAndSettle();
    expect(ws.colorScheme, LumitColorScheme.gruvboxDark);

    // Panel shape: same path, different dropdown.
    await tester.tap(find.text('Sharp'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Round'));
    await tester.pumpAndSettle();
    expect(ws.themeShape, ThemeShape.round);
  });
}
