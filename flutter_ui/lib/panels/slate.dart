// The missing-footage slate (phase F2, docs/07-UI-SPEC.md §3.3), colours ported
// one-for-one from crates/lumit-media/src/slate.rs. When footage goes missing a
// comp must not silently turn black — black reads as a deliberate edit and hides
// the mistake until export. Broadcast test bars are the opposite: unmistakably
// "no signal here". The pattern is drawn (never a bundled asset), from the
// engine's own bar colours via `documentColour` — it is document content, not
// chrome, so it never touches the theme.

import 'package:flutter/widgets.dart';

import '../theme/theme.dart';

// The bar colour table, digit-for-digit from slate.rs.
final _white = documentColour(255, 255, 255, 255);
final _yellow = documentColour(255, 240, 20, 255);
final _cyan = documentColour(20, 240, 255, 255);
final _green = documentColour(20, 220, 60, 255);
final _magenta = documentColour(220, 20, 120, 255);
final _red = documentColour(230, 20, 20, 255);
final _blue = documentColour(20, 20, 220, 255);
final _black = documentColour(0, 0, 0, 255);

/// The seven main bars, left to right — the classic descending-luminance run.
final List<Color> _bars = [
  _white,
  _yellow,
  _cyan,
  _green,
  _magenta,
  _red,
  _blue
];

/// The reversed band beneath, which is what makes it read as test bars.
final List<Color> _under = [
  _blue,
  _magenta,
  _yellow,
  _red,
  _cyan,
  _black,
  _white
];

/// Hue-sweep stops for the narrow strip beside the greyscale ramp.
final List<Color> _hueStops = [
  _red,
  _yellow,
  _green,
  _cyan,
  _blue,
  _magenta,
  _red
];

// Band boundaries as fractions of the height, and the ramp/wedge split, exactly
// as slate.rs defines them.
const double _bandUnder = 0.72;
const double _bandRamp = 0.80;
const double _bandSteps = 0.86;
const double _rampSplit = 0.58;
const int _steps = 12;

/// The translucent strip and text behind the missing-footage path overlay, and
/// the flat backdrop of the unreadable slate — all document colours, never the
/// theme (a slate is document content shown on the neutral surround).
final Color documentColourStrip = documentColour(0, 0, 0, 150);
final Color documentColourStripText = documentColour(255, 255, 255, 255);
final Color documentColourFailBg = documentColour(18, 18, 18, 255);

/// Paints the generated colour-bars slate, scaled to fill [rect]. Drawn as bands
/// of rectangles and gradients rather than per-pixel, so it stays cheap at any
/// size while reproducing slate.rs band-for-band.
class SlatePainter extends CustomPainter {
  const SlatePainter();

  @override
  void paint(Canvas canvas, Size size) {
    final w = size.width, h = size.height;
    final colW = w / 7.0;
    final paint = Paint()..style = PaintingStyle.fill;

    // Top band: the seven main bars.
    final underTop = h * _bandUnder;
    for (var c = 0; c < 7; c++) {
      paint.color = _bars[c];
      canvas.drawRect(
          Rect.fromLTRB(c * colW, 0, (c + 1) * colW, underTop), paint);
    }

    // Reversed band beneath.
    final rampTop = h * _bandRamp;
    for (var c = 0; c < 7; c++) {
      paint.color = _under[c];
      canvas.drawRect(
          Rect.fromLTRB(c * colW, underTop, (c + 1) * colW, rampTop), paint);
    }

    // Ramp band: greyscale ramp on the left of the split, hue sweep on the right.
    final stepsTop = h * _bandSteps;
    final splitX = w * _rampSplit;
    final greyRect = Rect.fromLTRB(0, rampTop, splitX, stepsTop);
    canvas.drawRect(
      greyRect,
      Paint()
        ..shader = LinearGradient(
          colors: [_white, _black],
        ).createShader(greyRect),
    );
    final hueRect = Rect.fromLTRB(splitX, rampTop, w, stepsTop);
    canvas.drawRect(
      hueRect,
      Paint()..shader = LinearGradient(colors: _hueStops).createShader(hueRect),
    );

    // Bottom band: stepped grey wedge on the left, black rest field on the right.
    final stepW = splitX / _steps;
    for (var s = 0; s < _steps; s++) {
      final v = (255 * s ~/ (_steps - 1));
      paint.color = documentColour(v, v, v, 255);
      canvas.drawRect(
          Rect.fromLTRB(s * stepW, stepsTop, (s + 1) * stepW, h), paint);
    }
    paint.color = _black;
    canvas.drawRect(Rect.fromLTRB(splitX, stepsTop, w, h), paint);
  }

  @override
  bool shouldRepaint(covariant SlatePainter oldDelegate) => false;
}
