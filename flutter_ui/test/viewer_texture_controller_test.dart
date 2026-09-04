// The Viewer's zero-copy texture controller against a fake runner.
//
// The failure being guarded is the silent one: a runner that registers the
// texture, accepts every frameReady, and never actually draws it. The
// controller detects that by counting the draws the runner reports back — so a
// runner whose frameReady answers null (as the Linux one did before it grew
// its own branch) can never be told apart from one that is drawing.
//
// **What it does about it changed.** It used to latch the texture path off,
// which only made sense while there was a read-back transport to fall back
// to; there is not, and the owner's ruling is that there will not be. So the
// detector's whole job is now to make the failure *loud* — a line in the
// diagnostics file — while the path keeps announcing, because switching off the
// only transport there is can only turn a recoverable Viewer into a dead one.
// These tests pin that: never-drawn must not disable the path, and a rising
// count must leave everything alone.

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

  test('a runner that never reports a draw is flagged, not switched off',
      () async {
    final controller =
        ViewerTextureController(channel: fakeRunner((_) => null));
    expect(await controller.ensureRegistered(0, 640, 360, fd: 3), 7);
    expect(controller.available, isTrue);

    // Past the grace window, so the never-drawn condition has held for a while.
    for (var i = 0; i < 20; i++) {
      await controller.frameReady();
    }

    expect(controller.debugAnnounced, 20,
        reason: 'the path keeps announcing; there is nothing else to move to');
    expect(controller.debugDrawn, 0);
    expect(controller.neverDrawn, isTrue);
    expect(controller.available, isTrue,
        reason: 'the only transport is not switched off for being '
            'broken — the failure is recorded and then fixed at its cause');
    // The record itself goes to the shared diagnostics file, so this test
    // appends one line to it. That is what the file is for, and giving the
    // controller a seam to write somewhere else would be a seam for one test.
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
