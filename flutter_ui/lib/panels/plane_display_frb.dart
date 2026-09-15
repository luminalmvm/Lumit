// What a model read of a shot, in words (docs/08 §3.104 and §3.105).
//
// **In plain terms.** Reading a shot with a trained model takes a while, and it
// happens somewhere else, on its own thread, over the media file, while you
// carry on editing. These are the sentences that say how it is getting on: how
// many frames are read, how much of the shot the answer covers, and which
// graphics provider read it, because a run that fell back to the processor is a
// run the person waiting should be told about. When it cannot be done, it says
// why, calmly, and nothing about the shot has changed.
//
// **One card, two effects.** Depth and Remove background are the same job with
// a different answer at the end of it: the same buttons, the same progress, the
// same span, the same refusals. So they are the same card, and the only thing
// that differs is the word for a run that kept nothing, which is no depth or no
// matte.
//
// **The words are separable from the row.** [planeStatusSentence] and
// [planeFailureSentence] are free functions so what a status *says* can be
// asserted directly: that is a decision about wording, and testing it through a
// mounted widget would be testing the mounting. [PlaneDisplayFrb] is the row
// itself. The buttons above it are the effect's own Action rows
// (`ParamKind::Action`), drawn by the ordinary parameter row.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/planes.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'camera_track_display_frb.dart' show TrackSpanBar;
import 'status_poller.dart';

/// Which of the two effects on the planes tier a card is drawn for.
///
/// It picks two things and no more: the word for an answer that read nothing,
/// and the prefix on the card's keys, so two cards on one layer are told apart.
enum PlaneCard {
  /// The Depth effect: how far away every pixel is.
  depth,

  /// Remove background: how much of each pixel is the subject.
  matte;

  /// What a run that kept no frames leaves the card saying.
  String get _nothing => switch (this) {
        PlaneCard.depth => l10n.planeNoDepth,
        PlaneCard.matte => l10n.planeNoMatte,
      };

  /// The prefix every key on this card carries.
  String get _key => switch (this) {
        PlaneCard.depth => 'fx-depth',
        PlaneCard.matte => 'fx-matte',
      };
}

/// The sentence for a refusal.
///
/// The engine sends a **reason**, never these words: its own `PlaneFailure`
/// carries English, and English crossing the bridge would ship untranslated
/// inside a translated window. The switch is exhaustive over the generated
/// enum, so a reason added to the engine is a compile error here rather than a
/// blank line on screen.
///
/// A missing runtime and a missing pack are two sentences rather than one
/// because they send the user to two different buttons on the Addons page.
String planeFailureSentence(BridgePlaneFailure failure) => switch (failure) {
      BridgePlaneFailure.runtimeMissing => l10n.planeFailedRuntimeMissing,
      BridgePlaneFailure.packMissing => l10n.planeFailedPackMissing,
      BridgePlaneFailure.modelFailed => l10n.planeFailedModelFailed,
      BridgePlaneFailure.busy => l10n.planeFailedBusy,
      BridgePlaneFailure.unreadable => l10n.planeFailedUnreadable,
      BridgePlaneFailure.noFrames => l10n.planeFailedNoFrames,
      BridgePlaneFailure.cancelled => l10n.planeFailedCancelled,
    };

/// The sentence for one reading of the analysis.
///
/// Pulled out of any widget so it can be asserted directly: what a status
/// *says* is a decision about wording, and testing it through a mounted widget
/// would be testing the mounting.
String planeStatusSentence(PlaneCard card, BridgePlaneStatus? status) {
  if (status == null) return '';
  switch (status.stage) {
    case BridgePlaneStage.idle:
      return l10n.planeNotAnalysed;
    case BridgePlaneStage.queued:
      return l10n.planeQueued;
    case BridgePlaneStage.solving:
      return l10n.planeReading(status.done, status.total);
    case BridgePlaneStage.done:
    case BridgePlaneStage.cancelled:
      return _span(card, status);
    case BridgePlaneStage.failed:
      return status.failure == null
          ? ''
          : planeFailureSentence(status.failure!);
  }
}

/// How far the answer reaches, and whether that is the whole shot.
///
/// A **cancelled** run says exactly the same thing as a finished one, because
/// it is the same kind of answer: the frames it got to are correct and are
/// kept, and the honest reading is how far it got, never "stopped", which
/// would suggest there was nothing to show.
String _span(PlaneCard card, BridgePlaneStatus status) {
  final first = status.firstFrame;
  final last = status.lastFrame;
  if (first == null || last == null) return card._nothing;
  final covered = last - first + 1;
  return covered >= status.clipFrames
      ? l10n.planeSpanWhole(covered)
      : l10n.planeSpanPartial(first, last);
}

/// How many source frames the answer covers. The accent half of the span bar.
///
/// Zero before anything is analysed, which draws the bar entirely in the
/// surface tone: an honest "none of this shot is read yet" rather than no bar
/// at all.
int planeCoveredFrames(BridgePlaneStatus status) {
  final first = status.firstFrame;
  final last = status.lastFrame;
  if (first == null || last == null) return 0;
  return (last - first + 1).clamp(0, status.clipFrames);
}

/// The lines under a planes-tier effect's Analyse and Cancel buttons: how far
/// the answer reaches, how the analysis is getting on, and what read it.
class PlaneDisplayFrb extends StatefulWidget {
  /// Which effect this card is drawn for. See [PlaneCard].
  final PlaneCard card;

  /// The layer the effect sits on. What the reading is asked about.
  final LayerReference layer;

  /// Which instance on that layer: an answer is filed under the effect, because
  /// two of them on one clip with different models are two answers.
  final UuidValue effectId;

  /// Something changed that the rest of the interface should re-read.
  final VoidCallback onChanged;

  /// Bumped by the panel every time one of the effect's Action buttons is
  /// pressed. A press changes nothing in the document. There is no revision to
  /// compare and no event to subscribe to, so the panel says so with a number.
  final int pressed;

  /// Where the reading comes from. The engine's own answer by default; a test
  /// hands one in, which is the seam the tracking and Roto brush displays
  /// already are. An analysis cannot be produced from Dart, so what this side
  /// *does* with one is asserted by handing one over.
  final BridgePlaneStatus Function()? fetch;

  const PlaneDisplayFrb({
    super.key,
    required this.card,
    required this.layer,
    required this.effectId,
    required this.onChanged,
    required this.pressed,
    this.fetch,
  });

  @override
  State<PlaneDisplayFrb> createState() => _PlaneDisplayFrbState();
}

class _PlaneDisplayFrbState extends State<PlaneDisplayFrb>
    with StatusPoller<BridgePlaneStatus, PlaneDisplayFrb> {
  @override
  BridgePlaneStatus fetchStatus() =>
      widget.fetch?.call() ??
      planeStatus(layer: widget.layer, effect: widget.effectId);

  @override
  VoidCallback get onChanged => widget.onChanged;

  // A press, and the card being pointed at another instance: both change what
  // this says, and neither is a tick of the clock.
  @override
  bool shouldResample(PlaneDisplayFrb old) =>
      old.pressed != widget.pressed || old.effectId != widget.effectId;

  @override
  bool isMoving(BridgePlaneStatus? status) => switch (status?.stage) {
        BridgePlaneStage.queued || BridgePlaneStage.solving => true,
        _ => false,
      };

  // `hasLanded` is left at its default, which is that it was moving and has
  // stopped. An analysis landing changes what every frame of this comp is
  // *named* by, so the picture has to be asked for again, and a cancelled run
  // lands like a finished one: the frames it reached are kept and are drawn.

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final status = this.status;
    final key = widget.card._key;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          // The same bar the tracking and Roto brush displays draw, measuring
          // the same thing in the same two weights: how much of the shot the
          // answer reaches, in the accent, against how much it does not.
          if (status != null && status.clipFrames > 0)
            TrackSpanBar(
              key: ValueKey<String>('$key-span'),
              analysed: planeCoveredFrames(status),
              total: status.clipFrames,
            ),
          Text(
            planeStatusSentence(widget.card, status),
            key: ValueKey<String>('$key-status'),
            style: t.small.copyWith(color: t.textMuted),
            overflow: TextOverflow.ellipsis,
          ),
          // Which provider read the frames. A model that fell back from the
          // graphics card to the processor paints a slightly different picture
          // and takes a hundred times longer, so it is never left unsaid
          // (docs/impl/addons.md §8). Empty until something has been read.
          if (status != null && status.provider.isNotEmpty)
            Text(
              l10n.planeProvider(status.provider),
              key: ValueKey<String>('$key-provider'),
              style: t.small.copyWith(color: t.textMuted),
              overflow: TextOverflow.ellipsis,
            ),
        ],
      ),
    );
  }
}
