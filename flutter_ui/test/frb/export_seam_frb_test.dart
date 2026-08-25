// The export seam's new surface (K-493, K-497, K-498, K-501, K-502).
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
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';

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
    // The spec check asks a *composition*, because whether a colour space can
    // be delivered is a question about that project's colour config (K-490).
    late CompositionReference comp;
    setUp(() {
      comp = LumitBridgeState.newProject().newComposition(name: 'Scene');
    });

    test('a spec that sets none of the new fields is the export we always had',
        () {
      final spec = plainSpec();
      expect(spec.resample, '');
      expect(spec.renderGuides, isFalse);
      expect(spec.motionBlur, 0);
      expect(spec.retimeBlend, 0);
      expect(spec.useProxies, isFalse);
      expect(comp.exportSpecCheck(spec: spec), '',
          reason: 'and the engine takes it without complaint');
    });

    test('the engine names the sound rates and the colour spaces it offers',
        () {
      expect(exportAudioRates(), [44100, 48000, 96000]);

      final mp4 = exportFormatCaps(codec: 'h264');
      expect(mp4.audio, isTrue);
      expect(mp4.audio24Bit, isFalse,
          reason: 'AAC stores coefficients, so it has no sample width to set');
      expect(mp4.colourSpaces, contains(''));
      expect(mp4.colourSpaces, contains('rec2020'));

      final wav = exportFormatCaps(codec: 'wav');
      expect(wav.audio24Bit, isTrue);
      expect(wav.colourSpaces, isEmpty, reason: 'a wav carries no picture');

      final png = exportFormatCaps(codec: 'png');
      expect(png.audio, isFalse);
      expect(png.audio24Bit, isFalse);
      expect(png.colourSpaces, [''],
          reason: 'a still can only be the space an untagged file is read as');
    });

    test('a setting the format cannot carry is refused in the footer words',
        () {
      expect(comp.exportSpecCheck(spec: plainSpec(audioDepth: 24)), isNotEmpty);
      expect(
        comp.exportSpecCheck(
          spec: plainSpec(
            codec: 'wav',
            audioDepth: 24,
            audioRate: 96000,
            audioChannels: 1,
          ),
        ),
        '',
      );
    });

    test('reordering an item the queue does not hold is a catchable refusal',
        () {
      expect(
        () => exportQueueMove(id: 4294967295, index: 0),
        throwsA(isA<Object>()),
        reason: 'a refusal crosses as an error, never as a crash',
      );
    });

    // `testWidgets` because opening a project sets the window title, which
    // needs the widget binding — not because anything here is drawn.
    testWidgets('the project-wide proxy switch reads and writes over the seam',
        (tester) async {
      final p = freshProject();
      final project = p.state.project!;
      expect(project.useProxies(), isTrue, reason: 'on by default');
      project.setUseProxies(useProxies: false);
      expect(project.useProxies(), isFalse);
      project.setUseProxies(useProxies: true);
      expect(project.useProxies(), isTrue);
    });

    test('the make-proxy job answers a state and cancels safely when idle', () {
      expect(
        proxyPoll(),
        anyOf(
          isA<BridgeProxyState_Idle>(),
          isA<BridgeProxyState_Running>(),
          isA<BridgeProxyState_Done>(),
          isA<BridgeProxyState_Failed>(),
        ),
      );
      proxyCancel();
      proxyCancel();
    });
  });
}
