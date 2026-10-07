// The export seam's new surface.
//
// What only Dart can prove is asserted here; the behaviour behind each field is
// covered by the engine's own tests. Two things, both of which would compile
// away silently if they were only tested in Rust:
//
// * **The generated defaults.** Every field these decisions added is optional in
//   the Dart constructor, so a caller that sets none of them — the export dialog
//   as it stands today — still compiles and still asks for the export Lumit has
//   always written. A required field would break that call site, which is how a
//   seam quietly forces a frontend change it has no business forcing.
// * **A refusal arrives as a catchable error**, not as a crash across the FFI
//   boundary (docs/17, "The four binding rules").

import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';

import 'frb_test_support.dart';

/// A spec stating only the fields that existed before this work — so every
/// added field is left to its generated default. The call itself is half the
/// assertion: it does not compile if one of them became required.
BridgeExportSpec plainSpec({
  String codec = 'h264',
  int audioRate = 0,
  int audioDepth = 0,
  int audioChannels = 0,
}) =>
    BridgeExportSpec(
      preset: '',
      codec: codec,
      width: 0,
      height: 0,
      bitrateMbps: 0,
      peakMbps: 0,
      bitrateAuto: true,
      fps: 0,
      rangeStartFrame: -1,
      rangeEndFrame: -1,
      includeAudio: true,
      audioBitRate: 320000,
      audioRate: audioRate,
      audioDepth: audioDepth,
      audioChannels: audioChannels,
      depth: 8,
      alphaChannel: false,
      straightAlpha: false,
      colourSpace: '',
      cropTop: 0,
      cropLeft: 0,
      cropBottom: 0,
      cropRight: 0,
      useRegionOfInterest: false,
      region: Float64List.fromList(const []),
      metadata: const [],
      qualityDivisor: 1,
      diskCacheReadOnly: false,
      effects: true,
      honourSolo: true,
      makeANoise: false,
      openFolder: false,
    );

void main() {
  setUpAll(initEngineForTests);

  group('Export seam (frb)', () {
    test('reordering an item the queue does not hold is a catchable refusal',
        () {
      expect(
        () => exportQueueMove(id: 4294967295, index: 0),
        throwsA(isA<Object>()),
        reason: 'a refusal crosses as an error, never as a crash',
      );
    });
  });
}
