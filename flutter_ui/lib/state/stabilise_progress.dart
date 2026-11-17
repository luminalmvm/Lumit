// How far the Stabilise analyses in the comp on screen have got, for the
// Viewer's progress slot.
//
// An analysis starts by itself when the effect is added, and runs for a while
// on its own thread. The status line under the effect says how it is getting
// on, but only while the Effect controls panel shows that layer; the Viewer's
// transport says it wherever you are, in the slot a slow preview frame uses.
//
// **Read, not subscribed to**, like the status lines: the instances are found
// in the read model the panels already hold, read again when the document's
// revision moves, and sampled on the status lines' clock only while one of
// them is running. The engine decides whether an edit changed what an
// analysis is keyed by, once per revision, and a read is what asks for the new
// one, wherever the status line is.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/stabilise.dart';
import 'package:lumit_flutter/src/rust/api/track.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../panels/status_poller.dart' show statusPoll;
import 'ui_state.dart';

/// Every running Stabilise in the comp on screen, summed into one bar.
class StabiliseProgress extends ChangeNotifier {
  StabiliseProgress(this._ui) {
    _ui.model.addListener(_onModel);
    // Made from a build, so the first read waits for the frame.
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (!_gone) _onModel();
    });
  }

  final LumitUiState _ui;
  Timer? _timer;
  bool _gone = false;

  /// What the last read was for, from [_keyOf], so a model notice that moved
  /// nothing asks nothing.
  String _readFor = '';

  /// Since the last read, so edits read at most once a [statusPoll].
  final Stopwatch _sinceRead = Stopwatch();

  /// A read put off until a [statusPoll] has passed since the last.
  Timer? _due;

  /// Each instance's stage at the last read, to see an analysis land.
  final Map<UuidValue, BridgeTrackStage> _stages = {};

  /// The analyses the bar counts: every instance seen queued or running since
  /// the bar came up, finished ones included, so it fills once for them all.
  final Set<UuidValue> _batch = {};

  bool _visible = false;
  double _fraction = 0;
  String _label = '';

  /// True while an analysis is queued or running.
  bool get visible => _visible;

  /// Frames analysed over frames asked for, 0..1.
  double get fraction => _fraction;

  /// What the analyses are doing, in words.
  String get label => _label;

  /// The enabled Stabilise instances taking the shake out, from the read model.
  List<({LayerReference layer, UuidValue effect})> _instances() => [
        for (final entry in _ui.model.heldLayers)
          for (final fx in entry.info.effects)
            if (_counts(fx)) (layer: entry.layer, effect: fx.id),
      ];

  static bool _counts(BridgeEffectInstanceInfo fx) =>
      fx.name == 'stabilise' && fx.enabled && !_reverses(fx.values);

  /// The document revision and the instances: every edit, the work area,
  /// Interpret footage and a replaced file included, moves the first. Empty
  /// with no Stabilise in the comp, which has nothing to read.
  String _keyOf() {
    final found = _instances();
    if (found.isEmpty) return '';
    return [
      '${_ui.model.heldRevision}',
      for (final i in found) '${i.effect}',
    ].join(',');
  }

  static bool _reverses(List<BridgeParamValue> values) {
    for (final v in values) {
      if (v.id != 'mode') continue;
      if (v.value case BridgeEffectValue_Choice(:final field0)) {
        return field0 == 1;
      }
    }
    return false;
  }

  void _onModel() {
    final key = _keyOf();
    if (key == _readFor) return;
    _readFor = key;
    // The clock, or a read already put off, reads the new instances anyway.
    if (_timer != null || _due != null) return;
    final wait = statusPoll - _sinceRead.elapsed;
    if (!_sinceRead.isRunning || wait <= Duration.zero) {
      _sample(_instances());
      return;
    }
    _due = Timer(wait, () {
      _due = null;
      _sample(_instances());
    });
  }

  /// A status line read, or something started an analysis without an edit,
  /// such as a press of Analyse. Read now, and restart the clock from here, so
  /// the transport keeps step with the line.
  void wake() {
    _timer?.cancel();
    _timer = null;
    _sample(_instances());
  }

  void _sample(List<({LayerReference layer, UuidValue effect})> found) {
    if (_gone) return;
    _sinceRead
      ..reset()
      ..start();
    var share = 0.0;
    var tracking = false;
    var solving = false;
    var queued = false;
    var landed = false;
    final seen = <UuidValue>{};
    for (final i in found) {
      final BridgeStabiliseStatus status;
      try {
        status = stabiliseStatus(layer: i.layer, effect: i.effect);
      } catch (_) {
        // The layer went away under the read; it simply stops counting.
        continue;
      }
      seen.add(i.effect);
      final was = _stages[i.effect];
      _stages[i.effect] = status.stage;
      // Each analysis is one share of the bar. A queued one has no frame
      // count yet, so frames cannot be summed without the bar going back.
      switch (status.stage) {
        case BridgeTrackStage.tracking:
          tracking = true;
          _batch.add(i.effect);
          if (status.total > 0) share += status.done / status.total;
        case BridgeTrackStage.solving:
          solving = true;
          _batch.add(i.effect);
          share += 1;
        case BridgeTrackStage.queued:
          queued = true;
          _batch.add(i.effect);
        case BridgeTrackStage.done:
          landed |= was == BridgeTrackStage.tracking ||
              was == BridgeTrackStage.solving ||
              was == BridgeTrackStage.queued;
          if (_batch.contains(i.effect)) share += 1;
        default:
          if (_batch.contains(i.effect)) share += 1;
      }
    }
    _stages.removeWhere((id, _) => !seen.contains(id));
    _batch.retainWhere(seen.contains);

    final visible = tracking || solving || queued;
    if (!visible) _batch.clear();
    final fraction = _batch.isEmpty ? 0.0 : share / _batch.length;
    final label = tracking
        ? l10n.stabiliseProgressAnalysing
        : solving
            ? l10n.stabiliseSolving
            : l10n.stabiliseProgressWaiting;
    final changed =
        visible != _visible || fraction != _fraction || label != _label;
    _visible = visible;
    _fraction = fraction;
    _label = label;
    if (changed) notifyListeners();

    // An analysis landing changes the frames without moving the playhead or
    // the revision, so the picture is asked for again here as well as by the
    // status line, which may not be on screen.
    if (landed) {
      _ui.solveLanded.value++;
      _ui.requestFrame();
    }

    if (visible) {
      _timer ??= Timer.periodic(statusPoll, (_) => _sample(_instances()));
    } else {
      _timer?.cancel();
      _timer = null;
    }
  }

  @override
  void dispose() {
    _gone = true;
    _timer?.cancel();
    _due?.cancel();
    _ui.model.removeListener(_onModel);
    super.dispose();
  }
}
