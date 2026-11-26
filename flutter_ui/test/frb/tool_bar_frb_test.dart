
import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart' show stillColour;
import 'package:lumit_flutter/panels/shape_entries.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/shell/tool_bar_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/settings.dart' show ToolBarPosition;
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Toolbar (frb)', () {
    Future<({LumitState state, LumitUiState uiState})> mount(
      WidgetTester tester, {
      ThemeShape shape = ThemeShape.studio,
      ToolBarPosition position = ToolBarPosition.top,
    }) async {
      final p = freshProject();
      p.uiState.workspace.interface.toolBarPosition = position;
      // Wide enough that no tool folds: the buttons are pressed by key, and
      // a folded one cannot be tapped. The **view** has to be told, not only
      // the MediaQuery: the tools fold against the real surface, which
      // otherwise stays at the 800x600 default and folds the last few tools
      // into the overflow mark.
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
        'right-clicking opens the tool\'s menu, and picking one arms it'
        ' and sticks to the button', (tester) async {
      final p = await mount(tester);
      final shape = find.byKey(const ValueKey('tool-shape'));

      await tester.tapAt(tester.getCenter(shape), buttons: kSecondaryButton);
      await tester.pumpAndSettle();

      final star = find.byKey(const ValueKey('menu-row-Star'));
      expect(star, findsOneWidget, reason: 'the menu lists the whole group');

      await tester.tap(star);
      await tester.pumpAndSettle();

      expect(p.uiState.tools.tool, ToolMode.shapeStar);
      expect(p.uiState.tools.memberOf(ToolGroup.shape), ToolMode.shapeStar,
          reason: 'the button now stands for the star, as AE does');
    });

    testWidgets('the strip lists the user\'s own after the presets',
        (tester) async {
      final p = await mount(tester);
      p.uiState.workspace.applyWorkspacePreset(WorkspacePreset.colour);
      p.uiState.workspace.saveWorkspaceAs('Grading');
      addTearDown(() => p.uiState.workspace.deleteUserWorkspace('Grading'));
      await tester.pumpAndSettle();

      expect(find.text('Grading'), findsOneWidget,
          reason: 'saved names join the strip as kickers');
      final last = tester.getRect(find.byKey(
          ValueKey<String>('workspace-${WorkspacePreset.values.last.name}')));
      final saved =
          tester.getRect(find.byKey(const ValueKey('workspace-user-Grading')));
      expect(saved.left, greaterThan(last.left));

      final button = tester.widget<HouseButton>(find.descendant(
        of: find.byKey(const ValueKey('workspace-user-Grading')),
        matching: find.byType(HouseButton),
      ));
      expect(button.active, isTrue);

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

    testWidgets('the shape options edit the selected shape layer',
        (tester) async {
      final p = await mount(tester);
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      BridgeKeyframe key(int frame, double value) => BridgeKeyframe(
            time: comp.timeOfFrame(frame: frame),
            value: value,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          );
      const still = BridgeScalar.static_(0);
      const full = BridgeScalar.static_(100);
      final layer = comp.addShapeLayer(name: 'Art', contents: [
        BridgeShapeItem(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Square',
          vertices: [
            for (final (x, y) in [(0.0, 0.0), (90.0, 0.0), (90.0, 90.0)])
              BridgeVertex(x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
          ],
          closed: true,
          fill: stillColour(const BridgeColourRgba(r: 1, g: 1, b: 1, a: 1)),
          strokeWidth: BridgeScalar.keyframed([key(0, 2), key(20, 10)]),
          opacity: full,
          dashes: const [],
          dashOffset: still,
          gradient: 0,
          gradientStartX: still,
          gradientStartY: still,
          gradientEndX: still,
          gradientEndY: still,
          gradientStartOpacity: full,
          gradientEndOpacity: full,
          gradientStops: const [],
          combine: 0,
          hidden: false,
          fillHidden: false,
          strokeHidden: false,
          pathKeys: const [],
        ),
      ]);
      p.uiState.model.refresh();
      p.uiState.setSelection([layer]);
      p.uiState.playheadFrame.value = 10;
      p.uiState.tools.select(ToolMode.shapeRectangle);
      await tester.pump();
      BridgeShapeItem art() => layer.getShapeContents().items.single;

      await tester.tap(find.byWidgetPredicate((w) =>
          w is Container &&
          w.constraints?.maxWidth == 20 &&
          w.child is CustomPaint));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('colour-picker-cancel')));
      await tester.pumpAndSettle();
      expect(art().stroke, isNull);

      await tester.tap(find.text('Fill'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tool-swatch-none')));
      await tester.pumpAndSettle();
      expect(art().fillHidden, isTrue);
      expect(art().fill, isNotNull, reason: 'None keeps the colour and its keys');
      expect(p.uiState.tools.fillOn, isTrue,
          reason: 'what the next new layer is drawn with is left alone');

      await tester.drag(find.byType(DragValueField), const Offset(40, 0));
      await tester.pump();
      final width = art().strokeWidth as BridgeScalar_Keyframed;
      expect(width.field0, hasLength(3));
      expect(width.field0[1].value, greaterThan(6));
      expect(p.uiState.tools.strokeWidth, 2);
      p.state.project!.undo();
      expect((art().strokeWidth as BridgeScalar_Keyframed).field0,
          hasLength(2));
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
