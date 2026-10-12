// The house scrollbar: the thumb every scrolling list wears, and the scroll
// behaviour that hands it to them.

import 'package:flutter/gestures.dart'
    show PointerExitEvent, PointerHoverEvent, Velocity;
import 'package:flutter/material.dart';

import '../../theme/motion.dart';
import '../../theme/theme.dart';
import 'base.dart';
import 'motion.dart';

/// How many pixels thicker a thumb is under the pointer, where things swell.
const double scrollbarSwell = 3;

/// A thumb's colour: a raised block at rest, lit by [lit] of the way as the
/// pointer comes over it, and brightest while it is [held].
Color scrollThumbColour(LumitTheme t, double lit, {bool held = false}) => held
    ? t.textMuted
    : Color.lerp(
        t.surface4, Color.lerp(t.hairlineStrong, t.textMuted, 0.5), lit)!;

/// The scrollbar a scrolling list wears: Flutter's own, in the theme's
/// colours, with a thumb that answers the pointer.
///
/// In plain terms: the thumb brightens when the pointer is on the bar and
/// brightens again while it is dragged, at every animation level, because
/// that is what says it can be grabbed. At Full it also grows a few pixels
/// with a small spring.
class HouseScrollbar extends RawScrollbar {
  const HouseScrollbar({
    super.key,
    required super.child,
    super.controller,
    super.thumbVisibility,
    super.trackVisibility,
    super.thickness,
    super.padding,
    super.crossAxisMargin,
  }) : super(radius: const Radius.circular(3), minThumbLength: 20);

  @override
  RawScrollbarState<HouseScrollbar> createState() => _HouseScrollbarState();
}

class _HouseScrollbarState extends RawScrollbarState<HouseScrollbar> {
  late final HoverGlow _glow = HoverGlow(this, updateScrollbarPainter);

  Motion? get _motion => ThemeScope.maybeOf(context)?.motion;

  @override
  void updateScrollbarPainter() {
    super.updateScrollbarPainter();
    final scope = ThemeScope.maybeOf(context);
    if (scope == null) return;
    final t = scope.theme;
    scrollbarPainter
      ..color = scrollThumbColour(t, _glow.light, held: _glow.held)
      ..thickness = (widget.thickness ?? t.density.scrollbar) +
          scrollbarSwell * _glow.swell(scope.motion)
      ..trackRadius = const Radius.circular(3)
      ..trackColor = widget.trackVisibility ?? false
          ? t.surface2
          : const Color(0x00000000)
      ..trackBorderColor = const Color(0x00000000);
  }

  @override
  void handleHover(PointerHoverEvent event) {
    super.handleHover(event);
    _glow.mark(_motion,
        over: isPointerOverScrollbar(event.position, event.kind,
            forHover: true));
  }

  @override
  void handleHoverExit(PointerExitEvent event) {
    super.handleHoverExit(event);
    _glow.mark(_motion, over: false);
  }

  @override
  void handleThumbPressStart(Offset localPosition) {
    super.handleThumbPressStart(localPosition);
    _glow.mark(_motion, held: true);
  }

  @override
  void handleThumbPressEnd(Offset localPosition, Velocity velocity) {
    super.handleThumbPressEnd(localPosition, velocity);
    _glow.mark(_motion, held: false);
  }

  @override
  void dispose() {
    _glow.dispose();
    super.dispose();
  }
}

/// Gives every scrolling list in the application a [HouseScrollbar].
class HouseScrollBehavior extends MaterialScrollBehavior {
  const HouseScrollBehavior();

  @override
  Widget buildScrollbar(
          BuildContext context, Widget child, ScrollableDetails details) =>
      details.direction == AxisDirection.down ||
              details.direction == AxisDirection.up
          ? HouseScrollbar(
              controller: details.controller,
              crossAxisMargin: 2,
              child: child,
            )
          : child;
}
