// Low-spec mode on battery: Low-spec mode follows the power only with the
// switch on, undoes only what the battery turned on, and leaves a hand on the
// switch alone until the power has come and gone again.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/settings.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  test('the battery settings round-trip and start off', () {
    expect(PerformanceSettings.fromJson(const {}).lowSpecOnBattery, isFalse);
    final p = PerformanceSettings(
      lowSpecOnBattery: true,
      lowSpecBatteryOffered: true,
      lowSpecByBattery: true,
    );
    final back = PerformanceSettings.fromJson(p.toJson());
    expect(back.lowSpecOnBattery, isTrue);
    expect(back.lowSpecBatteryOffered, isTrue);
    expect(back.lowSpecByBattery, isTrue);
  });

  testWidgets('with the switch off, the battery changes nothing',
      (tester) async {
    final ui = freshProject().uiState;
    ui.followBattery(true);
    expect(ui.workspace.performance.lowSpecMode, isFalse);
  });

  testWidgets('on battery it comes on, and off again when plugged in',
      (tester) async {
    final ui = freshProject().uiState;
    final perf = ui.workspace.performance;
    ui.setLowSpecOnBattery(true);
    ui.followBattery(true);
    expect(perf.lowSpecMode, isTrue);
    expect(perf.lowSpecByBattery, isTrue);
    ui.followBattery(false);
    expect(perf.lowSpecMode, isFalse);
    expect(perf.lowSpecByBattery, isFalse);
  });

  testWidgets('a Low-spec mode the user turned on stays on when plugged in',
      (tester) async {
    final ui = freshProject().uiState;
    final perf = ui.workspace.performance;
    ui.setLowSpecMode(true);
    ui.setLowSpecOnBattery(true);
    ui.followBattery(true);
    ui.followBattery(false);
    expect(perf.lowSpecMode, isTrue, reason: 'the user turned it on');
  });

  testWidgets(
      'turned off by hand on battery, it stays off until the power changes',
      (tester) async {
    final ui = freshProject().uiState;
    final perf = ui.workspace.performance;
    ui.setLowSpecOnBattery(true);
    ui.followBattery(true);
    ui.setLowSpecMode(false);
    ui.followBattery(true);
    expect(perf.lowSpecMode, isFalse, reason: 'the next reading respects it');
    ui.followBattery(false);
    ui.followBattery(true);
    expect(perf.lowSpecMode, isTrue, reason: 'a new time on battery');
  });
}
