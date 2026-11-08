// Ties on a graph canvas's wires.
//
// A tie gathers wires at one spot, so a bundle of them runs side by side
// through it. A drag with the right button across some wires ties them. With
// Shift held the drag reroutes instead: each wire it crosses gets a point of
// its own there, and wires from the same socket share one and fan out after
// it. A tie is moved by dragging it, and let go by a right-click on it, a drag
// across it, or a click to pick it and Delete. Ctrl and a drag out of a point
// draws a new wire from the socket its wires leave, through the point.
//
// A reroute point takes its wires through the one spot and is drawn as a
// dot, as a tie on a single wire is.
//
// A tie keeps where it is and which wires go through it, each named by the
// socket it ends in. Which way they run through it and in what order is worked
// out from where the wires go each time it is drawn, so a tie keeps up with a
// box that is moved.
//
// The layer canvas and the node graph canvas have different wires. Each hands
// [GraphTies] its wires as [TieWire]s, and the rest is here once.

import 'dart:math' as math;

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/gestures.dart'
    show PointerDownEvent, kPrimaryMouseButton, kSecondaryMouseButton;
import 'package:flutter/scheduler.dart' show Ticker;
import 'package:flutter/services.dart'
    show HardwareKeyboard, SystemMouseCursors;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/wire_tie.dart' show BridgeWireTie;

import '../theme/motion.dart' show MotionSpec;
import '../theme/theme.dart';
import '../widgets/controls.dart' show ThemeScope;
import 'node_menus.dart' show showGraphTieMenu;
import 'graph_panel.dart'
    show graphWireDistance, graphWirePath, graphWireStub, graphWireWidth;

/// How far apart the wires in a tie run, in canvas units.
const double graphTieGap = 4;

/// How thick a tie is drawn, in canvas units.
const double graphTieWidth = 3.5;

/// How big a reroute point is across, in canvas units.
const double graphRerouteSize = 9;

/// One wire as the ties see it: where it runs from and to, in canvas units,
/// and its name, the socket it ends in.
typedef TieWire = ({String key, Offset from, Offset to});

/// What a tie calls the wire ending in [socket] on [box].
String tieWireKey(String box, String socket) => '$box/$socket';

/// A tie as it is drawn: its middle, the way across it, how long it is, and
/// how many places it holds. One place is drawn as a reroute point.
typedef TieBar = ({Offset at, Offset across, double length, int lanes});

/// Where the wires run through the ties, worked out from the wires.
class TieLayout {
  /// For each tied wire, the points it passes through in the order it meets
  /// them, each with the way it is heading there.
  final Map<String, List<(Offset, Offset)>> through;

  /// Each tie as drawn, in the ties' order. Null for one none of whose wires
  /// are still there.
  final List<TieBar?> bars;

  const TieLayout._(this.through, this.bars);

  /// [level] runs every bundle level through its ties, for angled wires.
  factory TieLayout.of(List<BridgeWireTie> ties, List<TieWire> wires,
      {bool level = false}) {
    final ends = {for (final w in wires) w.key: w};
    Offset at(int i) => Offset(ties[i].x, ties[i].y);
    double dot(Offset a, Offset b) => a.dx * b.dx + a.dy * b.dy;

    // Each wire's ties, in the order the wire meets them.
    final order = <String, List<int>>{};
    for (final (i, tie) in ties.indexed) {
      for (final key in tie.wires.toSet()) {
        if (ends.containsKey(key)) (order[key] ??= []).add(i);
      }
    }
    for (final MapEntry(:key, value: list) in order.entries) {
      final w = ends[key]!;
      list.sort((p, q) => dot(at(p) - w.from, w.to - w.from)
          .compareTo(dot(at(q) - w.from, w.to - w.from)));
    }

    // Where a wire is just before and just after tie [i].
    (Offset, Offset) beside(String key, int i) {
      final list = order[key]!;
      final k = list.indexOf(i);
      final w = ends[key]!;
      return (
        k == 0 ? w.from : at(list[k - 1]),
        k == list.length - 1 ? w.to : at(list[k + 1]),
      );
    }

    final through = {
      for (final MapEntry(:key, value: list) in order.entries)
        key: List.filled(list.length, (Offset.zero, Offset.zero)),
    };
    final bars = <TieBar?>[];
    for (final (i, tie) in ties.indexed) {
      final members = [
        for (final key in tie.wires.toSet())
          if (order.containsKey(key)) key,
      ];
      if (members.isEmpty) {
        bars.add(null);
        continue;
      }
      final centre = at(i);
      // The way the bundle runs: where its wires come from to where they go.
      var heading = Offset.zero;
      for (final key in members) {
        final (p, q) = beside(key, i);
        if ((q - p).distance > 0) heading += (q - p) / (q - p).distance;
      }
      heading = heading.distance < 1e-6
          ? const Offset(1, 0)
          : heading / heading.distance;
      if (level) heading = Offset(heading.dx < 0 ? -1 : 1, 0);
      final across = Offset(-heading.dy, heading.dx);
      // Side by side in the order their two ends lie, which every tie on the
      // same wires agrees on, so they keep their places from tie to tie.
      double side(String key) {
        final w = ends[key]!;
        return dot(w.from + w.to - centre * 2, across);
      }

      members.sort((a, b) => side(a).compareTo(side(b)));
      // A point takes every wire through its middle; a tie lays them side
      // by side.
      final lanes = tie.point ? 1 : members.length;
      for (final (j, key) in members.indexed) {
        final slot = tie.point
            ? centre
            : centre + across * ((j - (lanes - 1) / 2) * graphTieGap);
        through[key]![order[key]!.indexOf(i)] = (slot, heading);
      }
      bars.add((
        at: centre,
        across: across,
        length: (lanes - 1) * graphTieGap + graphTieGap * 1.5,
        lanes: lanes,
      ));
    }
    return TieLayout._(through, bars);
  }

  /// The tie whose bar or point is under [at], or null.
  int? barAt(Offset at, double zoom) {
    final reach = math.max(graphTieWidth, 6 / zoom);
    for (final (i, bar) in bars.indexed.toList().reversed) {
      if (bar == null) continue;
      if (bar.lanes == 1) {
        if ((at - bar.at).distance <= math.max(graphRerouteSize, 8 / zoom)) {
          return i;
        }
        continue;
      }
      final half = bar.across * (bar.length / 2);
      if (_toSegment(at, bar.at - half, bar.at + half) <= reach) return i;
    }
    return null;
  }
}

/// A wire through [through], in the same units as its ends. Each stretch is a
/// curve that leaves heading the way the wire heads there, so the wires run
/// side by side for a while either side of a tie. With no ties it is
/// [graphWirePath]'s wire.
Path tiedWirePath(Offset a, Offset b, List<(Offset, Offset)> through,
    {double zoom = 1, WireStyle wires = WireStyle.curved}) {
  if (through.isEmpty) return graphWirePath(a, b, zoom: zoom, wires: wires);
  final path = Path()..moveTo(a.dx, a.dy);
  if (wires == WireStyle.angled) {
    // Each stretch is an angled wire of its own, level through every tie.
    var p = a;
    for (final q in [for (final (q, _) in through) q, b]) {
      path.extendWithPath(
          graphWirePath(p, q, zoom: zoom, wires: wires), Offset.zero);
      p = q;
    }
    return path;
  }
  final stub = graphWireStub * zoom;
  var (p, u) = (a, const Offset(1, 0));
  final stops = [...through, (b, const Offset(1, 0))];
  for (final (k, (q, v)) in stops.indexed) {
    final d = (q - p).distance;
    // A socket's end leaves level for at least the stub, as every wire does.
    double reach(bool socket) =>
        socket ? math.max(d * 0.4, math.min(stub, d / 2)) : d * 0.4;
    final c1 = p + u * reach(k == 0);
    final c2 = q - v * reach(k == stops.length - 1);
    path.cubicTo(c1.dx, c1.dy, c2.dx, c2.dy, q.dx, q.dy);
    (p, u) = (q, v);
  }
  return path;
}

/// A wire [t] of the way from running through [from] to running through [to],
/// in the same units as its ends.
///
/// Each tie it gains takes hold of the wire where it lies and pulls it in, and
/// each it loses lets it go back to where it lies without it. So the wire
/// moves like a cable being gathered, not a shape fading into another. A [t]
/// past 1 pulls it a little past its place, the overshoot a landing curve
/// gives.
Path tiedWireBetween(Offset a, Offset b, List<(Offset, Offset)> from,
    List<(Offset, Offset)> to, double t,
    {double zoom = 1, WireStyle wires = WireStyle.curved}) {
  final grows = to.length >= from.length;
  final held = grows ? to : from;
  final rest = tiedWirePath(a, b, grows ? from : to, zoom: zoom, wires: wires)
      .computeMetrics()
      .firstOrNull;
  if (rest == null) return tiedWirePath(a, b, to, zoom: zoom, wires: wires);
  // Where the wire lies, and the way it runs there, nearest [p].
  (Offset, Offset) lying(Offset p) {
    var best = (p, const Offset(1, 0));
    var gap = double.infinity;
    for (var i = 0; i <= 48; i++) {
      final at = rest.getTangentForOffset(rest.length * i / 48);
      if (at != null && (at.position - p).distance < gap) {
        gap = (at.position - p).distance;
        best = (at.position, at.vector);
      }
    }
    return best;
  }

  final s = grows ? t : 1 - t;
  return tiedWirePath(
      a,
      b,
      [
        for (final (p, d) in held)
          if (lying(p) case (final q, final e))
            if (e + (d - e) * s case final way)
              (
                q + (p - q) * s,
                way.distance < 1e-6 ? d : way / way.distance,
              ),
      ],
      zoom: zoom,
      wires: wires);
}

/// How far [at] is from the wire [w] running through [through], sampled
/// along it as [graphWireDistance] samples an untied one.
double tiedWireDistance(TieWire w, List<(Offset, Offset)> through, Offset at,
    {WireStyle wires = WireStyle.curved}) {
  if (through.isEmpty) {
    return graphWireDistance(w.from, w.to, at, wires: wires);
  }
  var best = double.infinity;
  final path = tiedWirePath(w.from, w.to, through, wires: wires);
  for (final metric in path.computeMetrics()) {
    final steps = math.max(20, (metric.length / 12).ceil());
    for (var i = 0; i <= steps; i++) {
      final point =
          metric.getTangentForOffset(metric.length * i / steps)?.position;
      if (point != null) best = math.min(best, (point - at).distance);
    }
  }
  return best;
}

/// How far [p] is from the segment [a] to [b].
double _toSegment(Offset p, Offset a, Offset b) {
  final ab = b - a;
  final len = ab.distanceSquared;
  final s = len == 0
      ? 0.0
      : (((p - a).dx * ab.dx + (p - a).dy * ab.dy) / len).clamp(0.0, 1.0);
  return (p - (a + ab * s)).distance;
}

/// Where segments [a]–[b] and [c]–[d] cross, or null.
Offset? _cross(Offset a, Offset b, Offset c, Offset d) {
  final r = b - a;
  final s = d - c;
  final denom = r.dx * s.dy - r.dy * s.dx;
  if (denom == 0) return null;
  final t = ((c - a).dx * s.dy - (c - a).dy * s.dx) / denom;
  final u = ((c - a).dx * r.dy - (c - a).dy * r.dx) / denom;
  if (t < 0 || t > 1 || u < 0 || u > 1) return null;
  return a + r * t;
}

/// What the painter needs of the ties.
class TieView {
  final List<BridgeWireTie> ties;

  /// The ties before the last change, while the wires are still moving from
  /// them. [t] is how far they have got.
  final List<BridgeWireTie>? was;
  final double t;

  /// The right-button drag being drawn, in canvas units.
  final (Offset, Offset)? stroke;

  /// The wires it crosses.
  final Set<String> crossed;

  /// Where a Shift drag's reroute points would land, in canvas units.
  final List<Offset> landing;

  /// The tie under the pointer or in hand.
  final int? lit;

  /// Where a wire in hand comes out of, when it is drawn out of a point.
  final Offset? via;

  const TieView({
    this.ties = const [],
    this.was,
    this.t = 1,
    this.stroke,
    this.crossed = const {},
    this.landing = const [],
    this.lit,
    this.via,
  });
}

/// Draw [wires], each with its colour and weight, through their ties, then
/// the ties over them and any stroke being drawn.
void paintTiedWires(
  Canvas canvas,
  List<(TieWire, Color, double)> wires,
  TieView view, {
  required Offset pan,
  required double zoom,
  required WireStyle style,
  required LumitTheme theme,
}) {
  final plain = [for (final (w, _, _) in wires) w];
  final level = style == WireStyle.angled;
  final now = TieLayout.of(view.ties, plain, level: level);
  final before =
      view.was == null ? null : TieLayout.of(view.was!, plain, level: level);
  Offset screen(Offset canvas) => canvas * zoom + pan;
  List<(Offset, Offset)> onScreen(List<(Offset, Offset)>? points) => [
        for (final (p, d) in points ?? const <(Offset, Offset)>[])
          (screen(p), d)
      ];

  for (final (w, colour, weight) in wires) {
    final a = screen(w.from);
    final b = screen(w.to);
    final to = now.through[w.key];
    final from = before?.through[w.key];
    final path = before != null && !_samePoints(from, to)
        ? tiedWireBetween(a, b, onScreen(from), onScreen(to), view.t,
            zoom: zoom, wires: style)
        : tiedWirePath(a, b, onScreen(to), zoom: zoom, wires: style);
    final lit = view.crossed.contains(w.key);
    canvas.drawPath(
        path,
        Paint()
          ..color = lit ? theme.animated : colour
          ..style = PaintingStyle.stroke
          ..strokeWidth = graphWireWidth * zoom * (lit ? 2 : weight));
  }

  void bar(TieBar bar, double grown, bool lit) {
    if (grown <= 0) return;
    final c = screen(bar.at);
    if (bar.lanes == 1) {
      paintReroutePoint(canvas, c, grown, lit, zoom: zoom, theme: theme);
      return;
    }
    final half = bar.across * (bar.length * zoom * grown / 2);
    // A rim of the ground first, so a tie reads over the wires it holds.
    canvas.drawLine(
        c - half,
        c + half,
        Paint()
          ..color = theme.surface0
          ..strokeWidth = (graphTieWidth + 2) * zoom
          ..strokeCap = StrokeCap.round);
    canvas.drawLine(
        c - half,
        c + half,
        Paint()
          ..color = lit ? theme.accent : theme.textSecondary
          ..strokeWidth = graphTieWidth * zoom
          ..strokeCap = StrokeCap.round);
  }

  // A tie that has just been made grows with its wires coming in, and one
  // let go shrinks away as they leave.
  for (final (i, b) in now.bars.indexed) {
    if (b == null) continue;
    final fresh = view.was != null && !_hasTie(view.was!, view.ties[i]);
    bar(b, fresh ? view.t : 1, view.lit == i);
  }
  if (before != null) {
    for (final (i, b) in before.bars.indexed) {
      if (b != null && !_hasTie(view.ties, view.was![i])) {
        bar(b, 1 - view.t, false);
      }
    }
  }

  if (view.stroke case (final p, final q)?) {
    canvas.drawLine(
        screen(p),
        screen(q),
        Paint()
          ..color = theme.accent
          ..strokeWidth = graphWireWidth
          ..strokeCap = StrokeCap.round);
    for (final at in view.landing) {
      paintReroutePoint(canvas, screen(at), 0.8, true, zoom: zoom, theme: theme);
    }
  }
}

/// A reroute point at [c] on screen, [grown] of the way in. It comes in from
/// half its size, fading up, never from nothing.
void paintReroutePoint(Canvas canvas, Offset c, double grown, bool lit,
    {required double zoom, required LumitTheme theme}) {
  if (grown <= 0) return;
  final r = graphRerouteSize / 2 * zoom * (0.5 + 0.5 * grown);
  final a = grown.clamp(0.0, 1.0);
  canvas.drawCircle(
      c, r + zoom, Paint()..color = theme.surface0.withValues(alpha: a));
  canvas.drawCircle(
      c,
      r,
      Paint()
        ..color = (lit ? theme.accent : theme.textSecondary)
            .withValues(alpha: a));
}

/// Whether [ties] has [tie], by value: ties read again after each change
/// are new lists, and the generated == compares the wires by identity.
bool _hasTie(List<BridgeWireTie> ties, BridgeWireTie tie) => ties.any((t) =>
    t.x == tie.x &&
    t.y == tie.y &&
    t.point == tie.point &&
    listEquals(t.wires, tie.wires));

bool _samePoints(List<(Offset, Offset)>? a, List<(Offset, Offset)>? b) {
  if ((a?.length ?? 0) != (b?.length ?? 0)) return false;
  for (var i = 0; i < (a?.length ?? 0); i++) {
    if (a![i] != b![i]) return false;
  }
  return true;
}

/// The tie half of a graph canvas.
mixin GraphTies<W extends StatefulWidget> on State<W> {
  /// The canvas's ties as the document has them.
  List<BridgeWireTie> get ties;

  /// Write the ties back as one undo step.
  void commitTies(List<BridgeWireTie> next);

  double get tieZoom;

  (Offset, Offset)? _stroke;
  Set<String> _crossed = const {};

  /// Whether the stroke in hand reroutes (Shift) rather than ties.
  bool _rerouting = false;
  List<Offset> _landing = const [];

  /// A tie in hand: which, and where it has got to.
  (int, Offset)? _tieDrag;
  Offset _tieGrab = Offset.zero;

  int? _tieHover;

  /// The tie a click picked, which Delete takes away.
  int? _tiePicked;

  /// A wire being drawn out of a point: the tie, and the socket it leaves.
  (int, Offset)? _growing;

  /// The canvas's wires as they stand now.
  List<TieWire> tieWiresNow();

  List<BridgeWireTie>? _tiesWas;
  double _tieT = 1;
  MotionSpec _tieSpec = MotionSpec.still;
  Ticker? _tieTicker;

  @override
  void dispose() {
    _tieTicker?.dispose();
    super.dispose();
  }

  WireStyle get _style =>
      ThemeScope.maybeOf(context)?.wireStyle ?? WireStyle.curved;

  /// [child] wearing a hand over a tie or a point, closed while one is held,
  /// so what can be picked up says so.
  Widget tieCursorRegion(Widget child) => MouseRegion(
        opaque: false,
        cursor: _tieDrag != null
            ? SystemMouseCursors.grabbing
            : _tieHover != null
                ? SystemMouseCursors.grab
                : MouseCursor.defer,
        child: child,
      );

  /// The ties for the painter, a tie in hand drawn where it has got to.
  TieView tieView() => TieView(
        ties: [
          for (final (i, tie) in ties.indexed)
            if (_tieDrag case (final at, final where) when at == i)
              BridgeWireTie(
                x: where.dx, y: where.dy, wires: tie.wires, point: tie.point)
            else
              tie,
        ],
        was: _tiesWas,
        t: _tieT,
        stroke: _stroke,
        crossed: _crossed,
        landing: _landing,
        lit: _tieDrag?.$1 ?? _tiePicked ?? _tieHover,
        via: switch (_growing) {
          (final i, _) when i < ties.length => Offset(ties[i].x, ties[i].y),
          _ => null,
        },
      );

  /// A press on empty canvas. The right button starts a stroke, with Shift a
  /// reroute, and the left takes hold of a tie under it. True when the press
  /// was the ties'.
  bool tieDown(PointerDownEvent event, Offset at, List<TieWire> wires) {
    if (_tiePicked != null) setState(() => _tiePicked = null);
    if (event.buttons == kSecondaryMouseButton) {
      setState(() {
        _stroke = (at, at);
        _rerouting = HardwareKeyboard.instance.isShiftPressed;
        _landing = const [];
      });
      return true;
    }
    if (event.buttons != kPrimaryMouseButton) return false;
    final hit = _layout(wires).barAt(at, tieZoom);
    if (hit == null) return false;
    final held = Offset(ties[hit].x, ties[hit].y);
    _tieGrab = held - at;
    setState(() => _tieDrag = (hit, held));
    return true;
  }

  /// With Ctrl held, a press on a tie starts a new wire from the socket its
  /// first wire leaves: where that socket is, for the canvas to start the
  /// wire from as if the socket itself were pressed. Null otherwise.
  Offset? tieWireFrom(PointerDownEvent event, Offset at, List<TieWire> wires) {
    final keys = HardwareKeyboard.instance;
    if (event.buttons != kPrimaryMouseButton ||
        !(keys.isControlPressed || keys.isMetaPressed)) {
      return null;
    }
    final hit = _layout(wires).barAt(at, tieZoom);
    if (hit == null) return null;
    final ends = {for (final w in wires) w.key: w.from};
    final from = ties[hit].wires.map((k) => ends[k]).nonNulls.firstOrNull;
    if (from == null) return null;
    _growing = (hit, from);
    return from;
  }

  /// A wire in hand was let go. One drawn out of a point joins that point
  /// once the canvas has it.
  void tieWireLanded() {
    final growing = _growing;
    _growing = null;
    if (growing == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final (i, from) = growing;
      if (i >= ties.length) return;
      final wires = tieWiresNow();
      final tie = ties[i];
      final joining = [
        for (final w in wires)
          if (w.from == from && !tie.wires.contains(w.key)) w.key,
      ];
      if (joining.isEmpty) return;
      _write([
        for (final (j, t) in ties.indexed)
          j == i
              ? BridgeWireTie(
                  x: t.x,
                  y: t.y,
                  wires: [...t.wires, ...joining],
                  point: t.point)
              : t,
      ], wires);
    });
  }

  /// Delete with a tie picked takes it away. True when there was one.
  bool tieDelete() {
    final i = _tiePicked;
    if (i == null || i >= ties.length) return false;
    setState(() => _tiePicked = null);
    _write([...ties]..removeAt(i), tieWiresNow());
    return true;
  }

  /// The pointer moving. True when it carried a stroke or a tie.
  bool tieMove(Offset at, List<TieWire> wires) {
    if (_tieDrag case (final i, _)?) {
      setState(() => _tieDrag = (i, at + _tieGrab));
      return true;
    }
    if (_stroke case (final from, _)?) {
      final crossings = _crossings(from, at, wires);
      setState(() {
        _stroke = (from, at);
        _crossed = crossings.keys.toSet();
        _landing = _rerouting
            ? [for (final (at, _) in _reroutes(crossings, wires)) at]
            : const [];
      });
      return true;
    }
    return false;
  }

  /// The pointer over the canvas with nothing held: a tie under it lights.
  void tieHover(Offset at, List<TieWire> wires) {
    final hit = ties.isEmpty ? null : _layout(wires).barAt(at, tieZoom);
    if (hit != _tieHover) setState(() => _tieHover = hit);
  }

  /// A reroute point on one wire, where the pointer is.
  void tieAddPoint(String wire, Offset at) => _write([
        ...ties,
        BridgeWireTie(x: at.dx, y: at.dy, wires: [wire], point: true),
      ], tieWiresNow());

  /// The press let go. False for a right-click that met no tie, which the
  /// canvas then treats as its own. A stationary right-click on a tie opens
  /// its menu at [menuAt], in window coordinates.
  bool tieUp(Offset at, bool moved, List<TieWire> wires, {Offset? menuAt}) {
    if (_tieDrag case (final i, final where)?) {
      setState(() {
        _tieDrag = null;
        // A click, not a drag, picks the tie.
        if (!moved) _tiePicked = i;
      });
      if (moved && i < ties.length) {
        _write([
          for (final (j, tie) in ties.indexed)
            j == i
                ? BridgeWireTie(
                x: where.dx, y: where.dy, wires: tie.wires, point: tie.point)
                : tie,
        ], wires, morph: false);
      }
      return true;
    }
    final stroke = _stroke;
    if (stroke == null) return false;
    setState(() {
      _stroke = null;
      _crossed = const {};
      _landing = const [];
    });
    final layout = _layout(wires);
    if (!moved) {
      final hit = layout.barAt(at, tieZoom);
      if (hit == null) return false;
      if (menuAt == null) {
        _write([...ties]..removeAt(hit), wires);
        return true;
      }
      setState(() => _tiePicked = hit);
      showGraphTieMenu(context, menuAt, onDelete: () {
        if (!mounted || hit >= ties.length) return;
        setState(() => _tiePicked = null);
        _write([...ties]..removeAt(hit), tieWiresNow());
      });
      return true;
    }
    final from = stroke.$1;
    // A stroke across a tie lets it go.
    final cut = <int>{
      for (final (i, bar) in layout.bars.indexed)
        if (bar != null &&
            _cross(from, at, bar.at - bar.across * (bar.length / 2 + 2),
                    bar.at + bar.across * (bar.length / 2 + 2)) !=
                null)
          i,
    };
    if (cut.isNotEmpty) {
      _write([
        for (final (i, tie) in ties.indexed)
          if (!cut.contains(i)) tie,
      ], wires);
      return true;
    }
    final crossed = _crossings(from, at, wires);
    if (crossed.isEmpty) return true;
    if (_rerouting) {
      _write([
        ...ties,
        for (final (at, keys) in _reroutes(crossed, wires))
          BridgeWireTie(x: at.dx, y: at.dy, wires: keys, point: true),
      ], wires);
      return true;
    }
    final centre =
        crossed.values.reduce((a, b) => a + b) / crossed.length.toDouble();
    _write([
      ...ties,
      BridgeWireTie(
          x: centre.dx,
          y: centre.dy,
          wires: crossed.keys.toList(),
          point: false),
    ], wires);
    return true;
  }

  TieLayout _layout(List<TieWire> wires) =>
      TieLayout.of(ties, wires, level: _style == WireStyle.angled);

  /// How far [at] is from wire [w] as it runs through the ties.
  double tieWireDistance(TieWire w, Offset at, List<TieWire> wires) =>
      tiedWireDistance(w, _layout(wires).through[w.key] ?? const [], at,
          wires: _style);

  /// One reroute point per socket the [crossed] wires leave, where they
  /// were crossed, each with its wires.
  List<(Offset, List<String>)> _reroutes(
      Map<String, Offset> crossed, List<TieWire> wires) {
    final from = {for (final w in wires) w.key: w.from};
    final bySocket = <Offset, List<String>>{};
    for (final key in crossed.keys) {
      (bySocket[from[key]!] ??= []).add(key);
    }
    return [
      for (final keys in bySocket.values)
        (
          keys.map((k) => crossed[k]!).reduce((a, b) => a + b) /
              keys.length.toDouble(),
          keys,
        ),
    ];
  }

  /// The wires the stroke from [p] to [q] crosses, each with where.
  Map<String, Offset> _crossings(Offset p, Offset q, List<TieWire> wires) {
    final layout = _layout(wires);
    final crossed = <String, Offset>{};
    for (final w in wires) {
      final path = tiedWirePath(w.from, w.to, layout.through[w.key] ?? const [],
          wires: _style);
      for (final metric in path.computeMetrics()) {
        Offset? last;
        for (var i = 0; i <= 32; i++) {
          final point =
              metric.getTangentForOffset(metric.length * i / 32)?.position;
          if (point == null) continue;
          if (last != null) {
            if (_cross(p, q, last, point) case final hit?) {
              crossed[w.key] = hit;
              break;
            }
          }
          last = point;
        }
      }
    }
    return crossed;
  }

  /// Write [next], each tie keeping only the wires still there and one with
  /// none left going, and draw the wires moving to it.
  void _write(List<BridgeWireTie> next, List<TieWire> wires,
      {bool morph = true}) {
    final live = {for (final w in wires) w.key};
    final kept = [
      for (final tie in next)
        if (tie.wires.where(live.contains).toList() case final keep
            when keep.isNotEmpty)
          BridgeWireTie(x: tie.x, y: tie.y, wires: keep, point: tie.point),
    ];
    _tieSpec = ThemeScope.maybeOf(context)?.motion.tie ?? MotionSpec.still;
    if (morph && !_tieSpec.isStill) {
      _tiesWas = ties;
      _tieT = 0;
      final ticker = _tieTicker ??= Ticker(_tieTick);
      ticker.stop();
      ticker.start();
    }
    commitTies(kept);
  }

  void _tieTick(Duration elapsed) {
    final f = (elapsed.inMicroseconds / _tieSpec.duration.inMicroseconds)
        .clamp(0.0, 1.0);
    setState(() {
      _tieT = _tieSpec.curve.transform(f);
      if (f >= 1) {
        _tiesWas = null;
        _tieT = 1;
      }
    });
    if (f >= 1) _tieTicker?.stop();
  }
}
