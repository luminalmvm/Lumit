// The boot splash (splash.rs): the app opens as a small centred card
// listing each module as it comes up, then gives way to the application.
// With a live bridge the lines are the engine's OWN boot log (library version,
// ABI, the compiled feature set — `app.bootLog()`, honoured here); the F0
// placeholder build (no bridge) falls back to the canned chrome start-up steps.
//
// Driven by one AnimationController rather than timers, so tests can
// pumpAndSettle through it and nothing is left pending.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../src/rust/api/state.dart' show OpenPhase;
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';

/// The line the opening card shows for one phase of a project open.
///
/// The engine names the phase and Lumit says it in the reader's language — the
/// engine sends no English here, which is why there is nothing for
/// `engine_labels.dart` to carry.
String openPhaseLabel(OpenPhase phase) => switch (phase) {
      OpenPhase.readingFile => l10n.openingReadingFile,
      OpenPhase.readingPackedMedia => l10n.openingReadingPackedMedia,
      OpenPhase.resolvingMedia => l10n.openingResolvingMedia,
      OpenPhase.preparingProject => l10n.openingPreparingProject,
      OpenPhase.startingPreview => l10n.openingStartingPreview,
    };

/// The fallback boot lines shown without an engine bridge (the F0 placeholder
/// build). A live bridge replaces these with `app.bootLog()`.
const List<String> bootLines = [
  'workspace store',
  'theme',
  'icon pack',
  'shell',
];

/// The card shown while a document is being read off disk.
///
/// Its job is what it *hides*: opening a project replaces the whole document,
/// and the panels behind this are still drawing the previous one. Rather than
/// letting them empty out panel by panel as the new document arrives, the shell
/// leaves whatever was on screen standing and covers it with this until the new
/// project is adopted — one swap, no half-loaded interface.
///
/// The bar fills when the job can say how far it has got and sweeps when it
/// cannot. Opening a `.lum` can: the engine names each phase of the
/// read as it begins, and [fraction] is the share of the whole open behind it,
/// drawn as a percentage beside the phase's own line. A job that reports
/// nothing passes null and gets the sweep, which claims nothing.
///
/// **A card keeps whichever bar it opened with.** The two are not swapped
/// mid-life — a sweep that turned into a fill a moment after appearing would
/// read as a stumble — so a caller that will report progress reports it from
/// the first frame.
///
/// [label] names what is being waited for. It defaults to opening a project,
/// which is what the card was built for; a job that takes the same seconds and
/// wants the same "hands off, this is working" reads its own line instead — see
/// [BusyOverlay].
class OpeningOverlay extends StatefulWidget {
  final String? label;

  /// How far the job has got, 0..1, or null for one that cannot say.
  final double? fraction;

  /// Stops the job, for one that can be stopped: the card then carries a
  /// Cancel button, the one thing on it that takes a click.
  final VoidCallback? onCancel;

  /// What the Cancel button says. Null is the plain word.
  final String? cancelLabel;

  const OpeningOverlay(
      {super.key, this.label, this.fraction, this.onCancel, this.cancelLabel});

  @override
  State<OpeningOverlay> createState() => _OpeningOverlayState();
}

class _OpeningOverlayState extends State<OpeningOverlay>
    with SingleTickerProviderStateMixin {
  /// Only ever built for the indeterminate card: a controller left repeating
  /// under a bar that is being told its own fill would keep the tree from ever
  /// settling, for an animation nothing draws.
  AnimationController? _sweep;

  VoidCallback? _escapeRelease;

  @override
  void initState() {
    super.initState();
    if (widget.fraction == null) {
      _sweep = AnimationController(
        vsync: this,
        duration: const Duration(milliseconds: 900),
      )..repeat();
    }
    // The card has the window, so Escape is its own: it presses Cancel where
    // there is one, and reaches nothing underneath either way.
    _escapeRelease = EscapeLadder.register(EscapeRung.dialog, () {
      widget.onCancel?.call();
      return true;
    });
  }

  @override
  void dispose() {
    _escapeRelease?.call();
    _sweep?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Nothing underneath is clickable while the document it belongs to is being
    // replaced, and the scrim is what says so. The card fades up rather than
    // cutting in, and the scrim takes every click meant for the shell from the
    // first frame either way. Absorbing is lifted only for the card's own
    // Cancel.
    return AbsorbPointer(
      absorbing: widget.onCancel == null,
      child: Entrance.fade(
        spec: ThemeScope.of(context).motion.swap,
        child: ColoredBox(
          color: t.scrim,
          child: Center(
            child: Container(
              width: 260,
              padding: const EdgeInsets.all(20),
              decoration: BoxDecoration(
                color: t.surface1,
                borderRadius: BorderRadius.circular(t.tokens.floatRadius),
                border: Border.all(color: t.hairline),
                boxShadow: t.floatShadow,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Expanded(
                        child: Text(widget.label ?? l10n.openingProject,
                            style: t.bodyPrimary),
                      ),
                      if (widget.fraction != null)
                        Text(
                          l10n.openingPercent(
                              (widget.fraction!.clamp(0.0, 1.0) * 100).round()),
                          style: t.small,
                        ),
                    ],
                  ),
                  const SizedBox(height: 12),
                  if (_sweep case final sweep?)
                    AnimatedBuilder(
                      animation: sweep,
                      builder: (context, _) =>
                          HouseProgressBar(fraction: sweep.value, eased: false),
                    )
                  else
                    HouseProgressBar(fraction: widget.fraction ?? 0),
                  if (widget.onCancel case final cancel?) ...[
                    const SizedBox(height: 14),
                    Align(
                      alignment: Alignment.centerRight,
                      child: HouseButton(
                        key: const ValueKey('busy-cancel'),
                        padding: const EdgeInsets.symmetric(horizontal: 12),
                        onPressed: cancel,
                        child: Text(widget.cancelLabel ?? l10n.cancel),
                      ),
                    ),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The opening card over the shell for any other job that takes seconds.
///
/// A job the interface must not be used during — beat detection is the first —
/// puts its line into [busy] while it runs and clears it after, and this shows
/// the same card the rest of the time it shows nothing.
///
/// [progress] is the fraction the job reports, where it reports one: beat
/// detection does, and its bar fills and says the percentage exactly as an open
/// does. A job with nothing to report leaves this out and keeps the sweep,
/// which claims nothing about work it cannot see.
///
/// [cancel] is how the job is stopped, where it can be: packing a project can,
/// and its card carries a Cancel button. It is read when the card goes up, and
/// so is [cancelLabel], which is what the button says.
class BusyOverlay extends StatelessWidget {
  final ValueListenable<String?> busy;
  final ValueListenable<double?>? progress;
  final ValueListenable<VoidCallback?>? cancel;
  final ValueListenable<String?>? cancelLabel;

  const BusyOverlay(
      {super.key,
      required this.busy,
      this.progress,
      this.cancel,
      this.cancelLabel});

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<String?>(
        valueListenable: busy,
        builder: (context, label, _) {
          if (label == null) return const SizedBox.shrink();
          final reported = progress;
          final onCancel = cancel?.value;
          final says = cancelLabel?.value;
          if (reported == null) {
            return OpeningOverlay(
                label: label, onCancel: onCancel, cancelLabel: says);
          }
          return ValueListenableBuilder<double?>(
            valueListenable: reported,
            builder: (context, fraction, _) => OpeningOverlay(
                label: label,
                fraction: fraction,
                onCancel: onCancel,
                cancelLabel: says),
          );
        },
      );
}

/// Run [job] with the card up, labelled [label], and take it down when the job
/// settles either way.
///
/// The card comes down on a failure as much as on a success: a job that ends in
/// nothing — no audio to find beats in — must not leave the shell covered.
Future<void> showBusyWhile(
  ValueNotifier<String?> busy,
  String label,
  Future<void> job,
) {
  busy.value = label;
  return job.whenComplete(() => busy.value = null);
}

class SplashOverlay extends StatefulWidget {
  final VoidCallback onDone;

  /// The engine's real boot log to stream, when a bridge supplied one. Null or
  /// empty falls back to the canned [bootLines] (the F0 promise: the real log
  /// streams here once the bridge is present).
  final List<String>? lines;

  const SplashOverlay({super.key, required this.onDone, this.lines});

  @override
  State<SplashOverlay> createState() => _SplashOverlayState();
}

class _SplashOverlayState extends State<SplashOverlay>
    with SingleTickerProviderStateMixin {
  static const _perLine = Duration(milliseconds: 180);
  static const _hold = Duration(milliseconds: 600);

  /// The lines actually shown: the engine's boot log when non-empty, else the
  /// canned fallback.
  late final List<String> _lines =
      (widget.lines != null && widget.lines!.isNotEmpty)
          ? widget.lines!
          : bootLines;

  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: _perLine * _lines.length + _hold,
  )
    ..addListener(() => setState(() {}))
    ..addStatusListener((status) {
      if (status == AnimationStatus.completed) widget.onDone();
    })
    ..forward();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  /// Lines shown so far: one more every 150 ms of the run.
  int get _shown {
    final total = _controller.duration!.inMilliseconds;
    final elapsed = _controller.value * total;
    return (elapsed / _perLine.inMilliseconds).floor().clamp(0, _lines.length);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Deliberately not clickable and fully opaque: the egui splash is the
    // window until boot ends, so nothing of the application shows through
    // and no input reaches it (owner feedback, 2026-07-21).
    return AbsorbPointer(
      child: ColoredBox(
        color: t.surface0,
        child: Center(
          child: Container(
            width: 300,
            padding: const EdgeInsets.all(20),
            decoration: BoxDecoration(
              color: t.surface1,
              borderRadius: BorderRadius.circular(t.tokens.floatRadius),
              border: Border.all(color: t.hairline),
              boxShadow: t.floatShadow,
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('Lumit', style: t.heading),
                const SizedBox(height: 2),
                Text(l10n.splashSubtitle, style: t.small),
                const SizedBox(height: 12),
                for (var i = 0; i < _shown; i++)
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 1),
                    child: Text(
                      _lines[i],
                      style: i == _shown - 1 &&
                              _controller.status != AnimationStatus.completed
                          ? t.bodyPrimary
                          : t.small,
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
