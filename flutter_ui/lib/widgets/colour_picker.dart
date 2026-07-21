// A house HSV colour picker: a saturation/value square, a hue strip, a
// current-vs-picked preview and a hex field, all kept in sync. Built to the
// same borderless, theme-driven style as the rest of controls.dart, and shaped
// so a future "edit every editor colour" submenu can reuse it (checklist F0).
//
// In plain terms: the big square chooses how vivid and how bright the colour
// is, the rainbow strip chooses the hue, and the hex box lets you type or read
// the exact value. Nothing here carries a fixed colour of its own — every
// swatch is built from the numbers the user is choosing, so the theme rule
// (only theme.dart may hold colour constants) still holds.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../theme/theme.dart';
import 'controls.dart';

/// A hue/saturation/value triple: `h` in [0, 360), `s` and `v` in [0, 1].
typedef Hsv = (double h, double s, double v);

/// Convert HSV to an opaque RGB colour. Thin wrapper over the standard
/// sextant formula so tests can pin an exact conversion table.
Color hsvToRgb(double h, double s, double v) {
  var hue = h % 360;
  if (hue < 0) hue += 360;
  final c = v * s;
  final x = c * (1 - ((hue / 60) % 2 - 1).abs());
  final m = v - c;
  double rr, gg, bb;
  if (hue < 60) {
    rr = c;
    gg = x;
    bb = 0;
  } else if (hue < 120) {
    rr = x;
    gg = c;
    bb = 0;
  } else if (hue < 180) {
    rr = 0;
    gg = c;
    bb = x;
  } else if (hue < 240) {
    rr = 0;
    gg = x;
    bb = c;
  } else if (hue < 300) {
    rr = x;
    gg = 0;
    bb = c;
  } else {
    rr = c;
    gg = 0;
    bb = x;
  }
  int ch(double f) => ((f + m) * 255).round().clamp(0, 255);
  return documentColour(ch(rr), ch(gg), ch(bb), 0xff);
}

/// Convert an RGB colour to HSV. For greys (`delta == 0`) the hue is 0 —
/// callers that want to preserve a chosen hue should check `s > 0` first.
Hsv rgbToHsv(Color colour) {
  final r = colour.r, g = colour.g, b = colour.b;
  final maxC = math.max(r, math.max(g, b));
  final minC = math.min(r, math.min(g, b));
  final delta = maxC - minC;
  double h;
  if (delta == 0) {
    h = 0;
  } else if (maxC == r) {
    h = 60 * (((g - b) / delta) % 6);
  } else if (maxC == g) {
    h = 60 * ((b - r) / delta + 2);
  } else {
    h = 60 * ((r - g) / delta + 4);
  }
  if (h < 0) h += 360;
  final s = maxC == 0 ? 0.0 : delta / maxC;
  return (h, s, maxC);
}

final RegExp _hexPattern = RegExp(r'^#?([0-9a-fA-F]{6})$');

/// Parse an RRGGBB hex string (a leading `#` is tolerated) to an opaque
/// colour, or null when the input is not exactly six hex digits.
Color? parseHex(String input) {
  final match = _hexPattern.firstMatch(input.trim());
  if (match == null) return null;
  final value = int.parse(match.group(1)!, radix: 16);
  return documentColour(
      (value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff, 0xff);
}

/// Format a colour as an upper-case RRGGBB string (no `#`, alpha dropped).
String formatHex(Color colour) {
  int ch(double f) => (f * 255).round().clamp(0, 255);
  String two(int v) => v.toRadixString(16).padLeft(2, '0');
  return '${two(ch(colour.r))}${two(ch(colour.g))}${two(ch(colour.b))}'
      .toUpperCase();
}

const double _pickerWidth = 208;
const double _squareHeight = 150;
const double _stripHeight = 16;

/// Open the colour picker near [position], seeded with [initial]. Completes
/// with the chosen colour on OK, or null when dismissed (outside click or
/// Escape). [presets] draws an optional row of quick swatches inside the
/// popup. Applying the result is the caller's job.
Future<Color?> showColourPicker({
  required BuildContext context,
  required Offset position,
  required Color initial,
  List<Color> presets = const [],
}) {
  return showLumitPopup<Color>(
    context: context,
    position: position,
    builder: (close) => FloatSurface(
      // An explicit inner width bounds the stretched column — a float in the
      // overlay has unbounded width, which otherwise crashes on layout
      // (the BareDropdown note in controls.dart).
      child: SizedBox(
        width: _pickerWidth,
        child: _ColourPickerBody(
          initial: initial,
          presets: presets,
          onCommit: close,
        ),
      ),
    ),
  );
}

class _ColourPickerBody extends StatefulWidget {
  final Color initial;
  final List<Color> presets;
  final ValueChanged<Color?> onCommit;

  const _ColourPickerBody({
    required this.initial,
    required this.presets,
    required this.onCommit,
  });

  @override
  State<_ColourPickerBody> createState() => _ColourPickerBodyState();
}

class _ColourPickerBodyState extends State<_ColourPickerBody> {
  late double _h, _s, _v;
  late final TextEditingController _hexController;
  final FocusNode _hexFocus = FocusNode();

  @override
  void initState() {
    super.initState();
    final hsv = rgbToHsv(widget.initial);
    _h = hsv.$1;
    _s = hsv.$2;
    _v = hsv.$3;
    _hexController = TextEditingController(text: formatHex(widget.initial));
    _hexFocus.addListener(() {
      if (!_hexFocus.hasFocus) _commitHex();
    });
  }

  @override
  void dispose() {
    _hexController.dispose();
    _hexFocus.dispose();
    super.dispose();
  }

  Color get _colour => hsvToRgb(_h, _s, _v);

  /// Push the current colour back into the hex field unless the user is
  /// typing there right now.
  void _syncHex() {
    if (!_hexFocus.hasFocus) _hexController.text = formatHex(_colour);
  }

  void _setSV(Offset local) {
    setState(() {
      _s = (local.dx / _pickerWidth).clamp(0.0, 1.0);
      _v = 1 - (local.dy / _squareHeight).clamp(0.0, 1.0);
      _syncHex();
    });
  }

  void _setHue(Offset local) {
    setState(() {
      _h = (local.dx / _pickerWidth).clamp(0.0, 1.0) * 360;
      _syncHex();
    });
  }

  void _setColour(Color colour) {
    final hsv = rgbToHsv(colour);
    setState(() {
      // Keep the chosen hue when the pick is a pure grey (hue undefined).
      if (hsv.$2 > 0) _h = hsv.$1;
      _s = hsv.$2;
      _v = hsv.$3;
      _syncHex();
    });
  }

  /// Live-parse while the user types: update the pick on a valid hex, leaving
  /// the field text alone so the caret does not jump.
  void _onHexTyped(String text) {
    final parsed = parseHex(text);
    if (parsed == null) return;
    final hsv = rgbToHsv(parsed);
    setState(() {
      if (hsv.$2 > 0) _h = hsv.$1;
      _s = hsv.$2;
      _v = hsv.$3;
    });
  }

  void _commitHex() {
    final parsed = parseHex(_hexController.text);
    if (parsed != null) _setColour(parsed);
    // Snap the field back to the canonical form (or the unchanged colour on a
    // rejected entry).
    _hexController.text = formatHex(_colour);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // Saturation / value square.
        GestureDetector(
          key: const Key('colour-picker-square'),
          behavior: HitTestBehavior.opaque,
          onTapDown: (d) => _setSV(d.localPosition),
          onPanStart: (d) => _setSV(d.localPosition),
          onPanUpdate: (d) => _setSV(d.localPosition),
          child: SizedBox(
            width: _pickerWidth,
            height: _squareHeight,
            child: CustomPaint(
              painter: _SvSquarePainter(hue: _h, s: _s, v: _v),
            ),
          ),
        ),
        const SizedBox(height: 8),
        // Hue strip.
        GestureDetector(
          key: const Key('colour-picker-strip'),
          behavior: HitTestBehavior.opaque,
          onTapDown: (d) => _setHue(d.localPosition),
          onPanStart: (d) => _setHue(d.localPosition),
          onPanUpdate: (d) => _setHue(d.localPosition),
          child: SizedBox(
            width: _pickerWidth,
            height: _stripHeight,
            child: CustomPaint(painter: _HueStripPainter(hue: _h)),
          ),
        ),
        const SizedBox(height: 8),
        // Preview: was / now, then the hex field.
        Row(
          children: [
            _previewSwatch(t, widget.initial, 'was'),
            const SizedBox(width: 4),
            _previewSwatch(t, _colour, 'now'),
            const Spacer(),
            _hexField(t),
          ],
        ),
        if (widget.presets.isNotEmpty) ...[
          const SizedBox(height: 8),
          _presetRow(t),
        ],
        const SizedBox(height: 8),
        Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            HouseButton(
              small: true,
              onPressed: () => widget.onCommit(null),
              child: const Text('Cancel'),
            ),
            const SizedBox(width: 6),
            HouseButton(
              small: true,
              onPressed: () => widget.onCommit(_colour),
              child: const Text('OK'),
            ),
          ],
        ),
      ],
    );
  }

  Widget _previewSwatch(LumitTheme t, Color colour, String label) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 28,
            height: 18,
            decoration: BoxDecoration(
              color: colour,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              border: Border.all(color: t.hairlineStrong),
            ),
          ),
          const SizedBox(height: 2),
          Text(label, style: t.small),
        ],
      );

  Widget _hexField(LumitTheme t) => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text('#', style: t.small),
          const SizedBox(width: 2),
          SizedBox(
            width: 72,
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
              decoration: BoxDecoration(
                color: t.surface0,
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                border: Border.all(
                  color: _hexFocus.hasFocus ? t.accent : t.hairline,
                ),
              ),
              child: EditableText(
                controller: _hexController,
                focusNode: _hexFocus,
                style: t.bodyPrimary,
                cursorColor: t.accent,
                backgroundCursorColor: t.surface2,
                selectionColor: t.accent.withValues(alpha: 0.5),
                // Fold a valid hex into the pick as it is typed, without
                // reformatting the field mid-entry; focus loss then snaps it
                // to canonical form.
                onChanged: _onHexTyped,
                onSubmitted: (_) => _commitHex(),
              ),
            ),
          ),
        ],
      );

  Widget _presetRow(LumitTheme t) => Row(
        children: [
          for (final c in widget.presets)
            Padding(
              padding: const EdgeInsets.only(right: 4),
              child: GestureDetector(
                onTap: () => _setColour(c),
                child: Container(
                  width: 18,
                  height: 18,
                  decoration: BoxDecoration(
                    color: c,
                    borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                    border: Border.all(color: t.hairlineStrong),
                  ),
                ),
              ),
            ),
        ],
      );
}

/// The saturation/value field: a white→pure-hue horizontal gradient under a
/// transparent→black vertical overlay, with a ring at the current pick.
class _SvSquarePainter extends CustomPainter {
  final double hue, s, v;
  const _SvSquarePainter({required this.hue, required this.s, required this.v});

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    final white = documentColour(0xff, 0xff, 0xff, 0xff);
    final pureHue = hsvToRgb(hue, 1, 1);
    final transparent = documentColour(0, 0, 0, 0);
    final black = documentColour(0, 0, 0, 0xff);

    canvas.drawRect(
      rect,
      Paint()
        ..shader = LinearGradient(colors: [white, pureHue]).createShader(rect),
    );
    canvas.drawRect(
      rect,
      Paint()
        ..shader = LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [transparent, black],
        ).createShader(rect),
    );

    final cx = (s * size.width).clamp(0.0, size.width);
    final cy = ((1 - v) * size.height).clamp(0.0, size.height);
    _drawRing(canvas, Offset(cx, cy));
  }

  @override
  bool shouldRepaint(_SvSquarePainter old) =>
      old.hue != hue || old.s != s || old.v != v;
}

/// The six-stop HSV rainbow, with a ring at the current hue.
class _HueStripPainter extends CustomPainter {
  final double hue;
  const _HueStripPainter({required this.hue});

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    final stops = [
      for (var h = 0; h <= 360; h += 60) hsvToRgb(h.toDouble(), 1, 1),
    ];
    canvas.drawRect(
      rect,
      Paint()..shader = LinearGradient(colors: stops).createShader(rect),
    );
    final cx = (hue / 360 * size.width).clamp(0.0, size.width);
    _drawRing(canvas, Offset(cx, size.height / 2));
  }

  @override
  bool shouldRepaint(_HueStripPainter old) => old.hue != hue;
}

/// A two-tone marker ring: a black outer stroke over a white inner one, so it
/// stays visible on any underlying colour.
void _drawRing(Canvas canvas, Offset centre) {
  final black = documentColour(0, 0, 0, 0xff);
  final white = documentColour(0xff, 0xff, 0xff, 0xff);
  canvas.drawCircle(
    centre,
    5.5,
    Paint()
      ..color = black
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2.5,
  );
  canvas.drawCircle(
    centre,
    5.5,
    Paint()
      ..color = white
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5,
  );
}
