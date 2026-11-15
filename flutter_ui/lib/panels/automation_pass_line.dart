// The stretch of a lane being written by a pass, drawn in the error tone
// while the comp plays (docs/impl/audio-timeline.md §5).
//
// One feed for the whole panel polls the pass once a frame, and only while
// the comp plays and at least one lane is on screen, taking only the points
// drawn since it last asked; each open lane draws its own stretch from what
// the feed has heard, carried on to the playhead at the control's value now. When playback stops the stretch
// goes, and the keys the pass wrote arrive with the document.

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';

import '../widgets/controls.dart';
import 'audio_timeline_live.dart';
import 'mixer_live.dart' show StripPosition, mixerPositions;
import 'timeline_extras_frb.dart' show TimelineAxis;
import 'volume_band_frb.dart' show BandScale;

/// What the running pass has written, by lane id, kept fresh while it runs.
class AutomationPassFeed {
  final ValueListenable<bool> _playing;
  final ValueNotifier<Map<String, List<PassPoint>>> points =
      ValueNotifier(const {});

  /// Where each strip's controls stand at the clock while the comp plays, by
  /// layer id, the master under the empty id: the value heard, held or read
  /// off the keys. Empty while stopped.
  final ValueNotifier<Map<String, StripPosition>> positions =
      ValueNotifier(const {});
  CompositionReference? _comp;
  int _lanes = 0;
  Ticker? _ticker;

  /// The pass heard last and the next point to ask it from, what it has
  /// drawn so far, and each lane's control.
  int _pass = 0;
  int _from = 0;
  Map<String, List<PassPoint>> _drawn = {};
  final Map<String, BridgeMixTarget> _targets = {};

  /// The playhead, and where it stood at the last poll: a lane's stretch is
  /// carried on to it, at [live], the lane's control as heard now.
  final ValueListenable<int> _playhead;
  int frame = 0;
  Map<String, double> live = const {};

  AutomationPassFeed(this._playing, this._playhead) {
    _playing.addListener(_sync);
  }

  /// A lane on [comp] came on screen.
  void attach(CompositionReference comp) {
    _comp = comp;
    _lanes += 1;
    _sync();
  }

  /// A lane left the screen.
  void detach() {
    _lanes -= 1;
    _sync();
  }

  void _sync() {
    if (_playing.value && _lanes > 0 && _comp != null) {
      _ticker ??= Ticker(_tick)..start();
      return;
    }
    _ticker?.dispose();
    _ticker = null;
    _drawn = {};
    _targets.clear();
    if (points.value.isNotEmpty) points.value = const {};
    if (positions.value.isNotEmpty) positions.value = const {};
  }

  void _tick(Duration _) {
    final comp = _comp;
    if (comp == null) return;
    try {
      positions.value = mixerPositions();
    } catch (_) {
      // Nothing loaded to ask.
    }
    final since = passPointsSince(comp, pass: _pass, from: _from);
    if (since != null) {
      if (since.pass != _pass) {
        _drawn = {};
        _targets.clear();
      }
      _pass = since.pass;
      _from = since.next;
      for (final MapEntry(:key, :value) in since.points.entries) {
        if (value.isNotEmpty) _drawn[key] = [...?_drawn[key], ...value];
      }
      _targets.addAll(since.targets);
    }
    final now = positions.value;
    live = {
      for (final MapEntry(:key, :value) in _targets.entries)
        if (switch (value) {
              BridgeMixTarget_Volume(:final layer) => now['$layer']?.volumeDb,
              BridgeMixTarget_Pan(:final layer) => now['$layer']?.pan,
              BridgeMixTarget_Master() => now['']?.volumeDb,
              BridgeMixTarget_Send() => null,
            }
            case final v?)
          key: v,
    };
    frame = _playhead.value;
    if (_drawn.isEmpty && points.value.isEmpty) return;
    points.value = Map.of(_drawn);
  }

  void dispose() {
    _playing.removeListener(_sync);
    _ticker?.dispose();
    _ticker = null;
    points.dispose();
    positions.dispose();
  }
}

/// One lane's stretch of the pass, over its curve.
class AutomationPassLine extends StatefulWidget {
  final AutomationPassFeed feed;
  final CompositionReference comp;
  final String laneId;
  final BandScale scale;
  final TimelineAxis axis;
  final double fps;
  final double height;

  const AutomationPassLine({
    super.key,
    required this.feed,
    required this.comp,
    required this.laneId,
    required this.scale,
    required this.axis,
    required this.fps,
    required this.height,
  });

  @override
  State<AutomationPassLine> createState() => _AutomationPassLineState();
}

class _AutomationPassLineState extends State<AutomationPassLine> {
  @override
  void initState() {
    super.initState();
    widget.feed.attach(widget.comp);
  }

  @override
  void dispose() {
    widget.feed.detach();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return IgnorePointer(
      child: ValueListenableBuilder<Map<String, List<PassPoint>>>(
        valueListenable: widget.feed.points,
        builder: (context, all, _) => CustomPaint(
          size: Size(widget.axis.width, widget.height),
          painter: _PassPainter(
            points: all[widget.laneId] ?? const [],
            tail: (
              x: widget.axis.xOf(widget.feed.frame),
              value: widget.feed.live[widget.laneId],
            ),
            scale: widget.scale,
            axis: widget.axis,
            fps: widget.fps,
            colour: t.error,
          ),
        ),
      ),
    );
  }
}

class _PassPainter extends CustomPainter {
  final List<PassPoint> points;

  /// The playhead's x and the control's value there, where it is known.
  final ({double x, double? value}) tail;
  final BandScale scale;
  final TimelineAxis axis;
  final double fps;
  final Color colour;

  const _PassPainter({
    required this.points,
    required this.tail,
    required this.scale,
    required this.axis,
    required this.fps,
    required this.colour,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (points.isEmpty) return;
    // On the band's own scale, a pixel in from the top and the foot.
    Offset at(PassPoint p) => Offset(axis.xOf(p.seconds * fps),
        1 + scale.unitOf(p.value).clamp(0.0, 1.0) * (size.height - 2));
    final path = Path()..moveTo(at(points.first).dx, at(points.first).dy);
    for (final p in points.skip(1)) {
      final o = at(p);
      path.lineTo(o.dx, o.dy);
    }
    // On to the playhead, while it is less than a second past the last point
    // drawn: the pass is still writing, and the line keeps up with it.
    final last = at(points.last);
    final gap = tail.x - last.dx;
    if (gap > 0 && gap <= axis.perFrame * fps) {
      final y = tail.value == null
          ? last.dy
          : at((seconds: 0, value: tail.value!)).dy;
      path.lineTo(tail.x, y);
    }
    canvas.drawPath(
        path,
        Paint()
          ..color = colour
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.5);
  }

  @override
  bool shouldRepaint(_PassPainter old) =>
      !identical(old.points, points) ||
      old.tail != tail ||
      old.colour != colour ||
      old.axis != axis ||
      old.scale != scale;
}
