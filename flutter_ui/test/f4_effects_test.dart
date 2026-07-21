// Phase F4: the Effect controls going fully live (transform read-back values,
// stopwatch, keyframe navigator, effect stack) and the Effects & presets panel.
// Widget tests over a fake DocumentBridge whose snapshot carries a transform
// read-back and an effect stack, and which records the ops the panels dispatch.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/effect_controls_panel.dart';
import 'package:lumit_flutter/panels/effects_presets_panel.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// A fake bridge whose front comp "Scene" carries one layer "l0" with a
/// transform read-back (Position animated with keys at frames 10 and 20;
/// Rotation static at 30) and one effect "e1" ("blur") with a scalar, a colour,
/// an enum and a bool parameter. Ops are recorded as strings for the
/// assertions.
class _FakeBridge implements DocumentBridge {
  final List<String> ops = [];

  static const _json = '''
  {
    "ok": true,
    "items": [
      {
        "id": "c1", "name": "Scene", "kind": "composition", "children": [],
        "comp": {
          "width": 1920, "height": 1080, "fps": {"num": 30, "den": 1},
          "frame_count": 300,
          "layers": [
            {"id":"l0","index":0,"name":"top","kind":"footage",
             "in_frame":0,"out_frame":300,"label":0,"switches":{},
             "transform": {
               "position_x": {"value": 123.0, "animated": true,
                 "keys": [
                   {"frame": 10, "value": 123.0, "interp_in":"Linear","interp_out":"Linear"},
                   {"frame": 20, "value": 200.0, "interp_in":"Linear","interp_out":"Linear"}
                 ]},
               "position_y": {"value": 45.0, "animated": true,
                 "keys": [
                   {"frame": 10, "value": 45.0, "interp_in":"Linear","interp_out":"Linear"},
                   {"frame": 20, "value": 60.0, "interp_in":"Linear","interp_out":"Linear"}
                 ]},
               "rotation": {"value": 30.0, "animated": false, "keys": []}
             },
             "effects": [
               {"id":"e1","name":"blur","enabled":true,"params":[
                 {"name":"radius","kind":"scalar","value":5.0},
                 {"name":"tint","kind":"colour","value":[1.0,0.0,0.0,1.0]},
                 {"name":"mode","kind":"enum","value":2},
                 {"name":"invert","kind":"bool","value":true}
               ]}
             ]
            }
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
      String compId, String layerId, String property, int frame) {
    ops.add('toggle:$compId/$layerId/$property@$frame');
    return _snap();
  }

  @override
  BridgeReply addKeyframe(String compId, String layerId, String property,
      int frame, double value) {
    ops.add('addkey:$compId/$layerId/$property@$frame=$value');
    return _snap();
  }

  @override
  BridgeReply removeKeyframe(
      String compId, String layerId, String property, int frame) {
    ops.add('removekey:$compId/$layerId/$property@$frame');
    return _snap();
  }

  @override
  BridgeReply shiftKeyframes(String compId, String layerId, String property,
          List<int> frames, int delta) =>
      _snap();
  @override
  BridgeReply setWorkAreaEdge(String compId, int frame, bool isOut) => _snap();
  @override
  List<BridgeEffectInfo> listEffects() => const [
        BridgeEffectInfo(name: 'blur', label: 'Gaussian blur'),
        BridgeEffectInfo(name: 'glow', label: 'Glow'),
        BridgeEffectInfo(name: 'tint', label: 'Tint'),
      ];
  @override
  BridgeReply addEffect(String compId, String layerId, String effectName) {
    ops.add('addeffect:$compId/$layerId/$effectName');
    return _snap();
  }

  @override
  BridgeReply removeEffect(String compId, String layerId, String effectId) {
    ops.add('fxremove:$compId/$layerId/$effectId');
    return _snap();
  }

  @override
  BridgeReply setEffectEnabled(
      String compId, String layerId, String effectId, bool enabled) {
    ops.add('fxenabled:$compId/$layerId/$effectId=$enabled');
    return _snap();
  }

  @override
  BridgeReply setEffectParamScalar(String compId, String layerId,
      String effectId, String paramName, double value) {
    ops.add('fxscalar:$compId/$layerId/$effectId/$paramName=$value');
    return _snap();
  }

  @override
  BridgeReply setEffectParamColour(String compId, String layerId,
      String effectId, String paramName, double r, double g, double b, double a) {
    ops.add('fxcolour:$compId/$layerId/$effectId/$paramName=$r,$g,$b,$a');
    return _snap();
  }

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
  group('Effect controls — transform read-back', () {
    testWidgets('rows show read-back values from the snapshot', (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final app = AppStateStub(bridge: _FakeBridge())..selectLayer('l0');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      // Position x/y read back their current values; Rotation reads 30.
      expect(find.text('123.0'), findsOneWidget);
      expect(find.text('45.0'), findsOneWidget);
      expect(find.text('30.0°'), findsOneWidget);
    });

    testWidgets('the stopwatch toggles animation at the playhead',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake)..selectLayer('l0');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      // Rotation is static; clicking its stopwatch animates it at frame 0.
      await tester.tap(find.byKey(const ValueKey('stopwatch-Rotation')));
      await tester.pump();
      expect(fake.ops, contains('toggle:c1/l0/rotation@0'));
    });

    testWidgets('the navigator diamond adds a key when off a key',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake)..selectLayer('l0');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      // The playhead sits at 0 — off the keys (10, 20) — so the diamond adds
      // a key on both animated axes at the playhead.
      await tester.tap(find.byKey(const ValueKey('kf-diamond')));
      await tester.pump();
      expect(fake.ops, contains('addkey:c1/l0/position_x@0=123.0'));
      expect(fake.ops, contains('addkey:c1/l0/position_y@0=45.0'));
    });

    testWidgets('the navigator diamond removes a key when on a key',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake)..selectLayer('l0');
      app.goToFrame(10); // sit exactly on the first Position key
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('kf-diamond')));
      await tester.pump();
      expect(fake.ops, contains('removekey:c1/l0/position_x@10'));
      expect(fake.ops, contains('removekey:c1/l0/position_y@10'));
    });
  });

  group('Effect controls — effect stack', () {
    testWidgets('renders the effect card with its parameters by kind',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final app = AppStateStub(bridge: _FakeBridge())..selectLayer('l0');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      // The effect's registry label and each parameter's derived label.
      expect(find.text('Gaussian blur'), findsOneWidget);
      expect(find.text('Radius'), findsOneWidget);
      expect(find.text('Tint'), findsOneWidget);
      expect(find.text('Mode'), findsOneWidget);
      expect(find.text('Invert'), findsOneWidget);
      // The scalar has an editable value box; the read-only kinds show values.
      expect(find.byKey(const ValueKey('fxparam-e1-radius')), findsOneWidget);
      expect(find.text('2'), findsOneWidget); // enum value, read-only
      expect(find.text('On'), findsOneWidget); // bool value, read-only
    });

    testWidgets('editing the scalar commits setEffectParamScalar',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake)..selectLayer('l0');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('fxparam-e1-radius')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText), '9');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(fake.ops, contains('fxscalar:c1/l0/e1/radius=9.0'));
    });

    testWidgets('the enable checkbox and remove button call their ops',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(420, 700));
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake)..selectLayer('l0');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pump();

      await tester.tap(find.byType(HouseCheckbox));
      await tester.pump();
      expect(fake.ops, contains('fxenabled:c1/l0/e1=false'));

      await tester.tap(find.text('×'));
      await tester.pump();
      expect(fake.ops, contains('fxremove:c1/l0/e1'));
    });
  });

  group('Effects & presets panel', () {
    testWidgets('search filters the registry by label', (tester) async {
      await tester.binding.setSurfaceSize(const Size(300, 500));
      final app = AppStateStub(bridge: _FakeBridge())..selectLayer('l0');
      await tester.pumpWidget(_host(EffectsPresetsPanel(app: app)));
      await tester.pump();

      // All three effects list before filtering.
      expect(find.text('Gaussian blur'), findsOneWidget);
      expect(find.text('Glow'), findsOneWidget);

      await tester.enterText(find.byType(EditableText), 'glo');
      await tester.pump();
      expect(find.text('Glow'), findsOneWidget);
      expect(find.text('Gaussian blur'), findsNothing);
    });

    testWidgets('double-clicking a row applies the effect to the layer',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(300, 500));
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake)..selectLayer('l0');
      await tester.pumpWidget(_host(EffectsPresetsPanel(app: app)));
      await tester.pump();

      // Double-click the Glow row: two taps within the double-tap window, then
      // settle past it so the recognizer fires.
      final centre = tester.getCenter(find.text('Glow'));
      await tester.tapAt(centre);
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tapAt(centre);
      await tester.pump(const Duration(milliseconds: 400));
      expect(fake.ops, contains('addeffect:c1/l0/glow'));
    });

    testWidgets('the hovered row Add button applies the effect', (tester) async {
      await tester.binding.setSurfaceSize(const Size(300, 500));
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake)..selectLayer('l0');
      await tester.pumpWidget(_host(EffectsPresetsPanel(app: app)));
      await tester.pump();

      // Hover the Glow row to reveal its Add button, then click it. The row's
      // double-tap recognizer contends with the button's tap in the same arena,
      // so settle past the double-tap window for the button's tap to resolve.
      final gesture =
          await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);
      await gesture.moveTo(tester.getCenter(find.text('Glow')));
      await tester.pump();

      await tester.tap(find.text('Add'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(fake.ops, contains('addeffect:c1/l0/glow'));
    });

    testWidgets('no selected layer shows a quiet hint', (tester) async {
      await tester.binding.setSurfaceSize(const Size(300, 500));
      final app = AppStateStub(bridge: _FakeBridge()); // nothing selected
      await tester.pumpWidget(_host(EffectsPresetsPanel(app: app)));
      await tester.pump();
      expect(find.textContaining('Select a layer'), findsOneWidget);
    });
  });
}
