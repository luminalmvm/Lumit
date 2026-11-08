// A node canvas with its cut edges faded: a box that runs past the panel's
// edge fades out there rather than stopping dead. Only an edge a box crosses
// fades, so a graph that fits is drawn plain.

import 'package:flutter/widgets.dart';

import '../widgets/controls.dart' show ThemeScope;

class GraphCutFade extends StatelessWidget {
  final bool left;
  final bool right;
  final bool top;
  final bool bottom;
  final Widget child;

  /// How far in from a cut edge the fade reaches.
  static const double reach = 24;

  const GraphCutFade({
    super.key,
    required this.left,
    required this.right,
    required this.top,
    required this.bottom,
    required this.child,
  });

  /// Which edges of [viewport] the [boxes] cross, with the view at [pan] and
  /// [zoom]. Canvas units in, screen edges out.
  static ({bool left, bool right, bool top, bool bottom}) cuts(
    Size viewport,
    Iterable<Rect> boxes,
    Offset pan,
    double zoom,
  ) {
    var left = false;
    var right = false;
    var top = false;
    var bottom = false;
    for (final box in boxes) {
      final r = Rect.fromLTRB(
        box.left * zoom + pan.dx,
        box.top * zoom + pan.dy,
        box.right * zoom + pan.dx,
        box.bottom * zoom + pan.dy,
      );
      if (r.bottom < 0 || r.top > viewport.height) continue;
      if (r.left < 0 && r.right > 0) left = true;
      if (r.left < viewport.width && r.right > viewport.width) right = true;
      if (r.right < 0 || r.left > viewport.width) continue;
      if (r.top < 0 && r.bottom > 0) top = true;
      if (r.top < viewport.height && r.bottom > viewport.height) bottom = true;
    }
    return (left: left, right: right, top: top, bottom: bottom);
  }

  @override
  Widget build(BuildContext context) {
    // Only the alpha of the mask counts, so any opaque theme colour will do.
    final ink = ThemeScope.of(context).theme.textPrimary;
    final clear = ink.withValues(alpha: 0);
    Widget out = child;
    if (left || right) {
      out = ShaderMask(
        blendMode: BlendMode.dstIn,
        shaderCallback: (bounds) {
          final edge = (reach / bounds.width).clamp(0.0, 0.5);
          return LinearGradient(
            colors: [left ? clear : ink, ink, ink, right ? clear : ink],
            stops: [0, edge, 1 - edge, 1],
          ).createShader(bounds);
        },
        child: out,
      );
    }
    if (top || bottom) {
      out = ShaderMask(
        blendMode: BlendMode.dstIn,
        shaderCallback: (bounds) {
          final edge = (reach / bounds.height).clamp(0.0, 0.5);
          return LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: [top ? clear : ink, ink, ink, bottom ? clear : ink],
            stops: [0, edge, 1 - edge, 1],
          ).createShader(bounds);
        },
        child: out,
      );
    }
    return out;
  }
}
