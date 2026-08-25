// Draws one glyph of Lumit's own icon set (docs/15-DESIGN.md §5, K-440).
//
// The glyphs are inline SVG documents in lumit_icons.dart, generated from
// tool/icons/glyphs.json. Each is drawn in `currentColor`, so a glyph takes
// the text colour of wherever it sits — text_secondary at rest,
// text_primary on hover, accent when active — exactly as a word would.

import 'package:flutter/widgets.dart';
import 'package:flutter_svg/flutter_svg.dart';

class LumitIcon extends StatelessWidget {
  /// One of the [LumitIcons] constants — never a looked-up string, so a call
  /// site cannot misspell a glyph.
  final String glyph;

  /// Side of the square box, in logical pixels. 16 is the floor the set is
  /// drawn to, not a preference (K-209).
  final double size;

  /// Overrides the ambient colour. Left null, the glyph follows the
  /// surrounding text.
  final Color? colour;

  /// The chrome word this glyph stands for, for screen readers.
  final String? semanticLabel;

  const LumitIcon(
    this.glyph, {
    super.key,
    this.size = 16,
    this.colour,
    this.semanticLabel,
  });

  @override
  Widget build(BuildContext context) {
    final tint = colour ??
        DefaultTextStyle.of(context).style.color ??
        IconTheme.of(context).color;
    return SvgPicture.string(
      glyph,
      width: size,
      height: size,
      colorFilter:
          tint == null ? null : ColorFilter.mode(tint, BlendMode.srcIn),
      semanticsLabel: semanticLabel,
      excludeFromSemantics: semanticLabel == null,
    );
  }
}
