// The spectral lane mode: plain wave / multiwave stack / spectrogram
// per layer, cycled by the chip on the Waveform row — and the toggle repaints
// the lane without rebuilding the table, which is the discipline the chip
// has to keep.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/spectral_lane_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';

import 'frb_test_support.dart';

/// Counts widget rebuilds by name, from the framework's own log — the
/// rebuild-budget test's counter, at the size this file needs.
class _Rebuilds {
  final Map<String, int> byName = {};
  bool counting = false;
  DebugPrintCallback? _previous;

  void install() {
    _previous = debugPrint;
    debugPrint = (String? message, {int? wrapWidth}) {
      if (!counting || message == null) return;
      var line = message;
      final tail = line.lastIndexOf('): ');
      if (tail >= 0) line = line.substring(tail + 3);
      line = line.replaceFirst(RegExp(r'^(Building|Rebuilding)\s+'), '');
      final name = line.trim().split(RegExp(r'[\s(<{-]')).first;
      byName[name] = (byName[name] ?? 0) + 1;
    };
    debugPrintRebuildDirtyWidgets = true;
  }

  void remove() {
    debugPrintRebuildDirtyWidgets = false;
    if (_previous != null) debugPrint = _previous!;
  }
}

void main() {
  setUpAll(initEngineForTests);

  testWidgets(
      'the chip cycles the three pictures without rebuilding the table',
      (tester) async {
    laneModes.reset();
    addTearDown(laneModes.reset);
    final rebuilds = _Rebuilds()..install();

    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Cut');
    p.uiState.setSelectedComp(comp);
    final music = p.state.project!.importFootage(path: _toneWavFile());
    comp.addFootageLayer(footage: music, asSequence: false);
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
    await settleFrb(tester, minRounds: 8);

    final layer = comp.getLayers().first;
    final id = layer.internallayerId;
    await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
    await tester.pump();
    await tester.tap(find.text('Audio'));
    await tester.pump();
    await tester.tap(find.text('Waveform'));
    await tester.pump();

    final chip = find.byKey(ValueKey<String>('tl-lane-mode-$id'));
    expect(chip, findsOneWidget, reason: 'the lane row carries its chip');
    expect(find.text('Stack'), findsOneWidget,
        reason: 'the Settings multiwave default is the stack');
    expect(find.byKey(ValueKey<String>('tl-wave-$id')), findsOneWidget);

    // --- The toggle: spectral on, and the table untouched by it.
    rebuilds.counting = true;
    await tester.tap(chip);
    await tester.pump();
    rebuilds.counting = false;
    // Back off the foundation debug variable here and not in a tear-down:
    // the framework refuses to end a test with it still set.
    rebuilds.remove();

    expect(find.text('Spectral'), findsOneWidget);
    expect(find.byKey(ValueKey<String>('tl-spectral-$id')), findsOneWidget,
        reason: 'the lane switched to the spectrogram');
    expect(find.byKey(ValueKey<String>('tl-wave-$id')), findsNothing);
    expect(rebuilds.byName['OutlineRow'] ?? 0, 0,
        reason: 'a lane-mode toggle rebuilds no layer row');
    expect(rebuilds.byName['Bar'] ?? 0, 0,
        reason: 'and no bar — the toggle is the lane\'s own business');

    // --- The engine's picture is real: the same window fetch the peaks
    // take, answered in columns of bands with signal in them.
    final grid = await tester.runAsync(() => layer.audioSpectrogram(
          startSeconds: 0,
          endSeconds: 0.4,
          columns: 64,
        ));
    expect(grid!.columns, 64);
    expect(grid.bins, greaterThan(0));
    expect(grid.values, hasLength(64 * grid.bins));
    expect(grid.values.any((v) => v > 0), isTrue,
        reason: 'a tone is not silence');

    // --- The cycle carries on: spectral → wave → stack.
    await tester.tap(chip);
    await tester.pump();
    expect(find.text('Wave'), findsOneWidget);
    expect(find.byKey(ValueKey<String>('tl-wave-$id')), findsOneWidget,
        reason: 'the plain wave is back');
    await tester.tap(chip);
    await tester.pump();
    expect(find.text('Stack'), findsOneWidget);
  });
}

/// A real, probeable WAV: half a second of 8 kHz mono square wave, written
/// synchronously like every fixture here.
String _toneWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-spectral');
  final file = File('${dir.path}/tone.wav');
  const rate = 8000;
  const samples = 4000;
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
  u16(1);
  u16(1);
  u32(rate);
  u32(rate * 2);
  u16(2);
  u16(16);
  ascii('data');
  u32(dataBytes);
  final data = Uint8List(dataBytes);
  for (var i = 0; i < samples; i++) {
    final v = (i ~/ 9).isEven ? 12000 : -12000;
    data[i * 2] = v & 0xff;
    data[i * 2 + 1] = (v >> 8) & 0xff;
  }
  out.add(data);
  file.writeAsBytesSync(out.toBytes());
  return file.path;
}
