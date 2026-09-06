// The brand's own colours — the two keys of the Lumit mark (docs/15-DESIGN.md
// §11).
//
// In plain terms: everything else in this folder is a *theme* colour, which
// changes with the scheme somebody picked. These four do not. They are the mark
// as it is drawn on the website, on the application icon and on the document
// icons, and a wordmark that changed colour with the theme would stop being the
// wordmark. So they live beside the theme rather than inside it: named tokens,
// spelled out here because `flutter_ui/lib/theme/` is the one place a colour may
// be written as a number (docs/15-DESIGN.md §4.1).
//
// Each key is a gradient of two stops, and the pair of gradients is what the
// brand is: a green key on the left and a blue one on the right. It was a blue
// key and a violet-magenta one until the accent changed - the violet-magenta
// was the old accent's family and went green with it, and the two keys swapped
// sides so the lockup opens green and closes blue. The blue's own two values
// did not change. All four are the SVG sources' own, character for character
// (§11).

import 'dart:ui' show Color;

/// The green key, jade stop — the top of the `l`.
const Color brandKeyJade = Color(0xff6fdca8);

/// The green key, lime stop — the foot of the `l`.
const Color brandKeyLime = Color(0xffb6e84f);

/// The blue key, light stop — the top of the `t`, which is the green key turned
/// through 180°, so its gradient runs the other way.
const Color brandKeyBlueLight = Color(0xff86e2ff);

/// The blue key, deep stop.
const Color brandKeyBlue = Color(0xff2f6fe0);

/// The wordmark's lettering on a dark ground — the near-white the SVG sources
/// set `umi` in. Lumit is dark-first, so this is the mark as it is usually seen.
const Color brandWordmarkPaper = Color(0xfff4f6f8);

/// The wordmark's lettering on a light ground: the mark's own rim, which is the
/// dark end of the brand's palette (docs/15-DESIGN.md §11).
const Color brandWordmarkInk = Color(0xff0c0e14);
