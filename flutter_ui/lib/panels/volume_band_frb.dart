// The volume rubber band (K-695, the AudioWorkspace board): the layer's
// Volume drawn ON the waveform lane as a line the pointer can take hold of,
// with a diamond per keyframe and a dB readout while a drag runs.
//
// In plain terms: every NLE draws a "rubber band" across an audio track —
// drag it down, the track gets quieter; the line IS the volume. Lumit's
// Volume has always been an ordinary keyframable property; this widget is
// that property wearing the band's clothes on the lane where the sound is
// looked at. Nothing new is stored: the line reads `Layer.volume_db`, and a
// drag writes it back through the same `setVolumeDb` every other control
// uses, so the graph editor, the Audio panel and this band are three views
// of one curve.
//
// The band claims the pointer only **near its own line or a diamond**
// (the painter's hitTest): everywhere else the lane's marquee and ground
// keep their gestures. A vertical drag moves the grabbed key's value — or
// the whole level, when the volume is still static — and commits once, on
// release. `Ctrl`-click plants a key on the line (the lane ground's own
// planting gesture, aimed at this row); `Alt`-click lifts one, the graph
// and envelope grammar.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'graph_maths.dart';
import 'layer_fold_frb.dart';
import 'timeline_extras_frb.dart';

/// The loudest the band's top means, in dB. A boost is real (+50 is the
/// property's ceiling) but the band is for riding a level, not for slamming
/// one — the value field takes the extremes.
const double volumeBandTopDb = 12;

/// The band's floor. −60 dB is where the ear stops caring and the property's
/// −100 knee reads "−inf" anyway; pixels spent below it say nothing.
const double volumeBandFloorDb = -60;

/// The layer's Volume as a band over its waveform lane.
class VolumeBand extends StatefulWidget {
  final BridgeLayerEntry entry;
  final TimelineAxis axis;
  final double fps;
  final int fpsNum;
  final int fpsDen;

  /// The comp-frame shift of a bar move in flight (§6.26), so the band's
  /// diamonds travel with the bar exactly as the lane's other keys do.
  final int barShift;

  /// The lane's row height — the band maps its dB scale over this row, the
  /// one the pointer can actually reach (the wave's borrowed upper row
  /// belongs to the Waveform heading and takes no gestures).
  final double rowHeight;

  final VoidCallback onChanged;

  const VolumeBand({
    super.key,
    required this.entry,
    required this.axis,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    required this.barShift,
    required this.rowHeight,
    required this.onChanged,
  });

  @override
  State<VolumeBand> createState() => VolumeBandState();
}

class VolumeBandState extends State<VolumeBand> {
  /// The drag in flight: which key it holds (null while the volume is
  /// static and the whole level moves), the dB it has reached, and the dB it
  /// began at — travel in, travel out, so the line never teleports to the
  /// pointer.
  ({int? index, double db})? _drag;
  double _grabbedAt = 0;
  double _travelled = 0;

  BridgeScalar get _scalar => widget.entry.info.volumeDb;

  List<BridgeKeyframe> get _keys => switch (_scalar) {
        BridgeScalar_Keyframed(:final field0) => field0,
        _ => const [],
      };

  double get _staticDb => switch (_scalar) {
        BridgeScalar_Static(:final field0) => field0,
        _ => 0,
      };

  /// dB → the lane row's y, [volumeBandTopDb] a pixel in from the top and
  /// the floor a pixel off the bottom.
  double _y(double db) {
    final span = volumeBandTopDb - volumeBandFloorDb;
    final unit = ((volumeBandTopDb - db) / span).clamp(0.0, 1.0);
    return 1 + unit * (widget.rowHeight - 2);
  }

  double _dbOfY(double y) {
    final span = volumeBandTopDb - volumeBandFloorDb;
    return volumeBandTopDb - (y - 1) / (widget.rowHeight - 2) * span;
  }

  /// A key's x, on the comp clock its times cross in (K-213), carried along
  /// by a bar move in flight.
  double _xOfKey(BridgeKeyframe key) =>
      widget.axis.xOf(laneKeyFrame(key, widget.fps) + widget.barShift);

  /// The band's points as drawn right now: each key's (x, y[dB]) with the
  /// drag in flight applied, or the flat static level.
  List<Offset> _points() {
    final keys = _keys;
    final held = _drag;
    if (keys.isEmpty) {
      final db = held != null ? held.db : _staticDb;
      final y = _y(db);
      return [Offset(0, y), Offset(widget.axis.width, y)];
    }
    return [
      for (var i = 0; i < keys.length; i++)
        Offset(
          _xOfKey(keys[i]),
          _y(held != null && held.index == i ? held.db : keys[i].value),
        ),
    ];
  }

  /// The dB the band reads at [x], off the drawn points — what a planted key
  /// takes, so planting moves nothing.
  ///
  /// ponytail: linear between keys, so an eased fade's planted key can land a
  /// shade off the engine's curve; sample through the engine if that shade is
  /// ever noticed.
  double _dbAt(double x, List<Offset> pts) {
    if (pts.isEmpty) return _staticDb;
    if (x <= pts.first.dx) return _dbOfY(pts.first.dy);
    for (var i = 0; i + 1 < pts.length; i++) {
      if (x <= pts[i + 1].dx) {
        final w = pts[i + 1].dx - pts[i].dx;
        final f = w <= 0 ? 0.0 : ((x - pts[i].dx) / w).clamp(0.0, 1.0);
        return _dbOfY(pts[i].dy + (pts[i + 1].dy - pts[i].dy) * f);
      }
    }
    return _dbOfY(pts.last.dy);
  }

  /// The nearest key within reach of [local], by x — the band is a line, so
  /// aim matters horizontally and the line answers vertically.
  int? _keyNear(Offset local) {
    final keys = _keys;
    int? best;
    var bestD = 12.0;
    for (var i = 0; i < keys.length; i++) {
      final d = (_xOfKey(keys[i]) - local.dx).abs();
      if (d < bestD) {
        bestD = d;
        best = i;
      }
    }
    return best;
  }

  void _startDrag(Offset local) {
    final keys = _keys;
    final index = keys.isEmpty ? null : (_keyNear(local) ?? _nearestByX(local));
    final at = index == null ? _staticDb : keys[index].value;
    setState(() {
      _grabbedAt = at;
      _travelled = 0;
      _drag = (index: index, db: at);
    });
  }

  /// The nearest key by x alone — the fallback that makes the band usable:
  /// grabbing the line between two diamonds moves the nearer one, the same
  /// forgiveness the envelope strip extends.
  int _nearestByX(Offset local) {
    final keys = _keys;
    var nearest = 0;
    var nearestD = double.infinity;
    for (var i = 0; i < keys.length; i++) {
      final d = (_xOfKey(keys[i]) - local.dx).abs();
      if (d < nearestD) {
        nearestD = d;
        nearest = i;
      }
    }
    return nearest;
  }

  void _moveDrag(double dy) {
    final held = _drag;
    if (held == null) return;
    _travelled += dy;
    final span = volumeBandTopDb - volumeBandFloorDb;
    final db = (_grabbedAt - _travelled / (widget.rowHeight - 2) * span)
        .clamp(volumeBandFloorDb, volumeBandTopDb);
    setState(() => _drag = (index: held.index, db: db));
  }

  void _commitDrag() {
    final held = _drag;
    setState(() => _drag = null);
    if (held == null) return;
    if (held.db == _grabbedAt) return;
    final keys = _keys;
    if (held.index == null || keys.isEmpty) {
      widget.entry.layer.setVolumeDb(value: BridgeScalar.static_(held.db));
    } else {
      widget.entry.layer.setVolumeDb(
        value: BridgeScalar.keyframed([
          for (var i = 0; i < keys.length; i++)
            i == held.index
                ? BridgeKeyframe(
                    time: keys[i].time,
                    value: held.db,
                    interpIn: keys[i].interpIn,
                    interpOut: keys[i].interpOut,
                  )
                : keys[i],
        ]),
      );
    }
    widget.onChanged();
  }

  /// `Ctrl`-click plants a key on the line at that moment, reading the value
  /// the band already draws there — a place to grab, not a change.
  /// `Alt`-click lifts the diamond under the pointer; lifting the last one
  /// leaves the level as a static at that key's value, which is what the
  /// curve read anyway.
  void _tap(Offset local) {
    final keyboard = HardwareKeyboard.instance;
    final keys = _keys;
    if (keyboard.isAltPressed) {
      final index = _keyNear(local);
      if (index == null) return;
      final left = [
        for (var i = 0; i < keys.length; i++)
          if (i != index) keys[i],
      ];
      widget.entry.layer.setVolumeDb(
        value: left.isEmpty
            ? BridgeScalar.static_(keys[index].value)
            : BridgeScalar.keyframed(left),
      );
      widget.onChanged();
      return;
    }
    if (!keyboard.isControlPressed && !keyboard.isMetaPressed) return;
    final frame = widget.axis.frameAtExact(local.dx) - widget.barShift;
    final time = timeOfSubframe(frame, widget.fpsNum, widget.fpsDen);
    final db = _dbAt(local.dx, _points());
    final planted = BridgeKeyframe(
      time: time,
      value: db,
      interpIn: const BridgeSideInterp.linear(),
      interpOut: const BridgeSideInterp.linear(),
    );
    final grown = [...keys, planted]
      ..sort((a, b) => rationalSeconds(a.time).compareTo(
          rationalSeconds(b.time)));
    // Two keys on one moment is a curve the engine refuses; the click that
    // lands exactly on an existing key plants nothing.
    for (var i = 1; i < grown.length; i++) {
      if (rationalSeconds(grown[i].time) <=
          rationalSeconds(grown[i - 1].time)) {
        return;
      }
    }
    widget.entry.layer.setVolumeDb(value: BridgeScalar.keyframed(grown));
    widget.onChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final points = _points();
    final held = _drag;
    final readout = held != null
        ? _readoutText(held.db)
        : _keys.isEmpty
            ? _readoutText(_staticDb)
            : null;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeUpDown,
      // Only where the band itself answers (the painter's hitTest): the
      // region must not steal hover from the whole lane.
      opaque: false,
      hitTestBehavior: HitTestBehavior.deferToChild,
      child: GestureDetector(
        key: ValueKey<String>(
            'tl-volume-band-${widget.entry.layer.internallayerId}'),
        behavior: HitTestBehavior.deferToChild,
        supportedDevices: dragDevices,
        onTapUp: (d) => _tap(d.localPosition),
        onVerticalDragStart: (d) => _startDrag(d.localPosition),
        onVerticalDragUpdate: (d) => _moveDrag(d.delta.dy),
        onVerticalDragEnd: (_) => _commitDrag(),
        onVerticalDragCancel: () => setState(() => _drag = null),
        child: CustomPaint(
          size: Size(widget.axis.width, widget.rowHeight),
          painter: VolumeBandPainter(
            points: points,
            line: t.accent,
            diamond: t.animated,
            readout: readout,
            readoutStyle:
                t.mono.copyWith(fontSize: 8, color: t.textMuted),
            keyed: _keys.isNotEmpty,
          ),
        ),
      ),
    );
  }

  String _readoutText(double db) => db <= volumeBandFloorDb
      ? l10n.volumeNegInf
      : '${db.toStringAsFixed(1)} dB';
}

/// The band's line, its diamonds, and the readout — and the hit test that
/// keeps the pointer's claim to the lane honest: only within reach of the
/// line or a diamond does the band answer at all.
class VolumeBandPainter extends CustomPainter {
  final List<Offset> points;
  final Color line;
  final Color diamond;
  final String? readout;
  final TextStyle readoutStyle;

  /// Whether the points are keyframes (drawn as diamonds) or the flat static
  /// level (drawn bare).
  final bool keyed;

  const VolumeBandPainter({
    required this.points,
    required this.line,
    required this.diamond,
    required this.readout,
    required this.readoutStyle,
    required this.keyed,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (points.isEmpty) return;
    final stroke = Paint()
      ..color = line
      ..strokeWidth = 1.4
      ..style = PaintingStyle.stroke;
    final path = Path()..moveTo(0, points.first.dy);
    path.lineTo(points.first.dx, points.first.dy);
    for (final p in points.skip(1)) {
      path.lineTo(p.dx, p.dy);
    }
    path.lineTo(size.width, points.last.dy);
    canvas.drawPath(path, stroke);

    if (keyed) {
      // The board's two-diamond grammar: a filled diamond per key, in the
      // animated gold every other keyframe diamond wears.
      final fill = Paint()..color = diamond;
      for (final p in points) {
        canvas.drawPath(
          Path()
            ..moveTo(p.dx, p.dy - 3.5)
            ..lineTo(p.dx + 3.5, p.dy)
            ..lineTo(p.dx, p.dy + 3.5)
            ..lineTo(p.dx - 3.5, p.dy)
            ..close(),
          fill,
        );
      }
    }

    if (readout case final text?) {
      final painter = TextPainter(
        text: TextSpan(text: text, style: readoutStyle),
        textDirection: TextDirection.ltr,
      )..layout();
      painter.paint(canvas, Offset(size.width - painter.width - 6, 1));
    }
  }

  /// The band answers near its line or a diamond and nowhere else, so the
  /// marquee and the lane ground keep the rest of the row.
  @override
  bool? hitTest(Offset position) {
    if (points.isEmpty) return false;
    for (final p in points) {
      if ((p - position).distance <= 7) return true;
    }
    // Against the segment run, x-clamped the way the drawn line extends.
    var previous = Offset(0, points.first.dy);
    for (final p in [...points, Offset(double.infinity, points.last.dy)]) {
      final to = p.dx.isFinite ? p : Offset(position.dx + 1, p.dy);
      if (position.dx >= previous.dx - 4 && position.dx <= to.dx + 4) {
        final w = to.dx - previous.dx;
        final f = w <= 0 ? 0.0 : ((position.dx - previous.dx) / w).clamp(0.0, 1.0);
        final y = previous.dy + (to.dy - previous.dy) * f;
        if ((position.dy - y).abs() <= 5) return true;
      }
      previous = to;
    }
    return false;
  }

  @override
  bool shouldRepaint(VolumeBandPainter old) =>
      old.points != points ||
      old.line != line ||
      old.diamond != diamond ||
      old.readout != readout ||
      old.keyed != keyed;
}
