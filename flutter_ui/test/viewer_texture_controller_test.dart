// The Viewer's zero-copy texture controller against a fake runner (K-177).
//
// The failure being guarded is the silent one: a runner that registers the
// texture, accepts every frameReady, and never actually draws it. The
// controller detects that by counting the draws the runner reports back — so a
// runner whose frameReady answers null (as the Linux one did before K-204's
// branch) can never be told apart from one that is drawing, and the fallback
// never fires. These tests pin both halves: null answers must give up after the
// grace window, and a rising count must keep the path alive.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_texture_controller.dart';

/// A stand-in runner. `register` hands back an id; `frameReady` answers with
/// whatever [drawn] returns for that call (null means "the runner told us
/// nothing", which is the bug).
MethodChannel fakeRunner(Object? Function(int call) drawn) {
  var calls = 0;
  final channel = const MethodChannel(ViewerTextureController.channelName);
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(channel, (call) async {
    switch (call.method) {
      case 'register':
        return 7;
      case 'frameReady':
        calls++;
        return drawn(calls);
      default:
        return null;
    }
  });
  return channel;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('a runner that never reports a draw drops the texture path', () async {
    final controller =
        ViewerTextureController(channel: fakeRunner((_) => null));
    expect(await controller.ensureRegistered(0, 640, 360, fd: 3), 7);
    expect(controller.available, isTrue);

    for (var i = 0; i < 12; i++) {
      await controller.frameReady();
    }

    expect(controller.debugAnnounced, 12);
    expect(controller.debugDrawn, 0);
    expect(controller.neverDrawn, isTrue);
    expect(controller.available, isFalse,
        reason: 'twelve announced frames and no draw means the runner is not '
            'showing the texture; the Viewer must stop waiting on it');
  });

  test('a runner that counts its draws keeps the texture path', () async {
    final controller =
        ViewerTextureController(channel: fakeRunner((call) => call));
    expect(await controller.ensureRegistered(0, 640, 360, fd: 3), 7);

    for (var i = 0; i < 12; i++) {
      await controller.frameReady();
    }

    expect(controller.debugDrawn, 12);
    expect(controller.neverDrawn, isFalse);
    expect(controller.available, isTrue);
  });

  /// **The blank flash on a resize.** A new shared texture means a new id, and
  /// the old one used to be unregistered *first* — so for the length of a
  /// platform round trip the Viewer had nothing to draw. The replacement is
  /// registered before the old one is let go, so the last good frame stays on
  /// screen until there is a newer one to put there.
  test('a replacement is registered before the old texture is let go',
      () async {
    final order = <String>[];
    var next = 7;
    final channel = const MethodChannel(ViewerTextureController.channelName);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
      order.add(call.method);
      return call.method == 'register' ? next++ : null;
    });
    final controller = ViewerTextureController(channel: channel);

    expect(await controller.ensureRegistered(0, 640, 360, fd: 3), 7);
    expect(order, ['register']);
    // A comp resize: same fd, new size.
    expect(await controller.ensureRegistered(0, 1280, 720, fd: 4), 8);
    expect(order, ['register', 'register', 'unregister'],
        reason: 'the new texture is up before the old one goes');
  });

  /// Frames arrive faster than a platform round trip, so a resize used to start
  /// one registration per frame that landed while the first was still out —
  /// every one but the last leaked, and the Viewer flickered between them.
  test('registrations for the same texture are not started twice', () async {
    var registers = 0;
    final channel = const MethodChannel(ViewerTextureController.channelName);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
      if (call.method == 'register') registers++;
      return call.method == 'register' ? 7 : null;
    });
    final controller = ViewerTextureController(channel: channel);

    final ids = await Future.wait([
      controller.ensureRegistered(0, 640, 360, fd: 3),
      controller.ensureRegistered(0, 640, 360, fd: 3),
      controller.ensureRegistered(0, 640, 360, fd: 3),
    ]);

    expect(ids, [7, 7, 7]);
    expect(registers, 1, reason: 'one texture, one registration');
  });

  test('a missing handler latches the path off at once', () async {
    final controller = ViewerTextureController(
        channel: const MethodChannel('lumit/viewer_texture_absent'));
    expect(await controller.ensureRegistered(0, 640, 360, fd: 3), isNull);
    expect(controller.available, isFalse);
  });
}
