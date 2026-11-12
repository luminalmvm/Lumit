// The scale a scope's trace is read against, drawn over the engine's picture.
//
// The positions here are the engine's own maths (`crates/lumit-gpu/src/
// scope.wgsl`): the vectorscope's Rec.709 Cb/Cr and its scale, and the CIE
// plot's window on the xy plane. Change one side and the other has to follow,
// or the trace leaves its boxes.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

/// The instruments a Scopes slot can show.
enum ScopeKind { waveform, parade, vectorscope, histogram, chromaticity }

/// How a waveform or histogram is numbered: 0 to 100, or 10-bit code values.
enum ScopeScale { percent, tenBit }

/// The vectorscope's reach: the engine plots Cb and Cr at 0.9 of the plot's
/// side per unit, so a fully saturated primary sits just inside the circle.
const double _vectorReach = 0.9;

/// The CIE plot's window on the xy plane: a square from [_xyMin], [_xySpan]
/// wide, as the engine bins it.
const double _xyMin = -0.03;
const double _xySpan = 0.9;

/// A colour's Rec.709 Cb and Cr, the way the engine's vectorscope reads it.
(double, double) _cbcr(double r, double g, double b) {
  final y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return ((b - y) / 1.8556, (r - y) / 1.5748);
}

/// Where a colour lands on a vectorscope drawn in [plot], at [zoom].
Offset vectorscopePoint(Rect plot, double r, double g, double b,
    {double zoom = 1}) {
  final (cb, cr) = _cbcr(r, g, b);
  final reach = plot.width * _vectorReach * zoom;
  return plot.center + Offset(cb * reach, -cr * reach);
}

/// Where an xy chromaticity lands on the CIE plot drawn in [plot].
Offset chromaticityPoint(Rect plot, double x, double y) => Offset(
      plot.left + (x - _xyMin) / _xySpan * plot.width,
      plot.top + (1 - (y - _xyMin) / _xySpan) * plot.height,
    );

/// A 75% bar, as an 8-bit picture carries it.
const double _bar = 191 / 255;

/// The six targets: each bar's colour and the mark the trade gives it.
const List<(double, double, double, String)> _targets = [
  (_bar, 0, 0, 'R'),
  (_bar, 0, _bar, 'Mg'),
  (0, 0, _bar, 'B'),
  (0, _bar, _bar, 'Cy'),
  (0, _bar, 0, 'G'),
  (_bar, _bar, 0, 'Yl'),
];

/// The skin tone line runs along the I axis of the old YIQ colour system,
/// whose positive end is this colour difference. Read through the same Cb/Cr
/// as the trace, so it points where skin actually lands.
const (double, double, double) _iAxis = (0.956, -0.272, -1.106);

/// The CIE 1931 2 degree spectral locus, 380 to 700 nm in 5 nm steps.
const List<(double, double)> _locus = [
  (0.1741, 0.0050),
  (0.1740, 0.0050),
  (0.1738, 0.0049),
  (0.1736, 0.0049),
  (0.1733, 0.0048),
  (0.1730, 0.0048),
  (0.1726, 0.0048),
  (0.1721, 0.0048),
  (0.1714, 0.0051),
  (0.1703, 0.0058),
  (0.1689, 0.0069),
  (0.1669, 0.0086),
  (0.1644, 0.0109),
  (0.1611, 0.0138),
  (0.1566, 0.0177),
  (0.1510, 0.0227),
  (0.1440, 0.0297),
  (0.1355, 0.0399),
  (0.1241, 0.0578),
  (0.1096, 0.0868),
  (0.0913, 0.1327),
  (0.0687, 0.2007),
  (0.0454, 0.2950),
  (0.0235, 0.4127),
  (0.0082, 0.5384),
  (0.0039, 0.6548),
  (0.0139, 0.7502),
  (0.0389, 0.8120),
  (0.0743, 0.8338),
  (0.1142, 0.8262),
  (0.1547, 0.8059),
  (0.1929, 0.7816),
  (0.2296, 0.7543),
  (0.2658, 0.7243),
  (0.3016, 0.6923),
  (0.3373, 0.6589),
  (0.3731, 0.6245),
  (0.4087, 0.5896),
  (0.4441, 0.5547),
  (0.4788, 0.5202),
  (0.5125, 0.4866),
  (0.5448, 0.4544),
  (0.5752, 0.4242),
  (0.6029, 0.3965),
  (0.6270, 0.3725),
  (0.6482, 0.3514),
  (0.6658, 0.3340),
  (0.6801, 0.3197),
  (0.6915, 0.3083),
  (0.7006, 0.2993),
  (0.7079, 0.2920),
  (0.7140, 0.2859),
  (0.7190, 0.2809),
  (0.7230, 0.2770),
  (0.7260, 0.2740),
  (0.7283, 0.2717),
  (0.7300, 0.2700),
  (0.7311, 0.2689),
  (0.7320, 0.2680),
  (0.7327, 0.2673),
  (0.7334, 0.2666),
  (0.7340, 0.2660),
  (0.7344, 0.2656),
  (0.7346, 0.2654),
  (0.7347, 0.2653),
];

/// The three gamuts drawn on the CIE plot, red, green and blue primaries, and
/// the name each is known by.
const List<(List<(double, double)>, String)> _gamuts = [
  ([(0.640, 0.330), (0.300, 0.600), (0.150, 0.060)], '709'),
  ([(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)], 'P3'),
  ([(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)], '2020'),
];

/// The D65 white point.
const (double, double) _d65 = (0.3127, 0.3290);

/// The scale a trace is read against: levels for the waveform and parade, the
/// colour targets and skin tone line for the vectorscope, the spectral locus
/// and the gamuts for the CIE plot, and the code range for the histogram.
///
/// The engine sends a picture of the measurement, and this draws the ruler
/// over it. Without one a waveform is a shape with no idea what counts as
/// black or white, and a vectorscope is a blob with no idea which way is red.
/// The numbers sit in the gutter beside the plot, never over the trace.
class ScopeGraticulePainter extends CustomPainter {
  final ScopeKind kind;
  final Rect plot;
  final Color line;
  final TextStyle label;
  final ScopeScale scale;
  final bool zoom;

  const ScopeGraticulePainter({
    required this.kind,
    required this.plot,
    required this.line,
    required this.label,
    this.scale = ScopeScale.percent,
    this.zoom = false,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = line
      ..strokeWidth = 1
      ..style = PaintingStyle.stroke;
    switch (kind) {
      // Both read as levels up the side, so both take the same scale. Black
      // and white are the two that matter, and the trace is clipped at them.
      case ScopeKind.waveform:
      case ScopeKind.parade:
        for (final (frac, name) in _levels) {
          final y = plot.top + plot.height * (1 - frac);
          canvas.drawLine(Offset(plot.left, y), Offset(plot.right, y), paint);
          _text(canvas, name, Offset(plot.left - 3, y), align: _Align.right);
        }
        // The parade's three cells, so it reads as R, G and B rather than one
        // wide trace. Each channel fills its own third in its own colour.
        if (kind == ScopeKind.parade) {
          for (var i = 1; i < 3; i++) {
            final x = plot.left + plot.width * i / 3;
            canvas.drawLine(Offset(x, plot.top), Offset(x, plot.bottom), paint);
          }
        }

      case ScopeKind.vectorscope:
        _vectorscope(canvas, paint);

      case ScopeKind.chromaticity:
        _chromaticity(canvas, paint);

      // Code value across, count up: only the horizontal axis carries meaning,
      // because the vertical is scaled to whatever the tallest bin happens to
      // be and a number on it would be a number about nothing.
      case ScopeKind.histogram:
        final top = scale == ScopeScale.tenBit ? 1023 : 255;
        for (final frac in const [0.0, 0.25, 0.5, 0.75, 1.0]) {
          final x = plot.left + plot.width * frac;
          canvas.drawLine(Offset(x, plot.top), Offset(x, plot.bottom), paint);
          _text(canvas, '${(top * frac).round()}', Offset(x, plot.bottom + 2),
              align: frac == 0
                  ? _Align.left
                  : frac == 1
                      ? _Align.right
                      : _Align.centre,
              below: true);
        }
    }
  }

  /// The waveform's lines: fractions of full scale and what each is called.
  List<(double, String)> get _levels => switch (scale) {
        ScopeScale.percent => const [
            (0, '0'),
            (0.25, '25'),
            (0.5, '50'),
            (0.75, '75'),
            (1, '100'),
          ],
        ScopeScale.tenBit => [
            for (final code in const [0, 256, 512, 768, 1023])
              (code / 1023, '$code'),
          ],
      };

  /// Hue round, saturation out from the middle: the rings and the cross are
  /// the frame of reference, the boxes are where 75% bars land, and the skin
  /// tone line is the one every colourist looks for.
  void _vectorscope(Canvas canvas, Paint paint) {
    final centre = plot.center;
    final radius = plot.shortestSide / 2;
    canvas.save();
    canvas.clipRect(plot);
    for (final r in const [0.25, 0.5, 0.75, 1.0]) {
      canvas.drawCircle(centre, radius * r, paint);
    }
    canvas.drawLine(
        Offset(centre.dx, plot.top), Offset(centre.dx, plot.bottom), paint);
    canvas.drawLine(
        Offset(plot.left, centre.dy), Offset(plot.right, centre.dy), paint);
    final z = zoom ? 2.0 : 1.0;
    final half = math.max(3.0, plot.width * 0.018);
    for (final (r, g, b, name) in _targets) {
      final p = vectorscopePoint(plot, r, g, b, zoom: z);
      canvas.drawRect(
          Rect.fromCenter(center: p, width: half * 2, height: half * 2), paint);
      // The name sits just outside its box, away from the centre.
      final out = p - centre;
      final away = out.distance == 0 ? Offset.zero : out / out.distance;
      _text(canvas, name, p + away * (half + 8), align: _Align.centre);
    }
    final (ir, ig, ib) = _iAxis;
    final skin = vectorscopePoint(plot, ir, ig, ib) - centre;
    canvas.drawLine(centre, centre + skin / skin.distance * radius, paint);
    canvas.restore();
  }

  /// The horseshoe of pure spectral colours closed by the line of purples,
  /// the three gamuts inside it, and the white point.
  void _chromaticity(Canvas canvas, Paint paint) {
    Offset at((double, double) xy) => chromaticityPoint(plot, xy.$1, xy.$2);
    final locus = Path()..addPolygon([for (final xy in _locus) at(xy)], true);
    canvas.drawPath(locus, paint);
    for (final (primaries, name) in _gamuts) {
      canvas.drawPath(
          Path()..addPolygon([for (final xy in primaries) at(xy)], true),
          paint);
      // Named beside its red corner, where the three are furthest apart.
      _text(canvas, name, at(primaries[0]) + const Offset(4, 0),
          align: _Align.left);
    }
    final white = at(_d65);
    canvas.drawLine(
        white - const Offset(4, 0), white + const Offset(4, 0), paint);
    canvas.drawLine(
        white - const Offset(0, 4), white + const Offset(0, 4), paint);
  }

  /// A label anchored at [at]: centred on its y, and set left, right or
  /// centred on its x; [below] hangs it under the point instead.
  void _text(Canvas canvas, String s, Offset at,
      {required _Align align, bool below = false}) {
    final p = TextPainter(
      text: TextSpan(text: s, style: label),
      textDirection: TextDirection.ltr,
    )..layout();
    final x = switch (align) {
      _Align.left => at.dx,
      _Align.right => at.dx - p.width,
      _Align.centre => at.dx - p.width / 2,
    };
    final y = below ? at.dy : at.dy - p.height / 2;
    p.paint(canvas, Offset(x, y));
  }

  @override
  bool shouldRepaint(ScopeGraticulePainter old) =>
      old.kind != kind ||
      old.line != line ||
      old.plot != plot ||
      old.scale != scale ||
      old.zoom != zoom ||
      old.label != label;

  /// A scale, not a control: clicks fall through to whatever is beneath.
  @override
  bool? hitTest(Offset position) => false;
}

enum _Align { left, right, centre }
