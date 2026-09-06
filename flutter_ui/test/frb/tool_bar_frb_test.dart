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
import 'package:lumit_flutter/shell/tool_bar_frb.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Toolbar (frb)', () {
    Future<({LumitState state, LumitUiState uiState})> mount(
        WidgetTester tester) async {
      final p = freshProject();
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
        child: const Align(
          alignment: Alignment.topLeft,
          child: LumitToolBarFrb(),
        ),
        state: p.state,
        uiState: p.uiState,
        size: size,
      ));
      await tester.pump();
      return p;
    }

    /// **The tool options fit the strip it was shrunk to** (30px
    /// tall). They are the only things on the bar taller than an icon — a
    /// colour well and a number field — so they are what a shorter strip breaks
    /// first, and an overflow stripe is not a design.
    testWidgets('the armed tool options fit the shortened strip',
        (tester) async {
      final p = await mount(tester);

      for (final (tool, what) in [
        (ToolMode.typeHorizontal, 'the Type tool shows a fill and a size'),
        (ToolMode.shapeRectangle, 'a shape tool shows a fill and a stroke'),
      ]) {
        p.uiState.tools.select(tool);
        await tester.pump();

        expect(tester.takeException(), isNull, reason: what);
        expect(find.text('Fill'), findsOneWidget, reason: what);
        // The strip is the height it says it is, options and all.
        expect(
          tester.getSize(find.byType(LumitToolBarFrb)).height,
          toolBarHeight,
        );
      }
    });

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

    testWidgets('a single-tool group offers no flyout', (tester) async {
      await mount(tester);
      final hand = find.byKey(const ValueKey('tool-hand'));
      await tester.tapAt(tester.getCenter(hand), buttons: kSecondaryButton);
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('tool-flyout-hand')), findsNothing);
    });

    // The switch that used to sit here governed nothing, so it is not
    // on the strip. This is the guard against it drifting back on before there
    // is snapping for it to govern.
    /// **The magnet is back**: it was taken off the strip when nothing in the
    /// application read it, and it returned with the snapping it governs — the
    /// Viewer's layer drags reach for the guides and the grid through exactly
    /// this switch.
    testWidgets('the snapping switch is on the strip', (tester) async {
      final p = await mount(tester);
      final magnet = find.byKey(const ValueKey('tool-snapping'));
      expect(magnet, findsOneWidget);

      expect(p.uiState.tools.snapping, isTrue, reason: 'on by default');
      await tester.tap(magnet);
      await tester.pump();
      expect(p.uiState.tools.snapping, isFalse);
    });

    /// **A button you cannot read is a button you cannot use.** The
    /// strip lost 14px of height and the workspace names kept 24px of padding,
    /// which squeezed the words out of a 30px band and left four pressable
    /// blanks on the right of the bar.
    testWidgets('the workspace names are readable inside the strip',
        (tester) async {
      await mount(tester);
      final bar = tester.getRect(find.byType(LumitToolBarFrb));

      for (final preset in WorkspacePreset.values) {
        // Set as a kicker, so what is on screen is the name in capitals.
        final label = find.text(preset.title.toUpperCase());
        expect(label, findsOneWidget,
            reason: '${preset.title} is on the strip');
        final rect = tester.getRect(label);
        // Its own line height, not whatever is left over. Squeezed into the
        // padding it measured three pixels tall and read as nothing at all —
        // which is how four pressable blanks came to sit on the right of the
        // bar. Eight is below a real line — the names are 9px kickers — and
        // far above a crushed one.
        expect(rect.height, greaterThanOrEqualTo(8),
            reason: '${preset.title} has room for its own words');
        expect(rect.top, greaterThanOrEqualTo(bar.top - 0.5));
        expect(rect.bottom, lessThanOrEqualTo(bar.bottom + 0.5),
            reason: '${preset.title} is inside the strip, not clipped by it');
      }
      expect(tester.takeException(), isNull);
    });

    /// **The workspace strip belongs at the right-hand end** (docs/07 §1.4),
    /// after a divider, not beside the last tool.
    ///
    /// It drifted left when the tool options arrived: the tools took a *loose*
    /// Flexible, which claims only the width it needs, so the free space was
    /// stranded past the workspace buttons instead of in front of them and the
    /// whole right-hand group sat wherever the tools happened to end.
    testWidgets('the workspace strip is held against the right-hand end',
        (tester) async {
      final p = await mount(tester);
      final bar = tester.getRect(find.byType(LumitToolBarFrb));
      // Whichever preset is last, not Audio by name: Retiming took that
      // place, and a fifth preset must not be able to hang off the bar unnoticed
      // because the test was watching the fourth.
      final lastKey =
          ValueKey<String>('workspace-${WorkspacePreset.values.last.name}');
      final last = tester.getRect(find.byKey(lastKey));
      expect(bar.right - last.right, lessThan(40),
          reason: 'the last workspace button ends where the bar does');

      // And it stays there when the armed tool grows an options strip, which
      // is the change that moved it in the first place.
      p.uiState.tools.select(ToolMode.brush);
      await tester.pumpAndSettle();
      final withOptions = tester.getRect(find.byKey(lastKey));
      expect(bar.right - withOptions.right, lessThan(40),
          reason: "the tool options push nothing off the bar's right end");
      expect(tester.takeException(), isNull);
    });

    /// **The workspace tabs are mono-caps kickers with an accent tick under
    /// the one in force** (docs/15 §12A.1, §3.1 — these are what "the active
    /// tab tick" means). They regressed to sentence-case words tinted in the
    /// accent, which spent the accent on text and left the strip with no tick
    /// at all.
    testWidgets('the workspace tabs are kickers, ticked in the accent',
        (tester) async {
      final p = await mount(tester);
      final t = LumitTheme.dark();

      expect(find.text('EDIT'), findsOneWidget,
          reason: 'the names are set as kickers, in capitals');
      expect(find.text('Edit'), findsNothing);

      Border? tickOf(WorkspacePreset preset) => (tester
              .widget<Container>(find
                  .descendant(
                    of: find
                        .byKey(ValueKey<String>('workspace-${preset.name}')),
                    matching: find.byType(Container),
                  )
                  .last)
              .decoration as BoxDecoration?)
          ?.border as Border?;

      expect(tickOf(WorkspacePreset.edit), isNull,
          reason: 'nothing is ticked until a preset is chosen');

      p.uiState.workspace.applyWorkspacePreset(WorkspacePreset.edit);
      await tester.pumpAndSettle();

      expect(tickOf(WorkspacePreset.edit)?.bottom.color, t.accent,
          reason: 'the workspace in force wears the accent tick');
      expect(tickOf(WorkspacePreset.audio), isNull,
          reason: 'and only that one does');
      expect(tester.widget<Text>(find.text('EDIT')).style?.color, t.textPrimary,
          reason: 'the word itself stays grey — the tick is the state');
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

    testWidgets('the workspace strip rearranges the panels', (tester) async {
      final p = await mount(tester);
      expect(p.uiState.workspace.activePreset, isNull,
          reason: 'nothing is ticked until a preset is chosen');

      await tester.tap(find.byKey(const ValueKey('workspace-effects')));
      await tester.pump();

      expect(p.uiState.workspace.activePreset, WorkspacePreset.effects);
    });

    /// The Nodes tab is generated from the enum like the rest —
    /// no strip of its own — and the arrangement it applies is the one with
    /// the Graph and Node panels in it.
    testWidgets('the Nodes tab is on the strip and switches to its workspace',
        (tester) async {
      final p = await mount(tester);
      final t = LumitTheme.dark();
      expect(find.text('NODES'), findsOneWidget,
          reason: 'a new preset joins the strip by existing');

      await tester.tap(find.byKey(const ValueKey('workspace-nodes')));
      await tester.pumpAndSettle();

      expect(p.uiState.workspace.activePreset, WorkspacePreset.nodes);
      expect(panelsIn(p.uiState.workspace.dock),
          [Panel.graph, Panel.timeline, Panel.viewer, Panel.node]);
      final tick = ((tester
                  .widget<Container>(find
                      .descendant(
                        of: find.byKey(const ValueKey('workspace-nodes')),
                        matching: find.byType(Container),
                      )
                      .last)
                  .decoration as BoxDecoration?)
              ?.border as Border?)
          ?.bottom
          .color;
      expect(tick, t.accent, reason: 'the workspace in force wears the tick');
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

    /// The Roto pair arms together (one gate read from the other side): the
    /// strip button, the flyout row and the chord all wake off one flag, so
    /// pressing the button arms the brush and the flyout offers Refine edge
    /// beside it.
    testWidgets('the roto group arms and opens its flyout', (tester) async {
      final p = await mount(tester);

      final roto = find.byKey(const ValueKey('tool-roto'));
      await tester.ensureVisible(roto);
      await tester.pumpAndSettle();

      // The flyout first, while the strip is still laid out as it was: arming
      // a roto tool brings its Size option onto the bar, which moves the
      // buttons.
      await tester.tapAt(
        tester.getCenter(roto),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      final refine = find.byKey(const ValueKey('tool-flyout-refineEdge'));
      expect(refine, findsOneWidget, reason: 'both are built');

      await tester.tap(refine);
      await tester.pumpAndSettle();
      expect(p.uiState.tools.tool, ToolMode.refineEdge);
    });

    /// The other side of the same rule: the four Puppet pins have an
    /// engine behind them now, so the button arms and the flyout lists them.
    testWidgets('the puppet group arms and opens its flyout', (tester) async {
      final p = await mount(tester);

      final puppet = find.byKey(const ValueKey('tool-puppet'));
      await tester.ensureVisible(puppet);
      await tester.pumpAndSettle();

      // The flyout first, while the strip is still laid out as it was: arming
      // a puppet tool brings its options onto the bar, which moves the buttons.
      await tester.tapAt(
        tester.getCenter(puppet),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      final bend = find.byKey(const ValueKey('tool-flyout-puppetBend'));
      expect(bend, findsOneWidget, reason: 'all four are built');

      await tester.tap(bend);
      await tester.pumpAndSettle();
      expect(p.uiState.tools.tool, ToolMode.puppetBend);
    });

    testWidgets('an unbuilt member of a mixed group is listed but inert',
        (tester) async {
      final p = await mount(tester);

      final pen = find.byKey(const ValueKey('tool-pen'));
      await tester.ensureVisible(pen);
      await tester.pumpAndSettle();
      await tester.tapAt(tester.getCenter(pen), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('tool-flyout-penMaskFeather')),
          findsOneWidget,
          reason: 'listed, so the gap is visible');
      expect(find.text('Not built'), findsWidgets);

      await tester
          .tap(find.byKey(const ValueKey('tool-flyout-penMaskFeather')));
      await tester.pumpAndSettle();
      expect(p.uiState.tools.tool, ToolMode.select,
          reason: 'and inert, so picking it does nothing');
    });

    testWidgets('the camera tools can be armed', (tester) async {
      final p = await mount(tester);
      final camera = find.byKey(const ValueKey('tool-camera'));
      await tester.ensureVisible(camera);
      await tester.pumpAndSettle();
      await tester.tap(camera);
      await tester.pump();
      expect(p.uiState.tools.tool.group, ToolGroup.camera);
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
