// The Viewer's preview progress bar (docs/07 §2.5).
//
// **In plain terms.** When a frame takes long enough to notice, this says so: a
// slim bar just left of the reading on the Viewer's transport that fills as the
// engine works through the frame, with a word for what it is doing. When the
// frame arrives the bar goes.
//
// **Where it lives.** On the transport, not floating over the
// picture. Over the picture it covered the bottom of the composition — the one
// thing the Viewer exists to show — and it did so exactly while a frame was
// being waited on, which is when people are looking hardest. On the bar it has
// a place of its own: the controls take the space that is left, so the bar
// arriving and leaving moves none of them.
//
// **What it is not.** It is not a playback indicator — nothing appears while
// the transport is running (the engine sends no reports then), because a bar
// blinking at every frame of playback would be the busiest thing on screen and
// would say nothing anyone could act on. And it is not a spinner for every
// render: a frame that lands inside
// [PreviewProgressTracker.appearsAfter] shows nothing at all, so ordinary work
// stays silent and only a genuine wait speaks.
//
// **The motion.** The fill animates towards each report rather than jumping,
// so a bar that advances in five steps reads as one movement; and while it is
// waiting it carries a slow sheen, which is what distinguishes "working" from
// "stuck" at a glance. The sheen is a loop, so it runs at Full only, and the
// fill moves as a state mark does, so at None the bar simply sits at its
// reported fraction.

import 'package:flutter/widgets.dart';

import '../state/preview_progress.dart';
import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';

/// How tall the bar's own track is.
const double _trackHeight = 3;

/// One sweep of the sheen along the fill.
const Duration _sheenPeriod = Duration(milliseconds: 1400);

/// How wide the track is on the transport. Fixed, so the percentage beside it
/// never moves as the fill grows.
const double _trackWidth = 64;

/// How much room the stage name gets. The longest of them ("Reading the
/// composition") is longer than this and is shortened with an ellipsis rather
/// than being allowed to push the bar about.
const double _labelWidth = 110;

class ViewerProgressBar extends StatefulWidget {
  final PreviewProgressTracker tracker;

  const ViewerProgressBar({super.key, required this.tracker});

  @override
  State<ViewerProgressBar> createState() => _ViewerProgressBarState();
}

class _ViewerProgressBarState extends State<ViewerProgressBar>
    with SingleTickerProviderStateMixin {
  /// Built here rather than lazily: a `late final` controller that is never
  /// touched while the bar stays hidden would be *created* by its own
  /// `dispose`, which looks up the ticker mode on a widget already going away.
  late final AnimationController _sheen;

  @override
  void initState() {
    super.initState();
    _sheen = AnimationController(vsync: this, duration: _sheenPeriod);
    widget.tracker.addListener(_onProgress);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _runSheen();
  }

  @override
  void didUpdateWidget(ViewerProgressBar old) {
    super.didUpdateWidget(old);
    if (old.tracker != widget.tracker) {
      old.tracker.removeListener(_onProgress);
      widget.tracker.addListener(_onProgress);
    }
  }

  @override
  void dispose() {
    widget.tracker.removeListener(_onProgress);
    _sheen.dispose();
    super.dispose();
  }

  void _onProgress() {
    if (!mounted) return;
    _runSheen();
    setState(() {});
  }

  /// The sheen runs only while a bar is on screen, and only at Full: an
  /// animation ticking behind a widget that is not drawn is a frame's work per
  /// frame for nothing, and — as the frame-rate readout found the hard way —
  /// it also stops the interface ever settling, which hangs every widget test
  /// that waits for it to.
  void _runSheen() {
    final loops = widget.tracker.visible &&
        (context.getInheritedWidgetOfExactType<ThemeScope>()?.motion.carries ??
            false);
    if (loops && !_sheen.isAnimating) {
      _sheen.repeat();
    } else if (!loops && _sheen.isAnimating) {
      _sheen.stop();
    }
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final tracker = widget.tracker;
    // Nothing to say: an empty box rather than a transparent bar, so the
    // Viewer's own hit testing is untouched while no frame is being waited on.
    if (!tracker.visible) return const SizedBox.shrink();
    final motion = scope.motion;

    return IgnorePointer(
      child: Padding(
        padding: const EdgeInsets.only(right: 12),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              width: _labelWidth,
              child: Text(
                tracker.label,
                style: t.small,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                softWrap: false,
                textAlign: TextAlign.right,
              ),
            ),
            const SizedBox(width: 6),
            // Its own layer, so the sheen repaints the track and not the
            // transport round it.
            RepaintBoundary(
              child: ClipRRect(
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                child: SizedBox(
                  width: _trackWidth,
                  height: _trackHeight,
                  child: Stack(
                    children: [
                      Positioned.fill(
                        child: ColoredBox(color: t.surface2),
                      ),
                      Positioned.fill(
                        child: Align(
                          alignment: Alignment.centerLeft,
                          child: _fill(t, motion, tracker.fraction),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
            const SizedBox(width: 6),
            // A fixed slot for the percentage: it counts up while the frame is
            // being waited for, and a number that resized itself as it counted
            // would jog the whole bar (the same rule the clock readouts keep).
            SizedBox(
              width: monoSlotWidth(t.small, 4),
              child: Text(
                '${(tracker.fraction * 100).round()}%',
                style: t.small,
                maxLines: 1,
                textAlign: TextAlign.right,
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// The filled part, with the sheen travelling along it.
  Widget _fill(LumitTheme t, Motion motion, double fraction) {
    final sheen = motion.carries;
    final bar = LayoutBuilder(
      builder: (context, constraints) => AnimatedBuilder(
        animation: _sheen,
        builder: (context, _) {
          final width = constraints.maxWidth;
          return DecoratedBox(
            decoration: BoxDecoration(
              color: t.accent,
              gradient: !sheen
                  ? null
                  : LinearGradient(
                      // The sheen is one brighter band sliding from the left
                      // edge of the fill to its right, and it is the whole of
                      // the "it is still working" signal.
                      begin: Alignment(-1 + 2 * _sheen.value, 0),
                      end: Alignment(-0.4 + 2 * _sheen.value, 0),
                      colors: [t.accent, t.accentHover, t.accent],
                      stops: const [0.0, 0.5, 1.0],
                    ),
            ),
            child: SizedBox(width: width, height: _trackHeight),
          );
        },
      ),
    );
    return motion.mark.isStill
        ? FractionallySizedBox(
            widthFactor: fraction.clamp(0.0, 1.0), child: bar)
        : TweenAnimationBuilder<double>(
            tween: Tween<double>(end: fraction.clamp(0.0, 1.0)),
            duration: motion.mark.duration,
            curve: motion.mark.curve,
            builder: (context, value, child) =>
                FractionallySizedBox(widthFactor: value, child: child),
            child: bar,
          );
  }
}
