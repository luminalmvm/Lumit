// The speed envelope: a Sequence layer's clips drawn as their speed in per
// cent, the editor under the sequence view.
//
// The envelope is the same editor as the graph's Vegas lens, over the same
// keyframes: a point per key, its height the playback speed in per cent, and
// between two points the curve the span was given (Linear, Fast, Slow,
// Smooth, Sharp or Hold, from the right-click menu on the line).
// `Ctrl`-click or double-click the line plants a point; `Alt`-click lifts
// one. Double-click a point to put it back to 100%; right-click it to type its
// speed. `Ctrl` held through a drag moves the point a tenth as far and keeps
// it at its time; `Shift` or `Alt` move it in time alone. A clip that has
// never been retimed draws the flat 100% it is playing at, and the first edit
// gives it a real map.

import 'package:flutter/gestures.dart' show kSecondaryMouseButton;
import 'package:flutter/services.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_key_fields.dart';
import 'graph_maths.dart';
import 'layer_fold_frb.dart';
import 'timeline_extras_frb.dart';

/// The speed envelope: every clip's map drawn as points and straight lines,
/// against an axis that grows to hold whatever the curves reach.
class SpeedEnvelope extends StatefulWidget {
  final BridgeLayerEntry entry;
  final CompositionReference? comp;
  final TimelineAxis axis;
  final ScrollController? hScroll;
  final double fps;
  final int fpsNum;
  final int fpsDen;
  final VoidCallback onChanged;
  final void Function(BridgeClip clip, List<BridgeKeyframe> keys)? onPreview;

  /// Reports a click and answers whether it was the second of a double.
  final bool Function() onTapped;

  /// Laid over the clips themselves, as Vegas draws them: only the lines and
  /// their points take the pointer, and everything else falls through to the
  /// clip underneath.
  final bool onClip;

  /// Which clips draw a line, where not every one does.
  final bool Function(BridgeClip clip)? shows;

  const SpeedEnvelope({
    super.key,
    required this.entry,
    this.comp,
    required this.axis,
    this.hScroll,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    required this.onChanged,
    this.onPreview,
    required this.onTapped,
    this.onClip = false,
    this.shows,
  });

  @override
  State<SpeedEnvelope> createState() => SpeedEnvelopeState();
}

class SpeedEnvelopeState extends State<SpeedEnvelope> {
  /// The point under the pointer while a drag runs: which clip, which key,
  /// the speed it is being asked for, and how far it has been carried in
  /// time. A point moves both ways — *when* a ramp reaches a speed matters as
  /// much as what the speed is. `fine` is whether `Ctrl` had the last move at
  /// a tenth of the pointer's rate, which the readout answers with a decimal.
  ({BridgeClip clip, int index, double speed, double dx, bool fine})? _drag;

  /// The point whose speed is being typed, as `clipId#index`, so it stays lit
  /// for as long as its box is open.
  String? _typing;

  /// A right-button press, which opens a point's menu and is no part of a
  /// click or a drag.
  bool _secondary = false;

  /// The speed at which each clip shows every frame of its footage once, by
  /// clip id, for the clips whose footage runs faster than the comp. A null
  /// entry is a clip already asked about.
  final Map<String, double?> _everyFrame = {};

  /// Each footage item's probe, by item id, so a row cut from one file asks
  /// about that file once.
  final Map<String, Future<BridgeMediaInfo?>> _probes = {};

  @override
  void initState() {
    super.initState();
    _probeRates();
  }

  @override
  void didUpdateWidget(SpeedEnvelope oldWidget) {
    super.didUpdateWidget(oldWidget);
    _probeRates();
  }

  /// Ask what rate each new clip's footage runs at. Never from the build, and
  /// once per clip: a clip already asked about costs a map lookup.
  // ponytail: one crossing per clip as the view opens, and the answer is kept
  // by clip id while it stays open. Carry the footage's rate on the clip's
  // read model if a long row opens slowly or a replaced file keeps its guide.
  void _probeRates() {
    final comp = widget.comp;
    if (comp == null) return;
    for (final clip in _clips) {
      final id = clip.id.toString();
      if (clip.sourceIsComp || _everyFrame.containsKey(id)) continue;
      _everyFrame[id] = null;
      final FootageReference? footage;
      try {
        // The clip showing at its own first frame is the clip itself.
        footage = comp
            .cutMatchFrame(frame: clip.startFrame, layer: widget.entry.layer)
            ?.footage;
      } catch (_) {
        continue;
      }
      if (footage == null) continue;
      (_probes[footage.internalid.toString()] ??= footage.mediaInfo())
          .then((info) {
        if (!mounted || info == null || info.fpsNum <= 0 || info.fpsDen <= 0) {
          return;
        }
        final speed = everyFrameSpeed(widget.fps, info.fpsNum / info.fpsDen);
        if (speed != null) setState(() => _everyFrame[id] = speed);
      });
    }
  }

  /// The speed the grabbed point had when the gesture began, and how far the
  /// pointer has travelled since.
  ///
  /// **The drag is relative, not absolute.** Reading the speed straight off
  /// the pointer's height teleports the point to the cursor the instant it is
  /// grabbed — the whole clip's width is the grab target, so the pointer is
  /// rarely at the point's own height — and then the change over a gesture is
  /// not proportional to the travel at all. Travel in, travel out.
  double _grabbedAt = 0;
  double _travelled = 0;

  /// Where the press landed, and whether it has moved since — a press that
  /// never moves is a click, and the two are told apart here because raw
  /// pointer events do not do it for you.
  Offset? _downAt;
  bool _moving = false;

  /// The selection box in flight, and the points it has caught, as
  /// `clipId#index`.
  ///
  /// A press begins a box unless it lands **on a line** — within reach of the
  /// curve at that x, which is a generous target and the one gesture that
  /// obviously means "take hold of this". Everywhere else in the strip is
  /// empty space, so it is free to select across.
  Rect? _box;
  final Set<String> _selected = {};

  /// How near a clip's own line counts as grabbing it rather than starting a
  /// selection box.
  static const double _lineGrab = 10;

  List<BridgeClip> get _clips => switch (widget.shows) {
        final shows? => widget.entry.info.clips.where(shows).toList(),
        null => widget.entry.info.clips,
      };

  /// A clip's envelope keys — the map it actually plays by, which the engine
  /// hands over whether or not the clip has one of its own.
  ///
  /// Nothing is constructed here on purpose. This used to fabricate the flat
  /// line for an un-retimed clip and started it at source **zero**, which is
  /// true only of a clip nobody has cut: every clip after a cut begins part
  /// way into its media, so ramping one sent it back to the top of the file
  /// and it played the same frame or two throughout.
  List<BridgeKeyframe> _keysOf(BridgeClip clip) => keysOf(clip.retime);

  /// The axis as it was when the drag began, held for the whole gesture.
  ///
  /// **Without this the drag runs away.** The range grows to hold whatever a
  /// point reaches, so a point dragged past the floor widened the axis, which
  /// stretched what the next pixel of travel was worth, which pushed the point
  /// further still — the value ran off exponentially for a steady hand. A
  /// gesture has to keep the scale it started on; the axis reframes when the
  /// pointer is let go.
  (double, double)? _frozen;

  /// The range the strip draws over: the documented default, grown to hold
  /// every point on every clip.
  (double, double) get _range {
    final frozen = _frozen;
    if (frozen != null) return frozen;
    var (lo, hi) = fitEnvelopeRange([for (final c in _clips) _keysOf(c)]);
    // A little air past whatever the curves reach, so a point at the extreme
    // is not drawn half outside its own row. The *default* range carries its
    // own headroom, so an ordinary clip needs none added.
    const air = 8.0;
    var (dlo, dhi) = envelopeDefaultRange;
    // Over a clip, 100% runs through the middle, clear of the clip's name
    // and buttons along its top, with room to ramp either way.
    if (widget.onClip) {
      (dlo, dhi) = (-50.0, 250.0);
      lo = lo < dlo ? lo : dlo;
      hi = hi > dhi ? hi : dhi;
    }
    return (lo < dlo ? lo - air : dlo, hi > dhi ? hi + air : dhi);
  }

  double _y(double speed, double height) {
    final (lo, hi) = _range;
    final span = (hi - lo).abs() < 1e-9 ? 1.0 : hi - lo;
    return height - (speed - lo) / span * height;
  }

  /// Where a clip's key sits on the comp's own clock, in x pixels.
  double _xOfKey(BridgeClip clip, BridgeKeyframe key) =>
      widget.axis.xOf(clip.startFrame.toInt() +
          (rationalSeconds(key.time) * widget.fps).round());

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(
      builder: (context, box) {
        final height = box.maxHeight;
        // **Every child keyed.** Flutter matches a Stack's children by
        // position when they are not, so the readout appearing mid-drag took
        // the slot before the gesture detector and rebuilt its element from
        // scratch — taking with it the recogniser holding the drag, which
        // ended the gesture the instant the readout showed up. The same trap
        // that caught the trimmed-layer ghost.
        return Stack(
          children: [
            Positioned.fill(
              key: const ValueKey('seq-envelope-curves'),
              child: IgnorePointer(
                // Clipped to the strip's own bounds. The axis is frozen for
                // the length of a gesture, so a point dragged past the top or
                // the bottom would otherwise be drawn outside this row and
                // over the layers below it — the picture reframes when the
                // pointer is let go and the freeze lifts.
                child: ClipRect(
                  child: CustomPaint(
                    painter: _SpeedEnvelopePainter(
                      lanes: [
                        for (final c in _clips)
                          (
                            clip: c,
                            keys: _shown(c),
                          ),
                      ],
                      xOfKey: _xOfKey,
                      y: (s) => _y(s, height),
                      chosen: t.accent,
                      selected: _selected,
                      typing: _typing,
                      lit: t.textPrimary,
                      guides: {
                        for (final c in _clips)
                          if (_everyFrame[c.id.toString()] case final speed?)
                            c.id.toString(): speed,
                      },
                      guideLabel: (speed) => l10n.speedGuideEveryFrame(
                          '${guideSpeedText(speed)}${l10n.unitSymbolPercent}'),
                      guide: t.textMuted,
                      // The point in hand has landed on its guide.
                      caught: switch (_drag) {
                        final held?
                            when held.speed ==
                                _everyFrame[held.clip.id.toString()] =>
                          held.clip.id.toString(),
                        _ => null,
                      },
                      range: _range,
                      line: t.hairline,
                      curve: t.curve.first,
                      // A halo in the ground's colour, so the words keep
                      // their edge where the curve runs through them.
                      label: t.small.copyWith(color: t.textMuted, shadows: [
                        for (final blur in const [2.0, 4.0])
                          Shadow(color: t.surface1, blurRadius: blur),
                      ]),
                      viewportLeft: (widget.hScroll?.hasClients ?? false)
                          ? widget.hScroll!.offset
                          : 0,
                      bare: widget.onClip,
                    ),
                  ),
                ),
              ),
            ),
            // What the drag is setting, beside the point it has hold of — a
            // speed is a number you are aiming at, and reading it off the
            // height of a dot against an axis that reframes as you drag is
            // not aiming.
            if (_drag case final held?) _readout(t, held, height),
            if (_box case final box?)
              Positioned(
                key: const ValueKey('seq-envelope-box'),
                left: box.left,
                top: box.top,
                width: box.width,
                height: box.height,
                child: IgnorePointer(
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: t.accent.withValues(alpha: 0.12),
                      border: Border.all(color: t.accent),
                    ),
                  ),
                ),
              ),
            // Last, so it is on top, and keyed, so it keeps its element
            // however the children above it come and go.
            // The line itself: click to plant a point, drag one to re-speed.
            Positioned.fill(
              key: const ValueKey('seq-envelope-gestures'),
              // **Raw pointer events, not a gesture.** A point moves in both
              // directions, and a pan recogniser that wants both axes is in
              // the arena against the lane's vertical scroll *and* its
              // horizontal one — which win, so the drag died the moment it
              // had travelled far enough for them to claim it and had to be
              // started again. A `Listener` is not in the arena at all, so
              // the first drag is the one that works.
              child: _HitWhere(
                test: (local) =>
                    !widget.onClip ||
                    _drag != null ||
                    _pointAt(local, height) != null ||
                    _onALine(local, height),
                // Over a clip, the line is found by the cursor changing.
                child: MouseRegion(
                  cursor: widget.onClip
                      ? SystemMouseCursors.resizeUpDown
                      : MouseCursor.defer,
                  child: Listener(
                    key: const ValueKey('seq-envelope'),
                    behavior: HitTestBehavior.opaque,
                    onPointerDown: (e) {
                      if (e.buttons & kSecondaryMouseButton != 0) {
                        _secondary = true;
                        _pointMenu(e.localPosition, e.position, height);
                        return;
                      }
                      _downAt = e.localPosition;
                      if (_onALine(e.localPosition, height)) {
                        _startDrag(e.localPosition, height);
                      } else {
                        setState(() => _box =
                            Rect.fromPoints(e.localPosition, e.localPosition));
                      }
                    },
                    onPointerMove: (e) => setState(() {
                      final from = _downAt;
                      if (_box != null && from != null) {
                        _moving = true;
                        _box = Rect.fromPoints(from, e.localPosition);
                        return;
                      }
                      final held = _drag;
                      if (held == null) return;
                      _moving = true;
                      // `Ctrl` is the value well's own fine rung, a tenth of the
                      // pointer's rate, with the point held to its time. Taken
                      // off each move as it is added, so pressing or letting go
                      // mid-drag changes what the next pixel is worth and moves
                      // nothing. `Shift` or `Alt` move it in time alone.
                      final keys = HardwareKeyboard.instance;
                      final fine = keys.isControlPressed;
                      final timeOnly = keys.isShiftPressed || keys.isAltPressed;
                      if (!timeOnly) _travelled += e.delta.dy * (fine ? 0.1 : 1);
                      final (lo, hi) = _range;
                      final span = (hi - lo).abs() < 1e-9 ? 1.0 : hi - lo;
                      final perPixel = span / (height <= 0 ? 1 : height);
                      // Down is slower: the axis runs fast at the top.
                      final asked = _grabbedAt - _travelled * perPixel;
                      _drag = (
                        clip: held.clip,
                        index: held.index,
                        // Onto the every-frame guide when it comes near, which a
                        // fine drag is too exact to want.
                        speed: fine
                            ? asked
                            : snapEnvelopeSpeed(
                                asked,
                                _everyFrame[held.clip.id.toString()],
                                envelopeSnapReach * perPixel),
                        dx: fine ? held.dx : held.dx + e.delta.dx,
                        fine: fine,
                      );
                      // The picture follows the point. A retime decides *which*
                      // frame is decoded, so this is the one drag where nothing
                      // moves on screen until it is asked to.
                      widget.onPreview?.call(held.clip, _shown(held.clip));
                    }),
                    onPointerUp: (e) {
                      if (_secondary) {
                        _secondary = false;
                        return;
                      }
                      setState(() => _frozen = null);
                      final box = _box;
                      if (box != null) {
                        setState(() {
                          _box = null;
                          if (_moving) _catch(box, height);
                        });
                      } else if (!_moving) {
                        // A press that never moved is a click: plant a point, lift
                        // one, or nothing at all.
                        setState(() => _drag = null);
                        _tap(_downAt ?? e.localPosition, height);
                      } else {
                        _commit();
                      }
                      _moving = false;
                      _downAt = null;
                    },
                    onPointerCancel: (_) => setState(() {
                      _drag = null;
                      _moving = false;
                      _frozen = null;
                      _box = null;
                      _secondary = false;
                    }),
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  /// The speed the drag is asking for, floating beside the point.
  Widget _readout(
      LumitTheme t,
      ({BridgeClip clip, int index, double speed, double dx, bool fine}) held,
      double height) {
    final keys = _shown(held.clip);
    final index = held.index.clamp(0, keys.length - 1);
    final x = _xOfKey(held.clip, keys[index]);
    final y = _y(envelopeSpeeds(keys)[index], height);
    return Positioned(
      key: const ValueKey('seq-envelope-readout'),
      left: x + 8,
      top: (y - 9).clamp(0.0, height - 18),
      child: IgnorePointer(
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
          decoration: BoxDecoration(
            color: t.surface3,
            border: Border.all(color: t.hairline),
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          child: Text(
              '${speedText(held.speed, fine: held.fine)}'
              '${l10n.unitSymbolPercent}',
              style: t.small.copyWith(color: t.textPrimary)),
        ),
      ),
    );
  }

  /// A clip's keys with the drag in flight applied, so the line follows the
  /// pointer rather than jumping on release.
  List<BridgeKeyframe> _shown(BridgeClip clip) {
    final keys = _keysOf(clip);
    final held = _drag;
    if (held == null) return keys;
    if (held.clip.id != clip.id) {
      // Another clip the selection reaches into: its caught points move too.
      return _selected.any((s) => s.startsWith('${clip.id}#'))
          ? _withSpeed(clip, keys, held.index, held.speed)
          : keys;
    }
    return _moved(clip, _withSpeed(clip, keys, held.index, held.speed),
        held.index, held.dx);
  }

  /// [keys] with the dragged one carried along in time, snapped to whole
  /// frames and held strictly between its neighbours.
  ///
  /// The two ends stay put: they are the clip's own edges, and a clip's
  /// length is trimmed on the clip, never on its speed curve.
  List<BridgeKeyframe> _moved(
      BridgeClip clip, List<BridgeKeyframe> keys, int index, double dx) {
    if (index <= 0 || index >= keys.length - 1) return keys;
    final perFrame = widget.axis.perFrame;
    if (perFrame <= 0) return keys;
    final frames = (dx / perFrame).round();
    if (frames == 0) return keys;
    final fps = widget.fps <= 0 ? 1.0 : widget.fps;
    final wanted = rationalSeconds(keys[index].time) + frames / fps;
    // One frame of daylight either side, so two keys can never share a time.
    final lo = rationalSeconds(keys[index - 1].time) + 1 / fps;
    final hi = rationalSeconds(keys[index + 1].time) - 1 / fps;
    if (lo >= hi) return keys;
    final at = wanted.clamp(lo, hi);
    // Through `moveEnvelopePoint`, which re-integrates: keeping the stored
    // tangents while the span's length changes is what bent a straight line.
    return moveEnvelopePoint(
        keys, index, timeOfSubframe(at * fps, widget.fpsNum, widget.fpsDen));
  }

  /// [keys] with the drag applied — one point, or the whole line.
  ///
  /// **A clip nobody has retimed moves as one level.** Its envelope is the two
  /// implied ends of a flat 100%, and dragging one of those alone would tilt
  /// the line into a ramp nobody asked for: the obvious reading of dragging a
  /// flat line is "this clip plays at that speed", which is also what Vegas's
  /// first envelope point does. Plant a point and the line has a shape worth
  /// keeping, so from then on a drag moves only the point it has hold of.
  List<BridgeKeyframe> _withSpeed(
      BridgeClip clip, List<BridgeKeyframe> keys, int index, double speed) {
    if (!clip.retimed) {
      return envelopeToKeys(keys, [for (final _ in keys) speed]);
    }
    // A caught set moves together, by the same amount the dragged point
    // moved. Its own points keep whatever spread they had — a selection is
    // for shifting a shape, not for flattening it.
    final held = _drag;
    if (held != null && _selected.contains('${held.clip.id}#${held.index}')) {
      final by = speed - _grabbedAt;
      final speeds = envelopeSpeeds(keys);
      var touched = false;
      for (var i = 0; i < speeds.length; i++) {
        if (_selected.contains('${clip.id}#$i')) {
          speeds[i] += by;
          touched = true;
        }
      }
      if (touched) return envelopeToKeys(keys, speeds);
    }
    return setEnvelopeSpeed(keys, index, speed);
  }

  /// Whether [local] is near enough to a clip's own line to mean "take hold
  /// of this" rather than "start a box here".
  ///
  /// Measured against the line at that x, not against the nearest point: a
  /// clip's line is what the eye follows, and asking for a direct hit on a
  /// 7px dot would make re-speeding a test of aim.
  bool _onALine(Offset local, double height) {
    final clip = _clipAt(local.dx);
    if (clip == null) return false;
    final keys = _keysOf(clip);
    final speeds = envelopeSpeeds(keys);
    // Tighter over a clip, where everything else under the line is a clip
    // to take hold of.
    final reach = widget.onClip ? _lineGrab / 2 : _lineGrab;
    for (var i = 0; i + 1 < keys.length; i++) {
      final x0 = _xOfKey(clip, keys[i]);
      final x1 = _xOfKey(clip, keys[i + 1]);
      if (local.dx < x0 - _lineGrab || local.dx > x1 + _lineGrab) continue;
      final f = x1 > x0 ? ((local.dx - x0) / (x1 - x0)).clamp(0.0, 1.0) : 0.0;
      final t0 = rationalSeconds(keys[i].time);
      final t1 = rationalSeconds(keys[i + 1].time);
      final speed = f <= 0 || f >= 1
          ? speeds[f <= 0 ? i : i + 1]
          : evaluateKeysSpeed(keys, t0 + (t1 - t0) * f) * 100;
      if ((local.dy - _y(speed, height)).abs() <= reach) return true;
    }
    return false;
  }

  /// Take every point inside [box] into the selection. `Shift` adds to what
  /// was already there; a plain box replaces it, and an empty one clears.
  void _catch(Rect box, double height) {
    if (!HardwareKeyboard.instance.isShiftPressed) _selected.clear();
    for (final clip in _clips) {
      final keys = _keysOf(clip);
      final speeds = envelopeSpeeds(keys);
      for (var i = 0; i < keys.length; i++) {
        final at = Offset(_xOfKey(clip, keys[i]), _y(speeds[i], height));
        if (box.contains(at)) _selected.add('${clip.id}#$i');
      }
    }
  }

  /// The envelope point [local] means: the nearest one within reach, or —
  /// failing that — the nearest point of whichever clip the pointer is over.
  ///
  /// The fallback is what makes the band usable. A point is a 7px dot on a
  /// 2px line; asking for a direct hit on one would make re-speeding a clip a
  /// test of aim, when the obvious reading of "drag anywhere on this clip's
  /// line" is never ambiguous — a clip's points are its own.
  ({BridgeClip clip, int index})? _nearestPoint(Offset local, double height) {
    ({BridgeClip clip, int index})? best;
    var bestD = 14.0;
    for (final clip in _clips) {
      final keys = _keysOf(clip);
      final speeds = envelopeSpeeds(keys);
      for (var i = 0; i < keys.length; i++) {
        final p = Offset(_xOfKey(clip, keys[i]), _y(speeds[i], height));
        final d = (p - local).distance;
        if (d < bestD) {
          bestD = d;
          best = (clip: clip, index: i);
        }
      }
    }
    if (best != null) return best;

    final over = _clipAt(local.dx);
    if (over == null) return null;
    final keys = _keysOf(over);
    var nearest = 0;
    var nearestD = double.infinity;
    for (var i = 0; i < keys.length; i++) {
      final d = (_xOfKey(over, keys[i]) - local.dx).abs();
      if (d < nearestD) {
        nearestD = d;
        nearest = i;
      }
    }
    return (clip: over, index: nearest);
  }

  /// The clip whose span covers [x] pixels.
  BridgeClip? _clipAt(double x) {
    for (final c in _clips) {
      final left = widget.axis.xOf(c.startFrame.toInt());
      final right = widget.axis.xOf(c.endFrame.toInt());
      if (x >= left && x < right) return c;
    }
    return null;
  }

  void _startDrag(Offset local, double height) {
    final found = _nearestPoint(local, height);
    if (found == null) return;
    final at = envelopeSpeeds(_keysOf(found.clip))[found.index];
    setState(() {
      _frozen = _range;
      _grabbedAt = at;
      _travelled = 0;
      _drag = (
        clip: found.clip,
        index: found.index,
        speed: at,
        dx: 0,
        fine: false,
      );
    });
  }

  /// The point [local] is on, or null when it is on none: the nearest one in
  /// reach, without the fallback a drag takes.
  ({BridgeClip clip, int index})? _pointAt(Offset local, double height) {
    final found = _nearestPoint(local, height);
    if (found == null) return null;
    final keys = _keysOf(found.clip);
    // A clip whose map is an expression has no points to be on.
    if (found.index >= keys.length) return null;
    final at = Offset(_xOfKey(found.clip, keys[found.index]),
        _y(envelopeSpeeds(keys)[found.index], height));
    return (at - local).distance < 14 ? found : null;
  }

  /// Right-click a point for what can be done to it, or the line between two
  /// for the curve it takes, as Vegas's envelope menu has them.
  Future<void> _pointMenu(Offset local, Offset global, double height) async {
    final found = _pointAt(local, height);
    if (found == null) {
      _lineMenu(local, global, height);
      return;
    }
    final inner =
        found.index > 0 && found.index < _keysOf(found.clip).length - 1;
    final picked = await showMenuAt<String>(
      context: context,
      position: global,
      rows: (close) => [
        MenuRow(
            key: const ValueKey('seq-envelope-set-speed'),
            onPressed: () => close('speed'),
            child: Text(l10n.menuSetSpeed)),
        MenuRow(
            key: const ValueKey('seq-envelope-reset-point'),
            onPressed: () => close('reset'),
            child: Text(l10n.speedPointReset)),
        if (inner)
          MenuRow(
              key: const ValueKey('seq-envelope-delete-point'),
              onPressed: () => close('delete'),
              child: Text(l10n.speedPointDelete)),
      ],
    );
    if (!mounted || picked == null) return;
    switch (picked) {
      case 'speed':
        _typeSpeed(found.clip, found.index, height);
      case 'reset':
        _resetPoint(found.clip, found.index);
      case 'delete':
        final all = _keysOf(found.clip);
        _write(found.clip, [
          for (var i = 0; i < all.length; i++)
            if (i != found.index) all[i],
        ]);
    }
  }

  /// The menu on the line between two points: plant one there, or choose
  /// the curve the speed takes across it.
  Future<void> _lineMenu(Offset local, Offset global, double height) async {
    if (!_onALine(local, height)) return;
    final clip = _clipAt(local.dx);
    if (clip == null) return;
    final keys = _keysOf(clip);
    final at = (widget.axis.frameAt(local.dx) - clip.startFrame) / widget.fps;
    var span = 0;
    while (span + 2 < keys.length &&
        at >= rationalSeconds(keys[span + 1].time)) {
      span++;
    }
    if (span + 1 >= keys.length) return;
    final current = speedCurveOf(keys[span].interpOut);
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
        MenuRow(
            key: const ValueKey('seq-envelope-add-point'),
            onPressed: () => close('add'),
            child: Text(l10n.speedPointAdd)),
        for (final MapEntry(key: curve, value: name) in names.entries)
          MenuRow.option(
            key: ValueKey<String>('seq-envelope-curve-${curve.name}'),
            onPressed: () => close(curve),
            child: Row(children: [
              menuTick(curve == current),
              Expanded(child: Text(name)),
            ]),
          ),
        MenuRow(
            key: const ValueKey('seq-envelope-flip'),
            onPressed: () => close('flip'),
            child: Text(l10n.speedFlip)),
        MenuRow(
            key: const ValueKey('seq-envelope-thin'),
            onPressed: () => close('thin'),
            child: Text(l10n.speedThin)),
        MenuRow(
            key: const ValueKey('seq-envelope-reset-clip'),
            onPressed: () => close('reset'),
            child: Text(l10n.clipResetSpeed)),
      ],
    );
    if (!mounted || picked == null) return;
    switch (picked) {
      case 'add':
        _plant(local);
      case 'flip':
        _write(clip, flipEnvelope(keys));
      case 'thin':
        _write(clip, thinEnvelope(keys));
      case 'reset':
        _write(clip, envelopeToKeys(keys, [for (final _ in keys) 100.0],
            curves: [for (var i = 1; i < keys.length; i++) SpeedCurve.linear]));
      case final SpeedCurve curve:
        _write(clip, setEnvelopeCurve(keys, span, curve));
    }
  }

  /// One point back to 100%: the whole clip when it has never been retimed.
  void _resetPoint(BridgeClip clip, int index) {
    final keys = _keysOf(clip);
    if (index < 0 || index >= keys.length) return;
    _write(clip, _withSpeed(clip, keys, index, 100));
  }

  /// Open the box that types one point's speed, beside the point.
  ///
  /// The number goes where a drag's goes, through [_withSpeed] and [_write],
  /// so a typed 37.5 and a point dragged to 37.5 are the same keys.
  Future<void> _typeSpeed(BridgeClip clip, int index, double height) async {
    final box = context.findRenderObject();
    final keys = _keysOf(clip);
    if (box is! RenderBox || index < 0 || index >= keys.length) return;
    final speed = envelopeSpeeds(keys)[index];
    final id = clip.id;
    setState(() => _typing = '$id#$index');
    await showSpeedFieldPopover(
      context: context,
      // Below and to the right, so the point stays in sight beside its box.
      position: box.localToGlobal(
              Offset(_xOfKey(clip, keys[index]), _y(speed, height))) +
          const Offset(10, 8),
      percent: speed,
      onApply: (percent, _) {
        // The clip as the document holds it now, never the one the box
        // opened on.
        for (final now in _clips) {
          if (now.id != id) continue;
          final all = _keysOf(now);
          if (index < all.length) {
            _write(now, _withSpeed(now, all, index, percent));
          }
        }
      },
    );
    if (mounted) setState(() => _typing = null);
  }

  /// `Ctrl`-click or double-click the line to plant a point; `Alt`-click one
  /// to lift it — the same gestures the graph editor uses, so nothing new has
  /// to be learnt for the strip.
  void _tap(Offset local, double height) {
    final doubled = widget.onTapped();
    final keys = HardwareKeyboard.instance;
    final found = _nearestPoint(local, height);

    if (found != null && keys.isAltPressed) {
      final all = _keysOf(found.clip);
      if (all.length <= 2) return; // never below the two ends
      _write(found.clip, [
        for (var i = 0; i < all.length; i++)
          if (i != found.index) all[i],
      ]);
      return;
    }
    if (!doubled && !keys.isControlPressed) return;
    // Double-click a point to put it back to 100%, as Vegas does. Anywhere
    // else on the line it plants one, as `Ctrl`-click does everywhere.
    if (doubled && !keys.isControlPressed) {
      if (_pointAt(local, height) case final on?) {
        _resetPoint(on.clip, on.index);
        return;
      }
    }

    _plant(local);
  }

  /// Plant a point on the line at [local], on the curve it is following.
  void _plant(Offset local) {
    final clip = _clipAt(local.dx);
    if (clip == null) return;
    final all = _keysOf(clip);
    final at = (widget.axis.frameAt(local.dx) - clip.startFrame) / widget.fps;
    final speeds = envelopeSpeeds(all);
    var index = all.length;
    for (var i = 0; i < all.length; i++) {
      if (at < rationalSeconds(all[i].time)) {
        index = i;
        break;
      }
    }
    if (index == 0 || index == all.length) return; // only between the ends
    final planted = evaluateKeysSpeed(all, at) * 100;
    // Both halves keep the curve the span had.
    final grown = [...all]..insert(
        index,
        BridgeKeyframe(
          time: timeOfSubframe(at * widget.fps, widget.fpsNum, widget.fpsDen),
          value: 0,
          interpIn: const BridgeSideInterp.linear(),
          interpOut: all[index - 1].interpOut,
        ));
    _write(clip, envelopeToKeys(grown, [...speeds]..insert(index, planted)));
  }

  void _commit() {
    final held = _drag;
    if (held == null) return;
    // Every clip the selection reaches into, not just the one under the
    // pointer: a box drawn across two clips moves the points in both.
    final touched = _selected.contains('${held.clip.id}#${held.index}')
        ? _clips
            .where((c) => _selected.any((s) => s.startsWith('${c.id}#')))
            .toList()
        : [held.clip];
    for (final clip in touched) {
      final keys = clip.id == held.clip.id
          ? _moved(
              clip,
              _withSpeed(clip, _keysOf(clip), held.index, held.speed),
              held.index,
              held.dx,
            )
          : _withSpeed(clip, _keysOf(clip), held.index, held.speed);
      _write(clip, keys);
    }
    setState(() => _drag = null);
  }

  void _write(BridgeClip clip, List<BridgeKeyframe> keys) {
    widget.entry.layer.setClipRetime(
      clip: clip.id,
      value: BridgeScalar.keyframed(keys),
    );
    widget.onChanged();
  }
}

/// The envelope's furniture and its lines: the 100% reference every clip is
/// measured against, the zero line that says where backwards begins, and each
/// clip's own straight run of points.
class _SpeedEnvelopePainter extends CustomPainter {
  final List<({BridgeClip clip, List<BridgeKeyframe> keys})> lanes;
  final double Function(BridgeClip, BridgeKeyframe) xOfKey;
  final double Function(double) y;
  final (double, double) range;
  final Color line;
  final Color curve;

  /// The accent a caught point takes, and which points are caught.
  final Color chosen;
  final Set<String> selected;

  /// The point whose speed is being typed, as `clipId#index`, and the colour
  /// it is lit and ringed in while its box is open.
  final String? typing;
  final Color lit;

  /// The speed at which a clip shows every frame of its footage once, by clip
  /// id, with what the guide drawn there is called and its colour.
  final Map<String, double> guides;
  final String Function(double speed) guideLabel;
  final Color guide;

  /// The clip whose dragged point is sitting on its guide, which is drawn
  /// [lit] while it does: the snap is seen as well as felt.
  final String? caught;
  final TextStyle label;

  /// Where the viewport's left edge sits in the canvas's own coordinates.
  final double viewportLeft;

  /// Drawn over clips, with only the lines and their points.
  final bool bare;

  const _SpeedEnvelopePainter({
    required this.lanes,
    required this.xOfKey,
    required this.y,
    required this.range,
    required this.line,
    required this.curve,
    required this.chosen,
    required this.selected,
    required this.typing,
    required this.lit,
    required this.guides,
    required this.guideLabel,
    required this.guide,
    this.caught,
    required this.label,
    required this.viewportLeft,
    this.bare = false,
  });

  @override
  void paint(Canvas canvas, Size size) {
    for (final (speed, text) in [
      if (!bare) ...[(100.0, '100%'), (0.0, '0')],
    ]) {
      final at = y(speed);
      if (at < 0 || at > size.height) continue;
      // Dotted, so the graph's own reference lines never read as the row
      // seams that rule the rest of the table — solid, they were the same
      // mark meaning two different things.
      final paint = Paint()
        ..color = line
        ..strokeWidth = 1;
      for (var x = 0.0; x < size.width; x += 6) {
        canvas.drawLine(
            Offset(x, at), Offset((x + 3).clamp(0, size.width), at), paint);
      }
      final painter = TextPainter(
        text: TextSpan(text: text, style: label),
        textDirection: TextDirection.ltr,
      )..layout();
      // At the window's edge, not the content's: a label at canvas x 0 is at
      // the start of time and scrolls away the moment the lane moves.
      painter.paint(canvas, Offset(viewportLeft + 2, at - painter.height));
    }

    if (!bare) _paintGuides(canvas, size);

    final stroke = Paint()
      ..color = curve
      ..strokeWidth = 2
      ..style = PaintingStyle.stroke;
    for (final lane in lanes) {
      final speeds = envelopeSpeeds(lane.keys);
      final points = [
        for (var i = 0; i < lane.keys.length; i++)
          Offset(xOfKey(lane.clip, lane.keys[i]), y(speeds[i])),
      ];
      if (points.length < 2) continue;
      canvas.drawPath(_curvePath(lane.keys, points), stroke);
      for (var i = 0; i < points.length; i++) {
        final id = '${lane.clip.id}#$i';
        if (id == typing) {
          // Brighter than anything else on the line, with a ring round it:
          // the box beside it is about this point and no other.
          canvas.drawCircle(points[i], 4.5, Paint()..color = lit);
          canvas.drawCircle(
              points[i],
              7.5,
              Paint()
                ..color = lit
                ..style = PaintingStyle.stroke);
          continue;
        }
        final caught = selected.contains(id);
        canvas.drawCircle(
          points[i],
          caught ? 4.5 : 3.5,
          Paint()..color = caught ? chosen : curve,
        );
      }
    }
  }

  /// The line through [points], following each span's curve: sampled inside
  /// the span, then down to where it arrives (a Hold arrives at the speed it
  /// left with) and up to the next point.
  Path _curvePath(List<BridgeKeyframe> keys, List<Offset> points) {
    final path = Path()..moveTo(points.first.dx, points.first.dy);
    for (var i = 0; i + 1 < points.length; i++) {
      final (a, b) = (points[i], points[i + 1]);
      final t0 = rationalSeconds(keys[i].time);
      final t1 = rationalSeconds(keys[i + 1].time);
      final n = ((b.dx - a.dx) / 3).clamp(1, 64).round();
      for (var k = 1; k < n; k++) {
        final t = t0 + (t1 - t0) * k / n;
        path.lineTo(a.dx + (b.dx - a.dx) * k / n,
            y(evaluateKeysSpeed(keys, t) * 100));
      }
      path
        ..lineTo(b.dx, y(sideSpeedAtKey(keys, i + 1, isOut: false) * 100))
        ..lineTo(b.dx, b.dy);
    }
    return path;
  }

  /// The every-frame guide: one faint dashed line across each clip whose
  /// footage runs faster than the comp, at the speed that shows every source
  /// frame once, and its name once at the window's left edge.
  ///
  /// Dashed longer than the reference lines are dotted, so the three never
  /// read as one mark. The name sits a column in from theirs, because the
  /// guide is often a few pixels off the zero line and two labels cannot share
  /// a corner.
  void _paintGuides(Canvas canvas, Size size) {
    if (guides.isEmpty) return;
    final paint = Paint()..strokeWidth = 1;
    final column = (TextPainter(
      text: TextSpan(text: '100%', style: label),
      textDirection: TextDirection.ltr,
    )..layout())
        .width;
    final named = <double>{};
    for (final lane in lanes) {
      final speed = guides[lane.clip.id.toString()];
      if (speed == null || lane.keys.length < 2) continue;
      final at = y(speed);
      if (at < 0 || at > size.height) continue;
      // A clip's first and last keys are its own two edges.
      final from = xOfKey(lane.clip, lane.keys.first).clamp(0.0, size.width);
      final to = xOfKey(lane.clip, lane.keys.last).clamp(0.0, size.width);
      paint.color = lane.clip.id.toString() == caught
          ? lit
          : guide.withValues(alpha: 0.7);
      for (var x = from; x < to; x += 10) {
        canvas.drawLine(
            Offset(x, at), Offset((x + 6).clamp(from, to), at), paint);
      }
      if (!named.add(speed)) continue;
      final painter = TextPainter(
        text: TextSpan(text: guideLabel(speed), style: label),
        textDirection: TextDirection.ltr,
      )..layout();
      painter.paint(
          canvas, Offset(viewportLeft + column + 10, at - painter.height));
    }
  }

  @override
  bool shouldRepaint(_SpeedEnvelopePainter old) =>
      old.lanes != lanes ||
      old.range != range ||
      old.curve != curve ||
      old.selected != selected ||
      old.typing != typing ||
      old.caught != caught ||
      old.viewportLeft != viewportLeft ||
      old.bare != bare;
}

/// Takes the pointer only where [test] says, so a line drawn over clips can be
/// grabbed without covering them.
class _HitWhere extends SingleChildRenderObjectWidget {
  final bool Function(Offset local) test;

  const _HitWhere({required this.test, super.child});

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderHitWhere(test);

  @override
  void updateRenderObject(BuildContext context, _RenderHitWhere renderObject) =>
      renderObject.test = test;
}

class _RenderHitWhere extends RenderProxyBox {
  bool Function(Offset local) test;

  _RenderHitWhere(this.test);

  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) =>
      test(position) && super.hitTest(result, position: position);
}
