// The captions of every caption layer in the fronted comp, held between
// edits.
//
// The engine decides where every caption is. They are read once per change of
// their layer and kept here, never in a build: a two-hour layer holds about
// 3,000 of them, and reading them is not free.

import 'dart:async';
import 'dart:math' as math;

import 'dart:ui' show Offset;

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/captions.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/state.dart' show ScopedChange;
import 'package:lumit_flutter/state/ui_state.dart';

double _seconds(BridgeRational r) => r.den == 0 ? 0 : r.num / r.den;

/// A caption layer's captions as the Timeline draws them: only those in the
/// edit, in the order they start, with their ends in comp frames.
class CaptionTrack {
  CaptionTrack._(this.cues, this.starts, this.ends, this.linked, this.source,
      this.fps, this.revision);

  factory CaptionTrack.of(BridgeCaptions captions, double fps, int? revision) {
    final shown = [
      for (final c in captions.cues)
        if (c.start != null && c.end != null) c
    ]..sort((a, b) => _seconds(a.start!).compareTo(_seconds(b.start!)));
    final starts = Float64List(shown.length);
    final ends = Float64List(shown.length);
    for (var i = 0; i < shown.length; i++) {
      starts[i] = _seconds(shown[i].start!) * fps;
      ends[i] = _seconds(shown[i].end!) * fps;
    }
    return CaptionTrack._(shown, starts, ends, captions.linked,
        captions.source?.toString(), fps, revision);
  }

  final List<BridgeCue> cues;
  final Float64List starts;
  final Float64List ends;

  /// Made from a layer and following it, so an edit to another layer can
  /// move these.
  final bool linked;

  /// That layer's id, while it is in the composition.
  final String? source;
  final double fps;

  /// The document revision these were read at.
  final int? revision;

  int get length => cues.length;

  /// How many captions start before [frame].
  int before(double frame) {
    var lo = 0, hi = starts.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (starts[mid] < frame) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return lo;
  }

  /// The first caption that may still show at [frame]: the one before the
  /// first to start there, which may not have ended yet.
  int firstFrom(double frame) => math.max(0, before(frame) - 1);

  /// The caption showing at [frame], by its place here.
  int? at(double frame) {
    var i = before(frame);
    // One starting exactly here shows here.
    if (i < starts.length && starts[i] == frame) return i;
    i -= 1;
    return i >= 0 && ends[i] > frame ? i : null;
  }

  bool raised(int i) => cues[i].offsetX != 0 || cues[i].offsetY != 0;

  /// Where caption [i] is by its place in the layer's own list.
  int? placeOf(int index) {
    for (var i = 0; i < cues.length; i++) {
      if (cues[i].index == index) return i;
    }
    return null;
  }
}

/// Every caption layer in the fronted comp, read when its layer changes and
/// kept for the Timeline and the Viewer to draw from, so neither asks the
/// engine while it draws.
class CaptionTracks extends ChangeNotifier {
  CaptionTracks(this._ui, Stream<ScopedChange> changes) {
    _changes = changes.listen(_onChange);
    _ui.model.addListener(_sync);
    // Not now: the first to ask may be in the middle of a build.
    scheduleMicrotask(() {
      if (!_gone) _sync();
    });
  }

  bool _gone = false;

  final LumitUiState _ui;
  late final StreamSubscription<ScopedChange> _changes;
  final Map<String, CaptionTrack> _tracks = {};

  /// Text layers that are not caption layers, so they are not asked again.
  final Set<String> _plain = {};

  /// How every other layer sits in time, last time it was looked at. A linked
  /// caption layer follows its source, so a change here reads it again.
  String? _timing;
  String? _comp;

  CaptionTrack? track(String id) => _tracks[id];
  bool has(String id) => _tracks.containsKey(id);
  Set<String> get ids => _tracks.keys.toSet();

  /// How far the caption showing on layer [id] at comp [frame] is moved off
  /// the layer's place, in its own pixels, or null when it is not.
  Offset? offsetAt(String id, int frame) {
    final track = _tracks[id];
    final i = track?.at(frame.toDouble());
    if (track == null || i == null || !track.raised(i)) return null;
    return Offset(track.cues[i].offsetX, track.cues[i].offsetY);
  }

  @override
  void dispose() {
    _gone = true;
    _changes.cancel();
    _ui.model.removeListener(_sync);
    super.dispose();
  }

  /// Read [layer]'s captions again now, after an edit made here.
  void reread(LayerReference layer) {
    if (_read(layer)) notifyListeners();
  }

  bool _read(LayerReference layer) {
    final id = layer.internallayerId.toString();
    final comp = _ui.selectedComp;
    BridgeCaptions? captions;
    int? revision;
    try {
      revision = comp?.documentRevision();
      final held = _tracks[id];
      if (held != null && revision != null && held.revision == revision) {
        return false;
      }
      captions = layer.captions();
    } catch (_) {
      captions = null;
    }
    if (captions == null) {
      _plain.add(id);
      return _tracks.remove(id) != null;
    }
    _plain.remove(id);
    _tracks[id] = CaptionTrack.of(captions, _ui.model.heldFps, revision);
    return true;
  }

  static String _r(BridgeRational r) => '${r.num}/${r.den}';

  static String _timingOf(BridgeLayerEntry e) {
    final b = StringBuffer()
      ..write(e.layer.internallayerId)
      ..write(_r(e.info.span.inPoint))
      ..write(_r(e.info.span.outPoint))
      ..write(_r(e.info.span.startOffset));
    for (final c in e.info.clips) {
      b
        ..write(c.id)
        ..write(_r(c.placeStart))
        ..write(_r(c.placeDuration))
        ..write(c.startFrame)
        ..write(c.endFrame)
        ..write(c.speedPercent)
        ..write(c.retimed);
    }
    return b.toString();
  }

  /// The model moved: find new caption layers, let go of gone ones, and read
  /// linked ones again when another layer has moved in time.
  void _sync() {
    final comp = _ui.selectedComp?.internalid.toString();
    var moved = false;
    if (comp != _comp) {
      _comp = comp;
      moved = _tracks.isNotEmpty;
      _tracks.clear();
      _plain.clear();
      _timing = null;
    }
    final fps = _ui.model.heldFps;
    final layers = _ui.model.heldLayers;
    final ids = <String>{};
    final timing = StringBuffer()..write(fps);
    for (final e in layers) {
      final id = e.layer.internallayerId.toString();
      ids.add(id);
      if (e.info.kind == BridgeLayerKind.text &&
          !_tracks.containsKey(id) &&
          !_plain.contains(id)) {
        moved |= _read(e.layer);
      }
      if (!_tracks.containsKey(id)) timing.write(_timingOf(e));
    }
    final before = _tracks.length;
    _tracks.removeWhere((id, _) => !ids.contains(id));
    _plain.removeWhere((id) => !ids.contains(id));
    moved |= _tracks.length != before;
    final now = timing.toString();
    if (_timing != null && now != _timing) {
      for (final e in layers) {
        final track = _tracks[e.layer.internallayerId.toString()];
        if (track != null && (track.linked || track.fps != fps)) {
          moved |= _read(e.layer);
        }
      }
    }
    _timing = now;
    if (moved) notifyListeners();
  }

  void _onChange(ScopedChange change) {
    final layer = change.layer;
    var moved = false;
    if (layer == null) {
      // Wider than one layer: an undo across several, or a transcript.
      if (change.items && change.item == null) return;
      for (final e in _ui.model.heldLayers) {
        if (_tracks.containsKey(e.layer.internallayerId.toString())) {
          moved |= _read(e.layer);
        }
      }
    } else if (layer.internalcompId.toString() == _comp) {
      final id = layer.internallayerId.toString();
      if (_tracks.containsKey(id) || _plain.contains(id)) {
        moved = _read(layer);
      }
      // A linked caption layer follows its source, Retime curve and all.
      for (final e in _ui.model.heldLayers) {
        if (_tracks[e.layer.internallayerId.toString()]?.source == id) {
          moved |= _read(e.layer);
        }
      }
    }
    if (moved) notifyListeners();
  }
}
