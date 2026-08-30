// The parked measurement probe behind docs/impl/ui-performance.md (K-676):
// the manual instrument for docs/13's B1/B2 until a real-window harness
// exists. Activated only when the build passes
// --dart-define=LUMIT_PROBE_PROJECT=<path-to-.lum> (see main.dart); an
// ordinary build compiles it out of reach, so it ships in nothing.
//
// What it does: opens the given project, fronts the comp named "Clips" (or
// the first comp), then drives the measured gestures — select clicks, wheel
// scrolls, Ctrl+wheel zoom flights, playhead and work-area drags, the graph
// mode comparison — with synthetic pointer events through the real hit-test
// path, recording every FrameTiming (UI-thread build ms, raster-thread ms,
// total span) and every bridge crossing per gesture. Results are written to
// LUMIT_PROBE_OUT as plain text: the note's §2 table is this file's output.
//
// Every ui-performance work package re-runs this before and after; it goes
// the day docs/13 §7.3's real-window CI harness supersedes it.

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import '../panels/timeline_extras_frb.dart' show workAreaWith;
import '../state/app_state.dart';
import '../state/ui_state.dart';

const String probeProjectPath = String.fromEnvironment('LUMIT_PROBE_PROJECT');

/// Where the table is written; relative paths land in the working directory.
const String probeOutPath = String.fromEnvironment('LUMIT_PROBE_OUT',
    defaultValue: 'lumit-probe-out.txt');

/// Counts every bridge crossing and what it cost, without the stack-trace tax
/// the debug tracer pays. Cheap enough to leave on while timing frames.
class CountingBridgeHandler extends BaseHandler {
  final Map<String, int> calls = {};
  final Map<String, int> micros = {};

  void _add(String name, int us) {
    calls[name] = (calls[name] ?? 0) + 1;
    micros[name] = (micros[name] ?? 0) + us;
  }

  ({Map<String, int> calls, Map<String, int> micros}) snapshot() => (
        calls: Map.of(calls),
        micros: Map.of(micros),
      );

  @override
  Future<S> executeNormal<S, E extends Object>(NormalTask<S, E> task) async {
    final sw = Stopwatch()..start();
    try {
      return await super.executeNormal(task);
    } finally {
      _add('${task.constMeta.debugName}~async', sw.elapsedMicroseconds);
    }
  }

  @override
  S executeSync<S, E extends Object, WireSyncType>(
      SyncTask<S, E, WireSyncType> task) {
    final sw = Stopwatch()..start();
    try {
      return super.executeSync(task);
    } finally {
      _add(task.constMeta.debugName, sw.elapsedMicroseconds);
    }
  }
}

void startPerfProbe(
    LumitState state, LumitUiState ui, CountingBridgeHandler? bridge) {
  _Probe(state, ui, bridge).run();
}

class _Probe {
  final LumitState state;
  final LumitUiState ui;
  final CountingBridgeHandler? bridge;
  final StringBuffer out = StringBuffer();

  _Probe(this.state, this.ui, this.bridge);

  // ---- frame timing collection -------------------------------------------

  List<FrameTiming>? _bucket;

  /// When the last frame report arrived, for quiescence detection — a wall
  /// clock, because [SchedulerBinding.endOfFrame] would *schedule* the frame
  /// it claims to wait for.
  DateTime _lastFrameAt = DateTime.fromMillisecondsSinceEpoch(0);

  /// When a frame that the *interface* built last arrived, as against one the
  /// preview merely republished into.
  ///
  /// A click on a switch is answered by the chrome and then, a while later, by
  /// a new picture: the engine re-renders, the texture republishes, and frames
  /// keep landing for a few hundred milliseconds with nothing to build in them
  /// (docs/13 §4 lets the picture lag; the chrome may not). Measuring
  /// "settled" off every frame therefore measured the render, not the wave —
  /// so a build worth more than a tenth of a millisecond is what counts as the
  /// interface still working.
  DateTime _lastBuiltAt = DateTime.fromMillisecondsSinceEpoch(0);

  void _onTimings(List<FrameTiming> timings) {
    _bucket?.addAll(timings);
    _lastFrameAt = DateTime.now();
    if (timings.any((f) => f.buildDuration.inMicroseconds > 100)) {
      _lastBuiltAt = _lastFrameAt;
    }
  }

  // ---- pointer dispatch ---------------------------------------------------

  int _pointer = 4242;

  void _send(PointerEvent e) => GestureBinding.instance.handlePointerEvent(e);

  Future<void> _click(Offset at) async {
    final id = _pointer++;
    _send(PointerDownEvent(
        pointer: id,
        position: at,
        kind: PointerDeviceKind.mouse,
        buttons: kPrimaryButton));
    await Future<void>.delayed(const Duration(milliseconds: 40));
    _send(PointerUpEvent(
        pointer: id, position: at, kind: PointerDeviceKind.mouse));
  }

  Future<void> _drag(Offset from, Offset to, Duration duration) async {
    final id = _pointer++;
    _send(PointerDownEvent(
        pointer: id,
        position: from,
        kind: PointerDeviceKind.mouse,
        buttons: kPrimaryButton));
    final sw = Stopwatch()..start();
    var last = from;
    while (sw.elapsed < duration) {
      await SchedulerBinding.instance.endOfFrame;
      final t = (sw.elapsed.inMicroseconds / duration.inMicroseconds)
          .clamp(0.0, 1.0);
      final p = Offset.lerp(from, to, t)!;
      _send(PointerMoveEvent(
          pointer: id,
          position: p,
          delta: p - last,
          kind: PointerDeviceKind.mouse,
          buttons: kPrimaryButton));
      last = p;
    }
    _send(PointerUpEvent(
        pointer: id, position: last, kind: PointerDeviceKind.mouse));
  }

  void _wheel(Offset at, double dy) => _send(PointerScrollEvent(
      position: at, scrollDelta: Offset(0, dy), kind: PointerDeviceKind.mouse));

  Future<void> _withCtrl(Future<void> Function() body) async {
    HardwareKeyboard.instance.handleKeyEvent(const KeyDownEvent(
        physicalKey: PhysicalKeyboardKey.controlLeft,
        logicalKey: LogicalKeyboardKey.controlLeft,
        timeStamp: Duration.zero));
    try {
      await body();
    } finally {
      HardwareKeyboard.instance.handleKeyEvent(const KeyUpEvent(
          physicalKey: PhysicalKeyboardKey.controlLeft,
          logicalKey: LogicalKeyboardKey.controlLeft,
          timeStamp: Duration.zero));
    }
  }

  // ---- element lookup -----------------------------------------------------

  List<Element> _elements(bool Function(Element) test) {
    final found = <Element>[];
    void visit(Element e) {
      if (test(e)) found.add(e);
      e.visitChildren(visit);
    }

    final root = WidgetsBinding.instance.rootElement;
    if (root != null) visit(root);
    return found;
  }

  List<Element> _byTypeName(String name) =>
      _elements((e) => e.widget.runtimeType.toString() == name);

  Element? _byKey(Key key) =>
      _elements((e) => e.widget.key == key).firstOrNull;

  Rect? _rectOf(Element? e) {
    final ro = e?.renderObject;
    if (ro is! RenderBox || !ro.hasSize || !ro.attached) return null;
    return ro.localToGlobal(Offset.zero) & ro.size;
  }

  // ---- gesture measurement ------------------------------------------------

  Future<void> _measure(String name, Future<void> Function() gesture) async {
    await _settle();
    final frames = <FrameTiming>[];
    _bucket = frames;
    final before = bridge?.snapshot();
    final wall = Stopwatch()..start();
    await gesture();
    // Let trailing frames (the follow-on cost of the gesture) land.
    await Future<void>.delayed(const Duration(milliseconds: 300));
    wall.stop();
    _bucket = null;
    final after = bridge?.snapshot();
    _report(name, frames, wall.elapsed, before, after);
  }

  Future<void> _settle() async {
    await Future<void>.delayed(const Duration(milliseconds: 800));
  }

  void _report(
    String name,
    List<FrameTiming> frames,
    Duration wall,
    ({Map<String, int> calls, Map<String, int> micros})? before,
    ({Map<String, int> calls, Map<String, int> micros})? after,
  ) {
    double ms(Duration d) => d.inMicroseconds / 1000.0;
    List<double> sorted(Iterable<double> xs) => xs.toList()..sort();
    double at(List<double> xs, double q) =>
        xs.isEmpty ? 0 : xs[((xs.length - 1) * q).round()];

    final build = sorted(frames.map((f) => ms(f.buildDuration)));
    final raster = sorted(frames.map((f) => ms(f.rasterDuration)));
    final span = sorted(frames.map((f) => ms(f.totalSpan)));
    final over17 = span.where((s) => s > 17.0).length;
    final over9 = span.where((s) => s > 8.7).length;
    final secs = wall.inMicroseconds / 1e6;

    out.writeln('== $name ==');
    out.writeln('frames=${frames.length} wall=${secs.toStringAsFixed(2)}s '
        'fps=${(frames.length / secs).toStringAsFixed(1)}');
    String row(String label, List<double> xs) =>
        '$label med=${at(xs, 0.5).toStringAsFixed(2)} '
        'p90=${at(xs, 0.9).toStringAsFixed(2)} '
        'max=${at(xs, 1.0).toStringAsFixed(2)}';
    out.writeln(row('build(ms):', build));
    out.writeln(row('raster(ms):', raster));
    out.writeln(row('span(ms):', span));
    out.writeln('frames>17ms=$over17 frames>8.7ms=$over9');

    if (before != null && after != null) {
      final delta = <String, (int, int)>{};
      for (final k in after.calls.keys) {
        final c = after.calls[k]! - (before.calls[k] ?? 0);
        if (c > 0) {
          delta[k] = (c, after.micros[k]! - (before.micros[k] ?? 0));
        }
      }
      final entries = delta.entries.toList()
        ..sort((a, b) => b.value.$2.compareTo(a.value.$2));
      final total = delta.values.fold(0, (s, v) => s + v.$1);
      final totalMs = delta.values.fold(0, (s, v) => s + v.$2) / 1000.0;
      out.writeln('bridge: $total calls, ${totalMs.toStringAsFixed(1)}ms total');
      for (final e in entries.take(8)) {
        out.writeln('  ${e.key} x${e.value.$1} '
            '${(e.value.$2 / 1000.0).toStringAsFixed(1)}ms');
      }
    }
    out.writeln();
    _flush();
  }

  void _flush() {
    try {
      File(probeOutPath).writeAsStringSync(out.toString());
    } catch (e) {
      debugPrint('PROBE write failed: $e');
    }
    debugPrint('PROBE flushed ${out.length} chars');
  }

  // ---- the run ------------------------------------------------------------

  Future<void> run() async {
    SchedulerBinding.instance.addTimingsCallback(_onTimings);
    out.writeln('LUMIT PERF PROBE ${DateTime.now()}');
    out.writeln('project: $probeProjectPath');

    // Wait for the project and the Timeline to be up.
    for (var i = 0; i < 240; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 500));
      if (state.project == null) continue;
      if (state.comps().isEmpty) continue;
      if (_byTypeName('TimelinePanelFrb').isEmpty) continue;
      break;
    }
    if (state.project == null) {
      out.writeln('FAILED: no project after 120s');
      _flush();
      return;
    }

    // Front the comp whose name contains "clip".
    final comps = state.comps();
    out.writeln('comps: ${comps.map((c) => c.$2).join(", ")}');
    final target = comps
            .where((c) => c.$2.toLowerCase() == 'clips')
            .firstOrNull ??
        comps
            .where((c) => c.$2.toLowerCase().contains('clip'))
            .firstOrNull ??
        comps.firstOrNull;
    if (target == null) {
      out.writeln('FAILED: no comp');
      _flush();
      return;
    }
    out.writeln('fronting: ${target.$2}');
    ui.setSelectedComp(target.$1);
    await Future<void>.delayed(const Duration(seconds: 3));

    // The conditions: window pixels, scale, and whether the preview is live.
    // The owner's condition is the window maximised on the 164 Hz monitor with
    // media resolving; the agent trap is a 1280x720 window over missing media.
    final view = WidgetsBinding.instance.platformDispatcher.views.first;
    out.writeln('window physical=${view.physicalSize.width.toStringAsFixed(0)}'
        'x${view.physicalSize.height.toStringAsFixed(0)} '
        'dpr=${view.devicePixelRatio}');
    final projectDir = File(probeProjectPath).parent.path;
    final mediaLive = Directory('$projectDir/Clips').existsSync();
    out.writeln('media beside project: '
        '${mediaLive ? "resolves (live preview)" : "MISSING (empty preview)"}');

    // Panel geometry.
    final panel = _rectOf(_byTypeName('TimelinePanelFrb').firstOrNull);
    final ruler = _rectOf(_byTypeName('TimelineRuler').firstOrNull);
    final lanes = _rectOf(_byTypeName('LayerArea').firstOrNull);
    final viewer = _rectOf(_byTypeName('ViewerPanelFrb').firstOrNull);
    final rows = _byTypeName('OutlineRow');
    out.writeln('panel=${_fmtRect(panel)} viewer=${_fmtRect(viewer)}');
    out.writeln('ruler=${_fmtRect(ruler)} lanes=${_fmtRect(lanes)} '
        'rows=${rows.length}');
    if (panel == null || ruler == null || lanes == null || rows.isEmpty) {
      out.writeln('FAILED: geometry not found');
      _flush();
      return;
    }
    final laneCentre = Offset(
        lanes.left + lanes.width * 0.55, lanes.top + lanes.height * 0.4);

    // Warm-up: zoom out to fit, scroll the rows to the top.
    await _withCtrl(() async {
      for (var i = 0; i < 24; i++) {
        _wheel(laneCentre, 120);
        await Future<void>.delayed(const Duration(milliseconds: 30));
      }
    });
    for (var i = 0; i < 40; i++) {
      _wheel(laneCentre, -120);
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
    await Future<void>.delayed(const Duration(seconds: 1));

    // G0: idle baseline — how many frames does a quiet editor draw?
    await _measure('idle 3s', () async {
      await Future<void>.delayed(const Duration(seconds: 3));
    });

    // G1: click-to-select on outline rows — per-click detail. x = left+40 is
    // the name cell's start (past the number and colour chip), so a click is a
    // plain select and never a switch toggle.
    await _selectClicks('select: clicks on outline row names', dx: 40);

    // G1b: the click that *edits* — WP-5's gate. The lock cell is a switch like
    // any other (`set_switch` → one committed op → the document-change wave),
    // and it is the one switch that changes no pixel, so what the row reports is
    // the follow-on wave itself rather than a re-render arriving behind it. The
    // cells are found by key rather than by an x offset because which switches
    // are shown depends on the outline's width (`switchCellsFor`).
    await _selectClicks('edit: clicks on a row lock switch',
        dx: 0, cellPrefix: 'tl-locked-');

    // G2: wheel-scroll the lanes down then up.
    await _measure('scroll: lanes wheel 30 down + 30 up', () async {
      for (var i = 0; i < 30; i++) {
        _wheel(laneCentre, 120);
        await Future<void>.delayed(const Duration(milliseconds: 25));
      }
      for (var i = 0; i < 30; i++) {
        _wheel(laneCentre, -120);
        await Future<void>.delayed(const Duration(milliseconds: 25));
      }
    });

    // G3: wheel-scroll over the outline half.
    final outlineCentre = Offset(
        panel.left + (lanes.left - panel.left) * 0.5,
        lanes.top + lanes.height * 0.4);
    await _measure('scroll: outline wheel 30 down + 30 up', () async {
      for (var i = 0; i < 30; i++) {
        _wheel(outlineCentre, 120);
        await Future<void>.delayed(const Duration(milliseconds: 25));
      }
      for (var i = 0; i < 30; i++) {
        _wheel(outlineCentre, -120);
        await Future<void>.delayed(const Duration(milliseconds: 25));
      }
    });

    // G4: ctrl+wheel zoom in then out over the lanes.
    await _measure('zoom: ctrl+wheel 14 in + 14 out', () async {
      await _withCtrl(() async {
        for (var i = 0; i < 14; i++) {
          _wheel(laneCentre, -120);
          await Future<void>.delayed(const Duration(milliseconds: 90));
        }
        for (var i = 0; i < 14; i++) {
          _wheel(laneCentre, 120);
          await Future<void>.delayed(const Duration(milliseconds: 90));
        }
      });
    });

    // G5: playhead drag along the ruler (top half, clear of the band row).
    // Two measures: the right sweep crosses frames this session has never
    // rendered (renders arriving), the return sweep re-crosses frames just
    // rendered (presenting from the bank) — the cached/uncached split.
    final rulerY = ruler.top + ruler.height * 0.30;
    await _measure('playhead drag: sweep right (fresh spans)', () async {
      await _drag(Offset(ruler.left + ruler.width * 0.15, rulerY),
          Offset(ruler.left + ruler.width * 0.75, rulerY),
          const Duration(milliseconds: 2500));
    });
    await _measure('playhead drag: sweep back left (revisited spans)',
        () async {
      await _drag(Offset(ruler.left + ruler.width * 0.75, rulerY),
          Offset(ruler.left + ruler.width * 0.25, rulerY),
          const Duration(milliseconds: 2500));
    });

    // G6: work-area end-handle drag. Narrow the work area first so the end
    // handle stands mid-panel, then find it by its key.
    try {
      final comp = target.$1;
      final duration = comp.durationFrames();
      comp.setWorkArea(
          span: workAreaWith(
              comp: comp,
              current: comp.getWorkArea(),
              wanted: (duration * 3) ~/ 4,
              isStart: false));
      comp.setWorkArea(
          span: workAreaWith(
              comp: comp,
              current: comp.getWorkArea(),
              wanted: duration ~/ 4,
              isStart: true));
      ui.model.refresh();
    } catch (e) {
      out.writeln('setWorkArea failed: $e');
    }
    await Future<void>.delayed(const Duration(milliseconds: 800));
    final endHandle = _rectOf(_byKey(const ValueKey('tl-work-end')));
    out.writeln('end handle: ${_fmtRect(endHandle)}');
    if (endHandle != null) {
      final from = endHandle.center;
      final to = Offset(
          (from.dx - ruler.width * 0.3).clamp(ruler.left + 10, ruler.right),
          from.dy);
      await _measure('work-area drag: end handle left then back', () async {
        await _drag(from, to, const Duration(milliseconds: 2000));
        await _drag(to, from, const Duration(milliseconds: 2000));
      });
    } else {
      out.writeln('work-area handle not found on screen; skipped');
      out.writeln();
    }

    // G7: the same zoom and scrub in GRAPH mode — the yardstick. If raster
    // stays where it was, the raster cost is the window composite, not the
    // lane pictures; if it falls, the lane pictures are what the raster
    // thread is paying for.
    final graphTab = _rectOf(_byKey(const ValueKey('tl-graph')));
    if (graphTab != null) {
      await _click(graphTab.center);
      await Future<void>.delayed(const Duration(seconds: 1));
      await _measure('graph mode: ctrl+wheel 14 in + 14 out', () async {
        await _withCtrl(() async {
          for (var i = 0; i < 14; i++) {
            _wheel(laneCentre, -120);
            await Future<void>.delayed(const Duration(milliseconds: 90));
          }
          for (var i = 0; i < 14; i++) {
            _wheel(laneCentre, 120);
            await Future<void>.delayed(const Duration(milliseconds: 90));
          }
        });
      });
      await _measure('graph mode: playhead drag on ruler', () async {
        await _drag(Offset(ruler.left + ruler.width * 0.2, rulerY),
            Offset(ruler.left + ruler.width * 0.7, rulerY),
            const Duration(milliseconds: 2500));
      });
      final lanesTab = _rectOf(_byKey(const ValueKey('tl-view-lanes')));
      if (lanesTab != null) await _click(lanesTab.center);
      await Future<void>.delayed(const Duration(milliseconds: 600));
    } else {
      out.writeln('graph tab not found; skipped');
    }

    // G8: playhead drag with render-time measuring OFF, against G5's default.
    ui.renderTimings.setMeasuring(false);
    await Future<void>.delayed(const Duration(milliseconds: 600));
    await _measure('playhead drag, measuring off', () async {
      await _drag(Offset(ruler.left + ruler.width * 0.15, rulerY),
          Offset(ruler.left + ruler.width * 0.75, rulerY),
          const Duration(milliseconds: 2500));
    });

    out.writeln('DONE ${DateTime.now()}');
    _flush();
    debugPrint('PROBE DONE');
  }

  String _fmtRect(Rect? r) => r == null
      ? 'null'
      : 'l=${r.left.toStringAsFixed(0)} t=${r.top.toStringAsFixed(0)} '
          'w=${r.width.toStringAsFixed(0)} h=${r.height.toStringAsFixed(0)}';

  /// One row of per-click evidence: acknowledgement (down → the next finished
  /// frame), quiet (down → 300ms with no frame at all), the frames in the
  /// burst, and every bridge call the click set off.
  Future<void> _selectClicks(String name,
      {double dx = 0, String? cellPrefix}) async {
    await _settle();
    out.writeln('== $name ==');
    for (var i = 0; i < 6; i++) {
      final live = cellPrefix == null
          ? _byTypeName('OutlineRow')
          : _elements((e) => switch (e.widget.key) {
                ValueKey<String>(:final value) => value.startsWith(cellPrefix),
                _ => false,
              });
      if (live.isEmpty) break;
      final row = _rectOf(live[(i * 3 + 1) % live.length]);
      if (row == null) continue;
      final at =
          cellPrefix == null ? Offset(row.left + dx, row.center.dy) : row.center;
      final frames = <FrameTiming>[];
      _bucket = frames;
      final before = bridge?.snapshot();
      final sw = Stopwatch()..start();
      final clickedAt = DateTime.now();
      _lastBuiltAt = clickedAt;
      await _click(at);
      await SchedulerBinding.instance.endOfFrame;
      final ack = sw.elapsedMicroseconds / 1000.0;
      // Quiet: 300ms with no frame reported, or 4s cap.
      while (sw.elapsed < const Duration(seconds: 4)) {
        await Future<void>.delayed(const Duration(milliseconds: 25));
        if (DateTime.now().difference(_lastFrameAt) >
            const Duration(milliseconds: 300)) {
          break;
        }
      }
      final quiet = sw.elapsedMicroseconds / 1000.0 - 300.0;
      // Settled: the last frame the interface actually *built* something in.
      // WP-5's gate is on this rather than on `quiet`, which keeps running for
      // as long as the preview is republishing behind the edit.
      final settled = _lastBuiltAt
          .difference(clickedAt)
          .inMicroseconds
          .clamp(0, 1 << 31) /
          1000.0;
      _bucket = null;
      final after = bridge?.snapshot();
      final builds = frames.map((f) => f.buildDuration.inMicroseconds / 1000.0);
      final worstBuild = builds.isEmpty ? 0.0 : builds.reduce((a, b) => a > b ? a : b);
      final delta = <String, (int, int)>{};
      if (before != null && after != null) {
        for (final k in after.calls.keys) {
          final c = after.calls[k]! - (before.calls[k] ?? 0);
          if (c > 0) delta[k] = (c, after.micros[k]! - (before.micros[k] ?? 0));
        }
      }
      final calls = delta.values.fold(0, (s, v) => s + v.$1);
      final callMs = delta.values.fold(0, (s, v) => s + v.$2) / 1000.0;
      final top = delta.entries.toList()
        ..sort((a, b) => b.value.$2.compareTo(a.value.$2));
      out.writeln('click ${i + 1}: ack=${ack.toStringAsFixed(0)}ms '
          'settled=${settled.toStringAsFixed(0)}ms '
          'quiet=${quiet.toStringAsFixed(0)}ms frames=${frames.length} '
          'worstBuild=${worstBuild.toStringAsFixed(1)}ms '
          'bridge=$calls calls ${callMs.toStringAsFixed(1)}ms');
      for (final e in top.take(4)) {
        out.writeln('    ${e.key} x${e.value.$1} '
            '${(e.value.$2 / 1000.0).toStringAsFixed(1)}ms');
      }
      _flush();
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
    out.writeln();
    _flush();
  }
}
