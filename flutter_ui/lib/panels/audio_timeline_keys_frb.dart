// Where the Audio timeline's keyframes are, and the edits on a lane's keys
// that the graph's channels do not make (docs/impl/audio-timeline.md §5).
//
// One walk down the tracks places every key - the band's rows, the open
// lanes, the twirl rows - so the marquee, the drag commit and the selection
// cannot disagree about which key is where. Volume keys are the graph's own
// channel and so Delete, Copy and Paste reach them through it; Pan and send
// keys have no channel, so the same edits are made here through the lane's
// own writer, with the clipboard the graph keeps. The master's Volume lane
// is one of those too.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import '../l10n/strings.dart';
import 'audio_master_track.dart' show masterLaneId;
import 'audio_timeline_rows_frb.dart';
import 'automation_lane.dart';
import 'graph_channels.dart' show GraphChannel, keyFrame;
import 'graph_clipboard.dart' show graphKeyClipboard;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart';
import 'timeline_extras_frb.dart' show TimelineAxis;
import 'timeline_lane_area_frb.dart' show SelectedKey;

/// How far along the comp a row's keys are drawn from their own zero.
///
/// Zero for a track's own rows, whose keys are already in comp time. A
/// **clip's** parameter is keyed from the clip's start, as its Retime is, so
/// its keys are drawn a clip's start along and written back that far short -
/// `startFrame` is the clip's place with the layer's own zero already added.
int clipShiftOf(AudioTrackRow track, LayerFoldRow row) {
  if (row is! FoldEffectParamRow || row.clip == null) return 0;
  for (final clip in track.entry.info.clips) {
    if (clip.id == row.clip) return clip.startFrame.toInt();
  }
  return 0;
}

/// Every keyframe on the lanes and the twirl rows, with where it sits.
///
/// [drawn] is where a clip's keys are measured: on the comp's clock, which is
/// where the marquee and the snap look for them, or on the clip's own, which
/// is what a commit writes back. A Pan lane's keys are placed only for the
/// marquee: no fold row moves them, so the shared key drag leaves them where
/// they are and the lane's own drag is how they travel.
List<SelectedKey> audioKeyPlaces(List<AudioTrackRow> tracks, double fps,
    {bool drawn = true}) {
  final out = <SelectedKey>[];
  var y = 0.0;
  for (final track in tracks) {
    final step = track.rowHeight;
    // The track's own band comes first, then its lanes, then the rows.
    y += track.laneHeight;
    for (final lane in track.drawnLanes) {
      final row = lane.row;
      if (drawn || row != null) {
        final keys = lane.keys;
        for (var i = 0; i < keys.length; i++) {
          out.add(SelectedKey(
            entry: track.entry,
            row: row ?? FoldVolumeRow(scalar: lane.scalar, depth: 1),
            rowId: lane.id,
            index: i,
            frame: laneKeyFrame(keys[i], fps),
            top: y,
            height: lane.height,
          ));
        }
      }
      y += lane.height;
    }
    for (final row in track.drawnRows) {
      final rowId = foldRowPath(track.id, row);
      final keys = laneKeysOf(row);
      final shift = drawn ? clipShiftOf(track, row) : 0;
      for (var i = 0; i < keys.length; i++) {
        out.add(SelectedKey(
          entry: track.entry,
          row: row,
          rowId: rowId,
          index: i,
          frame: laneKeyFrame(keys[i], fps) + shift,
          top: y,
          height: step,
        ));
      }
      y += step;
    }
  }
  return out;
}

/// Every keyframe the marquee's box caught, walking the same rows the lanes
/// draw.
Set<String> audioKeysIn(
    Rect rect, List<AudioTrackRow> tracks, TimelineAxis axis, double fps) {
  final caught = <String>{};
  for (final place in audioKeyPlaces(tracks, fps)) {
    if (place.top + place.height < rect.top || place.top > rect.bottom) {
      continue;
    }
    final x = axis.xOf(place.frame);
    if (x >= rect.left && x <= rect.right) {
      caught.add('${place.rowId}#${place.index}');
    }
  }
  return caught;
}

/// Each track's own band and each open lane, so the seam painter rules
/// between them and not through a wave or a curve, and the ground under the
/// last track, which is not a row and wears no lines. The twirl's rows are
/// outside the blanks and keep their hairlines, which is what makes an open
/// track read as a list.
List<(double, double)> audioSeamBlanks(List<AudioTrackRow> tracks) {
  final out = <(double, double)>[];
  var y = 0.0;
  for (final track in tracks) {
    out.add((y, y + track.laneHeight));
    y += track.laneHeight;
    for (final lane in track.drawnLanes) {
      out.add((y, y + lane.height));
      y += lane.height;
    }
    y += track.rowHeight * track.drawnRows.length;
  }
  return out..add((y, 1e7));
}

/// The indices of [laneId]'s keys among the picked ids.
Set<int> laneSelection(Set<String> selected, String laneId) {
  final prefix = '$laneId#';
  return {
    for (final id in selected)
      if (id.startsWith(prefix))
        if (int.tryParse(id.substring(prefix.length)) case final i?) i,
  };
}

/// The master's Volume lane for the key edits: the lane, its write, and a
/// layer of the comp for the clipboard to name its source by, or null in a
/// comp with no layers.
typedef MasterLaneKeys = ({
  AutomationLane lane,
  void Function(BridgeScalar next) write,
  BridgeLayerEntry? entry
});

/// A lane whose keys the graph's channels cannot reach: its name, the layer
/// it stands on (a track's own, or for the master any layer of the comp,
/// which a bare channel's writers never touch), and its write.
typedef _OwnLane = ({
  String label,
  BridgeLayerEntry? entry,
  AutomationLane lane,
  void Function(BridgeScalar next) write
});

/// The lanes among [tracks] whose keys the graph's channels cannot reach:
/// Pan, the sends and the master's Volume, by lane id.
Map<String, _OwnLane> _laneOnlyRows(
    List<AudioTrackRow> tracks, MasterLaneKeys? master) {
  return {
    for (final track in tracks)
      for (final lane in track.lanes)
        if (lane.kind != LaneKind.volume)
          lane.id: (
            label: '${track.entry.info.name} · ${lane.kind == LaneKind.pan
                ? l10n.audioPan
                : l10n.audioSendTo(lane.bus)}',
            entry: track.entry,
            lane: lane,
            write: (next) => lane.write(track.entry, next),
          ),
    if (master != null)
      master.lane.id: (
        label: '${l10n.mixerMaster} · ${l10n.volume}',
        entry: master.entry,
        lane: master.lane,
        write: master.write,
      ),
  };
}

/// Pan, send and master lanes as bare channels, for [paths] among them: the
/// selection fans out on them and Copy takes their keys, exactly as for a
/// Volume. The graph's own writer passes them by, so [deleteLaneKeys] and
/// [pasteOnLanes] write them. A comp with no layers has no channel for the
/// master, whose keys are then deleted but not copied.
List<GraphChannel> laneChannels(
    List<AudioTrackRow> tracks, Iterable<String> paths,
    {MasterLaneKeys? master}) {
  final lanes = _laneOnlyRows(tracks, master);
  final out = <GraphChannel>[];
  for (final path in paths) {
    if (lanes[path] case (:final label, :final entry?, :final lane, write: _)) {
      out.add(GraphChannel(
        path: path,
        id: path,
        label: label,
        colourIndex: out.length,
        scalar: lane.scalar,
        entry: entry,
      ));
    }
  }
  return out;
}

/// The picked keys on Pan, send and master lanes, gone. Lifting the last leaves the
/// value static at what the key read, as the band does. Says whether anything
/// was written; the caller holds the undo step.
bool deleteLaneKeys({
  required List<AudioTrackRow> tracks,
  required Set<String> selected,
  MasterLaneKeys? master,
}) {
  var changed = false;
  for (final (label: _, entry: _, :lane, :write)
      in _laneOnlyRows(tracks, master).values) {
    final picked = laneSelection(selected, lane.id);
    final keys = lane.keys;
    if (picked.isEmpty || keys.isEmpty) continue;
    final rest = <BridgeKeyframe>[];
    double last = 0;
    for (var i = 0; i < keys.length; i++) {
      picked.contains(i) ? last = keys[i].value : rest.add(keys[i]);
    }
    try {
      write(rest.isEmpty
          ? BridgeScalar.static_(last)
          : BridgeScalar.keyframed(rest));
      changed = true;
    } catch (_) {
      // A locked layer refuses, and quietly.
    }
  }
  return changed;
}

/// The keyframe clipboard put down at the playhead on the Pan and send lanes
/// among [channels], the clipboard's entries matched to the channels by
/// place as the graph's own paste matches them. The earliest copied key lands
/// on the playhead and the rest keep their spacing; a key already on a landed
/// frame is replaced, and a copied value with no keys is written at the
/// playhead as typing it would be. Says whether anything was written.
bool pasteOnLanes({
  required List<AudioTrackRow> tracks,
  required List<GraphChannel> channels,
  required CompositionReference comp,
  required int playheadFrame,
  required double fps,
  required int fpsNum,
  required int fpsDen,
  MasterLaneKeys? master,
}) {
  final lanes = _laneOnlyRows(tracks, master);
  final clips = graphKeyClipboard;
  if (clips.isEmpty) return false;
  var earliest = double.infinity;
  for (final clip in clips) {
    for (final k in clip.keys) {
      earliest = earliest < keyFrame(k, fps) ? earliest : keyFrame(k, fps);
    }
  }
  final shift = earliest.isFinite ? playheadFrame - earliest : 0.0;
  var changed = false;
  for (var i = 0; i < channels.length && i < clips.length; i++) {
    final found = lanes[channels[i].path];
    if (found == null) continue;
    final lane = found.lane;
    final clip = clips[i];
    final BridgeScalar next;
    if (clip.staticValue case final value?) {
      next = scalarWithValueAt(lane.scalar, value, comp, playheadFrame);
    } else if (clip.keys.isEmpty) {
      continue;
    } else {
      final merged = <double, BridgeKeyframe>{
        for (final k in lane.keys) keyFrame(k, fps): k,
      };
      for (final k in clip.keys) {
        final at = keyFrame(k, fps) + shift;
        merged[at] = BridgeKeyframe(
          time: timeOfSubframe(at, fpsNum, fpsDen),
          value: k.value,
          interpIn: k.interpIn,
          interpOut: k.interpOut,
        );
      }
      final frames = merged.keys.toList()..sort();
      next = BridgeScalar.keyframed([for (final f in frames) merged[f]!]);
    }
    try {
      found.write(next);
      changed = true;
    } catch (_) {
      // A locked layer refuses, and quietly.
    }
  }
  return changed;
}

/// The lane selection [selected] said in channel terms: a diamond stands for
/// its row, so `row#i` fans out to each channel of that path. The master's
/// keys stand for themselves where it has no channel, so Delete still sees
/// them.
Set<String> keysOnChannels(Set<String> selected, List<GraphChannel> channels) {
  final out = <String>{
    if (!channels.any((c) => c.path == masterLaneId))
      for (final id in selected)
        if (id.startsWith('$masterLaneId#')) id,
  };
  for (final id in selected) {
    final hash = id.lastIndexOf('#');
    if (hash <= 0) continue;
    final path = id.substring(0, hash);
    final index = int.tryParse(id.substring(hash + 1));
    if (index == null) continue;
    for (final channel in channels) {
      if (channel.path != path) continue;
      if (channel.ownKeyIndex(index) case final own?) {
        out.add('${channel.id}#$own');
      }
    }
  }
  return out;
}
