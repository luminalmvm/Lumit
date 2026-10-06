// The toolbar as it is mounted in the shell (docs/07 §1.7).
//
// It draws from `LumitUiState` — the armed tool, the keymap the tooltips quote,
// the workspace it rearranges — so it runs against the real engine like every
// other shell surface here. What is asserted is the gestures a toolbar lives or
// dies by: a click arms, a right-click reaches the hidden tools, and the button
// then shows the one that was picked.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/shell/tool_bar_frb.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/settings.dart' show ToolBarPosition;
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:provider/provider.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Toolbar (frb)', () {
    /// The strip as the shell mounts it, and under [position] Left the
    /// shell's own arrangement instead: the menu bar's line carrying the
    /// options and the workspaces, and the rail beside an empty dock.
    Future<({LumitState state, LumitUiState uiState})> mount(
      WidgetTester tester, {
      ThemeShape shape = ThemeShape.studio,
      ToolBarPosition position = ToolBarPosition.top,
    }) async {
      final p = freshProject();
      p.uiState.workspace.interface.toolBarPosition = position;
      // Wide enough that the strip is not scrolled off: the buttons are
      // pressed by key, and a widget scrolled out of view cannot be tapped.
      // The **view** has to be told, not only the MediaQuery: the tools sit in
      // a horizontal scroll view, and what they are laid out against is the
      // real surface, which otherwise stays at the 800x600 default and hides
      // the last few tools behind the workspace strip.
      const size = Size(1400, 300);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: position == ToolBarPosition.top
            ? const Align(
                alignment: Alignment.topLeft,
                child: LumitToolBarFrb(),
              )
            : Column(children: [
                Builder(builder: (context) {
                  final state = context.watch<LumitState>();
                  context.watch<LumitUiState>();
                  return LumitMenuBarFrb(app: state);
                }),
                const Expanded(
                  child: Row(children: [
                    LumitToolRailFrb(),
                    Expanded(child: SizedBox()),
                  ]),
                ),
              ]),
        state: p.state,
        uiState: p.uiState,
        size: size,
        shape: shape,
        // The line's height is the shape's: Lantern's 28 pills need its 40.
        density: DensityTokens.forShape(shape, false),
      ));
      await tester.pump();
      return p;
    }

    testWidgets('every tool group has a button', (tester) async {
      await mount(tester);
      for (final group in toolBarOrder) {
        expect(
            find.byKey(ValueKey<String>('tool-${group.name}')), findsOneWidget,
            reason: '$group has no way to be armed');
      }
      expect(toolBarOrder.toSet(), ToolGroup.values.toSet(),
          reason:
              'a tool group missing from the strip is a tool nobody can reach');
    });

    testWidgets('clicking a button arms that group', (tester) async {
      final p = await mount(tester);
      expect(p.uiState.tools.tool, ToolMode.select);

      await tester.tap(find.byKey(const ValueKey('tool-pen')));
      await tester.pump();

      expect(p.uiState.tools.tool, ToolMode.pen);
    });

    testWidgets(
        'right-clicking opens the hidden tools, and picking one arms it'
        ' and sticks to the button', (tester) async {
      final p = await mount(tester);
      final shape = find.byKey(const ValueKey('tool-shape'));

      await tester.tapAt(tester.getCenter(shape), buttons: kSecondaryButton);
      await tester.pumpAndSettle();

      final star = find.byKey(const ValueKey('tool-flyout-shapeStar'));
      expect(star, findsOneWidget, reason: 'the flyout lists the whole group');

      await tester.tap(star);
      await tester.pumpAndSettle();

      expect(p.uiState.tools.tool, ToolMode.shapeStar);
      expect(p.uiState.tools.memberOf(ToolGroup.shape), ToolMode.shapeStar,
          reason: 'the button now stands for the star, as AE does');
    });

    /// **The user's own workspaces are on the strip too** (docs/07 §1.4, item
    /// 7.19), after the presets and drawn by exactly the same rules — a
    /// workspace somebody saved is a workspace, not a lesser kind of one.
    testWidgets('the strip lists the user\'s own after the presets',
        (tester) async {
      final p = await mount(tester);
      p.uiState.workspace.applyWorkspacePreset(WorkspacePreset.colour);
      p.uiState.workspace.saveWorkspaceAs('Grading');
      addTearDown(() => p.uiState.workspace.deleteUserWorkspace('Grading'));
      await tester.pumpAndSettle();

      expect(find.text('GRADING'), findsOneWidget,
          reason: 'saved names join the strip as mono-caps kickers');
      // After the last preset, which is where the chords count them from.
      final last = tester.getRect(find.byKey(
          ValueKey<String>('workspace-${WorkspacePreset.values.last.name}')));
      final saved =
          tester.getRect(find.byKey(const ValueKey('workspace-user-Grading')));
      expect(saved.left, greaterThan(last.left));

      // And it is the one ticked, because saving switches to what was saved.
      final tick = (tester
              .widget<Container>(find
                  .descendant(
                    of: find.byKey(const ValueKey('workspace-user-Grading')),
                    matching: find.byType(Container),
                  )
                  .last)
              .decoration as BoxDecoration?)
          ?.border as Border?;
      expect(tick?.bottom.color, LumitTheme.dark().accent);

      await tester.tap(find.byKey(const ValueKey('workspace-edit')));
      await tester.pump();
      expect(p.uiState.workspace.activeUserWorkspace, isNull,
          reason: 'picking a preset unticks the saved one');

      await tester.tap(find.byKey(const ValueKey('workspace-user-Grading')));
      await tester.pump();
      expect(p.uiState.workspace.activeUserWorkspace, 'Grading');
      expect(panelsIn(p.uiState.workspace.dock),
          panelsIn(presetLayout(WorkspacePreset.colour)),
          reason: 'and it puts back the arrangement it was saved from');
    });

    /// The tool options area: After Effects shows the settings the
    /// armed tool draws with, and nothing at all for the tools that draw
    /// nothing.
    testWidgets('the options area follows the armed tool', (tester) async {
      final p = await mount(tester);
      expect(find.text('Fill'), findsNothing,
          reason: 'the Selection tool draws nothing');

      p.uiState.tools.select(ToolMode.typeHorizontal);
      await tester.pump();
      expect(find.text('Fill'), findsOneWidget);
      expect(find.text('Stroke'), findsNothing,
          reason: 'type has a fill and a size, not a stroke');

      p.uiState.tools.select(ToolMode.shapeRectangle);
      await tester.pump();
      expect(find.text('Fill'), findsOneWidget);
      expect(find.text('Stroke'), findsOneWidget);

      // A painting tool shows the brush's own three settings, all live — no
      // disabled stroke pair, because painting is built.
      p.uiState.tools.select(ToolMode.brush);
      await tester.pump();
      expect(find.text('Fill'), findsOneWidget);
      expect(find.text('Size'), findsOneWidget);
      expect(find.text('Hardness'), findsOneWidget);
      expect(find.text('Opacity'), findsOneWidget);
      expect(find.text('Stroke'), findsNothing);

      p.uiState.tools.select(ToolMode.hand);
      await tester.pump();
      expect(find.text('Fill'), findsNothing);
    });

    testWidgets('every tool group names a chord the engine knows',
        (tester) async {
      final p = await mount(tester);
      // The tooltips teach the shortcut (docs/07 §14), and they can only teach
      // one the keymap actually carries — this is the check that the ids in
      // `toolActions` match the ones the engine ships.
      for (final entry in toolActions.entries) {
        expect(p.uiState.keymap.chordFor(entry.key), isNotNull,
            reason: '${entry.key} has no binding in the shipped keymap');
      }
    });
  }, skip: !engineAvailable);
}
