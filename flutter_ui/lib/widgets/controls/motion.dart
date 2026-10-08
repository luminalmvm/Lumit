// The moving parts the house controls share: the entrance a floating surface
// makes, and the quarter turn a twirl takes.
//
// Timing is never stated here. Each of these is handed a `MotionSpec` from the
// theme scope's `motion` (theme/motion.dart), which is where the shape and the
// user's animation level are answered.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../../theme/motion.dart';
import 'base.dart';

/// Plays [child] in once, when it is first put on screen: a fade, and with it
/// a short travel from [rise] and a growth from [scale] where those are given.
///
/// In plain terms, this is how a menu, a dialogue or a tooltip arrives. It
/// only ever plays forwards and only once. Leaving is the caller taking the
/// widget away, which is immediate: what the user dismissed is gone when they
/// dismissed it, and nothing waits on an animation to finish.
///
/// **The child answers the pointer from its first frame.** The fade and the
/// transform are paint only, so a menu can be clicked while it is still
/// arriving.
///
/// **Under a still spec no controller is made** (docs/15-DESIGN.md §8, "springs
/// don't mount"). Whether to play is decided once, when the widget mounts, and
/// the tree it builds is the same either way, so changing the animation level
/// while a window is open never rebuilds what is inside that window.
class Entrance extends StatefulWidget {
  final MotionSpec spec;

  /// Where the child starts, in pixels below its rest. Negative starts above.
  final double rise;

  /// The size the child starts at, 1 being its own.
  final double scale;

  /// The point the growth is measured from.
  final Alignment alignment;

  /// Whether the child only fades. Fixed by the call site rather than worked
  /// out from [rise] and [scale], which change with the animation level: the
  /// tree built here must not change shape under a child that is on screen.
  final bool fadeOnly;

  final Widget child;

  const Entrance({
    super.key,
    required this.spec,
    this.rise = 0,
    this.scale = 1,
    this.alignment = Alignment.center,
    required this.child,
  }) : fadeOnly = false;

  /// An entrance that is a fade and nothing else, for a surface that fills
  /// its place and has nowhere to travel from: a wash, a whole window.
  const Entrance.fade({super.key, required this.spec, required this.child})
      : rise = 0,
        scale = 1,
        alignment = Alignment.center,
        fadeOnly = true;

  @override
  State<Entrance> createState() => _EntranceState();
}

class _EntranceState extends State<Entrance>
    with SingleTickerProviderStateMixin {
  AnimationController? _controller;
  Animation<double> _fade = kAlwaysCompleteAnimation;
  Animation<double> _travel = kAlwaysCompleteAnimation;

  @override
  void initState() {
    super.initState();
    final spec = widget.spec;
    if (spec.isStill) return;
    final controller =
        AnimationController(vsync: this, duration: spec.duration);
    _controller = controller;
    // The fade is done in the first two thirds, so the surface is whole while
    // it is still coming to rest. A plain ease, because a curve that
    // overshoots would ask for an opacity above one.
    _fade = CurvedAnimation(
      parent: controller,
      curve: const Interval(0, 0.66, curve: Curves.easeOut),
    );
    _travel = CurvedAnimation(parent: controller, curve: spec.curve);
    controller.forward();
  }

  @override
  void dispose() {
    _controller?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => FadeTransition(
        opacity: _fade,
        child: widget.fadeOnly
            ? widget.child
            : AnimatedBuilder(
                animation: _travel,
                child: widget.child,
                builder: (context, child) {
                  final left = 1 - _travel.value;
                  final scale = 1 - (1 - widget.scale) * left;
                  return Transform(
                    transform: Matrix4.identity()
                      ..translateByDouble(0, widget.rise * left, 0, 1)
                      ..scaleByDouble(scale, scale, 1, 1),
                    alignment: widget.alignment,
                    child: child,
                  );
                },
              ),
      );
}

/// A twirl's triangle, turned a quarter as it opens or shuts.
///
/// In plain terms: the little triangle beside a layer or an effect points
/// right when shut and down when open. The set draws those as two glyphs, one
/// a quarter turn of the other, so swapping them cut from one to the other.
/// This draws the new glyph a quarter turn back and lets it come round, which
/// reads as the one triangle turning.
///
/// [child] is the glyph for the state [open] says, exactly as the caller drew
/// it before. At rest nothing is rotated, so the glyph sits on the same pixels
/// it always did. It turns only when [open] changes while it is on screen: a
/// row scrolled into view arrives already facing the right way.
class TwirlTurn extends StatefulWidget {
  final bool open;
  final Widget child;

  const TwirlTurn({super.key, required this.open, required this.child});

  @override
  State<TwirlTurn> createState() => _TwirlTurnState();
}

class _TwirlTurnState extends State<TwirlTurn>
    with SingleTickerProviderStateMixin {
  AnimationController? _controller;
  Curve _curve = Curves.linear;

  @override
  void didUpdateWidget(TwirlTurn old) {
    super.didUpdateWidget(old);
    if (old.open == widget.open) return;
    final spec = ThemeScope.of(context).motion.mark;
    if (spec.isStill) {
      _controller?.value = 1;
      return;
    }
    _curve = spec.curve;
    final controller = _controller ??= AnimationController(vsync: this)
      ..addListener(() => setState(() {}));
    controller
      ..duration = spec.duration
      ..forward(from: 0);
  }

  @override
  void dispose() {
    _controller?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final controller = _controller;
    final left = controller == null || !controller.isAnimating
        ? 0.0
        : 1 - _curve.transform(controller.value);
    // Opening, the down-pointing glyph starts where the right-pointing one
    // was, a quarter turn anticlockwise of its rest. Shutting is the reverse.
    return Transform.rotate(
      angle: (widget.open ? -1 : 1) * left * math.pi / 2,
      child: widget.child,
    );
  }
}
