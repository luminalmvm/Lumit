// The Roto brush's status, in words (K-713, docs/08 §3.96).
//
// **In plain terms.** Cutting a subject out of a whole shot takes a while, and
// it happens somewhere else — on its own thread, over the media file, while you
// carry on editing. These are the sentences that say how it is getting on: how
// many frames are done, how many of them were copied rather than worked out
// again, and how much of the shot the matte actually covers. When it cannot be
// done, it says why, calmly, and nothing about the shot has changed.
//
// **Only the words are here.** The rows, the buttons and the overlay are the
// Roto brush's panel (RB3); what this file owns is the mapping from what the
// engine says to what a person reads, so it can be asserted directly instead of
// through a mounted widget.

import 'package:lumit_flutter/src/rust/api/roto.dart';

import '../l10n/strings.dart';

/// The sentence for a refusal.
///
/// The engine sends a **reason**, never these words (K-303): its own
/// `RotoFailure` carries English, and English crossing the bridge would ship
/// untranslated inside a translated window. The switch is exhaustive over the
/// generated enum, so a reason added to the engine is a compile error here
/// rather than a blank line on screen.
String rotoFailureSentence(BridgeRotoFailure failure) => switch (failure) {
      BridgeRotoFailure.offline => l10n.rotoFailedOffline,
      BridgeRotoFailure.flowUnavailable => l10n.rotoFailedFlowUnavailable,
      BridgeRotoFailure.busy => l10n.rotoFailedBusy,
      BridgeRotoFailure.noBaseFrame => l10n.rotoFailedNoBaseFrame,
      BridgeRotoFailure.unreadable => l10n.rotoFailedUnreadable,
      BridgeRotoFailure.noFrames => l10n.rotoFailedNoFrames,
      BridgeRotoFailure.noSeeds => l10n.rotoFailedNoSeeds,
    };

/// The sentence for one reading of the propagation.
///
/// Pulled out of any widget so it can be asserted directly: what a status
/// *says* is a decision about wording, and testing it through a mounted widget
/// would be testing the mounting.
String rotoStatusSentence(BridgeRotoStatus? status) {
  if (status == null) return '';
  switch (status.stage) {
    case BridgeRotoStage.idle:
      // Nothing has been asked for yet. What is worth saying is whether there
      // is anything to ask *with*.
      return status.baseFrame == null
          ? l10n.rotoNoStrokes
          : l10n.rotoReadyToPropagate(status.baseFrame!);
    case BridgeRotoStage.queued:
      return l10n.rotoQueued;
    case BridgeRotoStage.solving:
      // The copied count is the one number that makes the correction loop
      // legible: a re-run after a stroke is mostly copying, and saying so is
      // the difference between "it is doing it all again" and "it is not".
      return status.reused > 0
          ? l10n.rotoSolvingReusing(status.done, status.total, status.reused)
          : l10n.rotoSolving(status.done, status.total);
    case BridgeRotoStage.done:
    case BridgeRotoStage.cancelled:
      return _span(status);
    case BridgeRotoStage.failed:
      return status.failure == null ? '' : rotoFailureSentence(status.failure!);
  }
}

/// How far the matte reaches, and whether that is the whole shot.
///
/// A **cancelled** run says exactly the same thing as a finished one, because
/// it is the same kind of answer: the frames it got to are correct and are
/// kept, and the honest reading is how far it got — never "stopped", which
/// would suggest there was nothing to show.
String _span(BridgeRotoStatus status) {
  final first = status.firstFrame;
  final last = status.lastFrame;
  if (first == null || last == null) return l10n.rotoNoMatte;
  final covered = last - first + 1;
  return covered >= status.clipFrames
      ? l10n.rotoSpanWhole(covered)
      : l10n.rotoSpanPartial(first, last);
}
