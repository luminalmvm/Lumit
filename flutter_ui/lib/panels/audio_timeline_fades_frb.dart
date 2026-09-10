// The fades on an Audio timeline track (docs/impl/audio-timeline.md §3 and §5).
//
// A clip fades in from its start and out to its end, and where two clips
// overlap the overlap is the crossfade: the outgoing one's ramp falls across it
// while the incoming one's rises. This draws both, from the curves the mixer
// plays, and puts a handle at each top corner that drags one.
//
// A corner inside an overlap drags the clip's **edge** instead, because there
// the overlap is the fade and its length is what the edge says; that goes back
// to the panel as the same trim the clip's own edge commits.
//
// A ramp rises to its clip's own gain line rather than to the top of the box,
// so a faded clip reads as loud as it is, and the corner handle stands on that
// line as well.

import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import 'audio_timeline_clips_frb.dart';
import 'audio_timeline_rows_frb.dart';
import 'clip_fades.dart';
import 'timeline_bar_frb.dart' show BarGrab;
import 'timeline_extras_frb.dart';
import 'timeline_snap.dart';

/// How big a corner handle's target is, and the mark drawn inside it.
const double _handleWidth = 12;
const double _handleHeight = 14;
const double _handleMark = 5;

/// How strongly the room under a ramp is filled. Light: it says which part of
/// the clip is quiet, and the wave under it has to stay readable.
const double clipFadeFillAlpha = 0.14;

/// A corner drag, let go: [clip] is null on an unconverted Audio layer, whose
/// bar is its one clip before the conversion is made, and the seconds are the
/// new length of the fade at that end - zero takes it away.
typedef AudioClipFade = ({BridgeClip? clip, bool into, double seconds});

/// The ramps with a clip drag in flight in them: the dragged clip's own ramps
/// moved by [shift] frames, and every other clip's left where they are.
///
/// A move carries both of the clip's ends, a head trim carries the rising ramp
/// and a tail trim the falling one, which is the rule the box's own edges
/// follow. Pure, so it is tested without a widget tree.
List<ClipFadeRamp> shiftRamps(
    List<ClipFadeRamp> ramps, String clip, BarGrab grab, int shift) {
  if (shift == 0) return ramps;
  bool carried(ClipFadeRamp ramp) =>
      ramp.clip == clip &&
      switch (grab) {
        BarGrab.move => true,
        BarGrab.trimIn => ramp.into,
        BarGrab.trimOut => !ramp.into,
      };
  return [
    for (final ramp in ramps)
      if (!carried(ramp))
        ramp
      else
        (
          clip: ramp.clip,
          into: ramp.into,
          from: ramp.from + shift,
          to: ramp.to + shift,
          shape: ramp.shape,
        ),
  ];
}

/// A corner drag in flight: what it has hold of, whether it is dragging the
/// clip's edge instead (which is what a corner does inside an overlap), and
/// where the pointer started and stands now.
typedef _FadeGrab = ({
  BridgeClip? clip,
  bool into,
  bool trim,
  Offset from,
  Offset at,
});

/// The fades on a track: the ramps drawn on the clips, and the handle at each
/// top corner that drags one (docs/impl/audio-timeline.md §5).
///
/// It stands **over** the clip strip, so a corner is reachable on a box that
/// carries its own pointer; neither claims a pixel it has drawn nothing on.
class AudioFadeLayer extends StatefulWidget {
  final AudioTrackRow track;
  final TimelineAxis axis;
  final double fps;

  /// The track's lane in pixels, which is where the gain line each ramp tops
  /// out at is measured.
  final double height;

  final List<SnapTarget> snapTargets;

  /// Where the clip strip's box stands while a drag is on it. This layer is a
  /// sibling of the strip, so without it the ramps and the handles hold the
  /// committed frames while the box moves out from under them
  /// (docs/impl/audio-timeline.md §5).
  final ValueNotifier<AudioClipDrag?> preview;

  /// The razor is armed, so the handles stand down and the whole clip takes the
  /// blade - a corner is a place to cut like any other.
  final bool razor;

  /// A corner let go on a fade: the panel writes it.
  final void Function(AudioClipFade fade) onFade;

  /// A corner let go inside an overlap, which drags the clip's edge: the same
  /// trim the clip's own edge commits.
  final void Function(AudioClipEdit edit) onCommit;

  const AudioFadeLayer({
    super.key,
    required this.track,
    required this.axis,
    required this.fps,
    required this.height,
    required this.snapTargets,
    required this.preview,
    required this.razor,
    required this.onFade,
    required this.onCommit,
  });

  @override
  State<AudioFadeLayer> createState() => _AudioFadeLayerState();
}

class _AudioFadeLayerState extends State<AudioFadeLayer> {
  _FadeGrab? _grab;
  final DragEscape _escape = DragEscape();

  @override
  void dispose() {
    _escape.dispose();
    super.dispose();
  }

  List<BridgeClip> get _clips => widget.track.entry.info.clips;

  /// How far a drag on the clip strip has moved this end of [clip]: a move
  /// carries both ends, and a trim carries the end it has hold of.
  int _shiftFrom(BridgeClip? clip, bool into) {
    final live = widget.preview.value;
    if (live == null || live.clip != clip?.id.toString()) return 0;
    return switch (live.grab) {
      BarGrab.move => live.shift,
      BarGrab.trimIn => into ? live.shift : 0,
      BarGrab.trimOut => into ? 0 : live.shift,
    };
  }

  /// Where the fade at one end stands now, in comp frames, with a drag on the
  /// clip strip in it. An unconverted row has no clip yet, so its own two ends
  /// are the corners.
  double _edgeOf(BridgeClip? clip, bool into) {
    final info = widget.track.entry.info;
    final shift = _shiftFrom(clip, into);
    if (clip == null) {
      return (into ? info.inFrame : info.outFrame).toInt().toDouble() + shift;
    }
    // Inside an overlap the corner drags the clip's own edge, so that is where
    // the handle stands and what the magnet lands on. The stored seconds are
    // not read at an end that overlaps, so drawing the handle at them would
    // put it in the middle of the clip, away from the join it moves.
    if (clipFadePartner(_clips, clip, into: into) != null) {
      return (into ? clip.startFrame : clip.endFrame).toInt().toDouble() +
          shift;
    }
    return clipFadeEdge(clip, into: into, fps: widget.fps) + shift;
  }

  /// How far the drag has travelled in whole frames, with the magnet applied -
  /// taken afresh from the raw travel, as every other drag here does it.
  int _shiftOf(_FadeGrab held) => snappedDelta(
        rawFrames: widget.axis.framesOfPx(held.at.dx - held.from.dx),
        perFrame: widget.axis.perFrame,
        sources: [_edgeOf(held.clip, held.into)],
        targets: widget.snapTargets,
        magnet: !snapSuspended(
            controlPressed: HardwareKeyboard.instance.isControlPressed),
      ).delta;

  /// The fade's length after a travel of [shift] frames, held inside the clip.
  double _secondsOf(_FadeGrab held, int shift) {
    final info = widget.track.entry.info;
    final start = held.clip?.startFrame.toInt() ?? info.inFrame.toInt();
    final end = held.clip?.endFrame.toInt() ?? info.outFrame.toInt();
    final edge = _edgeOf(held.clip, held.into) + shift;
    final frames = held.into ? edge - start : end - edge;
    final fps = widget.fps <= 0 ? 1.0 : widget.fps;
    return frames.clamp(0.0, math.max(0.0, (end - start).toDouble())) / fps;
  }

  void _down(PointerDownEvent event, BridgeClip? clip, bool into) {
    if (event.buttons != kPrimaryMouseButton) return;
    setState(() => _grab = (
          clip: clip,
          into: into,
          // Inside an overlap the corner drags the clip's edge, because the
          // overlap is the fade and its length is what the edge says.
          trim:
              clip != null && clipFadePartner(_clips, clip, into: into) != null,
          from: event.position,
          at: event.position,
        ));
    _escape.begin(() => setState(() => _grab = null));
  }

  void _move(PointerMoveEvent event) {
    final held = _grab;
    if (held == null || !_escape.running) return;
    setState(() => _grab = (
          clip: held.clip,
          into: held.into,
          trim: held.trim,
          from: held.from,
          at: event.position,
        ));
  }

  void _up(PointerUpEvent event) {
    final held = _grab;
    final commit = _escape.end();
    setState(() => _grab = null);
    if (held == null || !commit) return;
    final shift = _shiftOf(held);
    if (shift == 0) return;
    if (held.trim && held.clip != null) {
      widget.onCommit((
        clip: held.clip,
        grab: held.into ? BarGrab.trimIn : BarGrab.trimOut,
        shift: shift,
        at: held.at,
      ));
      return;
    }
    widget.onFade(
        (clip: held.clip, into: held.into, seconds: _secondsOf(held, shift)));
  }

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<AudioClipDrag?>(
        valueListenable: widget.preview,
        builder: (context, drag, _) => _body(context, drag),
      );

  Widget _body(BuildContext context, AudioClipDrag? drag) {
    final t = ThemeScope.of(context).theme;
    final held = _grab;
    final live = held == null || held.trim || held.clip == null
        ? null
        : (
            clip: held.clip!.id.toString(),
            into: held.into,
            seconds: _secondsOf(held, _shiftOf(held)),
          );
    final bare = _clips.isEmpty;
    if (bare && !audioTrackTakesClips(widget.track.entry.info)) {
      return const SizedBox.shrink();
    }
    return Stack(children: [
      Positioned.fill(
        child: IgnorePointer(
          child: CustomPaint(
            key: ValueKey<String>('atl-fades-${widget.track.id}'),
            painter: ClipFadePainter(
              ramps: _ramps(live, drag),
              gains: {
                for (final clip in _clips) clip.id.toString(): clip.gainDb,
              },
              axis: widget.axis,
              rising: t.curve.first,
              falling: t.curve.length > 1 ? t.curve[1] : t.curve.first,
            ),
          ),
        ),
      ),
      ..._handles(t, bare: bare),
      // Keyed, and last: a child that comes and goes mid-gesture must be, or
      // the element holding the drag is rebuilt underneath it.
      if (held != null && !held.trim) _readout(t, held),
    ]);
  }

  /// Every ramp the lane draws, with the strip's drag carried into the dragged
  /// clip's own.
  List<ClipFadeRamp> _ramps(ClipFadeDrag? live, AudioClipDrag? drag) {
    final ramps = clipFadeRamps(_clips, widget.fps, live: live);
    final id = drag?.clip;
    return id == null ? ramps : shiftRamps(ramps, id, drag!.grab, drag.shift);
  }

  /// Both corners of every clip, or of the bar on a row that has none yet.
  List<Widget> _handles(LumitTheme t, {required bool bare}) {
    if (widget.razor) return const [];
    if (bare) {
      return [_handle(t, null, into: true), _handle(t, null, into: false)];
    }
    return [
      for (final clip in _clips) ...[
        _handle(t, clip, into: true),
        _handle(t, clip, into: false),
      ],
    ];
  }

  /// One top corner's handle, standing where the fade at that end ends.
  Widget _handle(LumitTheme t, BridgeClip? clip, {required bool into}) {
    final id = clip?.id.toString() ?? widget.track.id;
    final held = _grab;
    final moving = held != null &&
        held.into == into &&
        held.clip?.id == clip?.id &&
        !held.trim;
    final shift = moving ? _shiftOf(held) : 0;
    final x = widget.axis.xOf(_edgeOf(clip, into) + shift);
    // The handle stands on the clip's gain line, which is where its ramp tops
    // out; the mark inside it is drawn three pixels down from the handle's top.
    final top = (audioClipGainY(clip?.gainDb ?? 0, widget.height) - 3)
        .clamp(0.0, math.max(0.0, widget.height - _handleHeight))
        .toDouble();
    return Positioned(
      key: ValueKey<String>('atl-fade-${into ? 'in' : 'out'}-$id'),
      left: x - _handleWidth / 2,
      top: top,
      width: _handleWidth,
      height: _handleHeight,
      child: MouseRegion(
        cursor: SystemMouseCursors.resizeLeftRight,
        child: Listener(
          behavior: HitTestBehavior.opaque,
          onPointerDown: (e) => _down(e, clip, into),
          onPointerMove: _move,
          onPointerUp: _up,
          onPointerCancel: (_) {
            _escape.end();
            setState(() => _grab = null);
          },
          child: Align(
            alignment: Alignment.topCenter,
            child: Padding(
              padding: const EdgeInsets.only(top: 3),
              child: Transform.rotate(
                angle: math.pi / 4,
                child: Container(
                  width: _handleMark,
                  height: _handleMark,
                  decoration: BoxDecoration(
                    border: Border.all(color: t.textSecondary),
                    color: t.surface1,
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// How long the fade is, while it is being dragged.
  Widget _readout(LumitTheme t, _FadeGrab held) {
    final shift = _shiftOf(held);
    return Positioned(
      key: const ValueKey<String>('atl-fade-readout'),
      left: widget.axis.xOf(_edgeOf(held.clip, held.into) + shift) + 8,
      top: 2,
      child: IgnorePointer(
        child: Text(
          '${_secondsOf(held, shift).toStringAsFixed(2)} '
          '${l10n.unitSymbolSeconds}',
          style: t.mono.copyWith(fontSize: 8, color: t.textPrimary),
        ),
      ),
    );
  }
}

/// The ramps on a track: each fade drawn as its own curve down to the lane's
/// floor, with the quiet room under it filled.
///
/// An overlap arrives here as the two ramps that cross in it, which is the
/// board's X drawn as the two curves the mixer actually plays.
class ClipFadePainter extends CustomPainter {
  final List<ClipFadeRamp> ramps;

  /// Each clip's gain in dB, by id. A ramp rises to its own clip's gain line,
  /// so a faded clip reads as loud as it is (docs/impl/audio-timeline.md §5).
  final Map<String, double> gains;

  final TimelineAxis axis;
  final Color rising;
  final Color falling;

  const ClipFadePainter({
    required this.ramps,
    required this.gains,
    required this.axis,
    required this.rising,
    required this.falling,
  });

  /// Where the ramps of [clip] top out on a lane [height] pixels tall: its own
  /// gain line, and never below the foot of its box.
  double topOf(String clip, double height) => math.min(
      math.max(2.0, height - 2), audioClipGainY(gains[clip] ?? 0, height));

  @override
  void paint(Canvas canvas, Size size) {
    // The clip's box is inset two pixels top and bottom, so a ramp on a clip at
    // unity ends on the box's own edge.
    final bottom = math.max(2.0, size.height - 2);
    for (final ramp in ramps) {
      final top = topOf(ramp.clip, size.height);
      final left = axis.xOf(ramp.from);
      final right = axis.xOf(ramp.to);
      if (!(right > left)) continue;
      final curve = Path();
      const steps = 24;
      for (var i = 0; i <= steps; i++) {
        final u = i / steps;
        final gain = clipFadeGain(ramp.shape, ramp.into ? u : 1 - u);
        final point =
            Offset(left + (right - left) * u, bottom - gain * (bottom - top));
        if (i == 0) {
          curve.moveTo(point.dx, point.dy);
        } else {
          curve.lineTo(point.dx, point.dy);
        }
      }
      final colour = ramp.into ? rising : falling;
      canvas.drawPath(
        Path.from(curve)
          ..lineTo(right, bottom)
          ..lineTo(left, bottom)
          ..close(),
        Paint()..color = colour.withValues(alpha: clipFadeFillAlpha),
      );
      canvas.drawPath(
        curve,
        Paint()
          ..color = colour
          ..strokeWidth = 1.2
          ..style = PaintingStyle.stroke,
      );
    }
  }

  @override
  bool shouldRepaint(ClipFadePainter old) =>
      old.ramps != ramps ||
      !mapEquals(old.gains, gains) ||
      old.axis != axis ||
      old.rising != rising ||
      old.falling != falling;
}
