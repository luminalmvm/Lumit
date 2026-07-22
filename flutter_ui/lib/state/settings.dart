// Application-wide settings, ported from crates/lumit-ui/src/shell/settings.rs.
// Defaults reproduce the engine's constants exactly (pinned by tests), so a
// fresh install behaves like the egui frontend's.

import 'dart:io';

/// Export presets, mirroring `crate::export::ExportPreset` (labels verbatim).
enum ExportPreset {
  custom,
  youtube1080p60,
  youtube1440p60,
  youtube4k60,
  vertical1080p60;

  String get label => switch (this) {
        ExportPreset.custom => 'Custom (comp size)',
        ExportPreset.youtube1080p60 => 'YouTube 1080p60',
        ExportPreset.youtube1440p60 => 'YouTube 1440p60',
        ExportPreset.youtube4k60 => 'YouTube 4K60',
        ExportPreset.vertical1080p60 => 'Vertical 1080×1920p60',
      };
}

/// Frame-cache budgets (docs/06 §5). One RAM budget for everything cached in
/// memory; disk and video-memory caps beside it.
class PerformanceSettings {
  int ramBudgetMb;
  int diskCacheMb;
  int vramCacheMb;
  bool backgroundFill;
  String? cacheRoot;

  PerformanceSettings({
    int? ramBudgetMb,
    this.diskCacheMb = 50 * 1024,
    this.vramCacheMb = 512,
    this.backgroundFill = true,
    this.cacheRoot,
  }) : ramBudgetMb = ramBudgetMb ?? defaultRamBudgetMb();

  /// Half the machine's memory in MiB, floored at 2 GiB — the same default
  /// the Rust side computes with sysinfo. Dart has no portable total-memory
  /// query without a plugin, so each desktop OS gets a cheap native probe:
  /// on Windows we ask WMIC's successor via PowerShell once; on Linux we read
  /// `/proc/meminfo`. Either falls back to the floor if it fails.
  static int? _totalMbCache;
  static int defaultRamBudgetMb() {
    _totalMbCache ??= _queryTotalMb();
    final half = (_totalMbCache! / 2).floor();
    return half < 2048 ? 2048 : half;
  }

  static int _queryTotalMb() {
    try {
      if (Platform.isWindows) {
        final r = Process.runSync('powershell', [
          '-NoProfile',
          '-Command',
          '(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory',
        ]);
        final bytes = int.tryParse((r.stdout as String).trim());
        if (bytes != null) return bytes ~/ (1024 * 1024);
      } else if (Platform.isLinux) {
        // `MemTotal:  <kB> kB` is the first line of /proc/meminfo. No plugin
        // and no process spawn — a single small file read.
        final line = File('/proc/meminfo')
            .readAsLinesSync()
            .firstWhere((l) => l.startsWith('MemTotal:'), orElse: () => '');
        final match = RegExp(r'(\d+)').firstMatch(line);
        if (match != null) {
          final kb = int.tryParse(match.group(1)!);
          if (kb != null) return kb ~/ 1024;
        }
      }
    } catch (_) {}
    return 4096; // floor fallback: yields the 2048 MiB minimum budget
  }

  Map<String, dynamic> toJson() => {
        'ram_budget_mb': ramBudgetMb,
        'disk_cache_mb': diskCacheMb,
        'vram_cache_mb': vramCacheMb,
        'background_fill': backgroundFill,
        'cache_root': cacheRoot,
      };

  factory PerformanceSettings.fromJson(Map<String, dynamic> j) =>
      PerformanceSettings(
        ramBudgetMb: j['ram_budget_mb'] as int?,
        diskCacheMb: j['disk_cache_mb'] as int? ?? 50 * 1024,
        vramCacheMb: j['vram_cache_mb'] as int? ?? 512,
        backgroundFill: j['background_fill'] as bool? ?? true,
        cacheRoot: j['cache_root'] as String?,
      );
}

/// Autosave (Settings → General). Defaults mirror AUTOSAVE_INTERVAL_SECS
/// (300 s) and AUTOSAVE_KEEP (3).
class AutosaveSettings {
  int intervalMins;
  int keep;
  AutosaveSettings({this.intervalMins = 5, this.keep = 3});

  Map<String, dynamic> toJson() => {'interval_mins': intervalMins, 'keep': keep};
  factory AutosaveSettings.fromJson(Map<String, dynamic> j) => AutosaveSettings(
        intervalMins: j['interval_mins'] as int? ?? 5,
        keep: j['keep'] as int? ?? 3,
      );
}

/// Interface (Settings → Interface): UI scale and tooltips (K-117).
class InterfaceSettings {
  double uiScale;
  bool showTooltips;
  InterfaceSettings({this.uiScale = 1.0, this.showTooltips = true});

  Map<String, dynamic> toJson() =>
      {'ui_scale': uiScale, 'show_tooltips': showTooltips};
  factory InterfaceSettings.fromJson(Map<String, dynamic> j) =>
      InterfaceSettings(
        uiScale: (j['ui_scale'] as num?)?.toDouble() ?? 1.0,
        showTooltips: j['show_tooltips'] as bool? ?? true,
      );
}

/// Export defaults (Settings → Export, K-119).
class ExportSettings {
  ExportPreset defaultPreset;
  String? filenameTemplate;
  ExportSettings({this.defaultPreset = ExportPreset.custom, this.filenameTemplate});

  Map<String, dynamic> toJson() => {
        'default_preset': defaultPreset.name,
        'filename_template': filenameTemplate,
      };
  factory ExportSettings.fromJson(Map<String, dynamic> j) => ExportSettings(
        defaultPreset: ExportPreset.values.asNameMap()[j['default_preset']] ??
            ExportPreset.custom,
        filenameTemplate: j['filename_template'] as String?,
      );
}
