// The house scrollbar: the thumb every scrolling list wears, the glide a
// wheel gives a list, and the scroll behaviour that hands both to them.

import 'dart:async' show scheduleMicrotask;

import 'package:flutter/gestures.dart'
    show
        PointerExitEvent,
        PointerHoverEvent,
        PointerScrollEvent,
        PointerSignalEvent,
        Velocity;
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

/// Lets a notch of the wheel carry a list to where it lands, where Flutter
/// puts it there at once.
///
/// In plain terms: a mouse wheel moves a list a hundred pixels a notch, and
/// drawn as a jump that loses the eye its place. This plays the same move as
/// a short glide. A second notch while one is running adds to where it was
/// heading, so a spun wheel lands where it always would have.
///
/// **How, since a list takes the wheel itself.** The scroll view under
/// [child] has already claimed the event by the time it reaches here, and
/// moves when the claim is settled. So this notes where the list stood, and
/// straight after, before a frame is drawn, puts it back and runs it to
/// where it was sent. Nothing is ever shown at the far end early.
///
/// A step smaller than [_notch] is left alone: that is a wheel that already
/// turns smoothly, and easing each of its steps would only make it lag.
class SmoothWheel extends StatefulWidget {
  final ScrollController? controller;
  final Widget child;

  const SmoothWheel({super.key, required this.controller, required this.child});

  @override
  State<SmoothWheel> createState() => _SmoothWheelState();
}

class _SmoothWheelState extends State<SmoothWheel> {
  /// The least a step has to be, in pixels, to be a notch.
  static const double _notch = 24;

  /// Where the glide in flight is heading, or null when none is.
  double? _target;

  /// Counts the glides, so one that ends late does not clear a newer one.
  int _glides = 0;

  void _wheel(PointerSignalEvent event) {
    if (event is! PointerScrollEvent) return;
    final spec =
        context.getInheritedWidgetOfExactType<ThemeScope>()?.motion.scroll;
    if (spec == null || spec.isStill) return;
    final controller = widget.controller;
    if (controller == null || controller.positions.length != 1) return;
    final position = controller.position;
    final from = position.pixels;
    // Read now: the list's own move ends the glide in flight, and with it
    // where that glide was going.
    final base = _target ?? from;
    scheduleMicrotask(() {
      if (!mounted || !position.hasPixels) return;
      final step = position.pixels - from;
      if (step.abs() < _notch) return;
      final target = (base + step)
          .clamp(position.minScrollExtent, position.maxScrollExtent)
          .toDouble();
      // Back by a jump the list's listeners hear, so two lists that scroll
      // as one are never drawn a frame apart.
      position.jumpTo(from);
      final glide = ++_glides;
      _target = target;
      position
          .animateTo(target, duration: spec.duration, curve: spec.curve)
          .whenComplete(() {
        if (glide == _glides) _target = null;
      });
    });
  }

  @override
  Widget build(BuildContext context) =>
      Listener(onPointerSignal: _wheel, child: widget.child);
}

/// Gives every scrolling list in the application a [HouseScrollbar], and its
/// wheel a glide ([SmoothWheel]).
class HouseScrollBehavior extends MaterialScrollBehavior {
  const HouseScrollBehavior();

  // Flutter's own hook for wrapping every scroll view, the ones that draw a
  // scrollbar of their own included. A desktop has no overscroll indicator
  // to draw, so the wrap is all this adds.
  @override
  Widget buildOverscrollIndicator(
          BuildContext context, Widget child, ScrollableDetails details) =>
      SmoothWheel(
        controller: details.controller,
        child: super.buildOverscrollIndicator(context, child, details),
      );

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
