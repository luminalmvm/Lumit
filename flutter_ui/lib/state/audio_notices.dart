// The status line for the sound's own troubles: the output it played
// through went away and it moved to another, or some of it was played as
// silence because it was not ready in time.

import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';

import 'app_state.dart';

/// Reads the counts the audio clock carries and says when either moves:
/// a change of output each time, falling behind once a playback.
class AudioNotices {
  int? _changes;
  int? _starved;
  bool _toldStarved = false;

  /// A playback starts: falling behind may be said once more.
  void restart() => _toldStarved = false;

  void read(BridgeAudioClock clock, LumitState app) {
    final changes = _changes, starved = _starved;
    _changes = clock.deviceChanges;
    _starved = clock.starvedMs;
    if (changes != null && clock.deviceChanges != changes) {
      app.postNotice(l10n.audioOutputMoved);
    }
    if (starved != null && clock.starvedMs > starved && !_toldStarved) {
      _toldStarved = true;
      app.postNotice(l10n.audioFellBehind);
    }
  }
}

/// The one reader, shared by whatever polls the clock.
final audioNotices = AudioNotices();
