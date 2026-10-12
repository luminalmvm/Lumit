// Backup, and the two things it must never do to a project file: write an
// earlier version over work the server does not hold, and leave half a
// download where a good file was. And the one thing it must not do to a
// version's picture: stop it being one.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart' show saveProjectFrb;
import 'package:lumit_flutter/shell/project_history_window.dart';
import 'package:lumit_flutter/state/account.dart';
import 'package:lumit_flutter/state/backup.dart';
import 'package:lumit_flutter/state/transfer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  Directory scratch() {
    final dir = Directory.systemTemp.createTempSync('lumit-backup');
    addTearDown(() => dir.deleteSync(recursive: true));
    return dir;
  }

  test('a download that is cut off or wrong leaves the file as it was',
      () async {
    final dir = scratch();
    final file = File('${dir.path}${Platform.pathSeparator}Project.lum')
      ..writeAsStringSync('the good one');
    final sent = utf8.encode('an earlier version');
    final hash = sha256.convert(sent).toString();

    Stream<List<int>> cutOff() async* {
      yield sent.sublist(0, 5);
      throw const SocketException('the connection went');
    }

    await expectLater(writeChecked(cutOff(), hash, file.path),
        throwsA(isA<CloudError>()));
    expect(file.readAsStringSync(), 'the good one');

    await expectLater(
        writeChecked(Stream.value(utf8.encode('other bytes')), hash, file.path),
        throwsA(isA<CloudError>().having((e) => e.code, 'code', 'bad_hash')));
    expect(file.readAsStringSync(), 'the good one');
    expect(dir.listSync(), hasLength(1),
        reason: 'and nothing half-written is left beside it');

    // The version asked for, whole, does take its place.
    await writeChecked(Stream.value(sent), hash, file.path);
    expect(file.readAsBytesSync(), sent);

    // Footage is carried on from where it was cut off. Until it is whole
    // there is nothing where the engine reads, only the file beside it.
    final clip = '${dir.path}${Platform.pathSeparator}standin.mp4';
    await expectLater(writeChecked(cutOff(), null, clip, resume: true),
        throwsA(isA<CloudError>()));
    expect(File(clip).existsSync(), isFalse);
    expect(File('$clip.part').readAsBytesSync(), sent.sublist(0, 5));
    await writeChecked(Stream.value(sent.sublist(5)), null, clip, resume: true);
    expect(File(clip).readAsBytesSync(), sent);
    expect(File('$clip.part').existsSync(), isFalse);
  }, skip: !engineAvailable);

  test('putting a version back never writes over work that was not sent',
      () async {
    TestWidgetsFlutterBinding.ensureInitialized();
    final path = '${scratch().path}${Platform.pathSeparator}Project.lum';
    final p = freshProject();
    final app = p.state;
    final ui = p.uiState;
    // An account with Pro and no way to the server: nothing here is signed
    // in, so every request is refused before it is made.
    ui.account.account = const CloudAccount(
        id: 'a', email: 'a@example.com', name: 'A', pro: true);
    // ignore: invalid_use_of_protected_member, invalid_use_of_visible_for_testing_member
    ui.account.notifyListeners();

    app.project!.newComposition(name: 'Scene');
    await saveProjectFrb(app, ui, forcePicker: true, picker: () async => path);
    expect(ui.backup.id, isNotNull, reason: 'the save gave it a backup id');

    // Work that is in the open project and nowhere else.
    app.project!.newComposition(name: 'Not saved yet');
    expect(app.project!.isDirty(), isTrue);

    final earlier = BackupVersion(
        n: 1, hash: '0' * 64, size: 1, created: DateTime.now());
    expect(await putBackVersion(app, ui, earlier), isFalse);

    expect(app.project!.isDirty(), isFalse,
        reason: 'the unsaved work was saved first');
    expect(app.comps(), hasLength(2),
        reason: 'and the open project still has it');
    // Stopped because the project as it stands could not be sent, which is
    // before anything is fetched. A fetch that failed would say otherwise.
    expect(app.notice.value?.message, l10n.backupPutBackUnsent);
    expect(app.notice.value?.error, isTrue);

    // The same once Pro has ended, when nothing can be sent at all.
    ui.account.account =
        const CloudAccount(id: 'a', email: 'a@example.com', name: 'A');
    // ignore: invalid_use_of_protected_member, invalid_use_of_visible_for_testing_member
    ui.account.notifyListeners();
    app.project!.newComposition(name: 'Nor this');
    expect(await putBackVersion(app, ui, earlier), isFalse);
    expect(app.comps(), hasLength(3));
    expect(app.notice.value?.message, l10n.backupPutBackUnsent);
  }, skip: !engineAvailable);

  test('a picture that says what it shows is still the same picture',
      () async {
    final pixels = Uint8List.fromList([
      for (var i = 0; i < 32 * 18 * 4; i++) i % 4 == 3 ? 255 : (i * 7) % 256,
    ]);
    final made = Completer<ui.Image>();
    ui.decodeImageFromPixels(
        pixels, 32, 18, ui.PixelFormat.rgba8888, made.complete);
    final plain =
        (await (await made.future).toByteData(format: ui.ImageByteFormat.png))!
            .buffer
            .asUint8List();
    expect(stillCaption(plain), isNull,
        reason: 'a picture made before they said so says nothing');

    // A name longer than a picture carries, whose cut would land inside a
    // character that is two code units and four bytes.
    final name = '${'a' * 238}🎬 and the rest';
    final said = stillCaptioned(plain, name, '00:00:04:00');
    final frame = await (await ui.instantiateImageCodec(said)).getNextFrame();
    expect((await frame.image.toByteData())!.buffer.asUint8List(), pixels);
    expect(stillCaption(said), ('a' * 238, '00:00:04:00'));
    expect(stillCaption(stillCaptioned(plain, 'Títulos 片', '01:02:03:04')),
        ('Títulos 片', '01:02:03:04'));

    // The check on the new chunk is the one zlib works out for the same
    // bytes, which a strict reader holds a PNG to.
    final bytes = ByteData.sublistView(said);
    final length = bytes.getUint32(33);
    final packed = gzip.encode(said.sublist(37, 41 + length));
    expect(
        bytes.getUint32(41 + length),
        ByteData.sublistView(Uint8List.fromList(packed))
            .getUint32(packed.length - 8, Endian.little));
  });
}
