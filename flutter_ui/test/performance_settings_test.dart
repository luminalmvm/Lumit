// The drag settings After Effects CS6's Fast Previews gave: the adaptive
// resolution limit, wireframe drags and Caps Lock suspending the Viewer.
// Off and Auto when absent, so an older settings file behaves as it did.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/settings.dart';

void main() {
  test('an older file reads as Auto, no wireframe, Caps Lock on, shown size on',
      () {
    final p = PerformanceSettings.fromJson(const {});
    expect(p.dragResolutionLimit, 0);
    expect(p.wireframeDrags, isFalse);
    expect(p.capsLockSuspends, isTrue);
    expect(p.decodeAtShownSize, isTrue);
  });

  test('a false written under the old Caps Lock key is not a choice', () {
    final p = PerformanceSettings.fromJson(const {'caps_lock_suspends': false});
    expect(p.capsLockSuspends, isTrue);
  });

  test('the drag settings round-trip, and an odd limit is Auto', () {
    final p = PerformanceSettings(
        dragResolutionLimit: 8,
        wireframeDrags: true,
        capsLockSuspends: false,
        decodeAtShownSize: false);
    final back = PerformanceSettings.fromJson(p.toJson());
    expect(back.dragResolutionLimit, 8);
    expect(back.wireframeDrags, isTrue);
    expect(back.capsLockSuspends, isFalse);
    expect(back.decodeAtShownSize, isFalse);
    expect(
        PerformanceSettings.fromJson(const {'drag_resolution_limit': 3})
            .dragResolutionLimit,
        0);
  });
}
