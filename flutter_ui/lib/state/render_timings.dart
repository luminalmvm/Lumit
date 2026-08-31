// What the last measured frame cost, per layer and per effect (docs/13 §7.1).
//
// **In plain terms.** "Why is this comp slow?" should be answerable by looking
// at it. The engine can measure each layer's own picture and each effect within
// it, and this holds the latest set of those numbers so the Timeline's
// render-time column and the Effect controls panel can show them beside the
// things they are about.
//
// **Measuring is on by default, and the switch is in the bottom strip.** It is
// not free — the engine waits for the graphics card at every node, so a
// millisecond means the work rather than the paperwork, and a measured frame is
// composited rather than served from a cache — but numbers are what the column
// is *for*, and the first arrangement (off by default, switched on by a glyph in
// the column header) meant the honest answer to "why is my column empty" was
// "you have to find the switch". So it starts on, the clock in the bottom strip
// beside the cache meters turns it off for the session, and everything that
// shows a number reads the same one set.

import 'package:flutter/foundation.dart';

import '../src/rust/api/cache.dart';
import '../src/rust/api/state.dart';

/// The numbers from the last measured frame, and the switch that asks for them.
class RenderTimings extends ChangeNotifier {
  /// How the engine is asked to start or stop measuring. Injectable so the
  /// rules below can be tested without a bridge library loaded; the default is
  /// the real call and every caller in the application uses it.
  final void Function(bool on) _askEngine;

  /// Called when measuring starts, to ask for the frame under the playhead
  /// again. A number only exists for a frame the engine *composites*, and the
  /// frame on screen has already been made — so without a fresh ask the column
  /// would stay empty until something else happened to want a render.
  final void Function()? _onMeasuringStarted;

  /// Called when the engine refuses the switch. Nothing about a failed call
  /// used to be visible: the flag went on, the interface lit its stopwatch, and
  /// nothing was ever measured — a switch that looks on and does nothing is the
  /// hardest kind of fault to report, so this says so out loud instead.
  final void Function(Object error)? _onEngineError;

  /// [measuring] starts **on**, matching the engine's own default, so the
  /// column fills by itself and no call is needed at startup to put the two
  /// sides in step. The switch that turns it off lives in the bottom strip.
  RenderTimings({
    bool measuring = true,
    void Function(bool on)? askEngine,
    void Function()? onMeasuringStarted,
    void Function(Object error)? onEngineError,
  })  : _measuring = measuring,
        _askEngine = askEngine ?? ((on) => setRenderProfiling(on_: on)),
        _onMeasuringStarted = onMeasuringStarted,
        _onEngineError = onEngineError;

  bool _measuring;

  int? _frame;
  double? _totalMs;
  List<RenderStageMs> _stages = const [];
  Map<String, double> _layers = const {};
  Map<String, double> _effects = const {};

  /// The frame these numbers are of, or null before the first measured frame.
  int? get frame => _frame;

  /// The whole frame's cost, including the stages no layer owns. Null while
  /// measuring is on but no measured frame has arrived yet — which the column's
  /// header shows as `…`, so "the engine is not reporting" and "the engine
  /// reported, but not about this row" are different things on screen.
  double? get totalMs => _totalMs;

  /// True while the engine is measuring — what the indicators read to tell
  /// "not measured" from "measured, and this layer cost nothing".
  bool get measuring => _measuring;

  /// Where the total went, stage by stage (plan, decode, build, composite,
  /// present), in the render's own order. Empty before the first measured
  /// frame. This is what lets the header explain a total no layer row owns —
  /// a heavy draw-list build, a slow decode — instead of leaving it hanging
  /// over the column unattributed.
  List<RenderStageMs> get stages => _stages;

  /// One layer's cost in milliseconds, or null when the last measured frame
  /// had no such layer (it was hidden, out of its span, or inside a Precomp).
  double? layerMs(String layerId) => _layers[layerId];

  /// One effect instance's cost in milliseconds, or null as above.
  double? effectMs(String effectId) => _effects[effectId];

  /// Turn measuring on or off. Turning it off drops the numbers as well, so an
  /// indicator switched back on never opens on a stale frame's costs — which
  /// would be the one reading worse than none at all.
  void setMeasuring(bool on) {
    if (_measuring == on) return;
    // The engine first, and the flag only if it agreed: a switch that says it
    // is measuring while the engine never heard the ask is exactly the state
    // that reads as "this feature does not work".
    try {
      _askEngine(on);
    } catch (error) {
      _onEngineError?.call(error);
      return;
    }
    _measuring = on;
    if (on) {
      _onMeasuringStarted?.call();
    } else {
      _frame = null;
      _totalMs = null;
      _stages = const [];
      _layers = const {};
      _effects = const {};
    }
    notifyListeners();
  }

  /// A measured frame arrived. Ignored once measuring is off: a frame already
  /// in flight when the switch went out is not a reason to put numbers back.
  void report(BridgeFrameProfile profile) {
    if (!_measuring) return;
    _frame = profile.frame.toInt();
    _totalMs = profile.totalMs;
    _stages = [
      RenderStageMs(RenderStageKind.plan, profile.planMs),
      RenderStageMs(RenderStageKind.decode, profile.decodeMs),
      RenderStageMs(RenderStageKind.build, profile.buildMs),
      RenderStageMs(RenderStageKind.composite, profile.compositeMs),
      RenderStageMs(RenderStageKind.present, profile.presentMs),
    ];
    final layers = <String, double>{};
    final effects = <String, double>{};
    for (final layer in profile.layers) {
      layers[layer.layer] = layer.ms;
      for (final effect in layer.effects) {
        effects[effect.effect] = effect.ms;
      }
    }
    _layers = layers;
    _effects = effects;
    notifyListeners();
  }
}

/// The five stages a render passes through, in order. The words shown for
/// them come from the arb (`stagePlan`…`stagePresent`) — this is only which.
enum RenderStageKind { plan, decode, build, composite, present }

/// One stage's share of the measured frame.
class RenderStageMs {
  final RenderStageKind kind;
  final double ms;
  const RenderStageMs(this.kind, this.ms);
}

/// The stage that would explain the total to someone reading the column: the
/// costliest stage, but only when it is not compositing (whose time the layer
/// rows already itemise) and only when it genuinely dominates — owning more of
/// the frame than every other stage combined. Null otherwise, and the header
/// shows the plain total it always showed.
RenderStageKind? dominantUnownedStage(List<RenderStageMs> stages) {
  if (stages.isEmpty) return null;
  var total = 0.0;
  RenderStageMs? top;
  for (final s in stages) {
    total += s.ms;
    if (top == null || s.ms > top.ms) top = s;
  }
  if (top == null || top.kind == RenderStageKind.composite) return null;
  return top.ms > total - top.ms ? top.kind : null;
}

/// A measured cost as the indicators write it: milliseconds to one decimal
/// place while that is readable, whole milliseconds once the number is large,
/// and seconds past a thousand — so a column of them stays the same width and
/// a slow layer is obvious without arithmetic.
String formatRenderMs(double ms) {
  if (!ms.isFinite || ms < 0) return '—';
  if (ms >= 1000) return '${(ms / 1000).toStringAsFixed(2)} s';
  if (ms >= 100) return '${ms.round()} ms';
  // Two decimals always (owner's ruling): every value in the column carries
  // the same shape, so right-aligned tabular figures stack their dots into
  // one line and the column reads at a glance.
  return '${ms.toStringAsFixed(2)} ms';
}
