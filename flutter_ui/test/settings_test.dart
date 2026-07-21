// Settings defaults must reproduce the engine constants (the Rust
// settings.rs tests), and the workspace JSON must round-trip.

import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';

void main() {
  test('performance defaults match the hardcoded budgets', () {
    final p = PerformanceSettings();
    expect(p.ramBudgetMb, greaterThanOrEqualTo(2048));
    expect(p.diskCacheMb, 50 * 1024);
    expect(p.vramCacheMb, 512);
    expect(p.backgroundFill, isTrue);
    expect(p.cacheRoot, isNull);
  });

  test('interface defaults are a no-op for existing installs', () {
    final i = InterfaceSettings();
    expect(i.uiScale, 1.0);
    expect(i.showTooltips, isTrue);
  });

  test('autosave defaults mirror AUTOSAVE_INTERVAL_SECS/AUTOSAVE_KEEP', () {
    final a = AutosaveSettings();
    expect(a.intervalMins, 5);
    expect(a.keep, 3);
  });

  test('export defaults are a no-op for existing installs', () {
    final e = ExportSettings();
    expect(e.defaultPreset, ExportPreset.custom);
    expect(e.filenameTemplate, isNull);
  });

  test('export preset labels are the shipped ones', () {
    expect(ExportPreset.custom.label, 'Custom (comp size)');
    expect(ExportPreset.youtube1080p60.label, 'YouTube 1080p60');
    expect(ExportPreset.vertical1080p60.label, 'Vertical 1080×1920p60');
  });

  test('workspace JSON round-trips appearance and settings', () {
    final ws = Workspace();
    ws.colorScheme = LumitColorScheme.gruvboxDark;
    ws.themeShape = ThemeShape.round;
    ws.accentOverride = const Color(0xff804060);
    ws.animationLevel = AnimationLevel.minimal;
    ws.autosave.intervalMins = 9;
    ws.performance.diskCacheMb = 1234;
    ws.export.defaultPreset = ExportPreset.youtube4k60;
    ws.lastProjectPath = 'C:/edit/last.lum';
    ws.recompose();

    final j = ws.toJson();
    final back = Workspace()..applyJson(Map<String, dynamic>.from(j));
    expect(back.colorScheme, LumitColorScheme.gruvboxDark);
    expect(back.lastProjectPath, 'C:/edit/last.lum');
    expect(back.themeShape, ThemeShape.round);
    expect(back.animationLevel, AnimationLevel.minimal);
    expect(back.autosave.intervalMins, 9);
    expect(back.performance.diskCacheMb, 1234);
    expect(back.export.defaultPreset, ExportPreset.youtube4k60);
    expect((back.accentOverride!.r * 255).round(), 0x80);
    // The rebuilt theme carries the override and the shape tokens.
    expect(back.theme.tokens, ShapeTokens.round);
    expect((back.theme.accent.r * 255).round(), 0x80);
  });
}
