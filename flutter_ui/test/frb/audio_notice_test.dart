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
  });
}
