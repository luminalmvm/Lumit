// The Audio timeline's chrome strip over the outline's column header: the
// playhead's time and frame, the comp's length, and the search that narrows
// the list. The layer Timeline's own pieces at its own height, less LAYERS and
// GRAPH - this panel has one shape.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';
import 'timeline_extras_frb.dart' show layerSearchWellHeight;
import 'timeline_toolbar_frb.dart' show timelineChromeControl;

/// How narrow the chrome strip's search box may be squeezed before the
/// readouts beside it give up their room instead. A well under this cannot hold
/// its own hint on one line, and a long comp's readouts will squeeze it to
/// nothing given the chance.
const double audioTimelineSearchFloor = 120;

/// Whether the chrome strip can carry the frame count and the comp's length
/// beside the clock: only while the search box keeps its floor with the two of
/// them and their gaps in the row as well. Pure, so the rule can be read off
/// the numbers rather than off a screenshot.
bool audioTimelineChromeCarriesCount({
  required double strip,
  required double clock,
  required double count,
  required double length,
}) =>
    strip - clock - outlineGap - audioTimelineSearchFloor >= count + length + 8;

/// A typed frame number, with or without the letter the readout wears.
int? _frameOfTyped(String text) =>
    int.tryParse(text.trim().replaceFirst(RegExp('^[fF]'), ''));

/// The strip itself. [search] is the panel's own field, whose text narrows
/// the tracks to the names carrying it.
class AudioChromeStrip extends StatelessWidget {
  final TextEditingController search;

  const AudioChromeStrip({super.key, required this.search});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    final lastFrame = ui.model.durationFrames - 1;
    final clockFace = t.mono.copyWith(fontSize: 11, color: t.textPrimary);
    final countFace = t.mono.copyWith(fontSize: 10, color: t.textMuted);
    final length = '/${ui.model.durationFrames}';
    // What each piece takes: a readout's slot is its digits plus its own
    // inset, and a well's hairline adds two either side of that.
    final clockWidth = monoSlotWidth(clockFace, timecodeChars(fpsNum, fpsDen)) +
        readoutPadding.horizontal +
        2;
    final countWidth = monoSlotWidth(
            countFace, 2 + '${lastFrame < 0 ? 0 : lastFrame}'.length) +
        readoutPadding.horizontal +
        2;
    return Container(
      height: t.density.navigatorBand + t.density.timelineChromeRow,
      color: t.surface1,
      padding: const EdgeInsets.only(left: 10, right: 8),
      child: LayoutBuilder(
        builder: (context, box) {
          // The ladder a narrow outline goes down: the well keeps its floor,
          // and the frame count and the comp's length go together as soon as
          // what is left will not hold them. The clock is what stays, because
          // a strip that cannot say the time is not worth its row.
          final whole = audioTimelineChromeCarriesCount(
            strip: box.maxWidth,
            clock: clockWidth,
            count: countWidth,
            length: monoSlotWidth(countFace, length.length),
          );
          return Row(
            children: [
              // Both readouts stand in slots wide enough for the longest thing
              // they can say and both can be typed into, as the Timeline's are.
              // Only they listen to the playhead: the rest of this half stands
              // still while it runs.
              ValueListenableBuilder<int>(
                valueListenable: ui.playheadFrame,
                builder: (context, frame, _) => Row(
                  children: [
                    timelineChromeControl(
                        t,
                        TimeReadout(
                          key: const ValueKey('atl-timecode'),
                          frame: frame,
                          format: (f) => timecodeOfRate(f, fpsNum, fpsDen),
                          widthChars: timecodeChars(fpsNum, fpsDen),
                          style: clockFace,
                          parse: (text) =>
                              framesOfTimecode(text, fpsNum, fpsDen),
                          onCommit: ui.scrubTo,
                          minFrame: 0,
                          maxFrame: lastFrame,
                          tooltip: l10n.tipPlayheadTime,
                          well: true,
                        )),
                    if (whole) const SizedBox(width: 4),
                    if (whole)
                      timelineChromeControl(
                          t,
                          TimeReadout(
                            key: const ValueKey('atl-frame'),
                            frame: frame,
                            format: (f) => 'F$f',
                            widthChars:
                                2 + '${lastFrame < 0 ? 0 : lastFrame}'.length,
                            style: countFace,
                            parse: _frameOfTyped,
                            onCommit: ui.scrubTo,
                            minFrame: 0,
                            maxFrame: lastFrame,
                            tooltip: l10n.tipFrameNumber,
                            well: true,
                            editFormat: (f) => '$f',
                          )),
                  ],
                ),
              ),
              // How many frames there are in all, after the frame the playhead is
              // on. Outside the listener, because a comp's length does not move
              // as the playhead does.
              if (whole) const SizedBox(width: 4),
              if (whole) Text(length, style: countFace),
              const SizedBox(width: outlineGap),
              // The well takes what is left of the row and is the first to give
              // it back, so a narrow outline still reads its clock.
              Expanded(
                child: timelineChromeControl(
                  t,
                  SizedBox(
                    height: layerSearchWellHeight,
                    child: HouseTextField(
                      key: const ValueKey('atl-search'),
                      controller: search,
                      width: 1e9,
                      padding: const EdgeInsets.symmetric(horizontal: 6),
                      hint: l10n.searchTracks,
                      leading: glyph.LumitIcon(LumitIcons.search,
                          size: layerSearchWellHeight - 4, colour: t.textMuted),
                    ),
                  ),
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}
