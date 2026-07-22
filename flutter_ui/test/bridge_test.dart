// Bridge v0 Dart-side tests: the JSON → typed-model parsing (fed literal
// strings, no library needed), and the guarantee that AppStateStub without a
// bridge behaves exactly as the F0 placeholder did.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/state/app_state.dart';

/// A minimal in-memory [DocumentBridge] for the AppStateStub tests: it mirrors
/// the engine's shapes (ok snapshots, a calm error for a bad import path) so the
/// dialogue-wiring logic can be exercised without the library or plugin
/// channels. It also records what it was asked to do.
class _FakeBridge implements DocumentBridge {
  final List<BridgeItem> items = [];
  String? path;

  // Call records the tests assert on.
  final List<String> imported = [];
  int saveCalls = 0;
  String? lastSavePath;

  // Snapshot-v2 op records.
  final List<String> ops = [];

  /// When set, the next op returns this error instead of a snapshot.
  String? nextOpError;

  /// What [decodeFrame] should return (null by default).
  DecodedFrame? decodeResult;
  final List<String> decoded = [];

  BridgeSnapshot _snap() => BridgeSnapshot(
        items: List.of(items),
        canUndo: items.isNotEmpty,
        canRedo: false,
        path: path,
      );

  @override
  BridgeReply snapshot() => BridgeReply.ok(_snap());

  @override
  BridgeReply newProject() {
    items.clear();
    path = null;
    return BridgeReply.ok(_snap());
  }

  @override
  BridgeReply undo() => BridgeReply.ok(_snap());

  @override
  BridgeReply redo() => BridgeReply.ok(_snap());

  @override
  BridgeReply openProject(String p) {
    path = p;
    return BridgeReply.ok(_snap());
  }

  @override
  BridgeReply saveProject(String p) {
    saveCalls++;
    lastSavePath = p;
    if (p.isNotEmpty) path = p;
    if (path == null) {
      return const BridgeReply.err('save project: no path yet');
    }
    return BridgeReply.ok(_snap());
  }

  @override
  BridgeReply newComposition(String name) {
    items.add(BridgeItem(
      id: 'c${items.length}',
      name: name.isEmpty ? 'Comp ${items.length + 1}' : name,
      kind: BridgeItemKind.composition,
      children: const [],
    ));
    return BridgeReply.ok(_snap());
  }

  @override
  BridgeReply importFootage(String p) {
    if (p.isEmpty) return const BridgeReply.err('import footage: no path given');
    imported.add(p);
    items.add(BridgeItem(
      id: 'f${items.length}',
      name: p,
      kind: BridgeItemKind.footage,
      children: const [],
    ));
    return BridgeReply.ok(_snap());
  }

  BridgeReply _op(String record) {
    ops.add(record);
    final err = nextOpError;
    if (err != null) {
      nextOpError = null;
      return BridgeReply.err(err);
    }
    return BridgeReply.ok(_snap());
  }

  @override
  BridgeReply setLayerSwitch(
          String compId, String layerId, String switchName, bool value) =>
      _op('switch:$compId/$layerId/$switchName=$value');

  @override
  BridgeReply editLayerSpan(
          String compId, String layerId, String edit, int frame) =>
      _op('span:$compId/$layerId/$edit@$frame');

  @override
  BridgeReply setTransform(
          String compId, String layerId, String property, double value) =>
      _op('transform:$compId/$layerId/$property=$value');

  @override
  BridgeReply addMarker(String compId, int frame) =>
      _op('marker:$compId@$frame');

  @override
  BridgeReply addSolidLayer(String compId) => _op('add_solid:$compId');
  @override
  BridgeReply addTextLayer(String compId) => _op('add_text:$compId');
  @override
  BridgeReply addCameraLayer(String compId) => _op('add_camera:$compId');
  @override
  BridgeReply addAdjustmentLayer(String compId) => _op('add_adjustment:$compId');
  @override
  BridgeReply addSequenceLayer(String compId) => _op('add_sequence:$compId');
  @override
  BridgeReply addFootageLayer(String compId, String itemId) =>
      _op('add_footage:$compId/$itemId');
  @override
  BridgeReply reorderLayer(String compId, String layerId, int newIndex) =>
      _op('reorder:$compId/$layerId->$newIndex');

  @override
  BridgeReply deleteLayer(String compId, String layerId) =>
      _op('delete_layer:$compId/$layerId');
  @override
  BridgeReply duplicateLayer(String compId, String layerId) =>
      _op('duplicate_layer:$compId/$layerId');

  @override
  BridgeReply setCompSettings(String compId, String name, int width, int height,
          int fpsNum, int fpsDen, int durationFrames) =>
      _op('comp_settings:$compId/$name/${width}x$height@$fpsNum/$fpsDen'
          '#$durationFrames');

  @override
  BridgeReply togglePropertyAnimated(
          String compId, String layerId, String property, int frame) =>
      _op('stopwatch:$compId/$layerId/$property@$frame');

  @override
  BridgeReply addKeyframe(String compId, String layerId, String property,
          int frame, double value) =>
      _op('add_key:$compId/$layerId/$property@$frame=$value');

  @override
  BridgeReply removeKeyframe(
          String compId, String layerId, String property, int frame) =>
      _op('remove_key:$compId/$layerId/$property@$frame');

  @override
  BridgeReply shiftKeyframes(String compId, String layerId, String property,
          List<int> frames, int delta) =>
      _op('shift_keys:$compId/$layerId/$property/$frames+$delta');

  @override
  BridgeReply setWorkAreaEdge(String compId, int frame, bool isOut) =>
      _op('work_area:$compId@$frame/out=$isOut');

  @override
  List<BridgeEffectInfo> listEffects() =>
      const [BridgeEffectInfo(name: 'blur', label: 'Blur')];

  @override
  BridgeReply addEffect(String compId, String layerId, String effectName) =>
      _op('add_effect:$compId/$layerId/$effectName');
  @override
  BridgeReply removeEffect(String compId, String layerId, String effectId) =>
      _op('remove_effect:$compId/$layerId/$effectId');
  @override
  BridgeReply setEffectEnabled(
          String compId, String layerId, String effectId, bool enabled) =>
      _op('effect_enabled:$compId/$layerId/$effectId=$enabled');
  @override
  BridgeReply setEffectParamScalar(String compId, String layerId,
          String effectId, String paramName, double value) =>
      _op('effect_scalar:$compId/$layerId/$effectId/$paramName=$value');
  @override
  BridgeReply setEffectParamColour(String compId, String layerId,
          String effectId, String paramName, double r, double g, double b,
          double a) =>
      _op('effect_colour:$compId/$layerId/$effectId/$paramName=$r,$g,$b,$a');

  // Bridge v0.4 stubs (record the op like the others; typed reads return idle).
  @override
  BridgeReply setKeyframeInterp(String compId, String layerId, String property,
          int frame, String interpIn, String interpOut, double speedIn,
          double influenceIn, double speedOut, double influenceOut) =>
      _op('kfinterp:$compId/$layerId/$property@$frame=$interpIn/$interpOut');
  @override
  BridgeReply setRetimeEnabled(String compId, String layerId, bool enabled) =>
      _op('retime_enabled:$compId/$layerId=$enabled');
  @override
  BridgeReply setRetimeSpeed(String compId, String layerId, double speed) =>
      _op('retime_speed:$compId/$layerId=$speed');
  @override
  BridgeReply setSegmentPreset(
          String compId, String layerId, int frame, String ease) =>
      _op('segment_preset:$compId/$layerId@$frame=$ease');
  @override
  BridgeReply segmentToRate(String compId, String layerId, int frame) =>
      _op('segment_to_rate:$compId/$layerId@$frame');
  @override
  BridgeReply dragBoundary(
          String compId, String layerId, int index, int frame) =>
      _op('drag_boundary:$compId/$layerId/$index@$frame');
  @override
  List<BridgeBlendMode> listBlendModes() => const [];
  @override
  BridgeReply setBlendMode(String compId, String layerId, String mode) =>
      _op('blend:$compId/$layerId=$mode');
  @override
  BridgeReply setMatte(String compId, String layerId, String source,
          String channel, bool inverted) =>
      _op('matte:$compId/$layerId=$source/$channel/$inverted');
  @override
  BridgeReply setParent(String compId, String layerId, String parent) =>
      _op('parent:$compId/$layerId=$parent');
  @override
  BridgeReply setMotionBlur(String compId, bool enabled, double shutterAngle,
          double shutterPhase, int samples) =>
      _op('motion_blur:$compId=$enabled/$shutterAngle/$shutterPhase/$samples');
  @override
  BridgeReply addMask(String compId, String layerId, String kind) =>
      _op('mask:$compId/$layerId=$kind');
  @override
  BridgeExportPreset exportPreset(
          String presetName, String compName, String template) =>
      BridgeExportPreset.idle;
  @override
  BridgeReply startExport(String compId, String specJson, String outPath) =>
      _op('start_export:$compId->$outPath');
  @override
  BridgeExportState exportPoll() => BridgeExportState.idle;
  @override
  BridgeReply exportCancel() => _op('export_cancel');

  @override
  DecodedFrame? decodeFrame(String itemId, int frame) {
    decoded.add('$itemId@$frame');
    return decodeResult;
  }
}

void main() {
  group('BridgeSnapshot parsing', () {
    test('an empty document parses to no items and no undo', () {
      final reply = BridgeReply.parse(
        '{"ok":true,"items":[],"can_undo":false,"can_redo":false,"path":null}',
      );
      expect(reply.ok, isTrue);
      final snap = reply.snapshot!;
      expect(snap.items, isEmpty);
      expect(snap.canUndo, isFalse);
      expect(snap.canRedo, isFalse);
      expect(snap.path, isNull);
    });

    test('a nested folder tree parses with kinds and children', () {
      const json = '''
      {
        "ok": true,
        "items": [
          {
            "id": "f1", "name": "Compositions", "kind": "folder",
            "children": [
              {"id": "c1", "name": "Intro", "kind": "composition", "children": []}
            ]
          },
          {"id": "a1", "name": "clip.mp4", "kind": "footage", "children": []},
          {"id": "s1", "name": "White solid", "kind": "solid", "children": []}
        ],
        "can_undo": true, "can_redo": false, "path": "C:/edit.lum"
      }''';
      final reply = BridgeReply.parse(json);
      expect(reply.ok, isTrue);
      final snap = reply.snapshot!;
      expect(snap.canUndo, isTrue);
      expect(snap.path, 'C:/edit.lum');
      expect(snap.items.length, 3);

      final folder = snap.items[0];
      expect(folder.kind, BridgeItemKind.folder);
      expect(folder.name, 'Compositions');
      expect(folder.children.length, 1);
      expect(folder.children[0].kind, BridgeItemKind.composition);
      expect(folder.children[0].name, 'Intro');

      expect(snap.items[1].kind, BridgeItemKind.footage);
      expect(snap.items[2].kind, BridgeItemKind.solid);
    });

    test('an unknown kind degrades rather than throwing', () {
      final reply = BridgeReply.parse(
        '{"ok":true,"items":[{"id":"x","name":"?","kind":"nebula","children":[]}],'
        '"can_undo":false,"can_redo":false,"path":null}',
      );
      expect(reply.ok, isTrue);
      expect(reply.snapshot!.items.single.kind, BridgeItemKind.unknown);
    });

    test('an error reply carries the message, not a snapshot', () {
      final reply = BridgeReply.parse(
        '{"ok":false,"error":"open project: not a Lumit project"}',
      );
      expect(reply.ok, isFalse);
      expect(reply.snapshot, isNull);
      expect(reply.error, 'open project: not a Lumit project');
    });

    test('malformed JSON is reported, never thrown', () {
      final reply = BridgeReply.parse('not json at all');
      expect(reply.ok, isFalse);
      expect(reply.error, contains('malformed'));
    });
  });

  group('AppStateStub without a bridge', () {
    test('bridge is null and no snapshot is held', () {
      final app = AppStateStub();
      expect(app.bridge, isNull);
      expect(app.snapshot, isNull);
    });

    test('document actions keep the exact F0 notice text', () {
      // Each action must produce the same notice as the original
      // `engine('…')` call did, so the placeholder build is unchanged. A fresh
      // instance per action keeps the notices from bleeding together.
      var app = AppStateStub()..newProject();
      expect(app.notice, 'New project — engine bridge arrives in phase F1');

      app = AppStateStub()..newComposition();
      expect(app.notice, 'New composition — engine bridge arrives in phase F1');

      app = AppStateStub()..undo();
      expect(app.notice, 'Undo — engine bridge arrives in phase F1');

      app = AppStateStub()..redo();
      expect(app.notice, 'Redo — engine bridge arrives in phase F1');

      app = AppStateStub()..save();
      expect(app.notice, 'Save — engine bridge arrives in phase F1');

      app = AppStateStub()..openProject();
      expect(app.notice, 'Open project — engine bridge arrives in phase F1');

      app = AppStateStub()..importFootage();
      expect(app.notice, 'Import footage — engine bridge arrives in phase F1');
    });
  });

  group('AppStateStub file dialogues (fake bridge)', () {
    test('save with no path routes to the save-location seam', () async {
      final fake = _FakeBridge();
      var pickerCalled = false;
      final app = AppStateStub(
        bridge: fake,
        saveLocationPicker: () async {
          pickerCalled = true;
          return '/tmp/new.lum';
        },
      );
      await app.save();
      expect(pickerCalled, isTrue, reason: 'no path yet, so Save asks where');
      expect(fake.lastSavePath, '/tmp/new.lum');
      expect(app.snapshot!.path, '/tmp/new.lum');
      expect(app.notice, 'Project saved');
    });

    test('save with a known path saves in place, no dialogue', () async {
      final fake = _FakeBridge()..path = '/tmp/existing.lum';
      var pickerCalled = false;
      final app = AppStateStub(
        bridge: fake,
        saveLocationPicker: () async {
          pickerCalled = true;
          return null;
        },
      );
      await app.save();
      expect(pickerCalled, isFalse);
      expect(fake.saveCalls, 1);
      expect(fake.lastSavePath, '', reason: 'empty path = save in place');
    });

    test('cancelling the save dialogue changes nothing', () async {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake, saveLocationPicker: () async => null);
      await app.save();
      expect(fake.saveCalls, 0);
      expect(app.snapshot!.path, isNull);
    });

    test('importing N footage files posts a calm count', () async {
      final fake = _FakeBridge();
      final app = AppStateStub(
        bridge: fake,
        footagePicker: () async => ['/a/one.mp4', '/a/two.mov'],
      );
      await app.importFootage();
      expect(fake.imported, ['/a/one.mp4', '/a/two.mov']);
      expect(app.notice, '2 items imported');
      expect(app.errorNotice, isNull);
      expect(app.snapshot!.items.length, 2);
    });

    test('a single import reads as one item', () async {
      final fake = _FakeBridge();
      final app = AppStateStub(
        bridge: fake,
        footagePicker: () async => ['/a/clip.mp4'],
      );
      await app.importFootage();
      expect(app.notice, '1 item imported');
    });

    test('a partial import failure surfaces via the error tint', () async {
      final fake = _FakeBridge();
      final app = AppStateStub(
        bridge: fake,
        footagePicker: () async => ['', '/a/ok.mp4'],
      );
      await app.importFootage();
      expect(app.notice, '1 item imported');
      expect(app.errorNotice, 'import footage: no path given');
    });

    test('cancelling the footage dialogue changes nothing', () async {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake, footagePicker: () async => []);
      await app.importFootage();
      expect(fake.imported, isEmpty);
      expect(app.snapshot!.items, isEmpty);
    });

    test('opening a project remembers its path', () async {
      final fake = _FakeBridge();
      String? remembered;
      final app = AppStateStub(
        bridge: fake,
        openProjectPicker: () async => '/edit/project.lum',
        rememberProject: (p) => remembered = p,
      );
      await app.openProject();
      expect(app.snapshot!.path, '/edit/project.lum');
      expect(remembered, '/edit/project.lum');
      expect(app.notice, 'Project opened');
    });

    test('cancelling the open dialogue changes nothing', () async {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake, openProjectPicker: () async => null);
      await app.openProject();
      expect(app.snapshot!.path, isNull);
    });
  });

  group('AppStateStub last-project restore', () {
    test('a live bridge reopens the last project when its file exists', () {
      final file = File(
          '${Directory.systemTemp.path}${Platform.pathSeparator}restore-me.lum')
        ..writeAsStringSync('placeholder');
      addTearDown(() {
        if (file.existsSync()) file.deleteSync();
      });
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake, lastProjectPath: file.path);
      expect(app.snapshot!.path, file.path);
      expect(app.notice, 'Project reopened');
    });

    test('a missing last project degrades quietly, never a crash', () {
      final fake = _FakeBridge();
      final app = AppStateStub(
          bridge: fake, lastProjectPath: '/no/such/place/gone.lum');
      expect(app.snapshot!.path, isNull, reason: 'nothing was reopened');
    });
  });

  group('Snapshot v2 parsing', () {
    // A comp (two layers, switches, markers) plus a probed footage item, in the
    // exact shape the Rust bridge emits.
    const json = '''
    {
      "ok": true,
      "items": [
        {
          "id": "c1", "name": "Scene", "kind": "composition", "children": [],
          "comp": {
            "width": 1920, "height": 1080,
            "fps": {"num": 60, "den": 1}, "frame_count": 300,
            "layers": [
              {
                "id": "l0", "index": 0, "name": "top", "kind": "footage",
                "in_frame": 60, "out_frame": 240, "label": 2,
                "switches": {
                  "visible": true, "audible": true, "locked": false,
                  "three_d": false, "collapse": false, "fx": true,
                  "solo": true, "motion_blur": false
                }
              },
              {
                "id": "l1", "index": 1, "name": "bg", "kind": "solid",
                "in_frame": 0, "out_frame": 300, "label": 0,
                "switches": {
                  "visible": false, "audible": true, "locked": true,
                  "three_d": true, "collapse": false, "fx": true,
                  "solo": false, "motion_blur": true
                }
              }
            ],
            "markers": [120, 240]
          }
        },
        {
          "id": "f1", "name": "clip.mp4", "kind": "footage", "children": [],
          "status": "ok",
          "media": {
            "duration_frames": 150, "fps": {"num": 30000, "den": 1001},
            "width": 1280, "height": 720, "audio": true
          }
        }
      ],
      "can_undo": true, "can_redo": false, "path": null
    }''';

    test('a composition parses its size, rate, layers and markers', () {
      final snap = BridgeReply.parse(json).snapshot!;
      final comp = snap.items[0].comp!;
      expect(comp.width, 1920);
      expect(comp.height, 1080);
      expect(comp.fps.num, 60);
      expect(comp.fps.den, 1);
      expect(comp.fps.fps, 60.0);
      expect(comp.frameCount, 300);
      expect(comp.markers, [120, 240]);
      expect(comp.layers.length, 2);

      final top = comp.layers[0];
      expect(top.index, 0);
      expect(top.name, 'top');
      expect(top.kind, BridgeLayerKind.footage);
      expect(top.inFrame, 60);
      expect(top.outFrame, 240);
      expect(top.label, 2);
      expect(top.switches.solo, isTrue);
      expect(top.switches.visible, isTrue);

      final bg = comp.layers[1];
      expect(bg.kind, BridgeLayerKind.solid);
      expect(bg.switches.visible, isFalse);
      expect(bg.switches.locked, isTrue);
      expect(bg.switches.threeD, isTrue);
      expect(bg.switches.motionBlur, isTrue);
    });

    test('a footage item parses its status and media metadata', () {
      final snap = BridgeReply.parse(json).snapshot!;
      final footage = snap.items[1];
      expect(footage.kind, BridgeItemKind.footage);
      expect(footage.status, BridgeMediaStatus.ok);
      final media = footage.media!;
      expect(media.durationFrames, 150);
      expect(media.fps.num, 30000);
      expect(media.fps.den, 1001);
      expect(media.width, 1280);
      expect(media.height, 720);
      expect(media.audio, isTrue);
    });

    test('an unprobed footage item has a status but no media block', () {
      final snap = BridgeReply.parse(
        '{"ok":true,"items":[{"id":"f","name":"x.mp4","kind":"footage",'
        '"children":[],"status":"unprobed"}],'
        '"can_undo":false,"can_redo":false,"path":null}',
      ).snapshot!;
      expect(snap.items[0].status, BridgeMediaStatus.unprobed);
      expect(snap.items[0].media, isNull);
    });

    test('unknown layer kinds and statuses degrade rather than throwing', () {
      final snap = BridgeReply.parse(
        '{"ok":true,"items":[{"id":"c","name":"C","kind":"composition",'
        '"children":[],"comp":{"width":1,"height":1,"fps":{"num":1,"den":1},'
        '"frame_count":1,"layers":[{"id":"l","index":0,"name":"n",'
        '"kind":"nebula","in_frame":0,"out_frame":1,"label":0,"switches":{}}],'
        '"markers":[]}}],"can_undo":false,"can_redo":false,"path":null}',
      ).snapshot!;
      expect(snap.items[0].comp!.layers[0].kind, BridgeLayerKind.unknown);
      // Absent switch fields fall back to their model defaults.
      expect(snap.items[0].comp!.layers[0].switches.visible, isTrue);
      expect(snap.items[0].comp!.layers[0].switches.solo, isFalse);
    });
  });

  group('AppStateStub snapshot-v2 op pass-throughs (fake bridge)', () {
    test('frontComp resolves the first composition in the snapshot', () {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake);
      expect(app.frontComp, isNull, reason: 'no comp yet');
      // A snapshot carrying a comp makes frontComp resolve it.
      app.snapshot = BridgeReply.parse(
        '{"ok":true,"items":[{"id":"c1","name":"Scene","kind":"composition",'
        '"children":[],"comp":{"width":640,"height":480,"fps":{"num":24,'
        '"den":1},"frame_count":48,"layers":[],"markers":[]}}],'
        '"can_undo":false,"can_redo":false,"path":null}',
      ).snapshot;
      expect(app.frontComp, isNotNull);
      expect(app.frontComp!.width, 640);
      expect(app.frontComp!.fps.num, 24);
    });

    test('the ops route to the bridge and refresh the snapshot', () {
      final fake = _FakeBridge()..newComposition('Scene');
      final app = AppStateStub(bridge: fake);
      app.setLayerSwitch('c1', 'l0', 'solo', true);
      app.editLayerSpan('c1', 'l0', 'move_in', 120);
      app.setTransform('c1', 'l0', 'opacity', 42.0);
      app.addMarker('c1', 90);
      expect(fake.ops, [
        'switch:c1/l0/solo=true',
        'span:c1/l0/move_in@120',
        'transform:c1/l0/opacity=42.0',
        'marker:c1@90',
      ]);
      expect(app.snapshot, isNotNull);
      expect(app.errorNotice, isNull);
    });

    test('an op failure surfaces on the error tint, no snapshot change', () {
      final fake = _FakeBridge()..newComposition('Scene');
      final app = AppStateStub(bridge: fake);
      fake.nextOpError = 'set transform: unknown property';
      app.setTransform('c1', 'l0', 'wobble', 1.0);
      expect(app.errorNotice, 'set transform: unknown property');
    });

    test('the ops are quiet no-ops without a bridge', () {
      final app = AppStateStub();
      // None of these throw or touch a null bridge.
      app.setLayerSwitch('c', 'l', 'solo', true);
      app.editLayerSpan('c', 'l', 'trim_in', 0);
      app.setTransform('c', 'l', 'opacity', 1.0);
      app.addMarker('c', 0);
      expect(app.decodeFrame('f', 0), isNull);
      expect(app.errorNotice, isNull);
    });

    test('decodeFrame passes through to the bridge and returns its frame', () {
      final fake = _FakeBridge()
        ..decodeResult = DecodedFrame(
          width: 2,
          height: 1,
          rgba: Uint8List.fromList([1, 2, 3, 4, 5, 6, 7, 8]),
        );
      final app = AppStateStub(bridge: fake);
      final frame = app.decodeFrame('f1', 7);
      expect(fake.decoded, ['f1@7']);
      expect(frame, isNotNull);
      expect(frame!.width, 2);
      expect(frame.rgba.length, 8);
    });
  });

  group('Snapshot v3 parsing', () {
    // A comp with one footage layer carrying the transform read-back (opacity
    // keyframed, position static), its identity link, and an effect; plus the
    // comp's work area. The exact shape the Rust bridge v0.3 emits.
    const json = '''
    {
      "ok": true,
      "items": [
        {
          "id": "c1", "name": "Scene", "kind": "composition", "children": [],
          "comp": {
            "width": 1920, "height": 1080,
            "fps": {"num": 60, "den": 1}, "frame_count": 300,
            "work_area": [30, 121],
            "markers": [],
            "layers": [
              {
                "id": "l0", "index": 0, "name": "clip", "kind": "footage",
                "in_frame": 0, "out_frame": 300, "label": 0,
                "switches": {},
                "source_item_id": "item-7",
                "transform": {
                  "position_x": {"value": 960.0, "animated": false},
                  "opacity": {
                    "value": 100.0, "animated": true,
                    "keys": [
                      {"frame": 0, "value": 100.0,
                       "interp_in": "Linear", "interp_out": "Linear"},
                      {"frame": 60, "value": 0.0,
                       "interp_in": "Bezier", "interp_out": "Hold"}
                    ]
                  }
                },
                "effects": [
                  {
                    "id": "e1", "name": "blur", "enabled": true,
                    "params": [
                      {"name": "radius", "kind": "scalar", "value": 8.0},
                      {"name": "tint", "kind": "colour",
                       "value": [1.0, 0.0, 0.0, 1.0]}
                    ]
                  }
                ]
              }
            ]
          }
        }
      ],
      "can_undo": true, "can_redo": false, "path": null
    }''';

    test('the transform read-back parses values and keyframes', () {
      final layer = BridgeReply.parse(json).snapshot!.items[0].comp!.layers[0];
      final tr = layer.transform!;
      expect(tr['position_x']!.value, 960.0);
      expect(tr['position_x']!.animated, isFalse);
      final opacity = tr['opacity']!;
      expect(opacity.animated, isTrue);
      expect(opacity.keys.length, 2);
      expect(opacity.keys[0].frame, 0);
      expect(opacity.keys[0].interpIn, 'Linear');
      expect(opacity.keys[1].frame, 60);
      expect(opacity.keys[1].value, 0.0);
      expect(opacity.keys[1].interpIn, 'Bezier');
      expect(opacity.keys[1].interpOut, 'Hold');
    });

    test('identity links, effects and work area parse', () {
      final snap = BridgeReply.parse(json).snapshot!;
      final comp = snap.items[0].comp!;
      expect(comp.workArea, [30, 121]);
      final layer = comp.layers[0];
      expect(layer.sourceItemId, 'item-7');
      expect(layer.sourceCompId, isNull);
      expect(layer.effects.length, 1);
      final effect = layer.effects[0];
      expect(effect.id, 'e1');
      expect(effect.name, 'blur');
      expect(effect.enabled, isTrue);
      expect(effect.params[0].name, 'radius');
      expect(effect.params[0].kind, 'scalar');
      expect(effect.params[0].value, 8.0);
      expect(effect.params[1].kind, 'colour');
      expect(effect.params[1].value, [1.0, 0.0, 0.0, 1.0]);
    });

    test('a solid layer parses its colour; missing v3 fields degrade to null',
        () {
      final snap = BridgeReply.parse(
        '{"ok":true,"items":[{"id":"c","name":"C","kind":"composition",'
        '"children":[],"comp":{"width":1,"height":1,"fps":{"num":1,"den":1},'
        '"frame_count":1,"work_area":null,"markers":[],"layers":[{"id":"l",'
        '"index":0,"name":"n","kind":"solid","in_frame":0,"out_frame":1,'
        '"label":0,"switches":{},"colour":[0.5,0.25,0.75,1.0]}]}}],'
        '"can_undo":false,"can_redo":false,"path":null}',
      ).snapshot!;
      final comp = snap.items[0].comp!;
      expect(comp.workArea, isNull);
      final layer = comp.layers[0];
      expect(layer.colour, [0.5, 0.25, 0.75, 1.0]);
      // No transform/effects fields present: transform is null, effects empty.
      expect(layer.transform, isNull);
      expect(layer.effects, isEmpty);
    });
  });

  group('AppStateStub v3 op pass-throughs (fake bridge)', () {
    test('the lifecycle/keyframe/effect ops route to the bridge', () {
      final fake = _FakeBridge()..newComposition('Scene');
      final app = AppStateStub(bridge: fake);
      app.addSolidLayer('c1');
      app.addTextLayer('c1');
      app.deleteLayer('c1', 'l0');
      app.duplicateLayer('c1', 'l0');
      app.setCompSettings('c1', 'Retitled', 1280, 720, 24, 1, 48);
      app.togglePropertyAnimated('c1', 'l0', 'opacity', 30);
      app.addKeyframe('c1', 'l0', 'rotation', 60, 90.0);
      app.removeKeyframe('c1', 'l0', 'rotation', 60);
      app.shiftKeyframes('c1', 'l0', 'rotation', [60], 30);
      app.setWorkAreaEdge('c1', 120, true);
      app.addEffect('c1', 'l0', 'blur');
      app.setEffectEnabled('c1', 'l0', 'e1', false);
      app.setEffectParamScalar('c1', 'l0', 'e1', 'radius', 8.0);
      app.setEffectParamColour('c1', 'l0', 'e1', 'tint', 1.0, 0.0, 0.0, 1.0);
      app.removeEffect('c1', 'l0', 'e1');
      expect(fake.ops, [
        'add_solid:c1',
        'add_text:c1',
        'delete_layer:c1/l0',
        'duplicate_layer:c1/l0',
        'comp_settings:c1/Retitled/1280x720@24/1#48',
        'stopwatch:c1/l0/opacity@30',
        'add_key:c1/l0/rotation@60=90.0',
        'remove_key:c1/l0/rotation@60',
        'shift_keys:c1/l0/rotation/[60]+30',
        'work_area:c1@120/out=true',
        'add_effect:c1/l0/blur',
        'effect_enabled:c1/l0/e1=false',
        'effect_scalar:c1/l0/e1/radius=8.0',
        'effect_colour:c1/l0/e1/tint=1.0,0.0,0.0,1.0',
        'remove_effect:c1/l0/e1',
      ]);
      expect(app.errorNotice, isNull);
    });

    test('addFootageLayer and reorderLayer route to the bridge', () {
      final fake = _FakeBridge()..newComposition('Scene');
      final app = AppStateStub(bridge: fake);
      app.addFootageLayer('c1', 'f7');
      app.reorderLayer('c1', 'l2', 0);
      expect(fake.ops, [
        'add_footage:c1/f7',
        'reorder:c1/l2->0',
      ]);
      expect(app.errorNotice, isNull);
    });

    test('addFootageToFrontComp places into the front comp', () {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake);
      // Seed a snapshot whose composition carries a comp block so the front comp
      // resolves (the bare fake's newComposition omits it).
      app.snapshot = BridgeReply.parse(
        '{"ok":true,"items":[{"id":"c1","name":"Scene",'
        '"kind":"composition","children":[],"comp":{"width":1920,'
        '"height":1080,"fps":{"num":60,"den":1},"frame_count":100,'
        '"layers":[],"markers":[]}}],"can_undo":false,"can_redo":false,'
        '"path":null}',
      ).snapshot;
      app.addFootageToFrontComp('f9');
      expect(fake.ops, ['add_footage:c1/f9']);
    });

    test('addFootageToFrontComp with no comp surfaces a calm notice', () {
      final fake = _FakeBridge(); // no composition
      final app = AppStateStub(bridge: fake);
      app.addFootageToFrontComp('f9');
      expect(fake.ops, isEmpty);
      expect(app.notice, contains('Open a composition'));
    });

    test('listEffects passes through to the bridge', () {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake);
      final effects = app.listEffects();
      expect(effects.length, 1);
      expect(effects.single.name, 'blur');
      expect(effects.single.label, 'Blur');
    });

    test('a v3 op failure surfaces on the error tint', () {
      final fake = _FakeBridge()..newComposition('Scene');
      final app = AppStateStub(bridge: fake);
      fake.nextOpError = 'add effect: unknown effect';
      app.addEffect('c1', 'l0', 'nope');
      expect(app.errorNotice, 'add effect: unknown effect');
    });

    test('the v3 ops are quiet no-ops without a bridge', () {
      final app = AppStateStub();
      app.addSolidLayer('c');
      app.deleteLayer('c', 'l');
      app.addKeyframe('c', 'l', 'opacity', 0, 1.0);
      app.setWorkAreaEdge('c', 0, false);
      app.addEffect('c', 'l', 'blur');
      expect(app.listEffects(), isEmpty);
      expect(app.errorNotice, isNull);
    });

    test('transformValueFor reads the snapshot, then the session map', () {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake);
      // No snapshot layer yet: falls back to the session edit map (null first).
      expect(app.transformValueFor('l0', 'opacity'), isNull);
      app.setTransform('c1', 'l0', 'opacity', 42.0);
      expect(app.transformValueFor('l0', 'opacity'), 42.0);
      // A snapshot read-back wins over the session map.
      app.snapshot = BridgeReply.parse(
        '{"ok":true,"items":[{"id":"c1","name":"S","kind":"composition",'
        '"children":[],"comp":{"width":1,"height":1,"fps":{"num":1,"den":1},'
        '"frame_count":1,"work_area":null,"markers":[],"layers":[{"id":"l0",'
        '"index":0,"name":"n","kind":"solid","in_frame":0,"out_frame":1,'
        '"label":0,"switches":{},"transform":{"opacity":{"value":73.0,'
        '"animated":false}}}]}}],"can_undo":false,"can_redo":false,'
        '"path":null}',
      ).snapshot;
      expect(app.transformValueFor('l0', 'opacity'), 73.0);
    });
  });

  group('Bridge v0.4 parsing', () {
    test('a keyframe parses its Bezier tangents per side', () {
      final k = BridgeKeyframe.fromJson({
        'frame': 12,
        'value': 3.0,
        'interp_in': 'Bezier',
        'interp_out': 'Linear',
        'bezier_in': {'speed': 2.0, 'influence': 0.5},
      });
      expect(k.interpIn, 'Bezier');
      expect(k.bezierIn, isNotNull);
      expect(k.bezierIn!.speed, 2.0);
      expect(k.bezierIn!.influence, 0.5);
      expect(k.bezierOut, isNull); // a Linear side carries no tangent
    });

    test('a layer parses blend mode, matte, parent and retime', () {
      final layer = BridgeLayer.fromJson({
        'id': 'l',
        'index': 0,
        'name': 'clip',
        'kind': 'footage',
        'in_frame': 0,
        'out_frame': 300,
        'label': 0,
        'switches': {},
        'blend_mode': 'Multiply',
        'parent': 'p',
        'matte': {
          'source': 's',
          'channel': 'luma',
          'inverted': true,
          'source_mode': 'masks',
        },
        'retime': {
          'reverse': false,
          'interpolation': 'nearest',
          'boundaries': [
            {'t_frame': 0, 't_seconds': 0.0, 's_seconds': 0.0, 'smooth': false},
            {'t_frame': 300, 't_seconds': 5.0, 's_seconds': 2.5, 'smooth': false},
          ],
          'segments': [
            {'kind': 'rate', 'v0': 0.5, 'v1': 0.5, 'ease': 'Linear'},
          ],
        },
      });
      expect(layer.blendMode, 'Multiply');
      expect(layer.parent, 'p');
      expect(layer.matte!.channel, 'luma');
      expect(layer.matte!.inverted, isTrue);
      expect(layer.matte!.sourceMode, 'masks');
      expect(layer.retime!.interpolation, 'nearest');
      expect(layer.retime!.boundaries.length, 2);
      expect(layer.retime!.boundaries[1].tFrame, 300);
      expect(layer.retime!.segments.single.kind, 'rate');
      expect(layer.retime!.segments.single.v0, 0.5);
      expect(layer.retime!.segments.single.ease, 'Linear');
    });

    test('a comp parses its motion-blur master', () {
      final comp = BridgeComp.fromJson({
        'width': 1920,
        'height': 1080,
        'fps': {'num': 60, 'den': 1},
        'frame_count': 300,
        'layers': [],
        'markers': [],
        'work_area': null,
        'motion_blur': {
          'enabled': true,
          'shutter_angle': 180.0,
          'shutter_phase': -90.0,
          'samples': 16,
        },
      });
      expect(comp.motionBlur!.enabled, isTrue);
      expect(comp.motionBlur!.angle, 180.0);
      expect(comp.motionBlur!.phase, -90.0);
      expect(comp.motionBlur!.samples, 16);
    });

    test('an export poll reply parses its state and progress', () {
      final running = BridgeExportState.fromJson({
        'state': 'running',
        'frame': 12,
        'total': 90,
        'encoder': 'software x264',
      });
      expect(running.isRunning, isTrue);
      expect(running.frame, 12);
      expect(running.total, 90);
      expect(running.encoder, 'software x264');
      final done = BridgeExportState.fromJson({'state': 'done', 'path': 'out.mp4'});
      expect(done.isDone, isTrue);
      expect(done.path, 'out.mp4');
    });

    test('an export preset reply parses its stamped fields', () {
      final p = BridgeExportPreset.fromJson({
        'preset': 'youtube_1080p60',
        'codec': 'h264',
        'size': [1920, 1080],
        'bitrate_mbps': '16',
        'include_audio': true,
        'default_name': 'youtube-1080p60.mp4',
      });
      expect(p.codec, 'h264');
      expect(p.size, [1920, 1080]);
      expect(p.bitrateMbps, '16');
      expect(p.defaultName, 'youtube-1080p60.mp4');
    });
  });

  group('Bridge v0.9 parsing', () {
    test('a comp parses its marker details with kind and confidence', () {
      final comp = BridgeComp.fromJson({
        'width': 640,
        'height': 480,
        'fps': {'num': 60, 'den': 1},
        'frame_count': 300,
        'layers': [],
        'markers': [60, 120],
        'marker_details': [
          {'frame': 60, 'kind': 'user', 'label': 'cue'},
          {'frame': 120, 'kind': 'beat', 'confidence': 0.75, 'label': ''},
        ],
      });
      // The bare frame array is unchanged (additive).
      expect(comp.markers, [60, 120]);
      expect(comp.markerDetails.length, 2);
      expect(comp.markerDetails[0].kind, 'user');
      expect(comp.markerDetails[0].isBeat, isFalse);
      expect(comp.markerDetails[0].confidence, isNull);
      expect(comp.markerDetails[1].isBeat, isTrue);
      expect(comp.markerDetails[1].confidence, 0.75);
    });

    test('a layer parses start offset, local in/out and asset read-backs', () {
      final text = BridgeLayer.fromJson({
        'id': 'l',
        'index': 0,
        'name': 'title',
        'kind': 'text',
        'in_frame': 0,
        'out_frame': 300,
        'start_offset_frame': 60,
        'start_offset_secs': 1.0,
        'in_secs': 2.0,
        'out_secs': 4.0,
        'label': 0,
        'switches': {},
        'text': {
          'content': 'Hello',
          'size': 72.0,
          'fill': [1.0, 0.5, 0.25, 1.0],
        },
      });
      expect(text.startOffsetFrame, 60);
      expect(text.startOffsetSecs, 1.0);
      expect(text.inSecs, 2.0);
      expect(text.outSecs, 4.0);
      expect(text.text!.content, 'Hello');
      expect(text.text!.size, 72.0);
      expect(text.text!.fill, [1.0, 0.5, 0.25, 1.0]);

      final cam = BridgeLayer.fromJson({
        'id': 'c',
        'index': 1,
        'name': 'cam',
        'kind': 'camera',
        'in_frame': 0,
        'out_frame': 300,
        'label': 0,
        'switches': {},
        'camera': {'value': 1200.0, 'animated': false},
      });
      expect(cam.cameraZoom!.value, 1200.0);
      expect(cam.cameraZoom!.animated, isFalse);

      final solid = BridgeLayer.fromJson({
        'id': 's',
        'index': 2,
        'name': 'solid',
        'kind': 'solid',
        'in_frame': 0,
        'out_frame': 300,
        'label': 0,
        'switches': {},
        'colour': [1.0, 0.0, 0.0, 1.0],
        'solid_size': [800, 600],
      });
      expect(solid.solidSize, [800, 600]);
      expect(solid.colour, [1.0, 0.0, 0.0, 1.0]);
    });

    test('a sequence layer parses its clips', () {
      final layer = BridgeLayer.fromJson({
        'id': 'seq',
        'index': 0,
        'name': 'Sequence',
        'kind': 'sequence',
        'in_frame': 0,
        'out_frame': 300,
        'label': 0,
        'switches': {},
        'clips': [
          {
            'id': 'clip-1',
            'source_kind': 'footage',
            'source_id': 'foo',
            'source_in_secs': 0.0,
            'source_out_secs': 5.0,
            'place_start_frame': 0,
            'place_end_frame': 300,
            'place_start_secs': 0.0,
            'place_duration_secs': 5.0,
            'retime': {
              'reverse': false,
              'interpolation': 'nearest',
              'boundaries': [
                {'t_seconds': 0.0, 's_seconds': 0.0, 'smooth': false},
                {'t_seconds': 5.0, 's_seconds': 5.0, 'smooth': false},
              ],
              'segments': [
                {'kind': 'rate', 'v0': 1.0, 'v1': 1.0, 'ease': 'Linear'},
              ],
            },
          },
        ],
      });
      expect(layer.clips.length, 1);
      final clip = layer.clips.single;
      expect(clip.id, 'clip-1');
      expect(clip.sourceKind, 'footage');
      expect(clip.sourceId, 'foo');
      expect(clip.placeStartFrame, 0);
      expect(clip.placeEndFrame, 300);
      expect(clip.sourceOutSecs, 5.0);
      expect(clip.retime!.boundaries.length, 2);
    });

    test('an effect parses its identity and animated param keys', () {
      final effect = BridgeEffect.fromJson({
        'id': 'e',
        'name': 'blur',
        'namespace': 'builtin',
        'version': 1,
        'sample_temporally': true,
        'enabled': true,
        'params': [
          {
            'name': 'amount',
            'kind': 'scalar',
            'value': 20.0,
            'animated': true,
            'keys': [
              {'frame': 0, 'value': 5.0, 'interp_in': 'Linear', 'interp_out': 'Linear'},
              {'frame': 60, 'value': 20.0, 'interp_in': 'Linear', 'interp_out': 'Linear'},
            ],
          },
          {'name': 'quality', 'kind': 'enum', 'value': 1},
        ],
      });
      expect(effect.namespace, 'builtin');
      expect(effect.version, 1);
      expect(effect.sampleTemporally, isTrue);
      final amount = effect.params.firstWhere((p) => p.name == 'amount');
      expect(amount.animated, isTrue);
      expect(amount.keys.length, 2);
      expect(amount.keys[1].frame, 60);
      expect(amount.keys[1].value, 20.0);
      final quality = effect.params.firstWhere((p) => p.name == 'quality');
      expect(quality.animated, isFalse);
      expect(quality.keys, isEmpty);
    });

    test('an effect parses per-channel colour keys', () {
      final effect = BridgeEffect.fromJson({
        'id': 'e',
        'name': 'tint',
        'enabled': true,
        'params': [
          {
            'name': 'colour',
            'kind': 'colour',
            'value': [1.0, 0.0, 0.0, 1.0],
            'animated': true,
            'keys_r': [
              {'frame': 0, 'value': 1.0, 'interp_in': 'Linear', 'interp_out': 'Linear'},
            ],
          },
        ],
      });
      final colour = effect.params.single;
      expect(colour.animated, isTrue);
      expect(colour.channelKeys.containsKey('keys_r'), isTrue);
      expect(colour.channelKeys['keys_r']!.single.value, 1.0);
      // An older library with no identity defaults sensibly.
      expect(effect.namespace, 'builtin');
      expect(effect.version, 0);
    });
  });

  group('AppStateStub v0.4 pass-throughs', () {
    test('the column and retime ops route to the bridge', () {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake);
      app.setBlendMode('c', 'l', 'Multiply');
      app.setMatte('c', 'l', 's', 'luma', true);
      app.setParent('c', 'l', 'p');
      app.setMotionBlur('c', true, 180.0, -90.0, 16);
      app.addMask('c', 'l', 'rectangle');
      app.setRetimeSpeed('c', 'l', 50.0);
      app.setSegmentPreset('c', 'l', 30, 'Smth');
      app.setKeyframeInterp('c', 'l', 'opacity', 0, 'Bezier', 'Linear');
      expect(fake.ops, contains('blend:c/l=Multiply'));
      expect(fake.ops, contains('matte:c/l=s/luma/true'));
      expect(fake.ops, contains('parent:c/l=p'));
      expect(fake.ops, contains('mask:c/l=rectangle'));
      expect(fake.ops, contains('retime_speed:c/l=50.0'));
      expect(fake.ops, contains('segment_preset:c/l@30=Smth'));
      expect(app.errorNotice, isNull);
    });

    test('export start/poll/cancel route to the bridge, with none-safe defaults',
        () {
      final fake = _FakeBridge();
      final app = AppStateStub(bridge: fake);
      final reply = app.startExport('c', '{"preset":"custom"}', 'out.mp4');
      expect(reply.ok, isTrue);
      expect(fake.ops, contains('start_export:c->out.mp4'));
      // The poll seam and preset resolver return the fake's idle defaults.
      expect(app.pollExport().state, 'idle');
      expect(app.exportPreset('custom', 'Scene', '').preset, 'custom');
      app.cancelExport();
      expect(fake.ops, contains('export_cancel'));

      // Without a bridge everything is a quiet, safe no-op.
      final bare = AppStateStub(bridge: null);
      expect(bare.startExport('c', '{}', 'o.mp4').ok, isFalse);
      expect(bare.pollExport().state, 'idle');
      expect(bare.listBlendModes(), isEmpty);
    });
  });

  group('Loader candidate library paths (platform portability)', () {
    // The base name the loader searches for is the OS's cdylib name:
    // `lumit_bridge.dll` on Windows, `liblumit_bridge.so` elsewhere. This runs
    // green on the Windows host gate AND the Linux CI gate, asserting each side.
    final expectedName =
        Platform.isWindows ? 'lumit_bridge.dll' : 'liblumit_bridge.so';

    test('every candidate ends in the platform library name', () {
      final paths = LumitBridge.candidateLibraryPaths();
      expect(paths, isNotEmpty);
      for (final p in paths) {
        expect(p.endsWith(expectedName), isTrue,
            reason: 'candidate "$p" should end in $expectedName');
      }
    });

    test('the bare OS-loader name is the last candidate', () {
      final paths = LumitBridge.candidateLibraryPaths();
      expect(paths.last, expectedName);
    });

    test('the release target dir is searched before the debug one', () {
      final paths = LumitBridge.candidateLibraryPaths();
      final firstRelease =
          paths.indexWhere((p) => p.contains('release'));
      final firstDebug = paths.indexWhere((p) => p.contains('debug'));
      expect(firstRelease, greaterThanOrEqualTo(0));
      expect(firstDebug, greaterThan(firstRelease));
    });
  });
}
