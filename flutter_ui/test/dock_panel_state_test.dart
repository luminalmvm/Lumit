// Panel state must survive the active flip and tab switches. Three regressions
// for the "state resets when a panel flips active/inactive" defect (desk-test
// round 5): the pane chrome kept a constant composed tree shape across the
// active flip (a null-vs-non-null foregroundDecoration used to add or remove a
// DecoratedBox layer, so Flutter discarded the pane's Element subtree and its
// State — scroll offsets and half-armed gesture recognisers with it); and a
// tab group now keeps every tab's body alive offstage rather than building only
// the active one.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// A dock tab, by the panel's name. Tab labels are kickers (docs/15-DESIGN.md
/// §7.1), so what is on screen is the capitalised form of the sentence-case
/// string — the transform is the style's, not the arb file's.
Finder _tab(String title) => find.text(title.toUpperCase());

/// A scrollable panel body whose State construction is counted, so a test can
/// prove the State object survived (was not rebuilt from scratch). Reads/writes
/// its scroll through a controller supplied from outside, so the offset lives
/// where the test can inspect it.
int _scrollBodyBuilds = 0;

/// Taps that reached a bare pane's own body, for the corner-grip test below.
int taps = 0;

class _ScrollBody extends StatefulWidget {
  final ScrollController controller;
  const _ScrollBody(this.controller);

  @override
  State<_ScrollBody> createState() => _ScrollBodyState();
}

class _ScrollBodyState extends State<_ScrollBody> {
  _ScrollBodyState() {
    _scrollBodyBuilds++;
  }

  @override
  Widget build(BuildContext context) {
    return ListView(
      controller: widget.controller,
      children: [
        for (var i = 0; i < 60; i++)
          SizedBox(height: 30, child: Text('row $i')),
      ],
    );
  }
}

/// A drag target that counts horizontal-drag updates, to prove a drag begun
/// while the panel was inactive still takes effect on that same first gesture.
class _DragCounter extends StatefulWidget {
  final void Function() onDrag;
  const _DragCounter(this.onDrag);

  @override
  State<_DragCounter> createState() => _DragCounterState();
}

class _DragCounterState extends State<_DragCounter> {
  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onHorizontalDragUpdate: (_) => widget.onDrag(),
      child: const SizedBox.expand(child: Text('drag me')),
    );
  }
}

Widget _harness({
  required DockSplit root,
  required PanelBuilder buildPanel,
  required ValueNotifier<Panel?> active,
}) {
  return Directionality(
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
              buildPanel: buildPanel,
              onLayoutChanged: () {},
              activePanel: active,
            ),
          ),
        ],
      ),
    ),
  );
}

void main() {
  testWidgets('flipping a bare pane inactive keeps its State and scroll offset',
      (tester) async {
    final controller = ScrollController(keepScrollOffset: false);
    addTearDown(controller.dispose);
    final active = ValueNotifier<Panel?>(Panel.project);
    addTearDown(active.dispose);

    await tester.pumpWidget(_harness(
      root: DockSplit(
        DockAxis.horizontal,
        [DockPane(Panel.project), DockPane(Panel.viewer)],
        [0.5, 0.5],
      ),
      buildPanel: (context, panel) => panel == Panel.project
          ? _ScrollBody(controller)
          : const Text('pane B'),
      active: active,
    ));
    await tester.pump();

    // Scroll pane A and snapshot its State object while it is the active pane.
    controller.jumpTo(120);
    await tester.pump();
    final stateBefore = tester.state(find.byType(_ScrollBody));
    final buildsBefore = _scrollBodyBuilds;
    expect(controller.offset, 120);

    // Click pane B, making it active and pane A inactive — the flip that used
    // to discard pane A's subtree.
    await tester.tap(find.text('pane B'));
    await tester.pump();

    expect(active.value, Panel.viewer);
    expect(
        identical(tester.state(find.byType(_ScrollBody)), stateBefore), isTrue,
        reason: 'pane A kept the same State object across the flip');
    expect(_scrollBodyBuilds, buildsBefore,
        reason: 'pane A body was not reconstructed');
    expect(controller.offset, 120, reason: 'the scroll offset survived');
  });

  testWidgets('a drag begun on an inactive pane works on the first gesture',
      (tester) async {
    var drags = 0;
    final active =
        ValueNotifier<Panel?>(Panel.viewer); // pane A starts inactive
    addTearDown(active.dispose);

    await tester.pumpWidget(_harness(
      root: DockSplit(
        DockAxis.horizontal,
        [DockPane(Panel.project), DockPane(Panel.viewer)],
        [0.5, 0.5],
      ),
      buildPanel: (context, panel) => panel == Panel.project
          ? _DragCounter(() => drags++)
          : const Text('pane B'),
      active: active,
    ));
    await tester.pump();
    expect(active.value, Panel.viewer, reason: 'pane A is inactive to start');

    // One unbroken gesture: press (which activates pane A), move, release.
    final gesture =
        await tester.startGesture(tester.getCenter(find.byType(_DragCounter)));
    await tester.pump();
    await gesture.moveBy(const Offset(40, 0));
    await tester.pump();
    await gesture.moveBy(const Offset(40, 0));
    await tester.pump();
    await gesture.up();
    await tester.pump();

    expect(active.value, Panel.project, reason: 'the press activated pane A');
    expect(drags, greaterThan(0),
        reason: 'the drag took effect on the first gesture');
  });

  /// Airyzz's rule (663b6cc, restored after a merge overwrote it): invisible
  /// panels are not built — not at all before first shown, and not again
  /// while hidden, however often the dock itself rebuilds.
  testWidgets(
      'a hidden tab is never built until shown, nor rebuilt while hidden',
      (tester) async {
    final builds = <Panel, int>{};
    final active = ValueNotifier<Panel?>(null);
    addTearDown(active.dispose);

    await tester.pumpWidget(_harness(
      root: DockSplit(
        DockAxis.horizontal,
        [
          DockTabs([DockPane(Panel.project), DockPane(Panel.hierarchy)],
              active: 0),
        ],
        [1.0],
      ),
      buildPanel: (context, panel) {
        builds[panel] = (builds[panel] ?? 0) + 1;
        return Text('body of ${panel.title}');
      },
      active: active,
    ));
    await tester.pump();

    expect(builds[Panel.project], isNotNull, reason: 'the visible tab built');
    expect(builds[Panel.hierarchy], isNull,
        reason: 'a tab never shown builds nothing at all');

    // A dock-level rebuild for an unrelated reason must not reach it either.
    active.value = Panel.project;
    await tester.pump();
    expect(builds[Panel.hierarchy], isNull,
        reason: 'dock rebuilds never cascade into hidden tabs');

    // Showing it builds it; hiding it again freezes it at that count.
    await tester.tap(_tab('Hierarchy'));
    await tester.pump();
    expect(builds[Panel.hierarchy], isNotNull);
    final shown = builds[Panel.hierarchy]!;
    await tester.tap(_tab('Project'));
    await tester.pump();
    active.value = null;
    await tester.pump();
    expect(builds[Panel.hierarchy], shown,
        reason: 'hidden again, it is not rebuilt however the dock stirs');
  });

  testWidgets('a tab group preserves a hidden tab\'s scroll offset',
      (tester) async {
    final controller = ScrollController(keepScrollOffset: false);
    addTearDown(controller.dispose);
    final active = ValueNotifier<Panel?>(null);
    addTearDown(active.dispose);

    await tester.pumpWidget(_harness(
      root: DockSplit(
        DockAxis.horizontal,
        [
          DockTabs([DockPane(Panel.project), DockPane(Panel.hierarchy)],
              active: 0),
        ],
        [1.0],
      ),
      buildPanel: (context, panel) => panel == Panel.project
          ? _ScrollBody(controller)
          : const Text('body of Hierarchy'),
      active: active,
    ));
    await tester.pump();

    // Scroll tab 1's body, then switch to tab 2 and back.
    controller.jumpTo(90);
    await tester.pump();
    expect(controller.offset, 90);

    await tester.tap(_tab('Hierarchy')); // the second tab's pill
    await tester.pump();
    expect(find.text('body of Hierarchy'), findsOneWidget);

    await tester.tap(_tab('Project')); // back to the first tab's pill
    await tester.pump();

    expect(controller.offset, 90,
        reason: 'the hidden tab kept its scroll offset alive');
  });

  /// **A bare pane draws no corner grip** (owner review, 2026-08-24).
  ///
  /// A solo pane used to wear a 16px square of dots at its top-right, which
  /// dragged the panel. It is gone: nothing is painted over the panel's own
  /// top-right corner, and nothing there takes a pointer. What still moves a
  /// panel is a tab pill, and Window → Workspace.
  testWidgets('a bare pane paints nothing over its top-right corner',
      (tester) async {
    final active = ValueNotifier<Panel?>(Panel.viewer);
    addTearDown(active.dispose);

    await tester.pumpWidget(_harness(
      root: DockSplit(
        DockAxis.horizontal,
        [DockPane(Panel.viewer)],
        [1.0],
      ),
      buildPanel: (context, panel) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => taps++,
        child: const SizedBox.expand(child: Text('pane body')),
      ),
      active: active,
    ));
    await tester.pump();

    // The pane's own top-right corner, a few pixels in — where the grip stood.
    final pane = tester.getRect(find.text('pane body'));
    await tester.tapAt(Offset(pane.right - 8, pane.top + 8));
    await tester.pump();
    expect(taps, 1,
        reason: 'the corner belongs to the panel, not to a dock affordance');

    // And the grip's own tooltip is nowhere in the tree.
    expect(find.byType(LumitTooltip), findsNothing);
  });
}
