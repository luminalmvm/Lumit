// The status line's output reading (the AudioWorkspace board's own status
// caption): while the comp plays, the strip's far right says what the master
// just peaked at, off the same tap the meters read. Idle, it says nothing —
// the strip must keep costing nothing while nothing moves.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  testWidgets('playback puts the output peak on the strip, in dB',
      (tester) async {
    final p = freshProject();
    // ~ -2.1 dB on the loudest channel; the master is the empty-id row.
    const meters = [
      BridgeAudioMeter(
        layer: '',
        peakLeft: 0.7852,
        peakRight: 0.5,
        rmsLeft: 0.4,
        rmsRight: 0.3,
        clipped: false,
      ),
    ];
    await tester.pumpWidget(hostPanel(
      state: p.state,
      uiState: p.uiState,
      child: StatusLineFrb(metersFn: () => meters),
    ));
    await tester.pump();

    expect(find.byKey(const ValueKey('status-output-peak')), findsNothing,
        reason: 'a still transport says nothing');

    p.uiState.playing.value = true;
    await tester.pump();
    final text =
        tester.widget<Text>(find.byKey(const ValueKey('status-output-peak')));
    expect(text.data, 'Output -2.1 dB peak',
        reason: 'the loudest channel, one decimal');

    // Stopping takes the reading down (and the strip\'s timer with it).
    p.uiState.playing.value = false;
    await tester.pump();
    expect(find.byKey(const ValueKey('status-output-peak')), findsNothing);
  });
}
