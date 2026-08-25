// The live-drag preview rate limit. The behaviour that matters is what happens
// to a tick that arrives too soon: it must be *held* and sent, not dropped —
// dropping the last delta of a drag is what made the preview stutter one step
// behind the pointer.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/preview_throttle.dart';

void main() {
  // Every test ends by cancelling: a send arms the interval, and flutter_test
  // fails a test that leaves a timer pending — which is exactly what a widget
  // does in `dispose`.
  test('the first tick goes at once', () {
    final sent = <int>[];
    PreviewThrottle()
      ..request(() => sent.add(1))
      ..cancel();
    expect(sent, [1]);
  });

  testWidgets('a tick inside the interval is held, and the newest one goes',
      (tester) async {
    final sent = <int>[];
    final throttle = PreviewThrottle(interval: const Duration(seconds: 1));

    throttle.request(() => sent.add(1));
    throttle.request(() => sent.add(2));
    throttle.request(() => sent.add(3));
    expect(sent, [1], reason: 'only the leading tick has gone out');
    expect(throttle.holding, isTrue);

    await tester.pump(const Duration(seconds: 1));
    expect(sent, [1, 3],
        reason: 'the newest held tick goes, not the one it superseded');
    expect(throttle.holding, isFalse);

    // And the interval starts again from that send.
    throttle.request(() => sent.add(4));
    expect(sent, [1, 3]);
    await tester.pump(const Duration(seconds: 1));
    expect(sent, [1, 3, 4]);
    throttle.cancel();
  });

  testWidgets('a cancelled gesture drops the held tick', (tester) async {
    final sent = <int>[];
    final throttle = PreviewThrottle(interval: const Duration(seconds: 1));
    throttle.request(() => sent.add(1));
    throttle.request(() => sent.add(2));
    throttle.cancel();
    await tester.pump(const Duration(seconds: 2));
    expect(sent, [1], reason: 'the commit is the last word on the gesture');
  });

  testWidgets('flush sends the held tick without waiting', (tester) async {
    final sent = <int>[];
    final throttle = PreviewThrottle(interval: const Duration(seconds: 1));
    throttle.request(() => sent.add(1));
    throttle.request(() => sent.add(2));
    throttle.flush();
    expect(sent, [1, 2]);
    await tester.pump(const Duration(seconds: 2));
    expect(sent, [1, 2], reason: 'flushing leaves nothing behind to fire');
    throttle.cancel();
  });
}
