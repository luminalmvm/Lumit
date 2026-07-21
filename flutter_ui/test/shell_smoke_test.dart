// Widget smoke tests: the boot splash runs and gives way, the shell renders
// the default workspace, the Window menu opens Settings, the Settings window
// shows its pages, and clicking a pane moves the active-panel accent edge.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/splash.dart';
import 'package:lumit_flutter/state/workspace.dart';

Future<void> pumpApp(WidgetTester tester) async {
  await tester.binding.setSurfaceSize(const Size(1280, 800));
  await tester.pumpWidget(LumitApp(workspace: Workspace()));
  // Let the boot splash play out (it is animation-driven, so settling
  // carries it to completion).
  await tester.pumpAndSettle();
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

  testWidgets('a click skips the boot splash', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    await tester.pumpWidget(LumitApp(workspace: Workspace()));
    await tester.pump();
    await tester.tap(find.text('Lumit'));
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

  testWidgets('Window → Settings… opens the Settings window on Appearance',
      (tester) async {
    await pumpApp(tester);

    await tester.tap(find.text('Window'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Settings…'));
    await tester.pumpAndSettle();

    expect(find.text('Settings'), findsOneWidget);
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
}
