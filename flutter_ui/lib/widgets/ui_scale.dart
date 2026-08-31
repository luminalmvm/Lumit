// UI scale (Settings → Interface → UI scale, K-117): the whole interface draws
// larger or smaller relative to the display's native scale, exactly as egui's
// `ctx.set_pixels_per_point(scale)` does — layout AND hit-testing move together.
//
// Why this mechanism (recorded in docs/archive/flutter-port/05, and the one the ledger
// asked to choose):
//   * A `MediaQuery.devicePixelRatio` override does NOT work — the render tree
//     reads the real `FlutterView.devicePixelRatio` directly, so overriding it
//     in MediaQuery only misleads asset-resolution and changes nothing visible.
//   * The genuine pipeline scale (`RenderView.configuration.devicePixelRatio`)
//     is only reachable through the experimental multi-window `View` API, which
//     the stable SDK gated behind an `@internal`, feature-flagged surface we
//     cannot touch without failing `flutter analyze` when this was chosen
//     (3.44.7; the pin is 3.47.1 now and this has not been re-tried).
//   * `Transform.scale` DOES carry through hit-testing (RenderTransform applies
//     the inverse matrix to pointer events), so scaling paint + pointers is
//     coherent. Its one trap is layout: a bare `Transform.scale` lays the child
//     out at the UNSCALED size and paints it overflowing. We remove that trap by
//     giving the child constraints of `logical = physical / scale` (via an
//     `OverflowBox`), so it lays out at the scaled logical size and, once
//     Transform multiplies by `scale`, fills the window exactly.
// Glyphs stay crisp: Transform is applied to the vector draw ops at
// rasterisation, not to a pre-rendered bitmap, so text does not soften.
//
// Commit-on-release is handled upstream by the settings slider (K-117): this
// widget just reflects whatever `scale` the workspace currently holds.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/state/settings.dart' show effectiveUiScale;

class UiScaleView extends StatelessWidget {
  /// The user's own factor (Settings → Interface → Scale). What is drawn is
  /// this over the presentation baseline — see [effectiveUiScale] (K-560).
  final double scale;
  final Widget child;

  const UiScaleView({super.key, required this.scale, required this.child});

  @override
  Widget build(BuildContext context) {
    // The baseline (K-560) means the shipped 100% is a real scale of 1.1, so
    // the pass-through is now the rare case rather than the common one: it is
    // the *effective* scale that has to be 1 for there to be nothing to do.
    // A user who scales back down to the old size gets it, as does anything
    // mounting this view at an effective 1× — a Transform in the tree that
    // multiplies by one is only a matrix to invert on every pointer.
    final drawn = effectiveUiScale(scale);
    if ((drawn - 1.0).abs() < 1e-3) return child;

    return LayoutBuilder(
      builder: (context, constraints) {
        // Without a bounded box there is nothing to scale into — fall back to
        // the child untouched rather than force an infinite logical size.
        if (!constraints.hasBoundedWidth || !constraints.hasBoundedHeight) {
          return child;
        }
        final w = constraints.maxWidth;
        final h = constraints.maxHeight;
        final lw = w / drawn;
        final lh = h / drawn;

        // Correct MediaQuery.size for descendants that read it, when one exists
        // above us (the app runs without a WidgetsApp, so guard with maybeOf).
        final mq = MediaQuery.maybeOf(context);
        Widget scaled = SizedBox(width: lw, height: lh, child: child);
        if (mq != null) {
          scaled = MediaQuery(
            data: mq.copyWith(size: Size(lw, lh)),
            child: scaled,
          );
        }

        return Transform.scale(
          scale: drawn,
          alignment: Alignment.topLeft,
          child: OverflowBox(
            alignment: Alignment.topLeft,
            minWidth: lw,
            maxWidth: lw,
            minHeight: lh,
            maxHeight: lh,
            child: scaled,
          ),
        );
      },
    );
  }
}
