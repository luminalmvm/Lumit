// The Audio timeline's **Master** track, pinned at the foot of the track list
// on both halves of the table (docs/impl/audio-timeline.md §5).
//
// In the house style of the layer Timeline's Sound mix row: its lane draws
// the comp's mix, the sound that leaves the machine, over the stretch of time
// the lanes are showing, and its twirl opens a Volume lane over the master
// fader. It is not a layer: no clips, no switches, no drag. Pinned rather
// than stacked, so the block windows, the row seams and the drop arithmetic
// never learn it exists, and the mix is on screen whatever the list is
// scrolled to.
//
// The master's Volume is keyed in comp time, and its lane is the same lane
// every track has, with the master's own automation mode.

import 'dart:async';

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import 'audio_timeline_live.dart';
import 'audio_timeline_zoom.dart' show mixAsSamples, waveBuckets;
import 'automation_lane.dart';
import 'automation_mode_chip.dart';
import 'automation_pass_line.dart' show AutomationPassFeed;
import 'audio_timeline_keys_frb.dart' show laneSelection;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'loudness_commands.dart' show loudnessMenuRows;
import 'sound_mix_row_frb.dart' show soundMixRows;
import 'timeline_extras_frb.dart' show TimelineAxis, showMenuAt;
import 'volume_band_frb.dart';
import 'waveform_frb.dart';

/// The path the master's Volume lane keys are picked under, and the id its
/// height is kept by.
const String masterLaneId = 'master/volume';

/// What the panel holds for the master: its twirl, its Volume and mode, and
/// the mix's picture.
class MasterTrack {
  bool open = false;

  /// The master fader, keyed in comp time or a static level in dB, and its
  /// automation mode, read off the document once per revision.
  BridgeScalar scalar = const BridgeScalar.static_(0);
  BridgeAutomationMode mode = BridgeAutomationMode.read;

  /// The comp's loudness target, for the menu's tick.
  BridgeLoudnessTarget? loudness;
  int? _revision;
  CompositionReference? _comp;

  /// The mix summarised over the lanes' window.
  final MixPeaks mix = MixPeaks();

  BridgeAudioPeaks? get peaks => mix.peaks;

  bool get keyed => scalar is BridgeScalar_Keyframed;

  /// Read the master once per document revision. Called from the model's
  /// listener, which runs before the rebuild, so a build reads a field.
  void readLevel(CompositionReference? comp, int? revision) {
    if (comp == null) return;
    if (revision != null && revision == _revision && comp == _comp) return;
    _revision = revision;
    _comp = comp;
    try {
      scalar = comp.masterVolume();
      mode = comp.masterAutomationMode();
      loudness = comp.loudnessTarget();
    } catch (_) {
      // A comp that has gone keeps the last reading.
    }
  }

  /// The mix's peaks over the lanes' window ([MixPeaks.refresh]).
  void refreshPeaks({
    required CompositionReference comp,
    required double viewStart,
    required double viewEnd,
    required double pixels,
    required int? revision,
    required VoidCallback onPeaks,
    required VoidCallback retry,
  }) =>
      mix.refresh(
          comp: comp,
          viewStart: viewStart,
          viewEnd: viewEnd,
          pixels: pixels,
          revision: revision,
          onPeaks: onPeaks,
          retry: retry);

  /// The panel is going: no fetch may land on it.
  void dispose() => mix.dispose();

  /// The Volume lane, [rows] holding how tall it has been dragged by its id.
  AutomationLane lane(double rowHeight, Map<String, int> rows) =>
      AutomationLane(
        kind: LaneKind.volume,
        scalar: scalar,
        id: masterLaneId,
        rows: rows[masterLaneId] ?? automationLaneMinRows,
        rowHeight: rowHeight,
      );

  /// What the lane's band reads and writes. A drag while the master records
  /// goes to the live mix.
  BandTarget target(
          CompositionReference comp, ValueListenable<bool> playing) =>
      recordingBand(
        BandTarget(
          scalar: scalar,
          kind: BandKind.volume,
          scale: BandScale.fader,
          locked: false,
          id: 'master',
          write: (next) => writeLevel(comp, next),
        ),
        comp: comp,
        target: (layer: null, kind: LaneKind.volume, bus: null),
        mode: mode,
        playing: playing,
      );

  /// The Volume lane's time area under the mix, pinned and carried by the
  /// lanes' scroll, with no wave behind it: the mix is the row above. It
  /// boxes up its own keys, being outside the lanes' marquee.
  Widget laneArea(
    CompositionReference comp,
    ValueListenable<bool> playing, {
    required AutomationLane lane,
    required ScrollController hScroll,
    required TimelineAxis axis,
    required double fps,
    required (int, int) fpsExact,
    required ValueNotifier<Set<String>> picked,
    required void Function(String id, bool additive) onSelectKey,
    required AutomationPassFeed pass,
    required WaveformStyle style,
    required VoidCallback onChanged,
  }) =>
      PinnedLane(
        hScroll: hScroll,
        axis: axis,
        height: lane.height,
        child: ValueListenableBuilder<Set<String>>(
          valueListenable: picked,
          builder: (context, selected, _) => AutomationLaneArea(
            key: const ValueKey<String>('atl-master-lane-area'),
            lane: lane,
            target: target(comp, playing),
            pass: pass,
            comp: comp,
            axis: axis,
            fps: fps,
            fpsNum: fpsExact.$1,
            fpsDen: fpsExact.$2,
            height: lane.height,
            peaks: null,
            originSeconds: 0,
            secondsPerPixel: 0,
            left: 0,
            right: axis.width,
            style: style,
            selected: laneSelection(selected, lane.id),
            onSelectKey: (index, additive) =>
                onSelectKey('${lane.id}#$index', additive),
            onMarquee: (caught, additive) => picked.value = {
              if (additive) ...picked.value,
              for (final i in caught) '${lane.id}#$i',
            },
            onChanged: onChanged,
          ),
        ),
      );

  /// The master's Volume written whole.
  void writeLevel(CompositionReference comp, BridgeScalar next) =>
      comp.setMasterVolume(value: next);

  /// The master's right-click: its automation mode, Volume back to unity at
  /// the playhead, the loudness target and Normalise mix.
  void showMenu(BuildContext context, CompositionReference comp, Offset at,
      {required int frame, required VoidCallback onChanged}) {
    final changed = keyed || scalar != const BridgeScalar.static_(0);
    showMenuAt<void>(
      context: context,
      position: at,
      width: 180,
      rows: (close) => [
        SubmenuRow(
          key: const ValueKey<String>('atl-master-automation'),
          closeParent: () => close(null),
          submenu: (dismiss) => FloatSurface(
            width: 130,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: automationModeRows(
                keyPrefix: 'atl-master-automation',
                current: mode,
                dismiss: dismiss,
                onPick: (next) {
                  comp.setMasterAutomationMode(mode: next);
                  onChanged();
                },
              ),
            ),
          ),
          child: MenuLabel(l10n.audioAutomation, ticked: false),
        ),
        MenuRow(
          key: const ValueKey<String>('atl-master-reset'),
          enabled: changed,
          onPressed: () {
            close(null);
            writeLevel(comp, scalarWithValueAt(scalar, 0, comp, frame));
            onChanged();
          },
          child: MenuLabel(l10n.audioResetVolume,
              enabled: changed, ticked: false),
        ),
        const MenuDivider(),
        ...loudnessMenuRows(
          context: context,
          app: Provider.of<LumitState>(context, listen: false),
          comp: comp,
          current: loudness,
          close: () => close(null),
          onChanged: onChanged,
        ),
      ],
    );
  }
}

/// A mix's picture over the lanes' window: the comp's whole mix, or with a
/// [bus] the sum of what is routed to it. The mix changes with every edit, so
/// the document revision is part of what it was fetched for.
class MixPeaks {
  final UuidValue? bus;

  MixPeaks({this.bus});

  /// The mix summarised, or null while nothing has come back yet; what it
  /// was fetched for; and the retries an empty answer earns while the engine
  /// is still preparing the mix.
  BridgeAudioPeaks? peaks;
  String? _peakKey;
  String? _askKey;
  int _asks = 0;
  Timer? _retry;

  /// An empty answer is the engine still preparing the mix, or a machine with
  /// nothing to play it through: [retry] is asked after a moment, a handful of
  /// times, then left until the window or the document moves. [onPeaks] runs
  /// when a picture lands.
  void refresh({
    required CompositionReference comp,
    required double viewStart,
    required double viewEnd,
    required double pixels,
    required int? revision,
    required VoidCallback onPeaks,
    required VoidCallback retry,
  }) {
    final request = WaveformRequest.forView(
        startSeconds: viewStart, endSeconds: viewEnd, pixels: pixels);
    if (request == null) return;
    final key = '${request.key}|$revision';
    if (_peakKey == key) return;
    if (_askKey != key) {
      _asks = 0;
      _askKey = key;
    }
    _peakKey = key;
    busPeaks(
      comp,
      bus: bus,
      startSeconds: request.startSeconds,
      endSeconds: request.endSeconds,
      buckets: waveBuckets(request),
    ).then((got) async {
      if (_peakKey != key || got == null) return;
      if (got.buckets == 0) {
        if (_asks >= 8) return;
        _asks += 1;
        _retry?.cancel();
        _retry = Timer(const Duration(milliseconds: 400), () {
          _retry = null;
          if (_peakKey != key) return;
          _peakKey = null;
          retry();
        });
        return;
      }
      final drawn =
          await mixAsSamples(got, request, (viewEnd - viewStart) / pixels);
      if (_peakKey != key) return;
      peaks = drawn;
      onPeaks();
    });
  }

  /// Nothing more may land.
  void dispose() {
    _retry?.cancel();
    _peakKey = 'closed';
  }
}

/// Each of [buses]' mixes over the lanes' window, held in [mixes] by layer
/// id. A bus that has left the list keeps nothing.
void refreshBusMixes(
  Map<String, MixPeaks> mixes,
  Iterable<BridgeLayerEntry> buses, {
  required CompositionReference comp,
  required double viewStart,
  required double viewEnd,
  required double pixels,
  required int? revision,
  required VoidCallback onPeaks,
  required VoidCallback retry,
}) {
  final live = <String>{};
  for (final bus in buses) {
    final id = bus.layer.internallayerId.toString();
    live.add(id);
    mixes
        .putIfAbsent(id, () => MixPeaks(bus: bus.layer.internallayerId))
        .refresh(
            comp: comp,
            viewStart: viewStart,
            viewEnd: viewEnd,
            pixels: pixels,
            revision: revision,
            onPeaks: onPeaks,
            retry: retry);
  }
  mixes.removeWhere((id, mix) {
    if (live.contains(id)) return false;
    mix.dispose();
    return true;
  });
}

/// The outline's end of the master: the mode chip, the twirl and the name,
/// over the row the Sound mix wears.
class MasterOutlineRow extends StatelessWidget {
  final bool open;
  final VoidCallback onToggleOpen;

  /// The master's automation mode, whether its Volume is keyed, and the
  /// write a pick from the chip makes.
  final BridgeAutomationMode mode;
  final bool keyed;
  final ValueChanged<BridgeAutomationMode> onMode;

  /// A right click on the row, at the global position of the press.
  final void Function(Offset) onMenu;

  const MasterOutlineRow({
    super.key,
    required this.open,
    required this.onToggleOpen,
    required this.mode,
    required this.keyed,
    required this.onMode,
    required this.onMenu,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey<String>('atl-master-row'),
      height: t.density.laneRow * soundMixRows,
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(top: BorderSide(color: t.hairlineStrong)),
      ),
      padding: const EdgeInsets.only(left: outlineGap, right: 6),
      child: Row(
        children: [
          // The chip stands under the track heads' own, the twirl under
          // theirs.
          const SizedBox(width: switchCellWidth * 3),
          AutomationModeChip(
              id: 'master', mode: mode, keyed: keyed, onPick: onMode),
          const SizedBox(width: outlineGap),
          LumitTooltip(
            message: open ? l10n.tipHideProperties : l10n.tipProperties,
            child: HoverMark(
              builder: (context, lit) => GestureDetector(
                key: const ValueKey<String>('atl-master-twirl'),
                behavior: HitTestBehavior.opaque,
                onTap: onToggleOpen,
                child: SizedBox(
                  width: 16,
                  height: t.density.laneRow,
                  child: Center(
                    child: TwirlTurn(
                      open: open,
                      child: glyph.LumitIcon(
                        open ? LumitIcons.collapse : LumitIcons.expand,
                        size: iconSize,
                        colour: open
                            ? t.textPrimary
                            : Color.lerp(t.textMuted, t.textPrimary, lit),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
          const SizedBox(width: 4),
          Expanded(
            child: GestureDetector(
              behavior: HitTestBehavior.translucent,
              onSecondaryTapUp: (d) => onMenu(d.globalPosition),
              child: Row(
                children: [
                  glyph.LumitIcon(LumitIcons.audio,
                      size: iconSize, colour: t.textPrimary),
                  const SizedBox(width: 6),
                  Expanded(
                    child: Text(
                      l10n.mixerMaster,
                      key: const ValueKey<String>('atl-master-name'),
                      style: t.body.copyWith(color: t.textPrimary),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// A lane pinned under the lanes' scroll: [child] is laid out the full width
/// of the axis and carried left by however far the lanes are scrolled, so its
/// curve lines up with the ruler above while the row stays put.
class PinnedLane extends StatelessWidget {
  final ScrollController hScroll;
  final TimelineAxis axis;
  final double height;
  final Widget child;

  const PinnedLane({
    super.key,
    required this.hScroll,
    required this.axis,
    required this.height,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return SizedBox(
      height: height,
      child: ColoredBox(
        color: t.surface1,
        child: AnimatedBuilder(
          animation: hScroll,
          builder: (context, _) => ClipRect(
            child: Stack(
              children: [
                Positioned(
                  left: -(hScroll.hasClients ? hScroll.offset : 0.0),
                  top: 0,
                  bottom: 0,
                  width: axis.width,
                  child: child,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
