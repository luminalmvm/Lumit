// A house HSV colour picker, in the shape the egui build had: the red, green
// and blue numbers across the top, then the saturation/value square, the hue
// strip, and a hex field, all kept in sync.
//
// **In plain terms.** The three numbers at the top are the colour written out
// as red, green and blue; each can be dragged sideways or typed into, and the
// square and strip below follow. The big square chooses how vivid and how
// bright the colour is, the rainbow strip chooses the hue, and the hex box
// lets you type or read the exact value.
//
// **The numbers are the parameter's own.** A display colour — a theme colour, a
// solid's swatch — is 8-bit, so its channels read 0–255 and a hex is exact. A
// scene-linear colour in a float working depth is not: 0–1 is black to white,
// and a channel is free to go **above** 1 (an HDR tint, a glow overshoot) or
// below 0 (a lift) as far as the parameter's own declared range allows. So the
// picker is told which scale it is editing on, and shows the numbers in it.
//
// **It applies as you go.** Whatever the picker is showing is what the thing
// being coloured shows — there is no dialogue standing between choosing a
// colour and seeing it. A drag previews continuously and settles into one
// undoable edit when you let go, exactly as dragging a number in Effect
// controls does; clicking away from the picker closes it and keeps what is
// applied. Cancel is the way back: it puts the colour the picker opened with
// back and closes.
//
// Nothing here carries a fixed colour of its own — every swatch is built from
// the numbers the user is choosing, so the theme rule (only theme.dart may hold
// colour constants) still holds.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
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

/// How the picker's three numbers are shown and edited.
///
/// Not a cosmetic choice: it is the difference between a colour that *is* eight
/// bits and one that is a float, and the second cannot be typed at all on a
/// 0–255 dial.
enum ColourScale {
  /// 0–255 integers — a display colour, where a hex is the same value said
  /// another way. Theme colours and a solid's swatch.
  bytes,

  /// 0–1 for black to white, as decimals, and free to leave that range where
  /// the parameter allows it. What a scene-linear colour is in a float working
  /// depth (fp16 today, docs/06 §3.1): an HDR tint really does sit above 1, and
  /// clamping it at white in the picker is losing the value.
  unit,
}

/// A colour the picker is editing, in the space of whatever it is editing.
///
/// Not a `Color`: that is eight bits a channel and cannot hold 2.4, which a
/// scene-linear HDR tint legitimately is. [clipped] is the same colour as
/// something to *draw* — every swatch on screen is a display colour, whatever
/// the numbers behind it say.
@immutable
class PickedColour {
  final double r, g, b;

  const PickedColour(this.r, this.g, this.b);

  PickedColour.of(Color colour)
      : r = colour.r,
        g = colour.g,
        b = colour.b;

  /// The colour as it can be shown: each channel clamped into 0–1. Anything
  /// outside that is not a colour a screen has.
  Color get clipped => documentColour(
        (r.clamp(0.0, 1.0) * 255).round(),
        (g.clamp(0.0, 1.0) * 255).round(),
        (b.clamp(0.0, 1.0) * 255).round(),
        0xff,
      );

  /// True when a channel lies outside what a screen can show, so the swatch and
  /// the hex are both standing in for something bigger.
  bool get outOfGamut => r < 0 || g < 0 || b < 0 || r > 1 || g > 1 || b > 1;

  /// The brightest channel, or 1 — the factor the square and the strip are read
  /// through, so an over-range colour still has a place on them.
  double get gain {
    final peak = math.max(r, math.max(g, b));
    return peak > 1 ? peak : 1;
  }

  @override
  bool operator ==(Object other) =>
      other is PickedColour && other.r == r && other.g == g && other.b == b;

  @override
  int get hashCode => Object.hash(r, g, b);

  @override
  String toString() => 'PickedColour($r, $g, $b)';
}

const double _pickerWidth = 232;
const double _squareHeight = 150;
const double _stripHeight = 16;

/// Open the colour picker near [position], seeded with [initial].
///
/// **The result is not returned — it is applied as it changes.** [onPreview]
/// fires continuously while a drag is in flight (the same live-preview tick an
/// effect row's drag sends) and [onCommit] fires once each time a change
/// settles: a drag released, a number typed, a preset clicked, Apply pressed.
/// So closing the picker — by Apply, by Escape, or by clicking away from it —
/// needs nothing at all: what is applied is already what the picker last
/// settled on. Cancel commits [initial] back.
///
/// [scale] says what the three numbers mean, and [min]/[max] are the
/// parameter's own declared bounds — a colour that may reach 4 in linear light
/// says so, and the fields let it. Both are ignored under [ColourScale.bytes],
/// which is 0–255 by definition.
///
/// [presets] draws an optional row of quick swatches inside the popup. The
/// future completes when the picker closes, for a caller that wants to know.
Future<void> showColourPicker({
  required BuildContext context,
  required Offset position,
  required PickedColour initial,
  required ValueChanged<PickedColour> onCommit,
  ValueChanged<PickedColour>? onPreview,
  ColourScale scale = ColourScale.bytes,
  double min = 0,
  double max = 1,
  List<Color> presets = const [],
}) async {
  await showLumitPopup<PickedColour>(
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
          scale: scale,
          min: min,
          max: max,
          onPreview: onPreview ?? onCommit,
          onCommit: onCommit,
          onClose: () => close(null),
        ),
      ),
    ),
  );
}

class _ColourPickerBody extends StatefulWidget {
  final PickedColour initial;
  final List<Color> presets;
  final ColourScale scale;
  final double min, max;

  /// A tick of a drag: show it, do not record it.
  final ValueChanged<PickedColour> onPreview;

  /// A change that has settled: one undoable edit.
  final ValueChanged<PickedColour> onCommit;

  final VoidCallback onClose;

  const _ColourPickerBody({
    required this.initial,
    required this.presets,
    required this.scale,
    required this.min,
    required this.max,
    required this.onPreview,
    required this.onCommit,
    required this.onClose,
  });

  @override
  State<_ColourPickerBody> createState() => _ColourPickerBodyState();
}

class _ColourPickerBodyState extends State<_ColourPickerBody> {
  /// The chosen colour, in the scale being edited. Held as three channels
  /// rather than as hue/saturation/value because the channels are what the
  /// parameter stores and what the numbers show — and because a channel above
  /// white has no place on a 0–1 value dial.
  late PickedColour _colour;

  /// The hue, carried separately: it is undefined for a grey, so rebuilding it
  /// from the channels would swing the square to red the moment saturation
  /// reached zero.
  late double _hue;

  late final TextEditingController _hexController;
  final FocusNode _hexFocus = FocusNode();

  @override
  void initState() {
    super.initState();
    _colour = widget.initial;
    _hue = rgbToHsv(_colour.clipped).$1;
    _hexController = TextEditingController(text: formatHex(_colour.clipped));
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

  /// The colour as the square and the strip see it: divided down by its own
  /// peak, so an over-range colour still lands somewhere on them. Editing them
  /// multiplies that peak back in, so dragging the square about does not
  /// quietly throw away a value above white.
  Hsv get _hsv {
    final g = _colour.gain;
    final normalised = documentColour(
      ((_colour.r / g).clamp(0.0, 1.0) * 255).round(),
      ((_colour.g / g).clamp(0.0, 1.0) * 255).round(),
      ((_colour.b / g).clamp(0.0, 1.0) * 255).round(),
      0xff,
    );
    final hsv = rgbToHsv(normalised);
    return (_hue, hsv.$2, hsv.$3);
  }

  /// Push the current colour back into the hex field unless the user is
  /// typing there right now.
  void _syncHex() {
    if (!_hexFocus.hasFocus) _hexController.text = formatHex(_colour.clipped);
  }

  /// Take a new colour and show it everywhere: the square, the strip, the
  /// numbers, the hex box — and the thing being coloured.
  void _apply(PickedColour next, {required bool settled, double? hue}) {
    final clamped = PickedColour(
      next.r.clamp(widget.min, widget.max),
      next.g.clamp(widget.min, widget.max),
      next.b.clamp(widget.min, widget.max),
    );
    setState(() {
      if (hue != null) _hue = hue;
      _colour = clamped;
      _syncHex();
    });
    settled ? widget.onCommit(clamped) : widget.onPreview(clamped);
  }

  /// A colour chosen on the square or the strip, in 0–1, multiplied back up by
  /// whatever the colour was over-range by.
  void _fromHsv(double h, double s, double v, {required bool settled}) {
    final gain = _colour.gain;
    final base = hsvToRgb(h, s, v);
    _apply(
      PickedColour(base.r * gain, base.g * gain, base.b * gain),
      settled: settled,
      hue: h,
    );
  }

  void _setSV(Offset local, {required bool settled}) {
    final hsv = _hsv;
    _fromHsv(
      hsv.$1,
      (local.dx / _pickerWidth).clamp(0.0, 1.0),
      1 - (local.dy / _squareHeight).clamp(0.0, 1.0),
      settled: settled,
    );
  }

  void _setHue(Offset local, {required bool settled}) {
    final hsv = _hsv;
    _fromHsv(
      (local.dx / _pickerWidth).clamp(0.0, 1.0) * 360,
      hsv.$2,
      hsv.$3,
      settled: settled,
    );
  }

  /// A colour arriving whole — a preset, a hex, Cancel's restore.
  void _setColour(PickedColour colour, {required bool settled}) {
    final hsv = rgbToHsv(colour.clipped);
    // Keep the chosen hue when the pick is a pure grey (hue undefined) — the
    // square would otherwise jump to red the moment saturation reached zero.
    _apply(colour, settled: settled, hue: hsv.$2 > 0 ? hsv.$1 : _hue);
  }

  /// One of the three channel numbers, in the scale being edited.
  void _setChannel(int index, double value, {required bool settled}) {
    final v = widget.scale == ColourScale.bytes ? value / 255 : value;
    _setColour(
      PickedColour(
        index == 0 ? v : _colour.r,
        index == 1 ? v : _colour.g,
        index == 2 ? v : _colour.b,
      ),
      settled: settled,
    );
  }

  /// Live-parse while the user types: update the pick on a valid hex, leaving
  /// the field text alone so the caret does not jump.
  void _onHexTyped(String text) {
    final parsed = parseHex(text);
    if (parsed == null) return;
    final hsv = rgbToHsv(parsed);
    setState(() {
      if (hsv.$2 > 0) _hue = hsv.$1;
      _colour = PickedColour.of(parsed);
    });
    widget.onPreview(_colour);
  }

  void _commitHex() {
    final parsed = parseHex(_hexController.text);
    if (parsed != null) _setColour(PickedColour.of(parsed), settled: true);
    // Snap the field back to the canonical form (or the unchanged colour on a
    // rejected entry).
    _hexController.text = formatHex(_colour.clipped);
  }

  /// What a channel field shows: bytes under the 8-bit scale, the value itself
  /// under the unit one.
  double _shown(double channel) => widget.scale == ColourScale.bytes
      ? (channel * 255).roundToDouble()
      : channel;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final bytes = widget.scale == ColourScale.bytes;
    final hsv = _hsv;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // The three numbers, above the graph, as the egui picker had them:
        // each draggable and typeable, and each one changes the colour under
        // the pointer immediately.
        Row(
          children: [
            _channelField(t, 'R', 0, _colour.r),
            const SizedBox(width: 4),
            _channelField(t, 'G', 1, _colour.g),
            const SizedBox(width: 4),
            _channelField(t, 'B', 2, _colour.b),
          ],
        ),
        const SizedBox(height: 8),
        // Saturation / value square.
        GestureDetector(
          key: const Key('colour-picker-square'),
          behavior: HitTestBehavior.opaque,
          onTapDown: (d) => _setSV(d.localPosition, settled: true),
          onPanStart: (d) => _setSV(d.localPosition, settled: false),
          onPanUpdate: (d) => _setSV(d.localPosition, settled: false),
          onPanEnd: (_) => widget.onCommit(_colour),
          child: SizedBox(
            width: _pickerWidth,
            height: _squareHeight,
            child: CustomPaint(
              painter: _SvSquarePainter(hue: hsv.$1, s: hsv.$2, v: hsv.$3),
            ),
          ),
        ),
        const SizedBox(height: 8),
        // Hue strip.
        GestureDetector(
          key: const Key('colour-picker-strip'),
          behavior: HitTestBehavior.opaque,
          onTapDown: (d) => _setHue(d.localPosition, settled: true),
          onPanStart: (d) => _setHue(d.localPosition, settled: false),
          onPanUpdate: (d) => _setHue(d.localPosition, settled: false),
          onPanEnd: (_) => widget.onCommit(_colour),
          child: SizedBox(
            width: _pickerWidth,
            height: _stripHeight,
            child: CustomPaint(painter: _HueStripPainter(hue: hsv.$1)),
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
        // A colour outside 0–1 has no screen value and no hex: the swatch and
        // the box above are both standing in for it, and saying so is better
        // than letting them read as the truth.
        if (!bytes && _colour.outOfGamut) ...[
          const SizedBox(height: 4),
          Text(
            l10n.colourOutsideRange,
            key: const Key('colour-picker-clipped'),
            style: t.small.copyWith(color: t.textMuted),
          ),
        ],
        if (widget.presets.isNotEmpty) ...[
          const SizedBox(height: 8),
          _presetRow(t),
        ],
        const SizedBox(height: 8),
        Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            HouseButton(
              key: const Key('colour-picker-cancel'),
              small: true,
              onPressed: () {
                // Put back what the picker opened with: the live changes have
                // already landed, so closing alone would keep them.
                widget.onCommit(widget.initial);
                widget.onClose();
              },
              child: Text(l10n.cancel),
            ),
            const SizedBox(width: 6),
            HouseButton(
              key: const Key('colour-picker-apply'),
              small: true,
              onPressed: () {
                widget.onCommit(_colour);
                widget.onClose();
              },
              child: Text(l10n.apply),
            ),
          ],
        ),
      ],
    );
  }

  /// One channel: a drag-and-type field, labelled by its letter, over whatever
  /// the scale and the parameter's own bounds allow.
  Widget _channelField(LumitTheme t, String label, int index, double channel) {
    final bytes = widget.scale == ColourScale.bytes;
    return Expanded(
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(label, style: t.small.copyWith(color: t.textMuted)),
          const SizedBox(width: 3),
          Expanded(
            child: DragValueField(
              key: Key('colour-picker-$label'),
              value: _shown(channel),
              min: bytes ? 0 : widget.min,
              max: bytes ? 255 : widget.max,
              // A byte steps by one; a unit channel by a hundredth, so black to
              // white is a comfortable drag rather than a twitch.
              speed: bytes ? 1 : 0.01,
              decimals: bytes ? 0 : 3,
              fill: t.surface0,
              onChanged: (v) => _setChannel(index, v.toDouble(), settled: true),
              onChangeLive: (v) =>
                  _setChannel(index, v.toDouble(), settled: false),
              onChangeEnd: (v) =>
                  _setChannel(index, v.toDouble(), settled: true),
            ),
          ),
        ],
      ),
    );
  }

  Widget _previewSwatch(LumitTheme t, PickedColour colour, String label) =>
      Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 28,
            height: 18,
            decoration: BoxDecoration(
              color: colour.clipped,
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
                // to canonical form and commits.
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
                onTap: () => _setColour(PickedColour.of(c), settled: true),
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
