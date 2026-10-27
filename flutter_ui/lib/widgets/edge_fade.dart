// A sideways strip that fades out at whichever end has more to scroll, so a
// row of tabs cut off at its edge says there is more of it.

import 'package:flutter/widgets.dart';

import 'controls.dart';

class EdgeFade extends StatefulWidget {
  /// A horizontal scroll view.
  final Widget child;
  const EdgeFade({super.key, required this.child});

  @override
  State<EdgeFade> createState() => _EdgeFadeState();
}

class _EdgeFadeState extends State<EdgeFade> {
  bool _before = false;
  bool _after = false;

  bool _read(Notification said) {
    final metrics = switch (said) {
      ScrollMetricsNotification(:final metrics) => metrics,
      ScrollUpdateNotification(:final metrics) => metrics,
      _ => null,
    };
    if (metrics == null || metrics.axis != Axis.horizontal) return false;
    final before = metrics.extentBefore > 0.5;
    final after = metrics.extentAfter > 0.5;
    if (before != _before || after != _after) {
      setState(() {
        _before = before;
        _after = after;
      });
    }
    return false;
  }

  @override
  Widget build(BuildContext context) {
    // Only the alpha of the mask counts, so any opaque theme colour will do.
    final ink = ThemeScope.of(context).theme.textPrimary;
    final clear = ink.withValues(alpha: 0);
    // Always masked, even when nothing fades, so the scroll view under it is
    // never rebuilt and never loses its place.
    return ShaderMask(
      blendMode: BlendMode.dstIn,
      shaderCallback: (bounds) {
        final edge = (24 / bounds.width).clamp(0.0, 0.5);
        return LinearGradient(
          colors: [_before ? clear : ink, ink, ink, _after ? clear : ink],
          stops: [0, edge, 1 - edge, 1],
        ).createShader(bounds);
      },
      child: NotificationListener<Notification>(
          onNotification: _read, child: widget.child),
    );
  }
}
