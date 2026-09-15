// The Viewer's **deck**: the strip under the picture that holds everything
// about playing, when the arrangement setting asks for one. The transport,
// the clock and the frame count, the preview mode, the quality, the
// cache-ready meter, and the progress bar at the right.
//
// Built from viewer_bar.dart's transport and viewer_strips.dart's sizes, so
// a mark on the deck is the same mark it is on the bar.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/settings.dart';
import '../state/viewer_view.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show workAreaFrames;
import 'viewer_bar.dart';
import 'viewer_progress_bar.dart';
import 'viewer_strips.dart';

/// The deck's height under Desk, a fixed 32 instrument strip
/// (docs/design-alt/15-DESIGN-DESK.md 12B.4). Desk's header strip is 24, so
/// the deck states its own number rather than borrowing that one.
const double _deskDeckHeight = 32;

/// The clock on the deck: the one large number in the application under Desk
/// and Lantern; Studio keeps the bar's 11.
const double _deckClockSize = 15;

/// Below this the deck stops spreading and slides sideways instead. Nothing is
/// shed: the transport is first on the row, so it is what a narrow deck keeps.
const double _deckMinimum = 620;

/// The Viewer's **deck** (key `viewer-deck`): the playing half of the Viewer's
/// chrome, under the picture, when the arrangement is [ViewerBars.deck].
class ViewerDeck extends StatelessWidget {
  final bool playing;
  final int frame;
  final BridgeCompSettings settings;
  final CompositionReference comp;
  final VoidCallback onPlayPause;
  final ValueChanged<int> onSeek;

  /// What playback does at the end of the work area, and whether the output
  /// is muted. Both handed in: the deck never asks the engine in build.
  final LoopMode loop;
  final bool muted;

  /// Drawn as a tile of its own under Lantern, welded to the panel edge under
  /// Studio and Desk.
  final bool detached;

  const ViewerDeck({
    super.key,
    required this.playing,
    required this.frame,
    required this.settings,
    required this.comp,
    required this.onPlayPause,
    required this.onSeek,
    required this.loop,
    required this.muted,
    required this.detached,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context);
    final desk = t.shape == ThemeShape.desk;
    final duration = durationFramesOf(settings);
    final valueStyle =
        t.mono.copyWith(fontSize: barValueTextSize, color: t.textMuted);
    final height = desk ? _deskDeckHeight : t.density.headerStrip;
    // Under Lantern the transport pill sits in from the deck's rounded end
    // by the difference of the two radii, so the curves are concentric;
    // never less than nought, since a test can mount Lantern on a density
    // whose strip is shorter than the pill.
    final concentric = math.max(0.0, (height - viewerTransportPillHeight) / 2);
    return Container(
      key: const ValueKey('viewer-deck'),
      height: height,
      decoration: viewerStripDecoration(t, detached),
      padding: EdgeInsets.only(
        left: t.tokens.roomed
            ? concentric
            : viewerStripPadding - viewerMarkEdge,
        right: t.tokens.roomed ? concentric : viewerStripPadding,
      ),
      child: LayoutBuilder(
        builder: (context, constraints) {
          // Desk's round keys are wider than the bare marks, so its deck
          // needs more room before it can spread; below that the row keeps
          // its own width and the deck scrolls.
          final loose =
              constraints.maxWidth >= _deckMinimum + (desk ? 200 : 0);
          final row = Row(
              mainAxisSize: loose ? MainAxisSize.max : MainAxisSize.min,
              children: [
                ...viewerTransportMarks(
                  t,
                  playing: playing,
                  frame: frame,
                  settings: settings,
                  comp: comp,
                  onPlayPause: onPlayPause,
                  onSeek: onSeek,
                  detached: detached,
                  clockSize: t.shape == ThemeShape.studio
                      ? viewerTimecodeSize
                      : _deckClockSize,
                  playFilled: detached,
                ),
                viewerBarGapBox(viewerBarGap),
                // The frame count beside the clock: where the playhead is, out
                // of how many. Off the held settings, never the engine.
                Text(
                  'F$frame/$duration',
                  key: const ValueKey('viewer-frame-count'),
                  style: valueStyle,
                ),
                viewerBarGapBox(viewerBarGap),
                // Preview mode and quality: the same two answers the header's
                // quality menu carries, each as its own picker here.
                LumitTooltip(
                  message: l10n.viewerQualityPlayback,
                  child: BareDropdown<PlaybackMode>(
                    key: const ValueKey('viewer-preview-mode'),
                    dense: true,
                    value: ui.workspace.performance.playback,
                    options: PlaybackMode.values,
                    label: (mode) => mode == PlaybackMode.adaptive
                        ? l10n.playbackAdaptiveShort
                        : l10n.playbackEveryFrame,
                    onChanged: (mode) {
                      ui.workspace.performance.playback = mode;
                      ui.workspace.touch();
                    },
                  ),
                ),
                const SizedBox(width: viewerHeaderGap),
                LumitTooltip(
                  message: l10n.viewerQualityResolution,
                  child: BareDropdown<PreviewResolution>(
                    key: const ValueKey('viewer-quality'),
                    dense: true,
                    value: ui.previewResolution,
                    options: PreviewResolution.values,
                    label: (r) => r.title,
                    onChanged: ui.setPreviewResolution,
                  ),
                ),
                viewerBarGapBox(viewerBarGap),
                // The loop mode, one mark cycling through the three (docs/07
                // §9). The set has one loop glyph, so the colour tells the
                // modes apart: lit for the work-area loop, muted for once,
                // accent for ping-pong.
                viewerBarMark(
                  key: const ValueKey('viewer-loop'),
                  icon: LumitIcon.loop,
                  colour: switch (loop) {
                    LoopMode.workArea => t.textPrimary,
                    LoopMode.once => t.textMuted,
                    LoopMode.pingPong => t.accent,
                  },
                  onPressed: () => ui.workspace.setLoopMode(LoopMode
                      .values[(loop.index + 1) % LoopMode.values.length]),
                  tip: switch (loop) {
                    LoopMode.workArea => l10n.tipTransportLoopWorkArea,
                    LoopMode.once => l10n.tipTransportLoopOnce,
                    LoopMode.pingPong => l10n.tipTransportLoopPingPong,
                  },
                ),
                viewerBarGapBox(viewerBarGap),
                // The mute: lit while the output is silenced, which is the
                // state worth noticing.
                viewerBarMark(
                  key: const ValueKey('viewer-mute'),
                  icon: muted ? LumitIcon.mute : LumitIcon.audio,
                  colour: muted ? t.textPrimary : t.textMuted,
                  onPressed: () => ui.setAudioMuted(!muted),
                  tip: muted ? l10n.tipTransportUnmute : l10n.tipTransportMute,
                ),
                viewerBarGapBox(viewerBarGap),
                _CacheMeter(comp: comp, duration: duration),
                if (loose) const Spacer() else const SizedBox(width: 24),
                ViewerProgressBar(
                  tracker: Provider.of<LumitUiState>(context, listen: false)
                      .previewProgress,
                ),
              ]);
          return loose
              ? row
              : SingleChildScrollView(
                  scrollDirection: Axis.horizontal, child: row);
        },
      ),
    );
  }
}

/// How much of the work area is ready to play: a small meter and its figure.
///
/// The cache is asked only when the engine says it has changed, the way the
/// Timeline's cache bar holds its strip. Never in build: a zoom flight or an
/// arriving frame rebuilds the deck, and neither changes what is held.
class _CacheMeter extends StatefulWidget {
  final CompositionReference comp;
  final int duration;

  const _CacheMeter({required this.comp, required this.duration});

  @override
  State<_CacheMeter> createState() => _CacheMeterState();
}

class _CacheMeterState extends State<_CacheMeter> {
  double _fraction = 0;
  LumitUiState? _ui;

  @override
  void initState() {
    super.initState();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _ui = ui;
    ui.cacheChanged.addListener(_read);
    _fraction = _count(ui);
  }

  @override
  void didUpdateWidget(_CacheMeter old) {
    super.didUpdateWidget(old);
    // A different composition, or a comp of a new length, is a different set
    // of frames to count. A build follows this anyway.
    if (old.comp != widget.comp || old.duration != widget.duration) {
      _fraction = _count(_ui!);
    }
  }

  @override
  void dispose() {
    _ui?.cacheChanged.removeListener(_read);
    super.dispose();
  }

  /// The engine says the cache moved: count again and redraw.
  void _read() {
    if (!mounted) return;
    final fraction = _count(_ui!);
    if (fraction != _fraction) setState(() => _fraction = fraction);
  }

  /// The share of the work area's frames the cache holds at the Viewer's
  /// scale: a nonzero strip byte is a frame that is held somewhere.
  double _count(LumitUiState ui) {
    final frames = widget.duration;
    if (frames <= 0) return 0;
    final work = workAreaFrames(widget.comp);
    final held = widget.comp.cachedFrames(
      frames: BigInt.from(frames),
      scale: ui.viewerScale,
    );
    final start = work.start.clamp(0, frames);
    final end = work.end.clamp(start, frames);
    if (end <= start) return 0;
    var ready = 0;
    for (var f = start; f < end && f < held.length; f++) {
      if (held[f] != 0) ready++;
    }
    return ready / (end - start);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final radius = BorderRadius.circular(t.tokens.actionRadius);
    return LumitTooltip(
      message: l10n.tipViewerCacheReady,
      child: Row(
        key: const ValueKey('viewer-cache-ready'),
        mainAxisSize: MainAxisSize.min,
        children: [
          // The Lantern drawing's 54 by 6 meter; the held colour is the cache
          // bar's own mint.
          ClipRRect(
            borderRadius: radius,
            child: SizedBox(
              width: 54,
              height: 6,
              child: ColoredBox(
                color: t.hairline,
                child: FractionallySizedBox(
                  alignment: Alignment.centerLeft,
                  widthFactor: _fraction.clamp(0.0, 1.0),
                  child: ColoredBox(color: t.success),
                ),
              ),
            ),
          ),
          const SizedBox(width: 4),
          Text(
            '${(_fraction * 100).round()}%',
            style:
                t.mono.copyWith(fontSize: barValueTextSize, color: t.textMuted),
          ),
        ],
      ),
    );
  }
}
