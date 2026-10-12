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

  // A laptop with two graphics cards. The engine renders on the NVIDIA card,
  // Flutter draws with the other, and the runner refuses the texture, since
  // importing it crashed the application inside the other card's driver. These
  // three pin the Dart half of that: the runner is told which card the buffer
  // is on, and a refusal reaches the Viewer's message by either road.
  group('a texture on another graphics card', () {
    MethodChannel runner(Future<Object?> Function(MethodCall call) answer) {
      final channel = const MethodChannel(ViewerTextureController.channelName);
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, answer);
      return channel;
    }

    PlatformException mismatch() => PlatformException(
        code: ViewerTextureController.gpuMismatchCode,
        message: 'the render GPU is not the display GPU');

    setUp(() => ViewerTextureController.gpuMismatch.value = false);

    test('the runner is told which card the buffer is on', () async {
      final sent = <Map<Object?, Object?>>[];
      final controller = ViewerTextureController(channel: runner((call) async {
        if (call.method != 'register') return null;
        sent.add(call.arguments as Map<Object?, Object?>);
        return 7;
      }));

      await controller.ensureRegistered(0, 640, 360,
          fd: 3, renderMajor: 226, renderMinor: 129);
      expect(sent.single['renderMajor'], 226);
      expect(sent.single['renderMinor'], 129);
      // A node the driver didn't report is left out, which the runner reads
      // as unknown. A zero would name a real card and refuse a healthy one.
      expect(sent.single.containsKey('primaryMajor'), isFalse);
      expect(sent.single.containsKey('primaryMinor'), isFalse);
    });

    test('a refused registration raises the message', () async {
      final controller = ViewerTextureController(channel: runner((call) async {
        if (call.method == 'register') throw mismatch();
        return null;
      }));

      expect(await controller.ensureRegistered(0, 640, 360, fd: 3), isNull);
      expect(ViewerTextureController.gpuMismatch.value, isTrue);
      expect(controller.available, isFalse,
          reason: 'the cards will not change while Lumit is running');
    });

    test('a refusal found at import raises it too', () async {
      // The import itself runs on the raster thread, so a refusal there is
      // only heard when the next frame is announced.
      final controller = ViewerTextureController(channel: runner((call) async {
        if (call.method == 'register') return 7;
        if (call.method == 'frameReady') throw mismatch();
        return null;
      }));

      expect(await controller.ensureRegistered(0, 640, 360, fd: 3), 7);
      expect(ViewerTextureController.gpuMismatch.value, isFalse);
      await controller.frameReady();
      expect(ViewerTextureController.gpuMismatch.value, isTrue);
      expect(controller.available, isFalse);
    });
  });
}
