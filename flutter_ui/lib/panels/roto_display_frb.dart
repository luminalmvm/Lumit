// The Roto brush's status, in words (K-713, docs/08 §3.96).
//
// **In plain terms.** Cutting a subject out of a whole shot takes a while, and
// it happens somewhere else — on its own thread, over the media file, while you
// carry on editing. These are the sentences that say how it is getting on: how
// many frames are done, how many of them were copied rather than worked out
// again, and how much of the shot the matte actually covers. When it cannot be
// done, it says why, calmly, and nothing about the shot has changed.
//
// **The words are separable from the row.** [rotoStatusSentence] and
// [rotoFailureSentence] are free functions so what a status *says* can be
// asserted directly: that is a decision about wording, and testing it through a
// mounted widget would be testing the mounting. [RotoDisplayFrb] is the row
// itself (K-717) — the span bar, the sentence, and the base frame with the one
// button that moves it. The buttons above it are the effect's own Action rows
// (`ParamKind::Action`), drawn by the ordinary parameter row; the scribbling is
// the Viewer's (`panels/viewer_roto.dart`).

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/roto.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'camera_track_display_frb.dart' show TrackSpanBar;
import 'status_poller.dart';

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

/// How many source frames the matte covers — the accent half of the span bar.
///
/// Zero before anything is propagated, which draws the bar entirely in the
/// surface tone: an honest "none of this shot is cut yet" rather than no bar at
/// all.
int rotoCoveredFrames(BridgeRotoStatus status) {
  final first = status.firstFrame;
  final last = status.lastFrame;
  if (first == null || last == null) return 0;
  return (last - first + 1).clamp(0, status.clipFrames);
}

/// The line under the Roto brush's Propagate and Cancel buttons: how far the
/// matte reaches, how the propagation is getting on, and which frame it is
/// working outward from.
class RotoDisplayFrb extends StatefulWidget {
  /// The layer the effect sits on — what the base frame is written through.
  final LayerReference layer;

  /// Which instance on that layer: a matte is filed under the effect, because
  /// what was cut out is the subject this instance's strokes describe.
  final UuidValue effectId;

  /// The composition frame on screen — what "assign the base to here" means.
  /// Passed rather than read: the panel is rebuilt when it moves, and asking
  /// the engine per rebuild is exactly the traffic K-681 forbids.
  final int playheadFrame;

  /// Something changed that the rest of the interface should re-read.
  final VoidCallback onChanged;

  /// Bumped by the panel every time one of the effect's Action buttons is
  /// pressed. A press changes nothing in the document — there is no revision to
  /// compare and no event to subscribe to — so the panel says so with a number.
  final int pressed;

  /// Where the reading comes from. The engine's own answer by default; a test
  /// hands one in, which is the seam the two tracking displays already are — a
  /// propagation cannot be produced from Dart, so what this side *does* with one
  /// is asserted by handing one over.
  final BridgeRotoStatus Function()? fetch;

  const RotoDisplayFrb({
    super.key,
    required this.layer,
    required this.effectId,
    required this.playheadFrame,
    required this.onChanged,
    required this.pressed,
    this.fetch,
  });

  @override
  State<RotoDisplayFrb> createState() => _RotoDisplayFrbState();
}

class _RotoDisplayFrbState extends State<RotoDisplayFrb>
    with StatusPoller<BridgeRotoStatus, RotoDisplayFrb> {
  @override
  BridgeRotoStatus fetchStatus() =>
      widget.fetch?.call() ??
      rotoStatus(layer: widget.layer, effect: widget.effectId);

  @override
  VoidCallback get onChanged => widget.onChanged;

  // A press, and the card being pointed at another instance: both change what
  // this says, and neither is a tick of the clock.
  @override
  bool shouldResample(RotoDisplayFrb old) =>
      old.pressed != widget.pressed || old.effectId != widget.effectId;

  @override
  bool isMoving(BridgeRotoStatus? status) => switch (status?.stage) {
        BridgeRotoStage.queued || BridgeRotoStage.solving => true,
        _ => false,
      };

  // A cancelled propagation lands like a finished one: the frames it reached
  // are kept, so the picture on screen has changed either way — which is why
  // this takes the default reading (it was moving; it has stopped) rather than
  // the tracks' *done*.

  /// Move the base frame to the frame on screen.
  ///
  /// A real edit and not a preference: every cached matte depends on the base,
  /// so moving it retires the run — which is exactly what "decide this shot from
  /// somewhere else" means. Through the ordinary whole-stack commit, so it is
  /// one undo step like every other effect edit.
  void _assignBase() {
    try {
      final source =
          rotoSourceFrame(layer: widget.layer, frame: widget.playheadFrame);
      final staged = widget.layer.getEffects();
      final instance =
          staged.where((e) => e.id() == widget.effectId).firstOrNull;
      if (instance == null) return;
      instance.rotoSetBaseFrame(frame: source);
      widget.layer.setEffects(effects: staged);
      widget.onChanged();
      sample();
    } catch (_) {
      // Not a footage layer, or its media will not probe: there is no source
      // frame to move the base to, and the line above already says so.
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final status = this.status;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          // The same bar the two tracking displays draw, measuring the same
          // thing in the same two weights: how much of the shot the answer
          // reaches, in the accent, against how much it does not (K-540).
          if (status != null && status.clipFrames > 0)
            TrackSpanBar(
              key: const ValueKey('fx-roto-span'),
              analysed: rotoCoveredFrames(status),
              total: status.clipFrames,
            ),
          Text(
            rotoStatusSentence(status),
            key: const ValueKey('fx-roto-status'),
            style: t.small.copyWith(color: t.textMuted),
            overflow: TextOverflow.ellipsis,
          ),
          // The frame the propagation runs outward from, and the one gesture
          // that moves it. Offered only once there is something to move: before
          // the first scribble the base is set *by* that scribble, and a button
          // that assigned a base to a brush with no strokes would aim a
          // propagation at nothing.
          if (status != null && status.strokes > 0)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      status.baseFrame == null
                          ? l10n.rotoNoBaseFrame
                          : l10n.rotoBaseFrame(status.baseFrame!),
                      key: const ValueKey('fx-roto-base'),
                      style: t.small.copyWith(color: t.textSecondary),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  HouseButton(
                    key: const ValueKey('fx-roto-assign-base'),
                    small: true,
                    frameless: true,
                    onPressed: _assignBase,
                    child: Text(l10n.rotoAssignBase, style: t.small),
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}
