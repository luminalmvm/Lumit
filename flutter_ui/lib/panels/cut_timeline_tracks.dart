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

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

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
/// picture and every sound row stands below the dividing line.
List<CutTrack> cutTimelineTracks(List<BridgeLayerEntry> layers) {
  final above = <CutTrack>[];
  final below = <CutTrack>[];
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
    }
  }
  return [...above, ...below];
}

/// How many tracks stand above the dividing line.
int cutPictureCount(List<CutTrack> tracks) {
  final at = tracks.indexWhere((track) => track.sound);
  return at < 0 ? tracks.length : at;
}

/// What a clip's thumbnail is filed under: the frame it opens on is part of
/// the key, so a cut or a re-speed fetches the frame the clip now starts on.
String cutThumbKey(BridgeClip clip) =>
    '${clip.id}@${clip.startFrame}@${clip.retimed}';

/// Which track the point [y] lands on, measured down from the top of the
/// first track, with every track [height] tall. Null above the first or below
/// the last.
int? cutTrackAt(List<CutTrack> tracks, double y, double height) {
  if (y < 0 || height <= 0) return null;
  final at = y ~/ height;
  return at < tracks.length ? at : null;
}

/// Where a clip's box begins and how wide it is drawn: never under two pixels,
/// so a clip of one frame is still something the pointer can find.
(double, double) cutClipBox(TimelineAxis axis, int start, int end) {
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
/// how many tracks down it has crossed, and for a roll the edit point that is
/// moving.
typedef CutDrag = ({
  CutTool tool,
  Set<String> clips,
  BarGrab grab,
  int shift,
  int trackShift,
  int? editPoint,
});

/// Where a clip's box stands while [drag] is on: its own frames shifted by
/// what the gesture has done to them. A roll moves every edge standing on
/// the edit point, whichever clip it belongs to; a slip moves no box at all.
(int, int) cutDraggedSpan(BridgeClip clip, CutDrag? drag) {
  final start = clip.startFrame.toInt();
  final end = clip.endFrame.toInt();
  if (drag == null) return (start, end);
  if (drag.tool == CutTool.roll) {
    final at = drag.editPoint;
    return (
      at != null && start == at ? start + drag.shift : start,
      at != null && end == at ? end + drag.shift : end,
    );
  }
  if (drag.tool == CutTool.slip || !drag.clips.contains(clip.id.toString())) {
    return (start, end);
  }
  return switch (drag.grab) {
    BarGrab.move => (start + drag.shift, end + drag.shift),
    BarGrab.trimIn => (start + drag.shift, end),
    BarGrab.trimOut => (start, end + drag.shift),
  };
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
