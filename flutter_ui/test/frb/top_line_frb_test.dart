// The top line as the shell mounts it, under each shape and both toolbar
// positions.
//
// Under Left the strip above the dock is not mounted at all: the rail adds
// its column, and the menu bar's line takes the tool options and the
// workspace strip, so nothing still takes the strip's row. Under Top the
// shell is what it was, the strip under a 26 line for Studio.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/state/dock.dart' show WorkspacePreset;
import 'package:lumit_flutter/shell/tool_bar_frb.dart';
import 'package:lumit_flutter/state/settings.dart' show ToolBarPosition;
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('The top line (frb)', () {
    /// The whole shell view, at a real window size, dressed in [shape] with
    /// the density that shape draws at.
    Future<({LumitState state, LumitUiState uiState})> mount(
      WidgetTester tester, {
      required ThemeShape shape,
      required ToolBarPosition position,
      Size size = const Size(1800, 1100),
    }) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      p.uiState.workspace.interface.toolBarPosition = position;
      await tester.pumpWidget(hostPanel(
        child: const LumitAppView(),
        state: p.state,
        uiState: p.uiState,
        size: size,
        shape: shape,
        density: DensityTokens.forShape(shape, false),
      ));
      await tester.pumpAndSettle();
      return p;
    }

    Finder inTopLine(String key) => find.descendant(
        of: find.byType(LumitMenuBarFrb), matching: find.byKey(ValueKey(key)));

    testWidgets(
        'left: no strip, and the line carries the options and the'
        ' workspaces', (tester) async {
      for (final shape in ThemeShape.values) {
        final p =
            await mount(tester, shape: shape, position: ToolBarPosition.left);
        expect(find.byType(LumitToolBarFrb), findsNothing,
            reason: 'the strip is not mounted under $shape');
        expect(find.byType(LumitToolRailFrb), findsOneWidget);
        expect(tester.getSize(find.byType(LumitMenuBarFrb)).height,
            DensityTokens.forShape(shape, false).menuBar,
            reason: 'the line is the shape\'s own height under $shape');
        expect(inTopLine('workspace-edit'), findsOneWidget,
            reason: 'the workspaces ride the line under $shape');
        if (shape == ThemeShape.lantern) {
          expect(inTopLine('workspace-pill'), findsOneWidget);
          expect(inTopLine('tool-options-pill'), findsOneWidget);
          expect(inTopLine('tool-no-options'), findsOneWidget,
              reason: 'the Selection tool has no options, and the pill says'
                  ' so on the line');
        }
        // A tool with options puts them on the line, keyed as they always
        // were on the strip.
        p.uiState.tools.select(ToolMode.brush);
        await tester.pumpAndSettle();
        expect(inTopLine('tool-brush-shape'), findsOneWidget,
            reason: 'the brush\'s options are on the line under $shape');
        expect(inTopLine('tool-brush-pressure'), findsOneWidget);
        expect(tester.takeException(), isNull, reason: '$shape');
      }
    });

    /// Seen in a real window at this size: the workspace pill began too far
    /// right and lost Audio and Retiming past the edge, and the options pill
    /// drew its word in its upper half.
    testWidgets(
        'left, lantern at 1704: the workspaces end inside the window and the'
        ' options pill is centred on the line', (tester) async {
      const size = Size(1704, 961);
      await mount(tester,
          shape: ThemeShape.lantern,
          position: ToolBarPosition.left,
          size: size);
      final ws = tester.getRect(inTopLine('workspace-pill'));
      expect(ws.right, lessThanOrEqualTo(size.width));
      // The command box takes the line's end; the pill stands right up
      // against it.
      final box = tester.getRect(inTopLine('command-box'));
      expect(box.right, lessThanOrEqualTo(size.width));
      expect(ws.right, lessThanOrEqualTo(box.left));
      expect(ws.right, greaterThan(box.left - 16),
          reason: 'the pill stands at the right end when it fits');
      for (final preset in WorkspacePreset.values) {
        final r = tester.getRect(inTopLine('workspace-${preset.name}'));
        expect(r.left, greaterThanOrEqualTo(0), reason: preset.name);
        expect(r.right, lessThanOrEqualTo(size.width), reason: preset.name);
      }
      final band = tester.getRect(find.byType(LumitMenuBarFrb));
      expect(tester.getRect(inTopLine('tool-options-pill')).center.dy,
          closeTo(band.center.dy, 0.5));
      expect(tester.getRect(inTopLine('tool-no-options')).center.dy,
          closeTo(band.center.dy, 0.5),
          reason: 'the word is centred in the pill, not at its top');
      expect(tester.takeException(), isNull);
    });

    testWidgets('top: Studio keeps its 26 line and the strip under it',
        (tester) async {
      await mount(tester,
          shape: ThemeShape.studio, position: ToolBarPosition.top);
      expect(tester.getSize(find.byType(LumitMenuBarFrb)).height, 26);
      expect(find.byType(LumitToolBarFrb), findsOneWidget);
      expect(find.byType(LumitToolRailFrb), findsNothing);
      expect(inTopLine('workspace-edit'), findsNothing,
          reason: 'the workspaces stay on the strip');
      expect(
          find.descendant(
              of: find.byType(LumitToolBarFrb),
              matching: find.byKey(const ValueKey('workspace-edit'))),
          findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  }, skip: !engineAvailable);
}
