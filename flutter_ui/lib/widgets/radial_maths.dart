// The geometry of a Blender-style radial menu (K-324).
//
// In plain terms: a radial (or "pie") menu puts its choices in a ring around
// where the pointer already is, so every choice is the same short distance
// away and — the part that matters — always in the *same direction*. After a
// few uses the hand learns "delete is down-left" and stops reading the menu at
// all. That is what a list of items in a dropdown can never offer: a list's
// third entry moves the moment the list grows.
//
// Two rules follow from that, and they are the whole of this file:
//
//   - A slice is chosen by ANGLE alone, not by whether the pointer is inside
//     any drawn shape. Flick in a direction and the choice is made, however
//     far the pointer travelled — so the gesture can be as fast as the hand.
//   - There is a dead zone in the middle. Inside it nothing is selected, so
//     opening the menu and releasing without moving cancels rather than
//     picking whatever happened to be under the cursor.
//
// Kept free of widgets so it can be tested as arithmetic.

import 'dart:math' as math;

/// How far the pointer must leave the centre before any slice is chosen.
/// Below this the menu is open but nothing is picked (Blender's own idea).
const double radialDeadZone = 26;

/// The ring's radius: where the labels sit.
const double radialRadius = 96;

/// The ring's full visual reach from its centre: the labels sit at
/// [radialRadius] and a slice pill extends about half its width past them.
const double radialExtent = radialRadius + 56;

/// Clamp [v] into [lo, hi]; when the room is narrower than nothing (a window
/// smaller than the ring) settle on the middle rather than throwing.
double _fit(double v, double lo, double hi) =>
    hi < lo ? (lo + hi) / 2 : (v < lo ? lo : (v > hi ? hi : v));

/// Where the console sits (K-325): the ring centred on the pointer — pulled
/// in just enough that the whole ring stays on screen — and the search bar
/// above it, or below it when the top of the window would cut it off.
///
/// Pure arithmetic, so the placement rules are tested without a widget tree.
({double centreX, double centreY, double barLeft, double barTop, bool barBelow})
    fxConsoleLayout({
  required double screenWidth,
  required double screenHeight,
  required double anchorX,
  required double anchorY,
  required double barWidth,
  required double barHeight,
  double margin = 8,
  double gap = 12,
}) {
  final centreX =
      _fit(anchorX, radialExtent + margin, screenWidth - radialExtent - margin);
  final centreY = _fit(
      anchorY, radialExtent + margin, screenHeight - radialExtent - margin);
  final barLeft =
      _fit(centreX - barWidth / 2, margin, screenWidth - barWidth - margin);
  // Above the ring by default — the eye reads top-down, and the dropdown the
  // search opens needs the room below. Below only when above would clip.
  final above = centreY - radialExtent - gap - barHeight;
  final barBelow = above < margin;
  final barTop = barBelow ? centreY + radialExtent + gap : above;
  return (
    centreX: centreX,
    centreY: centreY,
    barLeft: barLeft,
    barTop: barTop,
    barBelow: barBelow,
  );
}

/// The angle, in radians clockwise from straight up, at which slice [index] of
/// [count] sits — the centre of its wedge.
///
/// Straight up is the first slice, because "up" is the direction a hand
/// reaches for first and the one the eye finds without looking.
double radialSliceAngle(int index, int count) {
  if (count <= 0) return 0;
  return index * 2 * math.pi / count;
}

/// Where slice [index] of [count] is drawn, relative to the menu's centre.
///
/// Screen coordinates: y grows downward, so "up" is a negative dy.
({double dx, double dy}) radialSliceOffset(int index, int count,
    {double radius = radialRadius}) {
  final angle = radialSliceAngle(index, count);
  return (dx: radius * math.sin(angle), dy: -radius * math.cos(angle));
}

/// Which slice a pointer at ([dx], [dy]) from the centre is choosing, or null
/// inside the dead zone (and for an empty menu).
///
/// By angle only: the distance beyond the dead zone does not matter, so a
/// confident flick lands the same choice as a careful one.
int? radialSliceAt(double dx, double dy, int count,
    {double deadZone = radialDeadZone}) {
  if (count <= 0) return null;
  if (dx * dx + dy * dy < deadZone * deadZone) return null;
  // atan2 measured from straight up, clockwise, wrapped into [0, 2pi).
  var angle = math.atan2(dx, -dy);
  if (angle < 0) angle += 2 * math.pi;
  final slice = 2 * math.pi / count;
  // Each wedge is centred on its angle, so the boundary sits half a wedge
  // either side — hence the half-slice shift before flooring.
  final index = ((angle + slice / 2) / slice).floor() % count;
  return index;
}
