// The Audio panel's refusals — and its one success sentence — reach the
// status line.
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

    // The success is said too (the AudioWorkspace board's own status caption):
    // a run that placed markers and confirmed a grid used to land without a
    // word, and the markers land off-screen as easily as on.
    testWidgets('a run that confirmed a grid says so, tempo and count',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Cut');
      p.uiState.setSelectedComp(comp);
      final music = p.state.project!.importFootage(path: _clickWavFile());
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

      // The default sensitivity, deliberately: a click train is unmissable,
      // and the sentence must not hinge on a tuned threshold.
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
        matches(RegExp(r'^Beat grid confirmed at \d+ BPM · \d+ markers$')),
        reason: 'a run that placed markers says what it confirmed',
      );
      expect(comp.getBeatGrid(), isNotNull,
          reason: 'the sentence describes a grid the document really keeps');
    });
  });
}

/// A real, probeable WAV holding a click train: three seconds of 8 kHz mono
/// with a 30 ms burst every half second — beats a detector cannot miss.
/// Written synchronously, like every fixture here.
String _clickWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-notice-beats');
  final file = File('${dir.path}/clicks.wav');
  const rate = 8000;
  const seconds = 3;
  const samples = rate * seconds;
  const dataBytes = samples * 2;
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
  final data = Uint8List(dataBytes);
  for (var click = 0; click < seconds * 2; click++) {
    final start = click * rate ~/ 2;
    for (var i = 0; i < 240; i++) {
      final v = i.isEven ? 20000 : -20000;
      data[(start + i) * 2] = v & 0xff;
      data[(start + i) * 2 + 1] = (v >> 8) & 0xff;
    }
  }
  out.add(data);
  file.writeAsBytesSync(out.toBytes());
  return file.path;
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
