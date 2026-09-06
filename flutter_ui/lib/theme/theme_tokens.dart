// Every colour a theme carries, as a list you can walk.
//
// **Why this exists.** `LumitTheme` is a struct of named fields, which is the
// right shape for *drawing* — `t.accent` reads better than a map lookup and
// the compiler catches a typo. It is the wrong shape for a settings page that
// has to offer one row per colour, and for a custom theme that has to be
// stored as data. So the tokens are declared once here, each with the reader
// and the writer that reach its field, and both the editor and the stored
// custom theme walk this list rather than restating the struct.
//
// Adding a colour to `LumitTheme` and forgetting to list it here would leave a
// row missing from the editor with nothing to say so — which is why
// `theme_tokens_test.dart` counts the struct's fields against this list.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';

import 'theme.dart';

/// One editable colour: what it is called, what it is for, and how to read or
/// write it on a theme.
class ThemeToken {
  /// The stable key a stored custom theme files this colour under. Never
  /// shown; never changed, or every saved theme loses that colour.
  final String key;

  /// The name the editor puts on the left of the row. Sentence case.
  final String label;

  /// One line saying what the colour does, for the row underneath.
  final String description;

  /// The group heading this token sits under in the editor.
  final String group;

  final Color Function(LumitTheme) read;
  final LumitTheme Function(LumitTheme, Color) write;

  const ThemeToken({
    required this.key,
    required this.label,
    required this.description,
    required this.group,
    required this.read,
    required this.write,
  });
}

/// Rebuild a theme with one field replaced. The struct has no general
/// `copyWith` (its own takes four fields, deliberately), so this is the one
/// place that restates it — every token's writer goes through here.
LumitTheme _with(
  LumitTheme t, {
  Color? surface0,
  Color? surface1,
  Color? surface2,
  Color? surface3,
  Color? surface4,
  Color? textPrimary,
  Color? textSecondary,
  Color? textMuted,
  Color? textDisabled,
  Color? hairline,
  Color? hairlineStrong,
  Color? accent,
  Color? accentHover,
  Color? animated,
  Color? success,
  Color? warning,
  Color? error,
  Color? cacheDisk,
  List<Color>? curve,
  LayerColours? layer,
  Color? timelineOutOfRange,
  Color? selectionFill,
  Color? marker,
  WaveformColours? waveform,
}) =>
    LumitTheme(
      mode: t.mode,
      shape: t.shape,
      tokens: t.tokens,
      surface0: surface0 ?? t.surface0,
      surface1: surface1 ?? t.surface1,
      surface2: surface2 ?? t.surface2,
      surface3: surface3 ?? t.surface3,
      surface4: surface4 ?? t.surface4,
      // Never a token: the Viewer's surround is strictly neutral by spec
      // (15-DESIGN §2.1/§11) because you cannot judge a grade against a
      // tinted surround. Carried through, never offered.
      viewerSurround: t.viewerSurround,
      textPrimary: textPrimary ?? t.textPrimary,
      textSecondary: textSecondary ?? t.textSecondary,
      textMuted: textMuted ?? t.textMuted,
      textDisabled: textDisabled ?? t.textDisabled,
      hairline: hairline ?? t.hairline,
      hairlineStrong: hairlineStrong ?? t.hairlineStrong,
      accent: accent ?? t.accent,
      accentHover: accentHover ?? t.accentHover,
      animated: animated ?? t.animated,
      success: success ?? t.success,
      warning: warning ?? t.warning,
      error: error ?? t.error,
      cacheDisk: cacheDisk ?? t.cacheDisk,
      curve: curve ?? t.curve,
      layer: layer ?? t.layer,
      timelineOutOfRange: timelineOutOfRange ?? t.timelineOutOfRange,
      selectionFill: selectionFill ?? t.selectionFill,
      marker: marker ?? t.marker,
      waveform: waveform ?? t.waveform,
    );

LayerColours _layerWith(
  LayerColours l, {
  Color? footage,
  Color? sequence,
  Color? precomp,
  Color? solid,
  Color? text,
  Color? camera,
}) =>
    LayerColours(
      footage: footage ?? l.footage,
      sequence: sequence ?? l.sequence,
      precomp: precomp ?? l.precomp,
      solid: solid ?? l.solid,
      text: text ?? l.text,
      camera: camera ?? l.camera,
    );

WaveformColours _waveformWith(
  WaveformColours w, {
  Color? rest,
  Color? low,
  Color? mid,
  Color? high,
}) =>
    WaveformColours(
      rest: rest ?? w.rest,
      low: low ?? w.low,
      mid: mid ?? w.mid,
      high: high ?? w.high,
    );

/// One curve stroke by index, preserving the rest of the ramp.
List<Color> _curveWith(List<Color> curve, int i, Color c) {
  final next = List<Color>.of(curve);
  if (i < next.length) next[i] = c;
  return next;
}

/// Every editable colour, in the order the editor lists them.
///
/// Grouped the way somebody changing a theme thinks: the grounds first (they
/// set the whole mood), then what sits on them, then the accents, then the
/// two areas with palettes of their own.
List<ThemeToken> get themeTokens => [
      // --- Surfaces -----------------------------------------------------------
      ThemeToken(
        key: 'surface0',
        label: l10n.tokenSurface0,
        description: l10n.tokenSurface0Help,
        group: l10n.tokenGroupSurfaces,
        read: (t) => t.surface0,
        write: (t, c) => _with(t, surface0: c),
      ),
      ThemeToken(
        key: 'surface1',
        label: l10n.tokenSurface1,
        description: l10n.tokenSurface1Help,
        group: l10n.tokenGroupSurfaces,
        read: (t) => t.surface1,
        write: (t, c) => _with(t, surface1: c),
      ),
      ThemeToken(
        key: 'surface2',
        label: l10n.tokenSurface2,
        description: l10n.tokenSurface2Help,
        group: l10n.tokenGroupSurfaces,
        read: (t) => t.surface2,
        write: (t, c) => _with(t, surface2: c),
      ),
      ThemeToken(
        key: 'surface3',
        label: l10n.tokenSurface3,
        description: l10n.tokenSurface3Help,
        group: l10n.tokenGroupSurfaces,
        read: (t) => t.surface3,
        write: (t, c) => _with(t, surface3: c),
      ),
      ThemeToken(
        key: 'surface4',
        label: l10n.tokenSurface4,
        description: l10n.tokenSurface4Help,
        group: l10n.tokenGroupSurfaces,
        read: (t) => t.surface4,
        write: (t, c) => _with(t, surface4: c),
      ),
      ThemeToken(
        key: 'timelineOutOfRange',
        label: l10n.tokenTimelineoutofrange,
        description: l10n.tokenTimelineoutofrangeHelp,
        group: l10n.tokenGroupSurfaces,
        read: (t) => t.timelineOutOfRange,
        write: (t, c) => _with(t, timelineOutOfRange: c),
      ),
      ThemeToken(
        key: 'selectionFill',
        label: l10n.tokenSelectionfill,
        description: l10n.tokenSelectionfillHelp,
        group: l10n.tokenGroupSurfaces,
        read: (t) => t.selectionFill,
        write: (t, c) => _with(t, selectionFill: c),
      ),

      // --- Text ---------------------------------------------------------------
      ThemeToken(
        key: 'textPrimary',
        label: l10n.tokenTextprimary,
        description: l10n.tokenTextprimaryHelp,
        group: l10n.tokenGroupText,
        read: (t) => t.textPrimary,
        write: (t, c) => _with(t, textPrimary: c),
      ),
      ThemeToken(
        key: 'textSecondary',
        label: l10n.tokenTextsecondary,
        description: l10n.tokenTextsecondaryHelp,
        group: l10n.tokenGroupText,
        read: (t) => t.textSecondary,
        write: (t, c) => _with(t, textSecondary: c),
      ),
      ThemeToken(
        key: 'textMuted',
        label: l10n.tokenTextmuted,
        description: l10n.tokenTextmutedHelp,
        group: l10n.tokenGroupText,
        read: (t) => t.textMuted,
        write: (t, c) => _with(t, textMuted: c),
      ),
      ThemeToken(
        key: 'textDisabled',
        label: l10n.tokenTextdisabled,
        description: l10n.tokenTextdisabledHelp,
        group: l10n.tokenGroupText,
        read: (t) => t.textDisabled,
        write: (t, c) => _with(t, textDisabled: c),
      ),

      // --- Lines --------------------------------------------------------------
      ThemeToken(
        key: 'hairline',
        label: l10n.tokenHairline,
        description: l10n.tokenHairlineHelp,
        group: l10n.tokenGroupLines,
        read: (t) => t.hairline,
        write: (t, c) => _with(t, hairline: c),
      ),
      ThemeToken(
        key: 'hairlineStrong',
        label: l10n.tokenHairlinestrong,
        description: l10n.tokenHairlinestrongHelp,
        group: l10n.tokenGroupLines,
        read: (t) => t.hairlineStrong,
        write: (t, c) => _with(t, hairlineStrong: c),
      ),

      // --- Roles --------------------------------------------------------------
      ThemeToken(
        key: 'accent',
        label: l10n.tokenAccent,
        description: l10n.tokenAccentHelp,
        group: l10n.tokenGroupRoles,
        read: (t) => t.accent,
        write: (t, c) => _with(t, accent: c),
      ),
      ThemeToken(
        key: 'accentHover',
        label: l10n.tokenAccenthover,
        description: l10n.tokenAccenthoverHelp,
        group: l10n.tokenGroupRoles,
        read: (t) => t.accentHover,
        write: (t, c) => _with(t, accentHover: c),
      ),
      ThemeToken(
        key: 'animated',
        label: l10n.tokenAnimated,
        description: l10n.tokenAnimatedHelp,
        group: l10n.tokenGroupRoles,
        read: (t) => t.animated,
        write: (t, c) => _with(t, animated: c),
      ),
      ThemeToken(
        key: 'success',
        label: l10n.tokenSuccess,
        description: l10n.tokenSuccessHelp,
        group: l10n.tokenGroupRoles,
        read: (t) => t.success,
        write: (t, c) => _with(t, success: c),
      ),
      ThemeToken(
        key: 'warning',
        label: l10n.tokenWarning,
        description: l10n.tokenWarningHelp,
        group: l10n.tokenGroupRoles,
        read: (t) => t.warning,
        write: (t, c) => _with(t, warning: c),
      ),
      ThemeToken(
        key: 'error',
        label: l10n.tokenError,
        description: l10n.tokenErrorHelp,
        group: l10n.tokenGroupRoles,
        read: (t) => t.error,
        write: (t, c) => _with(t, error: c),
      ),
      ThemeToken(
        key: 'marker',
        label: l10n.tokenMarker,
        description: l10n.tokenMarkerHelp,
        group: l10n.tokenGroupRoles,
        read: (t) => t.marker,
        write: (t, c) => _with(t, marker: c),
      ),
      ThemeToken(
        key: 'cacheDisk',
        label: l10n.tokenCachedisk,
        description: l10n.tokenCachediskHelp,
        group: l10n.tokenGroupRoles,
        read: (t) => t.cacheDisk,
        write: (t, c) => _with(t, cacheDisk: c),
      ),

      // --- Waveforms ----------------------------------------------------------
      ThemeToken(
        key: 'waveformRest',
        label: l10n.tokenWaveformrest,
        description: l10n.tokenWaveformrestHelp,
        group: l10n.tokenGroupWaveforms,
        read: (t) => t.waveform.rest,
        write: (t, c) => _with(t, waveform: _waveformWith(t.waveform, rest: c)),
      ),
      ThemeToken(
        key: 'waveformLow',
        label: l10n.tokenWaveformlow,
        description: l10n.tokenWaveformlowHelp,
        group: l10n.tokenGroupWaveforms,
        read: (t) => t.waveform.low,
        write: (t, c) => _with(t, waveform: _waveformWith(t.waveform, low: c)),
      ),
      ThemeToken(
        key: 'waveformMid',
        label: l10n.tokenWaveformmid,
        description: l10n.tokenWaveformmidHelp,
        group: l10n.tokenGroupWaveforms,
        read: (t) => t.waveform.mid,
        write: (t, c) => _with(t, waveform: _waveformWith(t.waveform, mid: c)),
      ),
      ThemeToken(
        key: 'waveformHigh',
        label: l10n.tokenWaveformhigh,
        description: l10n.tokenWaveformhighHelp,
        group: l10n.tokenGroupWaveforms,
        read: (t) => t.waveform.high,
        write: (t, c) => _with(t, waveform: _waveformWith(t.waveform, high: c)),
      ),

      // --- Graph curves -------------------------------------------------------
      for (var i = 0; i < 4; i++)
        ThemeToken(
          key: 'curve$i',
          label: l10n.tokenCurve('${i + 1}'),
          description: l10n.tokenCurveHelp,
          group: l10n.tokenGroupGraphCurves,
          read: (t) => i < t.curve.length ? t.curve[i] : t.accent,
          write: (t, c) => _with(t, curve: _curveWith(t.curve, i, c)),
        ),

      // --- Layer kinds --------------------------------------------------------
      ThemeToken(
        key: 'layerFootage',
        label: l10n.tokenLayerfootage,
        description: l10n.tokenLayerfootageHelp,
        group: l10n.tokenGroupLayerKinds,
        read: (t) => t.layer.footage,
        write: (t, c) => _with(t, layer: _layerWith(t.layer, footage: c)),
      ),
      ThemeToken(
        key: 'layerSequence',
        label: l10n.tokenLayersequence,
        description: l10n.tokenLayersequenceHelp,
        group: l10n.tokenGroupLayerKinds,
        read: (t) => t.layer.sequence,
        write: (t, c) => _with(t, layer: _layerWith(t.layer, sequence: c)),
      ),
      ThemeToken(
        key: 'layerPrecomp',
        label: l10n.tokenLayerprecomp,
        description: l10n.tokenLayerprecompHelp,
        group: l10n.tokenGroupLayerKinds,
        read: (t) => t.layer.precomp,
        write: (t, c) => _with(t, layer: _layerWith(t.layer, precomp: c)),
      ),
      ThemeToken(
        key: 'layerSolid',
        label: l10n.tokenLayersolid,
        description: l10n.tokenLayersolidHelp,
        group: l10n.tokenGroupLayerKinds,
        read: (t) => t.layer.solid,
        write: (t, c) => _with(t, layer: _layerWith(t.layer, solid: c)),
      ),
      ThemeToken(
        key: 'layerText',
        label: l10n.tokenLayertext,
        description: l10n.tokenLayertextHelp,
        group: l10n.tokenGroupLayerKinds,
        read: (t) => t.layer.text,
        write: (t, c) => _with(t, layer: _layerWith(t.layer, text: c)),
      ),
      ThemeToken(
        key: 'layerCamera',
        label: l10n.tokenLayercamera,
        description: l10n.tokenLayercameraHelp,
        group: l10n.tokenGroupLayerKinds,
        read: (t) => t.layer.camera,
        write: (t, c) => _with(t, layer: _layerWith(t.layer, camera: c)),
      ),
    ];

/// The token groups, in listing order and without repeats.
List<String> get themeTokenGroups {
  final seen = <String>[];
  for (final token in themeTokens) {
    if (!seen.contains(token.group)) seen.add(token.group);
  }
  return seen;
}

/// Read every token off a theme — what the editor opens with, and what a save
/// writes down.
Map<String, Color> tokensOf(LumitTheme theme) =>
    {for (final token in themeTokens) token.key: token.read(theme)};

/// Apply stored colours to a theme. A key this build does not know is
/// ignored, and a token the stored map does not carry keeps the base's
/// colour — so a theme saved by an older or newer Lumit still opens, with the
/// colours it does have.
LumitTheme applyTokens(LumitTheme base, Map<String, Color> colours) {
  var theme = base;
  for (final token in themeTokens) {
    final colour = colours[token.key];
    if (colour != null) theme = token.write(theme, colour);
  }
  return theme;
}
