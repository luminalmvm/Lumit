// The colour seam (docs/impl/ocio.md §6.1).
//
// What only Dart can prove is asserted here; the engine's own tests cover the
// behaviour behind each call. Two things, and both would compile away silently
// if they were only tested in Rust:
//
// * **The summary arrives whole and holds the config's own words** — the names
//   a picker is built from cross verbatim and are never put through the
//   translation table.
// * **A refusal arrives as an id plus its facts, and Dart writes the
//   sentence.** That is the whole point of the shape: the engine's English is
//   a fallback, not the text on screen.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';

import 'frb_test_support.dart';

/// A small, complete config: one space that is not the reference, one display
/// with one view, and the roles the resolution walk reads.
const goodConfig = '''
ocio_profile_version: 1
roles:
  scene_linear: lin
  reference: ref
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: out_srgb}
colorspaces:
  - !<ColorSpace>
    name: ref
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: srgb_texture
    to_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0]}
  - !<ColorSpace>
    name: out_srgb
    from_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}
''';

void main() {
  setUpAll(initEngineForTests);

  group('Colour seam (frb)', () {
    late Directory dir;

    setUp(() => dir = Directory.systemTemp.createTempSync('lumit-ocio'));
    tearDown(() => dir.deleteSync(recursive: true));

    test('a refusal crosses as an id and its facts, and Dart writes the words',
        () {
      final project = LumitBridgeState.newProject();
      project.setColourConfig(
          path: '${dir.path}${Platform.pathSeparator}not-here.ocio');

      final summary = project.colourSummary();
      expect(summary.loaded, isFalse);
      expect(summary.problem, 'config_unreadable',
          reason: 'an id, never a finished sentence');
      expect(summary.problemEnglish, isNotEmpty,
          reason: 'the engine\'s own words ride along as the fallback');

      final args = {for (final a in summary.problemArgs) a.name: a.value};
      expect(args['path'], endsWith('not-here.ocio'));

      final sentence = colourProblem(summary.problem, args);
      expect(sentence, isNotNull,
          reason: 'this build has a sentence for every id the engine can send');
      expect(sentence, contains('not-here.ocio'),
          reason: 'the path is the user\'s own and is never translated');
    });
  });
}
