// Bridge v0.5 Dart-side tests: the AppStateStub pass-throughs for the new edit
// ops route to the EditOpsBridge capability, apply the returned snapshot, and
// surface a calm notice when the loaded library is too old to carry it. Also
// the autosave switch-over: with the capability present, autosave uses the
// dedicated op (no path re-pointing) rather than saveProject.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/beats_worker.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/state/app_state.dart';

/// A fake that offers BOTH [DocumentBridge] and the v0.5 [EditOpsBridge]
/// capability. The DocumentBridge surface is satisfied by [noSuchMethod]
/// (returning an ok snapshot) except the calls these tests drive; the
/// EditOpsBridge methods record what they were asked to do and return ok.
class _EditFake implements DocumentBridge, EditOpsBridge {
  _EditFake({this.path});

  final String? path;
  final List<String> ops = [];
  int autosaveCalls = 0;
  int saveCalls = 0;

  BridgeSnapshot _snap() => BridgeSnapshot(
        items: const [],
        canUndo: true,
        canRedo: false,
        path: path,
      );

  BridgeReply _ok(String record) {
    ops.add(record);
    return BridgeReply.ok(_snap());
  }

  // The DocumentBridge calls the stub actually makes.
  @override
  BridgeReply snapshot() => BridgeReply.ok(_snap());

  @override
  BridgeReply saveProject(String p) {
    saveCalls++;
    return BridgeReply.ok(_snap());
  }

  // Everything else on DocumentBridge → a benign ok snapshot.
  @override
  dynamic noSuchMethod(Invocation invocation) => BridgeReply.ok(_snap());

  // --- EditOpsBridge -----------------------------------------------------
  @override
  BridgeReply cutClipAtPlayhead(String c, String l, int f) =>
      _ok('cut:$c:$l:$f');
  @override
  BridgeReply deleteClipAtPlayhead(String c, String l, int f) =>
      _ok('del_clip:$c:$l:$f');
  @override
  BridgeReply detectBeats(String c, int s) => _ok('beats:$c:$s');
  @override
  BridgeReply clearBeatMarkers(String c) => _ok('clear_beats:$c');
  @override
  BridgeReply deleteItem(String id) => _ok('del_item:$id');
  @override
  BridgeReply renameItem(String id, String n) => _ok('rename_item:$id:$n');
  @override
  BridgeReply moveToRoot(String id) => _ok('to_root:$id');
  @override
  BridgeReply relink(String id, String p) => _ok('relink:$id:$p');
  @override
  BridgeReply renameLayer(String c, String l, String n) =>
      _ok('rename_layer:$c:$l:$n');
  @override
  BridgeReply convertToSequenced(String c, String l) => _ok('convert:$c:$l');
  @override
  BridgeReply trimToSourceEnd(String c, String l) => _ok('trim:$c:$l');
  @override
  BridgeReply setRetimeReverse(String c, String l, bool r) =>
      _ok('reverse:$c:$l:$r');
  @override
  BridgeReply setRetimeInterpolation(String c, String l, String i) =>
      _ok('interp:$c:$l:$i');
  @override
  BridgeReply setTextContent(String c, String l, String t, double size,
          double r, double g, double b, double a) =>
      _ok('text:$c:$l:$t:$size');
  @override
  BridgeReply setSolid(String c, String l, double r, double g, double b,
          double a, int w, int h) =>
      _ok('solid:$c:$l:$w:$h');
  @override
  BridgeReply setCameraZoom(String c, String l, double z) => _ok('zoom:$c:$l:$z');
  @override
  BridgeReply setEffectParamChoice(
          String c, String l, String e, String p, int i) =>
      _ok('choice:$e:$p:$i');
  @override
  BridgeReply setEffectParamBool(String c, String l, String e, String p, bool v) =>
      _ok('bool:$e:$p:$v');
  @override
  BridgeReply setEffectParamSeed(String c, String l, String e, String p, int s) =>
      _ok('seed:$e:$p:$s');
  @override
  BridgeReply setEffectParamPoint(
          String c, String l, String e, String p, double x, double y) =>
      _ok('point:$e:$p:$x:$y');
  @override
  BridgeReply reorderEffect(String c, String l, String e, int i) =>
      _ok('reorder_fx:$e:$i');
  @override
  BridgeReply applyKeyframeBatch(String c, String l, String j) =>
      _ok('kf_batch:$c:$l');
  @override
  BridgeReply autosave(String p, int keep) {
    autosaveCalls++;
    return BridgeReply.ok(_snap());
  }

  @override
  List<BridgeAutosave> listAutosaves(String p) =>
      const [BridgeAutosave(slot: 1, path: '/x/autosaves/a.autosave-1.lum')];
  @override
  BridgeReply restoreJournal(String p) => _ok('restore:$p');
  @override
  List<String> bootLog() => const ['lumit-bridge 0.1.0', 'ABI v7'];

  // --- Bridge v0.9 -------------------------------------------------------
  @override
  BridgeReply addMaskGeometry(
          String c, String l, String k, double x, double y, double w, double h) =>
      _ok('mask_geom:$c:$l:$k:$x,$y,$w,$h');
  @override
  BridgeReply toggleEffectParamAnimated(
          String c, String l, String e, String p, int ch, int f) =>
      _ok('fx_toggle:$e:$p:$ch@$f');
  @override
  BridgeReply addEffectParamKeyframe(
          String c, String l, String e, String p, int ch, int f, double v) =>
      _ok('fx_addkey:$e:$p:$ch@$f=$v');
  @override
  BridgeReply removeEffectParamKeyframe(
          String c, String l, String e, String p, int ch, int f) =>
      _ok('fx_rmkey:$e:$p:$ch@$f');
  @override
  BridgeReply shiftEffectParamKeyframes(
          String c, String l, String e, String p, int ch, String frames, int d) =>
      _ok('fx_shift:$e:$p:$ch:$frames+$d');
  @override
  BridgeReply setEffectParamKeyframeInterp(
          String c,
          String l,
          String e,
          String p,
          int ch,
          int f,
          String ii,
          String io,
          double si,
          double fi,
          double so,
          double fo) =>
      _ok('fx_interp:$e:$p:$ch@$f=$ii/$io');
  @override
  BridgeReply saveEffectPreset(String c, String l, String n) =>
      _ok('save_preset:$c:$l:$n');
  @override
  BridgeReply loadEffectPreset(String c, String l, String t) =>
      _ok('load_preset:$c:$l');
  @override
  BridgePlaybackTier playbackTier() =>
      const BridgePlaybackTier(tier: 2, scale: 0.5);
  @override
  BridgePlaybackTier resetRealtime() => BridgePlaybackTier.full;
}

/// A DocumentBridge-only fake (no EditOpsBridge) — an "older library" for the
/// missing-capability path. Everything is a benign ok snapshot.
class _DocOnlyFake implements DocumentBridge {
  BridgeSnapshot _snap() => const BridgeSnapshot(
        items: [],
        canUndo: false,
        canRedo: false,
        path: null,
      );
  @override
  BridgeReply snapshot() => BridgeReply.ok(_snap());
  @override
  dynamic noSuchMethod(Invocation invocation) => BridgeReply.ok(_snap());
}

/// An [_EditFake] whose snapshot carries one composition, so the front-comp
/// resolution (`frontCompIdResolved`) finds `c1` — the beats ops need one.
class _CompEditFake extends _EditFake {
  @override
  BridgeSnapshot _snap() => BridgeSnapshot(
        items: [
          BridgeItem(
            id: 'c1',
            name: 'Scene',
            kind: BridgeItemKind.composition,
            children: const [],
            comp: BridgeComp(
              width: 4,
              height: 4,
              fps: const BridgeFps(24, 1),
              frameCount: 48,
              layers: const [],
              markers: const [],
            ),
          ),
        ],
        canUndo: true,
        canRedo: false,
        path: path,
      );
}

void main() {
  group('EditOps pass-throughs route to the capability', () {
    test('raw ops record their call and apply the snapshot', () {
      final fake = _EditFake(path: '/proj/scene.lum');
      final app = AppStateStub(bridge: fake);
      expect(app.editOps, isNotNull);

      app.deleteItem('item-1');
      app.renameItem('item-1', 'Renamed');
      app.moveToRoot('item-1');
      app.relink('item-1', '/new/clip.mp4');
      app.renameLayer('c1', 'l1', 'Hero');
      app.convertToSequenced('c1', 'l1');
      app.trimToSourceEnd('c1', 'l1');
      app.setRetimeReverse('c1', 'l1', true);
      app.setRetimeInterpolation('c1', 'l1', 'blend');
      app.setTextContent('c1', 'l1', 'Hello', 48, 1, 0, 0, 1);
      app.setSolid('c1', 'l1', 0.25, 0.5, 0.75, 1, 640, 480);
      app.setCameraZoom('c1', 'l1', 1234);
      app.setEffectParamChoice('c1', 'l1', 'e1', 'mode', 2);
      app.setEffectParamBool('c1', 'l1', 'e1', 'flag', true);
      app.setEffectParamSeed('c1', 'l1', 'e1', 'seed', 7);
      app.setEffectParamPoint('c1', 'l1', 'e1', 'centre', 10, 20);
      app.reorderEffect('c1', 'l1', 'e1', 0);
      app.applyKeyframeBatch('c1', 'l1', '[]');

      expect(
        fake.ops,
        containsAll(<String>[
          'del_item:item-1',
          'rename_item:item-1:Renamed',
          'to_root:item-1',
          'relink:item-1:/new/clip.mp4',
          'rename_layer:c1:l1:Hero',
          'convert:c1:l1',
          'trim:c1:l1',
          'reverse:c1:l1:true',
          'interp:c1:l1:blend',
          'text:c1:l1:Hello:48.0',
          'solid:c1:l1:640:480',
          'zoom:c1:l1:1234.0',
          'choice:e1:mode:2',
          'bool:e1:flag:true',
          'seed:e1:seed:7',
          'point:e1:centre:10.0:20.0',
          'reorder_fx:e1:0',
          'kf_batch:c1:l1',
        ]),
      );
      // A successful op clears any error tint.
      expect(app.errorNotice, isNull);
    });

    test('recovery + boot log read through the capability', () {
      final fake = _EditFake(path: '/proj/scene.lum');
      final app = AppStateStub(bridge: fake);
      expect(app.listAutosaves('').length, 1);
      expect(app.listAutosaves('').first.slot, 1);
      expect(app.bootLog(), contains('ABI v7'));
      app.restoreJournal('/proj/scene.lum');
      expect(fake.ops, contains('restore:/proj/scene.lum'));
    });

    test('a library without the capability surfaces a calm notice', () {
      final app = AppStateStub(bridge: _DocOnlyFake());
      expect(app.editOps, isNull);
      app.deleteItem('item-1');
      expect(app.errorNotice, contains('missing the edit ops'));
    });

    test('no bridge is a quiet no-op (no notice)', () {
      final app = AppStateStub();
      app.deleteItem('item-1');
      expect(app.errorNotice, isNull);
      expect(app.editOps, isNull);
    });
  });

  group('autosave switch-over', () {
    test('with the capability, autosave uses the dedicated op not saveProject',
        () {
      final start = DateTime.now();
      final fake = _EditFake(path: '/proj/scene.lum');
      final app = AppStateStub(bridge: fake)
        ..autosaveInterval = const Duration(minutes: 5)
        ..autosaveKeep = 3;
      // Dirty the document through a real edit op.
      app.deleteItem('item-1');
      // A due tick writes a copy through the dedicated autosave op.
      expect(app.autosaveTick(start.add(const Duration(minutes: 10))), isTrue);
      expect(fake.autosaveCalls, 1,
          reason: 'autosave routed through the dedicated op');
      expect(fake.saveCalls, 0,
          reason: 'the path-repointing saveProject is not used');
      app.dispose();
    });
  });

  group('bridge v0.9 pass-throughs', () {
    test('mask geometry, effect keyframes and presets route to the bridge', () {
      final fake = _EditFake();
      final app = AppStateStub(bridge: fake);
      app.addMaskGeometry('c', 'l', 'rectangle', 10, 20, 100, 50);
      app.toggleEffectParamAnimated('c', 'l', 'e', 'amount', 0, 0);
      app.addEffectParamKeyframe('c', 'l', 'e', 'amount', 0, 60, 20.0);
      app.removeEffectParamKeyframe('c', 'l', 'e', 'amount', 0, 0);
      app.shiftEffectParamKeyframes('c', 'l', 'e', 'amount', 0, '[60]', 30);
      app.setEffectParamKeyframeInterp(
          'c', 'l', 'e', 'amount', 0, 0, 'Hold', 'Linear', 0, 0, 0, 0);
      app.loadEffectPreset('c', 'l', '{"format":1,"name":"x","effects":[]}');
      expect(fake.ops, contains('mask_geom:c:l:rectangle:10.0,20.0,100.0,50.0'));
      expect(fake.ops, contains('fx_toggle:e:amount:0@0'));
      expect(fake.ops, contains('fx_addkey:e:amount:0@60=20.0'));
      expect(fake.ops, contains('fx_rmkey:e:amount:0@0'));
      expect(fake.ops, contains('fx_shift:e:amount:0:[60]+30'));
      expect(fake.ops, contains('fx_interp:e:amount:0@0=Hold/Linear'));
      expect(fake.ops, contains('load_preset:c:l'));
      expect(app.errorNotice, isNull);
    });

    test('the realtime tier reads back, and falls back to Full without a bridge',
        () {
      final fake = _EditFake();
      final app = AppStateStub(bridge: fake);
      final tier = app.playbackTier();
      expect(tier.tier, 2);
      expect(tier.scale, 0.5);
      expect(tier.label, 'Half');
      expect(app.resetRealtime().tier, 1);
      // Without a bridge, the readout is Full and the ops are quiet no-ops.
      final bare = AppStateStub(bridge: null);
      expect(bare.playbackTier().tier, 1);
      expect(bare.playbackTier().label, 'Full');
      bare.addMaskGeometry('c', 'l', 'ellipse', 0, 0, 10, 10);
      expect(bare.errorNotice, isNull);
    });
  });

  // Beat detection off the UI isolate (TF round 5): with the real FFI bridge
  // the blocking mixdown+analysis runs in a short-lived isolate; a fake bridge
  // keeps the synchronous path so these stay deterministic, and the
  // reply-adoption half (what the isolate's answer does on the UI isolate) is
  // unit-tested directly.
  group('detectBeats (TF round 5)', () {
    test('a fake bridge keeps the synchronous path and commits the op',
        () async {
      final fake = _CompEditFake();
      final app = AppStateStub(bridge: fake);
      await app.detectBeats(60);
      expect(fake.ops, contains('beats:c1:60'));
      expect(app.errorNotice, isNull);
    });

    test('no front comp is one calm notice, no op', () async {
      final fake = _EditFake();
      final app = AppStateStub(bridge: fake);
      await app.detectBeats(50);
      expect(fake.ops, isEmpty);
      expect(app.notice, contains('Open a composition'));
    });

    test('adoptDetectBeatsReply applies an ok reply and drops the busy notice',
        () {
      final app = AppStateStub(bridge: _EditFake());
      app.setNotice('Detecting beats…');
      final epochBefore = app.documentEpoch;
      app.adoptDetectBeatsReply(
          '{"ok":true,"items":[],"can_undo":true,"can_redo":false,"path":null}');
      expect(app.notice, isNull, reason: 'the busy notice is dropped');
      expect(app.errorNotice, isNull);
      expect(app.canUndo, isTrue, reason: 'the undo flag follows the snapshot');
      expect(app.documentEpoch, epochBefore + 1,
          reason: 'the adopted snapshot bumps the epoch like any edit');
    });

    test('adoptDetectBeatsReply surfaces an error reply in the error tint', () {
      final app = AppStateStub(bridge: _EditFake());
      app.setNotice('Detecting beats…');
      app.adoptDetectBeatsReply('{"ok":false,"error":"no audio to analyse"}');
      expect(app.notice, isNull);
      expect(app.errorNotice, 'no audio to analyse');
    });

    test('detectBeatsWithLibrary answers a bridge-shaped error when no library '
        'opens', () {
      final raw = detectBeatsWithLibrary(
          ['definitely-not-a-library-anywhere.dll'], 'c1', 50);
      final reply = BridgeReply.parse(raw);
      expect(reply.ok, isFalse);
      expect(reply.error, contains('could not be opened'));
    });
  });
}
