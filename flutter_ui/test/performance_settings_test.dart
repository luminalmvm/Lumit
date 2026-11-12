// The drag settings After Effects CS6's Fast Previews gave: the adaptive
// resolution limit, wireframe drags and Caps Lock suspending the Viewer.
// Off and Auto when absent, so an older settings file behaves as it did.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/settings.dart';

void main() {
  test('an older file reads as Auto, no wireframe, no Caps Lock', () {
    final p = PerformanceSettings.fromJson(const {});
    expect(p.dragResolutionLimit, 0);
    expect(p.wireframeDrags, isFalse);
    expect(p.capsLockSuspends, isFalse);
  });

  test('the drag settings round-trip, and an odd limit is Auto', () {
    final p = PerformanceSettings(
        dragResolutionLimit: 8, wireframeDrags: true, capsLockSuspends: true);
    final back = PerformanceSettings.fromJson(p.toJson());
    expect(back.dragResolutionLimit, 8);
    expect(back.wireframeDrags, isTrue);
    expect(back.capsLockSuspends, isTrue);
    expect(
        PerformanceSettings.fromJson(const {'drag_resolution_limit': 3})
            .dragResolutionLimit,
        0);
  });
}
