// The Stretch dialogue (Layer ▸ Stretch…, and a clip's Speed…).
//
// Two ways of asking the same question, linked (docs/04-RETIMING.md §12.1's
// `retime.set_segment_speed`, the numeric entry): a new **speed** in per cent,
// or the new **duration** the layer would have at it. Editing either well moves
// the other, because they are one number seen twice — at half speed a layer is
// twice as long, and a montage editor thinks in whichever of the two the shot
// is fighting them about.
//
// The dialogue decides nothing. It reads the length it starts from, collects a
// speed, and hands that speed back; the engine works out the span and rewrites
// the map (`LayerReference.stretch`), so the anchoring rule and the maths live
// in one place rather than two.
//
// A **clip** has no duration well. A clip's place and length are fixed by the
// beat-sync covenant — re-speeding one never moves its edges — so a
// duration to type would be a promise the engine is right to refuse.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

/// Ask for a new speed, in per cent. Completes with null when dismissed.
///
/// [durationFrames] is the length the thing has now: given, the duration well
/// appears and is linked to the speed; null (a clip) leaves the speed well
/// alone.
Future<double?> showStretchDialogFrb({
  required BuildContext context,
  required int? durationFrames,
  required double fps,
}) =>
    showLumitModal<double>(
      context: context,
      builder: (close) => _StretchBody(
        durationFrames: durationFrames,
        fps: fps,
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

class _StretchBody extends StatefulWidget {
  final int? durationFrames;
  final double fps;
  final ValueChanged<double> onConfirm;
  final VoidCallback onCancel;

  const _StretchBody({
    required this.durationFrames,
    required this.fps,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_StretchBody> createState() => _StretchBodyState();
}

class _StretchBodyState extends State<_StretchBody> {
  /// The speed asked for, in per cent. The duration well is derived from it
  /// rather than stored beside it: two stored numbers that must agree are two
  /// numbers that eventually will not.
  double _speed = 100;

  /// The length at the current speed, in whole frames — at least one, because
  /// a layer of no length is not a layer.
  int get _frames {
    final from = widget.durationFrames;
    if (from == null) return 0;
    final scaled = (from * 100 / _speed).round();
    return scaled < 1 ? 1 : scaled;
  }

  /// Typing a duration is asking for the speed that produces it.
  void _setFrames(num frames) {
    final from = widget.durationFrames;
    if (from == null) return;
    final wanted = frames < 1 ? 1.0 : frames.toDouble();
    setState(() => _speed = from * 100 / wanted);
  }

  void _confirm() => widget.onConfirm(_speed);

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final frames = widget.durationFrames;
    // The dialogue takes focus when it opens, and Enter applies wherever that
    // focus sits — the same rule the Pre-compose dialogue follows.
    return Focus(
      autofocus: true,
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        if (event.logicalKey != LogicalKeyboardKey.enter &&
            event.logicalKey != LogicalKeyboardKey.numpadEnter) {
          return KeyEventResult.ignored;
        }
        _confirm();
        return KeyEventResult.handled;
      },
      child: FloatSurface(
        width: 340,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.only(bottom: 10),
              child: Text(
                frames == null ? l10n.clipSpeedTitle : l10n.stretchTitle,
                style: t.bodyPrimary,
              ),
            ),
            _row(
              t,
              label: l10n.stretchNewSpeed,
              child: DragValueField(
                key: const ValueKey('stretch-speed'),
                value: _speed,
                min: 1,
                max: 10000,
                decimals: 2,
                suffix: l10n.unitSymbolPercent,
                onChanged: (v) => setState(() => _speed = v.toDouble()),
              ),
            ),
            if (frames != null) ...[
              const SizedBox(height: 8),
              _row(
                t,
                label: l10n.stretchNewDuration,
                child: DragValueField(
                  key: const ValueKey('stretch-duration'),
                  value: _frames,
                  min: 1,
                  max: 1000000,
                  decimals: 0,
                  onChanged: _setFrames,
                ),
              ),
              const SizedBox(height: 10),
              // The factual summary line the dialogue pattern asks for
              // (§12A.4), and the anchoring rule said plainly beneath it: the
              // in point holds and the end of the layer moves.
              Text(
                l10n.compDurationReading(
                  '$_frames',
                  (_frames / widget.fps).toStringAsFixed(2),
                ),
                key: const ValueKey('stretch-summary'),
                style: t.mono.copyWith(color: t.textMuted),
              ),
              const SizedBox(height: 4),
              Text(l10n.stretchAnchor,
                  style: t.caption.copyWith(color: t.textMuted)),
            ],
            const SizedBox(height: 16),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('stretch-confirm'),
                  primary: true,
                  onPressed: _confirm,
                  child: Text(l10n.apply),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('stretch-cancel'),
                  onPressed: widget.onCancel,
                  child: Text(l10n.cancel),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  /// Label left, control right — the dialogue pattern's row.
  Widget _row(LumitTheme t, {required String label, required Widget child}) =>
      Row(
        children: [
          SizedBox(width: 110, child: Text(label, style: t.small)),
          Expanded(child: child),
        ],
      );
}
