// One track of the Cut timeline, painted.
//
// The clips are drawn rather than built: a two-hour comp has thousands of
// them, and a widget per clip is a layout per clip on every scroll. This
// painter is handed the track's sorted clips and draws only the ones in the
// stretch of time the lanes are showing, found by binary search, so a scroll
// or a zoom costs what is on screen rather than what the comp has.
//
// It takes no pointer. The panel's one listener over the lanes does the hit
// testing, through the same pure functions this draws from.

import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../theme/theme.dart';
import 'cut_timeline_tracks.dart';
import 'graph_maths.dart' show rationalSeconds;
import 'timeline_bar_frb.dart' show BarGrab;
import 'timeline_extras_frb.dart'
    show TimelineAxis, clipEdgeWidth, clipFillAlpha, clipFillSelectedAlpha;
import 'waveform_frb.dart';

/// The room a box leaves at the top and foot of its track.
const double cutClipInset = 2;

/// The colours a track is drawn in, read off the theme once by the panel so
/// the painter has no context to ask.
typedef CutLaneColours = ({
  Color label,
  Color edge,
  Color picked,
  Color cross,
  WaveformColours wave,
});

class CutTrackPainter extends CustomPainter {
  final CutTrack track;
  final TimelineAxis axis;
  final double fps;

  /// The lanes' horizontal scroll: what decides which stretch is visible.
  /// Listened to, so a scroll repaints without a rebuild.
  final ScrollController hScroll;

  /// The picked clips by id, the drag in flight, and the pictures fetched
  /// so far. The maps are the panel's own and are read live: the panel bumps
  /// the repaint notifier when one of them gains an answer.
  final Set<String> selected;
  final ValueListenable<CutDrag?> drag;
  final Map<String, ui.Image?> thumbs;
  final Map<String, BridgeAudioPeaks> peaks;

  /// A laid-out name, cached by the panel across every track.
  final TextPainter Function(String name) nameOf;

  final CutLaneColours colours;
  final WaveformStyle style;
  final double radius;

  CutTrackPainter({
    required this.track,
    required this.axis,
    required this.fps,
    required this.hScroll,
    required this.selected,
    required this.drag,
    required this.thumbs,
    required this.peaks,
    required this.nameOf,
    required this.colours,
    required this.style,
    required this.radius,
    required Listenable repaint,
  }) : super(repaint: repaint);

  @override
  void paint(Canvas canvas, Size size) {
    final viewLeft = hScroll.hasClients ? hScroll.offset : 0.0;
    final viewWidth = hScroll.hasClients
        ? hScroll.position.viewportDimension
        : size.width;
    if (track.clips.isEmpty) {
      _plainBar(canvas, size);
      return;
    }
    final held = drag.value;
    // A dragged clip may be entering from outside the window, so the window
    // is widened by the travel.
    final slack = held == null ? 0 : held.shift.abs();
    final from = axis.frameAtExact(viewLeft).floor() - slack;
    final to = axis.frameAtExact(viewLeft + viewWidth).ceil() + slack;
    final (lo, hi) = track.window(from, to);
    for (var i = lo; i < hi; i++) {
      final clip = track.clips[i];
      final (start, end) = cutDraggedSpan(clip, held);
      final (left, width) = cutClipBox(axis, start, end);
      if (left > viewLeft + viewWidth || left + width < viewLeft) continue;
      final box = Rect.fromLTWH(
          left, cutClipInset, width, size.height - cutClipInset * 2);
      _clipBox(canvas, clip, box, held);
    }
    if (!track.sound) return;
    // A crossfade where two sound clips overlap: the join, drawn as an X
    // across the stretch both play.
    for (var i = lo; i < hi - 1; i++) {
      final (_, end) = cutDraggedSpan(track.clips[i], held);
      final (next, _) = cutDraggedSpan(track.clips[i + 1], held);
      if (next >= end) continue;
      final a = axis.xOf(next);
      final b = axis.xOf(end);
      final paint = Paint()
        ..color = colours.cross
        ..strokeWidth = 1;
      canvas.drawLine(Offset(a, cutClipInset),
          Offset(b, size.height - cutClipInset), paint);
      canvas.drawLine(Offset(a, size.height - cutClipInset),
          Offset(b, cutClipInset), paint);
    }
  }

  /// A layer with no clips: one bar from its in point to its out point.
  void _plainBar(Canvas canvas, Size size) {
    final info = track.entry.info;
    final (left, width) =
        cutClipBox(axis, info.inFrame.toInt(), info.outFrame.toInt());
    final box =
        Rect.fromLTWH(left, cutClipInset, width, size.height - cutClipInset * 2);
    _fill(canvas, box, picked: false);
    _name(canvas, info.name, box, from: clipEdgeWidth + 4);
  }

  void _fill(Canvas canvas, Rect box, {required bool picked}) {
    final rrect = RRect.fromRectAndRadius(box, Radius.circular(radius));
    canvas.drawRRect(
        rrect,
        Paint()
          ..color = colours.label.withValues(
              alpha: picked ? clipFillSelectedAlpha : clipFillAlpha));
    canvas.drawRect(
        Rect.fromLTWH(box.left, box.top, clipEdgeWidth, box.height),
        Paint()..color = colours.label);
    canvas.drawRRect(
        rrect,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1
          ..color = picked ? colours.picked : colours.edge);
  }

  void _name(Canvas canvas, String name, Rect box, {required double from}) {
    if (name.isEmpty || box.width - from < 12) return;
    final text = nameOf(name);
    canvas.save();
    canvas.clipRect(box.deflate(1));
    text.paint(canvas,
        Offset(box.left + from, box.top + (box.height - text.height) / 2));
    canvas.restore();
  }

  void _clipBox(Canvas canvas, BridgeClip clip, Rect box, CutDrag? held) {
    final id = clip.id.toString();
    _fill(canvas, box, picked: selected.contains(id));
    if (track.sound) {
      _wave(canvas, clip, box, held);
      _name(canvas, clip.sourceName, box, from: clipEdgeWidth + 4);
      return;
    }
    // The frame the clip opens on, at the head of the box, then the name.
    var from = clipEdgeWidth + 4;
    final image = thumbs[cutThumbKey(clip)];
    if (image != null && image.height > 0) {
      final inner = box.height - 2;
      final wide = inner * image.width / image.height;
      canvas.save();
      canvas.clipRect(box.deflate(1));
      canvas.drawImageRect(
        image,
        Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble()),
        Rect.fromLTWH(box.left + clipEdgeWidth, box.top + 1, wide, inner),
        Paint()..filterQuality = FilterQuality.low,
      );
      canvas.restore();
      from += wide + 2;
    }
    _name(canvas, clip.sourceName, box, from: from);
  }

  /// The clip's own sound across its box. The peaks are bucketed on the
  /// clip's placed clock, which starts at its place start, so a slid clip
  /// carries its wave along with nothing refetched.
  void _wave(Canvas canvas, BridgeClip clip, Rect box, CutDrag? held) {
    final summary = peaks[clip.id.toString()];
    if (summary == null || axis.perFrame <= 0 || fps <= 0) return;
    // A head being trimmed moves the box over a wave that stays put, and a
    // slip moves the wave under a box that stays put.
    final mine = held != null && held.clips.contains(clip.id.toString());
    final trimIn = mine && held.grab == BarGrab.trimIn &&
        held.tool != CutTool.slip;
    final slip = mine && held.tool == CutTool.slip;
    final origin = rationalSeconds(clip.placeStart) +
        (trimIn || slip ? held.shift / fps : 0);
    canvas.save();
    canvas.clipRect(box.deflate(1));
    canvas.translate(box.left, box.top);
    WaveformPainter(
      peaks: summary,
      originSeconds: origin,
      secondsPerPixel: 1 / (axis.perFrame * fps),
      left: 0,
      right: box.width,
      colours: colours.wave,
      style: style,
      height: box.height,
    ).paint(canvas, Size(box.width, box.height));
    canvas.restore();
  }

  @override
  bool shouldRepaint(CutTrackPainter old) =>
      old.track != track ||
      old.axis.frames != axis.frames ||
      old.axis.width != axis.width ||
      old.fps != fps ||
      old.selected != selected ||
      old.colours != colours ||
      old.style != style ||
      old.radius != radius;
}
