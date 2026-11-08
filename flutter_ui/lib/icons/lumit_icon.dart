// Draws one glyph of Lumit's own icon set (docs/15-DESIGN.md §5).
//
// The glyphs are inline SVG documents in lumit_icons.dart, generated from
// tool/icons/glyphs.json. Each is drawn in `currentColor`, so a glyph takes
// the text colour of wherever it sits — text_secondary at rest,
// text_primary on hover, accent when active — exactly as a word would.

import 'package:flutter/widgets.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../widgets/controls/base.dart' show ThemeScope;
import 'icon_style.dart';
import 'lumit_icons.dart';

class LumitIcon extends StatelessWidget {
  /// One of the [LumitIcons] constants — never a looked-up string, so a call
  /// site cannot misspell a glyph.
  final String glyph;

  /// Side of the square box, in logical pixels. 16 is the floor the set is
  /// drawn to, not a preference.
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
    // Restyled to the weight in force, or replaced by the person's own file.
    // A glyph drawn outside any theme (a test, a bare probe) is the set's.
    final scope = ThemeScope.maybeOf(context);
    var svg = scope == null ? glyph : IconStyle.resolve(glyph, scope.theme);
    // A file in colours of its own keeps them. Only its `currentColor` parts
    // follow the tint, and it greys out where a tinted icon would dim.
    final own = scope != null && IconStyle.keepsColours(svg);
    if (own) svg = IconStyle.themed(svg, scope.theme);
    final picture = SvgPicture.string(
      svg,
      width: size,
      height: size,
      theme: own && tint != null ? SvgTheme(currentColor: tint) : null,
      colorFilter: own
          ? (tint == scope.theme.textDisabled ? _greyed : null)
          : tint == null
              ? null
              : ColorFilter.mode(tint, BlendMode.srcIn),
      semanticsLabel: semanticLabel,
      excludeFromSemantics: semanticLabel == null,
    );
    // A glyph that points the way the line reads turns round right to left.
    return _mirrored.contains(glyph) &&
            Directionality.maybeOf(context) == TextDirection.rtl
        ? Transform.flip(flipX: true, child: picture)
        : picture;
  }
}

const _mirrored = {LumitIcons.expand, LumitIcons.undo, LumitIcons.redo};

/// Grey, at less than half strength: a coloured icon on a control that
/// can't be pressed.
const _greyed = ColorFilter.matrix([
  0.2126, 0.7152, 0.0722, 0, 0, //
  0.2126, 0.7152, 0.0722, 0, 0, //
  0.2126, 0.7152, 0.0722, 0, 0, //
  0, 0, 0, 0.4, 0,
]);
