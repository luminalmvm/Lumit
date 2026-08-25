// Menu hover intent: the "safe triangle" a pointer travels through on its way
// from a submenu row to its flyout (K-318).
//
// In plain terms: when a submenu is open beside a menu, the diagonal path from
// the row to the flyout crosses the rows below it. Without care, brushing one
// of those rows on the way switches the menu underneath the pointer and the
// flyout vanishes before it can be reached. The classic fix — the one menu
// toolkits and the JavaScript "hover intent" plugins use — is to draw an
// imaginary triangle from where the pointer is to the flyout's near corners,
// and treat any movement inside it as travel *towards* the flyout: the switch
// to the row actually under the pointer is held back briefly, and only lands
// if the pointer stops there or leaves the triangle.
//
// This file is the pure geometry, kept free of widgets so it can be tested as
// arithmetic; `controls.dart` owns the timers and the hover state.

import 'dart:ui';

import 'package:flutter/foundation.dart';

/// How long a pointer may sit inside the safe triangle over some other row
/// before that row wins anyway. Long enough to cross a wide menu on the
/// diagonal, short enough that resting on a row still switches promptly.
const Duration menuHoverGrace = Duration(milliseconds: 300);

/// Draw the live triangle over the menus — the Debug panel's switch, off in
/// every ordinary session. The geometry is invisible by nature, so the only
/// way to tell a working guard from a broken one is to look at it; this makes
/// it visible without changing a single decision the guard takes.
final ValueNotifier<bool> debugShowSafeTriangles = ValueNotifier<bool>(false);

/// The triangle between the pointer ([apex]) and the near edge of the flyout
/// it is presumed to be travelling to.
class SafeTriangle {
  final Offset apex;
  final Offset cornerA;
  final Offset cornerB;

  const SafeTriangle(this.apex, this.cornerA, this.cornerB);

  /// The triangle from [apex] to the vertical edge of [flyout] that faces it,
  /// grown by [slop] on every side: the apex backs away from the flyout and
  /// the corners stretch past it, so a pointer that wobbles a few pixels off
  /// the true diagonal still counts as travelling.
  factory SafeTriangle.towards(Offset apex, Rect flyout, {double slop = 6.0}) {
    final facingLeftEdge = apex.dx <= flyout.center.dx;
    final x = facingLeftEdge ? flyout.left : flyout.right;
    final back = facingLeftEdge ? -slop : slop;
    return SafeTriangle(
      apex.translate(back, 0),
      Offset(x, flyout.top - slop),
      Offset(x, flyout.bottom + slop),
    );
  }

  /// Whether [p] is inside (or on the edge of) the triangle. Same-side sign
  /// test against each edge; a degenerate triangle contains nothing.
  bool contains(Offset p) {
    double cross(Offset o, Offset a, Offset b) =>
        (a.dx - o.dx) * (b.dy - o.dy) - (a.dy - o.dy) * (b.dx - o.dx);
    final d1 = cross(p, apex, cornerA);
    final d2 = cross(p, cornerA, cornerB);
    final d3 = cross(p, cornerB, apex);
    final hasNeg = d1 < 0 || d2 < 0 || d3 < 0;
    final hasPos = d1 > 0 || d2 > 0 || d3 > 0;
    return !(hasNeg && hasPos);
  }
}
