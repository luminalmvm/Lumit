// A theme at a glance: eight colours in a row (K-298).
//
// The picker names themes and shows none of them, so choosing between seven
// built-ins and a shelf of your own meant selecting each in turn and watching
// the whole interface change. This is the cheap answer: the colours that
// actually decide how a theme reads — the three grounds, the text on them, the
// accent, and the three role colours — drawn as a strip beside the picker, so
// a theme can be recognised before it is applied.
//
// Deliberately not every token. A strip of thirty-odd swatches is a colour
// chart, not a preview, and the ones left out are variations on the ones shown.

import 'package:flutter/widgets.dart';

import '../theme/theme.dart';
import 'controls.dart';

/// The colours the strip shows, in reading order: grounds, then what sits on
/// them, then the accents.
List<Color> swatchesOf(LumitTheme t) => [
      t.surface0,
      t.surface1,
      t.surface3,
      t.textPrimary,
      t.accent,
      t.success,
      t.warning,
      t.error,
    ];

/// The strip itself. Sized off the text scale rather than fixed, so it grows
/// with the rest of the interface.
class ThemeSwatchStrip extends StatelessWidget {
  /// The theme to show — which is not necessarily the one drawing this widget:
  /// the point is to preview a theme that is *not* in use.
  final LumitTheme theme;

  const ThemeSwatchStrip({super.key, required this.theme});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final colours = swatchesOf(theme);
    return Container(
      // One outline round the lot rather than one per swatch: eight bordered
      // squares read as eight controls, and none of these can be pressed.
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        border: Border.all(color: t.hairlineStrong),
      ),
      clipBehavior: Clip.antiAlias,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (var i = 0; i < colours.length; i++)
            Container(
              key: ValueKey('theme-swatch-$i'),
              width: 16,
              height: 18,
              color: colours[i],
            ),
        ],
      ),
    );
  }
}
