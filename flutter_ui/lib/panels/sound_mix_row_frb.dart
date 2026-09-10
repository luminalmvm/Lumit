// The Timeline's **Sound mix** row: one row at the foot of the table standing
// for everything the comp sounds like (docs/09 §1, docs/07 §4.2).
//
// In plain terms: once a comp has been mixed the picture edit does not want
// six audio rows in the way, so its Audio layers fold away under this row,
// which draws the comp's mix - every layer through its fader, the master and
// the limiter, the sound that leaves the machine - with the master fader's
// own dB well beside it. A twirl opens the fold for a look and the next mount
// forgets it; a comp that has never been to the Audio timeline has no row at
// all. A double click on either half of the row opens the Audio workspace, and
// a right click offers that and Convert to precomp. The Audio timeline panel
// has no row of its own like this one: the master is the Mixer's job there
// (docs/07 §4.8).
//
// **Pinned, not stacked.** It is drawn under the scrolling rows on both halves
// of the table rather than as a row in the list, so the layer-drag arithmetic,
// the block windows and the row seams never learn it exists, and the mix is
// on screen whatever the stack is scrolled to - which is what a master row is
// for.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show TimelineAxis;
import 'waveform_frb.dart';

/// How tall the row stands, in lane rows: two, the height a waveform lane
/// borrows for itself, so the mix is drawn at the same size as a layer's wave.
const int soundMixRows = 2;

/// The outline's end of the row: the twirl, the speaker, the name, how many
/// layers are folded, and the master fader's dB well.
class SoundMixOutlineRow extends StatelessWidget {
  final CompositionReference comp;

  /// How many Audio layers the fold holds. Zero draws no twirl, for a caller
  /// that folds nothing away.
  final int folded;
  final bool open;
  final VoidCallback? onToggleOpen;

  /// The master fader in dB, read by the panel off the document each
  /// revision - never a bridge call in this build.
  final double masterDb;

  /// A committed move of the master fader, in dB.
  final ValueChanged<double> onMasterDb;

  /// The fader in hand, on every tick of a scrub. The number follows the
  /// pointer through this and only the release writes, so a drag is one
  /// undo step rather than one per pixel of travel.
  final ValueChanged<double> onMasterDbLive;

  /// A scrub that was cancelled: whatever it staged is dropped.
  final VoidCallback onMasterDbCancel;

  /// A double click on the row: the Audio workspace, where the mix is made.
  final VoidCallback? onOpen;

  /// A right click on the row, at the global position of the press.
  final void Function(Offset)? onMenu;

  const SoundMixOutlineRow({
    super.key,
    required this.comp,
    required this.folded,
    required this.open,
    required this.onToggleOpen,
    required this.masterDb,
    required this.onMasterDb,
    required this.onMasterDbLive,
    required this.onMasterDbCancel,
    this.onOpen,
    this.onMenu,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final twirl = onToggleOpen != null;
    return Container(
      key: const ValueKey('tl-sound-mix-row'),
      height: t.density.laneRow * soundMixRows,
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(top: BorderSide(color: t.hairlineStrong)),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
          if (twirl)
            LumitTooltip(
              message: open ? l10n.tipSoundMixFold : l10n.tipSoundMixUnfold,
              child: GestureDetector(
                key: const ValueKey('tl-sound-mix-twirl'),
                behavior: HitTestBehavior.opaque,
                onTap: onToggleOpen,
                child: SizedBox(
                  width: 16,
                  height: t.density.laneRow,
                  child: Center(
                    child: glyph.LumitIcon(
                      open ? LumitIcons.collapse : LumitIcons.expand,
                      size: iconSize,
                      colour: open ? t.textPrimary : t.textMuted,
                    ),
                  ),
                ),
              ),
            )
          else
            const SizedBox(width: 16),
          const SizedBox(width: 4),
          // The row's two gestures reach over the icon, the name and the
          // count, and no further. Over the twirl the double click would hold
          // its tap until a second one had been given up on, and over the dB
          // well it would sit on top of the scrub.
          Expanded(
            child: GestureDetector(
              // Translucent, so the gap between the name and the count
              // answers as readily as the words do.
              behavior: HitTestBehavior.translucent,
              onDoubleTap: onOpen,
              onSecondaryTapUp:
                  onMenu == null ? null : (d) => onMenu!(d.globalPosition),
              child: Row(
                children: [
                  glyph.LumitIcon(LumitIcons.audio,
                      size: iconSize, colour: t.textPrimary),
                  const SizedBox(width: 6),
                  Expanded(
                    child: Text(
                      l10n.timelineSoundMix,
                      style: t.body.copyWith(color: t.textPrimary),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  if (folded > 0)
                    Padding(
                      padding: const EdgeInsets.only(right: 8),
                      child: Text(
                        l10n.timelineSoundMixFolded(folded),
                        key: const ValueKey('tl-sound-mix-count'),
                        style: t.small.copyWith(color: t.textMuted),
                      ),
                    ),
                ],
              ),
            ),
          ),
          SizedBox(
            width: 72,
            child: DragValueField(
              key: const ValueKey('tl-sound-mix-db'),
              value: masterDb,
              // The fader's own travel (docs/09 §3.1): silence to a small
              // push, the same reach the Mixer's master has.
              min: -60,
              max: 12,
              decimals: 1,
              suffix: ' dB',
              speed: 0.2,
              onChanged: (v) => onMasterDb(v.toDouble()),
              onChangeLive: (v) => onMasterDbLive(v.toDouble()),
              onChangeEnd: (v) => onMasterDb(v.toDouble()),
              onDragCancel: onMasterDbCancel,
            ),
          ),
        ],
      ),
    );
  }
}

/// The lane's end of the row: the mix's waveform over the stretch of comp
/// time the lanes are showing, following the lanes' scroll.
class SoundMixLane extends StatelessWidget {
  /// The mix summarised over the window the panel asked for, in comp seconds,
  /// or null while nothing has come back yet.
  final BridgeAudioPeaks? peaks;

  /// The lanes' horizontal scroll: this row is pinned, so it repaints as the
  /// lanes move rather than scrolling with them.
  final ScrollController hScroll;
  final double secondsPerPixel;
  final WaveformStyle style;

  /// A double click on the lane: the Audio workspace, as on the outline row.
  final VoidCallback? onOpen;

  /// A right click on the lane, at the global position of the press.
  final void Function(Offset)? onMenu;

  const SoundMixLane({
    super.key,
    required this.peaks,
    required this.hScroll,
    required this.secondsPerPixel,
    required this.style,
    this.onOpen,
    this.onMenu,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey('tl-sound-mix-lane'),
      height: t.density.laneRow * soundMixRows,
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(top: BorderSide(color: t.hairlineStrong)),
      ),
      child: GestureDetector(
        // Translucent, so the lanes go on scrolling under the pointer while
        // the row answers a double or a right click.
        behavior: HitTestBehavior.translucent,
        onDoubleTap: onOpen,
        onSecondaryTapUp:
            onMenu == null ? null : (d) => onMenu!(d.globalPosition),
        child: AnimatedBuilder(
          animation: hScroll,
          builder: (context, _) => LayoutBuilder(
            builder: (context, box) => CustomPaint(
              painter: WaveformPainter(
                peaks: peaks,
                // Comp time 0 sits a padding's width into the lanes, and this
                // row's canvas starts wherever the lanes are scrolled to.
                originSeconds: ((hScroll.hasClients ? hScroll.offset : 0.0) -
                        TimelineAxis.pad) *
                    secondsPerPixel,
                secondsPerPixel: secondsPerPixel,
                left: 0,
                right: box.maxWidth,
                colours: t.waveform,
                // The mix has one band: the stack's filters want the samples,
                // and a mix keeps none. Where the wave sits follows Settings.
                style: WaveformStyle(
                    multiwave: false, fromBottom: style.fromBottom),
              ),
              size: Size(box.maxWidth, t.density.laneRow * soundMixRows),
            ),
          ),
        ),
      ),
    );
  }
}
