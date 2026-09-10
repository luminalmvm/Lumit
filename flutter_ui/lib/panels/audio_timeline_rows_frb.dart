// The Audio timeline's rows: what a track is made of, and the outline row that
// draws its head (docs/impl/audio-timeline.md §5).
//
// The panel beside this file draws two halves over one time axis, and both of
// them walk the list [audioTimelineTracks] returns - so a track's height is one
// answer rather than two that have to agree. A track stands two lane rows tall
// until its bottom edge is dragged, and grows by the rows its twirl opens.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'layer_fold_frb.dart';
import 'spectral_lane_frb.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_outline_row_frb.dart' show RowRenameField;

/// The lane-mode chip's own column, wide enough for *Spectral*.
const double audioTrackChipWidth = 56;

/// How many lane rows a track may stand on. Two to begin with, which is the
/// height the board draws and enough for a wave to read; eight at the most,
/// which is as far as the drag on the row's bottom edge takes it
/// (docs/impl/audio-timeline.md §5).
const int audioTrackMinRows = 2;
const int audioTrackMaxRows = 8;

/// One track of the Audio timeline: a layer that can make a sound, the rows
/// its twirl shows, and how tall the pair makes it.
///
/// The layer Timeline's [LayerRow] is not reused: that row carries a picture
/// edit's answers - the group header it draws, the sequence view's room, the
/// switches a kind can use - and a track carries none of them. What the two do
/// share is the rule that **one list decides the height**, which is why both
/// halves of this table read this one.
class AudioTrackRow {
  final BridgeLayerEntry entry;
  final String id;

  /// Twirled open - whether [foldRows] are drawn.
  final bool open;

  /// Drawn faded and deaf to the pointer: the row carries a picture as well as
  /// sound, so it is read here and worked on once its audio is detached
  /// ([audioTimelineDims]).
  final bool dimmed;

  /// The rows this track's own twirl shows: Volume, the Effects heading, and
  /// the rack under it. Held whether the twirl is open or shut, because a
  /// keyframe is somewhere in time whether or not its row is on screen and a
  /// drag can still land on it.
  final List<LayerFoldRow> foldRows;

  /// The rows the clips on this track show, in track order - nothing for a
  /// clip whose twirl is shut, so the list is already only what is asked for.
  final List<LayerFoldRow> clipRows;

  /// One lane row, `t.density.laneRow` - carried rather than looked up, so the
  /// arithmetic that has no `BuildContext` still answers the same.
  final double rowHeight;

  /// How many lane rows this track itself stands on, before its twirl adds
  /// anything. Its own, because how tall a wave has to be to read is a
  /// judgement per track: the drag on the row's bottom edge sets it.
  final int rows;

  const AudioTrackRow({
    required this.entry,
    required this.id,
    required this.open,
    required this.dimmed,
    required this.foldRows,
    this.clipRows = const [],
    required this.rowHeight,
    this.rows = audioTrackMinRows,
  });

  /// Every row under this track, drawn or not - what a drag can land on.
  List<LayerFoldRow> get allRows => [...foldRows, ...clipRows];

  /// What is drawn: the track's own rows while its twirl is open, and the open
  /// clips' rows either way, because a clip's twirl answers for itself.
  List<LayerFoldRow> get drawnRows =>
      open ? [...foldRows, ...clipRows] : clipRows;

  /// The track's own band: the room its wave, its clips and their gain lines
  /// have, and where the rows its twirl opened begin.
  double get laneHeight => rowHeight * rows;

  /// The band, then one lane row for each row the twirl opened.
  double get height => laneHeight + rowHeight * drawnRows.length;
}

/// The rows under a track: **Volume**, then the **Effects** heading, its rows
/// while the track carries an audio effect, and the rows of whichever clips on
/// it are twirled open, in track order.
///
/// Built from [layerFoldRows] and filtered, rather than written out again, so
/// the two panels cannot drift on what a parameter row is. There is no Audio
/// heading and no Waveform twirl here - the wave is on the lane - but the
/// Volume row keeps the path it has in the layer Timeline
/// (`<layer>/audio/volume`), so a fold path means the same thing in both. The
/// heading's own path is held open on the way in for exactly that reason.
List<LayerFoldRow> audioTimelineFoldRows({
  required BridgeLayerEntry entry,
  required Set<String> open,
  BridgeScalar? volumeDb,
}) {
  final id = entry.layer.internallayerId.toString();
  final audio = {
    for (final fx in entry.info.effects)
      if (fx.audio) fx.id.toString(),
  };
  final built = layerFoldRows(
    entry: entry,
    open: {...open, audioPath(id)},
    hasAudio: true,
    volumeDb: volumeDb,
  );
  final volume = <LayerFoldRow>[];
  // The Effects heading, one step in from the twirl now that no Audio heading
  // stands over it. It stands whether or not the track holds a plugin yet,
  // because the glyph that fills the rack is on it: a heading that appeared
  // only once there was something under it would be a door with no handle.
  // Built here rather than taken from [layerFoldRows], which leaves it out
  // while a layer's stack is empty.
  final effects = <LayerFoldRow>[
    FoldGroupRow(
      path: effectsPath(id),
      label: l10n.workspaceEffects,
      open: open.contains(effectsPath(id)),
      depth: 1,
    ),
  ];
  for (final row in built) {
    switch (row) {
      case FoldVolumeRow(:final scalar):
        volume.add(FoldVolumeRow(scalar: scalar, depth: 1));
      case FoldGroupRow(:final path, :final label, open: final isOpen)
          when audio.contains(effectIdOfPath(path)):
        effects.add(
            FoldGroupRow(path: path, label: label, open: isOpen, depth: 2));
      case FoldEffectParamRow(
            :final info,
            :final param,
            :final value,
            :final driven
          )
          when audio.contains(info.id.toString()):
        effects.add(
            FoldEffectParamRow(info, param, value, depth: 3, driven: driven));
      // A picture group - Transform, Masks, Styles, the Waveform twirl - has
      // nothing to say about a track.
      default:
        break;
    }
  }
  return [...volume, ...effects];
}

/// The rows the clips on a track show, in track order.
///
/// A clip's own twirl decides, so this is already only what is asked for -
/// [clipFoldRows] hands back nothing for a clip that is shut. Kept apart from
/// the track's own rows because the two twirls are separate: a clip can be open
/// under a track that is not.
List<LayerFoldRow> audioTimelineClipRows({
  required BridgeLayerEntry entry,
  required Set<String> open,
}) =>
    [
      for (final clip in entry.info.clips)
        ...clipFoldRows(clip: clip, open: open),
    ];

/// Every track the Audio timeline draws, in stack order.
///
/// [layers] and [dimmed] come from `timelineViewLayers(audioTimeline: true)`,
/// which is the rule for which layers are tracks at all.
List<AudioTrackRow> audioTimelineTracks({
  required List<BridgeLayerEntry> layers,
  required Set<String> dimmed,
  required Set<String> open,
  required double rowHeight,
  Map<String, int> trackRows = const {},
  Map<String, BridgeScalar> volumeDb = const {},
}) {
  final out = <AudioTrackRow>[];
  for (final entry in layers) {
    final id = entry.layer.internallayerId.toString();
    out.add(AudioTrackRow(
      entry: entry,
      id: id,
      open: open.contains(id),
      dimmed: dimmed.contains(id),
      foldRows: audioTimelineFoldRows(
          entry: entry, open: open, volumeDb: volumeDb[id]),
      clipRows: audioTimelineClipRows(entry: entry, open: open),
      rowHeight: rowHeight,
      rows: trackRows[id] ?? audioTrackMinRows,
    ));
  }
  return out;
}

/// The head of one track in the outline: mute, solo, the fx switch, the twirl,
/// the number, the label dot, the name, and at the right edge either the
/// lane-mode chip or - on a faded picture row - *Detach audio*.
///
/// As tall as its track's own band, which is the height before the twirl adds
/// anything and what the drag on the row's bottom edge sets.
class AudioTrackOutlineRow extends StatelessWidget {
  final CompositionReference comp;
  final AudioTrackRow track;

  /// The track's place in the list, drawn as its number.
  final int index;
  final LaneMode laneMode;

  /// This track's layer is in the shell's selection, so the Effect controls
  /// panel is showing it. The name reads at full strength for it, as a
  /// selected layer's does in the layer Timeline.
  final bool selected;

  /// The rename editor's text while this track is being named, null the rest
  /// of the time. Held by the panel, because Enter is the panel's key and this
  /// row is a plain view of what it says.
  final TextEditingController? rename;
  final VoidCallback onToggleOpen;
  final VoidCallback onCycleLaneMode;
  final VoidCallback onDetach;
  final VoidCallback onSelect;
  final VoidCallback onRenameCommit;
  final VoidCallback onRenameCancel;
  final VoidCallback onChanged;

  /// How far the row's bottom edge has just been dragged. Null leaves the
  /// edge alone, which is what a row nobody can resize wants.
  final ValueChanged<double>? onResize;

  /// A press on that edge, before any travel. What a drag ends short of a
  /// whole lane row is carried, and a new drag must not spend it.
  final VoidCallback? onResizeStart;

  const AudioTrackOutlineRow({
    super.key,
    required this.comp,
    required this.track,
    required this.index,
    required this.laneMode,
    required this.selected,
    this.rename,
    required this.onToggleOpen,
    required this.onCycleLaneMode,
    required this.onDetach,
    required this.onSelect,
    required this.onRenameCommit,
    required this.onRenameCancel,
    required this.onChanged,
    this.onResize,
    this.onResizeStart,
  });

  /// One switch cell, drawn and written exactly as the layer Timeline's outline
  /// draws and writes it: the glyph flips where it has a pair, the state is the
  /// two strengths, and the click is one `set_switch_on_layers` and so one undo
  /// step. Only this row's own layer is in the batch - the Audio timeline has
  /// no multiple selection yet.
  Widget _switch(
    LumitTheme t,
    String name,
    bool on,
    BridgeLayerSwitch which, {
    String? mark,
    String? offMark,
    LumitIcon? icon,
    required String tip,
  }) {
    final ink = on ? t.textPrimary : t.textMuted;
    return LumitTooltip(
      message: tip,
      child: GestureDetector(
        key: ValueKey<String>('atl-$name-${track.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: () {
          comp.setSwitchOnLayers(
            clicked: track.entry.layer.internallayerId,
            layers: [track.entry.layer.internallayerId],
            switch_: which,
            on_: !on,
          );
          onChanged();
        },
        child: SizedBox(
          width: switchCellWidth,
          height: t.density.laneRow,
          child: Center(
            child: mark != null
                ? glyph.LumitIcon(on || offMark == null ? mark : offMark,
                    size: iconSize, colour: ink)
                : lumitIcon(icon!, size: iconSize, color: ink),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final info = track.entry.info;
    final switches = info.switches;
    return SizedBox(
      height: track.laneHeight,
      child: Stack(children: [
        Positioned.fill(
            child: Padding(
          padding: const EdgeInsets.only(
              left: outlineGap, right: outlineRowTrailing),
          child: Row(
            children: [
              // Everything but the trailing cell fades and goes deaf on a
              // picture row. The cell itself does not: *Detach audio* is the
              // one control that undoes the dimming, so it has to stay
              // reachable.
              Expanded(
                child: dimmedIf(
                  track.dimmed,
                  // A press anywhere on the head picks the track, so the
                  // Effect controls panel follows it. Inside the dimming,
                  // which is what keeps a faded picture row out of the
                  // selection, and on the way down rather than on the tap, so
                  // a pick lands before any gesture the row's own controls
                  // start.
                  Listener(
                    behavior: HitTestBehavior.opaque,
                    onPointerDown: (_) => onSelect(),
                    child: _head(t, info, switches),
                  ),
                ),
              ),
              const SizedBox(width: outlineGap),
              track.dimmed ? _detach(t) : _laneChip(t),
            ],
          ),
        )),
        // The row's bottom edge, dragged to set how many lane rows the track
        // stands on. Four pixels: wide enough to catch, narrow enough that a
        // press meant for the row lands on the row.
        if (onResize != null)
          Positioned(
            left: 0,
            right: 0,
            bottom: 0,
            height: 4,
            child: MouseRegion(
              cursor: SystemMouseCursors.resizeUpDown,
              child: Listener(
                key: ValueKey<String>('atl-resize-${track.id}'),
                behavior: HitTestBehavior.opaque,
                onPointerDown: (_) => onResizeStart?.call(),
                onPointerMove: (event) => onResize!(event.delta.dy),
              ),
            ),
          ),
      ]),
    );
  }

  /// The row's own controls, left of the trailing cell.
  Widget _head(
          LumitTheme t, BridgeLayerInfo info, BridgeLayerSwitches switches) =>
      Row(children: [
        _switch(t, 'audible', switches.audible, BridgeLayerSwitch.audible,
            mark: LumitIcons.audio,
            offMark: LumitIcons.muted,
            tip: switches.audible ? l10n.switchAudible : l10n.switchMuted),
        _switch(t, 'solo', switches.solo, BridgeLayerSwitch.solo,
            mark: LumitIcons.solo,
            offMark: LumitIcons.solo,
            tip: switches.solo ? l10n.switchSoloed : l10n.switchSolo),
        _switch(t, 'fx', switches.fx, BridgeLayerSwitch.fx,
            icon: LumitIcon.fx,
            tip: switches.fx
                ? l10n.switchEffectsOn
                : l10n.switchEffectsBypassed),
        const SizedBox(width: outlineGap),
        // The twirl has its own gesture, so opening a track does not also
        // pick it.
        LumitTooltip(
          message: track.open ? l10n.tipHideProperties : l10n.tipProperties,
          child: GestureDetector(
            key: ValueKey<String>('atl-twirl-${track.id}'),
            behavior: HitTestBehavior.opaque,
            onTap: onToggleOpen,
            child: SizedBox(
              width: 16,
              height: t.density.laneRow,
              child: Center(
                child: glyph.LumitIcon(
                  track.open ? LumitIcons.collapse : LumitIcons.expand,
                  size: iconSize,
                  colour: track.open ? t.textPrimary : t.textMuted,
                ),
              ),
            ),
          ),
        ),
        const SizedBox(width: identityGap),
        SizedBox(
          width: numberCellWidth,
          child: Text('${index + 1}',
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
        ),
        const SizedBox(width: identityGap),
        // The label dot, the same 6px bullet the layer Timeline marks a row
        // with. A readout here: the label picker belongs to the outline
        // that owns the layer.
        Container(
          width: 6,
          height: 6,
          decoration: BoxDecoration(
            color: t.labelColour(info.label),
            borderRadius: BorderRadius.circular(3),
          ),
        ),
        const SizedBox(width: identityGap),
        // The name, or the field Enter turns it into. The picked track's name
        // is the one thing on its row read at full strength, which is how the
        // layer Timeline marks its own.
        Expanded(
          child: rename == null
              ? Text(
                  info.name,
                  key: ValueKey<String>('atl-name-${track.id}'),
                  style: selected ? t.bodyPrimary : t.body,
                  overflow: TextOverflow.ellipsis,
                )
              : RowRenameField(
                  key: ValueKey<String>('atl-rename-${track.id}'),
                  controller: rename!,
                  onCommit: onRenameCommit,
                  onCancel: onRenameCancel,
                ),
        ),
      ]);

  /// A faded picture row wears this where every other track wears its chip:
  /// its sound cannot be mixed here until it stands on a track of its own.
  ///
  /// Outside the dimming, deliberately - the row is deaf to the pointer, and
  /// the one control that undoes that has to stay reachable.
  Widget _detach(LumitTheme t) => HouseButton(
        key: ValueKey<String>('atl-detach-${track.id}'),
        small: true,
        frameless: true,
        padding: const EdgeInsets.symmetric(horizontal: 4),
        onPressed: onDetach,
        child: Text(l10n.menuDetachAudio,
            style: t.small, overflow: TextOverflow.ellipsis),
      );

  /// Which picture this track's lane draws. **Wave and Spectral only**: the
  /// stack is a setting of the layer Timeline's lanes, and this panel's chip is
  /// its own state, so a choice here does not change a lane there.
  Widget _laneChip(LumitTheme t) => LumitTooltip(
        message: l10n.laneModeTooltip,
        child: GestureDetector(
          key: ValueKey<String>('atl-lane-mode-${track.id}'),
          behavior: HitTestBehavior.opaque,
          onTap: onCycleLaneMode,
          // Centred, and sized by its text: one line tall, the board's chip,
          // not the two lane rows the track spends. (An alignment on the box
          // would have it fill the row again.)
          child: Center(
            child: Container(
              width: audioTrackChipWidth,
              padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
              decoration: BoxDecoration(
                border: Border.all(color: t.hairlineStrong),
                borderRadius: BorderRadius.circular(2),
              ),
              child: Text(
                laneMode == LaneMode.spectral
                    ? l10n.laneModeSpectral
                    : l10n.laneModeWave,
                textAlign: TextAlign.center,
                style: t.mono.copyWith(fontSize: 9, color: t.textSecondary),
              ),
            ),
          ),
        ),
      );
}

/// The board's column header over the outline: the number, the name and the
/// lane chip each get their kicker, stood over the cells the rows below use,
/// so the words line up with what they name. The switches and the twirl carry
/// no word, as the layer Timeline's own header leaves them.
class AudioTrackHeaderRow extends StatelessWidget {
  const AudioTrackHeaderRow({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    Widget kicker(String text,
            {AlignmentGeometry align = Alignment.centerLeft}) =>
        Align(
          alignment: align,
          child: Text(text.toUpperCase(),
              style: t.kicker, maxLines: 1, overflow: TextOverflow.ellipsis),
        );
    return Container(
      key: const ValueKey<String>('atl-column-header'),
      height: t.density.timelineHeaderRow,
      color: t.surface1,
      padding:
          const EdgeInsets.only(left: outlineGap, right: outlineRowTrailing),
      child: Row(
        children: [
          // The three switch cells, the gap, the twirl and its gap: blank, as
          // the row's own controls say what they are.
          const SizedBox(width: switchCellWidth * 3 + outlineGap + 16),
          const SizedBox(width: identityGap),
          SizedBox(width: numberCellWidth, child: kicker(l10n.columnNumber)),
          const SizedBox(width: identityGap + 6 + identityGap),
          Expanded(child: kicker(l10n.columnTrack)),
          const SizedBox(width: outlineGap),
          SizedBox(
              width: audioTrackChipWidth,
              child: kicker(l10n.columnLane, align: Alignment.center)),
        ],
      ),
    );
  }
}
