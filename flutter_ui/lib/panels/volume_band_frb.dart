// The volume rubber band (the AudioWorkspace board): the layer's
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
//
// The same band draws Pan when asked ([BandKind.pan]): centre in the middle,
// left at the top and right at the bottom, as Vegas draws it. A double-click
// on a key puts it back to 0 dB or centre, and anywhere else on the line
// plants one. `Ctrl` held during a drag moves it at
// a tenth of the speed. An expression's value or a locked layer is not
// for dragging: the first draws nothing, the second only the line.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/gestures.dart' show kDoubleTapSlop, kDoubleTapTimeout;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'graph_maths.dart';
import 'layer_fold_frb.dart';
import 'mixer_panel_frb.dart' show panLabel;
import 'timeline_extras_frb.dart';

/// The loudest the band's top means, in dB. A boost is real (+50 is the
/// property's ceiling) but the band is for riding a level, not for slamming
/// one — the value field takes the extremes.
const double volumeBandTopDb = 12;

/// The band's floor. −60 dB is where the ear stops caring and the property's
/// −100 knee reads "−inf" anyway; pixels spent below it say nothing.
const double volumeBandFloorDb = -60;

/// The y a band [height] pixels tall draws [db] at, [topDb] a pixel in from the
/// top and the floor a pixel off the bottom.
///
/// Top-level, and taking its top, because a clip's gain line on the Audio
/// timeline is this same scale read from 0 dB down: the two draw one shape and
/// so are one mapping (docs/impl/audio-timeline.md §5).
double volumeBandY(double db, double height, {double topDb = volumeBandTopDb}) {
  final span = topDb - volumeBandFloorDb;
  final unit = ((topDb - db) / span).clamp(0.0, 1.0);
  return 1 + unit * (height - 2);
}

/// The dB such a band reads at [y], which is [volumeBandY] the other way round.
double volumeBandDbOfY(double y, double height,
        {double topDb = volumeBandTopDb}) =>
    topDb - (y - 1) / (height - 2) * (topDb - volumeBandFloorDb);

/// Which property a band draws and edits.
enum BandKind { volume, pan }

/// The layer's Volume (or Pan) as a band over its lane.
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

  final BandKind kind;

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
    this.kind = BandKind.volume,
  });

  @override
  State<VolumeBand> createState() => VolumeBandState();
}

class VolumeBandState extends State<VolumeBand> {
  /// The drag in flight: which key it holds (null while the property is
  /// static and the whole level moves), the value it has reached, and the
  /// value it began at. Travel in, travel out, so the line never teleports to
  /// the pointer.
  ({int? index, double value})? _drag;
  double _grabbedAt = 0;
  double _travelled = 0;

  /// Where a static level was taken hold of, so its readout sits there.
  double _grabX = 0;

  /// The last plain click, so a second one close by makes a double-click
  /// without holding every single click back the way a double-tap gesture
  /// would.
  DateTime? _lastTap;
  Offset _lastTapAt = Offset.zero;

  /// Null while the pointer is off the band; on it, the diamond under it (null
  /// on the bare line). The painter listens, so hover repaints and rebuilds
  /// nothing.
  final ValueNotifier<({int? key})?> _hover = ValueNotifier(null);

  @override
  void didUpdateWidget(VolumeBand oldWidget) {
    super.didUpdateWidget(oldWidget);
    // The value changed under a held drag (an undo, say): let go, write
    // nothing.
    final was = oldWidget.kind == BandKind.pan
        ? oldWidget.entry.info.pan
        : oldWidget.entry.info.volumeDb;
    if (_drag != null && was != _scalar) _drag = null;
  }

  @override
  void dispose() {
    _hover.dispose();
    super.dispose();
  }

  bool get _pan => widget.kind == BandKind.pan;

  BridgeScalar get _scalar =>
      _pan ? widget.entry.info.pan : widget.entry.info.volumeDb;

  List<BridgeKeyframe> get _keys => switch (_scalar) {
        BridgeScalar_Keyframed(:final field0) => field0,
        _ => const [],
      };

  double get _staticValue => switch (_scalar) {
        BridgeScalar_Static(:final field0) => field0,
        _ => 0,
      };

  /// Pan runs left at the top to right at the bottom, centre between.
  double _y(double value) => _pan
      ? 1 + ((value + 100) / 200).clamp(0.0, 1.0) * (widget.rowHeight - 2)
      : volumeBandY(value, widget.rowHeight);

  double _valueOfY(double y) => _pan
      ? (-100 + (y - 1) / (widget.rowHeight - 2) * 200).clamp(-100.0, 100.0)
      : volumeBandDbOfY(y, widget.rowHeight)
          .clamp(volumeBandFloorDb, volumeBandTopDb);

  /// A key's x, on the comp clock its times cross in, carried along
  /// by a bar move in flight.
  double _xOfKey(BridgeKeyframe key) =>
      widget.axis.xOf(laneKeyFrame(key, widget.fps) + widget.barShift);

  /// The band's points as drawn right now: each key's (x, y[value]) with the
  /// drag in flight applied, or the flat static level.
  List<Offset> _points() {
    final keys = _keys;
    final held = _drag;
    if (keys.isEmpty) {
      final y = _y(held != null ? held.value : _staticValue);
      return [Offset(0, y), Offset(widget.axis.width, y)];
    }
    return [
      for (var i = 0; i < keys.length; i++)
        Offset(
          _xOfKey(keys[i]),
          _y(held != null && held.index == i ? held.value : keys[i].value),
        ),
    ];
  }

  /// The value the band reads at [x], off the drawn points — what a planted
  /// key takes, so planting moves nothing.
  ///
  /// ponytail: linear between keys, so an eased fade's planted key can land a
  /// shade off the engine's curve; sample through the engine if that shade is
  /// ever noticed.
  double _valueAt(double x, List<Offset> pts) {
    if (pts.isEmpty) return _staticValue;
    if (x <= pts.first.dx) return _valueOfY(pts.first.dy);
    for (var i = 0; i + 1 < pts.length; i++) {
      if (x <= pts[i + 1].dx) {
        final w = pts[i + 1].dx - pts[i].dx;
        final f = w <= 0 ? 0.0 : ((x - pts[i].dx) / w).clamp(0.0, 1.0);
        return _valueOfY(pts[i].dy + (pts[i + 1].dy - pts[i].dy) * f);
      }
    }
    return _valueOfY(pts.last.dy);
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
    final at = index == null ? _staticValue : keys[index].value;
    setState(() {
      _grabbedAt = at;
      _grabX = local.dx;
      _travelled = 0;
      _drag = (index: index, value: at);
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
    final keyboard = HardwareKeyboard.instance;
    // Ctrl held is the fine trim: a tenth of the speed.
    _travelled += keyboard.isControlPressed || keyboard.isMetaPressed
        ? dy / 10
        : dy;
    final value = _valueOfY(_y(_grabbedAt) + _travelled);
    setState(() => _drag = (index: held.index, value: value));
  }

  void _commitDrag() {
    final held = _drag;
    setState(() => _drag = null);
    if (held == null) return;
    if (held.value == _grabbedAt) return;
    final keys = _keys;
    _write(held.index == null || keys.isEmpty
        ? BridgeScalar.static_(held.value)
        : _withKey(keys, held.index!, held.value));
  }

  /// [keys] with key [index] set to [value], its time and easing kept.
  BridgeScalar _withKey(List<BridgeKeyframe> keys, int index, double value) =>
      BridgeScalar.keyframed([
        for (var i = 0; i < keys.length; i++)
          i == index
              ? BridgeKeyframe(
                  time: keys[i].time,
                  value: value,
                  interpIn: keys[i].interpIn,
                  interpOut: keys[i].interpOut,
                )
              : keys[i],
      ]);

  void _write(BridgeScalar next) {
    final layer = widget.entry.layer;
    try {
      _pan ? layer.setPan(value: next) : layer.setVolumeDb(value: next);
    } catch (_) {
      // A locked layer refuses, and quietly.
      return;
    }
    widget.onChanged();
  }

  /// A double-click, as in Vegas: on a key it puts that key back to 0 dB or
  /// centre, and anywhere else on the line it plants one.
  void _reset(Offset local) {
    final keys = _keys;
    final index = keys.isEmpty ? null : _keyNear(local);
    if (index == null) return _plant(local);
    if (keys[index].value == 0) return;
    _write(_withKey(keys, index, 0));
  }

  /// `Ctrl`-click plants a key on the line at that moment, reading the value
  /// the band already draws there — a place to grab, not a change.
  /// `Alt`-click lifts the diamond under the pointer; lifting the last one
  /// leaves the level as a static at that key's value, which is what the
  /// curve read anyway. Two plain clicks close together are a double-click.
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
      _write(left.isEmpty
          ? BridgeScalar.static_(keys[index].value)
          : BridgeScalar.keyframed(left));
      return;
    }
    if (!keyboard.isControlPressed && !keyboard.isMetaPressed) {
      final now = DateTime.now();
      final last = _lastTap;
      final twice = last != null &&
          now.difference(last) <= kDoubleTapTimeout &&
          (local - _lastTapAt).distance <= kDoubleTapSlop;
      _lastTap = twice ? null : now;
      _lastTapAt = local;
      if (twice) _reset(local);
      return;
    }
    _plant(local);
  }

  /// A key planted on the line at [local], at the value the line reads there.
  void _plant(Offset local) {
    final keys = _keys;
    final frame = widget.axis.frameAtExact(local.dx) - widget.barShift;
    final time = timeOfSubframe(frame, widget.fpsNum, widget.fpsDen);
    final value = _valueAt(local.dx, _points());
    final planted = BridgeKeyframe(
      time: time,
      value: value,
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
    _write(BridgeScalar.keyframed(grown));
  }

  @override
  Widget build(BuildContext context) {
    // An expression drives the value: nothing here may write over it.
    if (_scalar is BridgeScalar_Expression) {
      return SizedBox(width: widget.axis.width, height: widget.rowHeight);
    }
    final t = ThemeScope.of(context).theme;
    final points = _points();
    final held = _drag;
    final id = widget.entry.layer.internallayerId;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeUpDown,
      // Only where the band itself answers (the painter's hitTest): the
      // region must not steal hover from the whole lane.
      opaque: false,
      hitTestBehavior: HitTestBehavior.deferToChild,
      onHover: (e) => _hover.value = (key: _keyNear(e.localPosition)),
      // A held drag keeps the line lit wherever the pointer wanders.
      onExit: (_) {
        if (_drag == null) _hover.value = null;
      },
      child: GestureDetector(
        key: ValueKey<String>(
            _pan ? 'tl-pan-band-$id' : 'tl-volume-band-$id'),
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
            // Pan in the warning tone, so the two lines on one lane never
            // read as each other.
            line: _pan ? t.warning : t.accent,
            halo: t.surface0.withValues(alpha: 0.75),
            diamond: t.animated,
            readout: held == null ? null : _readoutText(held.value),
            readoutAt: held == null
                ? null
                : Offset(held.index == null ? _grabX : points[held.index!].dx,
                    _y(held.value)),
            readoutStyle:
                t.mono.copyWith(fontSize: 9, color: t.textMuted),
            keyed: _keys.isNotEmpty,
            hover: _hover,
            held: held != null,
            // A locked layer shows its line and takes no hold of it.
            grabbable: !widget.entry.info.switches.locked,
          ),
        ),
      ),
    );
  }

  String _readoutText(double value) => _pan
      ? panLabel(value)
      : value <= volumeBandFloorDb
          ? l10n.volumeNegInf
          : '${value.toStringAsFixed(1)} ${l10n.unitSymbolDb}';
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

  /// The held point the readout sits beside.
  final Offset? readoutAt;

  /// A drag is held: the band answers everywhere until it lets go, so the
  /// line stays lit and the cursor stays put.
  final bool held;

  /// False on a locked layer: the line draws and takes no pointer.
  final bool grabbable;

  /// A darker edge drawn under the line, so it reads over a busy waveform.
  final Color? halo;

  /// Whether the points are keyframes (drawn as diamonds) or the flat static
  /// level (drawn bare).
  final bool keyed;

  /// The pointer on the band: the line thickens, and the diamond under it
  /// grows.
  final ValueListenable<({int? key})?>? hover;

  const VolumeBandPainter({
    required this.points,
    required this.line,
    required this.diamond,
    required this.readout,
    required this.readoutStyle,
    required this.keyed,
    this.halo,
    this.hover,
    this.readoutAt,
    this.held = false,
    this.grabbable = true,
  }) : super(repaint: hover);

  @override
  void paint(Canvas canvas, Size size) {
    if (points.isEmpty) return;
    final over = hover?.value;
    final width = over != null ? 2.2 : 1.6;
    final stroke = Paint()
      ..color = line
      ..strokeWidth = width
      ..style = PaintingStyle.stroke;
    final path = Path()..moveTo(0, points.first.dy);
    path.lineTo(points.first.dx, points.first.dy);
    for (final p in points.skip(1)) {
      path.lineTo(p.dx, p.dy);
    }
    path.lineTo(size.width, points.last.dy);
    if (halo case final edge?) {
      canvas.drawPath(
          path,
          Paint()
            ..color = edge
            ..strokeWidth = width + 2
            ..style = PaintingStyle.stroke);
    }
    canvas.drawPath(path, stroke);

    if (keyed) {
      // The board's two-diamond grammar: a filled diamond per key, in the
      // animated gold every other keyframe diamond wears.
      final fill = Paint()..color = diamond;
      for (var i = 0; i < points.length; i++) {
        final p = points[i];
        final r = over?.key == i ? 5.0 : 3.5;
        canvas.drawPath(
          Path()
            ..moveTo(p.dx, p.dy - r)
            ..lineTo(p.dx + r, p.dy)
            ..lineTo(p.dx, p.dy + r)
            ..lineTo(p.dx - r, p.dy)
            ..close(),
          fill,
        );
      }
    }

    if ((readout, readoutAt) case (final text?, final at?)) {
      final painter = TextPainter(
        text: TextSpan(text: text, style: readoutStyle),
        textDirection: TextDirection.ltr,
      )..layout();
      // Just right of the held point and above the line, flipped left where
      // it would run off the end and below where it would leave the top.
      var x = at.dx + 8;
      if (x + painter.width > size.width) x = at.dx - 8 - painter.width;
      final above = at.dy - painter.height - 2;
      painter.paint(canvas, Offset(x, above >= 0 ? above : at.dy + 2));
    }
  }

  /// The band answers near its line or a diamond and nowhere else, so the
  /// marquee and the lane ground keep the rest of the row.
  @override
  bool? hitTest(Offset position) {
    if (!grabbable || points.isEmpty) return false;
    if (held) return true;
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
      old.halo != halo ||
      old.diamond != diamond ||
      old.readout != readout ||
      old.readoutAt != readoutAt ||
      old.held != held ||
      old.grabbable != grabbable ||
      old.keyed != keyed ||
      old.hover != hover;
}
