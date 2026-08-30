// One answer to "what time is frame N?", to "what markers does this comp
// have?" and to "what does this curve read now?" — shared by everything that
// draws.
//
// The conversion belongs to the engine (docs/17 §, `CompositionReference::
// time_of_frame` exists so no frontend does frame-rate arithmetic itself), but
// asking for it is a bridge call, and *every* animated row asks on *every*
// rebuild. A single click on the timeline ruler moved twenty of them, which is
// twenty crossings of the boundary for twenty copies of the same answer.
//
// So the answers are remembered here. The engine still computes each one; it
// just computes each one once. The memory is thrown away whenever the engine
// reports a committed change, because a comp-settings edit — or an undo of one
// — is the only thing that can change a frame rate, and therefore the only
// thing that can make a remembered answer wrong.

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

/// Frame → comp time, per composition. Keyed by the reference itself, which
/// compares by project and comp id, so two panels holding their own handle to
/// the same comp share one set of answers.
final _times = <CompositionReference, Map<int, BridgeRational>>{};

/// How many frames one comp remembers before starting over. A long playback
/// scrubs through every frame it plays, and nothing commits a change while it
/// runs, so without a ceiling the memory grows for as long as the session does.
const _maxPerComp = 8192;

/// The comp time of `frame`, from memory when it is there.
///
/// Prefer this to calling `comp.timeOfFrame` directly anywhere that runs during
/// a build or a paint: those paths must not make bridge calls.
BridgeRational timeOfFrame(CompositionReference comp, int frame) {
  final times = _times[comp] ??= {};
  if (times.length >= _maxPerComp) times.clear();
  return times[frame] ??= comp.timeOfFrame(frame: frame);
}

/// Comp time → frame, per composition: the same trip the other way, and the
/// more chatty of the two. Every row that draws a keyframe diamond walks its
/// whole key list asking which frame each key sits on, on every rebuild.
final _frames = <CompositionReference, Map<BridgeRational, int>>{};

/// The frame `time` falls on, from memory when it is there.
///
/// Times that mean the same thing but are written differently (2/4 and 1/2)
/// count as different questions here, so the worst a mismatch costs is a call
/// that would have happened anyway.
int frameAtTime(CompositionReference comp, BridgeRational time) {
  final frames = _frames[comp] ??= {};
  if (frames.length >= _maxPerComp) frames.clear();
  return frames[time] ??= comp.frameAtTime(time: time);
}

/// A comp's markers, remembered until the document changes (K-254).
///
/// Same bargain as the two above, and the one that made it worth having: the
/// time ruler draws markers on every rebuild — sixty times a second while
/// playback runs — and `get_markers` walks the whole list across the boundary
/// each time. What it answers can only change when the document changes.
final _markers = <CompositionReference, List<BridgeMarker>>{};

/// The comp's markers, from memory when they are there.
///
/// Prefer this to `comp.getMarkers()` anywhere that runs during a build or a
/// paint. Anything that *writes* markers must go through [writeMarkers], which
/// is what keeps this honest.
List<BridgeMarker> markersOf(CompositionReference comp) =>
    _markers[comp] ??= comp.getMarkers();

/// Replace a comp's whole marker list and forget the remembered copy.
///
/// The one way markers are written. A caller that reached for `setMarkers`
/// directly would leave [markersOf] answering the old list until something
/// else happened to clear it, which is the sort of bug that shows up as a
/// marker springing back after you drag it.
void writeMarkers(CompositionReference comp, List<BridgeMarker> markers) {
  comp.setMarkers(markers: markers);
  _markers.remove(comp);
}

/// The engine's answers to "what does this curve read at this time",
/// remembered per (scalar, time): the engine still computes each answer, once,
/// rather than once per rebuild of every animated row (K-184). A freezed
/// scalar compares by value, so an edited curve is a new question here, never
/// a stale answer; the ceiling only stops a long session growing forever.
final Map<(BridgeScalar, BridgeRational), double> _scalarSamples = {};

/// The scalars asked for at [_hotTime] — which is to say the animated rows on
/// screen, as of the last frame anything drew.
///
/// **What makes the sampling one call a frame rather than one call a row.**
/// Remembering an answer helps a rebuild that asks the same question twice; it
/// cannot help a scrub, where every frame is a new time and so a new question
/// for every row at once. Playback with a `U` open therefore crossed the
/// boundary once per animated row per frame, and the crossings grew with the
/// number of lanes open — which is why frames the cache already held could
/// still arrive late.
///
/// So the *first* row to miss at a new time samples the whole set the previous
/// frame used, in one crossing, and every row after it reads its answer out of
/// memory. A row that has just appeared misses once and joins the set; one that
/// has gone falls out of it the next time the playhead moves. Nothing registers
/// or unregisters — the set is simply what was asked for last.
Set<BridgeScalar> _hot = {};
BridgeRational? _hotTime;

/// What [scalar] reads at [time], from memory when it is there and from one
/// batched call across the whole screen when it is not.
///
/// Prefer this to `sampleScalar` anywhere a row shows the value under the
/// playhead. `sampleScalarWithContext` is a different question — an expression
/// needs the layer it runs on — and stays as it is.
double sampledScalar(BridgeScalar scalar, BridgeRational time) {
  if (_scalarSamples.length >= 8192) _scalarSamples.clear();
  final held = _scalarSamples[(scalar, time)];
  if (held != null) {
    _markHot(scalar, time);
    return held;
  }
  // The miss, and everything the last frame wanted that this one has not been
  // asked for yet: one call for the lot.
  final wanted = <BridgeScalar>[
    scalar,
    for (final other in _hot)
      if (other != scalar && !_scalarSamples.containsKey((other, time))) other,
  ];
  final values = sampleScalars(scalars: wanted, time: time);
  for (var i = 0; i < wanted.length && i < values.length; i++) {
    _scalarSamples[(wanted[i], time)] = values[i];
  }
  _markHot(scalar, time);
  return values.isEmpty ? 0 : values.first;
}

/// Note that [scalar] was wanted at [time], starting the set over when the time
/// has moved on.
void _markHot(BridgeScalar scalar, BridgeRational time) {
  if (_hotTime != time) {
    _hotTime = time;
    _hot = {};
  }
  _hot.add(scalar);
}

/// Forget everything. Called on every committed engine change.
void clearCompTimeCache() {
  _times.clear();
  _frames.clear();
  _markers.clear();
  _scalarSamples.clear();
  // [_hot] deliberately survives: it is not an answer that can go stale, only
  // the list of curves something drew last. Emptying it would cost the next
  // frame its batch — an edited scalar in it is sampled once for nothing and
  // then drops out.
}
