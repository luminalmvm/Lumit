// The Cut timeline's bus tracks: a Bus layer stands below the line after the
// sound tracks, takes no clips, and draws the mix it carries.

import 'dart:async';
import 'dart:ui' show Offset, VoidCallback;

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import 'cut_timeline_tracks.dart';
import 'waveform_frb.dart' show WaveformRequest;

/// The mix [bus] carries over a stretch of comp seconds.
Future<BridgeAudioPeaks> busMixPeaks(CompositionReference comp, UuidValue bus,
        {required double startSeconds,
        required double endSeconds,
        required int buckets}) =>
    comp.audioMixPeaks(
        startSeconds: startSeconds,
        endSeconds: endSeconds,
        buckets: buckets,
        bus: bus);

/// Whether [layer] is a bus track's, which takes no clips.
bool cutIsBusLayer(List<CutTrack> tracks, LayerReference layer) => tracks.any(
    (t) => t.bus && t.entry.layer.internallayerId == layer.internallayerId);

/// What is let go or pasted on a bus track is refused, and said so.
void cutRefuseOnBus(LumitState app) => app.postNotice(l10n.cutRefusedBusTrack);

/// Whether the point [at] on the lanes is over a bus track.
bool cutOverBus(List<CutTrack> tracks, List<double> tops, Offset? at) {
  final index = at == null ? null : cutTrackAt(tracks, at.dy, tops);
  return index != null && tracks[index].bus;
}

/// Each bus track's mix over the lanes' window, asked for the way the
/// Timeline's Sound mix row asks for the comp's: by the window and the
/// document revision, since the mix changes with every edit. An empty answer
/// is the engine still preparing the mix, so the track asks again a moment
/// later, a handful of times, then leaves it until the window or the document
/// moves.
class CutBusPeaks {
  final Map<String, String> _keys = {};

  /// How many times an empty answer has been asked again, and the key the
  /// count belongs to. The ask-again drops the track's key to come back
  /// through the fetch, so the count is held apart from it.
  final Map<String, (String, int)> _asks = {};
  final Map<String, Timer> _retries = {};

  void want({
    required CompositionReference comp,
    required CutTrack track,
    required WaveformRequest request,
    required int? revision,
    required bool Function() alive,
    required void Function(BridgeAudioPeaks peaks) onPeaks,
    required VoidCallback onRetry,
  }) {
    final id = track.id;
    final key = '${request.key}|$revision';
    if (_keys[id] == key) return;
    if (_asks[id]?.$1 != key) _asks[id] = (key, 0);
    _keys[id] = key;
    busMixPeaks(comp, track.entry.layer.internallayerId,
            startSeconds: request.startSeconds,
            endSeconds: request.endSeconds,
            buckets: request.buckets)
        .then((peaks) {
      if (!alive() || _keys[id] != key) return;
      if (peaks.buckets == 0) {
        final asks = _asks[id]?.$2 ?? 0;
        if (asks >= 8) return;
        _asks[id] = (key, asks + 1);
        _retries.remove(id)?.cancel();
        _retries[id] = Timer(const Duration(milliseconds: 400), () {
          _retries.remove(id);
          if (!alive() || _keys[id] != key) return;
          _keys.remove(id);
          onRetry();
        });
        return;
      }
      onPeaks(peaks);
    }, onError: (Object _) {});
  }

  /// Forget every track not in [live], which has left the view.
  void keep(Set<String> live) {
    _keys.removeWhere((id, _) => !live.contains(id));
    _asks.removeWhere((id, _) => !live.contains(id));
    _retries.removeWhere((id, timer) {
      if (live.contains(id)) return false;
      timer.cancel();
      return true;
    });
  }

  void dispose() {
    for (final timer in _retries.values) {
      timer.cancel();
    }
    _retries.clear();
  }
}
