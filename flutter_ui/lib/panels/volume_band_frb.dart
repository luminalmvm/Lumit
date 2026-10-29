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
// keep their gestures. Dragging a key moves it in value and in time, whole
// frames at a time and never past its neighbours; `Shift` or `Alt` move it
// in time alone, and `Ctrl` moves the value a tenth as far with the time
// held, as the speed line does. Dragging the line between two keys moves
// both, and the line with no keys moves the whole level. Each commits once,
// on release. `Ctrl`-click plants a key on the line (the lane ground's own
// planting gesture, aimed at this row); `Alt`-click lifts one, the graph
// and envelope grammar.
//
// The same band draws Pan when asked ([BandKind.pan]): centre in the middle,
// left at the top and right at the bottom, as Vegas draws it. A double-click
// on a key puts it back to 0 dB or centre, and anywhere else on the line
// plants one. Right-click a key to reset or delete it, or the line for a
// point, the curve of the span under the pointer, Flip and Reset all, as
// Vegas's envelope menu has them. An expression's value or a locked layer is
// not for dragging: the first draws nothing, the second only the line.

import 'package:flutter/foundation.dart' show ValueListenable, listEquals;
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

  /// How far the held key has been carried sideways, in pixels.
  double _dx = 0;

  /// The drag holds a key itself, so it moves in time too.
  bool _timed = false;

  /// The drag holds the line between key [_drag]'s index and the next, and
  /// moves both by the same amount.
  bool _span = false;

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

  /// [v] held inside the band's own range.
  double _clamped(double v) => _pan
      ? v.clamp(-100.0, 100.0)
      : v.clamp(volumeBandFloorDb, volumeBandTopDb);

  /// A key's x, on the comp clock its times cross in, carried along
  /// by a bar move in flight.
  double _xOfKey(BridgeKeyframe key) =>
      widget.axis.xOf(laneKeyFrame(key, widget.fps) + widget.barShift);

  /// The keys as drawn right now, with the drag in flight applied.
  List<BridgeKeyframe> _shown() {
    final keys = _keys;
    final held = _drag;
    if (held == null || held.index == null || keys.isEmpty) return keys;
    return _reshaped(_dragged(keys, held.index!, held.value));
  }

  /// [keys] with the drag applied: key [i] to [value] and carried in time,
  /// or the span from it to the next moved up or down as one.
  List<BridgeKeyframe> _dragged(
      List<BridgeKeyframe> keys, int i, double value) {
    final by = value - _grabbedAt;
    return [
      for (var k = 0; k < keys.length; k++)
        if (k == i)
          _key(keys[k], value: value, time: _timed ? _movedTime(keys, i) : null)
        else if (_span && k == i + 1)
          _key(keys[k], value: _clamped(keys[k].value + by))
        else
          keys[k],
    ];
  }

  /// Key [i]'s time after [_dx] of travel: whole frames, with at least one
  /// frame left between it and each neighbour. Null where it stays put.
  BridgeRational? _movedTime(List<BridgeKeyframe> keys, int i) {
    final perFrame = widget.axis.perFrame;
    if (perFrame <= 0) return null;
    final frames = (_dx / perFrame).round();
    if (frames == 0) return null;
    final fps = widget.fps;
    final lo = i > 0
        ? laneKeyFrame(keys[i - 1], fps) + 1
        : double.negativeInfinity;
    final hi = i + 1 < keys.length
        ? laneKeyFrame(keys[i + 1], fps) - 1
        : double.infinity;
    if (lo > hi) return null;
    final at = (laneKeyFrame(keys[i], fps) + frames).clamp(lo, hi).toDouble();
    return timeOfSubframe(at, widget.fpsNum, widget.fpsDen);
  }

  /// The diamonds: each key's (x, y), or the two ends of the static level.
  List<Offset> _points(List<BridgeKeyframe> keys) {
    if (keys.isEmpty) {
      final y = _y(_drag?.value ?? _staticValue);
      return [Offset(0, y), Offset(widget.axis.width, y)];
    }
    return [
      for (final k in keys) Offset(_xOfKey(k), _y(k.value)),
    ];
  }

  /// The line through [keys], each span sampled along its curve so a fade's
  /// shape shows; a Hold steps at the next key.
  List<Offset> _line(List<BridgeKeyframe> keys, List<Offset> points) {
    if (keys.length < 2) return points;
    const steps = 16;
    final out = <Offset>[];
    for (var i = 0; i + 1 < keys.length; i++) {
      final (a, b) = (points[i], points[i + 1]);
      out.add(a);
      if (keys[i].interpOut is BridgeSideInterp_Hold) {
        out.add(Offset(b.dx, a.dy));
        continue;
      }
      final t0 = rationalSeconds(keys[i].time);
      final t1 = rationalSeconds(keys[i + 1].time);
      for (var s = 1; s < steps; s++) {
        out.add(Offset(a.dx + (b.dx - a.dx) * s / steps,
            _y(evaluateKeys(keys, t0 + (t1 - t0) * s / steps))));
      }
    }
    return out..add(points.last);
  }

  /// The span under [x]: the index of the key it starts at, or null outside
  /// the keys.
  int? _spanAt(double x) {
    final keys = _keys;
    for (var i = 0; i + 1 < keys.length; i++) {
      if (x >= _xOfKey(keys[i]) && x < _xOfKey(keys[i + 1])) return i;
    }
    return null;
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
    final key = keys.isEmpty ? null : _keyNear(local);
    // Off every key, the line between two takes the span, as Vegas does;
    // past the ends it moves the nearer end's level.
    final span = key == null ? _spanAt(local.dx) : null;
    final index = keys.isEmpty ? null : key ?? span ?? _nearestByX(local);
    final at = index == null ? _staticValue : keys[index].value;
    setState(() {
      _grabbedAt = at;
      _grabX = local.dx;
      _travelled = 0;
      _dx = 0;
      _timed = key != null;
      _span = span != null;
      _drag = (index: index, value: at);
    });
  }

  /// The nearest key by x alone, which a press on the line past the first or
  /// last key takes.
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

  void _moveDrag(Offset delta) {
    final held = _drag;
    if (held == null) return;
    final keyboard = HardwareKeyboard.instance;
    // Ctrl held is the fine trim, a tenth of the speed with the time held;
    // Shift or Alt move the key in time alone.
    final fine = keyboard.isControlPressed || keyboard.isMetaPressed;
    final timeOnly = keyboard.isShiftPressed || keyboard.isAltPressed;
    if (!timeOnly) _travelled += fine ? delta.dy / 10 : delta.dy;
    if (!fine) _dx += delta.dx;
    final value = _valueOfY(_y(_grabbedAt) + _travelled);
    setState(() => _drag = (index: held.index, value: value));
  }

  void _commitDrag() {
    final held = _drag;
    final keys = _keys;
    setState(() => _drag = null);
    if (held == null) return;
    if (held.index == null || keys.isEmpty) {
      if (held.value != _grabbedAt) _write(BridgeScalar.static_(held.value));
      return;
    }
    final next = _dragged(keys, held.index!, held.value);
    if (listEquals(next, keys)) return;
    _write(BridgeScalar.keyframed(next));
  }

  /// [keys] with key [index] set to [value], its time and easing kept.
  BridgeScalar _withKey(List<BridgeKeyframe> keys, int index, double value) =>
      BridgeScalar.keyframed([
        for (var i = 0; i < keys.length; i++)
          i == index ? _key(keys[i], value: value) : keys[i],
      ]);

  void _write(BridgeScalar next) {
    // A steep side's speed follows its span, wherever the keys now sit.
    if (next case BridgeScalar_Keyframed(:final field0)) {
      next = BridgeScalar.keyframed(_reshaped(field0));
    }
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
      if (_keyNear(local) case final index?) _lift(keys, index);
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

  /// [keys] without key [index]; lifting the last leaves the level static at
  /// that key's value, which is what the curve read anyway.
  void _lift(List<BridgeKeyframe> keys, int index) {
    final left = [
      for (var i = 0; i < keys.length; i++)
        if (i != index) keys[i],
    ];
    _write(left.isEmpty
        ? BridgeScalar.static_(keys[index].value)
        : BridgeScalar.keyframed(left));
  }

  /// A key planted on the line at [local], at the value the line reads there.
  /// Between two keys both halves keep the span's curve.
  void _plant(Offset local) {
    final keys = _keys;
    final frame = widget.axis.frameAtExact(local.dx) - widget.barShift;
    final time = timeOfSubframe(frame, widget.fpsNum, widget.fpsDen);
    final at = rationalSeconds(time);
    final span = keys.indexWhere((k) => rationalSeconds(k.time) > at) - 1;
    const linear = BridgeSideInterp.linear();
    final planted = BridgeKeyframe(
      time: time,
      value: keys.isEmpty ? _staticValue : evaluateKeys(keys, at),
      interpIn: span >= 0 ? keys[span + 1].interpIn : linear,
      interpOut: span >= 0 ? keys[span].interpOut : linear,
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

  /// Right-click a key to reset or delete it, or the line for a point, the
  /// span's curve, Flip and Reset all.
  Future<void> _menu(Offset local, Offset global) async {
    final keys = _keys;
    final key = keys.isEmpty ? null : _keyNear(local);
    final span = key == null ? _spanAt(local.dx) : null;
    final current = span == null ? null : _curveOf(keys, span);
    final names = {
      SpeedCurve.linear: l10n.speedCurveLinear,
      SpeedCurve.fast: l10n.speedCurveFast,
      SpeedCurve.slow: l10n.speedCurveSlow,
      SpeedCurve.smooth: l10n.speedCurveSmooth,
      SpeedCurve.sharp: l10n.speedCurveSharp,
      SpeedCurve.hold: l10n.speedCurveHold,
    };
    final picked = await showMenuAt<Object>(
      context: context,
      position: global,
      width: 170,
      rows: (close) => [
        if (key != null) ...[
          MenuRow(
              key: const ValueKey('band-reset-point'),
              onPressed: () => close('reset'),
              child: Text(l10n.envelopePointReset)),
          MenuRow(
              key: const ValueKey('band-delete-point'),
              onPressed: () => close('delete'),
              child: Text(l10n.delete)),
        ] else ...[
          MenuRow(
              key: const ValueKey('band-add-point'),
              onPressed: () => close('add'),
              child: Text(l10n.speedPointAdd)),
          if (span != null)
            for (final MapEntry(key: curve, value: name) in names.entries)
              MenuRow.option(
                key: ValueKey<String>('band-curve-${curve.name}'),
                onPressed: () => close(curve),
                child: Row(children: [
                  menuTick(curve == current),
                  Expanded(child: Text(name)),
                ]),
              ),
          MenuRow(
              key: const ValueKey('band-flip'),
              onPressed: () => close('flip'),
              child: Text(l10n.envelopeFlipAll)),
          MenuRow(
              key: const ValueKey('band-reset-all'),
              onPressed: () => close('reset-all'),
              child: Text(l10n.envelopeResetAll)),
        ],
      ],
    );
    // The document moved on while the menu was up: its rows no longer mean
    // these keys.
    if (!mounted || picked == null || !listEquals(_keys, keys)) return;
    switch (picked) {
      case 'reset':
        if (keys[key!].value != 0) _write(_withKey(keys, key, 0));
      case 'delete':
        _lift(keys, key!);
      case 'add':
        _plant(local);
      case 'flip':
        // Volume mirrors about 0 dB inside the band; pan swaps sides.
        _write(keys.isEmpty
            ? BridgeScalar.static_(_clamped(-_staticValue))
            : BridgeScalar.keyframed(
                [for (final k in keys) _key(k, value: _clamped(-k.value))]));
      case 'reset-all':
        if (keys.isNotEmpty || _staticValue != 0) {
          _write(const BridgeScalar.static_(0));
        }
      case final SpeedCurve curve:
        _write(BridgeScalar.keyframed(_withCurve(keys, span!, curve)));
    }
  }

  @override
  Widget build(BuildContext context) {
    // An expression drives the value: nothing here may write over it.
    if (_scalar is BridgeScalar_Expression) {
      return SizedBox(width: widget.axis.width, height: widget.rowHeight);
    }
    final t = ThemeScope.of(context).theme;
    final shown = _shown();
    final points = _points(shown);
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
        // Claimed here, so the lane's own right-click under the band only
        // hears presses away from the line.
        onSecondaryTapDown: (d) => _menu(d.localPosition, d.globalPosition),
        onPanStart: (d) => _startDrag(d.localPosition),
        onPanUpdate: (d) => _moveDrag(d.delta),
        onPanEnd: (_) => _commitDrag(),
        onPanCancel: () => setState(() => _drag = null),
        child: CustomPaint(
          size: Size(widget.axis.width, widget.rowHeight),
          painter: VolumeBandPainter(
            points: points,
            curve: _line(shown, points),
            // Pan in the warning tone, so the two lines on one lane never
            // read as each other.
            line: _pan ? t.warning : t.accent,
            halo: t.surface0.withValues(alpha: 0.75),
            diamond: t.animated,
            readout: held == null ? null : _readoutText(held.value),
            readoutAt: held == null
                ? null
                : Offset(
                    held.index == null || _span
                        ? _grabX
                        : points[held.index!].dx,
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

/// [k] with whichever of its parts are given replaced.
BridgeKeyframe _key(BridgeKeyframe k,
        {double? value,
        BridgeRational? time,
        BridgeSideInterp? interpIn,
        BridgeSideInterp? interpOut}) =>
    BridgeKeyframe(
      time: time ?? k.time,
      value: value ?? k.value,
      interpIn: interpIn ?? k.interpIn,
      interpOut: interpOut ?? k.interpOut,
    );

// The fade curves on a plain value line, by Vegas's names. A side is flat
// (speed 0 at a third, the easy ease) or steep (reaching the far key's value
// at its control point), and the pair names the curve: Smooth flat both
// ends, Sharp steep both, Fast steep out of the first key, Slow steep into
// the second. A steep side's speed depends on the span, so [_reshaped] works
// it out again on every write.

/// The reach that marks a steep side. Beside a third so it can be read back.
// ponytail: a side eased elsewhere to exactly this reach reads as steep; give
// the curve a field of its own on the key if that is ever met.
const double _steepReach = 0.34;

bool _isSteep(BridgeSideInterp side) =>
    side is BridgeSideInterp_Bezier &&
    (side.field0.influence - _steepReach).abs() < 1e-3;

bool _isFlat(BridgeSideInterp side) =>
    side is BridgeSideInterp_Bezier &&
    side.field0.speed == 0 &&
    (side.field0.influence - 1 / 3).abs() < 1e-3;

/// The curve span [span] of [keys] is in, or null when it was eased some
/// other way.
SpeedCurve? _curveOf(List<BridgeKeyframe> keys, int span) {
  final out = keys[span].interpOut, into = keys[span + 1].interpIn;
  if (out is BridgeSideInterp_Hold) return SpeedCurve.hold;
  if (out is BridgeSideInterp_Linear && into is BridgeSideInterp_Linear) {
    return SpeedCurve.linear;
  }
  return switch ((
    _isSteep(out) || _isFlat(out) ? _isSteep(out) : null,
    _isSteep(into) || _isFlat(into) ? _isSteep(into) : null,
  )) {
    (false, false) => SpeedCurve.smooth,
    (true, true) => SpeedCurve.sharp,
    (true, false) => SpeedCurve.fast,
    (false, true) => SpeedCurve.slow,
    _ => null,
  };
}

/// [keys] with span [span] in [curve].
List<BridgeKeyframe> _withCurve(
    List<BridgeKeyframe> keys, int span, SpeedCurve curve) {
  const linear = BridgeSideInterp.linear();
  const steep = BridgeSideInterp.bezier(
      BridgeBezierSide(speed: 0, influence: _steepReach));
  final (out, into) = switch (curve) {
    SpeedCurve.linear => (linear, linear),
    SpeedCurve.hold => (const BridgeSideInterp.hold(), linear),
    SpeedCurve.smooth => (easyEase, easyEase),
    SpeedCurve.sharp => (steep, steep),
    SpeedCurve.fast => (steep, easyEase),
    SpeedCurve.slow => (easyEase, steep),
  };
  return _reshaped([
    for (var i = 0; i < keys.length; i++)
      if (i == span)
        _key(keys[i], interpOut: out)
      else if (i == span + 1)
        _key(keys[i], interpIn: into)
      else
        keys[i],
  ]);
}

/// [keys] with every steep side's speed set from its span, so the side
/// reaches the far key's value at its control point.
List<BridgeKeyframe> _reshaped(List<BridgeKeyframe> keys) {
  BridgeSideInterp side(BridgeSideInterp s, int span) {
    if (!_isSteep(s) || span < 0 || span + 1 >= keys.length) return s;
    final dt = rationalSeconds(keys[span + 1].time) -
        rationalSeconds(keys[span].time);
    final chord = dt > 0 ? (keys[span + 1].value - keys[span].value) / dt : 0.0;
    return BridgeSideInterp.bezier(
        BridgeBezierSide(speed: chord / _steepReach, influence: _steepReach));
  }

  return [
    for (var i = 0; i < keys.length; i++)
      _key(keys[i],
          interpIn: side(keys[i].interpIn, i - 1),
          interpOut: side(keys[i].interpOut, i)),
  ];
}

/// The band's line, its diamonds, and the readout — and the hit test that
/// keeps the pointer's claim to the lane honest: only within reach of the
/// line or a diamond does the band answer at all.
class VolumeBandPainter extends CustomPainter {
  final List<Offset> points;

  /// The line itself, through [points] and along each span's curve.
  final List<Offset> curve;
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
    required this.curve,
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
    if (points.isEmpty || curve.isEmpty) return;
    final over = hover?.value;
    final width = over != null ? 2.2 : 1.6;
    final stroke = Paint()
      ..color = line
      ..strokeWidth = width
      ..style = PaintingStyle.stroke;
    final path = Path()..moveTo(0, curve.first.dy);
    for (final p in curve) {
      path.lineTo(p.dx, p.dy);
    }
    path.lineTo(size.width, curve.last.dy);
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
    if (!grabbable || points.isEmpty || curve.isEmpty) return false;
    if (held) return true;
    for (final p in points) {
      if ((p - position).distance <= 7) return true;
    }
    // Against the segment run, x-clamped the way the drawn line extends.
    var previous = Offset(0, curve.first.dy);
    for (final p in [...curve, Offset(double.infinity, curve.last.dy)]) {
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
      old.curve != curve ||
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
