// The Cut timeline's rows, and the arithmetic over them: which layers are
// tracks, which side of the dividing line each stands on, which clips are in
// the visible stretch of time, and what a press at a point takes hold of.
//
// Pure, so the painter, the pointer handlers and the drop all read one answer
// and cannot disagree about where a clip is. Nothing here walks every clip on
// a pointer move: the clips are sorted once when the model changes, and a
// window of them is found by binary search.
//
// This panel calls its rows tracks, in its own strings and files only. Every
// one of them is a layer, and the ones that hold clips are Sequence layers.

import 'dart:math';
import 'dart:ui' show Offset, Rect;

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'clip_fades.dart' show clipFadeEdge;
import 'timeline_bar_frb.dart' show BarGrab, barGrabAt;
import 'timeline_extras_frb.dart' show TimelineAxis;

/// The tools the strip offers, in the strip's order. Select moves and trims
/// without moving anything else; Ripple trims and closes or opens the room
/// behind the edge; Roll moves an edit point between two clips; Slip changes
/// which frames a clip shows without moving it; Slide moves a clip between
/// its neighbours, which trim to keep against it.
enum CutTool { select, razor, ripple, roll, slip, slide }

/// Which side of the dividing line a track stands on, and how its bar draws.
enum CutTrackKind {
  /// A Sequence layer with a picture: boxes with a thumbnail and a name.
  picture,

  /// An audio-only Sequence layer: boxes with a waveform, overlaps drawn as a
  /// crossfade.
  sound,

  /// Any other layer, a text or a solid or an uncut footage layer: one plain
  /// bar from its in point to its out point.
  plain,

  /// The sound of an uncut footage layer, which also stands above the line
  /// as its plain bar: one bar with the layer's waveform. A second row for
  /// the same layer, so the document holds nothing new.
  layerSound,
}

/// One row of the Cut timeline.
class CutTrack {
  final BridgeLayerEntry entry;
  final String id;
  final CutTrackKind kind;

  /// The layer's clips sorted by start frame, with a prefix of how far the
  /// clips up to each index reach, so the clips over a stretch of time are
  /// found by two binary searches rather than a walk.
  final List<BridgeClip> clips;
  final List<int> _reach;

  CutTrack._(this.entry, this.id, this.kind, this.clips)
      : _reach = List<int>.filled(clips.length, 0) {
    var far = 0;
    for (var i = 0; i < clips.length; i++) {
      final end = clips[i].endFrame.toInt();
      if (i == 0 || end > far) far = end;
      _reach[i] = far;
    }
  }

  bool get sound => kind == CutTrackKind.sound;

  /// Whether the row stands below the dividing line.
  bool get below => sound || kind == CutTrackKind.layerSound;

  /// The index range `[lo, hi)` of the clips touching `[from, to]` in comp
  /// frames. Both ends by binary search: the starts are sorted, and the
  /// prefix reach is monotone by construction.
  (int, int) window(int from, int to) {
    if (clips.isEmpty) return (0, 0);
    var lo = 0;
    var hi = clips.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (_reach[mid] < from) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    final first = lo;
    lo = first;
    hi = clips.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (clips[mid].startFrame.toInt() <= to) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return first < lo ? (first, lo) : (0, 0);
  }

  /// The clip with this id, or null when the track no longer holds it.
  BridgeClip? clip(String id) {
    for (final clip in clips) {
      if (clip.id.toString() == id) return clip;
    }
    return null;
  }
}

/// Every track the panel draws: the picture and plain rows in stack order,
/// then the sound rows in stack order, so the top-most layer still wins the
/// picture and every sound row stands below the dividing line. Last come the
/// sound rows of the uncut footage layers [hasAudio] says carry sound, by
/// layer id: after the sound tracks, so those stay next to each other and a
/// clip crosses between them as it always did.
///
/// With [anyOrder] there is no dividing line: every track stands in stack
/// order, and an uncut footage layer's sound row stands under its bar.
List<CutTrack> cutTimelineTracks(List<BridgeLayerEntry> layers,
    {Map<String, bool> hasAudio = const {}, bool anyOrder = false}) {
  final above = <CutTrack>[];
  final below = <CutTrack>[];
  final own = <CutTrack>[];
  if (anyOrder) {
    final all = <CutTrack>[];
    for (final entry in layers) {
      final info = entry.info;
      final id = entry.layer.internallayerId.toString();
      final clips = [...info.clips]
        ..sort((a, b) => a.startFrame.compareTo(b.startFrame));
      if (info.kind == BridgeLayerKind.audio) {
        all.add(CutTrack._(entry, id, CutTrackKind.sound, clips));
      } else if (clips.isNotEmpty || info.kind == BridgeLayerKind.sequence) {
        all.add(CutTrack._(entry, id, CutTrackKind.picture, clips));
      } else {
        all.add(CutTrack._(entry, id, CutTrackKind.plain, clips));
        if (info.kind == BridgeLayerKind.footage && (hasAudio[id] ?? false)) {
          all.add(
              CutTrack._(entry, '$id-sound', CutTrackKind.layerSound, clips));
        }
      }
    }
    return all;
  }
  for (final entry in layers) {
    final info = entry.info;
    final id = entry.layer.internallayerId.toString();
    final clips = [...info.clips]
      ..sort((a, b) => a.startFrame.compareTo(b.startFrame));
    if (info.kind == BridgeLayerKind.audio) {
      below.add(CutTrack._(entry, id, CutTrackKind.sound, clips));
    } else if (clips.isNotEmpty || info.kind == BridgeLayerKind.sequence) {
      above.add(CutTrack._(entry, id, CutTrackKind.picture, clips));
    } else {
      above.add(CutTrack._(entry, id, CutTrackKind.plain, clips));
      // An id of its own, since the rows are keyed by it.
      if (info.kind == BridgeLayerKind.footage && (hasAudio[id] ?? false)) {
        own.add(CutTrack._(entry, '$id-sound', CutTrackKind.layerSound, clips));
      }
    }
  }
  return [...above, ...below, ...own];
}

/// How many tracks stand above the dividing line.
int cutPictureCount(List<CutTrack> tracks) {
  final at = tracks.indexWhere((track) => track.below);
  return at < 0 ? tracks.length : at;
}

/// Where a picture clip's opacity line stands in a track [height] tall: at
/// the box's top at full, at its foot at none, as Vegas draws event opacity.
double cutOpacityY(double opacity, double height) =>
    2 + (1 - opacity.clamp(0.0, 1.0)) * max(0.0, height - 4);

/// What a clip's thumbnail is filed under: the source frame it opens on is
/// part of the key, so a head trim, a slip or a re-speed fetches again, and a
/// move along the comp keeps the pictures it has.
String cutThumbKey(BridgeClip clip) =>
    '${clip.id}@${clip.sourceIn.num}/${clip.sourceIn.den}@${clip.retime.hashCode}';

/// How far apart the pictures along a clip are asked for, in pixels. The
/// painter draws whole frames and takes the nearest one it has.
const double cutFilmStep = 48;

/// How many comp frames apart the pictures along a clip are at [perFrame]
/// pixels a frame: a power of two at least [cutFilmStep] wide, so nearby
/// zooms share their pictures.
int cutFilmFrames(double perFrame) {
  if (perFrame <= 0) return 1;
  var frames = 1;
  while (frames * perFrame < cutFilmStep && frames < 1 << 20) {
    frames *= 2;
  }
  return frames;
}

/// How many comp frames into its clip picture [k] along shows.
int cutFilmOffset(int k, double perFrame) => k * cutFilmFrames(perFrame);

/// The key a picture [offset] comp frames into [clip] is held under.
String cutFilmKey(BridgeClip clip, int offset) =>
    '${cutThumbKey(clip)}+$offset';

/// Where each track's top stands, measured down from the top of the first,
/// with the foot of the last on the end: one more entry than [heights].
List<double> cutTrackTops(List<double> heights) {
  final tops = List<double>.filled(heights.length + 1, 0);
  for (var i = 0; i < heights.length; i++) {
    tops[i + 1] = tops[i] + heights[i];
  }
  return tops;
}

/// Which track the point [y] lands on, measured down from the top of the
/// first track, with [tops] from [cutTrackTops]. Null above the first or
/// below the last. By binary search, since a pointer move asks it.
int? cutTrackAt(List<CutTrack> tracks, double y, List<double> tops) {
  if (y < 0 || tops.length <= tracks.length || y >= tops.last) return null;
  var lo = 0;
  var hi = tracks.length - 1;
  while (lo < hi) {
    final mid = (lo + hi) >> 1;
    if (tops[mid + 1] <= y) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  return lo;
}

/// The tracks whose rows cross the stretch from [top] to [bottom], as the
/// range `[first, last)`.
(int, int) cutTracksBetween(List<double> tops, double top, double bottom) {
  final count = tops.length - 1;
  var first = 0;
  while (first < count && tops[first + 1] <= top) {
    first++;
  }
  var last = first;
  while (last < count && tops[last] < bottom) {
    last++;
  }
  return (first, last);
}

/// How short and how tall a track may be dragged, against the usual height.
const double cutTrackMinScale = 0.5;
const double cutTrackMaxScale = 4;

/// Where a clip's box begins and how wide it is drawn: never under two pixels,
/// so a clip of one frame is still something the pointer can find.
(double, double) cutClipBox(TimelineAxis axis, num start, num end) {
  final left = axis.xOf(start);
  return (left, (axis.xOf(end) - left).clamp(2.0, double.infinity));
}

/// Which clip a press at [x] takes hold of, and what of it, or null on empty
/// ground. Every clip's edges are hit before any clip's body, which is what
/// keeps the earlier clip's tail reachable under the later clip of a
/// crossfade. Only the clips near [x] are looked at.
({BridgeClip clip, BarGrab grab})? cutClipGrabAt(
    CutTrack track, TimelineAxis axis, double x) {
  final frame = axis.frameAtExact(x);
  final slack = axis.framesOfPx(8).ceil();
  final (lo, hi) = track.window(frame.floor() - slack, frame.ceil() + slack);
  for (var i = hi - 1; i >= lo; i--) {
    final clip = track.clips[i];
    final (left, width) =
        cutClipBox(axis, clip.startFrame.toInt(), clip.endFrame.toInt());
    if (x < left || x > left + width) continue;
    final grab = barGrabAt(x - left, width);
    if (grab != BarGrab.move) return (clip: clip, grab: grab);
  }
  for (var i = hi - 1; i >= lo; i--) {
    final clip = track.clips[i];
    final (left, width) =
        cutClipBox(axis, clip.startFrame.toInt(), clip.endFrame.toInt());
    if (x >= left && x <= left + width) return (clip: clip, grab: BarGrab.move);
  }
  return null;
}

/// How big a fade handle's target is: the Audio timeline's own corner handle,
/// standing across the top of the track with its middle on the fade's inner
/// end.
const double cutFadeHandleWidth = 12;
const double cutFadeHandleHeight = 14;

/// The narrowest box that offers its corners: room for a handle at each end
/// clear of the other end's trim zone.
const double cutFadeHandleMinWidth = 24;

/// How long the fade at one end of [clip] is, in frames, with a corner drag
/// of [shift] frames carried into it: a start corner lengthens its fade by
/// moving right and an end corner by moving left, and neither passes the
/// clip's other end.
int cutFadeFrames(BridgeClip clip, double fps,
    {required bool into, int shift = 0}) {
  final length = clip.endFrame.toInt() - clip.startFrame.toInt();
  final now = ((into ? clip.fadeIn : clip.fadeOut).seconds * fps).round();
  return (now + (into ? shift : -shift)).clamp(0, max(0, length));
}

/// Which fade handle a press at [x], [y] down from the track's top, takes
/// hold of, or null. A handle stands at each end nothing meets, since an end
/// a neighbour meets or overlaps is a dissolve and its edge already drags
/// that; and only on a box wide enough for a corner to be told from the trim
/// zone. Asked before [cutClipGrabAt], so the handle has the whole of its own
/// box, as the Audio timeline's has, and the edge is trimmed from below it.
/// Only the clips near [x] are looked at.
({BridgeClip clip, bool into})? cutFadeGrabAt(
    CutTrack track, TimelineAxis axis, double x, double y, double fps) {
  if (y < 0 || y > cutFadeHandleHeight || fps <= 0) return null;
  final frame = axis.frameAtExact(x);
  final slack = axis.framesOfPx(cutFadeHandleWidth / 2).ceil();
  final (lo, hi) = track.window(frame.floor() - slack, frame.ceil() + slack);
  for (var i = hi - 1; i >= lo; i--) {
    final clip = track.clips[i];
    final (_, width) =
        cutClipBox(axis, clip.startFrame.toInt(), clip.endFrame.toInt());
    if (width < cutFadeHandleMinWidth) continue;
    for (final into in const [true, false]) {
      if (cutClipAcross(track, clip, endEdge: !into) != null) continue;
      final at = axis.xOf(clipFadeEdge(clip, into: into, fps: fps));
      if ((x - at).abs() <= cutFadeHandleWidth / 2) {
        return (clip: clip, into: into);
      }
    }
  }
  return null;
}

/// The ids of every clip whose box the marquee [left]..[right] crosses on
/// [track], in comp frames.
Set<String> cutClipsBetween(CutTrack track, int from, int to) {
  final (lo, hi) = track.window(from, to);
  return {
    for (var i = lo; i < hi; i++)
      if (track.clips[i].endFrame.toInt() >= from &&
          track.clips[i].startFrame.toInt() <= to)
        track.clips[i].id.toString(),
  };
}

/// A clip gesture in flight, as the painters read it: which tool, which clips
/// travel, what the hand has hold of, how far it has gone in whole frames,
/// which track it began on and how many tracks down it has crossed, for a
/// roll the edit point that is moving, for a ripple the frame everything
/// after moves from, for a slide the neighbours' edges that follow, by
/// clip id as (start, end) shifts, and for a fade handle which end's fade is
/// in hand, the start's when [fade] is true, the end's when it is false, with
/// the travel in [shift] and no box moving at all. [sub] is the part of a
/// frame a move of sound alone carries past [shift] while Frames is off, and
/// nought for everything else.
typedef CutDrag = ({
  CutTool tool,
  Set<String> clips,
  BarGrab grab,
  int shift,
  int? track,
  int trackShift,
  int? editPoint,
  int? rippleFrom,
  Map<String, (int, int)> others,
  bool? fade,
  double sub,
});

/// How far a ripple moves what follows: the engine's rule. Everything from a
/// trimmed clip's old end moves by its change of length, and with the Ripple
/// switch on everything after a moved clip moves as far as it does. Zero for
/// any other gesture.
int cutRippleOf(CutDrag? drag) {
  if (drag == null || drag.rippleFrom == null) return 0;
  if (drag.grab == BarGrab.move) return drag.shift;
  if (drag.tool != CutTool.ripple) return 0;
  return switch (drag.grab) {
    BarGrab.trimOut => drag.shift,
    BarGrab.trimIn => -drag.shift,
    BarGrab.move => 0,
  };
}

/// Where a clip's box stands while [drag] is on: its own frames shifted by
/// what the gesture has done to them. A roll moves the edges of its two
/// clips, and their links, that stand on the edit point; a slip and a fade
/// handle move no box at all; a rippled head keeps the box's start and
/// shortens its end, as the engine will, and everything from the trimmed
/// clip's old end follows on every track that is not [locked].
(int, int) cutDraggedSpan(BridgeClip clip, CutDrag? drag,
    {bool locked = false}) {
  final start = clip.startFrame.toInt();
  final end = clip.endFrame.toInt();
  if (drag == null || drag.fade != null) return (start, end);
  final id = clip.id.toString();
  if (drag.others[id] case (final a, final b)) return (start + a, end + b);
  final mine = drag.clips.contains(id);
  if (drag.tool == CutTool.roll) {
    final at = drag.editPoint;
    if (!mine || at == null) return (start, end);
    return (
      start == at ? start + drag.shift : start,
      end == at ? end + drag.shift : end,
    );
  }
  if (drag.tool == CutTool.slip) return (start, end);
  final ripple = locked ? 0 : cutRippleOf(drag);
  if (!mine) {
    return ripple != 0 && start >= drag.rippleFrom!
        ? (start + ripple, end + ripple)
        : (start, end);
  }
  if (ripple != 0 && drag.grab != BarGrab.move) return (start, end + ripple);
  return switch (drag.grab) {
    BarGrab.move => (start + drag.shift, end + drag.shift),
    BarGrab.trimIn => (start + drag.shift, end),
    BarGrab.trimOut => (start, end + drag.shift),
  };
}

/// Where the clip's whole source would sit while [drag] pulls at it, in comp
/// frames: the reach, moved along by a slip or a rippled head, which keep the
/// box still and move the frames under it. Null at rest, on a move, on a
/// fade handle, where the reach is not known, and where the box already
/// fills it.
(int, int)? cutReachSpan(BridgeClip clip, CutDrag drag) {
  if (drag.fade != null || !drag.clips.contains(clip.id.toString())) {
    return null;
  }
  final reachStart = clip.reachStartFrame?.toInt();
  final reachEnd = clip.reachEndFrame?.toInt();
  if (reachStart == null || reachEnd == null) return null;
  final int along;
  switch (drag.tool) {
    case CutTool.slip:
      along = -drag.shift;
    case CutTool.roll:
      final at = drag.editPoint;
      if (clip.startFrame.toInt() != at && clip.endFrame.toInt() != at) {
        return null;
      }
      along = 0;
    case CutTool.select || CutTool.ripple:
      if (drag.grab == BarGrab.move) return null;
      along = drag.tool == CutTool.ripple && drag.grab == BarGrab.trimIn
          ? -drag.shift
          : 0;
    case CutTool.razor || CutTool.slide:
      return null;
  }
  final (start, end) = cutDraggedSpan(clip, drag);
  final a = reachStart + along;
  final b = reachEnd + along;
  return a < start || b > end ? (a, b) : null;
}

/// The clip standing on frame [at] of [track], or null on empty room. Where
/// two sound clips overlap in a crossfade, the later one, as a press finds.
BridgeClip? cutClipUnder(CutTrack track, int at) {
  final (lo, hi) = track.window(at, at);
  for (var i = hi - 1; i >= lo; i--) {
    final clip = track.clips[i];
    if (clip.startFrame.toInt() <= at && at < clip.endFrame.toInt()) {
      return clip;
    }
  }
  return null;
}

/// The nearest edit point on [track] strictly before or after frame [at]: a
/// clip's start or end. Found from the clips either side of [at] by binary
/// search, never by a walk.
int? cutEditPointNear(CutTrack track, int at, {required bool before}) {
  final clips = track.clips;
  if (clips.isEmpty) return null;
  // The first clip starting after [at].
  var lo = 0;
  var hi = clips.length;
  while (lo < hi) {
    final mid = (lo + hi) >> 1;
    if (clips[mid].startFrame.toInt() <= at) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  int? best;
  void offer(int frame) {
    if (before ? frame >= at : frame <= at) return;
    if (best == null || (before ? frame > best! : frame < best!)) best = frame;
  }

  // A few clips either side: the one under the playhead and its neighbours
  // hold the nearest edges, with room for a crossfade's overlap.
  for (var i = lo - 3; i < lo + 2; i++) {
    if (i < 0 || i >= clips.length) continue;
    offer(clips[i].startFrame.toInt());
    offer(clips[i].endFrame.toInt());
  }
  return best;
}

/// The clip across one edge of [clip] on the same track: the one that meets
/// the edge, or already runs into it, by the engine's own rule for what a
/// transition joins. Null where nothing stands across it, which is where a
/// transition is a fade.
BridgeClip? cutClipAcross(CutTrack track, BridgeClip clip,
    {required bool endEdge}) {
  final start = clip.startFrame.toInt();
  final end = clip.endFrame.toInt();
  final edge = endEdge ? end : start;
  final (lo, hi) = track.window(edge, edge);
  for (var i = lo; i < hi; i++) {
    final other = track.clips[i];
    if (other.id == clip.id) continue;
    final a = other.startFrame.toInt();
    final b = other.endFrame.toInt();
    final across = endEdge
        ? start <= a && a <= end && end <= b
        : a <= start && start <= b && b <= end;
    if (across) return other;
  }
  return null;
}

/// Whether [clip] overlaps a neighbour at one edge: a dissolve, or a
/// crossfade, that is already there.
bool cutEdgeOverlaps(CutTrack track, BridgeClip clip, {required bool endEdge}) {
  final other = cutClipAcross(track, clip, endEdge: endEdge);
  if (other == null) return false;
  return endEdge
      ? other.startFrame.toInt() < clip.endFrame.toInt()
      : other.endFrame.toInt() > clip.startFrame.toInt();
}

/// The clip that starts where [clip] ends on the same track, or null.
BridgeClip? cutClipAfter(CutTrack track, BridgeClip clip) {
  final end = clip.endFrame.toInt();
  final (lo, hi) = track.window(end, end);
  for (var i = lo; i < hi; i++) {
    if (track.clips[i].startFrame.toInt() == end) return track.clips[i];
  }
  return null;
}

/// [shift] held to what the source can give. The engine allows overrun, a
/// held frame past the media's end, but an edit that reaches for frames the
/// file has not got is nearly always a slip of the hand, so the drag stops at
/// the reach where it is known. An edge never passes the clip's other edge.
/// [next] is the clip on the far side of a rolled edit point.
int cutClampShift({
  required BridgeClip clip,
  required BarGrab grab,
  required CutTool tool,
  required int shift,
  BridgeClip? next,
}) {
  final start = clip.startFrame.toInt();
  final end = clip.endFrame.toInt();
  final reachStart = clip.reachStartFrame?.toInt();
  final reachEnd = clip.reachEndFrame?.toInt();
  var lo = -1 << 30;
  var hi = 1 << 30;
  switch (tool) {
    case CutTool.slip:
      if (reachStart != null) lo = reachStart - start;
      if (reachEnd != null) hi = reachEnd - end;
    case CutTool.roll:
      if (grab == BarGrab.trimOut) {
        if (reachEnd != null) hi = reachEnd - end;
        lo = -(end - start - 1);
        if (next != null) {
          hi = min(hi, next.endFrame.toInt() - next.startFrame.toInt() - 1);
          if (next.reachStartFrame case final reach?) {
            lo = max(lo, reach - next.startFrame.toInt());
          }
        }
      } else {
        hi = end - start - 1;
        if (reachStart != null) lo = reachStart - start;
        if (next != null) {
          lo = max(lo, -(next.endFrame.toInt() - next.startFrame.toInt() - 1));
          if (next.reachEndFrame case final reach?) {
            hi = min(hi, reach - next.endFrame.toInt());
          }
        }
      }
    case CutTool.select || CutTool.ripple:
      if (grab == BarGrab.trimIn) {
        hi = end - start - 1;
        if (reachStart != null) lo = reachStart - start;
      } else if (grab == BarGrab.trimOut) {
        lo = -(end - start - 1);
        if (reachEnd != null) hi = reachEnd - end;
      }
    case CutTool.razor || CutTool.slide:
      break;
  }
  if (lo > hi) return 0;
  return shift.clamp(lo, hi);
}

/// A button drawn on a clip, as Vegas puts its event buttons there.
enum CutBadge { fx, panCrop }

/// Where a clip's buttons sit in its [box]: fx at the top right of every
/// clip, and Pan/Crop beside it on a picture clip. None on a box too small to
/// keep room for its name.
List<(CutBadge, Rect)> cutBadges(Rect box, {required bool picture}) {
  // Below the band the fade handles are taken in, so a short fade-out's
  // handle and the fx button never share a pixel.
  final top = box.top + cutFadeHandleHeight - 1;
  final side = min(16.0, box.bottom - top - 2);
  if (side < 10 || box.width < side * 4 + 24) return const [];
  final fx = Rect.fromLTWH(box.right - 3 - side, top, side, side);
  return [
    (CutBadge.fx, fx),
    if (picture) (CutBadge.panCrop, fx.shift(Offset(-side - 2, 0))),
  ];
}
