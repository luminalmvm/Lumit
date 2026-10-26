// Settings → Account's Friends group.
//
// Asks wait for an answer, so they are in sight as the page opens. The
// friends themselves need nothing doing, so they are folded under their
// count until that is pressed.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/state/account.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  testWidgets('asks are in sight and the friends are folded until opened',
      (tester) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    final p = freshProject();
    // Signed in, with nothing to ask the server with: what is set here is
    // what the page draws.
    p.uiState.account.account = const CloudAccount(
        id: '1', email: 'a@example.com', name: 'Ana', username: 'ana');
    p.uiState.account.friends = const Friends(
      friends: [
        (username: 'jo', name: 'Jo'),
        (username: 'sam', name: 'Sam'),
        (username: 'li', name: 'Li'),
      ],
      asked: [(username: 'ben', name: 'Ben')],
      asking: [(username: 'cy', name: 'Cy')],
    );
    await tester.pumpWidget(hostPanel(
      child: Builder(
        builder: (context) => HouseButton(
          key: const ValueKey('open-settings'),
          onPressed: () =>
              showSettingsWindowFrb(context, initialPage: SettingsPage.account),
          child: const Text('Open'),
        ),
      ),
      state: p.state,
      uiState: p.uiState,
      size: const Size(1200, 900),
    ));
    await tester.tap(find.byKey(const ValueKey('open-settings')));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('settings-friend-accept-ben')),
        findsOneWidget);
    expect(find.byKey(const ValueKey('settings-friend-cancel-cy')),
        findsOneWidget);
    expect(find.text('Friends · 3'), findsOneWidget);
    expect(find.byKey(const ValueKey('settings-friend-jo')), findsNothing,
        reason: 'folded as the page opens');

    final fold = find.byKey(const ValueKey('settings-friends-fold'));
    await tester.ensureVisible(fold);
    await tester.tap(fold);
    await tester.pumpAndSettle();
    for (final name in ['jo', 'sam', 'li']) {
      expect(find.byKey(ValueKey('settings-friend-remove-$name')),
          findsOneWidget);
    }
    // Two to a line: the second sits beside the first, the third under it.
    final jo = tester.getTopLeft(find.byKey(const ValueKey('settings-friend-jo')));
    final sam =
        tester.getTopLeft(find.byKey(const ValueKey('settings-friend-sam')));
    final li = tester.getTopLeft(find.byKey(const ValueKey('settings-friend-li')));
    expect(sam.dy, jo.dy);
    expect(sam.dx, greaterThan(jo.dx));
    expect(li.dx, jo.dx);
    expect(li.dy, greaterThan(jo.dy));
  });
}
