// The beat band (K-698, docs/09 §5): bar numbers from the confirmed grid in
// the ruler's lower row, and a gold tick per detected beat where an ordinary
// marker wears a flag.
//
// The end-to-end half runs real detection over a click-train WAV, against the
// real engine like every frb panel test: a band drawn from markers a test
// injected by hand would never prove the mint, the grid commit and the ruler
// agree about what a beat is.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/beats.dart';
import 'package:lumit_flutter/state/comp_time.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('beatBarLabels (K-698)', () {
    const grid = BridgeBeatGrid(bpm: 120, phaseSeconds: 0);

    test('a wide bar is labelled every bar, from bar one', () {
      // 120 BPM is a 2 s bar; at 30 fps and 2 px a frame that is 120 px.
      final labels = beatBarLabels(
          grid: grid, fps: 30, perFrame: 2, untilFrame: 3600);
      expect(labels.first, (bar: 1, frame: 0.0));
      expect(labels[1], (bar: 2, frame: 60.0));
      expect(labels[2], (bar: 3, frame: 120.0));
      expect(labels.last.frame, lessThanOrEqualTo(3600));
    });

    test('narrow bars double the step until the numbers clear each other', () {
      // 3 px a bar: 1, 2, 4, 8, 16 — sixteen bars is the first step past
      // the 34 px a label needs.
      final labels = beatBarLabels(
          grid: grid, fps: 30, perFrame: 0.05, untilFrame: 100000);
      expect(labels[0].bar, 1);
      expect(labels[1].bar, 17);
      expect(labels[2].bar, 33);
    });

    test('the phase moves bar one, and bars before time zero are not drawn',
        () {
      final nudged = beatBarLabels(
        grid: const BridgeBeatGrid(bpm: 120, phaseSeconds: 0.5),
        fps: 30,
        perFrame: 2,
        untilFrame: 3600,
      );
      expect(nudged.first, (bar: 1, frame: 15.0));

      final early = beatBarLabels(
        grid: const BridgeBeatGrid(bpm: 120, phaseSeconds: -3.0),
        fps: 30,
        perFrame: 2,
        untilFrame: 3600,
      );
      expect(early.first.bar, 3, reason: 'bars one and two fall before zero');
      expect(early.first.frame, greaterThanOrEqualTo(0));
    });

    test('no grid, no rate or no room is an empty answer, calmly', () {
      expect(
          beatBarLabels(grid: grid, fps: 0, perFrame: 2, untilFrame: 100),
          isEmpty);
      expect(
          beatBarLabels(grid: grid, fps: 30, perFrame: 0, untilFrame: 100),
          isEmpty);
    });
  });

  group('The beat band on the ruler (K-698)', () {
    testWidgets(
        'detection ticks the beats, numbers the bars, and Clear takes both',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Cut');
      p.uiState.setSelectedComp(comp);
      final music =
          p.state.project!.importFootage(path: _clickWavFile('clicks.wav'));
      comp.addFootageLayer(footage: music, asSequence: false);
      p.uiState.model.refresh();

      // Real detection over the click train, with the BPM well filled so the
      // grid is the confirmed 120 rather than an estimate of a 3 s file.
      final found = await tester.runAsync(() => comp.detectBeats(
            options: const BridgeBeatOptions(
              sourceLayer: '',
              sensitivityPercent: 80,
              workAreaOnly: false,
              minSpacingMs: 200,
              bpmOverride: 120,
              phaseMs: 0,
            ),
          ));
      expect(found!.placed, greaterThan(0),
          reason: 'a click train has beats to find');
      expect(comp.getBeatGrid()!.bpm, closeTo(120, 1e-9),
          reason: 'the run confirmed the grid it was given');

      clearCompTimeCache();
      p.uiState.model.refresh();
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();

      expect(find.byType(BeatTick), findsWidgets,
          reason: 'each detected beat wears a tick');
      expect(find.byType(MarkerFlag), findsNothing,
          reason: 'and none of them wears a marker flag');
      expect(find.byKey(const ValueKey('tl-beat-bars')), findsOneWidget,
          reason: 'the confirmed grid numbers the bars');

      // Clear generated: the ticks go, and the band's numbers go with the
      // grid that described them.
      comp.clearBeatMarkers();
      clearCompTimeCache();
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.byType(BeatTick), findsNothing);
      expect(find.byKey(const ValueKey('tl-beat-bars')), findsNothing);
      expect(comp.getBeatGrid(), isNull);
    });
  });
}

/// A real, probeable WAV holding a click train: three seconds of 8 kHz mono
/// with a 30 ms burst every half second — beats a detector cannot miss, at a
/// tempo the test can name. Written synchronously, like every test fixture
/// here (an awaited async `dart:io` call in a `testWidgets` body hangs).
String _clickWavFile(String name) {
  final dir = Directory.systemTemp.createTempSync('lumit-beats');
  final file = File('${dir.path}/$name');
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
