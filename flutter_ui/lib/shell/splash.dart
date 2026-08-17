// The boot splash (K-008, splash.rs): the app opens as a small centred card
// listing each module as it comes up, then gives way to the application.
// With a live bridge the lines are the engine's OWN boot log (library version,
// ABI, the compiled feature set — `app.bootLog()`, honoured here); the F0
// placeholder build (no bridge) falls back to the canned chrome start-up steps.
//
// Driven by one AnimationController rather than timers, so tests can
// pumpAndSettle through it and nothing is left pending.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

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
/// The bar is indeterminate: reading a `.lum` reports no progress, so it sweeps
/// rather than claiming to know how far along it is.
class OpeningOverlay extends StatefulWidget {
  const OpeningOverlay({super.key});

  @override
  State<OpeningOverlay> createState() => _OpeningOverlayState();
}

class _OpeningOverlayState extends State<OpeningOverlay>
    with SingleTickerProviderStateMixin {
  late final AnimationController _sweep = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 900),
  )..repeat();

  @override
  void dispose() {
    _sweep.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Nothing underneath is clickable while the document it belongs to is being
    // replaced, and the scrim is what says so.
    return AbsorbPointer(
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
                Text(l10n.openingProject, style: t.bodyPrimary),
                const SizedBox(height: 12),
                AnimatedBuilder(
                  animation: _sweep,
                  builder: (context, _) =>
                      HouseProgressBar(fraction: _sweep.value),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
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
