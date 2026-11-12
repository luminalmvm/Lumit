// The Stabilise effect's status, above its Analyse, Cancel and Use this frame
// buttons.
//
// The analysis runs on its own thread, over the layer, while you carry on
// editing. This line says how it is getting on: how many frames have been
// followed, that the steady path is being worked out, and, once it is done,
// how far the picture had to be scaled to hide its edges. In Reverse it says
// which layer's shake it puts back. When it cannot be done it says why,
// calmly, with the trackers' own words.
//
// **Read, not subscribed to**, and sampled only while it is moving: the
// Camera track's rule, through the same poller. What it reads also greys
// Analyse while an analysis runs and Cancel while none does.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/stabilise.dart';
import 'package:lumit_flutter/src/rust/api/track.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'camera_track_display_frb.dart';
import 'effect_param_row_frb.dart' show actionsResting;
import 'fx_key_strip.dart';
import 'fx_section.dart';
import 'status_poller.dart';

/// [scale] as a percentage with one decimal at most: 1.093 is "109.3", 1.5
/// is "150".
String stabiliseScalePercent(double scale) {
  final text = (scale * 100).toStringAsFixed(1);
  return text.endsWith('.0') ? text.substring(0, text.length - 2) : text;
}

/// Whether a finished analysis stopped short of the frames it was asked for.
bool _partial(BridgeStabiliseStatus status) =>
    status.frames < status.clipFrames;

/// Whether the line reads as a warning: hiding the edges needs more scale than
/// the Maximum scale row allows.
bool stabiliseStatusWarns(BridgeStabiliseStatus? status) =>
    status != null &&
    status.stage == BridgeTrackStage.done &&
    !_partial(status) &&
    status.short;

/// The sentence for one reading of a Stabilise. [reverse] is the Mode row.
///
/// Pulled out of `build` so it can be asserted directly, as the Camera track's
/// is. The failure sentences are the trackers' own: one tracker, one set of
/// refusals to translate.
String stabiliseStatusSentence(
  BridgeStabiliseStatus? status, {
  required bool reverse,
}) {
  if (reverse) {
    final layer = status?.reversing;
    if (layer == null) return l10n.stabiliseNothingToFollow;
    // The stage is the followed layer's own: its refusals read as they do on
    // that layer's line, after its name.
    final failed = l10n.stabiliseFollowedFailed;
    return switch (status!.stage) {
      BridgeTrackStage.done => l10n.stabiliseFollows(layer),
      BridgeTrackStage.queued ||
      BridgeTrackStage.tracking ||
      BridgeTrackStage.solving =>
        l10n.stabiliseWaitingFor(layer),
      BridgeTrackStage.idle => failed(layer, l10n.trackNotAnalysed),
      BridgeTrackStage.cancelled => failed(layer, l10n.trackStopped),
      BridgeTrackStage.failed => failed(
          layer,
          status.failure == null
              ? l10n.trackFailedNoFeatures
              : trackFailureSentence(status.failure!)),
    };
  }
  if (status != null && !status.analysable) return l10n.stabiliseNeedsFootage;
  return switch (status?.stage) {
    null || BridgeTrackStage.idle => l10n.trackNotAnalysed,
    BridgeTrackStage.queued => l10n.trackWaiting,
    BridgeTrackStage.tracking =>
      l10n.stabiliseAnalysing(status!.done, status.total),
    BridgeTrackStage.solving => l10n.stabiliseSolving,
    BridgeTrackStage.cancelled => l10n.trackStopped,
    BridgeTrackStage.failed => status!.failure == null
        ? l10n.trackFailedNoFeatures
        : trackFailureSentence(status.failure!),
    // A partial analysis leads with its span, as a partial camera solve does:
    // the frames after it keep the last correction.
    // A row that changes what is analysed moved, and the analysis it asked
    // for was stopped or failed: the picture is still drawn from the old one.
    BridgeTrackStage.done when status!.stale => l10n.stabiliseStale,
    // Still stabilising until the scale for the rows now is known, so no
    // scale is claimed before it is.
    BridgeTrackStage.done => _partial(status!)
        ? l10n.stabilisePartial(status.frames, status.clipFrames)
        : !status.scaleReady
            ? l10n.stabiliseSolving
            : status.short
            ? l10n.stabiliseShort(stabiliseScalePercent(status.scale))
            : stabiliseScalePercent(status.scale) == '100'
                ? l10n.stabiliseDone
                : l10n.stabiliseScaled(stabiliseScalePercent(status.scale)),
  };
}

/// Whether a reading is still moving: an analysis queued or running, or a
/// Reverse waiting for the layer it follows.
bool stabiliseIsMoving(BridgeStabiliseStatus? status) =>
    switch (status?.stage) {
      BridgeTrackStage.queued ||
      BridgeTrackStage.tracking ||
      BridgeTrackStage.solving =>
        true,
      _ => false,
    };

/// What each status line on screen has greyed, by effect. Two panels can show
/// one effect, and a line mounted again in a frame overlaps its old self, so
/// the rows stay greyed while any line holds them.
final Map<UuidValue, Map<Object, Set<String>>> _restingBy = {};

/// Record what [holder] greys on [effect], null for nothing, and tell the rows.
void _holdResting(UuidValue effect, Object holder, Set<String>? rest) {
  final holders = _restingBy[effect] ?? {};
  if (rest == null) {
    holders.remove(holder);
  } else {
    holders[holder] = rest;
  }
  if (holders.isEmpty) {
    _restingBy.remove(effect);
  } else {
    _restingBy[effect] = holders;
  }
  final now = holders.isEmpty ? null : holders.values.last;
  if (setEquals(actionsResting.value[effect], now)) return;
  final next = {...actionsResting.value};
  if (now == null) {
    next.remove(effect);
  } else {
    next[effect] = now;
  }
  actionsResting.value = next;
}

/// The line above Stabilise's buttons.
class StabiliseDisplayFrb extends StatefulWidget {
  /// The layer the effect sits on.
  final LayerReference layer;

  /// Which instance on that layer: an analysis is filed under the effect.
  final UuidValue effectId;

  /// The Mode row is Reverse.
  final bool reverse;

  /// Something changed that the rest of the interface should re-read.
  final VoidCallback onChanged;

  /// Bumped by the panel every time one of the effect's Action buttons is
  /// pressed. A press moves no revision, so the panel says so with a number.
  final int pressed;

  const StabiliseDisplayFrb({
    super.key,
    required this.layer,
    required this.effectId,
    required this.reverse,
    required this.onChanged,
    required this.pressed,
  });

  @override
  State<StabiliseDisplayFrb> createState() => _StabiliseDisplayFrbState();
}

class _StabiliseDisplayFrbState extends State<StabiliseDisplayFrb>
    with StatusPoller<BridgeStabiliseStatus, StabiliseDisplayFrb> {
  @override
  VoidCallback get onChanged => widget.onChanged;

  // Never read inside a build: a read that finds an analysis landed tells
  // the rest of the interface, which is being built too.
  @override
  bool shouldResample(StabiliseDisplayFrb old) => false;

  /// The document revision the line was last read at.
  int? _readAt;

  // Read once as the line mounts, so its first frame says what is happening:
  // without it a freshly added effect would read as not analysed until the
  // first read after the frame. Only the reading is kept here; what a landing
  // tells the rest of the interface waits for that read.
  @override
  void initState() {
    super.initState();
    try {
      status = fetchStatus();
    } catch (_) {
      // The layer went away; the read after the frame finds the same.
    }
  }

  // A row that changes what is followed, a Mode change or a dropped point is
  // an edit, and a press moves the press count: either is read again after
  // the frame, as the Planar track's line reads an edit.
  @override
  void didUpdateWidget(StabiliseDisplayFrb oldWidget) {
    super.didUpdateWidget(oldWidget);
    final now =
        Provider.of<LumitUiState>(context, listen: false).model.heldRevision;
    if (now == _readAt && oldWidget.pressed == widget.pressed) return;
    _readAt = now;
    WidgetsBinding.instance.addPostFrameCallback((_) => sample());
  }

  /// Reads in a row that found a fresh effect idle on footage.
  int _idleReads = 0;

  bool _idleOnFootage(BridgeStabiliseStatus? status) =>
      !widget.reverse &&
      status != null &&
      status.analysable &&
      status.stage == BridgeTrackStage.idle;

  @override
  BridgeStabiliseStatus fetchStatus() {
    final next = stabiliseStatus(layer: widget.layer, effect: widget.effectId);
    _idleReads = _idleOnFootage(next) ? _idleReads + 1 : 0;
    return next;
  }

  // A fresh effect's analysis is asked for by a read, and the first read can
  // come before its footage can be asked for. Idle on footage keeps reading
  // for about ten seconds at the poller's pace, so the line sees the analysis
  // start. A finished analysis whose scale is still being worked out keeps
  // reading until it is.
  @override
  bool isMoving(BridgeStabiliseStatus? status) =>
      stabiliseIsMoving(status) ||
      (status?.stage == BridgeTrackStage.done && !status!.scaleReady) ||
      (_idleOnFootage(status) && _idleReads <= 20);

  // Reaching **done** is what changes the frames, as the Camera track counts it.
  @override
  bool hasLanded(BridgeStabiliseStatus? was, BridgeStabiliseStatus next) =>
      was?.stage != BridgeTrackStage.done &&
      next.stage == BridgeTrackStage.done;

  // A press here can start an analysis the Viewer's progress slot has not
  // seen yet, since a press moves no revision, and while one runs the slot
  // reads when the line does, so the two agree. Tell it.
  @override
  void sample() {
    super.sample();
    if (!mounted) return;
    final progress =
        Provider.of<LumitUiState>(context, listen: false).stabiliseProgress;
    if (!widget.reverse && (isMoving(status) || progress.visible)) {
      progress.wake();
    }
    _sayResting();
  }

  /// Tell the Analyse and Cancel rows which of them has nothing to do: Analyse
  /// while an analysis is queued or running, Cancel otherwise, and both on a
  /// layer with nothing to analyse. In Reverse the schema has greyed both
  /// already.
  void _sayResting() {
    final status = this.status;
    _holdResting(
      widget.effectId,
      this,
      widget.reverse || status == null
          ? null
          : !status.analysable
              ? const {'analyse', 'cancel'}
              : stabiliseIsMoving(status)
                  ? const {'analyse'}
                  : const {'cancel'},
    );
  }

  @override
  void dispose() {
    final effect = widget.effectId;
    // After the frame: the rows cannot be rebuilt while the tree is torn down.
    WidgetsBinding.instance
        .addPostFrameCallback((_) => _holdResting(effect, this, null));
    super.dispose();
  }

  void _makePrecomp() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    var name = '';
    for (final entry in ui.model.heldLayers) {
      if (entry.layer.internallayerId == widget.layer.internallayerId) {
        name = entry.info.name;
      }
    }
    final LayerReference precomp;
    try {
      precomp = makeStabilisedPrecomp(
        layer: widget.layer,
        effect: widget.effectId,
        name: l10n.stabilisePrecompName(name),
      );
    } catch (_) {
      // The layer or its analysis went away; nothing was made.
      return;
    }
    // The Precomp layer is what the user is now working on, as Precompose
    // leaves it.
    ui.setSelection([precomp]);
    widget.onChanged();
  }

  void _bake() {
    try {
      stabiliseBakeKeyframes(layer: widget.layer, effect: widget.effectId);
    } catch (_) {
      // The layer or its analysis went away; nothing was written.
      return;
    }
    widget.onChanged();
  }

  void _restore() {
    try {
      stabiliseRestorePoints(layer: widget.layer, effect: widget.effectId);
    } catch (_) {
      // The layer went away; the count goes with the next read.
    }
    widget.onChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final status = this.status;
    final reverse = widget.reverse;
    final dropped = reverse ? 0 : status?.dropped ?? 0;
    final sentence = Text(
      stabiliseStatusSentence(status, reverse: reverse),
      key: const ValueKey('fx-stabilise-status'),
      style: t.small.copyWith(
        color: stabiliseStatusWarns(status) ? t.warning : t.textMuted,
        fontFeatures: const [FontFeature.tabularFigures()],
      ),
      overflow: TextOverflow.ellipsis,
    );
    // What the engine says can be done now: nothing on a switched-off or
    // Reverse Stabilise, before an analysis, or a precomp inside a precomp.
    final actions = [
      if (status?.canPrecomp ?? false)
        fxTextAction(
          context,
          label: l10n.stabiliseMakePrecomp,
          tip: l10n.tipStabiliseMakePrecomp,
          keyName: 'fx-stabilise-make-precomp',
          onPressed: _makePrecomp,
        ),
      if (status?.canConvert ?? false)
        fxTextAction(
          context,
          label: l10n.stabiliseConvertToKeyframes,
          tip: l10n.tipTrackConvert,
          keyName: 'fx-stabilise-bake',
          onPressed: _bake,
        ),
    ];
    // The keyframe lane's width is left free, as every row leaves it, so the
    // actions end where the values do.
    return Padding(
      padding: EdgeInsetsDirectional.only(
          top: 4, bottom: 4, end: FxKeyStripScope.widthOf(context)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          // Its room is kept wherever an analysis can run, empty until one
          // has a span, so the rows below never jump when it starts.
          if (!reverse && (status?.analysable ?? true))
            TrackSpanBar(
              key: const ValueKey('fx-stabilise-span'),
              analysed: status?.frames ?? 0,
              total: status?.clipFrames ?? 0,
            ),
          // The Camera track's line where it fits: the sentence at the left,
          // the actions at the right. Narrower, the actions go under the
          // sentence from the same edge, and narrower still they stack.
          if (actions.isEmpty)
            sentence
          else
            OverflowBar(
              spacing: 12,
              alignment: MainAxisAlignment.spaceBetween,
              overflowAlignment: OverflowBarAlignment.start,
              children: [
                sentence,
                OverflowBar(spacing: 6, children: actions),
              ],
            ),
          if (dropped > 0)
            Row(
              children: [
                Expanded(
                  child: Text(
                    l10n.stabiliseDropped(dropped),
                    key: const ValueKey('fx-stabilise-dropped'),
                    style: t.small.copyWith(color: t.textMuted),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                fxTextAction(
                  context,
                  label: l10n.stabiliseRestorePoints,
                  tip: l10n.tipStabiliseRestorePoints,
                  keyName: 'fx-stabilise-restore',
                  onPressed: _restore,
                ),
              ],
            ),
        ],
      ),
    );
  }
}
