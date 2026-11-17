// Deleting words in the Transcript panel cuts exactly their stretch out of
// the clips, and one undo puts it back. The one check here guards lost work:
// the words a person picks are the media that goes, no more and no less.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/transcript_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/cut/transcript.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/transcript.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Transcript panel (frb)', () {
    testWidgets('Delete takes the selected words out of the clips, undo puts them back',
        (tester) async {
      final p = freshProject();
      final project = p.state.project!;
      final comp = project.newComposition(name: 'Talk');
      final footage = project.importFootage(path: _toneWavFile());
      comp.addFootageLayer(footage: footage, asSequence: true);
      // Five words, a tenth of a second each, over the half-second file.
      BridgeRational tenths(int n) => BridgeRational(num: n, den: 10);
      const said = ['one', 'two', 'three', 'four', 'five'];
      footage.setTranscript(
        transcript: BridgeTranscript(
          language: 'en',
          stream: 0,
          model: 'test',
          speakers: const [],
          words: [
            for (final (i, text) in said.indexed)
              BridgeWord(
                text: text,
                start: tenths(i),
                end: tenths(i + 1),
                confidence: 1,
                paragraph: i == 0,
              ),
          ],
        ),
      );
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const TranscriptPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(tester,
          until: () =>
              find.byKey(const ValueKey('transcript-page')).evaluate().isNotEmpty);

      // The stretch the engine says "two" and "three" take.
      final read = await tester.runAsync(() => comp.cutTranscript(
          look: const BridgeSpokenLook(pauseMs: 500, showRemoved: false)));
      final words = read!.tokens
          .where((t) => t.kind == BridgeSpokenKind.word)
          .toList();
      expect(words.map((t) => t.text), said);
      final start = words[1].startFrame;
      final taken = words[2].endFrame - start;
      expect(taken, greaterThan(0));

      // A footage layer carries no clips until a cut makes it a Sequence
      // layer, so the length to check against is the layer's own span.
      ({int start, int end}) span() {
        final info = p.uiState.model.layers.single.info;
        return (start: info.inFrame, end: info.outFrame);
      }
      List<(int, int)> clips() => (comp.getLayers().single.getClips()
            ..sort((a, b) => a.startFrame.compareTo(b.startFrame)))
          .map((c) => (c.startFrame, c.endFrame))
          .toList();
      final before = span();
      final length = before.end - before.start;
      expect(before.start, 0);
      expect(length, greaterThan(taken));

      // The caret after the first word, then two words taken with Shift.
      p.uiState.activePane.value = Panel.transcript.pane();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.home);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      expect(p.uiState.transcriptSpan.value, (start: start, end: start + taken),
          reason: 'the Cut timeline is told the selected words\' stretch');

      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await settleFrb(tester);

      // Exactly that stretch has gone: the clip is cut where the words
      // started and what followed them has moved up by their length.
      expect(clips(), [(0, start), (start, length - taken)]);

      project.undo();
      await settleFrb(tester);
      expect(span(), before);
      final again = await tester.runAsync(() => comp.cutTranscript(
          look: const BridgeSpokenLook(pauseMs: 500, showRemoved: false)));
      expect(
          again!.tokens
              .where((t) => t.kind == BridgeSpokenKind.word)
              .map((t) => t.text),
          said);
    });
  }, skip: !engineAvailable);
}

/// A real, probeable WAV: half a second of 8 kHz mono square wave. Written
/// synchronously, as the other frb tests write theirs.
String _toneWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-transcript');
  final file = File('${dir.path}/talk.wav');
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
