// The command line can name a .lum to open (the installer's file association
// passes the document path as an argument). projectPathFromArgs picks it out:
// the first existing .lum on the line, nothing else.

import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart'
    show listenForOpenRequests, projectPathFromArgs, takeOpenRequests;

void main() {
  late Directory tmp;
  late String real;

  setUp(() {
    tmp = Directory.systemTemp.createTempSync('lumit_args');
    real = '${tmp.path}${Platform.pathSeparator}shot.lum';
    File(real).writeAsStringSync('');
  });

  tearDown(() => tmp.deleteSync(recursive: true));

  test('flags and stray tokens around it are ignored', () {
    expect(projectPathFromArgs(['--verbose', real, 'other']), real);
    // A Linux desktop hands the same document over as a file: address.
    expect(projectPathFromArgs([Uri.file(real).toString()]), real);
  });

  // macOS names the document in an event instead, and so with an invite
  // link. The runner keeps each until it is taken (`OpenRequests` in
  // AppDelegate.swift, which this stands in for) and says when one has come.
  testWidgets('what macOS asks to open is taken at start-up, and as it comes',
      (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    const channel = MethodChannel('lumit/open');
    final messenger = tester.binding.defaultBinaryMessenger;
    var waiting = [real];
    messenger.setMockMethodCallHandler(channel, (call) async {
      expect(call.method, 'take');
      final taken = waiting;
      waiting = [];
      return taken;
    });
    try {
      // Started by a double-click: read as the argument it is elsewhere.
      expect(projectPathFromArgs(await takeOpenRequests()), real);
      expect(await takeOpenRequests(), isEmpty, reason: 'taken once');

      // Then one that came before anything was listening, and one after.
      waiting = ['lumit://join/early'];
      final asked = <List<String>>[];
      listenForOpenRequests(asked.add);
      await tester.pump();
      expect(asked, [
        ['lumit://join/early']
      ]);
      waiting = ['lumit://join/late'];
      await messenger.handlePlatformMessage(
          channel.name,
          channel.codec.encodeMethodCall(const MethodCall('open')),
          (_) {});
      await tester.pump();
      expect(asked.last, ['lumit://join/late']);
    } finally {
      channel.setMethodCallHandler(null);
      messenger.setMockMethodCallHandler(channel, null);
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
