// B1 and B2 (docs/13-PERFORMANCE-RULES.md §2), measured on a REAL window
// from a test.
//
// B1 is the UI thread's build time per frame while an interaction runs;
// B2 is whether a press is answered on the very next frame. Neither is a
// number a widget test can produce: `flutter test` has no compositor, its
// clock is fake, and no `FrameTiming` is ever reported. Here the app runs in
// the real runner, so every frame the engine reports is a frame that was
// actually built and rastered, and the probe's own gesture code
// (`lib/probe/perf_probe.dart`, `ProbeGestures`) drives the docs/impl/
// ui-performance.md §2 gesture list through the real hit-test path.
//
// What is asserted, and where:
//
// - B2, everywhere. "The next frame" is a frame count, not a millisecond,
//   so it means the same on a software rasteriser as on the reference
//   desktop: after a press, one `endOfFrame`, and the row is lit (or the
//   twirl is open, or the playhead has moved).
// - B1, only under LUMIT_REFERENCE_HW=1, the same switch the engine
//   harness (crates/lumit-bench) gates its absolute budgets behind. A shared
//   virtual machine building in debug mode would fail 8.3 ms for reasons
//   that have nothing to do with Lumit, so there the table is recorded and
//   published, and gates nothing.
//
// The table says which conditions it was measured in (build mode, window
// size, whether the reference switch was set) because §2.1 of the note
// measured a factor of four between a small empty window and a
// maximised one, and a row that does not name its conditions misreports.
//
// Run, from flutter_ui/ (profile is what the numbers are meant in;
// `flutter test` builds debug and says so in its header):
//
//   flutter drive --profile -d windows \
//     --driver=test_driver/integration_test.dart \
//     --target=integration_test/ui_budget_test.dart \
//     --dart-define=LUMIT_UI_BUDGET_OUT=<file>
//
//   flutter test integration_test/ui_budget_test.dart -d windows

import 'dart:async';
import 'dart:developer' as developer;
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_maths.dart' show easyEase;
import 'package:lumit_flutter/panels/timeline_outline_row_frb.dart'
    show OutlineRow;
import 'package:lumit_flutter/probe/perf_probe.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/widgets/time_readout.dart' show TimeReadout;
import 'package:uuid/uuid.dart';

/// docs/13 B1: the UI-thread budget for one interaction frame, at p95.
const double b1BudgetMs = 8.3;

/// Where the table is written, besides the log. Empty writes nothing.
const String outPath = String.fromEnvironment('LUMIT_UI_BUDGET_OUT');

/// The reference desktop, or a machine standing in for it. The env var is
/// how the engine harness is told; the define is for a `flutter test` whose
/// environment does not reach the app process.
bool get referenceHardware =>
    Platform.environment['LUMIT_REFERENCE_HW'] == '1' ||
    const bool.fromEnvironment('LUMIT_REFERENCE_HW');

/// One measured gesture: the frames whose vsync fell inside it.
class _Row {
  final String name;
  final int frames;
  final double buildP95;
  final double buildWorst;
  final double rasterMed;
  const _Row(
      this.name, this.frames, this.buildP95, this.buildWorst, this.rasterMed);

  bool get overB1 => buildP95 > b1BudgetMs;
}

class _Harness with ProbeGestures {
  final WidgetTester tester;
  final rows = <_Row>[];
  final acks = <(String, int)>[];
  final notes = <String>[];

  _Harness(this.tester);

  /// The test binding ignores a bare `handlePointerEvent` (its default
  /// source is the physical mouse, which a live test does not forward), so
  /// the probe's events go in as test events. Same hit test, same arena.
  @override
  void send(PointerEvent e) => tester.binding
      .handlePointerEventForSource(e, source: TestBindingEventSource.test);

  List<FrameTiming>? _bucket;
  void _onTimings(List<FrameTiming> timings) => _bucket?.addAll(timings);

  /// Drive [gesture] and keep the frames whose vsync fell inside it. The
  /// tail waits out the engine's timings batch, which flushes up to a
  /// second after the frames it describes (perf_probe.dart, `framesWithin`).
  Future<void> measure(String name, Future<void> Function() gesture) async {
    await Future<void>.delayed(const Duration(milliseconds: 800));
    final frames = <FrameTiming>[];
    _bucket = frames;
    final t0 = developer.Timeline.now;
    await gesture();
    final t1 = developer.Timeline.now;
    await Future<void>.delayed(const Duration(milliseconds: 1300));
    _bucket = null;
    // The live test binding asks the engine for a frame after every frame,
    // so the window is full of frames the app never asked for. A frame that
    // built nothing (under 0.1 ms, the probe's own threshold) is the
    // binding's and is left out; what is counted is what the gesture built.
    final inWindow = framesWithin(frames, t0, t1)
        .where((f) => f.buildDuration.inMicroseconds > 100)
        .toList();
    double ms(Duration d) => d.inMicroseconds / 1000.0;
    final build = inWindow.map((f) => ms(f.buildDuration)).toList()..sort();
    final raster = inWindow.map((f) => ms(f.rasterDuration)).toList()..sort();
    rows.add(_Row(name, inWindow.length, percentile(build, 0.95),
        percentile(build, 1.0), percentile(raster, 0.5)));
  }

  /// B2: frames from [input] to [acknowledged] reading true, capped at five
  /// so a miss is a number in the table rather than a hang.
  Future<void> ack(String name, Offset at, FutureOr<void> Function() input,
      bool Function() acknowledged) async {
    await Future<void>.delayed(const Duration(milliseconds: 800));
    await input();
    var frames = 0;
    while (frames < 5) {
      await SchedulerBinding.instance.endOfFrame;
      frames++;
      if (acknowledged()) break;
    }
    final hit = acknowledged();
    acks.add((name, hit ? frames : -1));
    if (!hit) {
      // A miss says what was under the press and whether the answer came
      // late or never, so the row in the table has a cause beside it.
      final result = HitTestResult();
      tester.binding.hitTestInView(result, at, tester.view.viewId);
      var late = frames;
      while (late < 120 && !acknowledged()) {
        await SchedulerBinding.instance.endOfFrame;
        late++;
      }
      notes.add('$name: pressed $at, under it '
          '${result.path.take(8).map((e) => e.target.runtimeType).join(' > ')}'
          '; ${acknowledged() ? "answered after $late frames" : "never answered"}');
    }
  }

  /// The rows as the framework's finders see them: on stage only, which the
  /// probe's own tree walk does not check.
  Iterable<OutlineRow> get outlineRows =>
      tester.widgetList<OutlineRow>(find.byType(OutlineRow));

  bool rowSelected(UuidValue id) =>
      outlineRows.any((r) => r.selected && r.entry.layer.internallayerId == id);

  bool rowOpen(UuidValue id) =>
      outlineRows.any((r) => r.open && r.entry.layer.internallayerId == id);

  /// The Timeline's clock, which is what a press on the ruler moves.
  int? readoutFrame() =>
      (byKey(const ValueKey('tl-timecode'))?.widget as TimeReadout?)?.frame;

  String table(String header) {
    final b = StringBuffer(header);
    String f(double v) => v.toStringAsFixed(2);
    b.writeln();
    b.writeln('| gesture | frames | build p95 ms | build worst ms | '
        'raster med ms | B1 (p95 <= $b1BudgetMs) |');
    b.writeln('|---|---|---|---|---|---|');
    for (final r in rows) {
      final verdict = r.frames == 0 ? 'no frames' : (r.overB1 ? 'OVER' : 'ok');
      b.writeln('| ${r.name} | ${r.frames} | ${f(r.buildP95)} | '
          '${f(r.buildWorst)} | ${f(r.rasterMed)} | $verdict |');
    }
    b.writeln();
    b.writeln('| press | frames to acknowledgement | B2 (next frame) |');
    b.writeln('|---|---|---|');
    for (final (name, frames) in acks) {
      b.writeln('| $name | ${frames < 0 ? "none in 5" : frames} | '
          '${frames == 1 ? "ok" : "MISSED"} |');
    }
    for (final n in notes) {
      b.writeln();
      b.writeln(n);
    }
    return b.toString();
  }
}

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  // Frames run as the framework asks for them, as in the app; a pump is
  // still honoured, which mounting the shell needs. The binding also
  // schedules a frame after every frame under this policy, which `measure`
  // filters out again; the `benchmark` policy stops that but drops the
  // app's own frame requests with it, so it is no use here.
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.fullyLive;

  testWidgets('B1 and B2 over the stress comp, on a real window',
      (tester) async {
    // Never the developer's own settings file (frb_test_support does the
    // same): a Workspace setter saves, and this one starts fresh.
    Workspace.storeOverride =
        '${Directory.systemTemp.createTempSync('lumit-ui-budget').path}'
        '/workspace.json';
    await BridgeLib.init();
    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: Workspace());

    // docs/13 §2.1's stress shape at the size the window-sized rules need:
    // far more layers than fit, so "the window" and "the comp" are different
    // lists (rebuild_budget_test uses the same 200). Every fourth layer keys
    // its opacity, so a scrub and a twirl have keyed rows to follow.
    const layers = 200;
    final comp = state.project!.newComposition(name: 'Stress');
    for (var i = 0; i < layers; i++) {
      final layer = comp.addSolidLayer();
      if (i % 4 == 0) {
        layer.setTransform(
          prop: BridgeTransformProp.opacity,
          value: BridgeScalar.keyframed([
            for (final f in [10, 60, 110])
              BridgeKeyframe(
                time: comp.timeOfFrame(frame: f),
                value: f.toDouble(),
                interpIn: easyEase,
                interpOut: easyEase,
              ),
          ]),
        );
      }
    }
    ui.setSelectedComp(comp);
    await tester.pumpWidget(LumitAppNew(state, ui, welcome: false));

    final h = _Harness(tester);
    SchedulerBinding.instance.addTimingsCallback(h._onTimings);
    addTearDown(
        () => SchedulerBinding.instance.removeTimingsCallback(h._onTimings));

    // The Timeline is up when it has geometry and rows.
    TimelineGeometry? geometry;
    for (var i = 0; i < 60 && geometry == null; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 500));
      final g = h.timelineGeometry();
      if (g != null && h.outlineRows.isNotEmpty) geometry = g;
    }
    expect(geometry, isNotNull, reason: 'the Timeline never came up');
    final (:panel, :ruler, :lanes, viewer: _, clamped: _) = geometry!;
    // The same warm-up as the probe: zoom out to fit, rows to the top.
    final laneCentre =
        Offset(lanes.left + lanes.width * 0.55, lanes.top + lanes.height * 0.4);
    final outlineCentre = Offset(panel.left + (lanes.left - panel.left) * 0.5,
        lanes.top + lanes.height * 0.4);
    await h.withCtrl(() async {
      for (var i = 0; i < 24; i++) {
        h.wheel(laneCentre, 120);
        await Future<void>.delayed(const Duration(milliseconds: 30));
      }
    });
    for (var i = 0; i < 40; i++) {
      h.wheel(laneCentre, -120);
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
    await Future<void>.delayed(const Duration(seconds: 1));
    final notches =
        ProbeGestures.notchesFor(h.verticalScrollExtentAt(laneCentre));

    final view = tester.view;
    final mode = kProfileMode
        ? 'profile'
        : kReleaseMode
            ? 'release'
            : 'debug';
    final header = StringBuffer()
      ..writeln('LUMIT UI BUDGET (docs/13 B1, B2) ${DateTime.now()}')
      ..writeln('build: $mode, ${Platform.operatingSystem}; '
          'reference hardware: ${referenceHardware ? "yes" : "no"} '
          '(LUMIT_REFERENCE_HW), so B1 ${referenceHardware ? "gates" : "is recorded only"}')
      ..writeln('window: ${view.physicalSize.width.toStringAsFixed(0)}x'
          '${view.physicalSize.height.toStringAsFixed(0)} '
          'dpr=${view.devicePixelRatio} '
          'refresh=${view.display.refreshRate.toStringAsFixed(0)}Hz; '
          'layers: $layers, rows on screen: ${h.outlineRows.length}, '
          'wheel legs: $notches notches');

    // ---- B1: the §2 gesture list ------------------------------------------
    await h.measure('idle 3 s', () async {
      await Future<void>.delayed(const Duration(seconds: 3));
    });
    Future<void> wheelLegs(Offset at) async {
      for (final dir in [1, -1]) {
        for (var i = 0; i < notches; i++) {
          h.wheel(at, 120.0 * dir);
          await Future<void>.delayed(const Duration(milliseconds: 25));
        }
      }
    }

    await h.measure(
        'scroll lanes, wheel, 25 ms a notch', () => wheelLegs(laneCentre));
    await h.measure(
        'scroll outline, wheel, 25 ms a notch', () => wheelLegs(outlineCentre));
    await h.measure('zoom, ctrl+wheel 14 in + 14 out', () async {
      await h.withCtrl(() async {
        for (final dir in [-1, 1]) {
          for (var i = 0; i < 14; i++) {
            h.wheel(laneCentre, 120.0 * dir);
            await Future<void>.delayed(const Duration(milliseconds: 90));
          }
        }
      });
    });
    // The rows actually on screen. The Timeline builds a screenful either
    // side of the window too, and a click aimed at one of those lands on
    // whatever is really there.
    List<Element> visibleRows() => [
          for (final e in tester.elementList(find.byType(OutlineRow)))
            if (h.rectOf(e) case final r?
                when r.top >= lanes.top && r.bottom <= lanes.bottom)
              e,
        ];
    Rect? nthRow(int n) {
      final live = visibleRows();
      return live.isEmpty ? null : h.rectOf(live[n % live.length]);
    }

    Rect? twirlOf(UuidValue id) {
      final found = find.byKey(ValueKey<String>('tl-twirl-$id'));
      return found.evaluate().isEmpty ? null : tester.getRect(found);
    }

    // Rows whose twirl is shut, by id: a twirl press is measured as the
    // row opening, so the rows are chosen by that and held by id rather
    // than by position, which moves as rows above them open.
    List<UuidValue> shutRows() => [
          for (final e in visibleRows())
            if (!(e.widget as OutlineRow).open)
              (e.widget as OutlineRow).entry.layer.internallayerId,
        ];

    // Row name cells: x = left + 40 is past the twirl, number and chip, so
    // a click is a plain select and never a switch toggle.
    await h.measure('select, 6 clicks on row names', () async {
      for (var i = 0; i < 6; i++) {
        final row = nthRow(i * 3 + 1);
        if (row == null) continue;
        await h.click(Offset(row.left + 40, row.center.dy));
        await Future<void>.delayed(const Duration(milliseconds: 150));
      }
    });
    final shut = shutRows();
    final twirled = [
      for (var i = 0; i < 4; i++) shut[(i * 2 + 2) % shut.length]
    ];
    await h.measure('twirl, 4 rows open then shut', () async {
      for (var pass = 0; pass < 2; pass++) {
        for (final id in twirled) {
          final twirl = twirlOf(id);
          if (twirl == null) continue;
          await h.click(twirl.center);
          await Future<void>.delayed(const Duration(milliseconds: 150));
        }
      }
    });
    final rulerY = ruler.top + ruler.height * 0.30;
    await h.measure('scrub, playhead drag right 2.5 s', () async {
      await h.drag(
          Offset(ruler.left + ruler.width * 0.15, rulerY),
          Offset(ruler.left + ruler.width * 0.75, rulerY),
          const Duration(milliseconds: 2500));
    });
    await h.measure('scrub, playhead drag back left 2.5 s', () async {
      await h.drag(
          Offset(ruler.left + ruler.width * 0.75, rulerY),
          Offset(ruler.left + ruler.width * 0.25, rulerY),
          const Duration(milliseconds: 2500));
    });

    // ---- B2: the frame after the press --------------------------------------
    // A row selects on pointer down (its raw Listener, outside the arena),
    // so the press alone is the input. A twirl is a tap, complete on the
    // release. A scrub is a drag, and the ruler's own tap-down only fires
    // when the arena's press deadline (100 ms) has passed, so the input a
    // scrub is measured from is the press plus its first move, which the
    // drag recogniser answers at once.
    for (var i = 0; i < 3; i++) {
      final live = visibleRows();
      final el = live[(i * 5 + 3) % live.length];
      final id = (el.widget as OutlineRow).entry.layer.internallayerId;
      final rect = h.rectOf(el)!;
      final at = Offset(rect.left + 40, rect.center.dy);
      late int pointer;
      await h.ack('select row ${i + 1}', at, () => pointer = h.press(at),
          () => h.rowSelected(id));
      h.release(pointer, at);
    }
    for (var i = 0; i < 2; i++) {
      final shut = shutRows();
      final id = shut[(i * 3 + 1) % shut.length];
      final at = twirlOf(id)!.center;
      await h.ack('twirl row ${i + 1}', at, () async {
        final pointer = h.press(at);
        await Future<void>.delayed(const Duration(milliseconds: 40));
        h.release(pointer, at);
      }, () => h.rowOpen(id));
    }
    {
      final before = h.readoutFrame();
      final from = Offset(ruler.left + ruler.width * 0.6, rulerY);
      final to = from + const Offset(8, 0);
      late int pointer;
      await h.ack('scrub, press and first move', from, () {
        pointer = h.press(from);
        h.send(PointerMoveEvent(
            pointer: pointer,
            position: to,
            delta: to - from,
            kind: PointerDeviceKind.mouse,
            buttons: kPrimaryButton));
      }, () => h.readoutFrame() != before);
      h.release(pointer, to);
    }

    final table = h.table(header.toString());
    // ignore: avoid_print
    print(table);
    if (outPath.isNotEmpty) File(outPath).writeAsStringSync(table);

    final missedB2 = [
      for (final (name, frames) in h.acks)
        if (frames != 1) name,
    ];
    expect(missedB2, isEmpty,
        reason: 'B2: a press must be answered on the next frame\n$table');
    if (referenceHardware) {
      final overB1 = [
        for (final r in h.rows)
          if (r.name != 'idle 3 s' && r.overB1) r.name,
      ];
      expect(overB1, isEmpty,
          reason: 'B1: build p95 over $b1BudgetMs ms on the reference '
              'desktop\n$table');
    }
  }, timeout: const Timeout(Duration(minutes: 10)));
}
