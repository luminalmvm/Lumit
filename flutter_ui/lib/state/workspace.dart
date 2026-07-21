// The workspace controller: everything the egui `Shell` persists (dock
// layout, colour scheme, shape, accent override, animation level, the
// settings structs), held in one ChangeNotifier and written to a JSON file —
// the Flutter counterpart of eframe's storage (docs/flutter-port/03).

import 'dart:convert';
import 'dart:io';
import 'dart:ui';

import 'package:flutter/foundation.dart';

import '../theme/theme.dart';
import 'dock.dart';
import 'settings.dart';

class Workspace extends ChangeNotifier {
  DockSplit dock = defaultLayout();
  LumitColorScheme colorScheme = LumitColorScheme.dark;
  ThemeShape themeShape = ThemeShape.sharp;
  Color? accentOverride;
  AnimationLevel animationLevel = AnimationLevel.all;

  PerformanceSettings performance = PerformanceSettings();
  AutosaveSettings autosave = AutosaveSettings();
  InterfaceSettings interface = InterfaceSettings();
  ExportSettings export = ExportSettings();

  /// The project last opened or saved with a path, restored on the next launch
  /// (the egui frontend reopens the last project the same way). Null until a
  /// project has been opened or saved to a file. This is only the *file* — the
  /// per-project session (open comps, playhead, selection) is a later slice.
  String? lastProjectPath;

  LumitTheme _theme = LumitTheme.dark();
  LumitTheme get theme => _theme;

  Workspace() {
    recompose();
  }

  /// Rebuild the theme from the current appearance fields — the single funnel
  /// every Appearance control uses (`Shell::recompose`).
  void recompose() {
    _theme = LumitTheme.forScheme(
      colorScheme,
      themeShape,
      accentOverride: accentOverride,
    );
    notifyListeners();
  }

  void setScheme(LumitColorScheme s) {
    colorScheme = s;
    recompose();
    save();
  }

  void setShape(ThemeShape s) {
    themeShape = s;
    recompose();
    save();
  }

  void setAccent(Color? c) {
    accentOverride = c;
    recompose();
    save();
  }

  void setAnimationLevel(AnimationLevel a) {
    animationLevel = a;
    notifyListeners();
    save();
  }

  void resetWorkspaceLayout() {
    dock = defaultLayout();
    notifyListeners();
    save();
  }

  void touch() {
    notifyListeners();
    save();
  }

  /// Remember the file a project was just opened from or saved to, so the next
  /// launch can reopen it. Persisted immediately; no theme rebuild is needed, so
  /// this does not notify listeners.
  void rememberProject(String path) {
    lastProjectPath = path;
    save();
  }

  // --- Persistence ---------------------------------------------------------

  /// `%APPDATA%\lumit\flutter-workspace.json` on Windows; a dotfolder
  /// fallback elsewhere. No plugin needed, and nothing machine-specific ever
  /// enters the repository.
  static File storeFile() {
    final base = Platform.environment['APPDATA'] ??
        '${Platform.environment['HOME'] ?? '.'}/.config';
    return File('$base${Platform.pathSeparator}lumit'
        '${Platform.pathSeparator}flutter-workspace.json');
  }

  Map<String, dynamic> toJson() => {
        'version': 1,
        'dock': dock.toJson(),
        'color_scheme': colorScheme.name,
        'theme_shape': themeShape.name,
        'accent_override': accentOverride == null
            ? null
            : [
                (accentOverride!.r * 255).round(),
                (accentOverride!.g * 255).round(),
                (accentOverride!.b * 255).round(),
              ],
        'animation_level': animationLevel.name,
        'performance': performance.toJson(),
        'autosave': autosave.toJson(),
        'interface': interface.toJson(),
        'export': export.toJson(),
        'last_project_path': lastProjectPath,
      };

  void applyJson(Map<String, dynamic> j) {
    final d = j['dock'];
    if (d is Map<String, dynamic>) {
      final parsed = DockNode.fromJson(d);
      if (parsed is DockSplit) dock = parsed;
    }
    colorScheme = LumitColorScheme.values.asNameMap()[j['color_scheme']] ??
        LumitColorScheme.dark;
    themeShape =
        ThemeShape.values.asNameMap()[j['theme_shape']] ?? ThemeShape.sharp;
    final acc = j['accent_override'];
    accentOverride = acc is List && acc.length == 3
        ? Color.fromARGB(0xff, acc[0] as int, acc[1] as int, acc[2] as int)
        : null;
    animationLevel = AnimationLevel.values.asNameMap()[j['animation_level']] ??
        AnimationLevel.all;
    if (j['performance'] is Map<String, dynamic>) {
      performance = PerformanceSettings.fromJson(j['performance']);
    }
    if (j['autosave'] is Map<String, dynamic>) {
      autosave = AutosaveSettings.fromJson(j['autosave']);
    }
    if (j['interface'] is Map<String, dynamic>) {
      interface = InterfaceSettings.fromJson(j['interface']);
    }
    if (j['export'] is Map<String, dynamic>) {
      export = ExportSettings.fromJson(j['export']);
    }
    lastProjectPath =
        j['last_project_path'] is String ? j['last_project_path'] as String : null;
    // The left group always opens on Project (activate_panel_tab at start-up).
    activatePanelTab(dock, Panel.project);
    recompose();
  }

  void load() {
    try {
      final f = storeFile();
      if (!f.existsSync()) return;
      final j = jsonDecode(f.readAsStringSync());
      if (j is Map<String, dynamic>) applyJson(j);
    } catch (_) {
      // A corrupt store falls back to defaults — never a crash.
    }
  }

  void save() {
    try {
      final f = storeFile();
      f.parent.createSync(recursive: true);
      f.writeAsStringSync(const JsonEncoder.withIndent('  ').convert(toJson()));
    } catch (_) {
      // Persistence is best-effort; the session keeps working without it.
    }
  }
}
