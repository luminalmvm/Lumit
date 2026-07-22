// The final UI wave (bridge v0.9): the Dart-side behaviours the v0.9 snapshot
// surface unblocks — asset read-back (text/solid/camera), the Shape-tool mask
// geometry commit, the Auto resolution tier + live playback tier, the `.lumfx`
// preset save/load file flow, and the overrun HOLD-hatch maths. All driven
// through a fake bridge (never the real library) so the suite stays hermetic.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/effect_controls_panel.dart';
import 'package:lumit_flutter/panels/effects_presets_panel.dart';
import 'package:lumit_flutter/panels/timeline/graph_maths.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

const _switches = BridgeSwitches(
  visible: true,
  audible: true,
  locked: false,
  threeD: false,
  collapse: false,
  fx: true,
  solo: false,
  motionBlur: false,
);

BridgeLayer _layer({
  required String id,
  required BridgeLayerKind kind,
  BridgeTextDocument? text,
  List<int>? solidSize,
  BridgeTransformProperty? cameraZoom,
  List<double>? colour,
  List<BridgeEffect> effects = const [],
}) =>
    BridgeLayer(
      id: id,
      index: 0,
      name: id,
      kind: kind,
      inFrame: 0,
      outFrame: 300,
      label: 0,
      switches: _switches,
      text: text,
      solidSize: solidSize,
      cameraZoom: cameraZoom,
      colour: colour,
      effects: effects,
    );

/// The theme/overlay host the widget tests pump panels into.
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

BridgeSnapshot _snapWith(List<BridgeLayer> layers,
        {List<BridgeMarker> markerDetails = const []}) =>
    BridgeSnapshot(
      items: [
        BridgeItem(
          id: 'comp1',
          name: 'Comp',
          kind: BridgeItemKind.composition,
          children: const [],
          comp: BridgeComp(
            width: 1920,
            height: 1080,
            fps: const BridgeFps(30, 1),
            frameCount: 300,
            layers: layers,
            markers: [for (final m in markerDetails) m.frame],
            markerDetails: markerDetails,
          ),
        ),
      ],
      canUndo: false,
      canRedo: false,
      path: null,
    );

/// A fake carrying a settable snapshot, recording ops, and offering the preset
/// JSON so the save-to-file flow is testable.
class _WaveFake implements DocumentBridge, EditOpsBridge, PresetJsonBridge {
  _WaveFake(this._snapshot);

  final BridgeSnapshot _snapshot;
  final List<String> ops = [];
  String presetJson = '{"format":1,"name":"x","effects":[]}';
  BridgePlaybackTier tier = const BridgePlaybackTier(tier: 3, scale: 1 / 3);

  BridgeReply _ok(String record) {
    ops.add(record);
    return BridgeReply.ok(_snapshot);
  }

  @override
  BridgeReply snapshot() => BridgeReply.ok(_snapshot);

  // The registry reads return lists, not replies — noSuchMethod cannot cover them.
  @override
  List<BridgeEffectInfo> listEffects() => const [];
  @override
  List<BridgeBlendMode> listBlendModes() => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => BridgeReply.ok(_snapshot);

  @override
  BridgeReply addMaskGeometry(
          String c, String l, String k, double x, double y, double w, double h) =>
      _ok('mask_geom:$c:$l:$k:$x,$y,$w,$h');

  @override
  BridgeReply toggleEffectParamAnimated(
          String c, String l, String e, String p, int ch, int f) =>
      _ok('fx_toggle:$e:$p:$ch@$f');

  @override
  BridgeReply loadEffectPreset(String c, String l, String t) =>
      _ok('load_preset:$c:$l:$t');

  @override
  BridgeReply saveEffectPreset(String c, String l, String n) => _ok('save:$c:$l');

  @override
  String? saveEffectPresetJson(String c, String l, String n) {
    ops.add('save_json:$c:$l:$n');
    return presetJson;
  }

  @override
  BridgePlaybackTier playbackTier() => tier;
  @override
  BridgePlaybackTier resetRealtime() => BridgePlaybackTier.full;
}

void main() {
  group('asset editors adopt the v0.9 read-back', () {
    test('text/solid/camera seed from the snapshot, not the session map', () {
      final fake = _WaveFake(_snapWith([
        _layer(
          id: 't1',
          kind: BridgeLayerKind.text,
          text: const BridgeTextDocument(
              content: 'Hello', size: 96, fill: [1, 0, 0, 1]),
        ),
        _layer(
          id: 's1',
          kind: BridgeLayerKind.solid,
          solidSize: const [640, 480],
          colour: const [0.5, 0.5, 0.5, 1],
        ),
        _layer(
          id: 'cam1',
          kind: BridgeLayerKind.camera,
          cameraZoom:
              const BridgeTransformProperty(value: 1500, animated: false, keys: []),
        ),
      ]));
      final app = AppStateStub(bridge: fake);

      final text = app.textContentFor('t1');
      expect(text.text, 'Hello');
      expect(text.size, 96);
      expect(text.rgba[0], 1);

      final size = app.solidSizeFor('s1');
      expect(size.width, 640);
      expect(size.height, 480);

      expect(app.cameraZoomFor('cam1'), 1500);
    });

    test('an older library (no read-back) falls back to the session default', () {
      final fake = _WaveFake(_snapWith([
        _layer(id: 't1', kind: BridgeLayerKind.text), // no text read-back
      ]));
      final app = AppStateStub(bridge: fake);
      // Falls back to the unedited default (empty, 72 pt).
      expect(app.textContentFor('t1').size, 72);
    });
  });

  group('Shape-tool mask geometry commit', () {
    test('drawShapeMask sends the dragged rect through add_mask_geometry', () {
      final fake = _WaveFake(_snapWith([
        _layer(id: 'l1', kind: BridgeLayerKind.solid),
      ]));
      final app = AppStateStub(bridge: fake)..selectLayer('l1');
      app.viewerShape = ShapeKind.ellipse;
      app.drawShapeMask(10, 20, 100, 50);
      expect(fake.ops, contains('mask_geom:comp1:l1:ellipse:10.0,20.0,100.0,50.0'));
      expect(app.errorNotice, isNull);
    });

    test('no selected layer surfaces a calm error, not a crash', () {
      final fake = _WaveFake(_snapWith([]));
      final app = AppStateStub(bridge: fake);
      app.drawShapeMask(0, 0, 10, 10);
      expect(app.errorNotice, contains('select a layer'));
    });
  });

  group('Auto resolution tier', () {
    test('Auto follows the live playback tier; a manual pick overrides', () {
      final fake = _WaveFake(_snapWith([]));
      final app = AppStateStub(bridge: fake);

      // Manual default: Full scale.
      expect(app.previewAutoRes, isFalse);
      expect(app.effectivePreviewScale, 1.0);

      // Switch to Auto: resets to Full until the first poll.
      app.setPreviewAuto();
      expect(app.previewAutoRes, isTrue);
      expect(app.effectivePreviewScale, 1.0);

      // While playing, a poll adopts the engine's live tier (Third → 1/3).
      app.playing = true;
      app.pollPlaybackTier();
      expect(app.autoTier.tier, 3);
      expect(app.effectivePreviewScale, closeTo(1 / 3, 1e-9));

      // A manual pick clears Auto and overrides.
      app.setPreviewScale(PreviewScale.half);
      expect(app.previewAutoRes, isFalse);
      expect(app.effectivePreviewScale, 0.5);
    });

    test('stopped, Auto reads back Full (no per-frame churn)', () {
      final fake = _WaveFake(_snapWith([]));
      final app = AppStateStub(bridge: fake)..setPreviewAuto();
      app.playing = false;
      app.pollPlaybackTier();
      expect(app.autoTier.tier, 1);
    });
  });

  group('.lumfx preset save/load file flow', () {
    test('save writes the bridge JSON to the picked path', () async {
      final dir = await Directory.systemTemp.createTemp('lumfx_test');
      final target = '${dir.path}${Platform.pathSeparator}hero.lumfx';
      final fake = _WaveFake(_snapWith([
        _layer(id: 'l1', kind: BridgeLayerKind.solid),
      ]))
        ..presetJson = '{"format":1,"name":"hero","effects":[]}';
      final app = AppStateStub(
        bridge: fake,
        presetSaveLocationPicker: (name) async => target,
      )..selectLayer('l1');

      await app.saveSelectedEffectPreset();
      expect(File(target).existsSync(), isTrue);
      expect(await File(target).readAsString(), contains('"name":"hero"'));
      expect(app.notice, contains('preset saved'));
      await dir.delete(recursive: true);
    });

    test('load reads the picked file and appends via load_effect_preset', () async {
      final dir = await Directory.systemTemp.createTemp('lumfx_test');
      final source = '${dir.path}${Platform.pathSeparator}in.lumfx';
      await File(source).writeAsString('{"format":1,"name":"in","effects":[]}');
      final fake = _WaveFake(_snapWith([
        _layer(id: 'l1', kind: BridgeLayerKind.solid),
      ]));
      final app = AppStateStub(
        bridge: fake,
        presetOpenPicker: () async => source,
      )..selectLayer('l1');

      await app.loadPresetOntoSelected();
      expect(
          fake.ops.any((o) => o.startsWith('load_preset:comp1:l1:')), isTrue);
      await dir.delete(recursive: true);
    });

    test('save with no selected layer is a calm error', () async {
      final fake = _WaveFake(_snapWith([]));
      final app = AppStateStub(bridge: fake);
      await app.saveSelectedEffectPreset();
      expect(app.errorNotice, contains('select a layer'));
    });
  });

  group('overrun HOLD-hatch maths (speed_rows.rs port)', () {
    // A 2× constant-speed retime over a 4 s layer needs 8 s of source; a 4 s
    // source therefore runs out halfway (local 2 s).
    BridgeRetime doubleRetime() => const BridgeRetime(
          reverse: false,
          interpolation: 'nearest',
          boundaries: [
            BridgeRetimeBoundary(tFrame: 0, tSeconds: 0, sSeconds: 0, smooth: false),
            BridgeRetimeBoundary(
                tFrame: 120, tSeconds: 4, sSeconds: 8, smooth: false),
          ],
          segments: [BridgeRetimeSegment(kind: 'rate', v0: 2, v1: 2, ease: 'Linear')],
        );

    test('source position maps local time through the rate segment', () {
      final r = doubleRetime();
      expect(sourceSecsAtLocal(r, 0), closeTo(0, 1e-6));
      expect(sourceSecsAtLocal(r, 2), closeTo(4, 1e-6));
      expect(sourceSecsAtLocal(r, 4), closeTo(8, 1e-6));
    });

    test('overrun local time is where the source is exhausted', () {
      final r = doubleRetime();
      // 4 s of source at 2× runs out at local 2 s.
      expect(overrunLocalTime(r, 4)!, closeTo(2, 1e-3));
      // 10 s of source never runs out over this 8-s demand.
      expect(overrunLocalTime(r, 10), isNull);
    });

    test('overrun span clamps to the in point and stops at the out point', () {
      final r = doubleRetime();
      // start_offset 0, in 0 s, out 4 s: the held span is [2 s, 4 s].
      final span = overrunSpanSecs(r, 4, 0, 0, 4);
      expect(span, isNotNull);
      expect(span!.$1, closeTo(2, 1e-3));
      expect(span.$2, closeTo(4, 1e-3));
      // A source that lasts the whole demand → no overrun span.
      expect(overrunSpanSecs(r, 10, 0, 0, 4), isNull);
    });
  });

  group('effect-param animation (Effect controls)', () {
    testWidgets('an animatable param shows a stopwatch that toggles animation',
        (tester) async {
      final fake = _WaveFake(_snapWith([
        _layer(
          id: 'l1',
          kind: BridgeLayerKind.solid,
          effects: const [
            BridgeEffect(
              id: 'e1',
              name: 'blur',
              enabled: true,
              params: [
                BridgeEffectParam(name: 'radius', kind: 'scalar', value: 5.0),
              ],
            ),
          ],
        ),
      ]));
      final app = AppStateStub(bridge: fake)..selectLayer('l1');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));

      final stopwatch = find.byKey(const ValueKey('fxstopwatch-e1-radius'));
      expect(stopwatch, findsOneWidget);
      await tester.tap(stopwatch);
      await tester.pump();
      expect(fake.ops, contains('fx_toggle:e1:radius:0@0'));
    });
  });

  group('.lumfx preset actions (Effects & presets)', () {
    testWidgets('Save/Load preset buttons drive the preset flows',
        (tester) async {
      var saved = false;
      var loaded = false;
      final fake = _WaveFake(_snapWith([
        _layer(id: 'l1', kind: BridgeLayerKind.solid),
      ]));
      final app = AppStateStub(
        bridge: fake,
        presetSaveLocationPicker: (_) async {
          saved = true;
          return null; // cancel after the picker opens (no disk write)
        },
        presetOpenPicker: () async {
          loaded = true;
          return null;
        },
      )..selectLayer('l1');
      await tester.pumpWidget(_host(EffectsPresetsPanel(app: app)));

      await tester.tap(find.byKey(const ValueKey('preset-save')));
      await tester.pump();
      expect(saved, isTrue);

      await tester.tap(find.byKey(const ValueKey('preset-load')));
      await tester.pump();
      expect(loaded, isTrue);
    });
  });
}
