// The screen that stands in for the editor when Lumit could not start.
//
// Worth its own test because the alternative is what was reported against
// 0.3.0: the window stays invisible until Flutter's first frame, so an error
// on the way up showed nothing at all — a process in Task Manager and no way
// to tell what had happened.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/startup_failure.dart';

void main() {
  testWidgets('a start that failed says so, with the reason on screen',
      (tester) async {
    await tester.pumpWidget(
      const StartupFailureApp('the engine library would not load'),
    );

    expect(find.text(l10n.startupFailedTitle), findsOneWidget);
    expect(
      find.textContaining('the engine library would not load'),
      findsOneWidget,
      reason: 'the reason is the whole point of the screen',
    );
    expect(
      find.textContaining('lumit-diagnostics.log'),
      findsOneWidget,
      reason: 'a bug report needs to be told where the file is',
    );
  });
}
