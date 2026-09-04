// The Planar track effect's status, under its Analyse, Cancel and Create
// corner pin buttons (docs/08 §3.86).
//
// **In plain terms.** A planar track follows one flat thing in the shot — a
// phone screen, a sign, a poster — and works out where its four corners are on
// every frame. This is the line that says how that is getting on: how many
// frames have been followed, and, when it is done, how much of the clip carries
// the surface. When it stops part-way it says so, because how far it reaches is
// what decides what the user does next.
//
// **Read, not subscribed to**, and sampled only while it is moving — the Camera
// track's rule, for the same reason: the engine keeps the reading as a
// value and a stream would be a second mechanism for the same fact.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/track.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'camera_track_display_frb.dart';
import 'status_poller.dart';

/// The sentence for one reading of a planar track.
///
/// Pulled out of `build` so it can be asserted directly: what a status *says*
/// is a decision about wording, and testing it through a mounted widget would
/// be testing the mounting.
///
/// The failure sentences are the Camera track's own — the two effects share a
/// tracker, so they share its refusals, and a second set of words for "there
/// was nothing to follow" would be a second thing to translate.
String planarStatusSentence(BridgePlanarStatus? status) => switch (status?.stage) {
      null || BridgeTrackStage.idle => l10n.trackNotAnalysed,
      BridgeTrackStage.queued => l10n.trackWaiting,
      BridgeTrackStage.tracking =>
        l10n.trackFollowing(status!.done, status.total),
      BridgeTrackStage.solving => l10n.planarSolvingSurface,
      BridgeTrackStage.cancelled => l10n.trackStopped,
      BridgeTrackStage.failed => status!.failure == null
          ? l10n.trackFailedNoSolve
          : trackFailureSentence(status.failure!),
      // A partial track leads with its span, exactly as a partial camera solve
      // does: the rest of the clip has no surface and needs a second pass.
      BridgeTrackStage.done => status!.frames < status.clipFrames
          ? l10n.planarPartial(status.frames, status.clipFrames)
          : l10n.planarTracked(status.frames),
    };

/// The line under the Planar track's buttons.
class PlanarTrackDisplayFrb extends StatefulWidget {
  /// The layer the effect sits on — what a press is fired against.
  final LayerReference layer;

  /// Which instance on that layer: a planar track is filed under the effect,
  /// not the media, because what was tracked is the quad this instance holds.
  final UuidValue effectId;

  /// Something changed that the rest of the interface should re-read.
  final VoidCallback onChanged;

  /// Bumped by the panel every time one of the effect's Action buttons is
  /// pressed. A press changes nothing in the document — there is no revision to
  /// compare and no event to subscribe to — so the panel says so with a number.
  final int pressed;

  /// Where the reading comes from. The engine's own answer by default; a test
  /// hands one in, which is the seam `ViewerTrackLayer.fetch` already is one
  /// level up (docs/impl/tracking.md §5c) — a planar track cannot be produced
  /// from Dart, so what this side *does* with one is asserted by handing one
  /// over rather than by mounting an engine that could not have one.
  final BridgePlanarStatus Function()? fetch;

  const PlanarTrackDisplayFrb({
    super.key,
    required this.layer,
    required this.effectId,
    required this.onChanged,
    required this.pressed,
    this.fetch,
  });

  @override
  State<PlanarTrackDisplayFrb> createState() => _PlanarTrackDisplayFrbState();
}

class _PlanarTrackDisplayFrbState extends State<PlanarTrackDisplayFrb>
    with StatusPoller<BridgePlanarStatus, PlanarTrackDisplayFrb> {
  @override
  BridgePlanarStatus fetchStatus() =>
      widget.fetch?.call() ??
      planarStatus(layer: widget.layer, effect: widget.effectId);

  @override
  VoidCallback get onChanged => widget.onChanged;

  @override
  bool shouldResample(PlanarTrackDisplayFrb old) =>
      old.pressed != widget.pressed;

  @override
  bool isMoving(BridgePlanarStatus? status) => switch (status?.stage) {
        BridgeTrackStage.queued ||
        BridgeTrackStage.tracking ||
        BridgeTrackStage.solving =>
          true,
        _ => false,
      };

  // Reaching **done**, exactly as the Camera track counts it: a track that
  // stopped or failed left no surface for anything else to redraw against.
  @override
  bool hasLanded(BridgePlanarStatus? was, BridgePlanarStatus next) =>
      was?.stage != BridgeTrackStage.done &&
      next.stage == BridgeTrackStage.done;

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
          // The same bar the Camera track draws, measuring the same thing: how
          // far the answer reaches, once the work is over.
          if (status != null && status.clipFrames > 0)
            TrackSpanBar(
              key: const ValueKey('fx-planar-track-span'),
              analysed: status.frames,
              total: status.clipFrames,
            ),
          Text(
            planarStatusSentence(status),
            key: const ValueKey('fx-planar-track-status'),
            style: t.small.copyWith(color: t.textMuted),
            overflow: TextOverflow.ellipsis,
          ),
          // A re-anchored track carries a little accumulated error at its far
          // end, and nothing else on screen would say so. One line, only when
          // there is something to say.
          if (status != null &&
              status.stage == BridgeTrackStage.done &&
              status.reanchors > 0)
            Text(
              l10n.planarReanchored(status.reanchors),
              key: const ValueKey('fx-planar-track-reanchors'),
              style: t.small.copyWith(color: t.textMuted),
              overflow: TextOverflow.ellipsis,
            ),
        ],
      ),
    );
  }
}
