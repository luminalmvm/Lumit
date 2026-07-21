// Phase F4 first slice: the Hierarchy tree, the Effect controls Transform rows,
// and the composition-settings dialogue. Widget tests over a fake DocumentBridge
// (no library, no plugin channels).

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel.dart';
import 'package:lumit_flutter/panels/hierarchy_panel.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// A fake bridge whose snapshot carries a front comp "Scene" (a footage layer
/// and a precomp layer nesting "Nested"), plus the "Nested" comp itself. Ops are
/// recorded as strings for the assertions.
class _FakeBridge implements DocumentBridge {
  final List<String> ops = [];

  static const _json = '''
  {
    "ok": true,
    "items": [
      {
        "id": "c1", "name": "Scene", "kind": "composition", "children": [],
        "comp": {
          "width": 1920, "height": 1080, "fps": {"num": 60, "den": 1},
          "frame_count": 300,
          "layers": [
            {"id":"l0","index":0,"name":"top","kind":"footage",
             "in_frame":0,"out_frame":300,"label":0,"switches":{}},
            {"id":"l1","index":1,"name":"Nested","kind":"precomp",
             "in_frame":0,"out_frame":300,"label":0,"switches":{}}
          ],
          "markers": []
        }
      },
      {
        "id": "c2", "name": "Nested", "kind": "composition", "children": [],
        "comp": {
          "width": 1920, "height": 1080, "fps": {"num": 60, "den": 1},
          "frame_count": 300,
          "layers": [
            {"id":"la","index":0,"name":"inner","kind":"footage",
             "in_frame":0,"out_frame":300,"label":0,"switches":{}}
          ],
          "markers": []
        }
      }
    ],
    "can_undo": false, "can_redo": false, "path": null
  }''';

  BridgeReply _snap() => BridgeReply.parse(_json);

  @override
  BridgeReply snapshot() => _snap();
  @override
  BridgeReply newProject() => _snap();
  @override
  BridgeReply undo() => _snap();
  @override
  BridgeReply redo() => _snap();
  @override
  BridgeReply openProject(String path) => _snap();
  @override
  BridgeReply saveProject(String path) => _snap();
  @override
  BridgeReply newComposition(String name) => _snap();
  @override
  BridgeReply importFootage(String path) => _snap();
  @override
  BridgeReply setLayerSwitch(
          String compId, String layerId, String switchName, bool value) =>
      _snap();
  @override
  BridgeReply editLayerSpan(
          String compId, String layerId, String edit, int frame) =>
      _snap();
  @override
  BridgeReply setTransform(
      String compId, String layerId, String property, double value) {
    ops.add('transform:$compId/$layerId/$property=$value');
    return _snap();
  }

  @override
  BridgeReply addMarker(String compId, int frame) => _snap();
  @override
  BridgeReply addSolidLayer(String compId) => _snap();
  @override
  BridgeReply addTextLayer(String compId) => _snap();
  @override
  BridgeReply addCameraLayer(String compId) => _snap();
  @override
  BridgeReply addAdjustmentLayer(String compId) => _snap();
  @override
  BridgeReply addSequenceLayer(String compId) => _snap();
  @override
  BridgeReply deleteLayer(String compId, String layerId) => _snap();
  @override
  BridgeReply duplicateLayer(String compId, String layerId) => _snap();
  @override
  BridgeReply setCompSettings(String compId, String name, int width, int height,
          int fpsNum, int fpsDen, int durationFrames) =>
      _snap();
  @override
  BridgeReply togglePropertyAnimated(
          String compId, String layerId, String property, int frame) =>
      _snap();
  @override
  BridgeReply addKeyframe(String compId, String layerId, String property,
          int frame, double value) =>
      _snap();
  @override
  BridgeReply removeKeyframe(
          String compId, String layerId, String property, int frame) =>
      _snap();
  @override
  BridgeReply shiftKeyframes(String compId, String layerId, String property,
          List<int> frames, int delta) =>
      _snap();
  @override
  BridgeReply setWorkAreaEdge(String compId, int frame, bool isOut) => _snap();
  @override
  List<BridgeEffectInfo> listEffects() => const [];
  @override
  BridgeReply addEffect(String compId, String layerId, String effectName) =>
      _snap();
  @override
  BridgeReply removeEffect(String compId, String layerId, String effectId) =>
      _snap();
  @override
  BridgeReply setEffectEnabled(
          String compId, String layerId, String effectId, bool enabled) =>
      _snap();
  @override
  BridgeReply setEffectParamScalar(String compId, String layerId,
          String effectId, String paramName, double value) =>
      _snap();
  @override
  BridgeReply setEffectParamColour(String compId, String layerId,
          String effectId, String paramName, double r, double g, double b,
          double a) =>
      _snap();
  @override
  DecodedFrame? decodeFrame(String itemId, int frame) => null;
}

/// A minimal host: the theme scope over an Overlay holding [child].
Widget _host(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: MediaQuery(
        data: const MediaQueryData(size: Size(420, 700)),
        child: ThemeScope(
          theme: LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [OverlayEntry(builder: (_) => child)],
          ),
        ),
      ),
    );

void main() {
  group('Hierarchy panel', () {
    testWidgets('renders the front comp, its layers, and selects on click',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final app = AppStateStub(bridge: _FakeBridge());
      await tester.pumpWidget(_host(HierarchyPanel(app: app)));
      await tester.pump();

      // The comp header and both layers of the front comp.
      expect(find.text('Scene'), findsOneWidget);
      expect(find.text('top'), findsOneWidget);
      expect(find.text('Nested'), findsOneWidget);

      // Clicking a layer row selects it by its stable layer id.
      expect(app.selectedLayer, isNull);
      await tester.tap(find.text('top'));
      await tester.pump();
      expect(app.selectedLayer, 'l0');
    });

    testWidgets('a precomp twirl folds the nested comp\'s layers',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final app = AppStateStub(bridge: _FakeBridge());
      await tester.pumpWidget(_host(HierarchyPanel(app: app)));
      await tester.pump();

      // The first-level precomp opens by default, so its nested layer shows.
      expect(find.text('inner'), findsOneWidget);

      // Collapsing the twirl hides it; expanding reveals it again.
      await tester.tap(find.byKey(const ValueKey('twirl-l1')));
      await tester.pump();
      expect(find.text('inner'), findsNothing);
      await tester.tap(find.byKey(const ValueKey('twirl-l1')));
      await tester.pump();
      expect(find.text('inner'), findsOneWidget);
    });
  });

  group('Effect controls panel', () {
    testWidgets('shows the Transform rows for the selected layer',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final app = AppStateStub(bridge: _FakeBridge())..selectLayer('l0');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      expect(find.text('top'), findsOneWidget); // the layer title
      expect(find.text('Transform'), findsOneWidget);
      for (final label in [
        'Anchor point',
        'Position',
        'Scale',
        'Rotation',
        'Opacity',
      ]) {
        expect(find.text(label), findsOneWidget);
      }
      // A non-3D layer shows no Rotation x/y rows.
      expect(find.text('Rotation x'), findsNothing);
    });

    testWidgets('no selection shows the hint', (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final app = AppStateStub(bridge: _FakeBridge());
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();
      expect(find.textContaining('Select a layer'), findsOneWidget);
    });

    testWidgets('editing Position x commits through setTransform',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake)..selectLayer('l0');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      // The value box seeds from read-back (or the property seed when the fake
      // snapshot carries no transform), and edits in place — no em-dash step:
      // tap to type, enter a value, commit.
      await tester.tap(find.byKey(const ValueKey('axis-position_x')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText), '250');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      expect(fake.ops, contains('transform:c1/l0/position_x=250.0'));
      // The commit is remembered so the box now reads the value back.
      expect(app.transformValueFor('l0', 'position_x'), 250.0);
    });
  });

  group('Composition settings dialogue', () {
    testWidgets('opens from the menu, edits a field, and closes on Apply',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(1280, 800));
      await tester.pumpWidget(LumitApp(workspace: Workspace()));
      await tester.pumpAndSettle();

      await tester.tap(find.text('Composition'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Composition settings…'));
      await tester.pumpAndSettle();

      // The dialogue is up with its fields.
      expect(find.text('Composition settings'), findsOneWidget);
      expect(find.text('Name'), findsOneWidget);
      expect(find.text('Size'), findsOneWidget);
      expect(find.text('Frame rate'), findsOneWidget);
      expect(find.text('Duration'), findsOneWidget);

      // The name field edits.
      await tester.enterText(find.byType(EditableText), 'Intro');
      await tester.pump();

      // Apply closes it (the commit is honestly stubbed).
      await tester.tap(find.text('Apply'));
      await tester.pumpAndSettle();
      expect(find.text('Composition settings'), findsNothing);
    });
  });
}
