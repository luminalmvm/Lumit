// Running beat detection, and the sentence it leaves on the status line.
//
// Three surfaces offer detection (the Audio panel's Generate, the Timeline's
// more menu, Composition ▸ Detect beats) and all three come through here, so a
// run started from the menu shows the same card, the same bar and the same
// words as a run started from the panel. It used to be one function for the
// sentence alone, and the menu path had no card at all.

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/shell/splash.dart';
import 'package:lumit_flutter/src/rust/api/beats.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/state/app_state.dart';

import '../l10n/strings.dart';

/// Detect this comp's beats with the card up over the shell, the engine's own
/// progress on its bar, and the answer on the status line.
///
/// [onFound] is what the caller does with a finished run — redraw the panel it
/// lives in, keep the tempo it was told. [noSound] is the sentence for a comp
/// with nothing to hear, for the caller that knows a better one than the
/// general "nothing in this comp sounds": a layer asked for by name cannot be
/// silenced by a mute or a solo, so blaming them would send the reader to the
/// wrong switch.
Future<void> runBeatDetection({
  required LumitState app,
  required CompositionReference comp,
  required BridgeBeatOptions options,
  void Function(BridgeBeatsResult found)? onFound,
  String? noSound,
}) {
  // Determinate from the first frame: the card keeps whichever bar it opened
  // with (shell/splash.dart), so the fraction is set before the card goes up.
  app.busyProgress.value = 0;
  final progress = RustStreamSink<double>();
  // The call is started before the sink is listened to, as an open's is: a
  // sink has no stream until it has been handed to a call, and nothing is lost
  // in between because the stream buffers what arrives first.
  final pending =
      comp.detectBeats(options: options, onProgressStream: progress);
  final watching = progress.stream.listen((fraction) {
    // Never backwards: a late report behind a later one would pull the fill
    // back, which reads as work being undone.
    if (fraction >= (app.busyProgress.value ?? 0)) {
      app.busyProgress.value = fraction;
    }
  });
  return showBusyWhile(
    app.busy,
    l10n.detectingBeats,
    pending.then<void>(
      (found) {
        onFound?.call(found);
        // A run that placed nothing is a legitimate answer (docs/09 §5) and
        // used to be an indistinguishable one: no markers, no grid, no word. A
        // run that placed markers says so too, because the markers land
        // off-screen as easily as on.
        app.postNotice(
            found.placed == 0 ? l10n.beatsNoneFound : beatsFoundNotice(found));
      },
      // **A refusal says why.** `NoAudio` is what the engine answers here in
      // every case a person can cause (docs/09 §5), the rest being a project
      // that closed, and a `BridgeError` reaches Dart as an opaque handle with
      // nothing readable on it. So one sentence per source rather than one per
      // reason.
      onError: (_) => app.postNotice(noSound ?? l10n.beatsNoSound),
    ),
  ).whenComplete(() {
    watching.cancel();
    app.busyProgress.value = null;
  });
}

/// What a detection that placed markers says (the AudioWorkspace board's own
/// status caption): the confirmed tempo and the count, or the count alone when
/// no grid stood. Its own function because a run that placed nothing, and a
/// run that was refused, say something else entirely.
String beatsFoundNotice(BridgeBeatsResult found) => found.bpm > 0
    ? l10n.beatsGridConfirmed(found.bpm.toStringAsFixed(0), '${found.placed}')
    : l10n.beatsPlaced('${found.placed}');
