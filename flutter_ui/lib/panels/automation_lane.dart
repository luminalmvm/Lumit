// The Audio timeline's automation lanes: Volume, Pan and one lane per send,
// opened by a track's twirl (docs/impl/audio-timeline.md §5).
//
// A lane is a value drawn as a curve across time, two lane rows tall and
// draggable taller like a track, with the track's wave faint behind it. Its
// outline carries the name, the value at the playhead and the keyframe
// controls every keyed row wears; its time area is the [VolumeBand] the clips
// already wear, handed the lane's own target, so a key dragged on either moves
// on both. Nothing new is stored: a lane reads the layer's Volume, Pan or
// send level and writes through the setters every other control uses.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';
import 'package:lumit_flutter/src/rust/api/routes.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'audio_routes_frb.dart' show muteSendRow, writeSend;
import '../widgets/marquee.dart';
import 'audio_timeline_live.dart';
import 'automation_pass_line.dart';
import 'graph_maths.dart' show rationalSeconds;
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart';
import 'mixer_panel_frb.dart' show panLabel;
import 'param_row_menu.dart';
import 'timeline_metrics_frb.dart' show dimmedRowOpacity;
import 'timeline_extras_frb.dart' show TimelineAxis, laneRowGap, rowSelectionFill;
import 'volume_band_frb.dart';
import 'waveform_frb.dart';

/// How many lane rows a lane stands on until its bottom edge is dragged.
const int automationLaneMinRows = 2;

/// Which value a lane draws.
enum LaneKind { volume, pan, send }

/// The paths a lane's keys are picked and shared under. Volume keeps the
/// path its row has in the layer Timeline, so a fold path means the same
/// thing in both; Pan sits beside it, and a send under the Sends heading as
/// its row did.
String laneVolumePath(String layerId) => '${audioPath(layerId)}/volume';
String lanePanPath(String layerId) => '${audioPath(layerId)}/pan';
String laneSendPath(String layerId, UuidValue bus) =>
    '${sendsPath(layerId)}/$bus';

/// One lane under a track: what it draws, where its keys are picked, and how
/// tall it stands.
class AutomationLane {
  final LaneKind kind;
  final BridgeScalar scalar;

  /// The send this lane rides, its place among the layer's sends, and its
  /// bus's name. Null on Volume and Pan.
  final BridgeSend? send;
  final int sendIndex;
  final String bus;

  /// The row path: what a picked key's id starts with.
  final String id;
  final int rows;
  final double rowHeight;

  const AutomationLane({
    required this.kind,
    required this.scalar,
    required this.id,
    required this.rows,
    required this.rowHeight,
    this.send,
    this.sendIndex = 0,
    this.bus = '',
  });

  double get height => rows * rowHeight;

  List<BridgeKeyframe> get keys => switch (scalar) {
        BridgeScalar_Keyframed(:final field0) => field0,
        _ => const [],
      };

  /// The fold row that moves this lane's keys in time, for the shared key
  /// drag. Pan has no row of its own, so its keys move on the lane alone.
  LayerFoldRow? get row => switch (kind) {
        LaneKind.volume => FoldVolumeRow(scalar: scalar, depth: 1),
        LaneKind.send => FoldSendRow(send!, bus: bus, depth: 1),
        LaneKind.pan => null,
      };

  /// The control this lane rides in the live mix, on [layer], or the
  /// master's Volume with none.
  LiveTarget live(LayerReference? layer) =>
      (layer: layer, kind: kind, bus: send?.bus);

  /// What the lane's band reads and writes, on [entry]. Given [comp] and
  /// [playing], a drag while the track records goes to the live mix.
  BandTarget target(BridgeLayerEntry entry,
      {CompositionReference? comp, ValueListenable<bool>? playing}) {
    final base = _target(entry);
    if (comp == null || playing == null) return base;
    return recordingBand(base,
        comp: comp,
        target: live(entry.layer),
        mode: entry.info.automationMode,
        playing: playing);
  }

  BandTarget _target(BridgeLayerEntry entry) => switch (kind) {
        LaneKind.volume => BandTarget.layer(entry, BandKind.volume,
            id: 'lane-$id', scale: BandScale.fader),
        LaneKind.pan => BandTarget.layer(entry, BandKind.pan, id: 'lane-$id'),
        LaneKind.send => BandTarget(
            scalar: scalar,
            kind: BandKind.volume,
            scale: BandScale.fader,
            locked: entry.info.switches.locked,
            id: 'lane-$id',
            write: (next) => writeSend(entry.layer, send!.bus, level: next),
            extent: BandTarget.extentOf(entry),
          ),
      };

  /// The lane's value written whole.
  void write(BridgeLayerEntry entry, BridgeScalar next) =>
      _target(entry).write(next);
}

/// [base] with its drags handed to the live mix while the comp plays and
/// [mode] records, so they are heard at once and written by the pass.
BandTarget recordingBand(
  BandTarget base, {
  required CompositionReference comp,
  required LiveTarget target,
  required BridgeAutomationMode mode,
  required ValueListenable<bool> playing,
}) =>
    base.withLive(
      touch: (value) =>
          laneRecords(mode, playing: playing.value) &&
          laneTouch(comp, target, value),
      release: () => laneRelease(comp, target),
    );

/// [entry]'s Volume or Pan band over its clips, its drags handed to the live
/// mix while the track records. The plain band with no [comp].
BandTarget trackBand(BridgeLayerEntry entry, BandKind kind,
    {required CompositionReference? comp,
    required ValueListenable<bool> playing}) {
  final base = BandTarget.layer(entry, kind);
  if (comp == null) return base;
  return recordingBand(base,
      comp: comp,
      target: (
        layer: entry.layer,
        kind: kind == BandKind.pan ? LaneKind.pan : LaneKind.volume,
        bus: null
      ),
      mode: entry.info.automationMode,
      playing: playing);
}

/// The lanes under [entry]: Volume, Pan, then a lane per send. [rows] is how
/// tall each has been dragged, by lane id.
List<AutomationLane> automationLanesOf({
  required BridgeLayerEntry entry,
  required double rowHeight,
  Map<String, int> rows = const {},
  Map<String, String> busNames = const {},
}) {
  final id = entry.layer.internallayerId.toString();
  AutomationLane lane(LaneKind kind, BridgeScalar scalar, String path,
          {BridgeSend? send, int index = 0}) =>
      AutomationLane(
        kind: kind,
        scalar: scalar,
        id: path,
        rows: rows[path] ?? automationLaneMinRows,
        rowHeight: rowHeight,
        send: send,
        sendIndex: index,
        bus: send == null ? '' : busNames[send.bus.toString()] ?? '',
      );
  return [
    lane(LaneKind.volume, entry.info.volumeDb, laneVolumePath(id)),
    lane(LaneKind.pan, entry.info.pan, lanePanPath(id)),
    for (final (i, send) in entry.info.sends.indexed)
      lane(LaneKind.send, send.level, laneSendPath(id, send.bus),
          send: send, index: i),
  ];
}

/// A lane's name.
String laneLabel(LumitTheme t, AutomationLane lane) => switch (lane.kind) {
      LaneKind.volume => t.propertyCase(l10n.volume),
      LaneKind.pan => t.propertyCase(l10n.audioPan),
      LaneKind.send => l10n.audioSendTo(lane.bus),
    };

/// The outline's end of a lane: the stopwatch, the name, the navigator, a
/// send's Pre switch, and the value at the playhead in a field. Dragging or
/// typing a keyed value keys it at the playhead, as every row does; the drag
/// is staged and written once, on release.
class AutomationLaneOutline extends StatefulWidget {
  final CompositionReference comp;
  final AutomationLane lane;

  /// The lane's name, and the write a finished edit makes.
  final String label;
  final void Function(BridgeScalar next) write;

  /// Where the value well goes, and how far in from the twirl the name starts.
  final ValueColumn valueColumn;
  final double indent;

  /// This lane is the picked property, so it draws lit.
  final bool selected;
  final VoidCallback onSelect;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  /// A locked layer's lane is read and not touched.
  final bool locked;

  /// The send's Pre switch, where the lane is a send's.
  final Widget? trailing;

  /// The layer the lane is on, for Mute send in a send lane's menu and the
  /// live value; null on the master's.
  final LayerReference? layer;

  /// While the comp plays, where the control stands now, shown in place of
  /// the curve's value so the head agrees with the fader during a ride.
  final AutomationPassFeed? pass;

  /// The lane's band, whose live hooks a drag of the field goes through
  /// while the track records ([BandTarget.touch]).
  final BandTarget? live;

  /// The drag on the lane's bottom edge, as on a track's.
  final ValueChanged<double>? onResize;
  final VoidCallback? onResizeStart;
  final VoidCallback? onResizeReset;

  const AutomationLaneOutline({
    super.key,
    required this.comp,
    required this.lane,
    required this.label,
    required this.write,
    required this.valueColumn,
    required this.indent,
    required this.selected,
    required this.onSelect,
    required this.onSeek,
    required this.onChanged,
    this.locked = false,
    this.trailing,
    this.layer,
    this.pass,
    this.live,
    this.onResize,
    this.onResizeStart,
    this.onResizeReset,
  });

  @override
  State<AutomationLaneOutline> createState() => _AutomationLaneOutlineState();
}

class _AutomationLaneOutlineState extends State<AutomationLaneOutline> {
  /// The value under the pointer during a drag. Sound is not redrawn, so a
  /// tick only holds the number and the release commits it.
  double? _staged;

  /// The live mix took this drag's moves, so its release writes nothing.
  bool _live = false;

  void _commitAt(BridgeScalar scalar, num value, int frame) {
    if (_live) {
      _dropLive();
    } else {
      widget.write(
          scalarWithValueAt(scalar, value.toDouble(), widget.comp, frame));
    }
    setState(() => _staged = null);
    widget.onChanged();
  }

  void _move(num value) {
    if (widget.live?.touch?.call(value.toDouble()) ?? false) _live = true;
    setState(() => _staged = value.toDouble());
  }

  /// A drag the live mix took, let go of.
  void _dropLive() {
    if (!_live) return;
    _live = false;
    widget.live?.release?.call();
  }

  @override
  void dispose() {
    _dropLive();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final lane = widget.lane;
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    final pan = lane.kind == LaneKind.pan;
    final positions = widget.pass?.positions;
    final row = ListenableBuilder(
      listenable: Listenable.merge([playhead, if (positions != null) positions]),
      builder: (context, _) {
        final frame = playhead.value;
        final scalar = lane.scalar;
        final animated = scalar is BridgeScalar_Keyframed;
        final at = positions?.value[
            widget.layer?.internallayerId.toString() ?? ''];
        final heard = at == null
            ? null
            : switch (lane.kind) {
                LaneKind.volume => at.volumeDb,
                LaneKind.pan => at.pan,
                LaneKind.send => lane.sendIndex < at.sends.length
                    ? at.sends[lane.sendIndex]
                    : null,
              };
        final value = _staged ??
            heard ??
            switch (scalar) {
              BridgeScalar_Static(:final field0) => field0,
              _ => sampledScalar(scalar, timeOfFrame(widget.comp, frame)),
            };
        final keys = KeyframeControlsFrb(
          scalars: [scalar],
          comp: widget.comp,
          onSeek: widget.onSeek,
          rowKey: 'atl-lane-${lane.id}',
          navigatorApart: true,
          onWrite: (next) {
            widget.write(next.single);
            widget.onChanged();
          },
        );
        // Pan runs minus a hundred to a hundred, the rest in dB on the
        // engine's own range: silence to a +12 dB boost.
        final min = pan ? -100.0 : -60.0;
        final max = pan ? 100.0 : 12.0;
        final suffix = pan ? null : ' dB';
        final decimals = pan ? 0 : 1;
        final speed = pan ? 1.0 : 0.2;
        // Pan reads as the Mixer reads it: C, L 30, R 20.
        final reading = pan ? (num v) => panLabel(v.toDouble()) : null;
        final fieldKey = ValueKey<String>('atl-lane-value-${lane.id}');
        // A lane stands on two rows at least, so the name and a send's Pre
        // switch take the first and the navigator and the value the second:
        // at the column's usual width the name keeps its room.
        final body = Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              height: lane.rowHeight,
              child: Row(children: [
                keys,
                const SizedBox(width: 4),
                Expanded(
                  child: Text(widget.label,
                      style: t.body,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis),
                ),
                if (widget.trailing case final trailing?) trailing,
                SizedBox(width: widget.valueColumn.rightInset),
              ]),
            ),
            SizedBox(
              height: lane.rowHeight - laneRowGap(t) * 2,
              child: Row(children: [
                const Spacer(),
                if (animated) keys.navigator(),
                const SizedBox(width: 4),
                SizedBox(
                  width: widget.valueColumn.width,
                  child: animated
                      ? KeyedValueField(
                          fieldKey: fieldKey,
                          value: value,
                          min: min,
                          max: max,
                          decimals: decimals,
                          suffix: suffix,
                          reading: reading,
                          speed: speed,
                          onLive: _move,
                          onCommit: (v) => _commitAt(scalar, v, frame),
                        )
                      : DragValueField(
                          key: fieldKey,
                          value: value,
                          min: min,
                          max: max,
                          decimals: decimals,
                          suffix: suffix,
                          reading: reading,
                          speed: speed,
                          onChanged: (v) => _commitAt(scalar, v, frame),
                          onChangeLive: _move,
                          onChangeEnd: (v) => _commitAt(scalar, v, frame),
                          onDragCancel: () {
                            _dropLive();
                            setState(() => _staged = null);
                          },
                        ),
                ),
                SizedBox(width: widget.valueColumn.rightInset),
              ]),
            ),
          ],
        );
        final menu = ParamRowMenu(
          menu: scalarRowMenu(context, keys, defaults: const [0]),
          child: body,
        );
        final send = lane.send, layer = widget.layer;
        return ParamRowPath(
          path: lane.id,
          child: send == null || layer == null
              ? menu
              : ParamRowExtras(
                  rows: (close) => [
                    muteSendRow(layer, send,
                        close: close, onChanged: widget.onChanged),
                  ],
                  // A muted send reads dimmed, as a muted layer's row does.
                  child: Opacity(
                      opacity: send.muted ? dimmedRowOpacity : 1,
                      child: menu),
                ),
        );
      },
    );
    return Listener(
      behavior: HitTestBehavior.opaque,
      onPointerDown: (_) {
        final keyboard = HardwareKeyboard.instance;
        if (keyboard.isControlPressed ||
            keyboard.isMetaPressed ||
            keyboard.isShiftPressed) {
          return;
        }
        widget.onSelect();
      },
      child: Container(
        key: ValueKey<String>('atl-lane-row-${lane.id}'),
        height: lane.height,
        padding: EdgeInsets.symmetric(vertical: laneRowGap(t)),
        child: Container(
          decoration: BoxDecoration(
            color: widget.selected ? rowSelectionFill(t) : null,
            borderRadius: t.shape == ThemeShape.lantern
                ? BorderRadius.circular(t.tokens.controlRadius)
                : null,
          ),
          padding: EdgeInsets.only(
              left: widget.indent, right: outlineRowTrailing),
          child: Stack(
            children: [
              Align(
                alignment: Alignment.topLeft,
                child: widget.locked
                    ? AbsorbPointer(child: Opacity(opacity: 0.5, child: row))
                    : row,
              ),
              if (widget.onResize != null)
                RowResizeEdge(
                  key: ValueKey<String>('atl-lane-resize-${lane.id}'),
                  onResize: widget.onResize!,
                  onResizeStart: widget.onResizeStart,
                  onResizeReset: widget.onResizeReset,
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The time area of a lane: the value as a curve across the lane's full
/// height, with the track's wave faint behind it. The curve is the clips'
/// band with the lane's target, so it takes the same drags, clicks and menu.
class AutomationLaneArea extends StatelessWidget {
  final AutomationLane lane;
  final BandTarget target;
  final TimelineAxis axis;
  final double fps;
  final int fpsNum;
  final int fpsDen;
  final double height;

  /// The track's wave over the lanes' window, or a bus's mix, or null for
  /// none.
  final BridgeAudioPeaks? peaks;
  final double originSeconds;
  final double secondsPerPixel;
  final double left;
  final double right;
  final WaveformStyle style;

  /// The lane's keys that are picked, by index.
  final Set<int> selected;
  final void Function(int index, bool additive) onSelectKey;
  final VoidCallback onChanged;

  /// The stretch a pass is writing, drawn over the curve while it runs.
  final AutomationPassFeed pass;
  final CompositionReference? comp;

  /// The wave's colours, where they are not the faint stack: a bus draws
  /// its mix in its own colour.
  final WaveformColours? waveColours;

  /// A converted track's clips, and each one's wave by clip id as the clips
  /// above fetched it, drawn faint where [peaks] would be.
  final List<BridgeClip> clips;
  final ValueListenable<Map<String, BridgeAudioPeaks>>? clipWaves;

  /// A box dragged on the lane's ground: the indices of the keys it caught,
  /// and whether it adds to the standing selection. Empty on a plain click.
  /// For a lane outside the panel's own marquee.
  final void Function(Set<int> caught, bool additive)? onMarquee;

  const AutomationLaneArea({
    super.key,
    required this.lane,
    required this.target,
    required this.axis,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    required this.height,
    required this.peaks,
    required this.originSeconds,
    required this.secondsPerPixel,
    required this.left,
    required this.right,
    required this.style,
    required this.selected,
    required this.onSelectKey,
    required this.onChanged,
    required this.pass,
    required this.comp,
    this.waveColours,
    this.clips = const [],
    this.clipWaves,
    this.onMarquee,
  });

  /// The keys inside [rect], by index.
  Set<int> _keysIn(Rect rect) => {
        for (var i = 0; i < lane.keys.length; i++)
          if (rect.left <= axis.xOf(laneKeyFrame(lane.keys[i], fps)) &&
              axis.xOf(laneKeyFrame(lane.keys[i], fps)) <= rect.right)
            i,
      };

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Context, not content: one full-range wave in one quiet tone, so a stack
    // of lanes under a track reads as lines over a shadow, not as the track
    // drawn again on every row.
    final wave = waveColours ?? t.waveform;
    final tone = wave.rest.withValues(alpha: wave.rest.a * 0.18);
    final colours =
        WaveformColours(rest: tone, low: tone, mid: tone, high: tone);
    final style = WaveformStyle(
        multiwave: false,
        fromBottom: this.style.fromBottom,
        scale: this.style.scale);
    final stack = Stack(
      children: [
        if (onMarquee case final marquee?)
          Positioned.fill(
            child: MarqueeSelect(
              key: ValueKey<String>('atl-lane-marquee-${lane.id}'),
              onSelect: (rect, additive) => marquee(_keysIn(rect), additive),
              onClear: () => marquee(const {}, false),
            ),
          ),
        if (clipWaves case final waves?)
          IgnorePointer(
            child: ValueListenableBuilder<Map<String, BridgeAudioPeaks>>(
              valueListenable: waves,
              builder: (context, got, _) => Stack(children: [
                for (final clip in clips)
                  if (got[clip.id.toString()] case final clipPeaks?)
                    Positioned(
                      left: axis.xOf(clip.startExact),
                      width: (axis.xOf(clip.endExact) - axis.xOf(clip.startExact))
                          .clamp(0.0, double.infinity),
                      top: 0,
                      height: height,
                      child: CustomPaint(
                        painter: WaveformPainter(
                          peaks: clipPeaks,
                          originSeconds: rationalSeconds(clip.placeStart),
                          secondsPerPixel: secondsPerPixel,
                          left: 0,
                          right: axis.xOf(clip.endExact) -
                              axis.xOf(clip.startExact),
                          colours: colours,
                          style: style,
                          height: height,
                        ),
                      ),
                    ),
              ]),
            ),
          ),
        if (peaks != null)
          IgnorePointer(
            child: CustomPaint(
              key: ValueKey<String>('atl-lane-wave-${lane.id}'),
              size: Size(axis.width, height),
              painter: WaveformPainter(
                peaks: peaks,
                originSeconds: originSeconds,
                secondsPerPixel: secondsPerPixel,
                left: left,
                right: right,
                colours: colours,
                style: style,
                height: height,
              ),
            ),
          ),
        RepaintBoundary(
          child: VolumeBand(
            target: target,
            axis: axis,
            fps: fps,
            fpsNum: fpsNum,
            fpsDen: fpsDen,
            barShift: 0,
            rowHeight: height,
            onChanged: onChanged,
            selected: selected,
            onSelectKey: onSelectKey,
            line: t.accent,
            flat: t.hairlineStrong,
          ),
        ),
        if (comp case final comp?)
          AutomationPassLine(
            feed: pass,
            comp: comp,
            laneId: lane.id,
            scale: target.scale,
            axis: axis,
            fps: fps,
            height: height,
          ),
      ],
    );
    return lane.send?.muted ?? false
        ? Opacity(opacity: dimmedRowOpacity, child: stack)
        : stack;
  }
}

/// How many lane rows a lane or a track stands on after a drag of [dy] on its
/// bottom edge, a whole row at a time: the outline and the lanes step
/// together and a wave never lands on half a row. [carry] is the travel short
/// of a row from earlier ticks of the same drag; the new carry comes back.
({int rows, double carry}) rowsAfterDrag({
  required int was,
  required double dy,
  required double carry,
  required double step,
  required int min,
  required int max,
}) {
  if (step <= 0) return (rows: was, carry: carry);
  carry += dy;
  final by = (carry / step).truncate();
  if (by == 0) return (rows: was, carry: carry);
  carry -= by * step;
  return (rows: (was + by).clamp(min, max), carry: carry);
}

/// A row's bottom edge, dragged to set how many lane rows it stands on. Four
/// pixels: wide enough to catch, narrow enough that a press meant for the row
/// lands on the row. Lit under the pointer, as the Cut timeline's edge is, so
/// the four pixels say they can be taken hold of. A double-click puts the row
/// back to its usual height.
class RowResizeEdge extends StatelessWidget {
  final ValueChanged<double> onResize;

  /// A press on the edge, before any travel. What a drag ends short of a
  /// whole lane row is carried, and a new drag must not spend it.
  final VoidCallback? onResizeStart;
  final VoidCallback? onResizeReset;

  const RowResizeEdge({
    super.key,
    required this.onResize,
    this.onResizeStart,
    this.onResizeReset,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Positioned(
      left: 0,
      right: 0,
      bottom: 0,
      height: 4,
      child: HoverMark(
        cursor: SystemMouseCursors.resizeUpDown,
        builder: (context, lit) => GestureDetector(
          onDoubleTap: onResizeReset,
          child: Listener(
            behavior: HitTestBehavior.opaque,
            onPointerDown: (_) => onResizeStart?.call(),
            onPointerMove: (event) => onResize(event.delta.dy),
            child: Align(
              alignment: Alignment.bottomCenter,
              child:
                  Container(height: 2, color: t.accent.withValues(alpha: lit)),
            ),
          ),
        ),
      ),
    );
  }
}
