// Hearing speech in the open project: the transcription that is running, and
// starting one.
//
// # In plain terms
//
// One transcription runs at a time, on the engine's own thread. Every panel
// that shows its progress reads [TranscriptionState.progress], fed by one
// listener on the project, so a job started anywhere (the Captions panel, Cut's
// transcript, the Project panel) shows everywhere and finishes everywhere.
//
// Starting one asks for the speech model first when it is not installed. The
// asking is a dialog, so the caller hands it in: `shell/speech_install_dialog_frb.dart`
// has `transcribeWithPrompt`, which is what a panel calls.

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/transcript.dart';

class TranscriptionState {
  /// The running job's latest step, or null while none runs.
  final ValueNotifier<BridgeTranscribeEvent?> progress = ValueNotifier(null);

  StreamSubscription<BridgeTranscribeEvent>? _watch;
  ProjectReference? _project;

  /// Follow `project`'s transcriptions, whoever starts them. Called when a
  /// project becomes the open one.
  void adopt(ProjectReference project) {
    unawaited(_watch?.cancel());
    _project = project;
    progress.value = project.transcriptionProgress();
    _watch = project.watchTranscription().listen(_heard, onError: (_) {});
  }

  void _heard(BridgeTranscribeEvent event) {
    final over = event.stage == BridgeTranscribeStage.cancelled ||
        (event.fraction >= 1 &&
            (event.stage == BridgeTranscribeStage.done ||
                event.stage == BridgeTranscribeStage.failed));
    progress.value = over ? null : event;
  }

  /// Whether a transcription of the open project is running.
  bool get running => progress.value != null;

  /// Hear `items`, after [install] has put the speech model in place if it is
  /// missing. Null when nothing ran: a job already running, the install
  /// turned down, or the engine refusing.
  Future<BridgeTranscribeOutcome?> start(
    List<FootageReference> items,
    BridgeTranscribeOptions options, {
    required Future<bool> Function() install,
  }) async {
    final project = _project;
    if (project == null || running || items.isEmpty) return null;
    final ready = speechReady();
    if (!ready.runtime || !ready.pack) {
      if (!await install()) return null;
      if (!speechReady().pack) return null;
    }
    try {
      return await project.transcribe(items: items, options: options);
    } catch (_) {
      return null;
    } finally {
      progress.value = null;
    }
  }

  /// Stop the running transcription. What has finished stays.
  void cancel() => _project?.cancelTranscription();

  void dispose() {
    unawaited(_watch?.cancel());
    progress.dispose();
  }
}
