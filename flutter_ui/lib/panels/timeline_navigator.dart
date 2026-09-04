// The time navigator: the whole composition as one slim strip, with the part
// the lanes are showing drawn as a window on it (docs/07 §4.6, T5).
//
// # In plain terms
//
// Zoom the Timeline in and the lanes show a slice of the composition. Nothing
// on screen said which slice, or how big it was against the whole: the
// scrollbar under the lanes says where you are, but it says it in pixels of a
// content width nobody can see, and it says nothing at all about the playhead.
// So the strip above the lanes draws the comp end to end at a fixed size, and
// on it: the visible span as a window, and the playhead as a line. Where you
// are, how much you have, and where the frame you are on sits in the whole —
// at a glance, without reading anything.
//
// It is a control as well as a readout, which is the half that makes it worth
// the room. Drag the window and the lanes pan. Drag either end of it and the
// lanes zoom, about the end you did *not* take hold of, so the frame you were
// looking at stays where it is. Press anywhere on the track and the window
// comes to you. That is After Effects' navigator, because it is the gesture
// people arrive already knowing.
//
// **What it is not** is a second time ruler. It carries no numbers, no ticks,
// no markers and no work area: everything it draws is about *the view*, and a
// strip that also drew the document would be a ruler at the wrong scale
// competing with the real one two pixels below it.
//
// The look is the ruler band's own: the lane ground it stands on, a hairline
// under it like every band in the panel, and a window edged with the same drawn
// tabs the work-area handles use — a step stronger under the pointer.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../widgets/controls.dart' show ThemeScope;
import 'timeline_extras_frb.dart' show TimelineAxis;

/// The visible span, in frames, that a scroll position implies.
///
/// Pure, and derived from the scroll's OWN numbers rather than from the zoom:
/// `content` is the whole scrollable width (`viewportDimension +
/// maxScrollExtent`), which is the one measurement that is never out of step
/// with the offset it is read beside. Working the window out from the
/// magnification instead means reading a zoom mid-flight against an offset
/// layout has not corrected yet, and the window jitters through every zoom.
///
/// [pad] is the axis's own padding at each end of the content, which the frames
/// do not occupy ([TimelineAxis.pad]).
({double start, double end}) navigatorWindow({
  required double offset,
  required double viewport,
  required double content,
  required int frames,
  double pad = TimelineAxis.pad,
}) {
  final span = content - pad * 2;
  if (frames <= 0 || span <= 0) return (start: 0, end: frames.toDouble());
  final perFrame = span / frames;
  final start = (offset - pad) / perFrame;
  final end = (offset + viewport - pad) / perFrame;
  // Fit-to-panel shows the whole comp and a little of the padding either side,
  // so the raw numbers run slightly outside it. The window is a statement about
  // the composition and cannot leave it.
  return (
    start: start.clamp(0.0, frames.toDouble()),
    end: end.clamp(0.0, frames.toDouble()),
  );
}

/// What a drag on the navigator asks the lanes for: where the window should
/// start, and how many frames wide it should be.
///
/// Pure so the three gestures are one piece of arithmetic rather than three.
/// [grab] says which part of the window was taken hold of. A window is never
/// allowed off either end of the composition, and never narrower than
/// [minSpan] — a window of no width is a view of no frames, and the zoom it
/// implies is a division by nothing.
///
/// [hold] is how far into the window the pointer took hold of it, in frames,
/// and it is what makes a pan *relative*: the frame under the pointer stays
/// under the pointer for the whole gesture. Centring the window on the pointer
/// instead would mean the window jumping the moment it was grabbed anywhere but
/// exactly its middle. A press on the bare track has no such frame to keep, so
/// the caller passes half the span and the window arrives centred.
({double start, double span}) navigatorDrag({
  required NavigatorGrab grab,
  required double frame,
  required double start,
  required double end,
  required int frames,
  double hold = 0,
  double minSpan = 1,
}) {
  final total = frames.toDouble();
  final span = (end - start).clamp(minSpan, total);
  return switch (grab) {
    // The window travels; its width is the drag's to leave alone.
    NavigatorGrab.body => (
        start: (frame - hold).clamp(0.0, (total - span).clamp(0.0, total)),
        span: span,
      ),
    // The far end stays where it is, so what the eye was on does not move.
    NavigatorGrab.start => () {
        final s = frame.clamp(0.0, end - minSpan);
        return (start: s, span: end - s);
      }(),
    NavigatorGrab.end => () {
        final e = frame.clamp(start + minSpan, total);
        return (start: start, span: e - start);
      }(),
  };
}

/// Which part of the window a press landed on.
enum NavigatorGrab { start, end, body }

/// The strip itself.
///
/// It stands over the lane area alone, above the ruler; [trailing] is the
/// width it leaves blank over the lanes' scroll gutter. The two halves are one
/// table, so the outline spends exactly this widget's [band] growing its
/// toolbar row to the panel top (the owner's ruling): the strip first
/// spanned the whole panel and stood blank over the outline, which read as a
/// sliver of dead ground above the timecode row. Either way the halves spend
/// the same height above the chrome pair the ruler is derived from, which is
/// what keeps every row level with its own name.
class TimelineNavigator extends StatefulWidget {
  const TimelineNavigator({
    super.key,
    required this.trailing,
    required this.frames,
    required this.zoom,
    required this.hScroll,
    required this.playhead,
    required this.onWindow,
    this.onWindowEnd,
  });

  final double trailing;

  /// The composition's length. A comp of no frames draws an empty track.
  final int frames;

  /// The lane side's magnification and scroll — listened to, so the window
  /// follows a zoom flight and a scroll without the panel rebuilding.
  final Listenable zoom;
  final ScrollController hScroll;
  final ValueListenable<int> playhead;

  /// The window a gesture is asking for, in frames. The panel turns it into a
  /// magnification and an anchored offset: what the view *is* belongs to the
  /// panel that owns the zoom, not to the strip that draws it.
  final void Function(double start, double span) onWindow;

  /// The gesture ended — the panel's cue to let go of the zoom anchor it held
  /// for the length of it.
  final VoidCallback? onWindowEnd;

  /// The bar's own height. Slim on purpose: it is a readout with a grip, not a
  /// band of content, and the ruler under it is what carries the clock.
  static const double height = 11;

  /// The whole band the strip occupies — its own height plus the hairline it
  /// closes with. What anything measuring down the panel has to allow for.
  static const double band = height + 1;

  /// How near an end of the window counts as taking hold of that end — the
  /// work-area handle's own reach, so the two grips feel the same.
  static const double handleGrab = 10;

  @override
  State<TimelineNavigator> createState() => _TimelineNavigatorState();
}

class _TimelineNavigatorState extends State<TimelineNavigator> {
  /// What the gesture in flight took hold of, `null` between gestures.
  NavigatorGrab? _grab;

  /// What the pointer is over at rest, so the grip it would take is the one
  /// that lights.
  NavigatorGrab? _hover;

  ({double start, double end}) get _window {
    // Three ways there is nothing yet to describe, and all three are ordinary.
    // **No client, or two**: the lanes are mid-rebuild, and the controller is
    // briefly attached to the outgoing view and the incoming one at once —
    // `position` asserts on exactly that, which is why the panel reads scroll
    // positions through the same count check. **No dimensions**: attached is
    // not laid out, and the strip is built in the same pass as the lanes it
    // describes, so on the first one there is a client with nothing measured.
    // The honest answer to all three is the whole composition, which is also
    // what the window will be: the first layout is fit-to-panel.
    final positions = widget.hScroll.positions;
    final position = positions.length == 1 ? positions.first : null;
    if (position == null ||
        !position.hasViewportDimension ||
        !position.hasContentDimensions) {
      return (start: 0, end: widget.frames.toDouble());
    }
    return navigatorWindow(
      offset: position.pixels,
      viewport: position.viewportDimension,
      content: position.viewportDimension + position.maxScrollExtent,
      frames: widget.frames,
    );
  }

  /// Which part of the window `x` — in the bar's own pixels — lands on.
  NavigatorGrab _grabAt(double x, TimelineAxis axis) {
    final window = _window;
    const half = TimelineNavigator.handleGrab / 2;
    if ((x - axis.xOf(window.start)).abs() <= half) return NavigatorGrab.start;
    if ((x - axis.xOf(window.end)).abs() <= half) return NavigatorGrab.end;
    return NavigatorGrab.body;
  }

  /// How far into the window the pointer took hold of it, in frames — see
  /// [navigatorDrag]'s `hold`. Chosen once, at the press, and kept for the
  /// gesture: re-measuring it per update against a window the last update has
  /// already moved is how a drag drifts away from the pointer.
  double _hold = 0;

  /// A press: decide what was taken hold of, and act on it at once — a press on
  /// the bare track brings the window there, which is the same gesture
  /// continued as a drag.
  void _press(double x, TimelineAxis axis) {
    final window = _window;
    final frame = axis.frameAtExact(x);
    final grab = _grabAt(x, axis);
    final inside = frame >= window.start && frame <= window.end;
    _hold = grab == NavigatorGrab.body && inside
        ? frame - window.start
        : (window.end - window.start) / 2;
    setState(() => _grab = grab);
    _ask(grab, x, axis);
  }

  void _ask(NavigatorGrab grab, double x, TimelineAxis axis) {
    final window = _window;
    final asked = navigatorDrag(
      grab: grab,
      frame: axis.frameAtExact(x),
      start: window.start,
      end: window.end,
      frames: widget.frames,
      hold: _hold,
    );
    widget.onWindow(asked.start, asked.span);
  }

  MouseCursor get _cursor => switch (_grab ?? _hover) {
        NavigatorGrab.start ||
        NavigatorGrab.end =>
          SystemMouseCursors.resizeLeftRight,
        NavigatorGrab.body => SystemMouseCursors.grab,
        null => MouseCursor.defer,
      };

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return SizedBox(
      height: TimelineNavigator.band,
      child: Column(
        children: [
          Expanded(
            child: Row(
              children: [
                // Blank over the lanes' gutter: the strip draws only where the
                // time it is describing is drawn.
                Expanded(
                  child: LayoutBuilder(
                    builder: (context, constraints) {
                      final axis = TimelineAxis(
                          frames: widget.frames, width: constraints.maxWidth);
                      return _surface(t, axis);
                    },
                  ),
                ),
                SizedBox(width: widget.trailing),
              ],
            ),
          ),
          // The hairline every band in the panel closes with.
          SizedBox(height: 1, child: ColoredBox(color: t.hairline)),
        ],
      ),
    );
  }

  Widget _surface(LumitTheme t, TimelineAxis axis) => MouseRegion(
        cursor: _cursor,
        onHover: (e) {
          final over = _grabAt(e.localPosition.dx, axis);
          if (over != _hover) setState(() => _hover = over);
        },
        onExit: (_) {
          if (_hover != null) setState(() => _hover = null);
        },
        child: GestureDetector(
          key: const ValueKey('tl-navigator'),
          behavior: HitTestBehavior.opaque,
          // The trackpad is excluded the way every other editing recogniser in
          // the panel excludes it: a two-finger scroll is the panel's to pan
          // with, and a recogniser here would take it.
          supportedDevices: const {
            PointerDeviceKind.mouse,
            PointerDeviceKind.touch,
            PointerDeviceKind.stylus,
            PointerDeviceKind.invertedStylus,
            PointerDeviceKind.unknown,
          },
          // **One recogniser, not a tap and a drag.** The press acts on the
          // pointer going *down* rather than on a tap being completed, so a
          // click on the bare track and a drag from it are the same gesture
          // caught at the same moment; a tap recogniser beside the drag would
          // have to be told which of the two won, and would cancel the drag's
          // press when it lost.
          onHorizontalDragDown: (d) => _press(d.localPosition.dx, axis),
          onHorizontalDragUpdate: (d) {
            final grab = _grab;
            if (grab != null) _ask(grab, d.localPosition.dx, axis);
          },
          onHorizontalDragEnd: (_) => _end(),
          onHorizontalDragCancel: _end,
          child: RepaintBoundary(
            // Its own layer: the window follows a zoom flight and the
            // playhead follows playback, and neither may repaint the lanes
            // beside it.
            child: ListenableBuilder(
              listenable: Listenable.merge(
                  [widget.zoom, widget.hScroll, widget.playhead]),
              builder: (context, _) {
                final window = _window;
                return CustomPaint(
                  size: Size.infinite,
                  painter: _NavigatorPainter(
                    axis: axis,
                    start: window.start,
                    end: window.end,
                    playhead: widget.playhead.value.toDouble(),
                    ground: t.timelineOutOfRange,
                    fill: t.surface2,
                    edge: t.hairline,
                    handle: t.hairlineStrong,
                    handleLit: t.textMuted,
                    accent: t.accent,
                    lit: _grab ?? _hover,
                  ),
                );
              },
            ),
          ),
        ),
      );

  void _end() {
    if (_grab != null) setState(() => _grab = null);
    widget.onWindowEnd?.call();
  }
}

class _NavigatorPainter extends CustomPainter {
  const _NavigatorPainter({
    required this.axis,
    required this.start,
    required this.end,
    required this.playhead,
    required this.ground,
    required this.fill,
    required this.edge,
    required this.handle,
    required this.handleLit,
    required this.accent,
    required this.lit,
  });

  final TimelineAxis axis;
  final double start, end, playhead;
  final Color ground, fill, edge, handle, handleLit, accent;
  final NavigatorGrab? lit;

  /// The tab at each end of the window: narrow, full height, rounded — the
  /// work-area handle's shape at this band's scale.
  static const double _tab = 3;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = ground);

    final x0 = axis.xOf(start);
    final x1 = axis.xOf(end);
    final window = RRect.fromRectAndRadius(
      Rect.fromLTRB(x0, 0.5, x1, size.height - 0.5),
      const Radius.circular(2),
    );
    canvas.drawRRect(window, Paint()..color = fill);
    canvas.drawRRect(
      window,
      Paint()
        ..color = edge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );

    for (final (at, which) in [
      (x0, NavigatorGrab.start),
      (x1, NavigatorGrab.end)
    ]) {
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(at - _tab / 2, 1, _tab, size.height - 2),
          const Radius.circular(1.5),
        ),
        Paint()..color = lit == which ? handleLit : handle,
      );
    }

    // The playhead, over the window rather than under it: where you are is the
    // one thing on this strip that must never be hidden by the rest of it.
    final px = axis.xOf(playhead);
    canvas.drawRect(
      Rect.fromLTWH(px - 0.5, 0, 1, size.height),
      Paint()..color = accent,
    );
  }

  @override
  bool shouldRepaint(_NavigatorPainter old) =>
      old.start != start ||
      old.end != end ||
      old.playhead != playhead ||
      old.lit != lit ||
      old.axis.width != axis.width ||
      old.axis.frames != axis.frames ||
      old.ground != ground ||
      old.fill != fill ||
      old.edge != edge ||
      old.handle != handle ||
      old.handleLit != handleLit ||
      old.accent != accent;
}
