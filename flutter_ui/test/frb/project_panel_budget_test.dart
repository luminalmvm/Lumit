// What a click and a scroll cost in the Project panel, on a project big enough
// for the cost to show.
//
// Two things were wrong and both are pinned here. The panel handed its whole
// open tree to a `ListView` as a list of already-built children, so showing
// twenty-eight rows made a widget for every item in the project; and a click
// was a `setState` on the panel, so lighting one row walked the whole tree
// again and redrew every row on screen. Measured at 982 widgets for one click
// on a 240-item project, growing with the project.

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

/// Counts widget rebuilds by name, from the framework's own log. The
/// rebuild-budget test's counter, at the size this file needs.
class _Rebuilds {
  final Map<String, int> byName = {};
  bool counting = false;
  DebugPrintCallback? _previous;

  void install() {
    _previous = debugPrint;
    debugPrint = (String? message, {int? wrapWidth}) {
      if (!counting || message == null) return;
      var line = message;
      final tail = line.lastIndexOf('): ');
      if (tail >= 0) line = line.substring(tail + 3);
      line = line.replaceFirst(RegExp(r'^(Building|Rebuilding)\s+'), '');
      final name = line.trim().split(RegExp(r'[\s(<{-]')).first;
      byName[name] = (byName[name] ?? 0) + 1;
    };
    debugPrintRebuildDirtyWidgets = true;
  }

  /// Both globals back where they were. `flutter_test` fails the test if a
  /// foundation debug variable is left set.
  void remove() {
    debugPrintRebuildDirtyWidgets = false;
    if (_previous != null) debugPrint = _previous!;
  }

  int get total => byName.values.fold(0, (a, b) => a + b);
  void reset() => byName.clear();

  String ranking() {
    final entries = byName.entries.toList()
      ..sort((a, b) => b.value.compareTo(a.value));
    return entries.take(12).map((e) => '${e.value}x ${e.key}').join('\n');
  }
}

/// How many built rows are drawn lit.
int _lit(WidgetTester tester) => find
    .byType(ProjectRowFrb)
    .evaluate()
    .where((e) => (e.widget as ProjectRowFrb).selected)
    .length;

void main() {
  setUpAll(initEngineForTests);

  group('Project panel budget', () {
    late _Rebuilds rebuilds;

    setUp(() => rebuilds = _Rebuilds()..install());
    tearDown(() => rebuilds.remove());

    /// A project of [items] compositions, at a size that shows about thirty
    /// rows. The tree is then an order of magnitude longer than the viewport,
    /// which is the state every one of these numbers is about.
    Future<({LumitState state, LumitUiState uiState, List<String> ids})> mount(
        WidgetTester tester,
        {int items = 240}) async {
      final p = freshProject();
      final ids = [
        for (var i = 0; i < items; i++)
          p.state.project!
              .newComposition(name: 'Scene ${i.toString().padLeft(3, '0')}')
              .internalid
              .toString(),
      ];
      tester.view.physicalSize = const Size(520, 760);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(520, 760),
      ));
      await settleFrb(tester);
      return (state: p.state, uiState: p.uiState, ids: ids);
    }

    /// **A click lights the row it landed on and the row it took the light
    /// from.** It is the panel's most-repeated gesture and it used to be its
    /// dearest: the selection now travels to the rows on a notifier, so the
    /// tree is not walked and the rows that did not change are not redrawn.
    testWidgets('a click lights the row and nothing else', (tester) async {
      await mount(tester);
      final target = find.text('Scene 003');
      expect(target, findsOneWidget);

      rebuilds
        ..reset()
        ..counting = true;
      await tester.tap(target);
      // The double-tap window waited out, so the row's own recogniser is not
      // left holding a timer.
      await tester.pump(kDoubleTapTimeout + const Duration(milliseconds: 50));
      rebuilds
        ..counting = false
        ..remove();

      // ignore: avoid_print
      print('CLICK REBUILDS ${rebuilds.total}\n${rebuilds.ranking()}');
      // The guard, without which a panel that had stopped answering clicks
      // altogether would pass: the row must be shown to have lit, and the
      // preview card above the tree must be describing it.
      expect(_lit(tester), 1, reason: 'the click did not light a row');
      expect(find.text('Scene 003'), findsNWidgets(2),
          reason: 'the row and the preview card, which names what is picked');
      // Measured at 24: the row that lit, the preview card, and the notifier
      // seam between them. It was 982. The cap leaves room for an honest new
      // badge or column; what must never come back is a count in the hundreds,
      // which is the panel redrawing whole.
      expect(rebuilds.total, lessThan(100),
          reason: 'a click redrew far too much:\n${rebuilds.ranking()}');
    });

    /// **A scroll costs the rows it brings in**, not the rows the project has.
    testWidgets('a scroll builds the rows it brings in', (tester) async {
      await mount(tester);

      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(
          pointer.hover(tester.getCenter(find.byType(ListView))));
      await tester.pump();

      rebuilds
        ..reset()
        ..counting = true;
      for (var i = 0; i < 10; i++) {
        await tester.sendEventToBinding(pointer.scroll(const Offset(0, 60)));
        // Real frames with time on the clock, as in the other budgets.
        await tester.pump(const Duration(milliseconds: 16));
      }
      rebuilds
        ..counting = false
        ..remove();

      // ignore: avoid_print
      print('SCROLL REBUILDS ${rebuilds.total}\n${rebuilds.ranking()}');
      // The guard: a list that had stopped scrolling would redraw nothing at
      // all and satisfy every cap below it.
      expect(find.text('Scene 000'), findsNothing,
          reason: 'the list did not scroll, so nothing was measured');
      expect(find.text('Scene 030'), findsOneWidget,
          reason: 'the rows the scroll brought in must be on screen');
      // 600px of travel is about twenty-seven rows, and 46 rows were built for
      // it: the ones that came in, twice over for the two frames a notch
      // spans. The number a regression would show is 240, the whole project.
      expect(rebuilds.byName['ProjectRowFrb'] ?? 0, lessThan(80),
          reason:
              'a scroll built rows it never showed:\n${rebuilds.ranking()}');
      // Measured at 950 in all, which is those rows and their contents.
      expect(rebuilds.total, lessThan(1600),
          reason: 'a scroll redrew far too much:\n${rebuilds.ranking()}');
    });

    /// **The tree is lazy, and the panel still knows the whole of it.** Those
    /// two pull against each other: `Shift`-click's range and `Ctrl+A` both
    /// mean "every row the filters leave", so the walk must visit rows the
    /// list will never build.
    testWidgets('only the viewport is built, and Ctrl+A still takes the tree',
        (tester) async {
      final p = await mount(tester);
      rebuilds.remove();

      final built = find.byType(ProjectRowFrb).evaluate().length;
      // ignore: avoid_print
      print('ROWS BUILT $built of ${p.ids.length}');
      expect(
        tester.widget<ListView>(find.byType(ListView)).childrenDelegate,
        isA<SliverChildBuilderDelegate>(),
        reason: 'a list of built children means the panel made a widget for '
            'every item in the project to show a viewport of them',
      );
      expect(built, lessThan(60),
          reason: 'the tree must build the viewport, not the project');

      p.uiState.activePane.value = Panel.project.pane();
      expect(p.uiState.requestSelectAll(), isTrue);
      await tester.pump();

      // The guard on the laziness: the walk still reached the bottom of the
      // tree, three hundred rows below the viewport. The anchor a select-all
      // leaves is the last row the panel lists.
      expect(
        p.uiState.selectedProjectItem.value,
        isNotNull,
        reason: 'select all published no anchor',
      );
      expect(
        projectItemId(p.uiState.selectedProjectItem.value!),
        p.ids.last,
        reason: 'Ctrl+A stopped at the viewport instead of taking the tree',
      );
      expect(_lit(tester), built, reason: 'every row on screen must be lit');
    });
  });
}
