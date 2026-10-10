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

import 'dart:math' show min, pi;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../icons/icons.dart' show iconSize;
import '../theme/theme.dart';
import 'audio_timeline_fades_frb.dart' show ClipFadePainter;
import 'clip_fades.dart' show ClipFadeRamp;
import 'cut_timeline_tracks.dart';
import 'graph_maths.dart' show rationalSeconds;
import 'timeline_bar_frb.dart' show BarGrab;
import 'timeline_extras_frb.dart'
    show
        TimelineAxis,
        clipEdgeHoverLift,
        clipEdgeWidth,
        clipFillAlpha,
        clipFillSelectedAlpha;
import 'audio_timeline_clips_frb.dart' show audioClipHeader;
import 'waveform_frb.dart';

/// The room a box leaves at the top and foot of its track.
const double cutClipInset = 2;

/// How faint the outline of a clip's whole source is drawn behind its box
/// while an edge is pulled: the Timeline's own ghost, at the same strength.
const double cutReachAlpha = 0.25;

/// The colours a track is drawn in, read off the theme once by the panel so
/// the painter has no context to ask. [handle] and [handleFill] are the fade
/// handle's outline and inside, the Audio timeline's own.
typedef CutLaneColours = ({
  Color label,
  Color edge,
  Color picked,
  Color cross,
  Color lift,
  Color rising,
  Color falling,
  Color handle,
  Color handleFill,
  WaveformColours wave,
});

/// The composition glyph as a picture the painters can draw, made once by
/// the panel in the name colour, for the clips that play a composition.
typedef CutGlyph = ({ui.Picture picture, Size size});

/// The edges the pointer is over: the hovered clip's own and, while Linked is
/// on, the same edge of the clips linked to it, so it is plain both will
/// move. Over a fade handle [fade] says which end's, the start's when true,
/// and the handle is marked rather than the edge.
typedef CutHover = ({Set<String> clips, BarGrab grab, bool? fade});

class CutTrackPainter extends CustomPainter {
  final CutTrack track;

  /// This track's place in [tracks], which a move across tracks reads to
  /// draw the travelling clips on the track they will land on.
  final int index;
  final List<CutTrack> tracks;
  final TimelineAxis axis;
  final double fps;

  /// The lanes' horizontal scroll: what decides which stretch is visible.
  /// Listened to, so a scroll repaints without a rebuild.
  final ScrollController hScroll;

  /// The picked clips by id, the drag in flight, the edges under the pointer,
  /// and the pictures fetched so far. The maps are the panel's own and are
  /// read live: the panel bumps the repaint notifier when one of them gains
  /// an answer.
  final Set<String> selected;
  final ValueListenable<CutDrag?> drag;
  final ValueListenable<CutHover?> hover;
  final Map<String, ui.Image?> thumbs;
  final Map<String, BridgeAudioPeaks> peaks;

  /// The composition glyph, or null until the panel has made it.
  final CutGlyph? compGlyph;

  /// A laid-out name, cached by the panel across every track.
  final TextPainter Function(String name) nameOf;

  /// Whether the fade handles are drawn: with the Select tool, which is the
  /// one that drags them.
  final bool handles;

  final CutLaneColours colours;
  final WaveformStyle style;
  final double radius;

  CutTrackPainter({
    required this.track,
    required this.index,
    required this.tracks,
    required this.axis,
    required this.fps,
    required this.hScroll,
    required this.selected,
    required this.drag,
    required this.hover,
    required this.thumbs,
    required this.peaks,
    required this.compGlyph,
    required this.nameOf,
    required this.handles,
    required this.colours,
    required this.style,
    required this.radius,
    required Listenable repaint,
  }) : super(repaint: repaint);

  @override
  void paint(Canvas canvas, Size size) {
    // Held to the lane: a trimmed clip's source outline can reach back past
    // frame zero, and unclipped it drew over the track names.
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    _paintLane(canvas, size);
    canvas.restore();
  }

  void _paintLane(Canvas canvas, Size size) {
    final viewLeft = hScroll.hasClients ? hScroll.offset : 0.0;
    final viewWidth = hScroll.hasClients
        ? hScroll.position.viewportDimension
        : size.width;
    final held = drag.value;
    final locked = track.entry.info.switches.locked;
    // A dragged clip may be entering from outside the window, so the window
    // is widened by the travel.
    final slack = held == null ? 0 : held.shift.abs();
    final from = axis.frameAtExact(viewLeft).floor() - slack;
    final to = axis.frameAtExact(viewLeft + viewWidth).ceil() + slack;
    // A move across tracks: the travelling clips of the grabbed clip's kind
    // leave their own track and are drawn on the one they will land on.
    final crossing = held != null &&
        held.trackShift != 0 &&
        held.grab == BarGrab.move &&
        held.tool != CutTool.slide &&
        held.track != null &&
        tracks[held.track!].kind == track.kind;
    final mark = hover.value;
    if (track.clips.isEmpty) {
      _plainBar(canvas, size, held, locked);
    } else {
      final (lo, hi) = track.window(from, to);
      // Each visible clip's span with the drag in it, read again by the
      // joins and the fades below; null for a clip crossing to another
      // track, which has left this one.
      final spans = <(int, int)?>[];
      for (var i = lo; i < hi; i++) {
        final clip = track.clips[i];
        if (crossing && held.clips.contains(clip.id.toString())) {
          spans.add(null);
          continue;
        }
        final span = cutDraggedSpan(clip, held, locked: locked);
        spans.add(span);
        final (start, end) = span;
        final (left, width) = cutClipBox(axis, start, end);
        if (left > viewLeft + viewWidth || left + width < viewLeft) continue;
        final box = Rect.fromLTWH(
            left, cutClipInset, width, size.height - cutClipInset * 2);
        _clipBox(canvas, clip, box, held, mark, sound: track.sound);
      }
      // Where two clips overlap the overlap is the transition, a dissolve
      // on a picture track and a crossfade on a sound track: the join,
      // drawn as an X across the stretch both play.
      for (var i = lo; i < hi - 1; i++) {
        final (_, end) = spans[i - lo] ?? (0, 0);
        final (next, _) = spans[i - lo + 1] ?? (0, 0);
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
      _fades(canvas, size, lo, hi, spans, held, mark);
    }
    if (!crossing) return;
    final source = index - held.trackShift;
    if (source < 0 || source >= tracks.length) return;
    final other = tracks[source];
    final (lo, hi) = other.window(from, to);
    for (var i = lo; i < hi; i++) {
      final clip = other.clips[i];
      if (!held.clips.contains(clip.id.toString())) continue;
      final (start, end) = cutDraggedSpan(clip, held);
      final (left, width) = cutClipBox(axis, start, end);
      if (left > viewLeft + viewWidth || left + width < viewLeft) continue;
      final box = Rect.fromLTWH(
          left, cutClipInset, width, size.height - cutClipInset * 2);
      _clipBox(canvas, clip, box, held, null, sound: other.sound);
    }
  }

  /// The fades at the ends nothing overlaps, drawn as the Audio timeline
  /// draws a fade: the ramp the clip follows to transparent or to silence,
  /// rising into the clip and falling out of it, with the quiet room under
  /// it filled. An end inside an overlap is the transition and has its X.
  /// Over the visible clips alone, with the drag carried into their spans,
  /// and a fade handle's travel into its ramp. Then the handles, one at each
  /// end nothing meets, standing on the ramp's inner end.
  void _fades(Canvas canvas, Size size, int lo, int hi,
      List<(int, int)?> spans, CutDrag? held, CutHover? mark) {
    if (fps <= 0) return;
    final ramps = <ClipFadeRamp>[];
    final corners = <(double, bool)>[];
    for (var i = lo; i < hi; i++) {
      final clip = track.clips[i];
      final span = spans[i - lo];
      if (span == null) continue;
      final (start, end) = span;
      final id = clip.id.toString();
      final mine = held != null && held.clips.contains(id);
      // The neighbours either side, by their dragged spans: the one before
      // overlaps the start when it ends after it, the one after overlaps
      // the end when it starts before it; a neighbour meeting an end makes
      // that end a dissolve's, with no fade handle.
      final before = i > lo ? spans[i - lo - 1]?.$2 : null;
      final after = i + 1 < hi ? spans[i - lo + 1]?.$1 : null;
      final overBefore = before != null && before > start;
      final overAfter = after != null && after < end;
      final into = cutFadeFrames(clip, fps,
          into: true, shift: mine && held.fade == true ? held.shift : 0);
      if (!overBefore && into > 0) {
        ramps.add((
          clip: id,
          into: true,
          from: start.toDouble(),
          to: (start + into).toDouble(),
          shape: clip.fadeIn.shape,
        ));
      }
      final out = cutFadeFrames(clip, fps,
          into: false, shift: mine && held.fade == false ? held.shift : 0);
      if (!overAfter && out > 0) {
        ramps.add((
          clip: id,
          into: false,
          from: (end - out).toDouble(),
          to: end.toDouble(),
          shape: clip.fadeOut.shape,
        ));
      }
      if (!handles || axis.xOf(end) - axis.xOf(start) < cutFadeHandleMinWidth) {
        continue;
      }
      final lit = mark != null && mark.clips.contains(id);
      if (before == null || before < start) {
        corners.add((axis.xOf(start + into), lit && mark.fade == true));
      }
      if (after == null || after > end) {
        corners.add((axis.xOf(end - out), lit && mark.fade == false));
      }
    }
    if (ramps.isNotEmpty) {
      // No gain lines here, so every ramp tops out at the box's own top.
      ClipFadePainter(
        ramps: ramps,
        gains: const {},
        axis: axis,
        rising: colours.rising,
        falling: colours.falling,
      ).paint(canvas, size);
    }
    for (final (x, lit) in corners) {
      _handle(canvas, x, lit: lit);
    }
  }

  /// A fade handle's mark, the Audio timeline's: a small square stood on its
  /// corner, three pixels down from the handle's top. Hovered, its outline
  /// is lifted to the text colour, as a hovered edge is lifted.
  void _handle(Canvas canvas, double x, {required bool lit}) {
    const mark = 5.0;
    canvas.save();
    canvas.translate(x, 3 + mark / 2);
    canvas.rotate(pi / 4);
    final square = Rect.fromCenter(center: Offset.zero, width: mark, height: mark);
    canvas.drawRect(square, Paint()..color = colours.handleFill);
    canvas.drawRect(
      square.deflate(0.5),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = lit ? colours.lift : colours.handle,
    );
    canvas.restore();
  }

  /// A layer with no clips: one bar from its in point to its out point. A
  /// ripple carries it along when it starts at or after the rippled frame,
  /// as the engine carries a layer of any other kind.
  void _plainBar(Canvas canvas, Size size, CutDrag? held, bool locked) {
    final info = track.entry.info;
    var start = info.inFrame.toInt();
    var end = info.outFrame.toInt();
    final ripple = locked ? 0 : cutRippleOf(held);
    if (ripple != 0 && start >= held!.rippleFrom!) {
      start += ripple;
      end += ripple;
    }
    final (left, width) = cutClipBox(axis, start, end);
    final box =
        Rect.fromLTWH(left, cutClipInset, width, size.height - cutClipInset * 2);
    _fill(canvas, box, picked: false);
    _name(canvas, info.name, box, from: clipEdgeWidth + 4);
  }

  /// The outline of the clip's whole source behind its box while an edge is
  /// pulled or the frames slipped: a hairline and nothing inside it, the
  /// Timeline's own ghost, so what shows past each end is what is left to
  /// pull out.
  void _reach(Canvas canvas, BridgeClip clip, Rect box, CutDrag held) {
    final reach = cutReachSpan(clip, held);
    if (reach == null) return;
    final (left, width) = cutClipBox(axis, reach.$1, reach.$2);
    canvas.drawRRect(
      RRect.fromRectAndRadius(
          Rect.fromLTWH(left, box.top, width, box.height),
          Radius.circular(radius)),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = colours.label.withValues(alpha: cutReachAlpha),
    );
  }

  /// The edge under the pointer, and the same edge of the clips linked to
  /// it: the leading edge lifted a step toward the text colour, as a hovered
  /// bar's is in the Timeline, and the trailing edge given the same strip.
  void _hoverEdge(Canvas canvas, Rect box, CutHover mark) {
    final left =
        mark.grab == BarGrab.trimIn ? box.left : box.right - clipEdgeWidth;
    canvas.drawRect(
      Rect.fromLTWH(left, box.top, clipEdgeWidth, box.height),
      Paint()
        ..color = Color.lerp(colours.label, colours.lift, clipEdgeHoverLift)!,
    );
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

  /// [band] is how much of the box's height the name line has, from its top:
  /// all of it for a picture clip, and the header a sound clip keeps above its
  /// wave.
  void _name(Canvas canvas, String name, Rect box,
      {required double from, double? band}) {
    if (name.isEmpty || box.width - from < 12) return;
    final text = nameOf(name);
    canvas.save();
    canvas.clipRect(box.deflate(1));
    text.paint(
        canvas,
        Offset(box.left + from,
            box.top + ((band ?? box.height) - text.height) / 2));
    canvas.restore();
  }

  /// The header a sound clip keeps for its name above its wave, the Audio
  /// timeline's own, or none on a row too short to hold both.
  double _soundHeader(Rect box) =>
      box.height >= audioClipHeader * 2 ? audioClipHeader : 0;

  void _clipBox(Canvas canvas, BridgeClip clip, Rect box, CutDrag? held,
      CutHover? mark,
      {required bool sound}) {
    final id = clip.id.toString();
    if (held != null) _reach(canvas, clip, box, held);
    _fill(canvas, box, picked: selected.contains(id));
    if (mark != null && mark.fade == null && mark.clips.contains(id)) {
      _hoverEdge(canvas, box, mark);
    }
    if (sound) {
      final header = _soundHeader(box);
      _wave(canvas, clip, box, held);
      _name(canvas, clip.sourceName, box,
          band: header > 0 ? header : null,
          from: _glyph(canvas, clip, box,
              from: clipEdgeWidth + 4, band: header > 0 ? header : null));
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
    _name(canvas, clip.sourceName, box,
        from: _glyph(canvas, clip, box, from: from));
  }

  /// The composition glyph the Project panel gives a composition, at the
  /// head of the name line of a clip that plays one. Answers where the name
  /// then starts.
  double _glyph(Canvas canvas, BridgeClip clip, Rect box,
      {required double from, double? band}) {
    final glyph = compGlyph;
    if (!clip.sourceIsComp || glyph == null || glyph.size.isEmpty) return from;
    final tall = band ?? box.height;
    final side = min(iconSize, tall - 4);
    if (box.width - from < side + 12) return from;
    canvas.save();
    canvas.clipRect(box.deflate(1));
    canvas.translate(box.left + from, box.top + (tall - side) / 2);
    canvas.scale(side / glyph.size.width, side / glyph.size.height);
    canvas.drawPicture(glyph.picture);
    canvas.restore();
    return from + side + 3;
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
    final trimIn = mine &&
        held.grab == BarGrab.trimIn &&
        held.tool != CutTool.slip &&
        held.fade == null;
    final slip = mine && held.tool == CutTool.slip;
    final origin = rationalSeconds(clip.placeStart) +
        (trimIn || slip ? held.shift / fps : 0);
    // Under the name's header, so the wave never runs through the name.
    final header = _soundHeader(box);
    final tall = box.height - header;
    canvas.save();
    canvas.clipRect(box.deflate(1));
    canvas.translate(box.left, box.top + header);
    WaveformPainter(
      peaks: summary,
      originSeconds: origin,
      secondsPerPixel: 1 / (axis.perFrame * fps),
      left: 0,
      right: box.width,
      colours: colours.wave,
      style: style,
      height: tall,
    ).paint(canvas, Size(box.width, tall));
    canvas.restore();
  }

  @override
  bool shouldRepaint(CutTrackPainter old) =>
      old.track != track ||
      old.index != index ||
      old.tracks != tracks ||
      old.axis.frames != axis.frames ||
      old.axis.width != axis.width ||
      old.fps != fps ||
      old.selected != selected ||
      old.compGlyph != compGlyph ||
      old.handles != handles ||
      old.colours != colours ||
      old.style != style ||
      old.radius != radius;
}
