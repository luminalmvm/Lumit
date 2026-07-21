// The boot splash (K-008, splash.rs): the app opens as a small centred card
// listing each module as it comes up, then gives way to the application.
// Phase F0 has no engine to boot, so the lines are the chrome's own start-up
// steps; the bridge's real boot log replaces them in F1. A click skips it.
//
// Driven by one AnimationController rather than timers, so tests can
// pumpAndSettle through it and nothing is left pending.

import 'package:flutter/widgets.dart';

import '../widgets/controls.dart';

/// The F0 boot lines. Real plumbing arrives with the bridge (F1) — the
/// engine's own boot log streams into this list then.
const List<String> bootLines = [
  'workspace store',
  'theme',
  'icon pack',
  'shell',
];

class SplashOverlay extends StatefulWidget {
  final VoidCallback onDone;
  const SplashOverlay({super.key, required this.onDone});

  @override
  State<SplashOverlay> createState() => _SplashOverlayState();
}

class _SplashOverlayState extends State<SplashOverlay>
    with SingleTickerProviderStateMixin {
  static const _perLine = Duration(milliseconds: 150);
  static const _hold = Duration(milliseconds: 400);

  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: _perLine * bootLines.length + _hold,
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
    return (elapsed / _perLine.inMilliseconds)
        .floor()
        .clamp(0, bootLines.length);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      // A click skips the boot card.
      onTap: () => _controller.value = 1.0,
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
                Text('Flutter frontend', style: t.small),
                const SizedBox(height: 12),
                for (var i = 0; i < _shown; i++)
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 1),
                    child: Text(
                      bootLines[i],
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
