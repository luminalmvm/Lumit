// The Skia pin, on all three runners.
//
// In plain terms: Lumit tells Flutter to draw with Skia rather than Impeller,
// and each platform's runner has to say so in a way the *shipped* build hears.
// An earlier pin said so on Linux and macOS through the FLUTTER_ENGINE_SWITCHES
// environment variables `flutter run` uses — and the engine reads those only
// under `#ifndef FLUTTER_RELEASE` (shell/platform/common/engine_switches.cc),
// so the pin held while developing and evaporated in the release build users
// download. On Linux that meant Impeller, and Impeller never draws the Viewer's
// external texture: issue #104, a blank preview in the 0.3.0/0.3.1 Flatpak.
//
// There is no compiling the runners from a Dart test, so this reads their
// source instead. Crude, and the only check that fails the day somebody
// reaches for the environment variables again.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  // Test working directory is flutter_ui/.
  final runners = <String, String>{
    'linux': 'linux/runner/my_application.cc',
    'macos-window': 'macos/Runner/MainFlutterWindow.swift',
    'macos-plist': 'macos/Runner/Info.plist',
    'windows': 'windows/runner/main.cpp',
  }.map((name, path) => MapEntry(name, File(path).readAsStringSync()));

  test('each runner pins Skia through an API the release build reads', () {
    expect(runners['linux'],
        contains('fl_dart_project_set_enable_impeller(project, FALSE)'));
    // macOS reads FLTEnableImpeller off the bundle, so the pin is a plist key,
    // not a line of Swift. Whitespace between the key and its value is the
    // plist's own formatting, hence the collapse.
    expect(runners['macos-plist']!.replaceAll(RegExp(r'\s+'), ''),
        contains('<key>FLTEnableImpeller</key><false/>'));
    expect(runners['windows'],
        contains('set_impeller_switch(flutter::ImpellerSwitch::Disabled)'));
  });

  test('no runner pins it through the environment, which release ignores', () {
    // The numbered key, `FLUTTER_ENGINE_SWITCH_<n>` — only code that builds one
    // writes that, so the runners' own comments about why they do not are not
    // mistaken for the thing they warn against.
    for (final entry in runners.entries) {
      expect(entry.value, isNot(contains('FLUTTER_ENGINE_SWITCH_')),
          reason: '${entry.key} reaches for an environment switch the release '
              'engine compiles out; use the platform API instead');
    }
  });
}
