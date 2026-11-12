// Denoise's own display: the patch the noise was measured in, and what was
// measured there.
//
// **In plain terms.** Every slider under it is a share of the noise this
// card shows, so the card is where to look first: the square of the shot that
// was read, blown up so its grain can be seen, and the noise it holds as bars,
// fine to broad, for brightness and for colour. Underneath, one line says how
// far the reading can be trusted, or that one is being taken.
//
// **Presentation only.** The bars are drawn from the effect's own rows, the
// ones under the Noise by size twirl; the picture and the quality come from
// the engine's last measure of this effect, which a press of Learn or a move
// of the square on the Viewer makes.

import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/denoise.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show scalarValueAt;
import 'status_poller.dart';
import 'viewer_noise_sample.dart' show noiseMeasures;
import 'package:lumit_flutter/src/rust/api/composition.dart';

/// The plot's size: the Levels histogram's, so the two cards line up.
const double _plotHeight = 54;
const double _barsWidth = 146;

/// The four sizes, fine first, as the rows spell them.
const List<String> _sizes = ['fine', 'medium', 'coarse', 'broad'];

class DenoiseDisplayFrb extends StatefulWidget {
  final LayerReference layer;
  final UuidValue effectId;
  final CompositionReference comp;
  final int playheadFrame;

  /// The effect's values, a drag in flight included.
  final Map<String, BridgeEffectValue> values;

  /// Bumped by the panel when one of the effect's buttons is pressed.
  final int pressed;
  final VoidCallback onChanged;

  const DenoiseDisplayFrb({
    super.key,
    required this.layer,
    required this.effectId,
    required this.comp,
    required this.playheadFrame,
    required this.values,
    required this.pressed,
    required this.onChanged,
  });

  @override
  State<DenoiseDisplayFrb> createState() => _DenoiseDisplayFrbState();
}

class _DenoiseDisplayFrbState extends State<DenoiseDisplayFrb>
    with StatusPoller<BridgeNoiseStatus, DenoiseDisplayFrb> {
  ui.Image? _patch;
  int _patchOf = -1;
  Timer? _peekSoon;

  @override
  BridgeNoiseStatus fetchStatus() =>
      noiseStatus(layer: widget.layer, effect: widget.effectId);

  @override
  bool isMoving(BridgeNoiseStatus? status) => status?.busy ?? false;

  @override
  VoidCallback get onChanged => widget.onChanged;

  @override
  bool shouldResample(DenoiseDisplayFrb old) =>
      old.pressed != widget.pressed || old.effectId != widget.effectId;

  @override
  void initState() {
    super.initState();
    // A measure asked for from a key, a menu or the square on the Viewer
    // never passes through this panel's button.
    noiseMeasures.addListener(sample);
    // A project just opened has the rows and no measure behind them. After
    // the poller's own first read, so the two do not ask at once.
    WidgetsBinding.instance.addPostFrameCallback((_) => _peek());
  }

  @override
  void didUpdateWidget(DenoiseDisplayFrb old) {
    super.didUpdateWidget(old);
    // The square moved without a measure (an undo, a typed row): show the
    // patch it stands on now, once the rows have settled.
    if (_square(old.values, old) != _square(widget.values, widget)) {
      _peekSoon?.cancel();
      _peekSoon = Timer(const Duration(milliseconds: 250), _peek);
    }
  }

  /// The Sample rows at the playhead, as one comparable value.
  static String _square(
      Map<String, BridgeEffectValue> values, DenoiseDisplayFrb of) {
    double at(String id) {
      final v = values[id];
      final s = v is BridgeEffectValue_Float ? v.field0 : null;
      return scalarValueAt(s, of.comp, of.playheadFrame) ?? 0;
    }

    return '${at('sample_x').round()} ${at('sample_y').round()} '
        '${at('sample_size').round()}';
  }

  /// Ask the engine to read the square as it stands, writing nothing. It
  /// answers at once when the measure it holds is already of this square.
  Future<void> _peek() async {
    if (!mounted) return;
    try {
      final asked = denoisePeek(
          layer: widget.layer,
          effect: widget.effectId,
          frame: widget.playheadFrame);
      // Read once it has started, so the line says Measuring meanwhile.
      WidgetsBinding.instance.addPostFrameCallback((_) => sample());
      await asked;
    } catch (_) {
      // The layer or the effect went away under the card.
    }
    if (mounted) sample();
  }

  @override
  void dispose() {
    noiseMeasures.removeListener(sample);
    _peekSoon?.cancel();
    _patch?.dispose();
    super.dispose();
  }

  /// The patch as a picture, decoded once per measure.
  void _decode(BridgeNoiseStatus s) {
    if (s.generation == _patchOf || s.side == 0 || s.picture.isEmpty) return;
    _patchOf = s.generation;
    ui.decodeImageFromPixels(
      Uint8List.fromList(s.picture),
      s.side,
      s.side,
      ui.PixelFormat.rgba8888,
      (image) {
        if (!mounted) {
          image.dispose();
          return;
        }
        setState(() {
          _patch?.dispose();
          _patch = image;
        });
      },
    );
  }

  double _row(String id) {
    final v = widget.values[id];
    final s = v is BridgeEffectValue_Float ? v.field0 : null;
    return scalarValueAt(s, widget.comp, widget.playheadFrame) ?? 0;
  }

  /// Each band's noise in code values: the channel's level times the band's
  /// share of it.
  List<double> _bands(String channel) {
    final level = _row('${channel}_noise');
    return [for (final size in _sizes) level * _row('${channel}_$size') / 100];
  }

  String _line(BridgeNoiseStatus? s) {
    if (s == null) return '';
    if (s.busy) return l10n.denoiseMeasuring;
    if (s.failure != 0) return l10n.denoiseNoPicture;
    if (s.generation == 0) return '';
    final percent = (s.quality * 100).round();
    final why = switch (s.reason) {
      1 => l10n.denoiseReasonSmall,
      2 => l10n.denoiseReasonDetail,
      3 => l10n.denoiseReasonClipped,
      4 => l10n.denoiseReasonFaint,
      _ => '',
    };
    return why.isEmpty
        ? l10n.denoiseQuality(percent)
        : l10n.denoiseQualityBecause(percent, why);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final s = status;
    if (s != null) _decode(s);
    final luma = _bands('luma'), chroma = _bands('chroma');
    final line = _line(s);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                key: const ValueKey('fx-denoise-patch'),
                width: _plotHeight,
                height: _plotHeight,
                color: t.surface0,
                child: _patch == null
                    ? null
                    : RawImage(
                        image: _patch,
                        fit: BoxFit.cover,
                        // Nearest, so the grain is the grain and not a blur
                        // of it.
                        filterQuality: FilterQuality.none,
                      ),
              ),
              const SizedBox(width: 6),
              Flexible(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: _barsWidth),
                  child: Container(
                    key: const ValueKey('fx-denoise-bars'),
                    height: _plotHeight,
                    color: t.surface0,
                    child: CustomPaint(
                      size: const Size(double.infinity, _plotHeight),
                      painter: _BarsPainter(
                        luma: luma,
                        chroma: chroma,
                        lumaColour: t.textPrimary,
                        chromaColour: t.accent,
                        hairline: t.hairline,
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 3),
          ConstrainedBox(
            constraints:
                const BoxConstraints(maxWidth: _plotHeight + 6 + _barsWidth),
            child: Row(
              children: [
                const SizedBox(width: _plotHeight + 6),
                Expanded(
                  child: Text(engineLabel('Luma'),
                      textAlign: TextAlign.center,
                      style: t.small.copyWith(color: t.textMuted)),
                ),
                Expanded(
                  child: Text(engineLabel('Chroma'),
                      textAlign: TextAlign.center,
                      style: t.small.copyWith(color: t.textMuted)),
                ),
              ],
            ),
          ),
          if (line.isNotEmpty) ...[
            const SizedBox(height: 2),
            Text(
              line,
              key: const ValueKey('fx-denoise-status'),
              style: t.small.copyWith(
                color: t.textMuted,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ],
        ],
      ),
    );
  }
}

/// Eight bars in two groups of four, fine to broad, on one scale so luma and
/// chroma can be compared.
class _BarsPainter extends CustomPainter {
  final List<double> luma, chroma;
  final Color lumaColour, chromaColour, hairline;

  const _BarsPainter({
    required this.luma,
    required this.chroma,
    required this.lumaColour,
    required this.chromaColour,
    required this.hairline,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // Never less than four code values tall, so a clean shot reads as short
    // bars rather than as full ones scaled up to fill the box.
    var top = 4.0;
    for (final v in [...luma, ...chroma]) {
      if (v > top) top = v;
    }
    const gap = 3.0, groupGap = 10.0, pad = 6.0;
    final bar = (size.width - pad * 2 - groupGap - gap * 6) / 8;
    final floor = size.height - 4;
    canvas.drawLine(Offset(pad, floor + 0.5),
        Offset(size.width - pad, floor + 0.5), Paint()..color = hairline);
    var x = pad;
    for (final (values, colour) in [(luma, lumaColour), (chroma, chromaColour)]) {
      for (var i = 0; i < values.length; i++) {
        final h = (values[i] / top).clamp(0.0, 1.0) * (floor - 4);
        canvas.drawRect(Rect.fromLTWH(x, floor - h, bar, h),
            Paint()..color = colour.withValues(alpha: 0.85 - i * 0.12));
        x += bar + gap;
      }
      x += groupGap - gap;
    }
  }

  @override
  bool shouldRepaint(_BarsPainter old) =>
      old.lumaColour != lumaColour ||
      old.chromaColour != chromaColour ||
      old.hairline != hairline ||
      !_same(old.luma, luma) ||
      !_same(old.chroma, chroma);

  static bool _same(List<double> a, List<double> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}
