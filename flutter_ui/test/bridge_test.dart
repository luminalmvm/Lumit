// Bridge v0 Dart-side tests: the JSON → typed-model parsing (fed literal
// strings, no library needed), and the guarantee that AppStateStub without a
// bridge behaves exactly as the F0 placeholder did.

import 'dart:io';

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
}
