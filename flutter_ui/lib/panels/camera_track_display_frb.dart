// The Camera track effect's status, under its Analyse and Cancel buttons
// (K-417, docs/08 §3.85).
//
// **In plain terms.** Tracking a shot takes minutes, and it happens somewhere
// else — on its own thread, over the media file, while you carry on editing.
// This is the line that tells you how it is getting on: how many frames have
// been followed, that the camera is being solved, and — when it is done — how
// many points were found and how closely they agree, which is the one number
// that says whether the solve is any good. When it cannot be done, it says why,
// calmly, and nothing about the shot has changed.
//
// **Read, not subscribed to.** The engine keeps the reading as a value and this
// samples it, exactly as the cache bar samples the cache: there is no stream to
// hold and nothing to unsubscribe. The sampling stops the moment the answer
// stops moving.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/track.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'fx_section.dart';

/// How often the reading is sampled while an analysis is moving. Twice a second
/// is faster than anyone reads a number and slower than anything that could
/// cost: the call is a lookup in a map behind a mutex, and it is made only
/// while a Camera track's card is open **and** a job is actually in flight.
const Duration _poll = Duration(milliseconds: 500);

/// The sentence for a refusal.
///
/// The engine sends a **reason**, never these words (K-303): its own
/// `AnalysisError` carries English, and English crossing the bridge would ship
/// untranslated inside a translated window. The switch is exhaustive over the
/// generated enum, so a reason added to the engine is a compile error here
/// rather than a blank line on screen.
String trackFailureSentence(BridgeTrackFailure failure) => switch (failure) {
      BridgeTrackFailure.unreadable => l10n.trackFailedUnreadable,
      BridgeTrackFailure.noFrames => l10n.trackFailedNoFrames,
      BridgeTrackFailure.tracking => l10n.trackFailedFollowing,
      BridgeTrackFailure.noFeatures => l10n.trackFailedNoFeatures,
      BridgeTrackFailure.rotationOnly => l10n.trackFailedRotationOnly,
      BridgeTrackFailure.noSolve => l10n.trackFailedNoSolve,
    };

/// The sentence for one reading of the analysis.
///
/// Pulled out of `build` so it can be asserted directly: what a status *says*
/// is a decision about wording, and testing it through a mounted widget would
/// be testing the mounting.
String trackStatusSentence(BridgeTrackStatus? status) => switch (status?.stage) {
      null || BridgeTrackStage.idle => l10n.trackNotAnalysed,
      BridgeTrackStage.queued => l10n.trackWaiting,
      BridgeTrackStage.tracking =>
        l10n.trackFollowing(status!.done, status.total),
      BridgeTrackStage.solving => l10n.trackSolvingCamera,
      BridgeTrackStage.cancelled => l10n.trackStopped,
      BridgeTrackStage.failed => status!.failure == null
          ? l10n.trackFailedNoSolve
          : trackFailureSentence(status.failure!),
      // A **partial** track says how far it got instead of how good it is. The
      // span is the fact that changes what the user does next — the rest of the
      // shot has no camera and needs a second pass or a different approach —
      // and the bar above the line is already showing it.
      BridgeTrackStage.done => status!.frames < status.clipFrames
          ? l10n.trackSolvedPartial(status.frames, status.clipFrames)
          : l10n.trackSolvedSummary(
              status.points,
              status.meanError.toStringAsFixed(2),
            ),
    };

/// How much of the clip carries a solved camera, as one thin bar.
///
/// **In plain terms.** A track can stop part-way: the lens racks, the frame
/// whites out, the specks stop crossing from one frame to the next. When that
/// happens the analysis stops there rather than inventing the rest, and this is
/// how far it got — the solved span at one end, the part with no camera at the
/// other. A whole track fills the bar, which is how a partial one is legible at
/// a glance without reading anything.
///
/// **Two weights in a row rather than a painter.** The bar is a ratio of two
/// integers and nothing else — no ticks, no labels, no hit testing — so the
/// layout does the arithmetic and there is no paint code to keep in step with
/// the theme.
class TrackSpanBar extends StatelessWidget {
  /// Frames of the clip that carry a solved camera. Always a prefix: the
  /// analysis follows the source from its first frame and can only stop early.
  final int analysed;

  /// Frames the clip has.
  final int total;

  const TrackSpanBar({
    super.key,
    required this.analysed,
    required this.total,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final done = analysed.clamp(0, total);
    final rest = total - done;
    return Padding(
      padding: const EdgeInsets.only(bottom: 5),
      child: SizedBox(
        height: 3,
        child: Row(
          children: [
            if (done > 0)
              Expanded(flex: done, child: ColoredBox(color: t.accent)),
            if (rest > 0)
              Expanded(flex: rest, child: ColoredBox(color: t.surface3)),
          ],
        ),
      ),
    );
  }
}

/// The line under the Camera track's buttons.
class CameraTrackDisplayFrb extends StatefulWidget {
  /// The layer the effect sits on — what a press is fired against and what the
  /// reading is asked about.
  final LayerReference layer;

  /// Something changed that the rest of the interface should re-read: a solve
  /// landing renames the frames it changes, so the picture is redrawn with the
  /// camera it was solved with.
  final VoidCallback onChanged;

  /// Bumped by the panel every time one of the effect's Action buttons is
  /// pressed. It is the *press* this line has to notice, and a press changes
  /// nothing in the document — there is no revision to compare and no event to
  /// subscribe to, so the panel says so with a number.
  final int pressed;

  const CameraTrackDisplayFrb({
    super.key,
    required this.layer,
    required this.onChanged,
    required this.pressed,
  });

  @override
  State<CameraTrackDisplayFrb> createState() => _CameraTrackDisplayFrbState();
}

class _CameraTrackDisplayFrbState extends State<CameraTrackDisplayFrb> {
  BridgeTrackStatus? _status;
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    // The first reading after the frame is up, never from `build`.
    WidgetsBinding.instance.addPostFrameCallback((_) => _sample());
  }

  @override
  void didUpdateWidget(CameraTrackDisplayFrb old) {
    super.didUpdateWidget(old);
    // A button was pressed: read once, which starts the sampling if the press
    // started something.
    if (old.pressed != widget.pressed) _sample();
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  /// Whether the reading is still moving, and so worth asking about again.
  static bool _moving(BridgeTrackStatus? status) => switch (status?.stage) {
        BridgeTrackStage.queued ||
        BridgeTrackStage.tracking ||
        BridgeTrackStage.solving =>
          true,
        _ => false,
      };

  void _sample() {
    if (!mounted) return;
    final BridgeTrackStatus next;
    try {
      next = trackStatus(layer: widget.layer);
    } catch (_) {
      // The layer went away under the card; the line simply stops moving.
      _timer?.cancel();
      _timer = null;
      return;
    }
    final was = _status;
    if (next != was) setState(() => _status = next);
    // A solve landing changes what every frame of this comp is *named* by, so
    // the picture has to be asked for again — otherwise the frames banked
    // before it would be served back after it.
    if (was?.stage != BridgeTrackStage.done &&
        next.stage == BridgeTrackStage.done) {
      widget.onChanged();
      // Re-reading is not enough on its own (K-430). A solve moves neither the
      // playhead nor the document's revision, so the picture would still be
      // the one banked before it, and the Viewer's point cloud — keyed by
      // exactly those two — would have no reason to ask the engine again. Both
      // are told here, at the one place that knows a solve has landed.
      final ui = Provider.of<LumitUiState>(context, listen: false);
      ui.solveLanded.value++;
      ui.requestFrame();
    }
    if (_moving(next)) {
      _timer ??= Timer.periodic(_poll, (_) => _sample());
    } else {
      _timer?.cancel();
      _timer = null;
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final status = _status;
    final line = trackStatusSentence(status);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          // Above the line, because it is the thing the line is about: there
          // is nothing to say about a span until there is a span.
          if (status != null && status.clipFrames > 0)
            TrackSpanBar(
              key: const ValueKey('fx-camera-track-span'),
              analysed: status.frames,
              total: status.clipFrames,
            ),
          Row(
            children: [
          Expanded(
            child: Text(
              line,
              key: const ValueKey('fx-camera-track-status'),
              style: t.small.copyWith(color: t.textMuted),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          // The one gesture a finished solve offers here: a Camera layer that
          // follows it. Nothing is copied — the camera holds a *link*, so
          // re-analysing the shot moves it too.
          if (status?.stage == BridgeTrackStage.done)
            fxTextAction(
                  context,
                  label: l10n.trackCreateCamera,
                  tip: l10n.tipTrackCreateCamera,
                  keyName: 'fx-camera-track-create-camera',
                  onPressed: () {
                    try {
                      addSolvedCamera(tracked: widget.layer);
                    } catch (_) {
                      // The layer went away; nothing to add it beside.
                    }
                    widget.onChanged();
                  },
                ),
            ],
          ),
        ],
      ),
    );
  }
}

/// A Camera layer's solve-link badge, and the one command that ends the link
/// (K-417).
///
/// **In plain terms.** A camera that follows a tracked shot does not hold its
/// own numbers: they are worked out per frame from the solve. So its position
/// and rotation cannot be dragged, and this says so rather than leaving a row
/// that silently refuses. It also says when the link has run past what was
/// solved (the motion is being held) and when it cannot be followed at all.
///
/// **Convert to keyframes** bakes the derived motion into one key per frame and
/// severs the link. From then on it is an ordinary camera: the keys are real,
/// editable, and the graph editor shows them like any others.
class CameraLinkBadge extends StatefulWidget {
  final LayerReference camera;

  /// The frame the badge speaks about: whether the link is derived or held is
  /// a property of *when*, not of the layer.
  final int playheadFrame;
  final VoidCallback onChanged;

  const CameraLinkBadge({
    super.key,
    required this.camera,
    required this.playheadFrame,
    required this.onChanged,
  });

  @override
  State<CameraLinkBadge> createState() => _CameraLinkBadgeState();
}

class _CameraLinkBadgeState extends State<CameraLinkBadge> {
  BridgeCameraLink? _link;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _read());
  }

  @override
  void didUpdateWidget(CameraLinkBadge old) {
    super.didUpdateWidget(old);
    // Once per frame change, never per rebuild — the Levels histogram's rule
    // (K-413), for the same reason: the answer only moves when the playhead or
    // the document does, and the bridge-call budget is the gate.
    if (old.playheadFrame != widget.playheadFrame ||
        old.camera.internallayerId != widget.camera.internallayerId) {
      _read();
    }
  }

  void _read() {
    if (!mounted) return;
    final BridgeCameraLink next;
    try {
      next = cameraLink(camera: widget.camera, frame: widget.playheadFrame);
    } catch (_) {
      return;
    }
    if (next != _link) setState(() => _link = next);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final link = _link;
    if (link == null || link.state == BridgeLinkState.unlinked) {
      return const SizedBox.shrink();
    }
    final (text, colour) = switch (link.state) {
      BridgeLinkState.derived => (l10n.trackLinkFollowing, t.textMuted),
      BridgeLinkState.held => (l10n.trackLinkHolding, t.textMuted),
      // Not an alarm, and not red: the camera is still drawing, on the numbers
      // it had when the link was made. It is a state the user has to be told
      // about, which is what the accent is for.
      BridgeLinkState.unresolved => (l10n.trackLinkLost, t.accent),
      BridgeLinkState.unlinked => ('', t.textMuted),
    };
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          text,
          key: const ValueKey('tf-camera-link-badge'),
          style: t.small.copyWith(color: colour),
        ),
        const SizedBox(width: 6),
        fxTextAction(
          context,
          label: l10n.trackConvertToKeyframes,
          tip: l10n.tipTrackConvert,
          keyName: 'tf-camera-link-convert',
          onPressed: () {
            try {
              convertCameraToKeyframes(camera: widget.camera);
            } catch (_) {
              // No solve to bake — the link resolves nowhere. The badge
              // already says so.
            }
            _read();
            widget.onChanged();
          },
        ),
      ],
    );
  }
}
