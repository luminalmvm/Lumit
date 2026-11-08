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
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';
import 'dialog_frame.dart';

/// Ask for a new speed, in per cent. Completes with null when dismissed.
///
/// [durationFrames] is the length the thing has now: given, the duration well
/// appears and is linked to the speed; null (a clip) leaves the speed well
/// alone.
Future<double?> showStretchDialogFrb({
  required BuildContext context,
  required int? durationFrames,
}) =>
    showLumitModal<double>(
      context: context,
      builder: (close) => _StretchBody(
        durationFrames: durationFrames,
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

class _StretchBody extends StatefulWidget {
  final int? durationFrames;
  final ValueChanged<double> onConfirm;
  final VoidCallback onCancel;

  const _StretchBody({
    required this.durationFrames,
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

  /// The length as a timecode, as the Retime panel's well shows it.
  Widget _duration(LumitTheme t) {
    final (fpsNum, fpsDen) =
        Provider.of<LumitUiState>(context, listen: false).model.fpsExact;
    return TimeReadout(
      key: const ValueKey('stretch-duration'),
      frame: _frames,
      format: (f) => timecodeOfRateSigned(f, fpsNum, fpsDen),
      parse: (text) => framesOfTimecodeSigned(text, fpsNum, fpsDen),
      widthChars: timecodeChars(fpsNum, fpsDen),
      style: t.mono.copyWith(fontSize: wellTextSize),
      well: true,
      wellFaceHeight: wellHeight,
      minFrame: 1,
      maxFrame: 1000000,
      draggable: true,
      onCommit: _setFrames,
    );
  }

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
      child: DialogFrame(
        width: 340,
        children: [
          dialogTitleBar(
            t,
            title: frames == null ? l10n.clipSpeedTitle : l10n.stretchTitle,
            onClose: widget.onCancel,
            keyPrefix: 'stretch',
          ),
          Padding(
            padding: const EdgeInsets.all(dialogPadding),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                dialogRow(
                  t,
                  l10n.stretchNewSpeed,
                  labelColumn: 64,
                  DragValueField(
                    key: const ValueKey('stretch-speed'),
                    value: _speed,
                    min: 1,
                    max: 10000,
                    decimals: 2,
                    suffix: l10n.unitSymbolPercent,
                    onChanged: (v) => setState(() => _speed = v.toDouble()),
                  ),
                ),
                if (frames != null)
                  dialogRow(
                    t,
                    l10n.stretchNewDuration,
                    labelColumn: 64,
                    _duration(t),
                  ),
              ],
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'stretch',
            actions: [
              dialogPlainAction(t,
                  key: const ValueKey('stretch-cancel'),
                  label: l10n.cancel,
                  onPressed: widget.onCancel),
              dialogFilledAction(t,
                  key: const ValueKey('stretch-confirm'),
                  label: l10n.apply,
                  onPressed: _confirm),
            ],
          ),
        ],
      ),
    );
  }
}
