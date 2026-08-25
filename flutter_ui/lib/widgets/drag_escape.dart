// Escape abandons the drag in flight (docs/impl/timeline-interaction.md P3).
//
// In plain terms: every drag in this application stages its work in Dart and
// writes it once, when the button comes up — which is what makes a drag one
// undo step. That same arrangement gives the drag a way out: if nothing has
// been written yet, pressing `Escape` half way through can put everything back
// and write nothing at all, and the release that follows has nothing left to
// do. This little object is that way out, kept in one place so a drag gains it
// by holding one field rather than by copying a key handler.
//
// Used from a `State`: [begin] when the gesture starts, handing it the code
// that puts things back; [running] guards the update handler, because the
// pointer carries on moving after `Escape` and an abandoned drag must not
// follow it; [end] on release, which says whether there is anything to commit.
// [dispose] in the state's own `dispose`, so a widget torn down mid-drag does
// not leave a key handler behind.

import 'package:flutter/foundation.dart';

import 'escape_ladder.dart';

class DragEscape {
  /// What puts the gesture back where it started — held only while a drag is
  /// in flight, which is also what says whether this is listening.
  void Function()? _revert;

  /// How to stand down from the ladder, held while this is registered.
  VoidCallback? _release;

  /// Whether `Escape` has already taken this drag.
  bool _abandoned = false;

  /// Whether a drag is in flight and still live. False before one starts and
  /// false the instant `Escape` takes it, which is what an update handler
  /// checks before moving anything.
  bool get running => _revert != null && !_abandoned;

  /// A drag has begun. [revert] is called if `Escape` arrives before the
  /// release, and must leave the widget exactly as the drag found it —
  /// including any preview it published for others to read.
  void begin(void Function() revert) {
    _release ??= EscapeLadder.register(EscapeRung.gesture, _claim);
    _revert = revert;
    _abandoned = false;
  }

  /// The gesture is over. True when the release should commit; false when
  /// `Escape` already reverted it and there is nothing to write.
  bool end() {
    final commit = running;
    _stop();
    return commit;
  }

  void dispose() => _stop();

  void _stop() {
    if (_revert == null) return;
    _release?.call();
    _release = null;
    _revert = null;
    _abandoned = false;
  }

  /// The ladder's gesture rung (escape_ladder.dart). Taking the press stops it
  /// there — an `Escape` that abandoned a drag has done its work, and letting
  /// it travel on would clear a selection or shut a panel as well.
  bool _claim() {
    final revert = _revert;
    if (revert == null || _abandoned) return false;
    _abandoned = true;
    revert();
    return true;
  }
}
