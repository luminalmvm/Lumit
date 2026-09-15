// The Lumit theme, ported one-for-one from crates/lumit-ui/src/theme.rs
// (docs/15-DESIGN.md). This is the ONLY Dart file where colour hex values may
// appear — everything else reads the LumitTheme object, mirroring the Rust
// no-hex-outside-theme rule.
//
// In plain terms: every colour, radius, gap and shadow the interface uses is a
// named token here: the seven colour schemes the Rust frontend ships, the
// twelve paired ones and Desk's two rooms added since, and the three shapes.
// The numbers the Rust frontend has are carried over digit-for-digit so the
// two can be compared.

import 'package:flutter/material.dart';
import 'package:lumit_flutter/l10n/strings.dart';

Color _rgb(int r, int g, int b) => Color.fromARGB(0xff, r, g, b);

/// Light vs dark colour family.
enum ThemeMode2 { dark, light }

/// The three arrangements the chrome can be laid out in. A shape names an
/// arrangement; a scheme names a palette, and the two are chosen apart.
enum ThemeShape {
  /// Panels meeting flush, hairlines between them: the default.
  studio,

  /// The same flush panels on a four-pixel module, with lowercase labels.
  desk,

  /// Panes as cards standing in a room, with a gap and a shadow round each.
  lantern,
}

/// How a container label is cased before it is drawn.
enum LabelCase {
  /// Every letter a capital.
  caps,

  /// Every letter small.
  lower,

  /// Left as the string was written.
  sentence,
}

/// How much UI-chrome motion to show.
enum AnimationLevel { all, minimal, none }

/// The duration owned widgets animate with under a level. All ≈ the egui
/// 120 ms micro-motion budget; None is instant.
Duration animationDuration(AnimationLevel level) => switch (level) {
      AnimationLevel.all => const Duration(milliseconds: 120),
      AnimationLevel.minimal => const Duration(milliseconds: 50),
      AnimationLevel.none => Duration.zero,
    };

/// Every named colour scheme Lumit ships, in picker order.
enum LumitColorScheme {
  dark,
  darkBlue,
  light,
  gruvboxDark,
  gruvboxLight,
  catppuccinMocha,
  catppuccinLatte,
  mallowDark,
  mallowLight,
  glacierDark,
  glacierLight,
  hearthDark,
  hearthLight,
  chalkDark,
  chalkLight,
  vellumDark,
  vellumLight,
  neonDark,
  neonLight,
  canopyDark,
  canopyLight,
  slateDark,
  slateLight,
  nocturneDark,
  nocturneLight,
  tavernDark,
  tavernLight,
  arcaneDark,
  arcaneLight,
  giltDark,
  giltLight,
  greyRoom,
  graphite;

  /// Sentence-case display name for menus and settings.
  String get label => switch (this) {
        LumitColorScheme.dark => l10n.schemeDark,
        LumitColorScheme.darkBlue => l10n.schemeDarkBlue,
        LumitColorScheme.light => l10n.schemeLight,
        LumitColorScheme.gruvboxDark => 'Gruvbox dark',
        LumitColorScheme.gruvboxLight => 'Gruvbox light',
        LumitColorScheme.catppuccinMocha => 'Catppuccin Mocha',
        LumitColorScheme.catppuccinLatte => 'Catppuccin Latte',
        LumitColorScheme.mallowDark => 'Mallow dark',
        LumitColorScheme.mallowLight => 'Mallow light',
        LumitColorScheme.glacierDark => 'Glacier dark',
        LumitColorScheme.glacierLight => 'Glacier light',
        LumitColorScheme.hearthDark => 'Hearth dark',
        LumitColorScheme.hearthLight => 'Hearth light',
        LumitColorScheme.chalkDark => 'Chalk dark',
        LumitColorScheme.chalkLight => 'Chalk light',
        LumitColorScheme.vellumDark => 'Vellum dark',
        LumitColorScheme.vellumLight => 'Vellum light',
        LumitColorScheme.neonDark => 'Neon dark',
        LumitColorScheme.neonLight => 'Neon light',
        LumitColorScheme.canopyDark => 'Canopy dark',
        LumitColorScheme.canopyLight => 'Canopy light',
        LumitColorScheme.slateDark => 'Slate dark',
        LumitColorScheme.slateLight => 'Slate light',
        LumitColorScheme.nocturneDark => 'Nocturne dark',
        LumitColorScheme.nocturneLight => 'Nocturne light',
        LumitColorScheme.tavernDark => 'Tavern dark',
        LumitColorScheme.tavernLight => 'Tavern light',
        LumitColorScheme.arcaneDark => 'Arcane dark',
        LumitColorScheme.arcaneLight => 'Arcane light',
        LumitColorScheme.giltDark => 'Gilt dark',
        LumitColorScheme.giltLight => 'Gilt light',
        LumitColorScheme.greyRoom => 'Grey room',
        LumitColorScheme.graphite => 'Graphite',
      };

  ThemeMode2 get mode => switch (this) {
        LumitColorScheme.light ||
        LumitColorScheme.gruvboxLight ||
        LumitColorScheme.catppuccinLatte ||
        LumitColorScheme.mallowLight ||
        LumitColorScheme.glacierLight ||
        LumitColorScheme.hearthLight ||
        LumitColorScheme.chalkLight ||
        LumitColorScheme.vellumLight ||
        LumitColorScheme.neonLight ||
        LumitColorScheme.canopyLight ||
        LumitColorScheme.slateLight ||
        LumitColorScheme.nocturneLight ||
        LumitColorScheme.tavernLight ||
        LumitColorScheme.arcaneLight ||
        LumitColorScheme.giltLight ||
        LumitColorScheme.greyRoom =>
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
        LumitColorScheme.mallowDark => LumitTheme.mallowDark(),
        LumitColorScheme.mallowLight => LumitTheme.mallowLight(),
        LumitColorScheme.glacierDark => LumitTheme.glacierDark(),
        LumitColorScheme.glacierLight => LumitTheme.glacierLight(),
        LumitColorScheme.hearthDark => LumitTheme.hearthDark(),
        LumitColorScheme.hearthLight => LumitTheme.hearthLight(),
        LumitColorScheme.chalkDark => LumitTheme.chalkDark(),
        LumitColorScheme.chalkLight => LumitTheme.chalkLight(),
        LumitColorScheme.vellumDark => LumitTheme.vellumDark(),
        LumitColorScheme.vellumLight => LumitTheme.vellumLight(),
        LumitColorScheme.neonDark => LumitTheme.neonDark(),
        LumitColorScheme.neonLight => LumitTheme.neonLight(),
        LumitColorScheme.canopyDark => LumitTheme.canopyDark(),
        LumitColorScheme.canopyLight => LumitTheme.canopyLight(),
        LumitColorScheme.slateDark => LumitTheme.slateDark(),
        LumitColorScheme.slateLight => LumitTheme.slateLight(),
        LumitColorScheme.nocturneDark => LumitTheme.nocturneDark(),
        LumitColorScheme.nocturneLight => LumitTheme.nocturneLight(),
        LumitColorScheme.tavernDark => LumitTheme.tavernDark(),
        LumitColorScheme.tavernLight => LumitTheme.tavernLight(),
        LumitColorScheme.arcaneDark => LumitTheme.arcaneDark(),
        LumitColorScheme.arcaneLight => LumitTheme.arcaneLight(),
        LumitColorScheme.giltDark => LumitTheme.giltDark(),
        LumitColorScheme.giltLight => LumitTheme.giltLight(),
        LumitColorScheme.greyRoom => LumitTheme.greyRoom(),
        LumitColorScheme.graphite => LumitTheme.graphite(),
      };
}

/// Shape-dependent chrome geometry. `studio` reproduces the egui frontend's
/// original numbers exactly; `desk` keeps them on a tighter float; `lantern`
/// is the card-in-a-room system (docs/design-alt/15-DESIGN-LANTERN.md §12).
class ShapeTokens {
  /// A control's corner radius: button, chip, tab, dropdown, value box.
  ///
  /// It is a **corner radius only**. Never use it as a length (a padding, a
  /// height, an inset). Where a control is a capsule the shape says so
  /// through [actionRadius], which may be [stadium] rather than a number.
  final double controlRadius;
  final double floatRadius;
  final double cardRadius;
  final double cardPadding;
  final double tileGap;
  final double windowInset;
  final List<BoxShadow> cardShadow;

  /// How a container label is cased before it is drawn.
  final LabelCase labelCase;

  /// Whether a panel title sits centred on its strip with a dot at the corner.
  final bool titleCentred;

  /// Whether a panel header carries the small accent dot before its title.
  final bool headerDot;

  /// The stroke an icon is drawn with, in logical pixels.
  final double strokeWeight;

  /// The corner of a band inside a pane: a stage, a lane area, an effect's rows.
  final double sectionRadius;

  /// The corner of an action: buttons, chips, badges and active segments,
  /// which may be [stadium].
  final double actionRadius;

  /// The corner of a well: value boxes and text fields.
  final double wellRadius;

  /// The corner of content on a lane: layer bars and clips.
  final double contentRadius;

  /// Whether panes are cards standing on a room colour rather than flush tiles.
  final bool roomed;

  /// The margin between a pill and the filled state drawn inside it, on every
  /// side. The inner corner radius is the outer minus this, so the margin
  /// stays the same the whole way round.
  final double pillInset;

  /// The face for words, and the face for text that is machine output (a
  /// path, an expression, the boot log). Desk sets one face for everything
  /// and keeps its mono cut for the machine's text only.
  final String sansFamily;
  final String monoFamily;

  /// Whether numbers stand in the sans with tabular figures, the way Desk
  /// sets timecode and values, rather than in the mono face.
  final bool tabularNumbers;

  /// The kicker's tracking in logical pixels at its 9px: 1.08 for the caps
  /// label (+0.12em), 0.36 for Desk's lowercase one (+0.04em).
  final double kickerTracking;
  const ShapeTokens({
    required this.controlRadius,
    required this.floatRadius,
    required this.cardRadius,
    required this.cardPadding,
    required this.tileGap,
    required this.windowInset,
    required this.cardShadow,
    required this.labelCase,
    required this.titleCentred,
    required this.headerDot,
    required this.strokeWeight,
    required this.sectionRadius,
    required this.actionRadius,
    required this.wellRadius,
    required this.contentRadius,
    required this.roomed,
    this.pillInset = 0,
    this.sansFamily = LumitTheme.fontFamily,
    this.monoFamily = LumitTheme.monoFontFamily,
    this.tabularNumbers = false,
    this.kickerTracking = 1.08,
  });

  /// The value a capsule radius carries: bigger than any control is tall, so
  /// the corner clamps to half the height and draws a stadium. A rounded
  /// rectangle scales its radii down to fit its own box, which is why one
  /// number serves every height.
  static const double stadium = 1000;

  static const studio = ShapeTokens(
    // The mockups draw 2 on almost every control (measured from the
    // computed-style manifests); 4 was the original guess.
    controlRadius: 2,
    floatRadius: 6,
    cardRadius: 0,
    cardPadding: 0,
    tileGap: 1.0,
    windowInset: 0.0,
    cardShadow: [],
    labelCase: LabelCase.caps,
    titleCentred: false,
    headerDot: false,
    strokeWeight: 1.5,
    sectionRadius: 2,
    actionRadius: 2,
    wellRadius: 2,
    contentRadius: 2,
    roomed: false,
    pillInset: 0,
  );

  static const desk = ShapeTokens(
    // Square everywhere: an instrument's plates and wells have no radius,
    // and the panels sit as plates in a 4px chassis of the ground.
    controlRadius: 0,
    floatRadius: 0,
    cardRadius: 0,
    cardPadding: 0,
    tileGap: 4.0,
    windowInset: 4.0,
    cardShadow: [],
    labelCase: LabelCase.lower,
    titleCentred: false,
    headerDot: false,
    strokeWeight: 1.25,
    sectionRadius: 0,
    actionRadius: 0,
    wellRadius: 0,
    contentRadius: 0,
    roomed: false,
    pillInset: 0,
    // One face, tabular figures, and a lowercase label at +0.04em
    // (docs/design-alt/15-DESIGN-DESK.md 7.2).
    sansFamily: 'IBM Plex Sans',
    monoFamily: 'IBM Plex Mono',
    tabularNumbers: true,
    kickerTracking: 0.36,
  );

  static const lantern = ShapeTokens(
    controlRadius: 7,
    floatRadius: 12,
    cardRadius: 16,
    cardPadding: 0,
    tileGap: 10.0,
    windowInset: 10.0,
    cardShadow: [
      // Wide and faint, so the shadow fades round the corner instead of
      // stopping where the straight edge does.
      BoxShadow(
        offset: Offset(0, 3),
        blurRadius: 12,
        color: Color(0x1A000000),
      ),
      BoxShadow(
        offset: Offset(0, 1),
        blurRadius: 2,
        color: Color(0x10000000),
      ),
    ],
    labelCase: LabelCase.caps,
    titleCentred: true,
    headerDot: true,
    strokeWeight: 1.5,
    sectionRadius: 12,
    actionRadius: stadium,
    wellRadius: 7,
    contentRadius: 3,
    roomed: true,
    pillInset: 3,
  );

  static ShapeTokens of(ThemeShape shape) => switch (shape) {
        ThemeShape.studio => studio,
        ThemeShape.desk => desk,
        ThemeShape.lantern => lantern,
      };
}

/// **How much room a row gets** (docs/15-DESIGN.md §12A.6).
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
  /// **The Timeline's own chrome is no longer one of these**: its two rows are
  /// [timelineChromeRow] and [timelineHeaderRow], which under Regular stand
  /// taller than a secondary row anywhere else.
  final double secondaryRow;

  /// The Timeline's first chrome row — the timecode and frame readouts, the
  /// layer search, and the Layers / Keys / Graph tabs.
  ///
  /// **24 under Regular**, where every other secondary row is 19: the owner's
  /// ruling from desktop testing is that this row is aimed at constantly and
  /// was too small to hit comfortably. Compact keeps the 18 it always drew.
  final double timelineChromeRow;

  /// The Timeline's second chrome row — the column-group headers in Layers
  /// mode, the dope sheet's filters in Keys mode, the graph's in Graph mode.
  /// **23 under Regular**, 18 under Compact.
  ///
  /// It is a separate number from [timelineChromeRow] because the two rows do
  /// different work: the row above is aimed at, this one is mostly read.
  final double timelineHeaderRow;

  /// How tall a control standing in either Timeline chrome row is *told* to
  /// be, or **null for "measure yourself"**.
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

  /// A panel's title strip, and the Viewer's two bars.
  final double headerStrip;

  /// The cache stripe on the ruler's floor.
  final double cacheBar;

  /// The time navigator strip above the ruler.
  final double navigatorBand;

  /// The gutter scrollbar's thickness.
  final double scrollbar;

  /// The top line's height: the menu bar under Studio and Desk, the room's
  /// own band under Lantern.
  final double menuBar;

  const DensityTokens({
    required this.laneRow,
    required this.secondaryRow,
    required this.timelineChromeRow,
    required this.timelineHeaderRow,
    required this.timelineChromeControl,
    required this.inRowPicker,
    required this.dropdownFace,
    required this.propertyRow,
    required this.headerStrip,
    required this.cacheBar,
    required this.navigatorBand,
    required this.scrollbar,
    required this.menuBar,
  });

  /// The Timeline ruler, which is **derived and not declared**: the lane side
  /// gives the ruler exactly the height the outline side spends on its two
  /// chrome rows, and that is the whole reason the two halves of the panel
  /// line up row for row. Grow either row and the ruler grows with it, which is
  /// how Regular's ruler reached **47** from 38 while Compact keeps its 36.
  ///
  /// The extra room goes to the clock: the ruler's two halves are no longer
  /// ruled apart, so what the reader sees is one taller band with the labels
  /// near its top and the markers and work area on its floor.
  double get ruler => timelineChromeRow + timelineHeaderRow;

  /// What the mockups render, with the Timeline's chrome at the height the
  /// owner asked for after desktop testing. The default.
  static const regular = DensityTokens(
    laneRow: 23,
    secondaryRow: 19,
    timelineChromeRow: 24,
    timelineHeaderRow: 23,
    timelineChromeControl: 20,
    inRowPicker: 18,
    dropdownFace: 20,
    propertyRow: 27,
    headerStrip: 22,
    cacheBar: 3,
    navigatorBand: 12,
    scrollbar: 7,
    menuBar: 26,
  );

  /// A pixel or two off each row, for more visible at once. What the app drew
  /// before the setting existed — **including the Timeline's chrome**, which
  /// grew under Regular alone.
  static const compact = DensityTokens(
    laneRow: 22,
    secondaryRow: 18,
    timelineChromeRow: 18,
    timelineHeaderRow: 18,
    timelineChromeControl: null,
    inRowPicker: 16,
    dropdownFace: 18,
    propertyRow: 26,
    headerStrip: 22,
    cacheBar: 3,
    navigatorBand: 12,
    scrollbar: 7,
    menuBar: 26,
  );

  static DensityTokens of(bool isCompact) => isCompact ? compact : regular;

  /// Desk's module: every row a multiple of four
  /// (docs/design-alt/15-DESIGN-DESK.md §7.1).
  static const deskRegular = DensityTokens(
    laneRow: 24,
    secondaryRow: 20,
    timelineChromeRow: 24,
    timelineHeaderRow: 24,
    timelineChromeControl: 20,
    inRowPicker: 16,
    dropdownFace: 20,
    propertyRow: 28,
    headerStrip: 24,
    cacheBar: 4,
    navigatorBand: 16,
    scrollbar: 8,
    menuBar: 28,
  );

  /// Desk compact: the row and the property row each drop one module, and
  /// nothing else moves.
  static const deskCompact = DensityTokens(
    laneRow: 20,
    secondaryRow: 20,
    timelineChromeRow: 24,
    timelineHeaderRow: 24,
    timelineChromeControl: 20,
    inRowPicker: 16,
    dropdownFace: 20,
    propertyRow: 24,
    headerStrip: 24,
    cacheBar: 4,
    navigatorBand: 16,
    scrollbar: 8,
    menuBar: 28,
  );

  /// Lantern's rows: the surveyed set with a 28 row pitch (26 drawn and a 2px
  /// gap) and a 36 card title line. The band is 40 rather than the 44 the
  /// document draws (docs/design-alt/15-DESIGN-LANTERN.md 12B.2), so the band
  /// and the 44 tool strip under it fit inside 84 together.
  static const lanternRegular = DensityTokens(
    laneRow: 28,
    secondaryRow: 19,
    timelineChromeRow: 24,
    timelineHeaderRow: 23,
    timelineChromeControl: 20,
    inRowPicker: 18,
    dropdownFace: 20,
    propertyRow: 27,
    headerStrip: 36,
    cacheBar: 4,
    navigatorBand: 12,
    scrollbar: 7,
    menuBar: 40,
  );

  /// Lantern compact: the surveyed compact set under the same row pitch and
  /// card title line.
  static const lanternCompact = DensityTokens(
    laneRow: 28,
    secondaryRow: 18,
    timelineChromeRow: 18,
    timelineHeaderRow: 18,
    timelineChromeControl: null,
    inRowPicker: 16,
    dropdownFace: 18,
    propertyRow: 26,
    headerStrip: 36,
    cacheBar: 4,
    navigatorBand: 12,
    scrollbar: 7,
    menuBar: 40,
  );

  /// The pair a shape draws on.
  static DensityTokens forShape(ThemeShape shape, bool isCompact) =>
      switch (shape) {
        ThemeShape.studio => of(isCompact),
        ThemeShape.desk => isCompact ? deskCompact : deskRegular,
        ThemeShape.lantern => isCompact ? lanternCompact : lanternRegular,
      };
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

/// Colours the Scopes panel draws with (15-DESIGN §8). One fixed set shared by
/// every theme — a scope is always read on a near-black graticule, whatever the
/// chrome, the same grading-accuracy reasoning that keeps `viewerSurround`
/// neutral.
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

/// What the fault box is painted in when a panel's build throws.
///
/// One fixed set, and it **cannot** come off the theme struct even though every
/// other colour does. The widget wearing these replaces one that has just
/// failed to build, and reading an inherited widget is among the ways a build
/// fails — so a fault box that asked for the theme could be the second thing to
/// throw, and Flutter's answer to an error widget that errors is to give up on
/// the frame. It asks for nothing, so it cannot.
///
/// Dark, in a theme that may be light: the box is a fault report rather than
/// chrome, it is legible either way, and dark-first is the house adaptation.
/// [detail] carries the exception's own words, which are the engineer's half of
/// the box and are deliberately quieter than the line above them.
class FaultColours {
  final Color background, heading, detail;
  const FaultColours({
    required this.background,
    required this.heading,
    required this.detail,
  });

  static const standard = FaultColours(
    background: Color(0xff1b1416),
    heading: Color(0xffd1729c),
    detail: Color(0xff9a9094),
  );
}

/// What a wire and its sockets are painted in on the Graph panel's canvas
/// (15-DESIGN §4.1/§12A.7). Seven port types wear **five** colours, grouped as
/// the approved NodeGraph drawing's legend groups them: image·matte, number,
/// colour, shape·points, audio.
///
/// One fixed set shared by every theme, and **deliberately not an editable
/// token** — the same reasoning that keeps `viewerSurround` out of the
/// editor's reach. Colour *is* the legend here: the strip along the canvas's
/// bottom edge, the manual and the drawings all say "amber is a number", so a
/// palette taste could retint would be a legend that lies. The five are the
/// layer palette's own azure, amber, magenta, teal and mint, which were picked
/// to be told apart at a glance and already read on both grounds.
class PortColours {
  /// Image and matte — the picture's own path down the stack.
  final Color image;

  /// Number: the type nearly every driven parameter is.
  final Color number;
  final Color colour;

  /// Shape and points — geometry, whether it is one outline or a stream of
  /// thousands.
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
/// zoom and to stack its bands — §6.4's standing direction is that each
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

  /// How much room a row gets. It rides on the theme because that is what every
  /// widget already has in hand, and because changing it has to repaint
  /// everything at once — the same journey a shape change makes.
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

  /// "This is animated or in hand": keyframe diamonds, stopwatch-on, selected
  /// keyframes, selected gizmo handles, the focused value field and the
  /// work-area band — and nothing else. The job list is closed, so a third kind
  /// of use is a mistake rather than a new job. A desaturated warm amber,
  /// deliberately quieter than `accent`: "keyed" is a state a composition is
  /// full of, while the accent marks the one thing in hand.
  final Color animated;
  final Color success, warning, error, cacheDisk;

  /// Graph-editor curve strokes.
  final List<Color> curve;
  final LayerColours layer;

  /// The Timeline's ground *outside* the work area. The lane, layer and graph
  /// areas are read as one long strip, and until this existed there was nothing
  /// to tell "the part you are delivering" from the rest — so the whole strip
  /// sat at one value and a selected row had only `surface2` to stand out
  /// against. Inside the work area the strip keeps `surface1`; this is the wash
  /// either side of it.
  final Color timelineOutOfRange;

  /// The fill under a selected row, and at reduced strength under a
  /// highlighted one. Its own token rather than `surface2` reused: a selection
  /// has to out-contrast the ground it sits on, and on a light scheme that
  /// means going *darker* where the surfaces go lighter — a rule the surface
  /// ramp cannot express because it is a ramp.
  final Color selectionFill;

  /// What waveforms draw in — the single wave and the three bands of the
  /// multiwave stack. Its own grouping rather than roles borrowed one at a
  /// time, per the §6.4 direction.
  final WaveformColours waveform;

  /// What the Graph panel's wires and sockets draw in, by port type. Fixed
  /// rather than per-scheme, and not offered to the theme editor — see
  /// [PortColours].
  final PortColours port;

  /// Comp markers on the time ruler. A plain grey, not a role colour: a marker
  /// says *here*, not *good* or *careful*, and the ruler already has the accent
  /// doing the work area. Light on a dark scheme and dark on a light one —
  /// After Effects' own reading, and the one that stays legible over the
  /// work-area band either way.
  final Color marker;

  /// The wash a modal window lays over the app behind it. Its own token because
  /// it is not a surface: it is the *absence* of attention, a translucent black
  /// that dims whatever it covers rather than a colour the ramp could supply.
  /// Translucent black under a light scheme too — dimming is dimming, and a
  /// pale scrim over pale panels would say nothing.
  final Color scrim;

  /// The ground panes stand on when the shape is roomed. A light neutral for
  /// a dark scheme, so the cards read as objects lit inside a room; a light
  /// scheme's own canvas otherwise. Studio and Desk never draw it.
  final Color room;

  /// The three Timeline tokens default from the mode rather than being spelled
  /// out by every scheme: they are a *relationship* to the surface ramp (a
  /// shade beyond `surface1`, a fill that out-contrasts it, a grey that reads
  /// against it), and every scheme restating that relationship would be one
  /// more chance to get it wrong. A custom theme, or any scheme that wants its
  /// own, passes them explicitly.
  LumitTheme({
    required this.mode,
    this.shape = ThemeShape.studio,
    this.tokens = ShapeTokens.studio,
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
    Color? room,
    WaveformColours? waveform,
  })  : timelineOutOfRange =
            timelineOutOfRange ?? defaultOutOfRange(mode, surface1),
        selectionFill = selectionFill ?? defaultSelectionFill(mode, surface2),
        marker = marker ?? defaultMarker(mode),
        scrim = scrim ?? defaultScrim(mode),
        room = room ?? defaultRoom(mode, surface0),
        waveform = waveform ?? defaultWaveform(mode);

  /// The room a dark scheme's cards stand in: the light neutral the Lantern
  /// drawing calls the day room. A light scheme already is a room, so its own
  /// canvas serves.
  static Color defaultRoom(ThemeMode2 mode, Color surface0) =>
      mode == ThemeMode2.dark ? dayRoom : nightRoom;

  /// The two rooms Lantern's cards can stand in: a light neutral the dark
  /// schemes get by default, and a near-black the light schemes get, so the
  /// cards are always the other way round from the room.
  static Color get dayRoom => _rgb(0xd9, 0xd9, 0xd6);
  static Color get nightRoom => _rgb(0x0b, 0x0c, 0x0e);

  /// Words that stand on the room itself (the menus of Lantern's top band)
  /// take their ink from the room, not from the scheme: dark on a light
  /// room, light on a dark one.
  Color get roomInk => room.computeLuminance() > 0.35
      ? _rgb(0x1a, 0x1d, 0x20)
      : _rgb(0xee, 0xf1, 0xf2);

  /// The ground outside the work area: a step *away* from the surface ramp's
  /// direction — darker under a dark scheme, and darker again under a light
  /// one, because on white the only direction with anywhere to go is down.
  /// Deliberately gentle: this marks a region, it is not a border.
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

  /// **Spruce** — THE default accent (after the brand went green with it; clay
  /// is one click away). The stock dark and light schemes both build their
  /// accent pair from it, so a single edit here moves the whole family. Hover
  /// derives by the same ±0x12 step [withAccent] gives a user-picked accent,
  /// which is what makes `withAccent(defaultAccent)` reproduce each stock
  /// scheme's pair exactly rather than approximately — now in **both** schemes,
  /// since spruce clears the contrast floor on white and Light no longer needs
  /// a hand-darkened accent of its own.
  static const defaultAccent = Color(0xff35785e);

  /// The layer-label palette (TL2): bright, clearly distinct chips. A layer's
  /// label colours both its swatch and its bar in the lane area, and each layer
  /// kind defaults to a different one — so the set is a dedicated palette, not
  /// the theme's role colours, which were built to be quiet rather than
  /// tellable-apart. Index 0 is the neutral default; the Null layer took the
  /// ninth chip, since every one below it was already a kind's own.
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
  /// channel on a dark surface, darkens by the same on a light one.
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
    Color? room,
  }) =>
      LumitTheme(
        mode: mode,
        shape: shape ?? this.shape,
        tokens: tokens ?? this.tokens,
        density: density ?? this.density,
        room: room ?? this.room,
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
        accent: defaultAccent, // spruce #35785e
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

  /// The older ramp: bluer, a step lighter; everything else shared with dark().
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

  /// The light ramp: one uniform light theme.
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
        accent: defaultAccent, // spruce #35785e
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

  /// Gruvbox dark.
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

  /// Gruvbox light.
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

  /// Catppuccin Mocha.
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

  /// Catppuccin Latte.
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

  /// Mallow dark.
  factory LumitTheme.mallowDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x1c, 0x19, 0x20),
        surface1: _rgb(0x26, 0x21, 0x29),
        surface2: _rgb(0x2f, 0x29, 0x33),
        surface3: _rgb(0x38, 0x31, 0x40),
        surface4: _rgb(0x44, 0x3b, 0x4d),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xf2, 0xec, 0xf2),
        textSecondary: _rgb(0xd6, 0xcc, 0xd8),
        textMuted: _rgb(0xa9, 0x9b, 0xb1),
        textDisabled: _rgb(0x7c, 0x6d, 0x84),
        hairline: _rgb(0x36, 0x2f, 0x3c),
        hairlineStrong: _rgb(0x82, 0x74, 0x8b),
        accent: _rgb(0xbd, 0xa3, 0xf2),
        accentHover: _rgb(0xcf, 0xb5, 0xff),
        animated: _rgb(0xd8, 0xb0, 0x71),
        success: _rgb(0x74, 0xc0, 0x9a),
        warning: _rgb(0xf8, 0xd1, 0x98),
        error: _rgb(0xc8, 0x67, 0x92),
        cacheDisk: _rgb(0x6e, 0xb3, 0xde),
        curve: [
          _rgb(0x72, 0xd5, 0xe1),
          _rgb(0xbd, 0xcd, 0x95),
          _rgb(0xec, 0xa8, 0xc6),
          _rgb(0xdf, 0xcb, 0x94),
        ],
        layer: LayerColours(
          footage: _rgb(0x6a, 0x8b, 0x9a),
          sequence: _rgb(0x6f, 0x70, 0x88),
          precomp: _rgb(0x6f, 0x57, 0x68),
          solid: _rgb(0x4d, 0x4a, 0x50),
          text: _rgb(0xbb, 0xb0, 0x97),
          camera: _rgb(0xae, 0x97, 0x85),
        ),
      );

  /// Mallow light.
  factory LumitTheme.mallowLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xeb, 0xe4, 0xec),
        surface1: _rgb(0xfb, 0xf8, 0xfb),
        surface2: _rgb(0xf3, 0xee, 0xf4),
        surface3: _rgb(0xfd, 0xfb, 0xfd),
        surface4: _rgb(0xe6, 0xdd, 0xe8),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x2b, 0x24, 0x30),
        textSecondary: _rgb(0x4d, 0x43, 0x54),
        textMuted: _rgb(0x76, 0x6a, 0x7d),
        textDisabled: _rgb(0x93, 0x86, 0x9a),
        hairline: _rgb(0xdd, 0xd4, 0xde),
        hairlineStrong: _rgb(0x8f, 0x83, 0x97),
        accent: _rgb(0x7e, 0x59, 0xab),
        accentHover: _rgb(0x6c, 0x47, 0x99),
        animated: _rgb(0x9a, 0x6f, 0x27),
        success: _rgb(0x2e, 0x80, 0x5c),
        warning: _rgb(0xb4, 0x82, 0x37),
        error: _rgb(0x9c, 0x30, 0x66),
        cacheDisk: _rgb(0x2a, 0x79, 0xa1),
        curve: [
          _rgb(0x02, 0x84, 0x8f),
          _rgb(0x6a, 0x7e, 0x40),
          _rgb(0xa5, 0x5a, 0x7d),
          _rgb(0x8d, 0x7b, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x5b, 0x82, 0x92),
          sequence: _rgb(0x64, 0x66, 0x81),
          precomp: _rgb(0x67, 0x4d, 0x60),
          solid: _rgb(0x44, 0x41, 0x47),
          text: _rgb(0xb1, 0xa5, 0x89),
          camera: _rgb(0xa6, 0x8c, 0x77),
        ),
      );

  /// Glacier dark.
  factory LumitTheme.glacierDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x0c, 0x10, 0x14),
        surface1: _rgb(0x12, 0x18, 0x1e),
        surface2: _rgb(0x18, 0x20, 0x27),
        surface3: _rgb(0x1e, 0x28, 0x30),
        surface4: _rgb(0x28, 0x34, 0x3e),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xe9, 0xf0, 0xf5),
        textSecondary: _rgb(0xbc, 0xca, 0xd4),
        textMuted: _rgb(0x83, 0x97, 0xa5),
        textDisabled: _rgb(0x5d, 0x6e, 0x7a),
        hairline: _rgb(0x22, 0x2c, 0x35),
        hairlineStrong: _rgb(0x5d, 0x72, 0x80),
        accent: _rgb(0x54, 0xca, 0xe4),
        accentHover: _rgb(0x66, 0xdc, 0xf6),
        animated: _rgb(0xdf, 0xb5, 0x6a),
        success: _rgb(0x4f, 0xb9, 0x85),
        warning: _rgb(0xf7, 0xd4, 0x6a),
        error: _rgb(0xd1, 0x56, 0x8b),
        cacheDisk: _rgb(0x56, 0x9c, 0xd4),
        curve: [
          _rgb(0x62, 0xd8, 0xdb),
          _rgb(0xaf, 0xd1, 0x95),
          _rgb(0xec, 0xa7, 0xd2),
          _rgb(0xdb, 0xcc, 0x94),
        ],
        layer: LayerColours(
          footage: _rgb(0x6c, 0x8c, 0x92),
          sequence: _rgb(0x65, 0x73, 0x86),
          precomp: _rgb(0x60, 0x5c, 0x6f),
          solid: _rgb(0x45, 0x4c, 0x51),
          text: _rgb(0xb8, 0xb0, 0x9b),
          camera: _rgb(0xae, 0x96, 0x8a),
        ),
      );

  /// Glacier light.
  factory LumitTheme.glacierLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xe3, 0xe8, 0xec),
        surface1: _rgb(0xf7, 0xf9, 0xfb),
        surface2: _rgb(0xee, 0xf2, 0xf5),
        surface3: _rgb(0xfb, 0xfc, 0xfd),
        surface4: _rgb(0xd9, 0xe0, 0xe6),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x16, 0x20, 0x2a),
        textSecondary: _rgb(0x3a, 0x49, 0x54),
        textMuted: _rgb(0x60, 0x72, 0x80),
        textDisabled: _rgb(0x7d, 0x8c, 0x98),
        hairline: _rgb(0xd2, 0xda, 0xe0),
        hairlineStrong: _rgb(0x7c, 0x8a, 0x96),
        accent: _rgb(0x00, 0x75, 0xc9),
        accentHover: _rgb(0x00, 0x63, 0xb7),
        animated: _rgb(0xa1, 0x74, 0x23),
        success: _rgb(0x00, 0x83, 0x53),
        warning: _rgb(0xab, 0x87, 0x20),
        error: _rgb(0xa7, 0x1f, 0x63),
        cacheDisk: _rgb(0x1e, 0x73, 0xa8),
        curve: [
          _rgb(0x00, 0x7f, 0x82),
          _rgb(0x5b, 0x81, 0x41),
          _rgb(0xa4, 0x59, 0x8a),
          _rgb(0x89, 0x7c, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x5c, 0x82, 0x89),
          sequence: _rgb(0x58, 0x6a, 0x7f),
          precomp: _rgb(0x57, 0x52, 0x68),
          solid: _rgb(0x3c, 0x43, 0x47),
          text: _rgb(0xae, 0xa5, 0x8d),
          camera: _rgb(0xa6, 0x8b, 0x7d),
        ),
      );

  /// Hearth dark.
  factory LumitTheme.hearthDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x1a, 0x15, 0x12),
        surface1: _rgb(0x24, 0x1d, 0x18),
        surface2: _rgb(0x2d, 0x25, 0x1f),
        surface3: _rgb(0x37, 0x2e, 0x27),
        surface4: _rgb(0x44, 0x39, 0x30),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xf3, 0xeb, 0xe2),
        textSecondary: _rgb(0xd9, 0xcc, 0xbe),
        textMuted: _rgb(0xa9, 0x98, 0x89),
        textDisabled: _rgb(0x7d, 0x6e, 0x62),
        hairline: _rgb(0x34, 0x29, 0x1f),
        hairlineStrong: _rgb(0x82, 0x72, 0x5f),
        accent: _rgb(0xe4, 0x81, 0x58),
        accentHover: _rgb(0xf6, 0x93, 0x6a),
        animated: _rgb(0xd2, 0xb2, 0x6c),
        success: _rgb(0x76, 0xb3, 0x86),
        warning: _rgb(0xf0, 0xc6, 0x75),
        error: _rgb(0xc2, 0x59, 0x80),
        cacheDisk: _rgb(0x49, 0xa6, 0xc4),
        curve: [
          _rgb(0x73, 0xd0, 0xc9),
          _rgb(0xb7, 0xc8, 0x90),
          _rgb(0xe9, 0xa2, 0xbc),
          _rgb(0xd5, 0xc6, 0x8f),
        ],
        layer: LayerColours(
          footage: _rgb(0x66, 0x8d, 0x92),
          sequence: _rgb(0x6c, 0x71, 0x89),
          precomp: _rgb(0x72, 0x57, 0x64),
          solid: _rgb(0x50, 0x4a, 0x46),
          text: _rgb(0xbd, 0xaf, 0x97),
          camera: _rgb(0xb2, 0x95, 0x88),
        ),
      );

  /// Hearth light.
  factory LumitTheme.hearthLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xec, 0xe5, 0xdc),
        surface1: _rgb(0xfb, 0xf7, 0xf1),
        surface2: _rgb(0xf3, 0xed, 0xe5),
        surface3: _rgb(0xfd, 0xfa, 0xf6),
        surface4: _rgb(0xe4, 0xdb, 0xcf),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x2a, 0x22, 0x1c),
        textSecondary: _rgb(0x4e, 0x43, 0x3a),
        textMuted: _rgb(0x78, 0x6a, 0x5e),
        textDisabled: _rgb(0x97, 0x8a, 0x7e),
        hairline: _rgb(0xdf, 0xd5, 0xc9),
        hairlineStrong: _rgb(0x8e, 0x80, 0x71),
        accent: _rgb(0xaa, 0x42, 0x20),
        accentHover: _rgb(0x98, 0x30, 0x0e),
        animated: _rgb(0x97, 0x70, 0x26),
        success: _rgb(0x3f, 0x7f, 0x52),
        warning: _rgb(0xb4, 0x83, 0x2e),
        error: _rgb(0x9e, 0x2e, 0x5e),
        cacheDisk: _rgb(0x00, 0x77, 0x93),
        curve: [
          _rgb(0x00, 0x80, 0x7a),
          _rgb(0x6a, 0x7e, 0x40),
          _rgb(0xa8, 0x59, 0x78),
          _rgb(0x89, 0x7c, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x56, 0x83, 0x89),
          sequence: _rgb(0x61, 0x67, 0x81),
          precomp: _rgb(0x6a, 0x4c, 0x5c),
          solid: _rgb(0x47, 0x41, 0x3d),
          text: _rgb(0xb4, 0xa4, 0x89),
          camera: _rgb(0xaa, 0x8a, 0x7c),
        ),
      );

  /// Chalk dark.
  factory LumitTheme.chalkDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x06, 0x06, 0x06),
        surface1: _rgb(0x0e, 0x0e, 0x0e),
        surface2: _rgb(0x18, 0x18, 0x18),
        surface3: _rgb(0x22, 0x22, 0x22),
        surface4: _rgb(0x2e, 0x2e, 0x2e),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xf8, 0xf8, 0xf8),
        textSecondary: _rgb(0xe2, 0xe2, 0xe2),
        textMuted: _rgb(0xb4, 0xb4, 0xb4),
        textDisabled: _rgb(0x8c, 0x8c, 0x8c),
        hairline: _rgb(0x34, 0x34, 0x34),
        hairlineStrong: _rgb(0x8c, 0x8c, 0x8c),
        accent: _rgb(0x7d, 0xc1, 0xfe),
        accentHover: _rgb(0x8f, 0xd3, 0xff),
        animated: _rgb(0xe9, 0xc2, 0x68),
        success: _rgb(0x1c, 0xbc, 0x81),
        warning: _rgb(0xfb, 0xe3, 0x5f),
        error: _rgb(0xcf, 0x4a, 0x8f),
        cacheDisk: _rgb(0x5a, 0xb7, 0xd4),
        curve: [
          _rgb(0x6f, 0xe3, 0xe6),
          _rgb(0xba, 0xdc, 0xa0),
          _rgb(0xf7, 0xb1, 0xdd),
          _rgb(0xe6, 0xd7, 0x9f),
        ],
        layer: LayerColours(
          footage: _rgb(0x6c, 0x8c, 0x92),
          sequence: _rgb(0x60, 0x6e, 0x81),
          precomp: _rgb(0x56, 0x52, 0x65),
          solid: _rgb(0x3e, 0x3e, 0x3e),
          text: _rgb(0xc3, 0xbb, 0xa5),
          camera: _rgb(0xb4, 0x9c, 0x8f),
        ),
      );

  /// Chalk light.
  factory LumitTheme.chalkLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xe6, 0xe6, 0xe6),
        surface1: _rgb(0xff, 0xff, 0xff),
        surface2: _rgb(0xf2, 0xf2, 0xf2),
        surface3: _rgb(0xff, 0xff, 0xff),
        surface4: _rgb(0xda, 0xda, 0xda),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x0a, 0x0a, 0x0a),
        textSecondary: _rgb(0x26, 0x26, 0x26),
        textMuted: _rgb(0x4a, 0x4a, 0x4a),
        textDisabled: _rgb(0x6a, 0x6a, 0x6a),
        hairline: _rgb(0xc8, 0xc8, 0xc8),
        hairlineStrong: _rgb(0x6e, 0x6e, 0x6e),
        accent: _rgb(0x00, 0x4e, 0xa1),
        accentHover: _rgb(0x00, 0x3c, 0x8f),
        animated: _rgb(0x93, 0x6b, 0x00),
        success: _rgb(0x00, 0x80, 0x55),
        warning: _rgb(0xa3, 0x8a, 0x00),
        error: _rgb(0xa3, 0x0c, 0x68),
        cacheDisk: _rgb(0x00, 0x72, 0x8b),
        curve: [
          _rgb(0x01, 0x7a, 0x7c),
          _rgb(0x52, 0x7d, 0x36),
          _rgb(0xa3, 0x50, 0x87),
          _rgb(0x84, 0x77, 0x30),
        ],
        layer: LayerColours(
          footage: _rgb(0x5c, 0x82, 0x89),
          sequence: _rgb(0x53, 0x65, 0x7a),
          precomp: _rgb(0x4d, 0x48, 0x5e),
          solid: _rgb(0x35, 0x35, 0x35),
          text: _rgb(0xb9, 0xb0, 0x97),
          camera: _rgb(0xac, 0x90, 0x82),
        ),
      );

  /// Vellum dark.
  factory LumitTheme.vellumDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x1e, 0x1d, 0x1b),
        surface1: _rgb(0x28, 0x27, 0x25),
        surface2: _rgb(0x32, 0x30, 0x2d),
        surface3: _rgb(0x3b, 0x39, 0x36),
        surface4: _rgb(0x47, 0x44, 0x3f),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xed, 0xe8, 0xdf),
        textSecondary: _rgb(0xcf, 0xc8, 0xbc),
        textMuted: _rgb(0xa4, 0x9d, 0x90),
        textDisabled: _rgb(0x7b, 0x75, 0x69),
        hairline: _rgb(0x38, 0x36, 0x32),
        hairlineStrong: _rgb(0x82, 0x79, 0x6d),
        accent: _rgb(0x54, 0xaa, 0xd4),
        accentHover: _rgb(0x66, 0xbc, 0xe6),
        animated: _rgb(0xcc, 0xac, 0x77),
        success: _rgb(0x7e, 0xb1, 0x91),
        warning: _rgb(0xe8, 0xc8, 0x8d),
        error: _rgb(0xb6, 0x6a, 0x87),
        cacheDisk: _rgb(0x64, 0xa5, 0xb4),
        curve: [
          _rgb(0x84, 0xc7, 0xc8),
          _rgb(0xb5, 0xc0, 0x98),
          _rgb(0xd5, 0xa3, 0xb8),
          _rgb(0xcc, 0xc1, 0x98),
        ],
        layer: LayerColours(
          footage: _rgb(0x71, 0x8a, 0x92),
          sequence: _rgb(0x6c, 0x72, 0x82),
          precomp: _rgb(0x6b, 0x59, 0x64),
          solid: _rgb(0x4f, 0x4b, 0x45),
          text: _rgb(0xb8, 0xb0, 0x9e),
          camera: _rgb(0xaa, 0x98, 0x8c),
        ),
      );

  /// Vellum light.
  factory LumitTheme.vellumLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xe9, 0xe3, 0xd8),
        surface1: _rgb(0xfa, 0xf6, 0xee),
        surface2: _rgb(0xf2, 0xec, 0xdf),
        surface3: _rgb(0xfc, 0xf9, 0xf3),
        surface4: _rgb(0xe0, 0xd9, 0xcc),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x2c, 0x2a, 0x25),
        textSecondary: _rgb(0x4b, 0x48, 0x3f),
        textMuted: _rgb(0x71, 0x6b, 0x60),
        textDisabled: _rgb(0x8f, 0x88, 0x79),
        hairline: _rgb(0xd9, 0xd1, 0xc3),
        hairlineStrong: _rgb(0x8a, 0x83, 0x75),
        accent: _rgb(0x13, 0x67, 0xa5),
        accentHover: _rgb(0x01, 0x55, 0x93),
        animated: _rgb(0x96, 0x71, 0x2e),
        success: _rgb(0x44, 0x7e, 0x5c),
        warning: _rgb(0xac, 0x85, 0x39),
        error: _rgb(0x8b, 0x37, 0x5b),
        cacheDisk: _rgb(0x28, 0x77, 0x86),
        curve: [
          _rgb(0x15, 0x7e, 0x81),
          _rgb(0x6e, 0x7d, 0x4d),
          _rgb(0x9b, 0x60, 0x7b),
          _rgb(0x87, 0x7c, 0x4c),
        ],
        layer: LayerColours(
          footage: _rgb(0x63, 0x81, 0x8a),
          sequence: _rgb(0x61, 0x68, 0x7b),
          precomp: _rgb(0x64, 0x4f, 0x5b),
          solid: _rgb(0x46, 0x41, 0x3c),
          text: _rgb(0xaf, 0xa5, 0x90),
          camera: _rgb(0xa2, 0x8c, 0x7f),
        ),
      );

  /// Neon dark.
  factory LumitTheme.neonDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x1a, 0x1a, 0x1f),
        surface1: _rgb(0x22, 0x22, 0x28),
        surface2: _rgb(0x2a, 0x2b, 0x30),
        surface3: _rgb(0x33, 0x33, 0x39),
        surface4: _rgb(0x3e, 0x3e, 0x44),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xed, 0xed, 0xf5),
        textSecondary: _rgb(0xce, 0xce, 0xd6),
        textMuted: _rgb(0xa0, 0xa0, 0xa7),
        textDisabled: _rgb(0x71, 0x71, 0x78),
        hairline: _rgb(0x30, 0x31, 0x3a),
        hairlineStrong: _rgb(0x78, 0x79, 0x83),
        accent: _rgb(0xff, 0x62, 0xdf),
        accentHover: _rgb(0xff, 0x74, 0xf1),
        animated: _rgb(0x23, 0xe7, 0xf3),
        success: _rgb(0x52, 0xce, 0x60),
        warning: _rgb(0xed, 0xe9, 0x2d),
        error: _rgb(0xf2, 0x2c, 0x73),
        cacheDisk: _rgb(0x63, 0x8e, 0xf0),
        curve: [
          _rgb(0x09, 0xdd, 0xdb),
          _rgb(0xa1, 0xd5, 0x7d),
          _rgb(0xff, 0x9e, 0xca),
          _rgb(0xeb, 0xd0, 0x7a),
        ],
        layer: LayerColours(
          footage: _rgb(0x4b, 0x96, 0xa0),
          sequence: _rgb(0x48, 0x82, 0x6f),
          precomp: _rgb(0x7c, 0x57, 0x77),
          solid: _rgb(0x51, 0x4f, 0x55),
          text: _rgb(0xbf, 0xb7, 0x8a),
          camera: _rgb(0xc2, 0x98, 0x7c),
        ),
      );

  /// Neon light.
  factory LumitTheme.neonLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xe5, 0xe5, 0xec),
        surface1: _rgb(0xf9, 0xf9, 0xfc),
        surface2: _rgb(0xef, 0xef, 0xf4),
        surface3: _rgb(0xfc, 0xfc, 0xfe),
        surface4: _rgb(0xdf, 0xdf, 0xe7),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x24, 0x25, 0x33),
        textSecondary: _rgb(0x45, 0x46, 0x55),
        textMuted: _rgb(0x6c, 0x6d, 0x7e),
        textDisabled: _rgb(0x88, 0x89, 0x9a),
        hairline: _rgb(0xd6, 0xd6, 0xe2),
        hairlineStrong: _rgb(0x85, 0x85, 0x90),
        accent: _rgb(0xc0, 0x00, 0xa4),
        accentHover: _rgb(0xae, 0x00, 0x92),
        animated: _rgb(0xa5, 0x62, 0x00),
        success: _rgb(0x00, 0x87, 0x29),
        warning: _rgb(0x9f, 0x8f, 0x00),
        error: _rgb(0xab, 0x00, 0x4a),
        cacheDisk: _rgb(0x25, 0x6a, 0xcf),
        curve: [
          _rgb(0x02, 0x85, 0x84),
          _rgb(0x51, 0x8a, 0x2c),
          _rgb(0xc3, 0x3e, 0x85),
          _rgb(0x97, 0x80, 0x11),
        ],
        layer: LayerColours(
          footage: _rgb(0x28, 0x82, 0x8c),
          sequence: _rgb(0x2e, 0x6e, 0x5b),
          precomp: _rgb(0x6a, 0x43, 0x65),
          solid: _rgb(0x3e, 0x3d, 0x43),
          text: _rgb(0xa9, 0xa1, 0x72),
          camera: _rgb(0xae, 0x82, 0x65),
        ),
      );

  /// Canopy dark.
  factory LumitTheme.canopyDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x16, 0x1c, 0x17),
        surface1: _rgb(0x1e, 0x24, 0x1f),
        surface2: _rgb(0x26, 0x2d, 0x27),
        surface3: _rgb(0x2e, 0x35, 0x30),
        surface4: _rgb(0x39, 0x41, 0x3b),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xe8, 0xf0, 0xea),
        textSecondary: _rgb(0xc9, 0xd1, 0xcb),
        textMuted: _rgb(0x9b, 0xa2, 0x9d),
        textDisabled: _rgb(0x6d, 0x74, 0x6e),
        hairline: _rgb(0x2a, 0x34, 0x2c),
        hairlineStrong: _rgb(0x71, 0x7c, 0x73),
        accent: _rgb(0xc2, 0xce, 0x75),
        accentHover: _rgb(0xd4, 0xe0, 0x87),
        animated: _rgb(0xe3, 0xbb, 0x77),
        success: _rgb(0x57, 0xab, 0x86),
        warning: _rgb(0xf8, 0xd3, 0x77),
        error: _rgb(0xca, 0x56, 0x61),
        cacheDisk: _rgb(0x45, 0xa0, 0xc1),
        curve: [
          _rgb(0x6e, 0xd0, 0xd3),
          _rgb(0xad, 0xca, 0x96),
          _rgb(0xec, 0xa8, 0xc6),
          _rgb(0xdb, 0xcc, 0x94),
        ],
        layer: LayerColours(
          footage: _rgb(0x66, 0x8d, 0x94),
          sequence: _rgb(0x64, 0x73, 0x89),
          precomp: _rgb(0x70, 0x57, 0x66),
          solid: _rgb(0x47, 0x4d, 0x48),
          text: _rgb(0xbd, 0xaf, 0x97),
          camera: _rgb(0xb2, 0x95, 0x88),
        ),
      );

  /// Canopy light.
  factory LumitTheme.canopyLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xe1, 0xe7, 0xe0),
        surface1: _rgb(0xf8, 0xfa, 0xf8),
        surface2: _rgb(0xed, 0xf0, 0xec),
        surface3: _rgb(0xfb, 0xfd, 0xfb),
        surface4: _rgb(0xdc, 0xe1, 0xdb),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x20, 0x28, 0x1e),
        textSecondary: _rgb(0x40, 0x49, 0x3f),
        textMuted: _rgb(0x67, 0x71, 0x65),
        textDisabled: _rgb(0x83, 0x8d, 0x81),
        hairline: _rgb(0xd1, 0xd9, 0xd0),
        hairlineStrong: _rgb(0x81, 0x88, 0x7f),
        accent: _rgb(0x55, 0x65, 0x00),
        accentHover: _rgb(0x43, 0x53, 0x00),
        animated: _rgb(0x9a, 0x6f, 0x27),
        success: _rgb(0x1d, 0x84, 0x5e),
        warning: _rgb(0xb0, 0x88, 0x29),
        error: _rgb(0xa3, 0x2e, 0x3f),
        cacheDisk: _rgb(0x00, 0x77, 0x95),
        curve: [
          _rgb(0x00, 0x7f, 0x82),
          _rgb(0x5f, 0x80, 0x47),
          _rgb(0xa5, 0x5a, 0x7d),
          _rgb(0x89, 0x7c, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x56, 0x83, 0x8b),
          sequence: _rgb(0x58, 0x69, 0x82),
          precomp: _rgb(0x69, 0x4c, 0x5e),
          solid: _rgb(0x3e, 0x44, 0x3f),
          text: _rgb(0xb4, 0xa4, 0x89),
          camera: _rgb(0xaa, 0x8a, 0x7c),
        ),
      );

  /// Slate dark.
  factory LumitTheme.slateDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x1a, 0x1a, 0x1a),
        surface1: _rgb(0x23, 0x23, 0x23),
        surface2: _rgb(0x2b, 0x2b, 0x2b),
        surface3: _rgb(0x34, 0x34, 0x34),
        surface4: _rgb(0x3f, 0x3f, 0x3f),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xee, 0xee, 0xee),
        textSecondary: _rgb(0xcf, 0xcf, 0xcf),
        textMuted: _rgb(0xa0, 0xa0, 0xa0),
        textDisabled: _rgb(0x72, 0x72, 0x72),
        hairline: _rgb(0x31, 0x31, 0x31),
        hairlineStrong: _rgb(0x79, 0x79, 0x79),
        accent: _rgb(0x33, 0xbe, 0xef),
        accentHover: _rgb(0x45, 0xd0, 0xff),
        animated: _rgb(0xe3, 0xbc, 0x6f),
        success: _rgb(0x54, 0xb8, 0x82),
        warning: _rgb(0xf5, 0xd4, 0x77),
        error: _rgb(0xd2, 0x55, 0x86),
        cacheDisk: _rgb(0x66, 0x9b, 0xca),
        curve: [
          _rgb(0x62, 0xd8, 0xdb),
          _rgb(0xaf, 0xd1, 0x95),
          _rgb(0xec, 0xa7, 0xd2),
          _rgb(0xdb, 0xcc, 0x94),
        ],
        layer: LayerColours(
          footage: _rgb(0x6c, 0x8c, 0x93),
          sequence: _rgb(0x67, 0x73, 0x86),
          precomp: _rgb(0x64, 0x5b, 0x6d),
          solid: _rgb(0x4b, 0x4b, 0x4b),
          text: _rgb(0xb8, 0xb0, 0x9b),
          camera: _rgb(0xad, 0x97, 0x89),
        ),
      );

  /// Slate light.
  factory LumitTheme.slateLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xe5, 0xe5, 0xe5),
        surface1: _rgb(0xf9, 0xf9, 0xf9),
        surface2: _rgb(0xef, 0xef, 0xef),
        surface3: _rgb(0xfc, 0xfc, 0xfc),
        surface4: _rgb(0xdf, 0xdf, 0xdf),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x26, 0x26, 0x26),
        textSecondary: _rgb(0x47, 0x47, 0x47),
        textMuted: _rgb(0x6e, 0x6e, 0x6e),
        textDisabled: _rgb(0x8a, 0x8a, 0x8a),
        hairline: _rgb(0xd7, 0xd7, 0xd7),
        hairlineStrong: _rgb(0x86, 0x86, 0x86),
        accent: _rgb(0x00, 0x6d, 0xa9),
        accentHover: _rgb(0x00, 0x5b, 0x97),
        animated: _rgb(0x9b, 0x6f, 0x22),
        success: _rgb(0x0f, 0x83, 0x51),
        warning: _rgb(0xad, 0x89, 0x28),
        error: _rgb(0xa8, 0x1e, 0x5f),
        cacheDisk: _rgb(0x39, 0x71, 0x9e),
        curve: [
          _rgb(0x00, 0x7f, 0x82),
          _rgb(0x5b, 0x81, 0x41),
          _rgb(0xa4, 0x59, 0x8a),
          _rgb(0x89, 0x7c, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x5d, 0x82, 0x8b),
          sequence: _rgb(0x5b, 0x69, 0x7f),
          precomp: _rgb(0x5b, 0x51, 0x66),
          solid: _rgb(0x42, 0x42, 0x42),
          text: _rgb(0xae, 0xa5, 0x8d),
          camera: _rgb(0xa5, 0x8c, 0x7c),
        ),
      );

  /// Nocturne dark.
  factory LumitTheme.nocturneDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x19, 0x1a, 0x24),
        surface1: _rgb(0x22, 0x22, 0x2d),
        surface2: _rgb(0x2a, 0x2a, 0x36),
        surface3: _rgb(0x33, 0x33, 0x3e),
        surface4: _rgb(0x3e, 0x3e, 0x4a),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xed, 0xed, 0xf7),
        textSecondary: _rgb(0xce, 0xce, 0xd7),
        textMuted: _rgb(0xa0, 0xa0, 0xa9),
        textDisabled: _rgb(0x71, 0x71, 0x7a),
        hairline: _rgb(0x30, 0x30, 0x3f),
        hairlineStrong: _rgb(0x78, 0x78, 0x89),
        accent: _rgb(0xff, 0x95, 0x60),
        accentHover: _rgb(0xff, 0xa7, 0x72),
        animated: _rgb(0xd8, 0xcd, 0x89),
        success: _rgb(0x5b, 0xb8, 0x83),
        warning: _rgb(0xf6, 0xd7, 0x82),
        error: _rgb(0xd3, 0x53, 0x8b),
        cacheDisk: _rgb(0x4f, 0x9e, 0xca),
        curve: [
          _rgb(0x6e, 0xd0, 0xd3),
          _rgb(0xad, 0xca, 0x96),
          _rgb(0xe6, 0xa9, 0xcf),
          _rgb(0xdb, 0xcc, 0x94),
        ],
        layer: LayerColours(
          footage: _rgb(0x67, 0x8c, 0x95),
          sequence: _rgb(0x60, 0x74, 0x89),
          precomp: _rgb(0x6b, 0x58, 0x6b),
          solid: _rgb(0x4a, 0x4b, 0x51),
          text: _rgb(0xb9, 0xb0, 0x97),
          camera: _rgb(0xb4, 0x94, 0x8c),
        ),
      );

  /// Nocturne light.
  factory LumitTheme.nocturneLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xe4, 0xe5, 0xee),
        surface1: _rgb(0xf9, 0xf9, 0xfc),
        surface2: _rgb(0xee, 0xef, 0xf5),
        surface3: _rgb(0xfc, 0xfc, 0xfe),
        surface4: _rgb(0xde, 0xdf, 0xe9),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x22, 0x25, 0x36),
        textSecondary: _rgb(0x43, 0x46, 0x58),
        textMuted: _rgb(0x6b, 0x6d, 0x81),
        textDisabled: _rgb(0x87, 0x89, 0x9e),
        hairline: _rgb(0xd5, 0xd6, 0xe5),
        hairlineStrong: _rgb(0x84, 0x85, 0x93),
        accent: _rgb(0xb5, 0x4d, 0x1b),
        accentHover: _rgb(0xa3, 0x3b, 0x09),
        animated: _rgb(0x92, 0x72, 0x29),
        success: _rgb(0x21, 0x82, 0x4c),
        warning: _rgb(0xaf, 0x88, 0x2e),
        error: _rgb(0xa7, 0x1f, 0x63),
        cacheDisk: _rgb(0x11, 0x75, 0x9f),
        curve: [
          _rgb(0x00, 0x7f, 0x82),
          _rgb(0x5f, 0x80, 0x47),
          _rgb(0x9f, 0x5c, 0x87),
          _rgb(0x89, 0x7c, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x57, 0x83, 0x8d),
          sequence: _rgb(0x52, 0x6b, 0x82),
          precomp: _rgb(0x63, 0x4e, 0x64),
          solid: _rgb(0x41, 0x42, 0x48),
          text: _rgb(0xaf, 0xa5, 0x89),
          camera: _rgb(0xad, 0x89, 0x7f),
        ),
      );

  /// Tavern dark.
  factory LumitTheme.tavernDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x26, 0x17, 0x10),
        surface1: _rgb(0x2f, 0x1f, 0x19),
        surface2: _rgb(0x38, 0x27, 0x21),
        surface3: _rgb(0x41, 0x30, 0x29),
        surface4: _rgb(0x4d, 0x3b, 0x34),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xff, 0xe9, 0xe1),
        textSecondary: _rgb(0xe1, 0xca, 0xc1),
        textMuted: _rgb(0xb2, 0x9c, 0x93),
        textDisabled: _rgb(0x82, 0x6e, 0x66),
        hairline: _rgb(0x43, 0x2c, 0x23),
        hairlineStrong: _rgb(0x90, 0x73, 0x68),
        accent: _rgb(0xf7, 0x92, 0x45),
        accentHover: _rgb(0xff, 0xa4, 0x57),
        animated: _rgb(0xe9, 0xd0, 0x82),
        success: _rgb(0x66, 0xb0, 0x7b),
        warning: _rgb(0xf2, 0xd9, 0x7a),
        error: _rgb(0xc2, 0x4d, 0x80),
        cacheDisk: _rgb(0x45, 0xa0, 0xc1),
        curve: [
          _rgb(0x70, 0xd0, 0xce),
          _rgb(0xb2, 0xc9, 0x93),
          _rgb(0xe9, 0xa2, 0xbc),
          _rgb(0xd5, 0xc6, 0x8f),
        ],
        layer: LayerColours(
          footage: _rgb(0x66, 0x8d, 0x94),
          sequence: _rgb(0x6f, 0x70, 0x88),
          precomp: _rgb(0x73, 0x56, 0x63),
          solid: _rgb(0x51, 0x4a, 0x47),
          text: _rgb(0xb9, 0xb0, 0x97),
          camera: _rgb(0xb5, 0x93, 0x8f),
        ),
      );

  /// Tavern light.
  factory LumitTheme.tavernLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xf4, 0xe3, 0xcf),
        surface1: _rgb(0xfe, 0xf8, 0xf2),
        surface2: _rgb(0xf9, 0xee, 0xe0),
        surface3: _rgb(0xff, 0xfc, 0xf8),
        surface4: _rgb(0xee, 0xdd, 0xc9),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x30, 0x24, 0x12),
        textSecondary: _rgb(0x53, 0x44, 0x32),
        textMuted: _rgb(0x7c, 0x6c, 0x57),
        textDisabled: _rgb(0x99, 0x88, 0x72),
        hairline: _rgb(0xe5, 0xd5, 0xc1),
        hairlineStrong: _rgb(0x93, 0x84, 0x72),
        accent: _rgb(0xad, 0x54, 0x0b),
        accentHover: _rgb(0x9b, 0x42, 0x00),
        animated: _rgb(0x95, 0x71, 0x25),
        success: _rgb(0x32, 0x81, 0x4d),
        warning: _rgb(0xb0, 0x88, 0x24),
        error: _rgb(0x9f, 0x1d, 0x5e),
        cacheDisk: _rgb(0x00, 0x77, 0x95),
        curve: [
          _rgb(0x00, 0x80, 0x7e),
          _rgb(0x64, 0x7f, 0x43),
          _rgb(0xa8, 0x59, 0x78),
          _rgb(0x89, 0x7c, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x56, 0x83, 0x8b),
          sequence: _rgb(0x64, 0x66, 0x81),
          precomp: _rgb(0x6b, 0x4c, 0x5a),
          solid: _rgb(0x48, 0x41, 0x3d),
          text: _rgb(0xaf, 0xa5, 0x89),
          camera: _rgb(0xae, 0x88, 0x83),
        ),
      );

  /// Arcane dark.
  factory LumitTheme.arcaneDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x11, 0x1b, 0x28),
        surface1: _rgb(0x19, 0x23, 0x30),
        surface2: _rgb(0x21, 0x2c, 0x39),
        surface3: _rgb(0x2a, 0x34, 0x42),
        surface4: _rgb(0x35, 0x3f, 0x4e),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xea, 0xee, 0xf5),
        textSecondary: _rgb(0xcb, 0xcf, 0xd6),
        textMuted: _rgb(0x9d, 0xa1, 0xa7),
        textDisabled: _rgb(0x6e, 0x72, 0x79),
        hairline: _rgb(0x25, 0x32, 0x43),
        hairlineStrong: _rgb(0x6d, 0x7b, 0x8e),
        accent: _rgb(0xaf, 0xa6, 0xff),
        accentHover: _rgb(0xc1, 0xb8, 0xff),
        animated: _rgb(0x7d, 0xe1, 0xed),
        success: _rgb(0x56, 0xb8, 0x8b),
        warning: _rgb(0xf3, 0xd8, 0x86),
        error: _rgb(0xd0, 0x55, 0x93),
        cacheDisk: _rgb(0x50, 0x9f, 0xc5),
        curve: [
          _rgb(0x76, 0xd6, 0xd4),
          _rgb(0xb2, 0xcf, 0x9b),
          _rgb(0xea, 0xa8, 0xca),
          _rgb(0xdb, 0xcc, 0x94),
        ],
        layer: LayerColours(
          footage: _rgb(0x66, 0x8d, 0x8e),
          sequence: _rgb(0x59, 0x76, 0x87),
          precomp: _rgb(0x69, 0x59, 0x6d),
          solid: _rgb(0x48, 0x4c, 0x51),
          text: _rgb(0xb9, 0xb0, 0x97),
          camera: _rgb(0xb0, 0x96, 0x86),
        ),
      );

  /// Arcane light.
  factory LumitTheme.arcaneLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xdf, 0xe6, 0xec),
        surface1: _rgb(0xf7, 0xfa, 0xfc),
        surface2: _rgb(0xeb, 0xf0, 0xf4),
        surface3: _rgb(0xfb, 0xfc, 0xfe),
        surface4: _rgb(0xd9, 0xe1, 0xe7),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x0b, 0x29, 0x38),
        textSecondary: _rgb(0x2f, 0x4a, 0x5b),
        textMuted: _rgb(0x57, 0x72, 0x84),
        textDisabled: _rgb(0x73, 0x8e, 0xa1),
        hairline: _rgb(0xca, 0xd9, 0xe5),
        hairlineStrong: _rgb(0x7a, 0x88, 0x93),
        accent: _rgb(0x60, 0x59, 0xc8),
        accentHover: _rgb(0x4e, 0x47, 0xb6),
        animated: _rgb(0x98, 0x77, 0x36),
        success: _rgb(0x11, 0x82, 0x55),
        warning: _rgb(0xa9, 0x8b, 0x31),
        error: _rgb(0x9e, 0x1b, 0x66),
        cacheDisk: _rgb(0x16, 0x75, 0x9a),
        curve: [
          _rgb(0x00, 0x80, 0x7e),
          _rgb(0x5f, 0x80, 0x47),
          _rgb(0xa3, 0x5b, 0x82),
          _rgb(0x89, 0x7c, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x57, 0x84, 0x85),
          sequence: _rgb(0x4b, 0x6c, 0x80),
          precomp: _rgb(0x61, 0x4f, 0x65),
          solid: _rgb(0x3f, 0x42, 0x48),
          text: _rgb(0xaf, 0xa5, 0x89),
          camera: _rgb(0xa8, 0x8b, 0x79),
        ),
      );

  /// Gilt dark.
  factory LumitTheme.giltDark() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x20, 0x18, 0x1f),
        surface1: _rgb(0x28, 0x20, 0x27),
        surface2: _rgb(0x31, 0x29, 0x2f),
        surface3: _rgb(0x39, 0x31, 0x38),
        surface4: _rgb(0x45, 0x3c, 0x43),
        viewerSurround: _rgb(0x1c, 0x1c, 0x1c),
        textPrimary: _rgb(0xf6, 0xeb, 0xf4),
        textSecondary: _rgb(0xd6, 0xcc, 0xd4),
        textMuted: _rgb(0xa8, 0x9e, 0xa6),
        textDisabled: _rgb(0x79, 0x6f, 0x77),
        hairline: _rgb(0x39, 0x2e, 0x37),
        hairlineStrong: _rgb(0x83, 0x76, 0x80),
        accent: _rgb(0xf0, 0xa8, 0x40),
        accentHover: _rgb(0xff, 0xba, 0x52),
        animated: _rgb(0xe2, 0xca, 0x84),
        success: _rgb(0x61, 0xab, 0x76),
        warning: _rgb(0xef, 0xde, 0x7d),
        error: _rgb(0xc1, 0x44, 0x74),
        cacheDisk: _rgb(0x4a, 0xa0, 0xc3),
        curve: [
          _rgb(0x70, 0xd0, 0xce),
          _rgb(0xb2, 0xc9, 0x93),
          _rgb(0xe6, 0xa2, 0xc0),
          _rgb(0xd5, 0xc6, 0x8f),
        ],
        layer: LayerColours(
          footage: _rgb(0x69, 0x8c, 0x98),
          sequence: _rgb(0x6f, 0x70, 0x88),
          precomp: _rgb(0x72, 0x57, 0x64),
          solid: _rgb(0x4f, 0x4a, 0x4e),
          text: _rgb(0xb6, 0xb1, 0x97),
          camera: _rgb(0xb3, 0x94, 0x8a),
        ),
      );

  /// Gilt light.
  factory LumitTheme.giltLight() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xed, 0xe4, 0xd6),
        surface1: _rgb(0xfc, 0xf9, 0xf4),
        surface2: _rgb(0xf5, 0xee, 0xe5),
        surface3: _rgb(0xfe, 0xfc, 0xf8),
        surface4: _rgb(0xe8, 0xdf, 0xd0),
        viewerSurround: _rgb(0xa8, 0xa8, 0xa8),
        textPrimary: _rgb(0x2c, 0x25, 0x18),
        textSecondary: _rgb(0x4e, 0x46, 0x37),
        textMuted: _rgb(0x76, 0x6d, 0x5e),
        textDisabled: _rgb(0x93, 0x89, 0x79),
        hairline: _rgb(0xe1, 0xd6, 0xc4),
        hairlineStrong: _rgb(0x8f, 0x85, 0x75),
        accent: _rgb(0xa1, 0x60, 0x00),
        accentHover: _rgb(0x8f, 0x4e, 0x00),
        animated: _rgb(0x93, 0x79, 0x2d),
        success: _rgb(0x32, 0x81, 0x4d),
        warning: _rgb(0xa8, 0x8c, 0x22),
        error: _rgb(0xa3, 0x15, 0x57),
        cacheDisk: _rgb(0x05, 0x76, 0x98),
        curve: [
          _rgb(0x00, 0x80, 0x7e),
          _rgb(0x64, 0x7f, 0x43),
          _rgb(0xa5, 0x5a, 0x7d),
          _rgb(0x89, 0x7c, 0x3d),
        ],
        layer: LayerColours(
          footage: _rgb(0x59, 0x82, 0x90),
          sequence: _rgb(0x64, 0x66, 0x81),
          precomp: _rgb(0x6a, 0x4c, 0x5c),
          solid: _rgb(0x46, 0x40, 0x45),
          text: _rgb(0xac, 0xa6, 0x89),
          camera: _rgb(0xac, 0x89, 0x7d),
        ),
      );

  /// Grey room, the lighter of Desk's two rooms
  /// (docs/design-alt/15-DESIGN-DESK.md 2 and 3). One signal colour does the
  /// accent's job and the animated one: the document dissolves amber into it.
  /// The curve hues are the layer family read at the same lightness.
  factory LumitTheme.greyRoom() => LumitTheme(
        mode: ThemeMode2.light,
        surface0: _rgb(0xde, 0xdc, 0xd8),
        surface1: _rgb(0xec, 0xeb, 0xe7),
        surface2: _rgb(0xe4, 0xe2, 0xde),
        surface3: _rgb(0xf7, 0xf6, 0xf3),
        surface4: _rgb(0xd2, 0xd0, 0xcb),
        viewerSurround: _rgb(0x7f, 0x7f, 0x7f),
        textPrimary: _rgb(0x1b, 0x1b, 0x1a),
        textSecondary: _rgb(0x4c, 0x4c, 0x49),
        textMuted: _rgb(0x69, 0x68, 0x64),
        textDisabled: _rgb(0x84, 0x83, 0x80),
        hairline: _rgb(0xcd, 0xcb, 0xc6),
        hairlineStrong: _rgb(0xa9, 0xa7, 0xa1),
        accent: _rgb(0xb8, 0x50, 0x0d),
        accentHover: _shift(_rgb(0xb8, 0x50, 0x0d), 0x12),
        animated: _rgb(0xb8, 0x50, 0x0d),
        success: _rgb(0x2f, 0x6f, 0x4a),
        warning: _rgb(0x8a, 0x5f, 0x04),
        error: _rgb(0xa6, 0x33, 0x1e),
        cacheDisk: _rgb(0x4a, 0x6b, 0x7d),
        curve: [
          _rgb(0x6f, 0x7f, 0x88),
          _rgb(0x8a, 0x72, 0x84),
          _rgb(0x8b, 0x85, 0x70),
          _rgb(0x8a, 0x7c, 0x5c),
        ],
        layer: LayerColours(
          footage: _rgb(0x6f, 0x7f, 0x88),
          sequence: _rgb(0x6c, 0x7a, 0x94),
          precomp: _rgb(0x8a, 0x72, 0x84),
          solid: _rgb(0x7c, 0x7c, 0x78),
          text: _rgb(0x8b, 0x85, 0x70),
          camera: _rgb(0x8a, 0x7c, 0x5c),
        ),
      );

  /// Graphite, the darker of Desk's two rooms. The same ramp as the grey room
  /// at the other lightness, and the same one signal.
  factory LumitTheme.graphite() => LumitTheme(
        mode: ThemeMode2.dark,
        surface0: _rgb(0x1c, 0x1c, 0x1b),
        surface1: _rgb(0x26, 0x26, 0x25),
        surface2: _rgb(0x2e, 0x2e, 0x2c),
        surface3: _rgb(0x38, 0x38, 0x36),
        surface4: _rgb(0x45, 0x45, 0x42),
        viewerSurround: _rgb(0x7f, 0x7f, 0x7f),
        textPrimary: _rgb(0xf0, 0xef, 0xeb),
        textSecondary: _rgb(0xc4, 0xc2, 0xbc),
        textMuted: _rgb(0x91, 0x8f, 0x89),
        textDisabled: _rgb(0x78, 0x75, 0x6f),
        hairline: _rgb(0x35, 0x35, 0x32),
        hairlineStrong: _rgb(0x4e, 0x4e, 0x4a),
        accent: _rgb(0xe8, 0x71, 0x2a),
        accentHover: _shift(_rgb(0xe8, 0x71, 0x2a), 0x12),
        animated: _rgb(0xe8, 0x71, 0x2a),
        success: _rgb(0x5c, 0xb0, 0x83),
        warning: _rgb(0xd2, 0xa2, 0x3f),
        error: _rgb(0xe5, 0x83, 0x67),
        cacheDisk: _rgb(0x5f, 0x81, 0x96),
        curve: [
          _rgb(0x77, 0x87, 0x8f),
          _rgb(0x91, 0x7a, 0x8c),
          _rgb(0x93, 0x8d, 0x78),
          _rgb(0x92, 0x84, 0x63),
        ],
        layer: LayerColours(
          footage: _rgb(0x77, 0x87, 0x8f),
          sequence: _rgb(0x74, 0x83, 0x9d),
          precomp: _rgb(0x91, 0x7a, 0x8c),
          solid: _rgb(0x84, 0x84, 0x7f),
          text: _rgb(0x93, 0x8d, 0x78),
          camera: _rgb(0x92, 0x84, 0x63),
        ),
      );

  // --- Type scale (docs/15-DESIGN §density: 11 px body, 10 px small) -------

  static const String fontFamily = 'Hanken Grotesk';

  /// Hanken Grotesk has no Cyrillic; Inter stays bundled as the fallback so
  /// the Ukrainian and Kazakh locales keep a designed face instead of the
  /// platform default.
  static const List<String> fontFamilyFallback = ['Inter'];

  /// The face every number, timecode and container label is set in
  /// (docs/15-DESIGN.md §7.1). The fallbacks only matter if the bundled asset
  /// is missing, and both platform names resolve to a monospaced face.
  static const String monoFontFamily = 'Geist Mono';
  static const List<String> monoFontFamilyFallback = [
    'Consolas',
    'monospace',
  ];

  /// Tabular figures where the shape sets numbers in its sans.
  List<FontFeature>? get _figures =>
      tokens.tabularNumbers ? const [FontFeature.tabularFigures()] : null;
  TextStyle get heading => TextStyle(
      fontFamily: tokens.sansFamily,
      fontFamilyFallback: fontFamilyFallback,
      fontFeatures: _figures,
      fontSize: 16,
      color: textPrimary,
      decoration: TextDecoration.none,
      fontWeight: FontWeight.w500);
  TextStyle get body => TextStyle(
      fontFamily: tokens.sansFamily,
      fontFamilyFallback: fontFamilyFallback,
      fontFeatures: _figures,
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
      fontFamily: tokens.sansFamily,
      fontFamilyFallback: fontFamilyFallback,
      fontFeatures: _figures,
      fontSize: 10,
      color: textMuted,
      decoration: TextDecoration.none,
      fontWeight: FontWeight.w400);

  /// The note under a field explaining its format — smaller than a label so it
  /// reads as an aside rather than as another thing to fill in
  /// (docs/15-DESIGN.md §7.1).
  TextStyle get caption => small.copyWith(fontSize: 9);

  /// **Every container label** (docs/15-DESIGN.md §7.1): panel titles, section
  /// headers, column headers, tab labels, dialog titles, attribution.
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
        // Desk's label is its one sans, lowercase; the caps kicker is mono.
        fontFamily:
            tokens.tabularNumbers ? tokens.sansFamily : tokens.monoFamily,
        fontFamilyFallback: monoFontFamilyFallback,
        fontFeatures: _figures,
        // 9px at +0.12em, regular weight — **the approved mockups' own
        // `.kick`** (their metrics are canonical), and the bottom of §7.1's
        // 9–11px / 0.08–0.12em band rather than its middle. It was 10px at
        // +0.10em in Medium, which read a size heavier than every kicker the
        // mockups draw. Flutter measures tracking in logical pixels, so an em
        // is the font size.
        fontSize: 9,
        // 1.08 written out, not `9 * 0.12`: the product is 0.12000000000000001
        // in binary floating point, which lands a hair outside the band the
        // spec states and the primitives test checks.
        letterSpacing: tokens.kickerTracking,
        color: textMuted,
        decoration: TextDecoration.none,
        fontWeight: FontWeight.w400,
      );

  /// The kicker of the container that is *in force* — the fronted dock tab, the
  /// section being edited. Only the colour changes: same face, same size, same
  /// tracking, same weight, so nothing shifts by a pixel when the active one
  /// moves (§7.1 — state reads from colour, never from size or weight).
  TextStyle get kickerOn => kicker.copyWith(color: textPrimary);

  /// A container label cased the way the shape wants it: Studio and Lantern
  /// shout, Desk whispers.
  String kickerCase(String s) => switch (tokens.labelCase) {
        LabelCase.caps => s.toUpperCase(),
        LabelCase.lower => s.toLowerCase(),
        LabelCase.sentence => s,
      };
  TextStyle get mono => TextStyle(
      fontFamily: tokens.monoFamily,
      fontFamilyFallback: monoFontFamilyFallback,
      fontFeatures: _figures,
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
    defaultAccent, // spruce - THE default
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
