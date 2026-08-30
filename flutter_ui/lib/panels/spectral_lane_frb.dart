// The spectral lane mode (K-699): a layer's waveform lane drawn as a
// spectrogram — time along the lane, frequency up it, brightness the level —
// and the little store that remembers which of the three pictures each
// layer's lane shows.
//
// In plain terms: the plain wave says how loud, the multiwave stack hints at
// what, and the spectrogram *shows* what — kicks along the bottom edge, hats
// along the top, a voice in the middle. The engine summarises each file once
// (docs/09 §4, the tiled grid beside the peak pyramid) and answers the same
// window fetch the peaks answer, so the picture gains detail as the zoom
// closes in.
//
// The bytes become **one image**, not thousands of rectangles: each fetch is
// mapped through the theme's own band ramp into RGBA once, decoded to a GPU
// texture, and painted with a single `drawImageRect` inside the lane's own
// repaint boundary — which is what keeps the K-681 gates honest about it.

import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

/// Which picture a layer's audio lane draws (the board's per-layer chips):
/// the plain wave, the three-band multiwave stack, or the spectrogram.
enum LaneMode { wave, stack, spectral }

/// Every layer's lane-mode choice, for the session (K-699).
///
/// Session state like an open twirl, not project data: the mode changes no
/// sample and no export. Module-level for the same reason the sequence
/// clipboard is — it belongs to no one panel instance, and it has to survive
/// the panel rebuilding around it. The Settings multiwave toggle keeps
/// deciding what a fresh lane shows ([stackDefault], written by the panel);
/// the chip overrides it per layer.
class LaneModes extends ChangeNotifier {
  final Map<String, LaneMode> _byLayer = {};

  /// What a layer with no choice of its own shows — the Settings multiwave
  /// toggle, handed in by the panel that reads it (K-184: read once, not per
  /// row build).
  bool stackDefault = true;

  LaneMode of(String layerId) =>
      _byLayer[layerId] ?? (stackDefault ? LaneMode.stack : LaneMode.wave);

  /// The chip's tap: wave → stack → spectral → wave.
  void cycle(String layerId) {
    _byLayer[layerId] = switch (of(layerId)) {
      LaneMode.wave => LaneMode.stack,
      LaneMode.stack => LaneMode.spectral,
      LaneMode.spectral => LaneMode.wave,
    };
    notifyListeners();
  }

  /// Every choice forgotten — a test's isolation, and a closed project's.
  void reset() {
    _byLayer.clear();
    notifyListeners();
  }
}

/// The session's one store.
final LaneModes laneModes = LaneModes();

/// One layer's spectrogram, as a texture the lane paints in a single call.
///
/// The widget owns the conversion: when a new answer (or a new theme ramp)
/// arrives, the bytes are mapped low-band-at-the-bottom through the ramp into
/// RGBA and decoded off the build; until the image lands the lane simply
/// draws nothing, which is what it drew before the fetch answered anyway.
class SpectralLane extends StatefulWidget {
  final BridgeSpectrogram? grid;

  /// The grid's own clock at canvas x = 0, and the zoom — the same mapping
  /// the waveform painter takes, so the two modes agree about where a moment
  /// is.
  final double originSeconds;
  final double secondsPerPixel;

  /// The columns to draw between — the visible part of the layer's bar.
  final double left;
  final double right;

  /// How tall to draw, anchored to the bottom of the row (K-437's borrowed
  /// pair, exactly as the wave uses it).
  final double height;

  const SpectralLane({
    super.key,
    required this.grid,
    required this.originSeconds,
    required this.secondsPerPixel,
    required this.left,
    required this.right,
    required this.height,
  });

  @override
  State<SpectralLane> createState() => _SpectralLaneState();
}

class _SpectralLaneState extends State<SpectralLane> {
  ui.Image? _image;

  /// What [_image] was made from, so an unchanged answer is not re-decoded.
  BridgeSpectrogram? _imageOf;
  WaveformColours? _rampOf;

  @override
  void dispose() {
    _image?.dispose();
    super.dispose();
  }

  void _wantImage(WaveformColours ramp) {
    final grid = widget.grid;
    if (grid == null || grid.columns == 0 || grid.bins == 0) return;
    if (identical(grid, _imageOf) && ramp == _rampOf) return;
    _imageOf = grid;
    _rampOf = ramp;
    final cols = grid.columns;
    final bins = grid.bins;
    // The ramp per band, worked out once: the multiwave's own three colours,
    // bass to treble, blended across the bins — so the spectrogram and the
    // stack speak the same colour language and a theme owns both.
    final stops = [ramp.low, ramp.mid, ramp.high];
    final colours = List<Color>.generate(bins, (b) {
      final at = bins <= 1 ? 0.0 : b / (bins - 1) * (stops.length - 1);
      final i = at.floor().clamp(0, stops.length - 2);
      return Color.lerp(stops[i], stops[i + 1], at - i)!;
    });
    final rgba = Uint8List(cols * bins * 4);
    for (var c = 0; c < cols; c++) {
      for (var b = 0; b < bins; b++) {
        final v = grid.values[c * bins + b];
        if (v == 0) continue;
        // Low band at the bottom of the picture.
        final y = bins - 1 - b;
        final at = (y * cols + c) * 4;
        final colour = colours[b];
        rgba[at] = (colour.r * 255).round();
        rgba[at + 1] = (colour.g * 255).round();
        rgba[at + 2] = (colour.b * 255).round();
        rgba[at + 3] = v;
      }
    }
    ui.decodeImageFromPixels(rgba, cols, bins, ui.PixelFormat.rgba8888,
        (image) {
      if (!mounted || !identical(_imageOf, grid)) {
        image.dispose();
        return;
      }
      setState(() {
        _image?.dispose();
        _image = image;
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    _wantImage(t.waveform);
    return CustomPaint(
      painter: _SpectralPainter(
        image: _image,
        grid: widget.grid,
        originSeconds: widget.originSeconds,
        secondsPerPixel: widget.secondsPerPixel,
        left: widget.left,
        right: widget.right,
        height: widget.height,
        // The board's own caption: what the picture's height spans.
        caption: l10n.spectralRange,
        captionStyle: t.mono.copyWith(fontSize: 8, color: t.textMuted),
      ),
    );
  }
}

class _SpectralPainter extends CustomPainter {
  final ui.Image? image;
  final BridgeSpectrogram? grid;
  final double originSeconds;
  final double secondsPerPixel;
  final double left;
  final double right;
  final double height;
  final String caption;
  final TextStyle captionStyle;

  const _SpectralPainter({
    required this.image,
    required this.grid,
    required this.originSeconds,
    required this.secondsPerPixel,
    required this.left,
    required this.right,
    required this.height,
    required this.caption,
    required this.captionStyle,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final held = image;
    final window = grid;
    if (held == null || window == null || secondsPerPixel <= 0) return;
    if (!(window.endSeconds > window.startSeconds)) return;
    final from = left.clamp(0.0, size.width);
    final to = right.clamp(0.0, size.width);
    if (!(to > from)) return;
    // Where the fetched window lands on the canvas, in the shared clock.
    final destLeft = (window.startSeconds - originSeconds) / secondsPerPixel;
    final destRight = (window.endSeconds - originSeconds) / secondsPerPixel;
    canvas.save();
    // Only the layer's own bar shows its sound, exactly as the wave clips.
    canvas.clipRect(Rect.fromLTRB(from, size.height - height, to, size.height));
    canvas.drawImageRect(
      held,
      Rect.fromLTWH(0, 0, held.width.toDouble(), held.height.toDouble()),
      Rect.fromLTRB(destLeft, size.height - height, destRight, size.height),
      Paint()..filterQuality = FilterQuality.low,
    );
    canvas.restore();
    // The caption, at the top-right of the drawn band — dropped when the
    // visible slice is too narrow to carry writing.
    if (to - from > 90) {
      final painter = TextPainter(
        text: TextSpan(text: caption, style: captionStyle),
        textDirection: TextDirection.ltr,
      )..layout();
      painter.paint(
          canvas, Offset(to - painter.width - 6, size.height - height + 1));
    }
  }

  @override
  bool shouldRepaint(_SpectralPainter old) =>
      !identical(old.image, image) ||
      !identical(old.grid, grid) ||
      old.originSeconds != originSeconds ||
      old.secondsPerPixel != secondsPerPixel ||
      old.left != left ||
      old.right != right ||
      old.height != height ||
      old.caption != caption ||
      old.captionStyle != captionStyle;

  /// A picture, not a control — the volume band above keeps its gestures.
  @override
  bool? hitTest(Offset position) => false;
}
