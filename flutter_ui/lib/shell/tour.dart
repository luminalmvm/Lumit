// The guided tour: the window dimmed, one part of it left clear, and a small
// card beside that part saying what it is.
//
// One overlay above the shell, driven by a plain list of steps (the shipped
// list is `tourSteps` in tour_frb.dart). A step names the `ValueKey<String>`
// of the widget it is about, and the tour finds that widget in the live tree
// and measures it, which is how the screenshot sweeps aim at things too
// (tool/shots/shots_common.dart). There are no anchors to keep in step with
// the panels, and a step whose widget is not on screen is passed over instead
// of pointing at nothing.

import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';

/// Which side of its target a step's card stands on.
enum TourSide { left, right, above, below }

/// A state of the window that some steps need and the tour has to make: a
/// tab brought to the front, or another arrangement of the panels altogether.
/// A step with no scene is shown in the window as the tour found it.
class TourScene {
  /// Whether there is anything to show in it as things stand. Its steps are
  /// passed over while there is not.
  final bool Function() offered;

  /// Put the window into this state.
  final VoidCallback enter;

  /// Put the window back as the tour found it.
  final VoidCallback leave;

  const TourScene({
    required this.offered,
    required this.enter,
    required this.leave,
  });
}

/// One stop on the tour.
class TourStep {
  /// The `ValueKey<String>` of the widget this step is about.
  final String target;

  /// Whether a key is only how the widget's own key starts. A layer's twirl is
  /// keyed by the layer's id, and the step means whichever layer comes first.
  /// It reads [target] and [opensFrom] alike.
  final bool prefix;

  final String title;

  /// Two sentences at most.
  final String body;

  /// Where the card would like to stand. It moves to another side when this
  /// one has no room for it.
  final TourSide side;

  /// Whether the card points at the target, for one small enough to miss.
  final bool pointer;

  /// For a step that shows something shut away: what puts [target] on screen.
  /// Run as the step is reached, when the target is not there already.
  final VoidCallback? open;

  /// The key of the widget [open] works on. The step is offered while this is
  /// on screen, since its own target is not until [open] has run.
  final String? opensFrom;

  /// For a step whose [open] puts up something that should not outlast it:
  /// what takes that down again. Run as the tour leaves the step or ends.
  final VoidCallback? close;

  /// The state of the window this step is shown in, when that is not the one
  /// the tour found.
  final TourScene? scene;

  const TourStep({
    required this.target,
    this.prefix = false,
    required this.title,
    required this.body,
    required this.side,
    this.pointer = false,
    this.open,
    this.opensFrom,
    this.close,
    this.scene,
  });

  bool _means(String wanted, String key) =>
      prefix ? key.startsWith(wanted) : key == wanted;
}

/// The card's width. Its height is whatever the words need.
const double _cardWidth = 288;

/// How far the hole stands off its target, so the target's own edge is inside
/// the clear part instead of under the ring.
const double _holePad = 4;

/// The air between the hole and the card, which the pointer reaches across.
const double _cardGap = 12;

/// How close to the window's edge the card may stand.
const double _edge = 8;

/// The pointer: how far it reaches from the card, and half its width there.
const double _pointerReach = 7;
const double _pointerHalf = 7;

/// The least of a target that has to show for its step to be worth taking.
const double _leastShown = 6;

/// How many frames a step that opens something waits for its target to
/// arrive before the tour moves on without it.
const int _openPatience = 30;

/// How far through a move the card changes its words. It has faded out by
/// then, and fades back in over the rest.
const double _cardSwap = 0.4;

/// The tour that is up, so asking for it twice raises one.
OverlayEntry? _tourEntry;

/// Raise the tour over the shell, and say whether it went up. Finishing or
/// skipping it is recorded in [workspace], and [onEnd] runs after that.
///
/// With no step's target on screen there is nothing to show: nothing is
/// raised, nothing is recorded, and [onEnd] is not called.
bool showTour(
  BuildContext context,
  Workspace workspace, {
  required List<TourStep> steps,
  VoidCallback? onEnd,
}) {
  if (_tourEntry?.mounted ?? false) return false;
  final overlay = Overlay.of(context);
  final shown = _measure(steps, overlay);
  if (shown.rects.isEmpty) return false;
  late final OverlayEntry entry;
  entry = OverlayEntry(
    builder: (_) => _Tour(
      steps: steps,
      overlay: overlay,
      shown: shown,
      onEnd: () {
        entry.remove();
        _tourEntry = null;
        workspace.finishTour();
        onEnd?.call();
      },
    ),
  );
  _tourEntry = entry;
  overlay.insert(entry);
  return true;
}

/// What is on screen for the tour: where each step's target stands, by the
/// step's place in the list, and which steps could open theirs.
typedef _Shown = ({Map<int, Rect> rects, Set<int> openable});

/// Where each step's target stands in [overlay]'s own coordinates. A step with
/// nothing on screen is left out.
///
/// The overlay's coordinates and not the window's, because the UI scale sits
/// between the two (widgets/ui_scale.dart) and the tour is drawn inside it.
_Shown _measure(List<TourStep> steps, OverlayState overlay) {
  final room = overlay.context.findRenderObject();
  final rects = <int, Rect>{};
  final openable = <int>{};
  if (room is! RenderBox || !room.hasSize) {
    return (rects: rects, openable: openable);
  }
  void visit(Element element) {
    final key = element.widget.key;
    if (key is ValueKey<String>) {
      for (var i = 0; i < steps.length; i++) {
        final step = steps[i];
        if (!rects.containsKey(i) && step._means(step.target, key.value)) {
          final rect = _shownRect(element.renderObject, room);
          if (rect != null) rects[i] = rect;
        }
        final from = step.opensFrom;
        if (from != null &&
            !openable.contains(i) &&
            step._means(from, key.value) &&
            _shownRect(element.renderObject, room) != null) {
          openable.add(i);
        }
      }
    }
    element.visitChildren(visit);
  }

  overlay.context.visitChildElements(visit);
  return (rects: rects, openable: openable);
}

/// The part of [target] that is actually drawn, in [room]'s coordinates, or
/// null when too little of it is: a tab that is not the fronted one, a row
/// scrolled out of its list, a panel the window is too narrow to reach.
Rect? _shownRect(RenderObject? target, RenderBox room) {
  if (target is! RenderBox || !target.attached || !target.hasSize) return null;
  var rect = Offset.zero & target.size;
  RenderObject child = target;
  while (child != room) {
    final parent = child.parent;
    if (parent == null) return null;
    if (parent is RenderOffstage && parent.offstage) return null;
    final toParent = Matrix4.identity();
    parent.applyPaintTransform(child, toParent);
    rect = MatrixUtils.transformRect(toParent, rect);
    final clip = parent.describeApproximatePaintClip(child);
    if (clip != null) rect = rect.intersect(clip);
    if (rect.width < _leastShown || rect.height < _leastShown) return null;
    child = parent;
  }
  rect = rect.intersect(Offset.zero & room.size);
  if (rect.width < _leastShown || rect.height < _leastShown) return null;
  return rect;
}

/// Where a card stands and which side of the hole that is. No side means it
/// stands inside the hole, which is what a target filling the window leaves.
typedef _Placed = ({Rect card, TourSide? side});

/// Put a card of [card]'s size beside [hole]: on the side asked for when it
/// fits there, then opposite, then on either of the other two.
_Placed _place(Size room, Size card, Rect hole, TourSide want) {
  final cut = hole.inflate(_holePad);
  // Level with the middle of the hole, and kept inside the window.
  double along(double centre, double length, double most) =>
      (centre - length / 2)
          .clamp(_edge, math.max(_edge, most - _edge - length))
          .toDouble();
  Offset? on(TourSide side) => switch (side) {
        TourSide.right
            when cut.right + _cardGap + card.width <= room.width - _edge =>
          Offset(cut.right + _cardGap,
              along(cut.center.dy, card.height, room.height)),
        TourSide.left when cut.left - _cardGap - card.width >= _edge => Offset(
            cut.left - _cardGap - card.width,
            along(cut.center.dy, card.height, room.height)),
        TourSide.below
            when cut.bottom + _cardGap + card.height <= room.height - _edge =>
          Offset(along(cut.center.dx, card.width, room.width),
              cut.bottom + _cardGap),
        TourSide.above when cut.top - _cardGap - card.height >= _edge => Offset(
            along(cut.center.dx, card.width, room.width),
            cut.top - _cardGap - card.height),
        _ => null,
      };
  final order = switch (want) {
    TourSide.right => const [TourSide.right, TourSide.left],
    TourSide.left => const [TourSide.left, TourSide.right],
    TourSide.below => const [TourSide.below, TourSide.above],
    TourSide.above => const [TourSide.above, TourSide.below],
  };
  for (final side in [...order, ...TourSide.values]) {
    final at = on(side);
    if (at != null) return (card: at & card, side: side);
  }
  return (
    card: Offset(
          math.max(_edge, cut.right - _holePad - _cardGap - card.width),
          math.max(_edge, cut.bottom - _holePad - _cardGap - card.height),
        ) &
        card,
    side: null,
  );
}

class _Tour extends StatefulWidget {
  final List<TourStep> steps;
  final OverlayState overlay;

  /// What [showTour] measured before raising this.
  final _Shown shown;
  final VoidCallback onEnd;

  const _Tour({
    required this.steps,
    required this.overlay,
    required this.shown,
    required this.onEnd,
  });

  @override
  State<_Tour> createState() => _TourState();
}

class _TourState extends State<_Tour> with SingleTickerProviderStateMixin {
  /// What is on screen, as of the last frame.
  late _Shown _shown = widget.shown;

  /// The first step with something to point at in the window as it stands.
  late int _index = () {
    final found = [
      for (final i in _shown.rects.keys)
        if (widget.steps[i].scene == null) i,
    ];
    return (found.isEmpty ? _shown.rects.keys : found).reduce(math.min);
  }();

  /// The scene the window is in, null for as the tour found it.
  TourScene? _scene;

  /// Where the step being shown was last seen. The hole stays there while the
  /// next step's scene is being set, so the move to it has somewhere to start.
  late Rect? _held = _shown.rects[_index];

  /// The move from the last step to this one: 0 as it sets off, 1 at rest.
  /// The hole crosses the window over the whole of it, and the card goes with
  /// the hole, changing its words part of the way while it is faded out.
  late final AnimationController _move =
      AnimationController(vsync: this, value: 1);

  /// The curve [_move] is read through, which is the style's own.
  Curve _curve = Curves.linear;

  /// Where the move set off from: the hole as it stood, and the step whose
  /// card was up. Null on the first step, which has nowhere to come from.
  Rect? _fromHole;
  int? _fromIndex;

  /// A step that is waiting for its target to arrive, how many more frames it
  /// will wait, and whether its own [TourStep.open] has been run yet.
  int? _opening;
  int _patience = 0;
  bool _asked = false;

  /// The step whose [TourStep.open] has put something up that its
  /// [TourStep.close] has yet to take down.
  int? _put;

  /// Steps whose target never arrived after being opened. Not offered again.
  final Set<int> _unopened = {};

  bool _ended = false;

  /// Where the card was last laid out, for the pointer drawn from it.
  final ValueNotifier<_Placed?> _placed = ValueNotifier(null);

  VoidCallback? _escapeRelease;

  /// The tour's own focus scope. It holds the focus while the tour is up, so
  /// Tab moves between the card's buttons and an arrow key is the tour's
  /// before it is anything's under the wash.
  final FocusScopeNode _focus = FocusScopeNode(debugLabel: 'tour');

  @override
  void initState() {
    super.initState();
    // A surface that has taken the window: the panels' keys stand down while
    // it is up, and Escape ends it from the dialogue rung.
    markModalMounted();
    _escapeRelease = EscapeLadder.register(EscapeRung.dialog, () {
      _end();
      return true;
    });
    WidgetsBinding.instance.addPostFrameCallback(_follow);
  }

  @override
  void dispose() {
    markModalUnmounted();
    _escapeRelease?.call();
    _move.dispose();
    _focus.dispose();
    _placed.dispose();
    super.dispose();
  }

  /// Measure again after every frame drawn while the tour is up, so the hole
  /// stays on a target the window has just moved, and keep hold of the focus.
  /// This asks for no frames of its own except while a step is opening: it
  /// runs when something else has drawn one.
  void _follow(Duration _) {
    if (!mounted || _ended) return;
    WidgetsBinding.instance.addPostFrameCallback(_follow);
    // Asked for here and not in initState: a scope that is not in the tree yet
    // cannot take the focus, and the request is not kept for when it is.
    if (!_focus.hasFocus) _focus.requestFocus();
    final shown = _measure(widget.steps, widget.overlay);
    final opening = _opening;
    if (opening != null) {
      _shown = shown;
      if (shown.rects.containsKey(opening)) {
        _arrive(opening);
        return;
      }
      final open = widget.steps[opening].open;
      if (!_asked && open != null && shown.openable.contains(opening)) {
        _asked = true;
        _patience = _openPatience;
        open();
        _put = opening;
      }
      if (--_patience > 0) {
        WidgetsBinding.instance.scheduleFrame();
      } else {
        // It was asked for and nothing came of it. On past it.
        _opening = null;
        _unopened.add(opening);
        _leave(opening);
      }
      return;
    }
    if (shown.rects.containsKey(_index)) {
      _held = shown.rects[_index];
      if (mapEquals(shown.rects, _shown.rects) &&
          setEquals(shown.openable, _shown.openable)) {
        return;
      }
      setState(() => _shown = shown);
      return;
    }
    // The target has left the screen under the card.
    _shown = shown;
    _leave(_index);
  }

  /// Whether step [i] can be shown: its target is on screen, or it knows how
  /// to put it there. A step in another scene is taken on that scene's word
  /// until it has been tried, since what is on screen now says nothing of it.
  bool _offered(int i) {
    final scene = widget.steps[i].scene;
    if (!identical(scene, _scene)) {
      return !_unopened.contains(i) && (scene == null || scene.offered());
    }
    return _shown.rects.containsKey(i) ||
        (_shown.openable.contains(i) && !_unopened.contains(i));
  }

  int? _after(int from) {
    for (var i = from + 1; i < widget.steps.length; i++) {
      if (_offered(i)) return i;
    }
    return null;
  }

  int? _before(int from) {
    for (var i = from - 1; i >= 0; i--) {
      if (_offered(i)) return i;
    }
    return null;
  }

  /// Go to step [to], setting its scene first when the window is in another,
  /// and opening its target when that is shut away.
  void _go(int to) {
    if (_put != to) _takeDown();
    final scene = widget.steps[to].scene;
    if (identical(scene, _scene) && _shown.rects.containsKey(to)) {
      _arrive(to);
      return;
    }
    if (!identical(scene, _scene)) {
      _scene?.leave();
      _scene = scene;
      scene?.enter();
    }
    // The follower takes it from here, a frame at a time.
    setState(() {
      _opening = to;
      _asked = false;
      _patience = _openPatience;
    });
    WidgetsBinding.instance.scheduleFrame();
  }

  /// Step [to]'s target is on screen: move the hole and the card to it.
  void _arrive(int to) {
    final motion = ThemeScope.of(context).motion.tour;
    // Nowhere to come from when the last step's target has left the screen:
    // the hole and the card are then simply at the new one.
    final from = _holeNow;
    setState(() {
      _opening = null;
      _fromHole = from;
      _fromIndex = from == null ? null : _index;
      _index = to;
      _held = _shown.rects[to];
      _curve = motion.curve;
    });
    if (motion.isStill || from == null) {
      _move.value = 1;
    } else {
      _move.duration = motion.duration;
      _move.forward(from: 0);
    }
  }

  /// Step [gone] cannot be shown: on to the next that can, or back to the last
  /// that could, and with neither the tour is over.
  void _leave(int gone) {
    final to = _after(gone) ?? _before(gone);
    if (to == null) {
      _end();
    } else {
      _go(to);
    }
  }

  /// Where the step being shown has its target: on screen, or where it last
  /// was while the next step's scene is taking it away.
  Rect? get _target =>
      _shown.rects[_index] ?? (_opening == null ? null : _held);

  /// The hole where it is drawn this instant, part of the way through a move
  /// or at rest.
  Rect? get _holeNow {
    final to = _target;
    final from = _fromHole;
    if (from == null || to == null) return to;
    return Rect.lerp(from, to, _curve.transform(_move.value));
  }

  void _next() {
    final to = _after(_opening ?? _index);
    if (to == null) {
      _end();
    } else {
      _go(to);
    }
  }

  void _back() {
    final to = _before(_opening ?? _index);
    if (to != null) _go(to);
  }

  /// Take down whatever a step's [TourStep.open] put up for it.
  void _takeDown() {
    final put = _put;
    if (put == null) return;
    _put = null;
    widget.steps[put].close?.call();
  }

  void _end() {
    if (_ended) return;
    _ended = true;
    _takeDown();
    // The window goes back as it was found before anybody is told.
    _scene?.leave();
    _scene = null;
    widget.onEnd();
  }

  /// Right is Next and Left is Back. Both are taken whole, repeats included,
  /// so neither goes on to move the focus along the menu bar.
  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    final right = event.logicalKey == LogicalKeyboardKey.arrowRight;
    if (!right && event.logicalKey != LogicalKeyboardKey.arrowLeft) {
      return KeyEventResult.ignored;
    }
    if (event is KeyDownEvent) {
      if (right) {
        _next();
      } else {
        _back();
      }
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final to = _target;
    return FocusScope(
      node: _focus,
      onKeyEvent: _onKey,
      // The wash takes every click, so nothing under it can be pressed while
      // the tour is up. A click on it does nothing: only Skip and Escape end
      // the tour.
      child: Listener(
        behavior: HitTestBehavior.opaque,
        child: Entrance.fade(
          spec: scope.motion.scrim,
          child: AnimatedBuilder(
            animation: _move,
            builder: (context, _) {
              final at = _move.value;
              final hole = _holeNow;
              // Before the swap the card is still the last step's, on its way
              // out; after it, this step's, on its way in.
              final leaving = at < _cardSwap && _fromIndex != null;
              final showing = leaving ? _fromIndex! : _index;
              final step = widget.steps[showing];
              return Stack(
                children: [
                  Positioned.fill(
                    child: CustomPaint(
                      key: const ValueKey('tour-wash'),
                      painter: _WashPainter(
                        hole: hole,
                        wash: t.scrim,
                        ring: t.accent,
                        radius: t.tokens.floatRadius,
                      ),
                    ),
                  ),
                  if (to != null && hole != null)
                    Positioned.fill(
                      child: Opacity(
                        opacity: leaving
                            ? 1 - at / _cardSwap
                            : _fromIndex == null
                                ? 1
                                : (at - _cardSwap) / (1 - _cardSwap),
                        child: Stack(
                          children: [
                            Positioned.fill(
                              child: CustomSingleChildLayout(
                                delegate: _CardLayout(
                                  from: _fromHole,
                                  fromSide: _fromIndex == null
                                      ? null
                                      : widget.steps[_fromIndex!].side,
                                  to: to,
                                  toSide: widget.steps[_index].side,
                                  travelled: _curve.transform(at),
                                  leaving: leaving,
                                  placed: _placed,
                                ),
                                child: KeyedSubtree(
                                  key: ValueKey<int>(showing),
                                  child: _card(t, step, showing),
                                ),
                              ),
                            ),
                            if (step.pointer)
                              Positioned.fill(
                                child: IgnorePointer(
                                  child: CustomPaint(
                                    painter: _PointerPainter(
                                      _placed,
                                      hole: hole,
                                      fill: t.surface3,
                                      edge: t.hairline,
                                      corner: t.tokens.floatRadius,
                                    ),
                                  ),
                                ),
                              ),
                          ],
                        ),
                      ),
                    ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }

  Widget _card(LumitTheme t, TourStep step, int index) => FloatSurface(
        key: const ValueKey('tour-card'),
        width: _cardWidth,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(8, 6, 8, 6),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(step.title,
                  style: t.bodyStrong.copyWith(color: t.textPrimary)),
              const SizedBox(height: 4),
              Text(step.body, style: t.body),
              const SizedBox(height: 12),
              Row(
                children: [
                  HouseButton(
                    key: const ValueKey('tour-skip'),
                    small: true,
                    frameless: true,
                    onPressed: _end,
                    child: Text(l10n.skip, style: t.small),
                  ),
                  const Spacer(),
                  if (_before(index) != null) ...[
                    HouseButton(
                      key: const ValueKey('tour-back'),
                      small: true,
                      onPressed: _back,
                      child: Text(l10n.tourBack),
                    ),
                    const SizedBox(width: 6),
                  ],
                  HouseButton(
                    key: const ValueKey('tour-next'),
                    small: true,
                    primary: true,
                    onPressed: _next,
                    // The last step with anything to point at says so.
                    child:
                        Text(_after(index) == null ? l10n.done : l10n.tourNext),
                  ),
                ],
              ),
            ],
          ),
        ),
      );
}

/// Stands the card beside the hole and says where it put it. Part of the way
/// through a move it stands part of the way between its two places.
class _CardLayout extends SingleChildLayoutDelegate {
  final Rect? from;
  final TourSide? fromSide;
  final Rect to;
  final TourSide toSide;

  /// How far the move has got along its curve, 1 at rest.
  final double travelled;

  /// Whether the card being laid out is still the last step's.
  final bool leaving;
  final ValueNotifier<_Placed?> placed;

  const _CardLayout({
    required this.from,
    required this.fromSide,
    required this.to,
    required this.toSide,
    required this.travelled,
    required this.leaving,
    required this.placed,
  });

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) =>
      BoxConstraints.loose(Size(
        math.max(0.0, constraints.maxWidth - 2 * _edge),
        math.max(0.0, constraints.maxHeight - 2 * _edge),
      ));

  @override
  Offset getPositionForChild(Size size, Size childSize) {
    final there = _place(size, childSize, to, toSide);
    final from = this.from;
    final fromSide = this.fromSide;
    if (from == null || fromSide == null || travelled >= 1) {
      placed.value = there;
      return there.card.topLeft;
    }
    final here = _place(size, childSize, from, fromSide);
    final at =
        Offset.lerp(here.card.topLeft, there.card.topLeft, travelled)!;
    placed.value = (card: at & childSize, side: (leaving ? here : there).side);
    return at;
  }

  @override
  bool shouldRelayout(_CardLayout old) =>
      old.from != from ||
      old.fromSide != fromSide ||
      old.to != to ||
      old.toSide != toSide ||
      old.travelled != travelled ||
      old.leaving != leaving;
}

/// The dimmed window with the hole cut in it, and the ring round the hole.
class _WashPainter extends CustomPainter {
  final Rect? hole;
  final Color wash;
  final Color ring;
  final double radius;

  const _WashPainter({
    required this.hole,
    required this.wash,
    required this.ring,
    required this.radius,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final hole = this.hole;
    final all = Path()
      ..fillType = PathFillType.evenOdd
      ..addRect(Offset.zero & size);
    if (hole == null) {
      canvas.drawPath(all, Paint()..color = wash);
      return;
    }
    final cut = RRect.fromRectAndRadius(
        hole.inflate(_holePad), Radius.circular(radius));
    canvas.drawPath(all..addRRect(cut), Paint()..color = wash);
    canvas.drawRRect(
      cut.deflate(0.5),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = ring,
    );
  }

  @override
  bool shouldRepaint(_WashPainter old) =>
      old.hole != hole ||
      old.wash != wash ||
      old.ring != ring ||
      old.radius != radius;
}

/// The pointer from the card to the hole: a small triangle on the card's near
/// edge, in the card's own fill and edge so the two read as one piece.
class _PointerPainter extends CustomPainter {
  final ValueNotifier<_Placed?> placed;
  final Rect hole;
  final Color fill;
  final Color edge;

  /// The card's corner, which the pointer keeps clear of.
  final double corner;

  _PointerPainter(
    this.placed, {
    required this.hole,
    required this.fill,
    required this.edge,
    required this.corner,
  }) : super(repaint: placed);

  @override
  void paint(Canvas canvas, Size size) {
    final at = placed.value;
    final side = at?.side;
    if (at == null || side == null) return;
    final card = at.card;
    final clear = corner + _pointerHalf + 2;
    // Level with the middle of the hole, as far as the card's edge reaches.
    double level(double centre, double from, double to) =>
        from + clear > to - clear
            ? (from + to) / 2
            : centre.clamp(from + clear, to - clear).toDouble();
    final y = level(hole.center.dy, card.top, card.bottom);
    final x = level(hole.center.dx, card.left, card.right);
    // The base sits a pixel inside the card, over its hairline, so the edge
    // runs out along the pointer instead of across the foot of it.
    final (a, tip, b) = switch (side) {
      TourSide.right => (
          Offset(card.left + 1, y - _pointerHalf),
          Offset(card.left - _pointerReach, y),
          Offset(card.left + 1, y + _pointerHalf),
        ),
      TourSide.left => (
          Offset(card.right - 1, y - _pointerHalf),
          Offset(card.right + _pointerReach, y),
          Offset(card.right - 1, y + _pointerHalf),
        ),
      TourSide.below => (
          Offset(x - _pointerHalf, card.top + 1),
          Offset(x, card.top - _pointerReach),
          Offset(x + _pointerHalf, card.top + 1),
        ),
      TourSide.above => (
          Offset(x - _pointerHalf, card.bottom - 1),
          Offset(x, card.bottom + _pointerReach),
          Offset(x + _pointerHalf, card.bottom - 1),
        ),
    };
    canvas.drawPath(
      Path()
        ..moveTo(a.dx, a.dy)
        ..lineTo(tip.dx, tip.dy)
        ..lineTo(b.dx, b.dy)
        ..close(),
      Paint()..color = fill,
    );
    final line = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = edge;
    canvas.drawLine(a, tip, line);
    canvas.drawLine(tip, b, line);
  }

  @override
  bool shouldRepaint(_PointerPainter old) =>
      old.hole != hole ||
      old.fill != fill ||
      old.edge != edge ||
      old.corner != corner;
}
