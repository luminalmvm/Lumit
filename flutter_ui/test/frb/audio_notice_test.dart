// The Audio panel's refusals reach the status line.
//
// Beat detection on a composition with nothing sounding in it refuses
// (`BridgeError::NoAudio`, docs/09 §5) — and the three call sites that ask for
// it used to swallow that with `onError: (_) {}`, so Generate placed no
// markers, cleared the grid, and said nothing at all. A soloed picture row
// (K-435) silences the mix, which is the everyday way to reach it by accident.
//
// The panel does not draw the notice — the status line does — so what is
// asserted here is the notice landing on the shell state, which is the road it
// travels.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_meters_feed.dart';
import 'package:lumit_flutter/panels/audio_panel_frb.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Audio panel notices (frb)', () {
    testWidgets('a silent comp explains itself when Generate is pressed',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Cut');
      // A solid makes a picture and no sound, so the mix has nothing in it.
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);
      p.uiState.model.refresh();
      final feed = AudioMeterFeed();
      feed.read = () => const [];
      addTearDown(feed.dispose);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: AudioPanelFrb(feed: feed),
      ));
      await settleFrb(tester, minRounds: 4);

      expect(p.state.notice.value, isNull, reason: 'nothing has refused yet');

      await tester.tap(find.byKey(const ValueKey('beats-generate')));
      await tester.pump();
      await settleFrb(
        tester,
        minRounds: 6,
        until: () => p.state.notice.value != null,
        maxRounds: coldWorkerRounds,
      );

      expect(
        p.state.notice.value?.message,
        'No beats: nothing in this composition is sounding — a mute or a '
            'solo can silence the mix.',
        reason: 'a refused detection says why rather than doing nothing',
      );
    });

    // A layer picked by name is heard through a mute and through somebody
    // else's solo (K-718), so a named source that still finds nothing has
    // nothing in it — and blaming the switches would send the reader off to
    // press one that changes nothing.
    testWidgets('a named source that has no sound blames the layer, not a mute',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Cut');
      p.uiState.setSelectedComp(comp);
      final music = p.state.project!.importFootage(path: _silentWavFile());
      comp.addFootageLayer(footage: music, asSequence: false);
      p.uiState.model.refresh();
      final feed = AudioMeterFeed();
      feed.read = () => const [];
      addTearDown(feed.dispose);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: AudioPanelFrb(feed: feed),
      ));
      await settleFrb(tester, minRounds: 4);

      // Pick the row by name in the Source dropdown.
      await tester.tap(find.byKey(const ValueKey('beats-source')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('tone.wav').last);
      await tester.pumpAndSettle();

      // The row goes; the panel keeps the choice, which is how a source with
      // nothing to hear is reached without touching a switch.
      comp.getLayers().first.delete();
      p.uiState.model.refresh();
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('beats-generate')));
      await tester.pump();
      await settleFrb(
        tester,
        minRounds: 6,
        until: () => p.state.notice.value != null,
        maxRounds: coldWorkerRounds,
      );

      expect(
        p.state.notice.value?.message,
        'No beats: the layer chosen as the source has no sound in it.',
        reason: 'the refusal names the source that was picked',
      );
    });
  });
}

/// A real, probeable WAV: a second of 8 kHz mono silence, which is enough for
/// the probe to say the layer can make a sound. Written synchronously, like
/// every fixture here (an awaited async `dart:io` call in a `testWidgets` body
/// hangs).
String _silentWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-beats-notice');
  final file = File('${dir.path}/tone.wav');
  const rate = 8000;
  const dataBytes = rate * 2;
  final out = BytesBuilder();
  void ascii(String s) => out.add(s.codeUnits);
  void u16(int v) => out.add([v & 0xff, (v >> 8) & 0xff]);
  void u32(int v) =>
      out.add([v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff]);
  ascii('RIFF');
  u32(36 + dataBytes);
  ascii('WAVE');
  ascii('fmt ');
  u32(16);
  u16(1); // PCM
  u16(1); // mono
  u32(rate);
  u32(rate * 2);
  u16(2);
  u16(16);
  ascii('data');
  u32(dataBytes);
  out.add(Uint8List(dataBytes));
  file.writeAsBytesSync(out.toBytes());
  return file.path;
}
