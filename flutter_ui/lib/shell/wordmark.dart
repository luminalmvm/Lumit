// The Lumit wordmark (K-480).
//
// In plain terms: this draws the word "lumit" exactly as the website draws it —
// the `l` and the `t` are the mark's two keys, a blue one and a violet one, and
// `umi` between them is lettering that takes the colour of whatever it is
// standing on. It is a picture, not a line of type: the keys are drawn shapes,
// and the `t` is the `l` turned upside down about the lockup's centre, which is
// the whole idea of the mark.
//
// **It is the site's own file**, `web/public/lumit-wordmark.svg`, copied into
// `assets/brand/` and drawn through flutter_svg — the same route Lumit's icons
// take (icons/lumit_icon.dart). Three deliberate differences, and nothing else:
//
// 1. the view box is tightened to the lockup itself, so a caller asking for 22
//    logical pixels gets 22 pixels of ink rather than 22 of mostly margin;
// 2. the three letter paths are filled `currentColor` instead of the site's
//    near-white, which is what lets them follow the theme (see below);
// 3. the glow ellipse — drawn at zero scale and zero opacity, a leftover of the
//    animated version — is gone.
//
// **The keys never change colour and the letters always do.** The keys are the
// brand (theme/brand.dart); a wordmark whose blue went green under a custom
// theme would not be the wordmark. The letters are only lettering, and lettering
// that cannot be read is worse than lettering in the wrong grey — so they are
// chosen against the surface the mark is standing on: dark letters on a light
// ground, light on a dark one, and light when there is no ground to judge.

import 'package:flutter/widgets.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../theme/brand.dart';

/// Where the drawing lives.
const String lumitWordmarkAsset = 'assets/brand/lumit-wordmark.svg';

/// The lockup's own box — the `l`'s left edge to the `t`'s right, the cap line
/// down to the `u`'s overshoot — as width ÷ height, so a caller need only say
/// how tall the mark should be.
const double lumitWordmarkAspect = 253.82 / 71.55;

/// Which way `umi` goes on [ground].
///
/// Relative luminance, the ordinary sRGB one Flutter already computes, against
/// the middle of the range: over half means a light surface, which wants dark
/// letters. A null ground is a surface nobody can judge — an unfinished custom
/// theme, a caller with nothing to say — and takes the light letters, because
/// Lumit is dark-first and light lettering is the mark as it is usually seen.
Color wordmarkLetters(Color? ground) => (ground?.computeLuminance() ?? 0) > 0.5
    ? brandWordmarkInk
    : brandWordmarkPaper;

/// The wordmark, [height] logical pixels from its cap line to the `u`'s
/// overshoot; the width follows from the lockup.
///
/// [ground] is the surface it is standing on, which decides the lettering.
class LumitWordmark extends StatelessWidget {
  final double height;
  final Color? ground;

  const LumitWordmark({super.key, required this.height, this.ground});

  /// The colour `umi` will be drawn in. Public so the screens that place the
  /// mark — and the tests that check them — can ask without rendering.
  Color get letters => wordmarkLetters(ground);

  @override
  Widget build(BuildContext context) => SvgPicture.asset(
        lumitWordmarkAsset,
        height: height,
        width: height * lumitWordmarkAspect,
        // The letters are the only part of the drawing that inherits: the two
        // keys carry their own gradients.
        theme: SvgTheme(currentColor: letters),
        semanticsLabel: 'Lumit',
      );
}
