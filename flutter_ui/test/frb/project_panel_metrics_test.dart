// The Project panel measured against the approved mockup, band by band.
//
// **Why this file exists.** `project_panel_frb_test` is about what the panel
// *does* — what a click selects, what a rename commits, what a drag carries.
// This one is about what it *looks like*, and specifically about the numbers
// the mockups' own computed styles resolved to: every row
// height, every column width, every face, every colour token. Nothing here
// names a private widget class, because none of these claims is about how the
// panel is built — each is something a person could point at on screen and
// measure with a ruler.
//
// A value that disagrees with the mockup is a defect, so each expectation
// carries the mockup's own number in its reason.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Project panel metrics (frb)', () {
    /// A project with a comp (filed under its auto-folder) and one clip.
    ({LumitState state, LumitUiState uiState, String compId}) withItems() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      return (
        state: p.state,
        uiState: p.uiState,
        compId: comp.internalid.toString()
      );
    }

    Future<void> mount(WidgetTester tester, dynamic p,
        {double width = 480,
        DensityTokens density = DensityTokens.regular}) async {
      tester.view.physicalSize = Size(width, 760);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: Size(width, 760),
        density: density,
      ));
      await tester.pump();
    }

    /// 4b. **The seams between the headings drag**, on the Timeline outline's
    /// own rule: a seam widens the column to its left and every other column
    /// keeps its width, so the drag moves one boundary and nothing else.
    testWidgets('dragging a column seam moves that column and no other',
        (tester) async {
      final p = withItems();
      await mount(tester, p, width: 560);
      await settleFrb(tester, minRounds: 6);

      final header =
          find.byKey(const ValueKey<String>('project-column-header'));
      double heading(String word) => tester
          .getRect(find.descendant(of: header, matching: find.text(word)))
          .width;

      // The seam right of Name, the one drawn before the Items column.
      final seam = find.byKey(const ValueKey<String>('project-seam-name'));
      expect(seam, findsOneWidget);
      await tester.drag(seam, const Offset(40, 0));
      await tester.pump();

      expect(heading('Name'), projectNameColumn + 40,
          reason: 'the seam widened the column it follows');
      expect(heading('Size'), projectSizeColumn,
          reason: 'and left the columns between alone');
      // 560 less the arrangement every column asks for at its starting
      // width (512) is 48 of slack, and all of it was Path's.
      expect(heading('Path'), projectPathColumn + 48 - 40,
          reason: 'Path gave up exactly what Name took, since it holds the '
              'panel\'s slack');

      // Back past its minimum: the column stops there rather than vanishing.
      await tester.drag(seam, const Offset(-400, 0));
      await tester.pump();
      expect(heading('Name'), minProjectColumnWidth(ProjectColumn.name),
          reason: 'a column stops at what its cells need');
    });
  });
}
