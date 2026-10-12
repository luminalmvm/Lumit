// A group drawn as a stack: panels one above another, each under a header
// that twirls it open or shut, the open ones sharing the height.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  /// A Viewer beside a stack of three, the first open.
  DockSplit layout({bool solo = false}) => DockSplit(
        DockAxis.horizontal,
        [
          DockPane(Panel.viewer),
          DockTabs(
            [
              DockPane(Panel.effectsAndPresets),
              DockPane(Panel.text),
              DockPane(Panel.paragraph),
            ],
            stacked: true,
            solo: solo,
            open: {Panel.effectsAndPresets.pane()},
          ),
        ],
        [0.6, 0.4],
      );

  DockTabs stackOf(DockSplit root) => root.children[1] as DockTabs;

  group('The model', () {
    test('a stack round-trips what is open and how tall', () {
      final root = layout();
      final stack = stackOf(root)
        ..setOpen(Panel.paragraph.pane(), true)
        ..shares[Panel.paragraph.pane()] = 0.4;
      final json = root.toJson();
      final back = stackOf(DockNode.fromJson(json) as DockSplit);
      expect(back.stacked, isTrue);
      expect(back.open, stack.open);
      expect(back.shareOf(Panel.paragraph.pane()), 0.4);
      expect(back.shareOf(Panel.text.pane()), 1);
      expect((DockNode.fromJson(json) as DockSplit).toJson(), json);
    });

    test('a plain tab group writes no stack keys', () {
      final json = DockTabs([DockPane(Panel.project), DockPane(Panel.scopes)])
          .toJson();
      expect(json.keys, ['kind', 'active', 'children']);
    });

    test('a panel dropped from a saved stack leaves the rest as they were',
        () {
      final json = stackOf(layout()..children).toJson()
        ..['open'] = [0, 2]
        ..['heights'] = [1.0, 2.0, 3.0];
      // The middle panel is one this build no longer has.
      (json['children'] as List)[1] = {'kind': 'pane', 'panel': 'gone'};
      final back = DockNode.fromJson(json) as DockTabs;
      expect([for (final c in back.children) c.panel],
          [Panel.effectsAndPresets, Panel.paragraph]);
      expect(back.open,
          {Panel.effectsAndPresets.pane(), Panel.paragraph.pane()});
      expect(back.shareOf(Panel.paragraph.pane()), 3);
    });

    test('solo shuts the others when one opens', () {
      final stack = stackOf(layout(solo: true));
      stack.setOpen(Panel.text.pane(), true);
      expect(stack.open, {Panel.text.pane()});
      stack.setOpen(Panel.text.pane(), false);
      expect(stack.open, isEmpty);
    });

    test('fronting a panel in a stack twirls it open', () {
      final root = layout();
      expect(activatePanelTab(root, Panel.text), isTrue);
      expect(stackOf(root).isOpen(Panel.text.pane()), isTrue);
      expect(activatePanelTab(root, Panel.text), isFalse,
          reason: 'already open and in front, so nothing moved');
    });

    test('a panel shown from the Window menu joins the stack, open', () {
      final root = layout();
      setPanelVisible(root, Panel.scopes, true);
      final stack = stackOf(root);
      expect(stack.children.last.panel, Panel.scopes);
      expect(stack.isOpen(Panel.scopes.pane()), isTrue);
    });

    test('a panel dropped on a stack lands under the panel it was dropped on',
        () {
      final root = layout();
      movePanel(root, Panel.viewer.pane(), Panel.effectsAndPresets.pane(),
          DropPosition.stack);
      // The Viewer left its own tile, so the stack is the whole root now.
      final stack = root.children.single as DockTabs;
      expect([
        for (final c in stack.children) c.panel
      ], [
        Panel.effectsAndPresets,
        Panel.viewer,
        Panel.text,
        Panel.paragraph,
      ]);
      expect(stack.isOpen(Panel.viewer.pane()), isTrue);
    });

    test('reordering inside a stack keeps what was open', () {
      final root = layout();
      movePanel(root, Panel.effectsAndPresets.pane(), Panel.paragraph.pane(),
          DropPosition.stack);
      final stack = stackOf(root);
      expect([for (final c in stack.children) c.panel],
          [Panel.text, Panel.paragraph, Panel.effectsAndPresets]);
      expect(stack.isOpen(Panel.effectsAndPresets.pane()), isTrue);
    });

    test('the Edit arrangement keeps Text and Paragraph in its stack', () {
      final upper = defaultLayout().children[0] as DockSplit;
      final right = upper.children[2] as DockTabs;
      expect(right.stacked, isTrue);
      expect(right.open, {Panel.effectsAndPresets.pane()});
      expect([for (final c in right.children) c.panel],
          containsAll([Panel.text, Panel.paragraph]));
    });
  });

  group('The widget', () {
    Widget harness(DockSplit root, {VoidCallback? onLayoutChanged}) =>
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
                    buildPanel: (context, pane) =>
                        _Counter(key: ValueKey<String>('pane-${pane.panel.name}')),
                    onLayoutChanged: onLayoutChanged ?? () {},
                    activePanel: ValueNotifier<PaneId?>(null),
                    maximised: ValueNotifier<PaneId?>(null),
                  ),
                ),
              ],
            ),
          ),
        );

    Finder header(Panel panel) =>
        find.byKey(ValueKey<String>('dock-stack-${panel.name}'));
    Finder body(Panel panel) =>
        find.byKey(ValueKey<String>('pane-${panel.name}'));

    tearDown(closeLumitPopups);

    testWidgets('every panel has a header, and only the open one a body',
        (tester) async {
      await tester.pumpWidget(harness(layout()));
      for (final panel in [
        Panel.effectsAndPresets,
        Panel.text,
        Panel.paragraph
      ]) {
        expect(header(panel), findsOneWidget);
      }
      expect(body(Panel.effectsAndPresets), findsOneWidget);
      expect(body(Panel.text), findsNothing,
          reason: 'a panel never opened is not built');
      expect(find.byKey(const ValueKey('dock-tab-text')), findsNothing,
          reason: 'a stack draws no tab strip');
    });

    testWidgets('a click on a header twirls its panel open and shut',
        (tester) async {
      final root = layout();
      var saved = 0;
      await tester.pumpWidget(harness(root, onLayoutChanged: () => saved++));
      await tester.tap(header(Panel.text));
      await tester.pump();
      expect(stackOf(root).isOpen(Panel.text.pane()), isTrue);
      expect(body(Panel.text), findsOneWidget);
      expect(saved, 1, reason: 'what is open is part of the arrangement');
      // Both open: they share the height the headers leave.
      final first = tester.getSize(body(Panel.effectsAndPresets)).height;
      final second = tester.getSize(body(Panel.text)).height;
      expect(first, closeTo(second, 1));

      await tester.tap(header(Panel.text));
      await tester.pump();
      expect(stackOf(root).isOpen(Panel.text.pane()), isFalse);
      expect(tester.getSize(body(Panel.effectsAndPresets)).height,
          greaterThan(first));
    });

    testWidgets('a panel keeps its state through being shut', (tester) async {
      await tester.pumpWidget(harness(layout()));
      await tester.tap(body(Panel.effectsAndPresets));
      await tester.pump();
      expect(find.text('1'), findsOneWidget);
      await tester.tap(header(Panel.effectsAndPresets));
      await tester.pump();
      expect(find.text('1'), findsNothing);
      await tester.tap(header(Panel.effectsAndPresets));
      await tester.pump();
      expect(find.text('1'), findsOneWidget,
          reason: 'the same State, not a fresh one');
    });

    testWidgets('the seam between two open panels moves their heights',
        (tester) async {
      final root = layout();
      stackOf(root).setOpen(Panel.text.pane(), true);
      await tester.pumpWidget(harness(root));
      final before = tester.getSize(body(Panel.effectsAndPresets)).height;
      final seam = tester.getBottomLeft(body(Panel.effectsAndPresets)) +
          const Offset(40, 3);
      await tester.dragFrom(seam, const Offset(0, 60));
      await tester.pump();
      final after = tester.getSize(body(Panel.effectsAndPresets)).height;
      expect(after, greaterThan(before + 30));
      expect(stackOf(root).shareOf(Panel.effectsAndPresets.pane()),
          greaterThan(stackOf(root).shareOf(Panel.text.pane())));
    });

    testWidgets('the header menu turns a stack back into tabs',
        (tester) async {
      final root = layout();
      await tester.pumpWidget(harness(root));
      final gesture = await tester.startGesture(
          tester.getCenter(header(Panel.text)),
          buttons: kSecondaryButton);
      await gesture.up();
      await tester.pump();
      expect(find.byKey(const ValueKey('tab-menu-solo')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('tab-menu-stacked')));
      await tester.pump();
      expect(stackOf(root).stacked, isFalse);
      expect(find.byKey(const ValueKey('dock-tab-text')), findsOneWidget);
      expect(header(Panel.text), findsNothing);
    });

    testWidgets('dragging a header onto another reorders the stack',
        (tester) async {
      final root = layout();
      await tester.pumpWidget(harness(root));
      final from = tester.getCenter(header(Panel.effectsAndPresets));
      final to = tester.getCenter(header(Panel.paragraph));
      final gesture = await tester.startGesture(from);
      await gesture.moveTo(from + const Offset(0, 12));
      await gesture.moveTo(to);
      await tester.pump();
      await gesture.up();
      await tester.pump();
      expect([for (final c in stackOf(root).children) c.panel],
          [Panel.text, Panel.paragraph, Panel.effectsAndPresets]);
    });
  });
}

/// A body with State to lose: a count of the taps it has had.
class _Counter extends StatefulWidget {
  const _Counter({super.key});

  @override
  State<_Counter> createState() => _CounterState();
}

class _CounterState extends State<_Counter> {
  int _taps = 0;

  @override
  Widget build(BuildContext context) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => setState(() => _taps++),
        child: SizedBox.expand(child: Text('$_taps')),
      );
}
