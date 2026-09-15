// The dock's chrome and the status line under the three shapes. Studio is
// the flush default and draws no title on a bare pane; Desk gives a bare pane
// a lowercase label at the left; Lantern makes each pane a card with a title
// line, an accent dot at its corner and the words centred, and stands each
// status readout in a bubble of its own on the room.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/app_shell.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/ui_scale.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  /// The window's own ground, behind the menu and toolbar bands: the room
  /// under Lantern, where the cards stand in it, and surface0 elsewhere.
  testWidgets('the window ground is the room under Lantern', (tester) async {
    tester.view.physicalSize = const Size(1400, 800);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    for (final shape in ThemeShape.values) {
      final p = freshProject();
      p.uiState.setShape(shape);
      await tester.pumpWidget(LumitAppNew(p.state, p.uiState, welcome: false));
      await tester.pump();
      final t = p.uiState.theme;
      final ground = find
          .ancestor(
              of: find.byType(UiScaleView), matching: find.byType(ColoredBox))
          .first;
      expect(tester.widget<ColoredBox>(ground).color,
          t.tokens.roomed ? t.room : t.surface0,
          reason: 'under $shape');
      // Let the splash run out, so no timer is left behind.
      await tester.pump(const Duration(seconds: 3));
    }
  });

  /// A bare pane beside a tab group, over the status line: the two kinds of
  /// title line the dock draws, and the strip under them.
  Future<void> mountShell(WidgetTester tester, ThemeShape shape) async {
    final p = freshProject();
    tester.view.physicalSize = const Size(1400, 600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(hostPanel(
      state: p.state,
      uiState: p.uiState,
      shape: shape,
      // The rows a shape draws on, as the app pairs them.
      density: DensityTokens.forShape(shape, false),
      size: const Size(1400, 600),
      child: Column(children: [
        Expanded(
          child: DockWidget(
            root: DockSplit(
              DockAxis.horizontal,
              [
                DockPane(Panel.project),
                DockTabs([DockPane(Panel.hierarchy), DockPane(Panel.easing)]),
              ],
              [0.5, 0.5],
            ),
            buildPanel: (context, pane) =>
                SizedBox(key: ValueKey<String>('pane-${pane.panel.name}')),
            onLayoutChanged: () {},
            activePanel: ValueNotifier<PaneId?>(null),
            maximised: ValueNotifier<PaneId?>(null),
          ),
        ),
        StatusLineFrb(
          poll: () => const BridgeExportState.idle(),
          proxyPollFn: () => const BridgeProxyState.idle(),
        ),
      ]),
    ));
    await tester.pump();
  }

  /// The accent dot: a six-pixel circle, and nothing else on the chrome is one.
  final dot = find.byWidgetPredicate((w) =>
      w is Container &&
      w.decoration is BoxDecoration &&
      (w.decoration! as BoxDecoration).shape == BoxShape.circle);

  final bubble = find.byKey(const ValueKey('status-bubble'));
  final titleLine = find.byKey(const ValueKey('pane-title-line'));
  final bareTitle = find.byKey(const ValueKey('pane-title'));

  /// The fronted tab's slot on the title line, the box it fills inside that
  /// slot, and the box of the tab behind it.
  final frontSlot = find.byKey(const ValueKey<String>('dock-tab-hierarchy'));
  final frontFill =
      find.byKey(const ValueKey<String>('dock-tab-fill-hierarchy'));
  final backFill = find.byKey(const ValueKey<String>('dock-tab-fill-easing'));

  /// The painted box of a tab's fill: the keyed container's own rect takes in
  /// its margin, so the decorated box inside it is what is measured.
  Rect boxOf(WidgetTester tester, Finder fill) => tester.getRect(
      find.descendant(of: fill, matching: find.byType(DecoratedBox)).first);

  testWidgets(
      'Lantern gives every pane a title line with the dot, and puts '
      'each status readout in a bubble', (tester) async {
    await mountShell(tester, ThemeShape.lantern);

    expect(titleLine, findsNWidgets(2),
        reason: 'the bare pane and the tab group each carry one');
    expect(dot, findsNWidgets(2), reason: 'one dot per title line');
    expect(find.text(Panel.project.title.toUpperCase()), findsOneWidget,
        reason: 'the bare pane says its own name, in capitals');
    expect(find.text(Panel.hierarchy.title.toUpperCase()), findsOneWidget,
        reason: 'the tabs sit on the title line');

    expect(bubble, findsAtLeastNWidgets(3),
        reason: 'saved, the cache meters and the clock each take a bubble');
    expect(find.byKey(const ValueKey('status-saved')), findsOneWidget);
    expect(find.byKey(const ValueKey('cache-meter')), findsOneWidget);
    final strip = tester.getSize(find.byType(StatusLineFrb));
    expect(strip.height, 32);

    // The fronted tab's fill stands off its slot by the pill inset on every
    // side, its corner the stadium less the inset, with the word centred in
    // it; the tab behind keeps no fill.
    final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.lantern);
    expect(t.tokens.pillInset, 3);
    final slot = tester.getRect(frontSlot);
    final fill = boxOf(tester, frontFill);
    expect(fill.left - slot.left, closeTo(3, 0.01));
    expect(slot.right - fill.right, closeTo(3, 0.01));
    expect(fill.top - slot.top, closeTo(3, 0.01));
    expect(slot.bottom - fill.bottom, closeTo(3, 0.01));
    final d = tester.widget<Container>(frontFill).decoration! as BoxDecoration;
    expect(d.color, t.surface2);
    expect(d.borderRadius, BorderRadius.circular(ShapeTokens.stadium - 3));
    final word = tester.getRect(find.text(Panel.hierarchy.title.toUpperCase()));
    expect(word.center.dx, closeTo(fill.center.dx, 0.5));
    expect(word.center.dy, closeTo(fill.center.dy, 0.5));
    expect(
        (tester.widget<Container>(backFill).decoration! as BoxDecoration)
            .color!
            .a,
        0,
        reason: 'the tab behind has no fill');
  });

  testWidgets('Desk labels a bare pane in lowercase at the left, with no dot',
      (tester) async {
    await mountShell(tester, ThemeShape.desk);

    expect(find.text(Panel.project.title.toLowerCase()), findsOneWidget);
    expect(dot, findsNothing);
    expect(bubble, findsNothing);
    final line = tester.getRect(titleLine.first);
    final title = tester.getRect(bareTitle);
    expect(line.height, 24);
    expect(title.left - line.left, lessThan(line.width / 4),
        reason: 'the label sits at the left of its line');
    expect(tester.getSize(find.byType(StatusLineFrb)).height, 20);

    // The mockup's tab: the fronted lowercase word over a 2px accent rule at
    // the foot of the 24 line, spanning the word alone, and no fill on either
    // tab.
    final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.desk);
    final front = tester.widget<Container>(frontFill);
    expect((front.decoration! as BoxDecoration).color!.a, 0,
        reason: 'no fill under the fronted word');
    expect((front.foregroundDecoration! as BoxDecoration).border,
        Border(bottom: BorderSide(color: t.accent, width: 2)));
    expect(tester.widget<Container>(backFill).foregroundDecoration, isNull,
        reason: 'only the fronted word takes the rule');
    final tabLine = tester.getRect(titleLine.last);
    final ruleBox = boxOf(tester, frontFill);
    final word = tester.getRect(find.text(Panel.hierarchy.title.toLowerCase()));
    // The box carries the pill's 1px transparent edge, so the word sits 1
    // inside it either way.
    expect(ruleBox.left, closeTo(word.left - 1, 0.01));
    expect(ruleBox.right, closeTo(word.right + 1, 0.01));
    expect(ruleBox.bottom, closeTo(tabLine.bottom, 0.01),
        reason: 'the rule sits at the foot of the line');
    expect(word.center.dy, closeTo(tabLine.center.dy, 0.5),
        reason: 'the word is centred on the line, the rule painted over it');
  });

  testWidgets('Studio draws no title on a bare pane and no bubbles',
      (tester) async {
    await mountShell(tester, ThemeShape.studio);

    expect(bareTitle, findsNothing);
    expect(dot, findsNothing);
    expect(bubble, findsNothing);
    expect(titleLine, findsOneWidget, reason: 'the tab group keeps its strip');
    expect(tester.getSize(find.byType(StatusLineFrb)).height, 20);
  });
}
