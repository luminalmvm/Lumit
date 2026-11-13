// The axes of the variable fonts the layers use, asked of the engine once a
// face and kept for the session.
//
// Asking may read the font's file, so it is not done while drawing: a panel
// asks, gets nothing the first time, and draws again when [fontAxesArrived]
// ticks with the answer.

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';

final Map<String, List<BridgeFontAxis>> _axes = {};

/// Ticks each time the axes of another face come in.
final ValueNotifier<int> fontAxesArrived = ValueNotifier(0);

/// The axes of [family]'s [face], or an empty list until they are in. The
/// built-in font has none.
List<BridgeFontAxis> fontAxesOf(String family, String face) {
  if (family.isEmpty) return const [];
  final key = '$family\u0000$face';
  final held = _axes[key];
  if (held != null) return held;
  _axes[key] = const [];
  textFontAxes(family: family, face: face).then((axes) {
    _axes[key] = axes;
    _asked.add(key);
    fontAxesArrived.value++;
  }).catchError((Object _) {});
  return const [];
}

/// The axes of [family]'s [face] once they are in, for a menu that is about
/// to list them: asked of the engine the first time, at once after that.
Future<List<BridgeFontAxis>> fontAxesReady(String family, String face) async {
  if (family.isEmpty) return const [];
  final key = '$family\u0000$face';
  final held = _axes[key];
  if (held != null && _asked.contains(key)) return held;
  try {
    final axes = await textFontAxes(family: family, face: face);
    _axes[key] = axes;
    _asked.add(key);
    fontAxesArrived.value++;
    return axes;
  } catch (_) {
    return held ?? const [];
  }
}

/// The faces whose axes have come in, as against asked for and pending.
final Set<String> _asked = {};

/// The axes an animator or the Text panel offers: the hidden ones are left to
/// the animators that already name them.
List<BridgeFontAxis> shownFontAxes(String family, String face) => [
      for (final a in fontAxesOf(family, face))
        if (!a.hidden) a
    ];

/// What [tag] is called in [family]'s [face], or the tag itself.
String fontAxisName(String family, String face, String tag) {
  for (final a in fontAxesOf(family, face)) {
    if (a.tag == tag) return a.name;
  }
  return switch (tag) {
    'wght' => l10n.textAxisWeight,
    'wdth' => l10n.textAxisWidth,
    'slnt' => l10n.textAxisSlant,
    'ital' => l10n.textAxisItalic,
    'opsz' => l10n.textAxisOpticalSize,
    _ => tag,
  };
}

/// The place [family]'s [face] sits at on [tag], which an animator's axis
/// starts from and resets to.
double fontAxisFace(String family, String face, String tag) {
  for (final a in fontAxesOf(family, face)) {
    if (a.tag == tag) return a.face;
  }
  return 0;
}

/// [tag]'s range in [family]'s [face], or a wide one before it is known.
(double, double) fontAxisRange(String family, String face, String tag) {
  for (final a in fontAxesOf(family, face)) {
    if (a.tag == tag) return (a.min, a.max);
  }
  return (-1000, 1000);
}
