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

import 'dart:math' show max, min, pi, pow;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../icons/icons.dart' show iconSize;
import 'audio_timeline_fades_frb.dart' show ClipFadePainter;
import 'clip_fades.dart' show ClipFadeRamp, clipFadeGain;
import 'cut_timeline_tracks.dart';
import 'graph_maths.dart' show rationalSeconds;
import 'timeline_bar_frb.dart' show BarGrab;
import 'timeline_extras_frb.dart'
    show
        TimelineAxis,
        clipEdgeHoverLift,
        clipEdgeWidth,
        clipFillAlpha,
        clipFillSelectedAlpha,
        clipWaveColours;
import 'audio_timeline_clips_frb.dart' show audioClipGainY, audioClipHeader;
import 'waveform_frb.dart';

/// The room a box leaves at the top and foot of its track.
const double cutClipInset = 2;

/// How faint the outline of a clip's whole source is drawn behind its box
/// while an edge is pulled: the Timeline's own ghost, at the same strength.
const double cutReachAlpha = 0.25;

/// The colours a track is drawn in, read off the theme once by the panel so
/// the painter has no context to ask. [label] is the track's own, and
/// [labels] the palette a clip with a colour of its own takes from.
/// [handle] and [handleFill] are the fade handle's outline and inside, the
/// Audio timeline's own.
typedef CutLaneColours = ({
  Color label,
  List<Color> labels,
  Color edge,
  Color picked,
  Color cross,
  Color lift,
  Color rising,
  Color falling,
  Color handle,
  Color handleFill,
  Color drop,
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

  /// The clips on picture tracks that play something with no picture, by
  /// id. They draw as sound clips do.
  final Set<String> soundOnPicture;

  /// The composition glyph, or null until the panel has made it.
  final CutGlyph? compGlyph;

  /// A laid-out name, cached by the panel across every track.
  final TextPainter Function(String name) nameOf;

  /// Whether the fade handles are drawn: with the Select tool, which is the
  /// one that drags them.
  final bool handles;

  /// The clip an effect or a preset in hand is over, by id, which is ringed
  /// so it is plain where the drop will land.
  final ValueListenable<String?> dropOn;

  /// A sound clip's level while its gain line is in hand.
  final ValueListenable<({String clip, double db})?> gain;

  /// The engine's blend mode names, for the chip a clip with a mode of its
  /// own wears.
  final List<String> blendNames;

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
    required this.soundOnPicture,
    required this.compGlyph,
    required this.nameOf,
    required this.handles,
    required this.dropOn,
    required this.gain,
    required this.blendNames,
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
    // Sound crossing between a sound track and a picture track takes the
    // grabbed clip's track alone.
    final moving = held != null &&
        held.trackShift != 0 &&
        held.grab == BarGrab.move &&
        held.tool != CutTool.slide &&
        held.track != null;
    final kinds = {CutTrackKind.picture, CutTrackKind.sound};
    final crossing = moving && tracks[held.track!].kind == track.kind;
    final across = moving &&
        !crossing &&
        index - held.trackShift == held.track &&
        kinds.contains(track.kind) &&
        kinds.contains(tracks[held.track!].kind);
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
        final sound = _isSound(track, clip);
        final (left, width) = _boxOf(clip, span, held, sound: sound);
        if (left > viewLeft + viewWidth || left + width < viewLeft) continue;
        final box = Rect.fromLTWH(
            left, cutClipInset, width, size.height - cutClipInset * 2);
        _clipBox(canvas, clip, box, held, mark, sound: sound);
      }
      // Where two clips overlap the overlap is the transition, a dissolve
      // on a picture track and a crossfade on a sound track: the join,
      // drawn as an X across the stretch both play, each side in the curve
      // it takes. A dissolve shows the outgoing picture at what the incoming
      // one leaves, so both its sides follow the incoming clip's shape.
      final paint = Paint()
        ..color = colours.cross
        ..strokeWidth = 1
        ..style = PaintingStyle.stroke;
      final top = cutClipInset;
      final foot = size.height - cutClipInset;
      for (var i = lo; i < hi - 1; i++) {
        final (_, end) = spans[i - lo] ?? (0, 0);
        final (next, _) = spans[i - lo + 1] ?? (0, 0);
        if (next >= end) continue;
        final a = axis.xOf(next);
        final b = axis.xOf(end);
        final out = track.clips[i].fadeOut.shape;
        final into = track.clips[i + 1].fadeIn.shape;
        final falling = Path()..moveTo(a, top);
        final rising = Path()..moveTo(a, foot);
        const steps = 16;
        for (var k = 1; k <= steps; k++) {
          final u = k / steps;
          final x = a + (b - a) * u;
          final gain = clipFadeGain(into, u);
          final left = track.sound ? clipFadeGain(out, 1 - u) : 1 - gain;
          falling.lineTo(x, foot - left * (foot - top));
          rising.lineTo(x, foot - gain * (foot - top));
        }
        canvas.drawPath(falling, paint);
        canvas.drawPath(rising, paint);
      }
      _fades(canvas, size, lo, hi, spans, held, mark);
    }
    if (!crossing && !across) return;
    final source = index - held.trackShift;
    if (source < 0 || source >= tracks.length) return;
    final other = tracks[source];
    final (lo, hi) = other.window(from, to);
    for (var i = lo; i < hi; i++) {
      final clip = other.clips[i];
      if (!held.clips.contains(clip.id.toString())) continue;
      final sound = _isSound(other, clip);
      final (left, width) =
          _boxOf(clip, cutDraggedSpan(clip, held), held, sound: sound);
      if (left > viewLeft + viewWidth || left + width < viewLeft) continue;
      final box = Rect.fromLTWH(
          left, cutClipInset, width, size.height - cutClipInset * 2);
      _clipBox(canvas, clip, box, held, null, sound: sound);
    }
  }

  /// Whether [clip] on [on] draws as sound: every clip on a sound track,
  /// and one that plays no picture on a picture track.
  bool _isSound(CutTrack on, BridgeClip clip) =>
      on.sound || soundOnPicture.contains(clip.id.toString());

  /// A clip's box for its dragged [span]. A sound clip may sit between two
  /// frames, so its box is drawn where it is to the part of a frame, and a
  /// move with Frames off carries the part of a frame the pointer has gone.
  (double, double) _boxOf(BridgeClip clip, (int, int) span, CutDrag? held,
      {required bool sound}) {
    if (!sound) return cutClipBox(axis, span.$1, span.$2);
    var start = span.$1 + (clip.startExact - clip.startFrame.toInt());
    var end = span.$2 + (clip.endExact - clip.endFrame.toInt());
    if (held != null &&
        held.sub != 0 &&
        held.grab == BarGrab.move &&
        held.clips.contains(clip.id.toString())) {
      start += held.sub;
      end += held.sub;
    }
    return cutClipBox(axis, start, end);
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
    // Lit, it grows about the same centre and fills, so the pointer can
    // tell it has the handle before it presses.
    const base = 5.0;
    final mark = lit ? 7.0 : base;
    canvas.save();
    canvas.translate(x, 3 + base / 2);
    canvas.rotate(pi / 4);
    final square = Rect.fromCenter(center: Offset.zero, width: mark, height: mark);
    canvas.drawRect(
        square, Paint()..color = lit ? colours.lift : colours.handleFill);
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
    _fill(canvas, box, colours.label, picked: false);
    // A layer's own sound row: its wave under its name. The peaks are on the
    // layer's clock, which starts where its source does.
    final own = track.kind == CutTrackKind.layerSound;
    final summary = own ? peaks[track.id] : null;
    if (summary != null && axis.perFrame > 0 && fps > 0) {
      _waveAt(
          canvas,
          summary,
          rationalSeconds(info.span.inPoint) -
              rationalSeconds(info.span.startOffset),
          box,
          colours.label);
    }
    final header = own ? _soundHeader(box) : 0.0;
    _name(canvas, info.name, box,
        from: clipEdgeWidth + 4, band: header > 0 ? header : null);
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
  void _hoverEdge(Canvas canvas, Rect box, CutHover mark, Color label) {
    final left =
        mark.grab == BarGrab.trimIn ? box.left : box.right - clipEdgeWidth;
    canvas.drawRect(
      Rect.fromLTWH(left, box.top, clipEdgeWidth, box.height),
      Paint()
        ..color = Color.lerp(label, colours.lift, clipEdgeHoverLift)!,
    );
  }

  void _fill(Canvas canvas, Rect box, Color label, {required bool picked}) {
    final rrect = RRect.fromRectAndRadius(box, Radius.circular(radius));
    canvas.drawRRect(
        rrect,
        Paint()
          ..color = label.withValues(
              alpha: picked ? clipFillSelectedAlpha : clipFillAlpha));
    canvas.drawRect(
        Rect.fromLTWH(box.left, box.top, clipEdgeWidth, box.height),
        Paint()..color = label);
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
      {required double from,
      double? band,
      double room = 0,
      bool ground = false}) {
    if (name.isEmpty || box.width - from - room < 12) return;
    final text = nameOf(name);
    canvas.save();
    canvas.clipRect(Rect.fromLTRB(
        box.left + 1, box.top + 1, box.right - 1 - room, box.bottom - 1));
    final at = Offset(
        box.left + from, box.top + ((band ?? box.height) - text.height) / 2);
    // A sound clip on a track too short for a header has its name over its
    // wave, so the name stands on a ground of its own.
    if (ground) {
      canvas.drawRRect(
          RRect.fromRectAndRadius(
              Rect.fromLTWH(at.dx - 3, at.dy, text.width + 6, text.height),
              Radius.circular(radius)),
          Paint()..color = colours.handleFill.withValues(alpha: 0.75));
    }
    text.paint(canvas, at);
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
    // The clip's own colour where it has one, its track's where not.
    final label = switch (clip.label) {
      final own? => colours.labels[own % colours.labels.length],
      null => colours.label,
    };
    if (held != null) _reach(canvas, clip, box, held);
    _fill(canvas, box, label, picked: selected.contains(id));
    if (mark != null && mark.fade == null && mark.clips.contains(id)) {
      _hoverEdge(canvas, box, mark, label);
    }
    if (dropOn.value == id) {
      canvas.drawRRect(
          RRect.fromRectAndRadius(box.deflate(1), Radius.circular(radius)),
          Paint()
            ..style = PaintingStyle.stroke
            ..strokeWidth = 2
            ..color = colours.drop);
    }
    final badges = cutBadges(box, picture: !sound);
    if (sound) {
      // The wave has the whole box, as Vegas draws it, and the name stands
      // over it on a ground of its own.
      final header = min(_soundHeader(box), audioClipHeader);
      _wave(canvas, clip, box, held, label, header: false);
      _gainLine(canvas, clip, box);
      final chips = _chips(canvas, clip, box,
          band: header > 0 ? header : null, badges: badges);
      _name(canvas, clip.sourceName, box,
          band: header > 0 ? header : null,
          ground: true,
          room: chips,
          from: _glyph(canvas, clip, box,
              from: clipEdgeWidth + 4, band: header > 0 ? header : null));
      _badges(canvas, clip, badges);
      return;
    }
    // A picture clip that is heard has its sound in a band along the foot
    // of the box, and its picture and name above it.
    final band = _bandOf(clip, box);
    if (band > 0) {
      _wave(
          canvas,
          clip,
          Rect.fromLTRB(box.left, box.bottom - band, box.right, box.bottom),
          held,
          label,
          header: false);
    }
    final above =
        Rect.fromLTRB(box.left, box.top, box.right, box.bottom - band);
    // The frame the clip opens on, at the head of the box, then the frames
    // along it as Vegas draws them, and the name over them at the top.
    var from = clipEdgeWidth + 4;
    var strip = false;
    final image = thumbs[cutThumbKey(clip)];
    if (image != null && image.height > 0) {
      final inner = above.height - 2;
      // The whole frame on a track of the usual height. On a tall one it
      // would fill the box, so it keeps to the head of it and shows the
      // middle of the frame, and the name still has its room.
      final whole = inner * image.width / image.height;
      final wide = min(whole, box.width * 0.6);
      final shown = image.width * wide / whole;
      canvas.save();
      canvas.clipRect(above.deflate(1));
      canvas.drawImageRect(
        image,
        Rect.fromLTWH((image.width - shown) / 2, 0, shown,
            image.height.toDouble()),
        Rect.fromLTWH(box.left + clipEdgeWidth, box.top + 1, wide, inner),
        Paint()..filterQuality = FilterQuality.low,
      );
      // Each later frame is the nearest one fetched, or the one before it
      // until it arrives, so the strip never shows a hole.
      var last = image;
      final step = cutFilmFrames(axis.perFrame) * axis.perFrame;
      final first = box.left + clipEdgeWidth + wide;
      strip = first < box.right - 1;
      // Only the frames on screen are drawn.
      final view = hScroll.hasClients ? hScroll.offset : box.left;
      final viewEnd = hScroll.hasClients
          ? view + hScroll.position.viewportDimension
          : box.right;
      final end = min(box.right - 1, viewEnd);
      for (var x = first + max(0, ((view - first) / wide).floor()) * wide;
          x < end;
          x += wide) {
        final k = step > 0 ? ((x - box.left) / step).round() : 0;
        final frame = k < 1
            ? null
            : thumbs[cutFilmKey(clip, cutFilmOffset(k, axis.perFrame))];
        if (frame != null && frame.height > 0) last = frame;
        canvas.drawImageRect(
          last,
          Rect.fromLTWH((last.width - shown) / 2, 0, shown,
              last.height.toDouble()),
          Rect.fromLTWH(x, box.top + 1, wide, inner),
          Paint()..filterQuality = FilterQuality.low,
        );
      }
      canvas.restore();
      if (!strip) from += wide + 2;
    }
    final nameBand =
        badges.isEmpty && !strip ? above.height : min(above.height, 20.0);
    final chips = _chips(canvas, clip, box, band: nameBand, badges: badges);
    _name(canvas, clip.sourceName, box,
        band: nameBand,
        room: chips,
        ground: strip,
        from: _glyph(canvas, clip, box, from: from, band: nameBand));
    _badges(canvas, clip, badges);
    _opacityLine(canvas, clip, box);
  }

  /// A picture clip's opacity as a line across it, once it is below full or
  /// while it is dragged: at the top for full, at the foot for none. At full
  /// it is the box's own top edge, where the pointer still finds it.
  void _opacityLine(Canvas canvas, BridgeClip clip, Rect box) {
    final live = gain.value;
    final mine = live != null && live.clip == clip.id.toString();
    final opacity = mine ? live.db : clip.opacity;
    if (!mine && opacity >= 1) return;
    final y = box.top -
        cutClipInset +
        cutOpacityY(opacity, box.height + cutClipInset * 2);
    canvas.drawLine(
        Offset(box.left + clipEdgeWidth, y),
        Offset(box.right - 1, y),
        Paint()
          ..strokeWidth = 1.4
          ..color = colours.rising);
  }

  /// A sound clip's level, as a line across it: at the top for 0 dB, lower
  /// as it is turned down, the Audio timeline's own line.
  void _gainLine(Canvas canvas, BridgeClip clip, Rect box) {
    final live = gain.value;
    final db = live != null && live.clip == clip.id.toString()
        ? live.db
        : clip.gainDb;
    final y = box.top -
        cutClipInset +
        audioClipGainY(db, box.height + cutClipInset * 2);
    canvas.drawLine(
        Offset(box.left + clipEdgeWidth, y),
        Offset(box.right - 1, y),
        Paint()
          ..strokeWidth = 1.4
          ..color = colours.rising);
  }

  /// The clip's buttons: fx, lit while it has effects that are on, and on a
  /// picture clip Pan/Crop, drawn as a frame's two corners.
  void _badges(Canvas canvas, BridgeClip clip, List<(CutBadge, Rect)> badges) {
    for (final (kind, at) in badges) {
      canvas.drawRRect(RRect.fromRectAndRadius(at, Radius.circular(radius)),
          Paint()..color = colours.handleFill.withValues(alpha: 0.85));
      switch (kind) {
        case CutBadge.fx:
          final text = nameOf('fx');
          final lit = clip.effects.isNotEmpty && clip.fx;
          if (!lit) {
            canvas.saveLayer(
                at, Paint()..color = colours.edge.withValues(alpha: 0.5));
          }
          text.paint(
              canvas,
              Offset(at.center.dx - text.width / 2,
                  at.center.dy - text.height / 2));
          if (!lit) canvas.restore();
        case CutBadge.panCrop:
          final r = at.deflate(at.width * 0.25);
          final arm = r.width * 0.45;
          canvas.drawPath(
              Path()
                ..moveTo(r.left, r.top + arm)
                ..lineTo(r.left, r.top)
                ..lineTo(r.left + arm, r.top)
                ..moveTo(r.right, r.bottom - arm)
                ..lineTo(r.right, r.bottom)
                ..lineTo(r.right - arm, r.bottom),
              Paint()
                ..style = PaintingStyle.stroke
                ..strokeWidth = 1.2
                ..color = colours.handle);
      }
    }
  }

  /// How tall the band a picture clip's sound is drawn in stands: none for a
  /// clip that is not heard, or has no sound, or on a track too short.
  double _bandOf(BridgeClip clip, Rect box) {
    if (!track.entry.info.switches.audible || !clip.audible || clip.retimed) {
      return 0;
    }
    final summary = peaks[clip.id.toString()];
    if (summary == null || summary.buckets == 0) return 0;
    if (box.height < audioClipHeader * 2) return 0;
    return (box.height * 0.4).roundToDouble();
  }

  /// What a clip carries of its own, said at the right of its name line: its
  /// blend mode where it has one, and "fx" where it has effects, faint while
  /// they are bypassed. A sound clip that sits between two frames says how
  /// far past the frame it starts, since at most zooms that is under a pixel
  /// and nothing else shows it. Answers the width they took, which the name
  /// gives up.
  double _chips(Canvas canvas, BridgeClip clip, Rect box,
      {double? band, List<(CutBadge, Rect)> badges = const []}) {
    final past = clip.startExact - clip.startFrame.toInt();
    final words = <(TextPainter, bool)>[
      if (past.abs() >= 0.005)
        (nameOf('${past > 0 ? '+' : ''}${past.toStringAsFixed(2)}'), true),
      if (clip.blend case final blend? when blend < blendNames.length)
        (nameOf(blendNames[blend]), true),
      // Said by the fx button where the clip has room for one.
      if (clip.effects.isNotEmpty && badges.isEmpty) (nameOf('fx'), clip.fx),
    ];
    const pad = 4.0;
    const gap = 3.0;
    final start = badges.isEmpty ? box.right - pad : badges.last.$2.left - gap;
    if (words.isEmpty) return box.right - start;
    final tall = band ?? box.height;
    var right = start;
    canvas.save();
    canvas.clipRect(box.deflate(1));
    for (final (text, lit) in words.reversed) {
      final wide = text.width + pad * 2;
      // Over the thumbnail where the box is short, since what a clip
      // carries matters more than the far side of its picture. None on a
      // box too short even for that.
      if (right - wide < box.left + clipEdgeWidth + 4) break;
      final high = min(text.height + 2, tall - 2);
      final chip = Rect.fromLTWH(
          right - wide, box.top + (tall - high) / 2, wide, high);
      canvas.drawRRect(
          RRect.fromRectAndRadius(chip, Radius.circular(radius)),
          Paint()..color = colours.handleFill.withValues(alpha: lit ? 0.9 : 0.5));
      if (!lit) {
        canvas.saveLayer(chip, Paint()..color = colours.edge.withValues(alpha: 0.45));
      }
      text.paint(canvas,
          Offset(chip.left + pad, chip.top + (high - text.height) / 2));
      if (!lit) canvas.restore();
      right = chip.left - gap;
    }
    canvas.restore();
    return box.right - right;
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
  void _wave(
      Canvas canvas, BridgeClip clip, Rect box, CutDrag? held, Color label,
      {bool header = true}) {
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
    // Drawn at the clip's own level, as Vegas draws it, so a clip turned
    // down looks quieter. The line in hand counts before it is let go.
    final live = gain.value;
    final db = live != null && live.clip == clip.id.toString()
        ? live.db
        : clip.gainDb;
    _waveAt(canvas, summary, origin, box, label,
        header: header, level: db >= 0 ? 1.0 : pow(10, db / 20).toDouble());
  }

  /// A wave across [box], whose left edge is [origin] on the peaks' clock,
  /// in a bright shade of [label].
  void _waveAt(Canvas canvas, BridgeAudioPeaks summary, double origin, Rect box,
      Color label,
      {bool header = true, double level = 1}) {
    // Under the name's header, so the wave never runs through the name.
    final top = header ? _soundHeader(box) : 0.0;
    final room = box.height - top;
    final tall = max(2.0, room * level);
    canvas.save();
    canvas.clipRect(box.deflate(1));
    // Shrunk about its middle, or onto the foot where waves stand on it.
    final drop = style.fromBottom ? room - tall : (room - tall) / 2;
    canvas.translate(box.left, box.top + top + drop);
    WaveformPainter(
      peaks: summary,
      originSeconds: origin,
      secondsPerPixel: 1 / (axis.perFrame * fps),
      left: 0,
      right: box.width,
      colours: clipWaveColours(label, colours.lift),
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
      old.soundOnPicture != soundOnPicture ||
      old.compGlyph != compGlyph ||
      old.handles != handles ||
      old.blendNames != blendNames ||
      old.colours != colours ||
      old.style != style ||
      old.radius != radius;
}
