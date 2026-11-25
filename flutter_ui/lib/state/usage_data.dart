import 'dart:async';
import 'dart:convert';
import 'dart:ffi' show Abi;
import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../src/rust/api/keymap.dart' show BridgeKeymapPreset;
import 'account.dart' show cloudUrl;
import 'dock.dart' show PaneId, WorkspacePreset;
import 'updates.dart' show isNightlyVersion;
import 'workspace.dart';

const bool usageCollectorLive = false;

typedef UsageMachine = ({int ram, int vram, int vendor, String gpu});

class UsageData {
  final Workspace workspace;

  final String? Function() version;

  final File Function() store;

  final UsageMachine Function()? machine;

  UsageData({
    required this.workspace,
    required this.version,
    File Function()? store,
    this.machine,
  }) : store = store ??
            (() => File('${Workspace.storeFile().parent.path}'
                '${Platform.pathSeparator}usage.json')) {
    _current = this;
  }

  static UsageData? _current;

  static void note(String kind, String name) => _current?.count(kind, name);

  static void flushNow() => _current?.flush();

  static void took(String what, Duration time) {
    final s = time.inMilliseconds / 1000;
    final band = s < 2
        ? 'lt2s'
        : s < 5
            ? '2-5s'
            : s < 10
                ? '5-10s'
                : s < 30
                    ? '10-30s'
                    : 'gt30s';
    note('perf', '$what|$band');
  }

  static final Stopwatch launch = Stopwatch();

  final Map<String, int> _effects = {};
  final Map<String, int> _panels = {};
  final Map<String, Map<String, int>> _counts = {};

  DateTime? _begun;
  bool _loaded = false;
  Timer? _flush;
  Timer? _clock;

  bool get active =>
      !kIsWeb &&
      workspace.shareUsage &&
      workspace.usageNoticeSeen &&
      !Platform.environment.containsKey('FLUTTER_TEST');

  void start() {
    if (!active || _clock != null) return;
    _load();
    count('session', 'launch');
    if (!launch.isRunning && launch.elapsedTicks > 0) {
      took('startup', launch.elapsed);
    }
    _begun = DateTime.now();
    _clock = Timer.periodic(const Duration(seconds: 60), (_) => _write());
  }

  void listenTo(ValueNotifier<PaneId?> pane) {
    pane.addListener(() {
      final now = pane.value;
      if (now != null) panel(now.panel.name);
    });
  }

  void effect(String name) => _count(
      _effects,
      !name.contains(':') || _prefixes.any(name.startsWith) ? name : 'other');

  static const _prefixes = ['ofx:', 'clap:', 'vst3:', 'lfx:'];

  void panel(String name) => _count(_panels, name);

  void count(String kind, String name) {
    if (!_kinds.contains(kind) ||
        name.length > 100 ||
        !_name.hasMatch(name)) {
      return;
    }
    _count(_counts.putIfAbsent(kind, () => {}), name);
  }

  static const _kinds = {
    'command',
    'import',
    'export',
    'error',
    'perf',
    'session',
    'update',
    'stability',
  };
  static final _name = RegExp(r'^[A-Za-z0-9_.:|<>%+ -]+$');

  static String extensionOf(String path) {
    final name = path.split(RegExp(r'[\\/]')).last;
    final dot = name.lastIndexOf('.');
    final ext = dot < 0 ? '' : name.substring(dot + 1).toLowerCase();
    return RegExp(r'^[a-z0-9]{1,8}$').hasMatch(ext) ? ext : 'other';
  }

  void exported(
      {required String path,
      required String codec,
      required int height,
      required double fps,
      String? error}) {
    final container = extensionOf(path);
    final format =
        RegExp(r'^[a-z0-9_]{1,16}$').hasMatch(codec) ? codec : 'other';
    final res = height < 720
        ? 'sd'
        : height < 1080
            ? '720p'
            : height < 1440
                ? '1080p'
                : height < 2160
                    ? '1440p'
                    : height < 4320
                        ? '4k'
                        : '8k';
    final r = fps.round();
    final rate = r < 24
        ? 'lt24'
        : r < 25
            ? '24'
            : r < 30
                ? '25'
                : r < 50
                    ? '30'
                    : r < 60
                        ? '50'
                        : r == 60
                            ? '60'
                            : 'gt60';
    final result = error == null
        ? 'ok'
        : error == 'cancelled'
            ? 'cancelled'
            : 'failed';
    count('export', '$container|$format|$res|$rate|$result');
    if (result == 'failed') count('error', 'export:unknown');
  }

  void _count(Map<String, int> into, String key) {
    if (!active) return;
    _load();
    if (!into.containsKey(key) && into.length >= 200) return;
    into[key] = (into[key] ?? 0) + 1;
    _flush ??= Timer(const Duration(seconds: 30), _write);
  }

  void flush() {
    _flush?.cancel();
    _write();
  }

  void _write() {
    _flush = null;
    if (!active) return;
    try {
      store().writeAsStringSync(jsonEncode({
        'effects': _effects,
        'panels': _panels,
        'counts': _counts,
        'elapsed': switch (_begun) {
          final begun? => DateTime.now().difference(begun).inSeconds,
          null => 0,
        },
      }));
    } catch (_) {}
  }

  void _load() {
    if (_loaded) return;
    _loaded = true;
    try {
      final j = jsonDecode(store().readAsStringSync());
      void merge(Map<String, int> into, Object? from) {
        if (from is! Map) return;
        for (final e in from.entries) {
          final n = e.value;
          if (n is num) into['${e.key}'] = (into['${e.key}'] ?? 0) + n.toInt();
        }
      }

      merge(_effects, j['effects']);
      merge(_panels, j['panels']);
      final counts = j['counts'];
      if (counts is Map) {
        for (final e in counts.entries) {
          if (_kinds.contains(e.key)) {
            merge(_counts.putIfAbsent('${e.key}', () => {}), e.value);
          }
        }
      }
      final last = j['elapsed'];
      if (last is num && last > 0) {
        final m = last / 60;
        final band = m < 1
            ? 'lt1m'
            : m < 10
                ? '1-10m'
                : m < 30
                    ? '10-30m'
                    : m < 60
                        ? '30-60m'
                        : m < 120
                            ? '1-2h'
                            : m < 240
                                ? '2-4h'
                                : 'gt4h';
        count('session', 'length|$band');
      }
    } catch (_) {}
  }

  Map<String, Object?> payload() {
    _load();
    final current = version() ?? '';
    final abi = Abi.current().toString().split('_');
    final counts = {
      for (final e in _counts.entries)
        if (e.value.isNotEmpty) e.key: e.value,
    };
    return {
      'id': workspace.usageId.isEmpty
          ? '(made when first sent)'
          : workspace.usageId,
      'version': current,
      'channel': isNightlyVersion(current) ? 'nightly' : 'stable',
      'os': Platform.operatingSystem,
      'os_version': _osVersion(),
      'arch': abi.length > 1 ? abi.last : '',
      'language': Platform.localeName.split(RegExp('[_-]')).first,
      if (_effects.isNotEmpty) 'effects': _effects,
      if (_panels.isNotEmpty) 'panels': _panels,
      if (counts.isNotEmpty) 'counts': counts,
      'settings': _settings(current),
    };
  }

  Map<String, String> _settings(String current) {
    final w = workspace;
    final i = w.interface;
    final keys = w.keymapSet == null && w.keymapJson == null
        ? BridgeKeymapPreset.lumit
        : BridgeKeymapPreset.values.asNameMap()[w.keymapSet];
    final scale = (i.uiScale * 100).round();
    final cores = Platform.numberOfProcessors;
    final out = <String, String>{
      'workspace': switch (w.activePreset) {
        WorkspacePreset.edit => 'edit',
        WorkspacePreset.cut => 'cut',
        WorkspacePreset.nodes => 'nodes',
        _ => 'custom',
      },
      'style': w.themeShape.name,
      'scheme': w.activeCustomTheme != null
          ? 'custom'
          : w.colorScheme.name.replaceAllMapped(
              RegExp('[A-Z]'), (m) => '_${m[0]!.toLowerCase()}'),
      'keys': switch (keys) {
        BridgeKeymapPreset.lumit => 'lumit',
        BridgeKeymapPreset.afterEffects => 'after_effects',
        BridgeKeymapPreset.vegas => 'vegas',
        _ => 'custom',
      },
      'motion': w.animationLevel.name,
      'toolbar': i.toolBarPosition.name,
      'ui_scale': scale < 100
          ? 'lt100'
          : scale < 125
              ? '100'
              : scale < 150
                  ? '125'
                  : scale == 150
                      ? '150'
                      : 'gt150',
      'language_setting': i.language ?? 'follow_system',
      'nightly': (w.nightlyUpdates ?? isNightlyVersion(current)) ? 'on' : 'off',
      'welcome': w.showWelcomeOnLaunch ? 'on' : 'off',
      'cores': cores <= 4
          ? '1-4'
          : cores <= 8
              ? '5-8'
              : cores <= 12
                  ? '9-12'
                  : cores <= 16
                      ? '13-16'
                      : 'gt16',
    };
    final screens = PlatformDispatcher.instance;
    final displays = screens.displays;
    if (displays.isNotEmpty) out['displays'] = '${min(displays.length, 3)}';
    var display = displays.firstOrNull;
    try {
      display = screens.implicitView?.display ?? display;
    } catch (_) {}
    if (display != null && display.devicePixelRatio > 0) {
      final h = (display.size.height / display.devicePixelRatio).round();
      out['screen'] = h < 1080
          ? 'lt1080'
          : h < 1440
              ? '1080'
              : h < 2160
                  ? '1440'
                  : h == 2160
                      ? '2160'
                      : 'gt2160';
    }
    final m = machine?.call();
    if (m != null) {
      final ram = (m.ram / (1 << 30)).round();
      if (m.ram > 0) {
        out['ram_gb'] = ram < 8
            ? 'lt8'
            : ram <= 16
                ? '8-16'
                : ram <= 32
                    ? '16-32'
                    : ram <= 64
                        ? '32-64'
                        : 'gt64';
      }
      final vram = (m.vram / (1 << 30)).round();
      if (m.vram > 0) {
        out['vram_gb'] = vram < 4
            ? 'lt4'
            : vram <= 8
                ? '4-8'
                : vram <= 12
                    ? '8-12'
                    : vram <= 16
                        ? '12-16'
                        : vram <= 24
                            ? '16-24'
                            : 'gt24';
      }
      if (m.vendor != 0) {
        out['gpu_vendor'] = switch (m.vendor) {
          0x10DE => 'nvidia',
          0x1002 || 0x1022 => 'amd',
          0x8086 => 'intel',
          0x106B => 'apple',
          _ => 'other',
        };
      }
      out['gpu_kind'] = m.gpu == 'discrete' || m.gpu == 'integrated'
          ? m.gpu
          : 'unknown';
    }
    final value = RegExp(r'^[A-Za-z0-9_.<>+=-]{1,24}$');
    out.removeWhere((_, v) => !value.hasMatch(v));
    return out;
  }

  static String _osVersion() {
    final v = Platform.operatingSystemVersion;
    if (Platform.isWindows) {
      final build =
          int.tryParse(RegExp(r'Build (\d+)').firstMatch(v)?.group(1) ?? '');
      return build == null ? '' : (build >= 22000 ? '11' : '10');
    }
    if (Platform.isMacOS) {
      return RegExp(r'Version (\d+)').firstMatch(v)?.group(1) ?? '';
    }
    return '';
  }

  Future<void> sendKept() async {
    if (!active || !usageCollectorLive) return;
    final target = Uri.parse('$cloudUrl/v1/usage');
    if (target.scheme != 'https' && target.host != 'localhost') return;
    if (workspace.usageId.isEmpty) workspace.setUsageId(_newId());
    final client = HttpClient()
      ..connectionTimeout = const Duration(seconds: 10);
    try {
      final request = await client.postUrl(target);
      request.headers.contentType = ContentType.json;
      request.write(jsonEncode(payload()));
      final response =
          await request.close().timeout(const Duration(seconds: 15));
      await response.drain<void>();
      if (response.statusCode == 200 || response.statusCode == 204) {
        _clear();
        try {
          store().deleteSync();
        } catch (_) {}
      }
    } catch (_) {
    } finally {
      client.close(force: true);
    }
  }

  void _clear() {
    _effects.clear();
    _panels.clear();
    _counts.clear();
  }

  void forget() {
    _flush?.cancel();
    _flush = null;
    _clock?.cancel();
    _clock = null;
    _clear();
    _begun = null;
    workspace.setUsageId('');
    try {
      store().deleteSync();
    } catch (_) {}
  }

  static String _newId() {
    final r = Random.secure();
    return List.generate(
        16, (_) => r.nextInt(256).toRadixString(16).padLeft(2, '0')).join();
  }
}
