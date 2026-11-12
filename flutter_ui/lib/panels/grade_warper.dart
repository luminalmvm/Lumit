// The Colour warper's mesh: a web over the colour wheel, spokes at even hues
// and three rings out to full colour, plus the centre. Drag a point and the
// colours under it go where it went.
//
// Drawn the vectorscope's way round, red up and to the left, so the picture's
// own colours, plotted faintly underneath, sit under the points that move
// them. The plot is the Scopes panel's vectorscope of the picture on screen,
// asked for once per new picture.

import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart' show DragStartBehavior, kSecondaryButton;
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:provider/provider.dart';

import '../theme/theme.dart';
import '../widgets/colour_picker.dart' show hsvToRgb;
import '../widgets/controls.dart' show ThemeScope;
import 'scopes_panel_frb.dart' show scopeColoursFor;

/// Where red sits round the vectorscope, as the engine counts hue from it.
const double _redAngle = 1.7960518;

/// The mesh's rings, the centre not counted.
const int _rings = 3;

/// The engine's vectorscope trace: its size, its kind, and how far out a
/// full colour (the outer ring) lands, as a share of the trace's width.
const int _traceEdge = 256;
const int _traceKind = 2;
const double _traceReach = 0.45;

/// Where the outer ring sits, as a share of the editor's radius: room past it
/// for a point pushed out, as far as the rim.
const double _ringAt = 0.7;

/// The mesh point at ring [ring] (0 the centre) and spoke [spoke], as the
/// `points` row numbers them.
int warperIndex(int hues, int ring, int spoke) =>
    ring == 0 ? 0 : 1 + (ring - 1) * hues + spoke;

/// How many points a mesh of [hues] has.
int warperPoints(int hues) => 1 + _rings * hues;

class GradeWarperMesh extends StatefulWidget {
  /// Six or eight.
  final int hues;

  /// Each point's move, `(Cb, Cr) / 0.5` with Cr up, centre first; as many
  /// as [warperPoints] says.
  final List<Offset> moves;

  final double size;
  final bool enabled;
  final VoidCallback onStart;
  final ValueChanged<List<Offset>> onLive;
  final ValueChanged<List<Offset>> onCommit;
  final VoidCallback onCancel;

  const GradeWarperMesh({
    super.key,
    required this.hues,
    required this.moves,
    required this.size,
    required this.enabled,
    required this.onStart,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
  });

  @override
  State<GradeWarperMesh> createState() => _GradeWarperMeshState();
}

class _GradeWarperMeshState extends State<GradeWarperMesh> {
  /// The moves in hand while a drag runs, and the point it holds.
  List<Offset>? _held;
  int? _point;
  int? _hover;

  ui.Image? _trace;

  /// The scope's ground and trace colours the plot was asked in.
  List<Uint8List> _inks = const [];
  StreamSubscription<WorkerResponse>? _responses;
  LumitUiState? _ui;
  int _asked = -1;

  @override
  void initState() {
    super.initState();
    _responses = context.read<LumitState>().onWorkerResponse.listen(_onTrace);
    _ui = context.read<LumitUiState>();
    _ui!.frameArrived.addListener(_ask);
    WidgetsBinding.instance.addPostFrameCallback((_) => _ask());
  }

  @override
  void dispose() {
    _ui?.frameArrived.removeListener(_ask);
    _responses?.cancel();
    _trace?.dispose();
    super.dispose();
  }

  /// A vectorscope of the new picture, in the Scopes panel's own colours so
  /// an open Scopes panel takes the same trace unchanged.
  void _ask() {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (!mounted || ui == null || comp == null) return;
    if (ui.frameArrived.value == _asked) return;
    _asked = ui.frameArrived.value;
    _inks = scopeColoursFor(ThemeScope.of(context).theme,
        themed: ui.workspace.themedScopes);
    try {
      comp.renderScope(
        frame: ui.playheadFrame.value,
        scale: ui.measuredScale,
        kinds: const [_traceKind],
        colours: _inks,
        view: ui.measuredViewId,
      );
    } catch (_) {
      // No worker, as in a test: the mesh draws over nothing.
    }
  }

  void _onTrace(WorkerResponse response) {
    if (response is! WorkerResponse_Scope ||
        response.field0.kind != _traceKind) {
      return;
    }
    ui.decodeImageFromPixels(Uint8List.fromList(response.field0.rgba),
        _traceEdge, _traceEdge, ui.PixelFormat.rgba8888, (image) {
      if (!mounted) {
        image.dispose();
        return;
      }
      setState(() {
        _trace?.dispose();
        _trace = image;
      });
    });
  }

  double get _radius => widget.size / 2;

  /// Pixels a mesh unit (the outer ring's radius).
  double get _unit => _radius * _ringAt;

  List<Offset> get _moves => _held ?? _padded(widget.moves);

  List<Offset> _padded(List<Offset> m) => [
        for (var i = 0; i < warperPoints(widget.hues); i++)
          i < m.length ? m[i] : Offset.zero,
      ];

  /// Where point [i] rests, in mesh units with y up.
  Offset _rest(int i) {
    if (i == 0) return Offset.zero;
    final ring = (i - 1) ~/ widget.hues + 1;
    final spoke = (i - 1) % widget.hues;
    final a = _redAngle + spoke * 2 * math.pi / widget.hues;
    return Offset(math.cos(a), math.sin(a)) * (ring / _rings);
  }

  Offset _screen(Offset mesh) =>
      Offset(_radius + mesh.dx * _unit, _radius - mesh.dy * _unit);

  int? _nearest(Offset p) {
    final moves = _moves;
    int? best;
    var far = 12.0;
    for (var i = 0; i < moves.length; i++) {
      final d = (_screen(_rest(i) + moves[i]) - p).distance;
      if (d < far) {
        far = d;
        best = i;
      }
    }
    return best;
  }

  /// A move as the row can hold it, a unit either way, and short of the rim.
  Offset _clamp(int i, Offset o) {
    var m = Offset(o.dx.clamp(-1.0, 1.0), o.dy.clamp(-1.0, 1.0));
    final at = _rest(i) + m;
    const rim = 0.95 / _ringAt;
    if (at.distance > rim) m = at / at.distance * rim - _rest(i);
    return m;
  }

  void _start(Offset at) {
    final i = _nearest(at);
    if (i == null) return;
    widget.onStart();
    setState(() {
      _point = i;
      _held = _padded(widget.moves);
    });
  }

  void _move(Offset delta) {
    final i = _point, held = _held;
    if (i == null || held == null) return;
    final rate = HardwareKeyboard.instance.isShiftPressed ? 0.1 : 1.0;
    final by = Offset(delta.dx, -delta.dy) / _unit * rate;
    final next = List.of(held)..[i] = _clamp(i, held[i] + by);
    setState(() => _held = next);
    widget.onLive(next);
  }

  void _end() {
    final held = _held;
    if (_point == null || held == null) return;
    setState(() {
      _point = null;
      _held = null;
    });
    widget.onCommit(held);
  }

  void _cancel() {
    if (_point == null) return;
    setState(() {
      _point = null;
      _held = null;
    });
    widget.onCancel();
  }

  /// One point, or every point, back where it rests, as one step. Nothing
  /// is written when it is there already.
  void _reset(int? only) {
    final now = _padded(widget.moves);
    final next = [
      for (var i = 0; i < now.length; i++)
        only == null || only == i ? Offset.zero : now[i],
    ];
    if (next.every((o) => o == Offset.zero) &&
        now.every((o) => o == Offset.zero)) {
      return;
    }
    if (only != null && now[only] == Offset.zero) return;
    widget.onStart();
    widget.onCommit(next);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final paint = CustomPaint(
      key: const ValueKey('grade-warper-mesh'),
      size: Size.square(widget.size),
      painter: _MeshPainter(
        hues: widget.hues,
        moves: _moves,
        rest: _rest,
        unit: _unit,
        held: _point,
        hover: _hover,
        trace: _trace,
        inks: _inks,
        enabled: widget.enabled,
        t: t,
      ),
    );
    if (!widget.enabled) return paint;
    int? tapped;
    return MouseRegion(
      onHover: (e) {
        final i = _nearest(e.localPosition);
        if (i != _hover) setState(() => _hover = i);
      },
      onExit: (_) {
        if (_hover != null) setState(() => _hover = null);
      },
      cursor: _hover == null
          ? SystemMouseCursors.basic
          : SystemMouseCursors.precise,
      child: Listener(
        // A right-click puts every point back.
        onPointerDown: (e) {
          if (e.buttons == kSecondaryButton) _reset(null);
        },
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          // The point under the press, not where the drag was recognised.
          dragStartBehavior: DragStartBehavior.down,
          onPanStart: (d) => _start(d.localPosition),
          onPanUpdate: (d) => _move(d.delta),
          onPanEnd: (_) => _end(),
          onPanCancel: _cancel,
          onDoubleTapDown: (d) => tapped = _nearest(d.localPosition),
          onDoubleTap: () {
            if (tapped != null) _reset(tapped);
          },
          child: paint,
        ),
      ),
    );
  }
}

/// The hue rim, the picture's colours, the mesh at rest in hairline, the
/// mesh as it is moved, and its points.
class _MeshPainter extends CustomPainter {
  final int hues;
  final List<Offset> moves;
  final Offset Function(int) rest;
  final double unit;
  final int? held, hover;
  final ui.Image? trace;

  /// The scope's ground and trace, which the plot is read against.
  final List<Uint8List> inks;
  final bool enabled;
  final LumitTheme t;

  const _MeshPainter({
    required this.hues,
    required this.moves,
    required this.rest,
    required this.unit,
    required this.held,
    required this.hover,
    required this.trace,
    required this.inks,
    required this.enabled,
    required this.t,
  });

  static double _lum(Uint8List c) =>
      c.length < 3 ? 0 : 0.25 * c[0] + 0.6 * c[1] + 0.15 * c[2];

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final r = size.shortestSide / 2;
    Offset at(Offset mesh) => c + Offset(mesh.dx * unit, -mesh.dy * unit);

    // A thin hue band at the rim, muted as the grade wheels are.
    const steps = 36;
    final rim = Rect.fromCircle(center: c, radius: r - 3);
    for (var i = 0; i < steps; i++) {
      final a0 = _redAngle + i * 2 * math.pi / steps;
      final hue = i * 360 / steps;
      canvas.drawArc(
          rim,
          -a0,
          -2 * math.pi / steps - 0.01,
          false,
          Paint()
            ..style = PaintingStyle.stroke
            ..strokeWidth = 4
            ..color =
                enabled ? hsvToRgb(hue, 0.6, 0.92) : hsvToRgb(hue, 0, 0.42));
    }

    // The picture's colours, as alpha in a theme ink, so the plot reads
    // faintly whatever colours the scope was drawn in.
    if (trace case final image? when inks.length >= 2) {
      final side = unit / _traceReach;
      final ink = t.textSecondary;
      // How far each pixel is from the scope's ground, toward its trace.
      final ground = _lum(inks[0]);
      final s = _lum(inks[1]) >= ground ? 1.5 : -1.5;
      canvas.drawImageRect(
        image,
        Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble()),
        Rect.fromCenter(center: c, width: side, height: side),
        Paint()
          ..filterQuality = FilterQuality.medium
          ..colorFilter = ColorFilter.matrix([
            0, 0, 0, 0, ink.r * 255, //
            0, 0, 0, 0, ink.g * 255,
            0, 0, 0, 0, ink.b * 255,
            0.25 * s, 0.6 * s, 0.15 * s, 0, -s * ground - 12,
          ])
          ..color = ink.withValues(alpha: 0.55),
      );
    }

    final restLine = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = t.hairline;
    for (var ring = 1; ring <= _rings; ring++) {
      canvas.drawCircle(c, unit * ring / _rings, restLine);
    }
    for (var k = 0; k < hues; k++) {
      canvas.drawLine(c, at(rest(warperIndex(hues, _rings, k))), restLine);
    }

    // The mesh as moved.
    Offset point(int i) =>
        at(rest(i) + (i < moves.length ? moves[i] : Offset.zero));
    final moved = moves.any((m) => m != Offset.zero);
    final line = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = !enabled
          ? t.textDisabled
          : moved
              ? t.textSecondary
              : t.textMuted;
    for (var ring = 1; ring <= _rings; ring++) {
      final path = Path();
      for (var k = 0; k <= hues; k++) {
        final p = point(warperIndex(hues, ring, k % hues));
        k == 0 ? path.moveTo(p.dx, p.dy) : path.lineTo(p.dx, p.dy);
      }
      canvas.drawPath(path, line);
    }
    for (var k = 0; k < hues; k++) {
      final path = Path()..moveTo(point(0).dx, point(0).dy);
      for (var ring = 1; ring <= _rings; ring++) {
        final p = point(warperIndex(hues, ring, k));
        path.lineTo(p.dx, p.dy);
      }
      canvas.drawPath(path, line);
    }

    for (var i = 0; i < moves.length; i++) {
      final p = point(i);
      final lit = i == held || i == hover;
      final off = moves[i] != Offset.zero;
      canvas.drawCircle(p, lit ? 5 : 4, Paint()..color = t.surface0);
      canvas.drawCircle(
          p,
          lit ? 3.5 : 2.5,
          Paint()
            ..color = !enabled
                ? t.textDisabled
                : off || lit
                    ? t.accent
                    : t.textPrimary);
    }
  }

  @override
  bool shouldRepaint(_MeshPainter old) =>
      old.hues != hues ||
      old.held != held ||
      old.hover != hover ||
      old.trace != trace ||
      old.enabled != enabled ||
      old.unit != unit ||
      old.t != t ||
      old.moves.length != moves.length ||
      [for (var i = 0; i < moves.length; i++) old.moves[i] != moves[i]]
          .any((x) => x);
}
