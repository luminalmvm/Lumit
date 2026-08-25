// One answer to "what time is frame N?" — and to "what markers does this comp
// have?" — shared by everything that draws.
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

/// Forget everything. Called on every committed engine change.
void clearCompTimeCache() {
  _times.clear();
  _frames.clear();
  _markers.clear();
}
