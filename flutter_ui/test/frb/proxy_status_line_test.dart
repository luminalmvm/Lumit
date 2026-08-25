// MAKE-PROXY on the status line (K-501, K-504).
//
// **Why this is a test.** A proxy takes minutes and shows nothing while it
// runs, so the whole of what a user sees of it is this strip: what it is doing,
// how far along, and a Cancel that works from anywhere. The poll is injected,
// so no transcode has to run — what is being asserted is that each of the
// job's four states reaches the strip in its own words, and that the strip
// tells whoever is listening when the job stops (which is how the Project
// panel learns to re-read the item that just gained a proxy).

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('The status line\'s proxy job (frb)', () {
    Future<void> mount(WidgetTester tester, BridgeProxyState state) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: StatusLineFrb(proxyPollFn: () => state),
      ));
      await tester.pump();
    }

    testWidgets('a running transcode says how far it has got, with a Cancel',
        (tester) async {
      await mount(
          tester,
          BridgeProxyState.running(
              frame: BigInt.from(120), total: BigInt.from(500)));

      expect(
          find.byKey(const ValueKey('status-proxy-progress')), findsOneWidget);
      expect(find.byKey(const ValueKey('status-proxy-cancel')), findsOneWidget);
      final text = tester
          .widget<Text>(find.byKey(const ValueKey('status-proxy-progress')));
      expect(text.data, contains('120'));
      expect(text.data, contains('500'));
    });

    testWidgets('an idle job draws nothing at all', (tester) async {
      await mount(tester, const BridgeProxyState.idle());
      expect(find.byKey(const ValueKey('status-proxy-progress')), findsNothing);
      expect(find.byKey(const ValueKey('status-proxy-done')), findsNothing);
      expect(find.byKey(const ValueKey('status-proxy-failed')), findsNothing);
    });

    testWidgets('a finished job names the file it wrote', (tester) async {
      await mount(
          tester, const BridgeProxyState.done(path: 'C:/clips/shot_proxy.mov'));
      final text =
          tester.widget<Text>(find.byKey(const ValueKey('status-proxy-done')));
      expect(text.data, contains('shot_proxy.mov'));
    });

    /// Cancelling is not a failure, and it does not read as one.
    testWidgets('a cancelled job says so in its own words', (tester) async {
      await mount(tester, const BridgeProxyState.failed(error: 'cancelled'));
      final text = tester
          .widget<Text>(find.byKey(const ValueKey('status-proxy-failed')));
      expect(text.data, 'Proxy cancelled');
      expect(text.data, isNot(contains('failed')),
          reason: 'stopping a job on purpose is not a failure');
    });

    testWidgets('a failed job reports the engine\'s own reason',
        (tester) async {
      await mount(tester,
          const BridgeProxyState.failed(error: 'that footage is not here'));
      final text = tester
          .widget<Text>(find.byKey(const ValueKey('status-proxy-failed')));
      expect(text.data, contains('that footage is not here'));
    });

    /// The edge, not the tick: the strip tells listeners once, when the job
    /// stops running, so a panel re-reads once per job rather than twice a
    /// second for the length of one.
    testWidgets('the strip announces the job stopping, exactly once',
        (tester) async {
      final p = freshProject();
      var state = BridgeProxyState.running(
          frame: BigInt.from(1), total: BigInt.from(2));
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: StatusLineFrb(proxyPollFn: () => state),
      ));
      await tester.pump();

      final before = proxyJobChanged.value;
      // Still running: the tick changes nothing for a listener.
      proxyJobChanged.value++;
      await tester.pump();
      expect(proxyJobChanged.value, before + 1);

      state = const BridgeProxyState.done(path: 'C:/clips/shot_proxy.mov');
      final atFinish = proxyJobChanged.value;
      proxyJobChanged.value++;
      await tester.pump();
      expect(proxyJobChanged.value, atFinish + 2,
          reason: 'the strip added its own bump when the job stopped');

      // And no further bumps while it stays finished.
      final settled = proxyJobChanged.value;
      proxyJobChanged.value++;
      await tester.pump();
      expect(proxyJobChanged.value, settled + 1);
    });
  }, skip: !engineAvailable);
}
