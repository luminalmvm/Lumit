// The Lumit theme, ported one-for-one from crates/lumit-ui/src/theme.rs
// (K-084/K-092/K-097; docs/15-DESIGN.md). This is the ONLY Dart file where
// colour hex values may appear — everything else reads the LumitTheme object,
// mirroring the Rust no-hex-outside-theme rule.
//
// In plain terms: every colour, radius, gap and shadow the interface uses is a
// named token here, in the same seven colour schemes the Rust frontend ships.
// The numbers are carried over digit-for-digit so the two frontends can be
// compared side by side.

import 'package:flutter/material.dart';
import 'package:lumit_flutter/l10n/strings.dart';

Color _rgb(int r, int g, int b) => Color.fromARGB(0xff, r, g, b);

/// Light vs dark colour family (K-092).
enum ThemeMode2 { dark, light }

/// Sharp (edge-to-edge, hairline) or Round (floating card) geometry (K-092).
enum ThemeShape { sharp, round }

/// How much UI-chrome motion to show (K-092).
enum AnimationLevel { all, minimal, none }

/// The duration owned widgets animate with under a level. All ≈ the egui
/// 120 ms micro-motion budget; None is instant.
Duration animationDuration(AnimationLevel level) => switch (level) {
      AnimationLevel.all => const Duration(milliseconds: 120),
      AnimationLevel.minimal => const Duration(milliseconds: 50),
      AnimationLevel.none => Duration.zero,
    };

/// Every named colour scheme Lumit ships (K-097), in picker order.
enum LumitColorScheme {
  dark,
  darkBlue,
  light,
  gruvboxDark,
  gruvboxLight,
  catppuccinMocha,
  catppuccinLatte;

  /// Sentence-case display name for menus and settings.
  String get label => switch (this) {
        LumitColorScheme.dark => l10n.schemeDark,
        LumitColorScheme.darkBlue => l10n.schemeDarkBlue,
        LumitColorScheme.light => l10n.schemeLight,
        LumitColorScheme.gruvboxDark => 'Gruvbox dark',
        LumitColorScheme.gruvboxLight => 'Gruvbox light',
        LumitColorScheme.catppuccinMocha => 'Catppuccin Mocha',
        LumitColorScheme.catppuccinLatte => 'Catppuccin Latte',
      };

  ThemeMode2 get mode => switch (this) {
        LumitColorScheme.light ||
        LumitColorScheme.gruvboxLight ||
        LumitColorScheme.catppuccinLatte =>
          ThemeMode2.light,
        _ => ThemeMode2.dark,
      };

  LumitTheme build() => switch (this) {
        LumitColorScheme.dark => LumitTheme.dark(),
        LumitColorScheme.darkBlue => LumitTheme.darkBlue(),
        LumitColorScheme.light => LumitTheme.light(),
        LumitColorScheme.gruvboxDark => LumitTheme.gruvboxDark(),
        LumitColorScheme.gruvboxLight => LumitTheme.gruvboxLight(),
        LumitColorScheme.catppuccinMocha => LumitTheme.catppuccinMocha(),
        LumitColorScheme.catppuccinLatte => LumitTheme.catppuccinLatte(),
      };
}

/// Shape-dependent chrome geometry (K-092). `sharp` reproduces the egui
/// frontend's pre-K-092 numbers exactly; `round` is the floating-card system.
class ShapeTokens {
  /// A control's corner radius — button, chip, tab, dropdown, value box.
  ///
  /// Under Round this is [stadium] rather than a number (K-394, §12.1): every
  /// control is a full capsule, and a capsule's radius is half the control's
  /// **own** height, which a token cannot know. It does not have to: a rounded
  /// rectangle scales its radii down to fit the box it is drawn into, so a
  /// radius larger than any control is tall *is* the capsule, at whatever
  /// height the control turns out to be. That is why this stays one number
  /// instead of growing a flag — every existing
  /// `BorderRadius.circular(tokens.controlRadius)` is already correct under
  /// both shapes, with nothing to change and nothing to forget.
  ///
  /// It is a **corner radius only**. Never use it as a length (a padding, a
  /// height, an inset): under Round it is not one.
  final double controlRadius;
  final double floatRadius;
  final double cardRadius;
  final double cardPadding;
  final double tileGap;
  final double windowInset;
  final List<BoxShadow> cardShadow;

  const ShapeTokens({
    required this.controlRadius,
    required this.floatRadius,
    required this.cardRadius,
    required this.cardPadding,
    required this.tileGap,
    required this.windowInset,
    required this.cardShadow,
  });

  static const sharp = ShapeTokens(
    // The mockups draw 2 on almost every control (K-476, measured from the
    // computed-style manifests); 4 was K-394's original guess.
    controlRadius: 2,
    floatRadius: 6,
    cardRadius: 0,
    cardPadding: 0,
    tileGap: 1.0,
    windowInset: 0.0,
    cardShadow: [],
  );

  /// The value [controlRadius] carries under Round: bigger than any control is
  /// tall, so the corner clamps to half the height and draws a stadium.
  static const double stadium = 1000;

  static const round = ShapeTokens(
    controlRadius: stadium,
    floatRadius: 16,
    cardRadius: 18,
    cardPadding: 10,
    tileGap: 12.0,
    windowInset: 12.0,
    cardShadow: [
      BoxShadow(
        offset: Offset(0, 4),
        blurRadius: 16,
        color: Color(0x30000000),
      ),
    ],
  );

  static ShapeTokens of(ThemeShape shape) =>
      shape == ThemeShape.sharp ? sharp : round;
}

/// **How much room a row gets** (K-454, docs/15-DESIGN.md §12A.6).
///
/// In plain terms: the same panels, drawn with a little more or a little less
/// air between their lines. There are exactly two settings of this dial and
/// there will not be a third — a slider would let a user land on a half-pixel
/// row and misalign the Timeline's two halves.
///
/// [regular] is the default, and it is what the approved mockups actually
/// render: their *effective* heights, meaning the content plus the seams and
/// borders painted around it. [compact] is a pixel or two tighter per row,
/// for someone who would rather see more layers at once than have the room —
/// it is the set of values the app shipped before this dial existed.
///
/// Only the rows that genuinely differ are listed. A panel header strip, a
/// clip bar, a value well and an effect heading measure the same under both
/// densities, so they stay plain constants where they are declared: a token
/// whose two values are equal is a knob that does nothing.
class DensityTokens {
  /// A layer's row in the outline, and its lane beside it. The mockups draw
  /// 22 of row with a 1px seam under it and the eye reads 23; the tighter
  /// setting fits the seam inside the 22.
  final double laneRow;

  /// The thin rows that frame a panel — the Project panel's column header, a
  /// panel's bottom bar, the graph's key readout. 18 of content; Regular
  /// counts the hairline beneath it in, as §12A.6 already does for the
  /// Project panel's column header.
  ///
  /// **The Timeline's own chrome is no longer one of these** (K-512): its two
  /// rows are [timelineChromeRow] and [timelineHeaderRow], which under Regular
  /// stand taller than a secondary row anywhere else.
  final double secondaryRow;

  /// The Timeline's first chrome row — the timecode and frame readouts, the
  /// layer search, and the Layers / Keys / Graph tabs (K-512).
  ///
  /// **24 under Regular**, where every other secondary row is 19: the owner's
  /// ruling from desktop testing is that this row is aimed at constantly and
  /// was too small to hit comfortably. Compact keeps the 18 it always drew.
  final double timelineChromeRow;

  /// The Timeline's second chrome row — the column-group headers in Layers
  /// mode, the dope sheet's filters in Keys mode, the graph's in Graph mode
  /// (K-512). **23 under Regular**, 18 under Compact.
  ///
  /// It is a separate number from [timelineChromeRow] because the two rows do
  /// different work: the row above is aimed at, this one is mostly read.
  final double timelineHeaderRow;

  /// How tall a control standing in either Timeline chrome row is *told* to
  /// be, or **null for "measure yourself"** (K-512).
  ///
  /// In plain terms: Regular's rows grew, so the buttons and wells in them
  /// grow too rather than floating in a band of empty ground — one number, so
  /// the tabs, the search well and the two readouts all stand level. Compact's
  /// rows are exactly the rows these controls were built for, so it states
  /// nothing and each control keeps the size it has always measured itself to.
  final double? timelineChromeControl;

  /// The pickers that sit *inside* a layer's row: matte, blend and parent.
  final double inRowPicker;

  /// A dropdown's closed face anywhere else — a panel row, a bar.
  final double dropdownFace;

  /// A property or effect-parameter row in the Effect controls.
  final double propertyRow;

  const DensityTokens({
    required this.laneRow,
    required this.secondaryRow,
    required this.timelineChromeRow,
    required this.timelineHeaderRow,
    required this.timelineChromeControl,
    required this.inRowPicker,
    required this.dropdownFace,
    required this.propertyRow,
  });

  /// The Timeline ruler, which is **derived and not declared**: the lane side
  /// gives the ruler exactly the height the outline side spends on its two
  /// chrome rows, and that is the whole reason the two halves of the panel
  /// line up row for row. Grow either row and the ruler grows with it — which
  /// is what K-512 did, taking Regular's ruler from 38 to **47** while Compact
  /// keeps its 36.
  ///
  /// The extra room goes to the clock: the ruler's two halves are no longer
  /// ruled apart (K-512 again), so what the reader sees is one taller band
  /// with the labels near its top and the markers and work area on its floor.
  double get ruler => timelineChromeRow + timelineHeaderRow;

  /// What the mockups render, with the Timeline's chrome at the height the
  /// owner asked for after desktop testing. The default (K-454, K-512).
  static const regular = DensityTokens(
    laneRow: 23,
    secondaryRow: 19,
    timelineChromeRow: 24,
    timelineHeaderRow: 23,
    timelineChromeControl: 20,
    inRowPicker: 18,
    dropdownFace: 20,
    propertyRow: 27,
  );

  /// A pixel or two off each row, for more visible at once. What the app drew
  /// before the setting existed — **including the Timeline's chrome**, which
  /// K-512 grew under Regular alone.
  static const compact = DensityTokens(
    laneRow: 22,
    secondaryRow: 18,
    timelineChromeRow: 18,
    timelineHeaderRow: 18,
    timelineChromeControl: null,
    inRowPicker: 16,
    dropdownFace: 18,
    propertyRow: 26,
  );

  static DensityTokens of(bool isCompact) => isCompact ? compact : regular;
}

/// Per-layer-type identity colours (docs/15-DESIGN.md §6.1).
class LayerColours {
  final Color footage, sequence, precomp, solid, text, camera;
  const LayerColours({
    required this.footage,
    required this.sequence,
    required this.precomp,
    required this.solid,
    required this.text,
    required this.camera,
  });
}

/// Colours the Scopes panel draws with (15-DESIGN §8, K-096). One fixed set
/// shared by every theme — a scope is always read on a near-black graticule,
/// whatever the chrome, the same grading-accuracy reasoning that keeps
/// `viewerSurround` neutral.
class ScopeColours {
  final Color bg, graticule, trace, red, green, blue;
  const ScopeColours({
    required this.bg,
    required this.graticule,
    required this.trace,
    required this.red,
    required this.green,
    required this.blue,
  });

  static const standard = ScopeColours(
    bg: Color(0xff0a0b0c),
    graticule: Color(0xff393d40),
    trace: Color(0xff86dd9a),
    red: Color(0xffe2555f),
    green: Color(0xff54cf6b),
    blue: Color(0xff5387e0),
  );
}

/// What a wire and its sockets are painted in on the Graph panel's canvas
/// (K-472, 15-DESIGN §4.1/§12A.7). Seven port types wear **five** colours,
/// grouped as the approved NodeGraph drawing's legend groups them:
/// image·matte, number, colour, shape·points, audio.
///
/// One fixed set shared by every theme, and **deliberately not an editable
/// token** — the same reasoning that keeps `viewerSurround` out of the
/// editor's reach. Colour *is* the legend here (K-445): the strip along the
/// canvas's bottom edge, the manual and the drawings all say "amber is a
/// number", so a palette taste could retint would be a legend that lies. The
/// five are the layer palette's own azure, amber, magenta, teal and mint, which
/// were picked to be told apart at a glance and already read on both grounds.
class PortColours {
  /// Image and matte — the picture's own path down the stack.
  final Color image;

  /// Number: the type nearly every driven parameter is.
  final Color number;
  final Color colour;

  /// Shape and points (K-472 §6.2) — geometry, whether it is one outline or a
  /// stream of thousands.
  final Color geometry;
  final Color audio;

  const PortColours({
    required this.image,
    required this.number,
    required this.colour,
    required this.geometry,
    required this.audio,
  });

  static const standard = PortColours(
    image: Color(0xff4aa3e0),
    number: Color(0xffe0a33c),
    colour: Color(0xffd45cb8),
    geometry: Color(0xff3cc9c0),
    audio: Color(0xff46c98e),
  );
}

/// The colours a waveform draws in (docs/15-DESIGN.md §6.4). Split out of the
/// roles the lanes used to borrow when the waveform lane learned to follow the
/// zoom and to stack its bands (K-280) — §6.4's standing direction is that each
/// grouping becomes a token of its own as its area is next touched, and this is
/// that touch. Waveforms are **content, not state**, so none of these is the
/// accent: a wave says what the sound is, never that something is selected.
class WaveformColours {
  /// The single full-range wave, and the envelope a multiwave stack is read
  /// against — the muted steel-cyan §6.4 names.
  final Color rest;

  /// The three bands of the multiwave stack: bass, middle, treble. They are
  /// drawn over one another in one lane, so they are ranked by **brightness**
  /// rather than by hue — the bass a dim broad body, the treble bright and
  /// thin over it. Hue-coding them read as three unrelated waveforms; a
  /// brightness ramp reads as one waveform with its inside showing.
  final Color low, mid, high;

  const WaveformColours({
    required this.rest,
    required this.low,
    required this.mid,
    required this.high,
  });

  /// Value equality, so a painter handed the same colours from a rebuilt theme
  /// does not repaint every lane for nothing.
  @override
  bool operator ==(Object other) =>
      other is WaveformColours &&
      other.rest == rest &&
      other.low == low &&
      other.mid == mid &&
      other.high == high;

  @override
  int get hashCode => Object.hash(rest, low, mid, high);
}

/// Semantic colour tokens; names mirror docs/15-DESIGN.md §tokens and the
/// Rust `Theme` struct field-for-field.
class LumitTheme {
  final ThemeMode2 mode;
  final ThemeShape shape;
  final ShapeTokens tokens;

  /// How much room a row gets (K-454). It rides on the theme because that is
  /// what every widget already has in hand, and because changing it has to
  /// repaint everything at once — the same journey a shape change makes.
  final DensityTokens density;

  // Surfaces (near-neutral ramp; direction depends on mode).
  final Color surface0, surface1, surface2, surface3, surface4;

  /// The Viewer pasteboard — exactly neutral, R = G = B, never mode-mirrored
  /// (grading accuracy, 15-DESIGN §2.1/§11).
  final Color viewerSurround;

  // Text.
  final Color textPrimary, textSecondary, textMuted, textDisabled;

  // Hairlines.
  final Color hairline, hairlineStrong;

  // Roles — the accent is THE single accent per view.
  final Color accent, accentHover;

  /// "This is animated or in hand" (K-439): keyframe diamonds, stopwatch-on,
  /// selected keyframes, selected gizmo handles, the focused value field and
  /// the work-area band — and nothing else. The job list is closed, so a third
  /// kind of use is a mistake rather than a new job. A desaturated warm amber,
  /// deliberately quieter than `accent`: "keyed" is a state a composition is
  /// full of, while the accent marks the one thing in hand.
  final Color animated;
  final Color success, warning, error, cacheDisk;

  /// Graph-editor curve strokes.
  final List<Color> curve;
  final LayerColours layer;

  /// The Timeline's ground *outside* the work area (K-202). The lane, layer
  /// and graph areas are read as one long strip, and until this existed there
  /// was nothing to tell "the part you are delivering" from the rest — so the
  /// whole strip sat at one value and a selected row had only `surface2` to
  /// stand out against. Inside the work area the strip keeps `surface1`; this
  /// is the wash either side of it.
  final Color timelineOutOfRange;

  /// The fill under a selected row, and at reduced strength under a
  /// highlighted one (K-202). Its own token rather than `surface2` reused: a
  /// selection has to out-contrast the ground it sits on, and on a light
  /// scheme that means going *darker* where the surfaces go lighter — a rule
  /// the surface ramp cannot express because it is a ramp.
  final Color selectionFill;

  /// What waveforms draw in (K-280) — the single wave and the three bands of
  /// the multiwave stack. Its own grouping rather than roles borrowed one at a
  /// time, per the §6.4 direction.
  final WaveformColours waveform;

  /// What the Graph panel's wires and sockets draw in, by port type (K-472).
  /// Fixed rather than per-scheme, and not offered to the theme editor — see
  /// [PortColours].
  final PortColours port;

  /// Comp markers on the time ruler (K-254). A plain grey, not a role colour:
  /// a marker says *here*, not *good* or *careful*, and the ruler already has
  /// the accent doing the work area. Light on a dark scheme and dark on a light
  /// one — After Effects' own reading, and the one that stays legible over the
  /// work-area band either way.
  final Color marker;

  /// The wash a modal window lays over the app behind it (K-269). Its own
  /// token because it is not a surface: it is the *absence* of attention, a
  /// translucent black that dims whatever it covers rather than a colour the
  /// ramp could supply. Translucent black under a light scheme too — dimming
  /// is dimming, and a pale scrim over pale panels would say nothing.
  final Color scrim;

  /// The three Timeline tokens default from the mode rather than being spelled
  /// out by every scheme: they are a *relationship* to the surface ramp (a
  /// shade beyond `surface1`, a fill that out-contrasts it, a grey that reads
  /// against it), and seven schemes restating that relationship would be seven
  /// chances to get it wrong. A custom theme, or any scheme that wants its own,
  /// passes them explicitly.
  LumitTheme({
    required this.mode,
    this.shape = ThemeShape.sharp,
    this.tokens = ShapeTokens.sharp,
    this.density = DensityTokens.regular,
    required this.surface0,
    required this.surface1,
    required this.surface2,
    required this.surface3,
    required this.surface4,
    required this.viewerSurround,
    required this.textPrimary,
    required this.textSecondary,
    required this.textMuted,
    required this.textDisabled,
    required this.hairline,
    required this.hairlineStrong,
    required this.accent,
    required this.accentHover,
    required this.animated,
    required this.success,
    required this.warning,
    required this.error,
    required this.cacheDisk,
    required this.curve,
    required this.layer,
    this.port = PortColours.standard,
    Color? timelineOutOfRange,
    Color? selectionFill,
    Color? marker,
    Color? scrim,
    WaveformColours? waveform,
  })  : timelineOutOfRange =
            timelineOutOfRange ?? defaultOutOfRange(mode, surface1),
        selectionFill = selectionFill ?? defaultSelectionFill(mode, surface2),
        marker = marker ?? defaultMarker(mode),
        scrim = scrim ?? defaultScrim(mode),
        waveform = waveform ?? defaultWaveform(mode);

  /// The ground outside the work area: a step *away* from the surface ramp's
  /// direction — darker under a dark scheme, and darker again under a light
  /// one, because on white the only direction with anywhere to go is down
  /// (K-202). Deliberately gentle: this marks a region, it is not a border.
  static Color defaultOutOfRange(ThemeMode2 mode, Color surface1) {
    final by = mode == ThemeMode2.dark ? -0x08 : -0x0e;
    return _shift(surface1, by);
  }

  /// The selected-row fill. Under a dark scheme it lifts, under a light one it
  /// drops — either way it lands clear of both grounds, which reusing a
  /// surface could not promise once the Timeline gained a second ground.
  static Color defaultSelectionFill(ThemeMode2 mode, Color surface2) {
    final by = mode == ThemeMode2.dark ? 0x0e : -0x1c;
    return _shift(surface2, by);
  }

  /// The marker grey. A fixed pair rather than a shift off the surface ramp:
  /// it has to read against the ruler's ground *and* the work-area wash over
  /// it, so it is pinned to the two values that do, not derived from one of
  /// the things it must stand out from.
  static Color defaultMarker(ThemeMode2 mode) =>
      mode == ThemeMode2.dark ? _rgb(0xc4, 0xc4, 0xc4) : _rgb(0x56, 0x56, 0x56);

  /// The waveform palette. A fixed set per mode rather than a shift off the
  /// surface ramp, for the same reason the marker grey is: a wave has to read
  /// against the lane's ground *and* against a selected row's fill over it, and
  /// a colour derived from one of those cannot promise to stand out from both.
  /// Steel-cyan at rest (§6.4); the three bands run dim → bright as the
  /// frequency climbs, so the treble reads as highlights inside the body the
  /// bass fills. On a light scheme the ramp runs the other way — *darker* is
  /// what stands out on white, so the treble is the darkest of the three.
  static WaveformColours defaultWaveform(ThemeMode2 mode) =>
      mode == ThemeMode2.dark
          ? WaveformColours(
              rest: _rgb(0x5d, 0x8a, 0x96),
              low: _rgb(0x3c, 0x5c, 0x66),
              mid: _rgb(0x6d, 0x9a, 0xa6),
              high: _rgb(0xd4, 0xf0, 0xf6),
            )
          : WaveformColours(
              rest: _rgb(0x3f, 0x6b, 0x78),
              low: _rgb(0x9d, 0xba, 0xc2),
              mid: _rgb(0x59, 0x87, 0x94),
              high: _rgb(0x14, 0x33, 0x3c),
            );

  /// The modal scrim. Black either way — a scrim dims, and on a light scheme
  /// there is nothing above white to dim *with* — but a shade lighter over a
  /// light one, where the same opacity would read as a blackout rather than a
  /// hush (§calm voice: the window in front is louder, the app behind is not
  /// being punished).
  static Color defaultScrim(ThemeMode2 mode) => Color.fromARGB(
        mode == ThemeMode2.dark ? 0x99 : 0x66,
        0,
        0,
        0,
      );

  /// Shift every channel by [by], clamped — the one place the theme nudges a
  /// colour, so "a shade darker" means the same thing wherever it is said.
  static Color _shift(Color c, int by) {
    int ch(double v) => ((v * 255).round() + by).clamp(0, 255);
    return Color.fromARGB(0xff, ch(c.r), ch(c.g), ch(c.b));
  }

  /// **Spruce** — THE default accent (K-538, after the brand went green with
  /// it; clay is one click away). The stock dark and light schemes both build
  /// their accent pair from it, so a single edit here moves the whole family.
  /// Hover derives by the same ±0x12 step [withAccent] gives a user-picked
  /// accent (K-092), which is what makes `withAccent(defaultAccent)` reproduce
  /// each stock scheme's pair exactly rather than approximately — now in
  /// **both** schemes, since spruce clears the contrast floor on white and
  /// Light no longer needs a hand-darkened accent of its own.
  static const defaultAccent = Color(0xff35785e);

  /// The layer-label palette (TL2, K-188): bright, clearly distinct chips. A
  /// layer's label colours both its swatch and its bar in the lane area, and
  /// each layer kind defaults to a different one — so the set is a dedicated
  /// palette, not the theme's role colours, which were built to be quiet rather
  /// than tellable-apart. Index 0 is the neutral default; the Null layer (K-206)
  /// took the ninth chip, since every one below it was already a kind's own.
  static const _labelSet = [
    Color(0xff8b93a3), // slate — the quiet default
    Color(0xff4aa3e0), // azure — footage
    Color(0xffe0a33c), // amber — solids
    Color(0xffa06ce0), // violet — precomps
    Color(0xff46c98e), // mint — text
    Color(0xff3cc9c0), // teal — cameras
    Color(0xff6673e6), // indigo — sequences
    Color(0xffd45cb8), // magenta — adjustments
    Color(0xffe0704a), // coral — nulls
  ];

  /// How many chips the palette holds, so the picker draws exactly the set
  /// rather than a number that has to be kept in step by hand.
  static int get labelCount => _labelSet.length;

  Color labelColour(int i) => _labelSet[i % _labelSet.length];

  /// This theme with a user-picked accent: hover brightens by 0x12 per
  /// channel on a dark surface, darkens by the same on a light one (K-092).
  LumitTheme withAccent(Color rgb) {
    int shift(int c) => mode == ThemeMode2.dark
        ? (c + 0x12).clamp(0, 255)
        : (c - 0x12).clamp(0, 255);
    final hover = Color.fromARGB(
      0xff,
      shift((rgb.r * 255).round()),
      shift((rgb.g * 255).round()),
      shift((rgb.b * 255).round()),
    );
    return copyWith(accent: rgb, accentHover: hover);
  }

  LumitTheme copyWith({
    ThemeShape? shape,
    ShapeTokens? tokens,
    DensityTokens? density,
    Color? accent,
    Color? accentHover,
  }) =>
      LumitTheme(
        mode: mode,
        shape: shape ?? this.shape,
        tokens: tokens ?? this.tokens,
        density: density ?? this.density,
        surface0: surface0,
        surface1: surface1,
        surface2: surface2,
        surface3: surface3,
        surface4: surface4,
        viewerSurround: viewerSurround,
        textPrimary: textPrimary,
        textSecondary: textSecondary,
        textMuted: textMuted,
        textDisabled: textDisabled,
        hairline: hairline,
        hairlineStrong: hairlineStrong,
        accent: accent ?? this.accent,
        accentHover: accentHover ?? this.accentHover,
        animated: animated,
        success: success,
        warning: warning,
        error: error,
        cacheDisk: cacheDisk,
        curve: curve,
        layer: layer,
        waveform: waveform,
        timelineOutOfRange: timelineOutOfRange,
        selectionFill: selectionFill,
        marker: marker,
        scrim: scrim,
      );

  /// The full composition a scheme + shape (+ accent override) resolves to —
  /// the Dart `Theme::for_scheme` + `with_accent`.
  static LumitTheme forScheme(
    LumitColorScheme scheme,
    ThemeShape shape, {
    Color? accentOverride,
  }) {
    var t =
        scheme.build().copyWith(shape: shape, tokens: ShapeTokens.of(shape));
    if (accentOverride != null) t = t.withAccent(accentOverride);
    return t;
  }

  factory LumitTheme.dark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x0b, 0x0c, 0x0e),
        surface1: _rgb(0x13, 0x15, 0x17),
        surface2: _rgb(0x1a, 0x1d, 0x20),
        surface3: _rgb(0x21, 0x25, 0x28),
        surface4: _rgb(0x2b, 0x30, 0x34),
        viewerSurround: _rgb(0x12, 0x12, 0x12),
        textPrimary: _rgb(0xee, 0xf1, 0xf2),
        textSecondary: _rgb(0xc2, 0xc8, 0xcb),
        textMuted: _rgb(0x8b, 0x92, 0x96),
        textDisabled: _rgb(0x5e, 0x66, 0x6b),
        hairline: _rgb(0x26, 0x29, 0x2c),
        hairlineStrong: _rgb(0x3c, 0x41, 0x45),
        accent: defaultAccent, // spruce #35785e (K-538)
        accentHover: _shift(defaultAccent, 0x12), // #478a70, derived - which is
        // what keeps withAccent(defaultAccent) reproducing this pair
        animated: _rgb(0xd8, 0xa2, 0x4a),
        success: _rgb(0x5f, 0xcf, 0xae),
        warning: _rgb(0xdd, 0x9a, 0x82),
        error: _rgb(0xd1, 0x72, 0x9c),
        cacheDisk: _rgb(0x5f, 0x93, 0xb8),
        curve: [
          _rgb(0x8e, 0xe3, 0xef),
          _rgb(0xae, 0xf3, 0xe7),
          _rgb(0xe8, 0xa7, 0xb4),
          _rgb(0xd8, 0xcb, 0xa0),
        ],
        layer: LayerColours(
          footage: _rgb(0x56, 0x70, 0x7f),
          sequence: _rgb(0x5a, 0x6a, 0x8c),
          precomp: _rgb(0x7a, 0x5a, 0x74),
          solid: _rgb(0x5c, 0x61, 0x65),
          text: _rgb(0x8c, 0x84, 0x68),
          camera: _rgb(0x80, 0x6f, 0x4a),
        ),
      );

  /// The pre-K-084 ramp: bluer, a step lighter; everything else shared with
  /// dark().
  factory LumitTheme.darkBlue() {
    final base = LumitTheme.dark();
    return LumitTheme(
      mode: ThemeMode2.dark,
      surface0: _rgb(0x14, 0x16, 0x18),
      surface1: _rgb(0x1b, 0x1e, 0x20),
      surface2: _rgb(0x22, 0x26, 0x2a),
      surface3: _rgb(0x2b, 0x30, 0x34),
      surface4: _rgb(0x34, 0x3a, 0x3f),
      viewerSurround: _rgb(0x1e, 0x1e, 0x1e),
      textPrimary: _rgb(0xe6, 0xe9, 0xea),
      textSecondary: _rgb(0xb6, 0xbc, 0xbf),
      textMuted: _rgb(0x83, 0x8b, 0x90),
      textDisabled: _rgb(0x66, 0x70, 0x77),
      hairline: _rgb(0x25, 0x27, 0x29),
      hairlineStrong: _rgb(0x3d, 0x40, 0x42),
      accent: base.accent,
      accentHover: base.accentHover,
      animated: base.animated,
      success: base.success,
      warning: base.warning,
      error: base.error,
      cacheDisk: base.cacheDisk,
      curve: base.curve,
      layer: base.layer,
    );
  }

  /// The light ramp (K-092): one uniform light theme.
  factory LumitTheme.light() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xee, 0xec, 0xe9),
        surface1: _rgb(0xff, 0xff, 0xff),
        surface2: _rgb(0xf6, 0xf5, 0xf3),
        surface3: _rgb(0xff, 0xff, 0xff),
        surface4: _rgb(0xe9, 0xe7, 0xe4),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x1a, 0x1a, 0x18),
        textSecondary: _rgb(0x45, 0x45, 0x42),
        textMuted: _rgb(0x7a, 0x7a, 0x76),
        textDisabled: _rgb(0xa8, 0xa8, 0xa4),
        hairline: _rgb(0xd8, 0xd6, 0xd2),
        hairlineStrong: _rgb(0xc4, 0xc1, 0xbc),
        // Light used to keep a hand-darkened accent of its own, because clay
        // on white sat under the contrast floor. Spruce clears it at 5.3:1, so
        // the exception is gone: both stock schemes are now exactly
        // withAccent(defaultAccent), and the hover is the same ±0x12 step in
        // each - lighter in Dark, darker here.
        accent: defaultAccent, // spruce #35785e (K-538)
        accentHover: _shift(defaultAccent, -0x12), // #23664c
        // Provisional pending the light-mode pass: the dark scheme's amber
        // taken down until it holds against white.
        animated: _rgb(0x9a, 0x6a, 0x1c),
        success: _rgb(0x2f, 0x8f, 0x71),
        warning: _rgb(0xb5, 0x5f, 0x46),
        error: _rgb(0x9c, 0x3f, 0x66),
        cacheDisk: _rgb(0x2f, 0x5f, 0x82),
        curve: [
          _rgb(0x2f, 0x8a, 0x96),
          _rgb(0x3f, 0x9c, 0x8e),
          _rgb(0xb5, 0x5f, 0x6e),
          _rgb(0x8a, 0x76, 0x42),
        ],
        layer: LayerColours(
          footage: _rgb(0x3d, 0x52, 0x60),
          sequence: _rgb(0x40, 0x4d, 0x68),
          precomp: _rgb(0x5c, 0x40, 0x56),
          solid: _rgb(0x42, 0x46, 0x49),
          text: _rgb(0x66, 0x5e, 0x46),
          camera: _rgb(0x5e, 0x50, 0x30),
        ),
      );

  /// Gruvbox dark (K-097).
  factory LumitTheme.gruvboxDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x28, 0x28, 0x28),
        surface1: _rgb(0x3c, 0x38, 0x36),
        surface2: _rgb(0x50, 0x49, 0x45),
        surface3: _rgb(0x66, 0x5c, 0x54),
        surface4: _rgb(0x7c, 0x6f, 0x64),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xfb, 0xf1, 0xc7),
        textSecondary: _rgb(0xeb, 0xdb, 0xb2),
        textMuted: _rgb(0xd5, 0xc4, 0xa1),
        textDisabled: _rgb(0xbd, 0xae, 0x93),
        hairline: _rgb(0x92, 0x83, 0x74),
        hairlineStrong: _rgb(0xa8, 0x99, 0x84),
        accent: _rgb(0xfe, 0x80, 0x19),
        accentHover: _rgb(0xfd, 0x94, 0x38),
        animated: _rgb(0xd7, 0x99, 0x21),
        success: _rgb(0xb8, 0xbb, 0x26),
        warning: _rgb(0xfa, 0xbd, 0x2f),
        error: _rgb(0xcc, 0x24, 0x1d),
        cacheDisk: _rgb(0x83, 0xa5, 0x98),
        curve: [
          _rgb(0x8e, 0xc0, 0x7c),
          _rgb(0x83, 0xa5, 0x98),
          _rgb(0xd3, 0x86, 0x9b),
          _rgb(0xfa, 0xbd, 0x2f),
        ],
        layer: LayerColours(
          footage: _rgb(0x6a, 0x77, 0x6e),
          sequence: _rgb(0x8b, 0x7b, 0x7c),
          precomp: _rgb(0x92, 0x68, 0x70),
          solid: _rgb(0x87, 0x7a, 0x6c),
          text: _rgb(0x94, 0x77, 0x3c),
          camera: _rgb(0x96, 0x5f, 0x33),
        ),
      );

  /// Gruvbox light (K-097).
  factory LumitTheme.gruvboxLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xeb, 0xdb, 0xb2),
        surface1: _rgb(0xfb, 0xf1, 0xc7),
        surface2: _rgb(0xf3, 0xe6, 0xbc),
        surface3: _rgb(0xfb, 0xf1, 0xc7),
        surface4: _rgb(0xd5, 0xc4, 0xa1),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x28, 0x28, 0x28),
        textSecondary: _rgb(0x3c, 0x38, 0x36),
        textMuted: _rgb(0x50, 0x49, 0x45),
        textDisabled: _rgb(0x66, 0x5c, 0x54),
        hairline: _rgb(0xe0, 0xd0, 0xaa),
        hairlineStrong: _rgb(0xbd, 0xae, 0x93),
        accent: _rgb(0xaf, 0x3a, 0x03),
        accentHover: _rgb(0x86, 0x35, 0x0e),
        // Provisional pending the light-mode pass.
        animated: _rgb(0x8f, 0x64, 0x14),
        success: _rgb(0x79, 0x74, 0x0e),
        warning: _rgb(0xb5, 0x76, 0x14),
        error: _rgb(0x9d, 0x00, 0x06),
        cacheDisk: _rgb(0x07, 0x66, 0x78),
        curve: [
          _rgb(0x42, 0x7b, 0x58),
          _rgb(0x07, 0x66, 0x78),
          _rgb(0x8f, 0x3f, 0x71),
          _rgb(0xb5, 0x76, 0x14),
        ],
        layer: LayerColours(
          footage: _rgb(0x13, 0x50, 0x5c),
          sequence: _rgb(0x42, 0x48, 0x61),
          precomp: _rgb(0x70, 0x38, 0x5b),
          solid: _rgb(0x5b, 0x52, 0x4c),
          text: _rgb(0x8b, 0x5f, 0x1a),
          camera: _rgb(0x91, 0x36, 0x0b),
        ),
      );

  /// Catppuccin Mocha (K-097).
  factory LumitTheme.catppuccinMocha() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x11, 0x11, 0x1b),
        surface1: _rgb(0x1e, 0x1e, 0x2e),
        surface2: _rgb(0x31, 0x32, 0x44),
        surface3: _rgb(0x45, 0x47, 0x5a),
        surface4: _rgb(0x58, 0x5b, 0x70),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xcd, 0xd6, 0xf4),
        textSecondary: _rgb(0xba, 0xc2, 0xde),
        textMuted: _rgb(0x7f, 0x84, 0x9c),
        textDisabled: _rgb(0x6c, 0x70, 0x86),
        hairline: _rgb(0x39, 0x3a, 0x4d),
        hairlineStrong: _rgb(0x4f, 0x52, 0x66),
        accent: _rgb(0xcb, 0xa6, 0xf7),
        accentHover: _rgb(0xcc, 0xb2, 0xf6),
        animated: _rgb(0xfa, 0xb3, 0x87),
        success: _rgb(0xa6, 0xe3, 0xa1),
        warning: _rgb(0xf9, 0xe2, 0xaf),
        error: _rgb(0xf3, 0x8b, 0xa8),
        cacheDisk: _rgb(0x74, 0xc7, 0xec),
        curve: [
          _rgb(0x94, 0xe2, 0xd5),
          _rgb(0xa6, 0xe3, 0xa1),
          _rgb(0xf5, 0xc2, 0xe7),
          _rgb(0xf9, 0xe2, 0xaf),
        ],
        layer: LayerColours(
          footage: _rgb(0x61, 0x7d, 0xaf),
          sequence: _rgb(0x7e, 0x80, 0xb9),
          precomp: _rgb(0x8c, 0x74, 0xae),
          solid: _rgb(0x76, 0x7a, 0x91),
          text: _rgb(0x94, 0x87, 0x71),
          camera: _rgb(0xab, 0x7d, 0x65),
        ),
      );

  /// Catppuccin Latte (K-097).
  factory LumitTheme.catppuccinLatte() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xe6, 0xe9, 0xef),
        surface1: _rgb(0xef, 0xf1, 0xf5),
        surface2: _rgb(0xea, 0xed, 0xf2),
        surface3: _rgb(0xef, 0xf1, 0xf5),
        surface4: _rgb(0xdc, 0xe0, 0xe8),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x4c, 0x4f, 0x69),
        textSecondary: _rgb(0x5c, 0x5f, 0x77),
        textMuted: _rgb(0x8c, 0x8f, 0xa1),
        textDisabled: _rgb(0x9c, 0xa0, 0xb0),
        hairline: _rgb(0xcc, 0xd0, 0xda),
        hairlineStrong: _rgb(0xbc, 0xc0, 0xcc),
        accent: _rgb(0x88, 0x39, 0xef),
        accentHover: _rgb(0x6a, 0x2c, 0xba),
        // Provisional pending the light-mode pass.
        animated: _rgb(0x9c, 0x61, 0x14),
        success: _rgb(0x40, 0xa0, 0x2b),
        warning: _rgb(0xdf, 0x8e, 0x1d),
        error: _rgb(0xd2, 0x0f, 0x39),
        cacheDisk: _rgb(0x20, 0x9f, 0xb5),
        curve: [
          _rgb(0x17, 0x92, 0x99),
          _rgb(0x40, 0xa0, 0x2b),
          _rgb(0xea, 0x76, 0xcb),
          _rgb(0xdf, 0x8e, 0x1d),
        ],
        layer: LayerColours(
          footage: _rgb(0x2e, 0x5e, 0xc4),
          sequence: _rgb(0x51, 0x50, 0xd0),
          precomp: _rgb(0x76, 0x40, 0xc7),
          solid: _rgb(0x74, 0x77, 0x8c),
          text: _rgb(0xba, 0x7e, 0x30),
          camera: _rgb(0xd2, 0x5f, 0x22),
        ),
      );

  // --- Type scale (docs/15-DESIGN §density: 11 px body, 10 px small) -------

  static const String fontFamily = 'Hanken Grotesk';

  /// Hanken Grotesk has no Cyrillic; Inter stays bundled as the fallback so
  /// the Ukrainian and Kazakh locales keep a designed face instead of the
  /// platform default.
  static const List<String> fontFamilyFallback = ['Inter'];

  /// The face every number, timecode and container label is set in
  /// (docs/15-DESIGN.md §7.1, K-438). The fallbacks only matter if the bundled
  /// asset is missing, and both platform names resolve to a monospaced face.
  static const String monoFontFamily = 'Geist Mono';
  static const List<String> monoFontFamilyFallback = [
    'Consolas',
    'monospace',
  ];

  TextStyle get heading => TextStyle(
      fontFamily: fontFamily,
      fontFamilyFallback: fontFamilyFallback,
      fontSize: 16,
      color: textPrimary,
      decoration: TextDecoration.none,
      fontWeight: FontWeight.w500);
  TextStyle get body => TextStyle(
      fontFamily: fontFamily,
      fontFamilyFallback: fontFamilyFallback,
      fontSize: 11,
      color: textSecondary,
      decoration: TextDecoration.none,
      fontWeight: FontWeight.w400);
  TextStyle get bodyPrimary => body.copyWith(color: textPrimary);

  /// Medium-weight body for the few places docs/15-DESIGN §7.1 keeps
  /// emphasis: panel tab labels and dialog body emphasis. Everything else
  /// reads at regular weight.
  TextStyle get bodyStrong => body.copyWith(fontWeight: FontWeight.w500);
  TextStyle get small => TextStyle(
      fontFamily: fontFamily,
      fontFamilyFallback: fontFamilyFallback,
      fontSize: 10,
      color: textMuted,
      decoration: TextDecoration.none,
      fontWeight: FontWeight.w400);

  /// The note under a field explaining its format — smaller than a label so it
  /// reads as an aside rather than as another thing to fill in
  /// (docs/15-DESIGN.md §7.1).
  TextStyle get caption => small.copyWith(fontSize: 9);

  /// **Every container label** (docs/15-DESIGN.md §7.1, K-438): panel titles,
  /// section headers, column headers, tab labels, dialog titles, attribution.
  ///
  /// In plain terms, a kicker is the small capitalised word above a thing that
  /// says what the thing is. Lumit sets all of them in Geist Mono, small, with
  /// the letters spaced out, and quiet — so everything the *application* names
  /// looks unmistakably different from everything the *user* names, which stays
  /// sentence-case Hanken Grotesk.
  ///
  /// The capitals are the *style*, not the string: a widget upper-cases what it
  /// is handed, so the translated phrase in the arb file stays an ordinary
  /// sentence and no key has to be spelled twice.
  TextStyle get kicker => TextStyle(
        fontFamily: monoFontFamily,
        fontFamilyFallback: monoFontFamilyFallback,
        // 9px at +0.12em, regular weight — **the approved mockups' own
        // `.kick`** (K-451: the mockups' metrics are canonical), and the
        // bottom of §7.1's 9–11px / 0.08–0.12em band rather than its middle.
        // It was 10px at +0.10em in Medium, which read a size heavier than
        // every kicker the mockups draw. Flutter measures tracking in logical
        // pixels, so an em is the font size.
        fontSize: 9,
        // 1.08 written out, not `9 * 0.12`: the product is 0.12000000000000001
        // in binary floating point, which lands a hair outside the band the
        // spec states and the primitives test checks.
        letterSpacing: 1.08,
        color: textMuted,
        decoration: TextDecoration.none,
        fontWeight: FontWeight.w400,
      );

  /// The kicker of the container that is *in force* — the fronted dock tab, the
  /// section being edited. Only the colour changes: same face, same size, same
  /// tracking, same weight, so nothing shifts by a pixel when the active one
  /// moves (§7.1 — state reads from colour, never from size or weight).
  TextStyle get kickerOn => kicker.copyWith(color: textPrimary);
  TextStyle get mono => TextStyle(
      fontFamily: monoFontFamily,
      fontFamilyFallback: monoFontFamilyFallback,
      fontSize: 12,
      color: textSecondary,
      decoration: TextDecoration.none);

  /// The float shadow (menus, dialogs): rerun's offset 0/15, blur 50.
  List<BoxShadow> get floatShadow => const [
        BoxShadow(
            offset: Offset(0, 15), blurRadius: 50, color: Color(0x80000000)),
      ];

  /// The dimmed backdrop behind a true modal (Settings, the palette).
  Color get modalBackdrop => const Color(0x59000000);

  /// The one-click presets on the comp background swatch.
  ///
  /// These are **content**, not chrome: a comp background of black must be
  /// exactly black in a light theme as in a dark one, so unlike every other
  /// entry here they are the same in every scheme and are deliberately not
  /// mode-mirrored. They live in the theme all the same, because that is where
  /// colour values live (docs/15-DESIGN.md §4.1) — and black and white are what
  /// that control is reached for nine times in ten, so they are one click
  /// rather than a trip round the wheel.
  List<Color> get backgroundPresets =>
      const [Color(0xFF000000), Color(0xFFFFFFFF)];

  /// The six accents the Settings drawing offers as one click each: spruce in
  /// front, clay right behind it (one click back for anyone who preferred it),
  /// then a blue, a mint, an amber and a violet.
  ///
  /// The same six in every scheme, and deliberately: an accent is the one
  /// colour the user is choosing *for* the chrome rather than out of it, so it
  /// cannot be mirrored per mode without the swatch under the pointer changing
  /// meaning as the theme does. The full wheel is still a click away in the
  /// theme editor — these are the quick answers, not the whole answer.
  static const List<Color> accentPresets = [
    defaultAccent, // spruce - THE default (K-538)
    Color(0xFFE05A72), // clay - what it replaced; one click back
    Color(0xFF4AA3E0),
    Color(0xFF46C98E),
    Color(0xFFE0A33C),
    Color(0xFFA06CE0),
  ];
}

/// A colour that comes from the *document* (a solid's swatch, a comp
/// background) rather than the design system — the one sanctioned
/// constructor outside the scheme tables.
Color documentColour(int r, int g, int b, int a) => Color.fromARGB(a, r, g, b);

extension LumitMaterialTheme on ThemeData {
  static ThemeData fromLumitTheme(LumitTheme theme) {
    return ThemeData(
        brightness:
            theme.mode == ThemeMode2.dark ? Brightness.dark : Brightness.light,
        tooltipTheme: TooltipThemeData(
            textStyle: theme.small,
            decoration: BoxDecoration(
                color: theme.surface1,
                borderRadius: BorderRadius.circular(5),
                border: BoxBorder.all(color: theme.surface2, width: 1))),
        colorScheme: ColorScheme.fromSeed(
          seedColor: theme.accent,
          surface: theme.surface3,
          brightness: theme.mode == ThemeMode2.dark
              ? Brightness.dark
              : Brightness.light,
        ));
  }
}
