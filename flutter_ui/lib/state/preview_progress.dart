// How far the frame the Viewer is waiting for has got, and whether that is
// worth drawing (docs/07 §2.5, docs/13 §7.1).
//
// **In plain terms.** Most frames arrive too quickly to be worth mentioning:
// the picture simply changes. Some do not — a heavy comp under a dragged value,
// a scrub onto a frame nothing has cached — and until now those were silent.
// The picture sat on the previous frame with nothing to say whether the engine
// was working or had given up.
//
// So the engine reports how far each waited-on frame has got, and this follows
// those reports and answers one question: is there something to draw, and how
// far along is it? Two rules keep it quiet:
//
// - **Never during playback.** The engine sends no reports then at all — a
//   frame due in sixteen milliseconds has neither the need for a bar nor the
//   time to describe itself — and [PreviewProgressTracker.stop] clears anything
//   left over when playback starts.
// - **Never for a quick frame.** Nothing is shown until a render has been
//   outstanding for [PreviewProgressTracker.appearsAfter]. A bar that flashed
//   up for every frame of a drag would be noise, and a frame that beat the
//   delay never needed one.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../l10n/strings.dart';
import '../src/rust/api/state.dart';

/// What the engine is doing for the frame being waited on. The codes are the
/// engine's own (`lumit_render::RenderStage::code`) and are fixed: a reordered
/// enum must not silently relabel anything.
String previewStageLabel(int stage) => switch (stage) {
      0 => l10n.previewPreparing,
      1 => l10n.previewReadingMedia,
      2 => l10n.previewReadingComposition,
      3 => l10n.previewCompositing,
      4 => l10n.previewShowing,
      _ => l10n.previewRendering,
    };

/// Follows the engine's progress reports for the frame the Viewer is waiting
/// on, and decides what — if anything — the Viewer should draw.
class PreviewProgressTracker extends ChangeNotifier {
  /// How long a render must have been outstanding before a bar appears. Long
  /// enough that an ordinary frame never shows one, short enough that a frame
  /// worth waiting for says so before the wait feels like a fault.
  static const Duration appearsAfter = Duration(milliseconds: 150);

  /// The frame being waited on, or null when nothing is.
  int? _frame;
  int _stage = 0;
  double _fraction = 0;
  bool _outstanding = false;
  bool _visible = false;
  Timer? _timer;

  /// True when a bar should be on screen.
  bool get visible => _visible;

  /// True when nothing is being waited on: no render outstanding, and so no
  /// timer pending to decide whether a bar should appear.
  ///
  /// Distinct from [`visible`], which is false both before a slow frame's bar
  /// appears and after any frame finishes. A test that has asked for a render
  /// waits on *this* rather than on a round count, so a frame that takes longer
  /// on one machine — or under the load of a whole suite — than another does
  /// not decide whether the test passes.
  bool get idle => !_outstanding;

  /// How far the frame has got, 0..1 — the engine's own estimate.
  double get fraction => _fraction;

  /// Which frame is being waited for, or null when nothing is.
  int? get frame => _frame;

  /// What the engine is doing, in words.
  String get label => previewStageLabel(_stage);

  /// One report from the engine.
  void report(BridgeRenderProgress p) {
    if (p.done) {
      _finish();
      return;
    }
    if (!_outstanding) {
      _outstanding = true;
      _timer?.cancel();
      _timer = Timer(appearsAfter, _appear);
    }
    _frame = p.frame.toInt();
    _stage = p.stage;
    _fraction = p.fraction.clamp(0.0, 1.0);
    // While nothing is on screen there is nothing to repaint: the reports are
    // still followed, they simply cost a field each until the bar appears.
    if (_visible) notifyListeners();
  }

  /// Playback started, the Viewer went away, or the project was swapped —
  /// whatever was being waited on is no longer being waited on here.
  void stop() {
    _timer?.cancel();
    _timer = null;
    _outstanding = false;
    _frame = null;
    _fraction = 0;
    if (_visible) {
      _visible = false;
      notifyListeners();
    }
  }

  void _appear() {
    _timer = null;
    // The render finished inside the delay: the picture is already there and a
    // bar would be a flash of nothing.
    if (!_outstanding) return;
    _visible = true;
    notifyListeners();
  }

  void _finish() {
    _timer?.cancel();
    _timer = null;
    _outstanding = false;
    _fraction = 1.0;
    if (_visible) {
      _visible = false;
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }
}
