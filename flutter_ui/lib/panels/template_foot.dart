// The foot of the Graphics template panel: the poster frame, where the intro
// ends and where the outro starts, and the doors to the Data window and the
// Save dialogue. Also the small box that sets a number control's range.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';

/// One of the three frames a template marks.
enum TemplateMark { poster, intro, outro }

/// The three marks, each with its frame as the Timeline writes time, a
/// button that sets it at the playhead and a mark that clears it. Under
/// them the two buttons.
class TemplateFoot extends StatelessWidget {
  final BridgeTemplate template;

  /// The composition's rate, the frame its clock starts on and its last
  /// frame, from the read model.
  final (int, int) fps;
  final int startFrame;
  final int lastFrame;

  /// A mark set to a frame, or cleared with null.
  final void Function(TemplateMark mark, int? frame) onMark;

  /// A mark set where the playhead is.
  final void Function(TemplateMark mark) onMarkAtPlayhead;
  final VoidCallback onData;
  final VoidCallback onSave;

  const TemplateFoot({
    super.key,
    required this.template,
    required this.fps,
    required this.startFrame,
    required this.lastFrame,
    required this.onMark,
    required this.onMarkAtPlayhead,
    required this.onData,
    required this.onSave,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(top: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.fromLTRB(10, 4, 6, 6),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _mark(t, TemplateMark.poster, l10n.templatePoster, template.poster),
          _mark(t, TemplateMark.intro, l10n.templateIntro, template.intro),
          _mark(t, TemplateMark.outro, l10n.templateOutro, template.outro),
          const SizedBox(height: 4),
          Wrap(
            alignment: WrapAlignment.end,
            spacing: 6,
            runSpacing: 4,
            children: [
              HouseButton(
                key: const ValueKey('template-data'),
                small: true,
                onPressed: onData,
                child: Text(l10n.templateData),
              ),
              HouseButton(
                key: const ValueKey('template-save'),
                small: true,
                onPressed: onSave,
                child: Text(l10n.templateSave),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _mark(LumitTheme t, TemplateMark mark, String label, int? frame) {
    final (n, d) = fps;
    final style = t.mono.copyWith(fontSize: 11, color: t.textPrimary);
    final chars = timecodeChars(n, d);
    return SizedBox(
      height: 22,
      child: Row(
        children: [
          Expanded(
            child: Text(label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: t.small.copyWith(color: t.textMuted)),
          ),
          const SizedBox(width: 6),
          if (frame == null)
            // The readout's own width, so the buttons stand where they do
            // beside a frame.
            SizedBox(
              width: monoSlotWidth(style, chars) + readoutPadding.horizontal,
              child: Text('—',
                  textAlign: TextAlign.center,
                  style: style.copyWith(color: t.textDisabled)),
            )
          else
            TimeReadout(
              key: ValueKey<String>('template-${mark.name}'),
              frame: frame,
              format: (f) => timecodeOfRate(f + startFrame, n, d),
              parse: (text) => switch (framesOfTimecode(text, n, d)) {
                final f? => f - startFrame,
                null => null,
              },
              widthChars: chars,
              style: style,
              onCommit: (f) => onMark(mark, f),
              minFrame: 0,
              maxFrame: lastFrame,
              well: true,
            ),
          const SizedBox(width: 6),
          HouseButton(
            key: ValueKey<String>('template-${mark.name}-here'),
            small: true,
            frameless: true,
            padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
            onPressed: () => onMarkAtPlayhead(mark),
            child: Text(l10n.templateAtPlayhead,
                style: t.small.copyWith(color: t.textMuted)),
          ),
          // Kept in the row with nothing to clear, so the three line up.
          SizedBox(
            width: 18,
            child: frame == null
                ? null
                : LumitTooltip(
                    message: l10n.clear,
                    child: HouseButton(
                      key: ValueKey<String>('template-${mark.name}-clear'),
                      small: true,
                      frameless: true,
                      padding: const EdgeInsets.all(1),
                      onPressed: () => onMark(mark, null),
                      child: glyph.LumitIcon(LumitIcons.close,
                          size: 12, colour: t.textMuted),
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}

/// The box that sets a number control's range: a tick, and the two ends.
/// Every change is sent to [onSet] as it is made.
Future<void> showTemplateRange(
  BuildContext context,
  Offset position,
  BridgeControl control, {
  required void Function(bool ranged, double min, double max) onSet,
}) {
  var ranged = control.ranged;
  // A control never given a range starts from its slider's travel.
  final unset = control.rangeMin == 0 && control.rangeMax == 0;
  var low = unset ? control.sliderMin : control.rangeMin;
  var high = unset ? control.sliderMax : control.rangeMax;
  final span = (control.sliderMax - control.sliderMin).abs();
  final speed = span <= 0 ? 0.5 : span / 200;
  return showLumitPopup<void>(
    context: context,
    position: position,
    builder: (close) => FloatSurface(
      width: 196,
      child: Padding(
        padding: const EdgeInsets.all(8),
        child: StatefulBuilder(builder: (context, again) {
          final t = ThemeScope.of(context).theme;
          void send() => onSet(ranged, low, high);
          Widget end(String key, double value, ValueChanged<double> take) =>
              Expanded(
                child: IgnorePointer(
                  ignoring: !ranged,
                  child: Opacity(
                    opacity: ranged ? 1 : 0.5,
                    child: DragValueField(
                      key: ValueKey<String>(key),
                      value: value,
                      min: -1000000,
                      max: 1000000,
                      speed: speed,
                      decimals: 2,
                      onChanged: (v) {
                        again(() => take(v.toDouble()));
                        send();
                      },
                      // Shown as it moves, and sent once on release.
                      onChangeLive: (v) => again(() => take(v.toDouble())),
                      onChangeEnd: (v) {
                        again(() => take(v.toDouble()));
                        send();
                      },
                    ),
                  ),
                ),
              );
          return Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              HouseCheckbox(
                key: const ValueKey('template-range-on'),
                value: ranged,
                label: l10n.templateRangeLimit,
                onChanged: (on) {
                  again(() => ranged = on);
                  send();
                },
              ),
              const SizedBox(height: 6),
              Row(
                children: [
                  end('template-range-min', low, (v) => low = v),
                  Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 6),
                    child: Text('–', style: t.small),
                  ),
                  end('template-range-max', high, (v) => high = v),
                ],
              ),
            ],
          );
        }),
      ),
    ),
  );
}
