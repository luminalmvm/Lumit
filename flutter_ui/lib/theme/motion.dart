// How the chrome moves (docs/15-DESIGN.md §8).
//
// In plain terms: every animation in the interface has a name here, and the
// name answers two questions. Which shape is drawn, because Studio, Desk and
// Lantern do not move alike any more than they look alike. And how much motion
// the user asked for in Settings, because Full, Minimal and None are decided
// one animation at a time rather than by scaling everything down together.
//
// Nothing outside this file states a duration or a curve for chrome. A widget
// asks the theme scope for `motion` and reads the entry it needs, so a new
// animation the setting cannot reach has to be written on purpose.
//
// **What the three levels keep.** Full plays everything. Minimal keeps the
// motion that says something, where a row went or that a menu opened, as a
// 50ms fade or slide, and drops the motion that is only manner: nothing rises,
// grows, overshoots or trails. None plays nothing, and a widget given a still
// spec does not mount a controller at all.
//
// **What the three shapes change.** Studio is quick and quiet. Desk is an
// instrument, so its movements are shorter, stop dead and never bounce, and a
// menu only fades. Lantern's cards are soft, so things grow from where they
// came, take the whole 150ms budget and land with a small overshoot.

import 'package:flutter/animation.dart';

import 'theme.dart';

/// One movement: how long it takes and the curve it follows.
class MotionSpec {
  final Duration duration;
  final Curve curve;

  const MotionSpec(this.duration, [this.curve = Curves.easeOutCubic]);

  /// No movement. A widget handed this arrives where it is going.
  static const still = MotionSpec(Duration.zero, Curves.linear);

  bool get isStill => duration == Duration.zero;
}

const Duration _ms50 = Duration(milliseconds: 50);
const Duration _ms60 = Duration(milliseconds: 60);
const Duration _ms80 = Duration(milliseconds: 80);
const Duration _ms90 = Duration(milliseconds: 90);
const Duration _ms100 = Duration(milliseconds: 100);
const Duration _ms110 = Duration(milliseconds: 110);
const Duration _ms120 = Duration(milliseconds: 120);
const Duration _ms130 = Duration(milliseconds: 130);
const Duration _ms140 = Duration(milliseconds: 140);
const Duration _ms150 = Duration(milliseconds: 150);
const Duration _ms200 = Duration(milliseconds: 200);
const Duration _ms280 = Duration(milliseconds: 280);
const Duration _ms320 = Duration(milliseconds: 320);

/// The house ease-out: most of the distance early, a gentle stop.
const Curve _out = Curves.easeOutCubic;

/// Desk's stop: nearly all the distance at once, then nothing. A detent.
const Curve _detent = Curves.easeOutQuart;

/// Landing a little past the mark and coming back, once. About a twentieth of
/// the distance, so half a row dropped lands under a pixel long.
const Curve _land = Cubic(0.3, 1.4, 0.6, 1.0);

/// Lantern's landing, nearer a tenth.
const Curve _landSoft = Cubic(0.3, 1.6, 0.6, 1.0);

/// Every named movement, for one shape at one level.
class Motion {
  final ThemeShape shape;
  final AnimationLevel level;

  /// A fill coming up under the pointer, and leaving after it. Leaving is the
  /// slower of the two, so a pointer crossing a bar leaves a short trail.
  final MotionSpec hoverIn;
  final MotionSpec hoverOut;

  /// A menu row's highlight leaving. It always arrives at once: a menu is
  /// read by where the highlight is, and that may not lag the pointer.
  final MotionSpec rowTrail;

  /// A state mark changing: the tab that is fronted, the tool that is armed,
  /// a focus ring.
  final MotionSpec mark;

  /// A tick or a radio dot arriving, and the size it starts from.
  final MotionSpec pop;
  final double popScale;

  /// A switch's knob crossing its track.
  final MotionSpec toggle;

  /// A menu, dropdown or picker opening. A rise is where the thing starts, in
  /// pixels below its rest, so a negative one drops into place from above:
  /// a menu comes down from the control that opened it. The scale is the size
  /// it starts at.
  final MotionSpec popup;
  final double popupRise;
  final double popupScale;

  /// A submenu flying out beside its row. Opacity only at every level, since
  /// the menu's hover guard measures the flyout where it stands.
  final MotionSpec flyout;

  final MotionSpec tooltip;
  final double tooltipRise;

  /// A dialogue opening, and the wash behind it.
  final MotionSpec modal;
  final double modalRise;
  final double modalScale;
  final MotionSpec scrim;

  /// Rows moving out of the way of something dragged past them.
  final MotionSpec makeWay;

  /// A thing in hand being picked up: its shadow and wash arriving.
  final MotionSpec lift;

  /// A dropped thing landing in its slot.
  final MotionSpec settle;

  /// A drop target lighting up, and moving between the places a drop can go.
  final MotionSpec dropZone;

  /// Content arriving in a place that was already there: a settings page, a
  /// tab's body.
  final MotionSpec reveal;
  final double revealRise;

  /// One whole surface taking over from another: splash, welcome, shell.
  final MotionSpec swap;

  /// The guided tour going from one step to the next: the hole crossing the
  /// window and the card going with it. The one movement longer than 150ms,
  /// because it carries the eye from one panel to another and a cut that far
  /// loses it. Full only: at Minimal and None the next step is simply there.
  final MotionSpec tour;

  const Motion._({
    required this.shape,
    required this.level,
    required this.hoverIn,
    required this.hoverOut,
    required this.rowTrail,
    required this.mark,
    required this.pop,
    required this.popScale,
    required this.toggle,
    required this.popup,
    required this.popupRise,
    required this.popupScale,
    required this.flyout,
    required this.tooltip,
    required this.tooltipRise,
    required this.modal,
    required this.modalRise,
    required this.modalScale,
    required this.scrim,
    required this.makeWay,
    required this.lift,
    required this.settle,
    required this.dropZone,
    required this.reveal,
    required this.revealRise,
    required this.swap,
    required this.tour,
  });

  /// Whether nothing moves at all.
  bool get isStill => level == AnimationLevel.none;

  /// Whether a row being reordered is carried: it rides the pointer and the
  /// rows it passes step out of its way. Where it is not, the rows stay put
  /// and a line marks the place a drop would take, which says the same thing
  /// without anything travelling.
  bool get carries => level == AnimationLevel.all;

  static final List<Motion?> _table = List<Motion?>.filled(
      ThemeShape.values.length * AnimationLevel.values.length, null);

  /// The movements for [shape] at [level]. Built once each and kept.
  static Motion of(ThemeShape shape, AnimationLevel level) =>
      _table[shape.index * AnimationLevel.values.length + level.index] ??=
          switch (level) {
        AnimationLevel.none => _none(shape),
        AnimationLevel.minimal => _minimal(shape),
        AnimationLevel.all => switch (shape) {
            ThemeShape.studio => _studio,
            ThemeShape.desk => _desk,
            ThemeShape.lantern => _lantern,
          },
      };

  static Motion _none(ThemeShape shape) => Motion._(
        shape: shape,
        level: AnimationLevel.none,
        hoverIn: MotionSpec.still,
        hoverOut: MotionSpec.still,
        rowTrail: MotionSpec.still,
        mark: MotionSpec.still,
        pop: MotionSpec.still,
        popScale: 1,
        toggle: MotionSpec.still,
        popup: MotionSpec.still,
        popupRise: 0,
        popupScale: 1,
        flyout: MotionSpec.still,
        tooltip: MotionSpec.still,
        tooltipRise: 0,
        modal: MotionSpec.still,
        modalRise: 0,
        modalScale: 1,
        scrim: MotionSpec.still,
        makeWay: MotionSpec.still,
        lift: MotionSpec.still,
        settle: MotionSpec.still,
        dropZone: MotionSpec.still,
        reveal: MotionSpec.still,
        revealRise: 0,
        swap: MotionSpec.still,
        tour: MotionSpec.still,
      );

  /// Minimal is the same under every shape: what is left is too short to
  /// carry a manner. A 50ms fade or slide where the motion says something,
  /// and nothing where it only decorates.
  static Motion _minimal(ThemeShape shape) {
    const snap = MotionSpec(_ms50, Curves.easeOut);
    return Motion._(
      shape: shape,
      level: AnimationLevel.minimal,
      hoverIn: snap,
      hoverOut: snap,
      rowTrail: MotionSpec.still,
      mark: snap,
      pop: MotionSpec.still,
      popScale: 1,
      toggle: snap,
      popup: snap,
      popupRise: 0,
      popupScale: 1,
      flyout: MotionSpec.still,
      tooltip: MotionSpec.still,
      tooltipRise: 0,
      modal: snap,
      modalRise: 0,
      modalScale: 1,
      scrim: snap,
      // Nothing is carried at this level (see [carries]), so there is nothing
      // to make way for and nothing to land.
      makeWay: MotionSpec.still,
      lift: MotionSpec.still,
      settle: MotionSpec.still,
      dropZone: snap,
      reveal: MotionSpec.still,
      revealRise: 0,
      swap: snap,
      tour: MotionSpec.still,
    );
  }

  static const Motion _studio = Motion._(
    shape: ThemeShape.studio,
    level: AnimationLevel.all,
    hoverIn: MotionSpec(_ms60, _out),
    hoverOut: MotionSpec(_ms140, _out),
    rowTrail: MotionSpec(_ms90, Curves.easeOut),
    mark: MotionSpec(_ms120, _out),
    pop: MotionSpec(_ms110, _out),
    popScale: 0.5,
    toggle: MotionSpec(_ms120, _out),
    popup: MotionSpec(_ms110, _out),
    popupRise: -4,
    popupScale: 1,
    flyout: MotionSpec(_ms80, Curves.easeOut),
    tooltip: MotionSpec(_ms90, Curves.easeOut),
    tooltipRise: -2,
    modal: MotionSpec(_ms150, _out),
    modalRise: 8,
    modalScale: 0.985,
    scrim: MotionSpec(_ms150, Curves.easeOut),
    makeWay: MotionSpec(_ms130, _out),
    lift: MotionSpec(_ms90, Curves.easeOut),
    settle: MotionSpec(_ms150, _land),
    dropZone: MotionSpec(_ms100, _out),
    reveal: MotionSpec(_ms120, _out),
    revealRise: 4,
    swap: MotionSpec(_ms150, Curves.easeOut),
    tour: MotionSpec(_ms280, Curves.easeInOutCubic),
  );

  static const Motion _desk = Motion._(
    shape: ThemeShape.desk,
    level: AnimationLevel.all,
    hoverIn: MotionSpec(_ms50, _detent),
    hoverOut: MotionSpec(_ms90, _detent),
    rowTrail: MotionSpec(_ms60, Curves.easeOut),
    mark: MotionSpec(_ms80, _detent),
    // A tick on an instrument is on or it is off.
    pop: MotionSpec.still,
    popScale: 1,
    // A slide switch travels at one speed and stops against its end.
    toggle: MotionSpec(_ms80, Curves.linear),
    popup: MotionSpec(_ms80, _detent),
    popupRise: 0,
    popupScale: 1,
    flyout: MotionSpec(_ms60, Curves.easeOut),
    tooltip: MotionSpec(_ms60, Curves.easeOut),
    tooltipRise: 0,
    modal: MotionSpec(_ms100, _detent),
    modalRise: 0,
    modalScale: 1,
    scrim: MotionSpec(_ms100, Curves.easeOut),
    makeWay: MotionSpec(_ms90, _detent),
    lift: MotionSpec(_ms60, Curves.easeOut),
    settle: MotionSpec(_ms90, _detent),
    dropZone: MotionSpec(_ms60, _detent),
    reveal: MotionSpec(_ms80, Curves.easeOut),
    revealRise: 0,
    swap: MotionSpec(_ms100, Curves.easeOut),
    tour: MotionSpec(_ms200, _detent),
  );

  static const Motion _lantern = Motion._(
    shape: ThemeShape.lantern,
    level: AnimationLevel.all,
    hoverIn: MotionSpec(_ms90, _out),
    hoverOut: MotionSpec(_ms150, _out),
    rowTrail: MotionSpec(_ms110, Curves.easeOut),
    mark: MotionSpec(_ms150, _out),
    pop: MotionSpec(_ms140, _landSoft),
    popScale: 0.4,
    toggle: MotionSpec(_ms150, _landSoft),
    popup: MotionSpec(_ms140, _out),
    popupRise: -2,
    popupScale: 0.96,
    flyout: MotionSpec(_ms100, Curves.easeOut),
    tooltip: MotionSpec(_ms110, Curves.easeOut),
    tooltipRise: -3,
    modal: MotionSpec(_ms150, _out),
    modalRise: 0,
    modalScale: 0.96,
    scrim: MotionSpec(_ms150, Curves.easeOut),
    makeWay: MotionSpec(_ms150, _out),
    lift: MotionSpec(_ms110, Curves.easeOut),
    settle: MotionSpec(_ms150, _landSoft),
    dropZone: MotionSpec(_ms130, _out),
    reveal: MotionSpec(_ms150, _out),
    revealRise: 6,
    swap: MotionSpec(_ms150, Curves.easeOut),
    tour: MotionSpec(_ms320, Curves.easeInOutCubic),
  );
}
