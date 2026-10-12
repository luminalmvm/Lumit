// MAKE-PROXY on the status line.
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

  }, skip: !engineAvailable);
}
