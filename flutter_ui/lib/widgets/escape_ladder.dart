// One Escape, one step back: the ladder every surface registers on (K-575).
//
// **In plain terms.** Escape means "take that back", and in an application this
// dense several things could plausibly be taken back at once — a drag in
// flight, an open menu, a dialogue, a selection. Whoever answers has to be the
// innermost of those, and only that one.
//
// Flutter will not arrange this by itself. Every handler added to
// `HardwareKeyboard.instance` runs on every key press *regardless of what the
// ones before it returned* — its own documentation says so — and the focus and
// shortcut path runs afterwards as well. So the old arrangement, where each
// surface added a handler and returned true to mean "mine", was not an order at
// all: `Escape` mid-drag also shut the menu behind it, and the dialogue behind
// that.
//
// This is the arbiter that replaces the race. One handler on the keyboard, a
// fixed order of rungs, and each surface registering a *claim* on the rung it
// belongs to; the first claim that says it took the press wins and no other is
// asked. The order is the spec's (docs/07-UI-SPEC.md §14.1), and the rung list
// below is the only place it exists in code.

import 'package:flutter/services.dart';

/// The rungs, in the order Escape climbs them: earlier wins.
enum EscapeRung {
  /// A gesture in flight — a drag, a pick, a path being drawn, a chord being
  /// captured. Nothing was written yet, so cancelling writes nothing.
  gesture,

  /// The open menu, dropdown or flyout chain (`closeLumitPopups`).
  popup,

  /// A dialogue or a surface that has taken the window: a modal, the FX
  /// console, the command palette, the welcome screen.
  dialog,

  /// The finest selection held on screen.
  selection,
}

/// Where the surfaces register. Static because Escape is one key and there is
/// one keyboard: an instance per screen would be the race again with extra
/// steps.
///
/// A focused text editor is deliberately not on this ladder: it answers Escape
/// on its own focus node (K-323), which Flutter runs after this, and it is the
/// last thing to get a look in.
abstract final class EscapeLadder {
  static final Map<EscapeRung, List<bool Function()>> _claims = {
    for (final rung in EscapeRung.values) rung: <bool Function()>[],
  };

  /// Claim Escape at [rung]. [claim] returns whether it took the press — false
  /// when the surface has nothing to take back just now, which passes the press
  /// down the ladder. Call the returned function to stand down (a `dispose`, a
  /// gesture ending).
  static VoidCallback register(EscapeRung rung, bool Function() claim) {
    _claims[rung]!.add(claim);
    // Removed and added rather than added once behind a flag: the handler list
    // belongs to the binding, which empties it between widget tests, and a flag
    // that said "already listening" would leave the ladder deaf from the second
    // test on. Removing something that is not there is a no-op.
    HardwareKeyboard.instance
      ..removeHandler(_onKey)
      ..addHandler(_onKey);
    return () => _release(rung, claim);
  }

  static void _release(EscapeRung rung, bool Function() claim) {
    _claims[rung]!.remove(claim);
    if (_claims.values.every((list) => list.isEmpty)) {
      HardwareKeyboard.instance.removeHandler(_onKey);
    }
  }

  /// Climb the ladder once and say whether anything took the press. Public for
  /// the tests, and for anything that wants Escape's meaning without the key.
  ///
  /// Within a rung the newest claim is asked first: two surfaces on the same
  /// rung means the one raised last is the inner one.
  static bool press() {
    for (final rung in EscapeRung.values) {
      for (final claim in _claims[rung]!.reversed.toList()) {
        if (claim()) return true;
      }
    }
    return false;
  }

  static bool _onKey(KeyEvent event) =>
      event is KeyDownEvent &&
      event.logicalKey == LogicalKeyboardKey.escape &&
      press();
}
