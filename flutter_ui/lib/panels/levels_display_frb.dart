// Levels' own display (docs/08 §3.31): the frame's histogram with the
// input black, gamma and white handles over it, and the output range as a bar
// beneath.
//
// **In plain terms.** The histogram is a picture of how much of the frame is
// dark and how much is bright — a hill piled up on the left is a dark shot.
// Drag the black handle in to where the hill starts and the shadows go properly
// black; drag the white handle in to where it ends and the highlights go
// properly white; the middle handle bends everything between them. The bar
// underneath says what black and white come *out* as, which is how a picture is
// deliberately flattened for a look.
//
// **Presentation only.** Every number this touches already has its own row
// underneath, and this writes exactly the same values through exactly the same
// callbacks — nothing about the effect, its parameters or its import changes.
// The handles drive **Master**; the three colour channels stay on their rows,
// where a per-channel move is a rarer, more deliberate act.
//
// **Where the picture comes from.** The same trace the Scopes panel reads
// (`renderScope`, kind 3), asked for once per displayed frame and only while
// this row is mounted. The bridge-call budget is the gate.
// The reply carries the kind it answers, so a Scopes panel open on a waveform
// at the same time cannot leave its trace behind these handles.

import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../state/comp_time.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'keyframe_controls_frb.dart';
import 'scopes_panel_frb.dart';

/// The histogram's trace code (docs/17: 0 waveform, 1 parade, 2 vectorscope,
/// 3 histogram).
const int _histogramKind = 3;

/// The engine's fixed trace size.
const int _traceEdge = 256;

/// The parameters the display drives, in the order they are drawn.
const String _inBlack = 'master_in_black';
const String _gamma = 'master_gamma';
const String _inWhite = 'master_in_white';
const String _outBlack = 'master_out_black';
const String _outWhite = 'master_out_white';

/// Gamma at the far end of the middle handle's travel. Photoshop's, and the
/// number that makes the handle useful across its whole width rather than
/// spending most of it near 1.
const double _gammaExtreme = 9.99;

/// How wide the plot is drawn, and how tall each of its two parts.
const double _plotWidth = 200;
const double _plotHeight = 54;
const double _handleStrip = 9;
const double _outputBar = 8;

/// What the histogram behind the handles is drawn in (owner, desk test).
///
/// The display used to ask for `scopeColoursFor(themed: true)` outright, so the
/// three channel humps came out in whatever the theme happened to offer — a
/// layer-palette blue for red, a solid green for green, the accent for blue.
/// A channel graph has to be readable as *that channel*, which is what the
/// standard R, G and B are for; the ground and the luma trace stay themed,
/// because those are chrome and not a measurement.
///
/// [themed] — Settings → "Use theme colours in effect graphs", off by default
/// — gives the whole graph back to the theme for anyone who wants it that way.
List<Uint8List> levelsHistogramColours(LumitTheme t, {required bool themed}) {
  final chrome = scopeColoursFor(t, themed: true);
  if (themed) return chrome;
  final standard = scopeColoursFor(t);
  return [chrome[0], chrome[1], standard[2], standard[3], standard[4]];
}

/// Levels' histogram, handles and output bar.
class LevelsDisplayFrb extends StatefulWidget {
  final UuidValue effectId;

  /// The effect's current values, staged drag included — what the panel is
  /// already showing on the rows beneath.
  final Map<String, BridgeEffectValue> values;
  final CompositionReference comp;
  final int playheadFrame;

  /// The panel's commit and preview channels, the same two every row uses.
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onWrite;
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onLive;

  const LevelsDisplayFrb({
    super.key,
    required this.effectId,
    required this.values,
    required this.comp,
    required this.playheadFrame,
    required this.onWrite,
    required this.onLive,
  });

  @override
  State<LevelsDisplayFrb> createState() => _LevelsDisplayFrbState();
}

class _LevelsDisplayFrbState extends State<LevelsDisplayFrb> {
  ui.Image? _trace;
  StreamSubscription<WorkerResponse>? _responses;
  int _asked = -1;

  /// Which handle a drag has hold of, by parameter id.
  String? _held;

  @override
  void initState() {
    super.initState();
    _responses = Provider.of<LumitState>(context, listen: false)
        .onWorkerResponse
        .listen(_onResponse);
    // The first trace: asked for after the frame is up, never from `build`.
    WidgetsBinding.instance.addPostFrameCallback((_) => _request());
  }

  @override
  void didUpdateWidget(LevelsDisplayFrb old) {
    super.didUpdateWidget(old);
    if (old.playheadFrame != widget.playheadFrame) _request();
  }

  @override
  void dispose() {
    _responses?.cancel();
    _trace?.dispose();
    super.dispose();
  }

  /// One trace of the frame showing, and only when it has moved. Never called
  /// from `build`.
  void _request() {
    if (!mounted || _asked == widget.playheadFrame) return;
    _asked = widget.playheadFrame;
    final state = Provider.of<LumitUiState>(context, listen: false);
    final t = ThemeScope.of(context).theme;
    try {
      widget.comp.renderScope(
        frame: BigInt.from(widget.playheadFrame),
        scale: state.viewerScale,
        kind: _histogramKind,
        colours: levelsHistogramColours(t,
            themed: state.workspace.themedEffectGraphs),
      );
    } catch (_) {
      // A comp that went away under the row is not worth an error here: the
      // handles read their numbers and the plot simply stays empty.
    }
  }

  void _onResponse(WorkerResponse response) {
    // Somebody else's trace: the Scopes panel may be asking for a waveform on
    // the same stream at the same time.
    if (response is! WorkerResponse_Scope) return;
    if (response.field0.kind != _histogramKind) return;
    ui.decodeImageFromPixels(
      Uint8List.fromList(response.field0.rgba),
      _traceEdge,
      _traceEdge,
      ui.PixelFormat.rgba8888,
      (image) {
        if (!mounted) {
          image.dispose();
          return;
        }
        setState(() {
          _trace?.dispose();
          _trace = image;
        });
      },
    );
  }

  BridgeScalar? _scalarOf(String param) => switch (widget.values[param]) {
        BridgeEffectValue_Float(:final field0) => field0,
        _ => null
      };

  /// What a parameter reads right now — the static value, or the curve's value
  /// under the playhead.
  double _valueOf(String param, double fallback) {
    final scalar = _scalarOf(param);
    if (scalar == null) return fallback;
    if (scalar is BridgeScalar_Static) return scalar.field0;
    if (scalar is BridgeScalar_Expression) return fallback;
    return sampledScalar(
        scalar, timeOfFrame(widget.comp, widget.playheadFrame));
  }

  /// Write a parameter the way every row writes one: into the key under the
  /// playhead when it is animated, straight when it is not.
  BridgeEffectValue _written(String param, double value) {
    final scalar = _scalarOf(param);
    return BridgeEffectValue.float(scalar == null
        ? BridgeScalar.static_(value)
        : scalarWithValueAt(scalar, value, widget.comp, widget.playheadFrame));
  }

  /// An expression decides this number, so no handle may move it.
  bool _driven(String param) => _scalarOf(param) is BridgeScalar_Expression;

  /// Where the middle handle sits between the two input ends, and back again.
  /// Left of centre is a gamma above 1 (a lift), which is the direction every
  /// other editor's midtone handle moves.
  static double _gammaToFraction(double gamma) {
    final g = gamma.clamp(1 / _gammaExtreme, _gammaExtreme).toDouble();
    return (0.5 - math.log(g) / math.log(_gammaExtreme) / 2).clamp(0.0, 1.0);
  }

  static double _fractionToGamma(double f) =>
      math.pow(_gammaExtreme, (0.5 - f.clamp(0.0, 1.0)) * 2).toDouble();

  /// The x of each handle, as a fraction of the plot.
  Map<String, double> _positions() {
    final black = _valueOf(_inBlack, 0);
    final white = _valueOf(_inWhite, 1);
    final span = white - black;
    final mid = span.abs() < 1e-6
        ? black
        : black + _gammaToFraction(_valueOf(_gamma, 1)) * span;
    return {
      _inBlack: black.clamp(0.0, 1.0).toDouble(),
      _gamma: mid.clamp(0.0, 1.0).toDouble(),
      _inWhite: white.clamp(0.0, 1.0).toDouble(),
      _outBlack: _valueOf(_outBlack, 0).clamp(0.0, 1.0).toDouble(),
      _outWhite: _valueOf(_outWhite, 1).clamp(0.0, 1.0).toDouble(),
    };
  }

  /// The value a drag to fraction [f] means for the handle being held.
  double _valueAt(String param, double f) {
    if (param != _gamma) return f;
    // The middle handle says where it sits between the ends, and the ends do
    // not move with it.
    final black = _valueOf(_inBlack, 0);
    final white = _valueOf(_inWhite, 1);
    final span = white - black;
    if (span.abs() < 1e-6) return 1;
    return _fractionToGamma(((f - black) / span).clamp(0.0, 1.0));
  }

  String? _grab(double f, bool output) {
    final at = _positions();
    final candidates = output
        ? const [_outBlack, _outWhite]
        : const [_inBlack, _gamma, _inWhite];
    String? best;
    var bestD = double.infinity;
    for (final p in candidates) {
      if (_driven(p)) continue;
      final d = (at[p]! - f).abs();
      if (d < bestD) {
        bestD = d;
        best = p;
      }
    }
    return best;
  }

  double _fractionOf(double dx) => (dx / _plotWidth).clamp(0.0, 1.0);

  Widget _strip({
    required bool output,
    required Widget child,
    required double height,
  }) =>
      GestureDetector(
        behavior: HitTestBehavior.opaque,
        // Which handle is grabbed is decided where the pointer went down, not
        // where the slop was exceeded — see curve_editor.dart.
        dragStartBehavior: DragStartBehavior.down,
        // Horizontal, not a pan: a handle only ever moves sideways, and the
        // panel this sits in is a vertical list that would win a pan's larger
        // slop every time (curve_editor.dart carries the long version).
        onHorizontalDragStart: (d) {
          _held = _grab(_fractionOf(d.localPosition.dx), output);
        },
        onHorizontalDragUpdate: (d) {
          final held = _held;
          if (held == null) return;
          widget.onLive(widget.effectId, held,
              _written(held, _valueAt(held, _fractionOf(d.localPosition.dx))));
        },
        onHorizontalDragEnd: (d) {
          final held = _held;
          _held = null;
          if (held == null) return;
          widget.onWrite(widget.effectId, held,
              _written(held, _valueAt(held, _fractionOf(d.localPosition.dx))));
        },
        onHorizontalDragCancel: () => _held = null,
        child: SizedBox(width: _plotWidth, height: height, child: child),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final at = _positions();
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _strip(
            output: false,
            height: _plotHeight + _handleStrip,
            child: Column(
              children: [
                SizedBox(
                  width: _plotWidth,
                  height: _plotHeight,
                  child: Container(
                    key: const ValueKey('fx-levels-histogram'),
                    color: t.surface0,
                    child: _trace == null
                        ? null
                        : RawImage(
                            image: _trace,
                            fit: BoxFit.fill,
                            filterQuality: FilterQuality.none,
                          ),
                  ),
                ),
                SizedBox(
                  width: _plotWidth,
                  height: _handleStrip,
                  child: CustomPaint(
                    key: const ValueKey('fx-levels-input-handles'),
                    painter: _HandlePainter(
                      at: [at[_inBlack]!, at[_gamma]!, at[_inWhite]!],
                      colours: [t.textPrimary, t.accent, t.textPrimary],
                      outline: t.hairlineStrong,
                    ),
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 4),
          _strip(
            output: true,
            height: _outputBar + _handleStrip,
            child: Column(
              children: [
                SizedBox(
                  width: _plotWidth,
                  height: _outputBar,
                  child: CustomPaint(
                    key: const ValueKey('fx-levels-output-bar'),
                    painter: _OutputBarPainter(
                      black: at[_outBlack]!,
                      white: at[_outWhite]!,
                      low: t.surface0,
                      high: t.textPrimary,
                      edge: t.hairline,
                    ),
                  ),
                ),
                SizedBox(
                  width: _plotWidth,
                  height: _handleStrip,
                  child: CustomPaint(
                    key: const ValueKey('fx-levels-output-handles'),
                    painter: _HandlePainter(
                      at: [at[_outBlack]!, at[_outWhite]!],
                      colours: [t.textPrimary, t.textPrimary],
                      outline: t.hairlineStrong,
                    ),
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 2),
          Text(
            '${engineLabel('Input black')} · ${engineLabel('Gamma')} · '
            '${engineLabel('Input white')}',
            style: t.small.copyWith(color: t.textMuted),
          ),
        ],
      ),
    );
  }
}

/// Triangles pointing up at the values they mark.
class _HandlePainter extends CustomPainter {
  final List<double> at;
  final List<Color> colours;
  final Color outline;

  const _HandlePainter({
    required this.at,
    required this.colours,
    required this.outline,
  });

  @override
  void paint(Canvas canvas, Size size) {
    for (var i = 0; i < at.length; i++) {
      final x = at[i] * size.width;
      final path = Path()
        ..moveTo(x, 0)
        ..lineTo(x - 4, size.height)
        ..lineTo(x + 4, size.height)
        ..close();
      canvas.drawPath(path, Paint()..color = colours[i]);
      canvas.drawPath(
        path,
        Paint()
          ..color = outline
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1,
      );
    }
  }

  @override
  bool shouldRepaint(_HandlePainter old) =>
      old.at.length != at.length ||
      List.generate(at.length, (i) => old.at[i] != at[i]).contains(true);
}

/// The output range: what black and white come out as.
class _OutputBarPainter extends CustomPainter {
  final double black, white;
  final Color low, high, edge;

  const _OutputBarPainter({
    required this.black,
    required this.white,
    required this.low,
    required this.high,
    required this.edge,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    canvas.drawRect(
      rect,
      Paint()
        ..shader = ui.Gradient.linear(
          rect.centerLeft,
          rect.centerRight,
          [
            Color.lerp(low, high, black.clamp(0.0, 1.0))!,
            Color.lerp(low, high, white.clamp(0.0, 1.0))!,
          ],
        ),
    );
    canvas.drawRect(
      rect,
      Paint()
        ..color = edge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_OutputBarPainter old) =>
      old.black != black || old.white != white;
}
