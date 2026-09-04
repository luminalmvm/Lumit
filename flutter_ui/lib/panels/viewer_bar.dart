// The Viewer's **bottom bar**: the ways of looking, the snapshot pair,
// the transport with its clock, and the composition's own reading — plus the
// shedding ladder that decides which of them a narrow Viewer keeps.
//
// Split out of viewer_panel_frb.dart. The sizes and the mark it is
// drawn with are viewer_strips.dart's, shared with the header strip.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/timecode.dart';
import '../state/workspace.dart' show ViewerLook;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_overlays.dart';
import 'viewer_progress_bar.dart';
import 'viewer_stage.dart';
import 'viewer_strips.dart';

/// The Viewer's **bottom bar** (§12A.6: 22 tall).
///
/// Left to right, and this is the drawing's own order: the ways of *looking* —
/// the transparency board, the view menu, the channel, the exposure — then a
/// hairline seam and the snapshot; the transport with its clock in the middle;
/// and at the right-hand end the composition's own reading, which says what is
/// being shown, at what time, at how many pixels, and how big.
class ViewerBar extends StatelessWidget {
  final ViewerChannel channel;
  final bool grid;
  final bool wireframes;

  /// How the fronted comp is being looked at. Passed down rather than
  /// read here: this bar rebuilds for every frame that arrives, and a control
  /// that asked the engine what it is set to would cross the boundary sixty
  /// times a second to be told what the frontend already knows.
  final ViewerLook look;
  final bool playing;
  final int frame;
  final BridgeCompSettings settings;
  final CompositionReference comp;

  /// The comp's own pixel size, off the panel's held facts.
  final BridgeCompSize compSize;

  /// The preview tier the last frame was made at, off the frame itself. Given
  /// rather than asked for, for the same reason as everything else here.
  final int tier;

  /// The magnification actually on screen, as a multiple of comp resolution.
  final double shownScale;

  /// The comp's background colour, off the held read model. Null before the
  /// model's first read, which the swatch draws as black.
  final F32Array4? background;

  final ValueChanged<ViewerChannel> onChannel;
  final VoidCallback onGrid;
  final VoidCallback onWireframes;
  final ValueChanged<double> onStops;
  final VoidCallback onPlayPause;
  final ValueChanged<int> onSeek;

  /// Whether a snapshot has been taken — what makes a hold do anything.
  final bool hasSnapshot;
  final VoidCallback onSnapshotTake;
  final ValueChanged<bool> onSnapshotHold;

  /// Drawn as a tile of its own under Round, rather than a strip welded to the
  /// panel's bottom edge under Sharp.
  final bool detached;

  /// What leads the strip when the setting has gathered both bars into one:
  /// the panel's kicker and the three pickers the header would otherwise
  /// carry, in that same order. Empty in the drawing's own split, where the
  /// header carries them.
  final List<Widget> leading;

  const ViewerBar({
    super.key,
    required this.channel,
    required this.grid,
    required this.wireframes,
    required this.look,
    required this.playing,
    required this.frame,
    required this.settings,
    required this.comp,
    required this.compSize,
    required this.tier,
    required this.shownScale,
    required this.background,
    required this.onChannel,
    required this.onGrid,
    required this.onWireframes,
    required this.onStops,
    required this.onPlayPause,
    required this.onSeek,
    required this.hasSnapshot,
    required this.onSnapshotTake,
    required this.onSnapshotHold,
    required this.detached,
    this.leading = const [],
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey('viewer-bar'),
      height: viewerStripHeight,
      decoration: viewerStripDecoration(t, detached),
      // The drawing's 10 either end, measured to the first *glyph* and to the
      // last word: the left one allows for the mark's own transparent edge,
      // the right one has nothing to allow for because a reading is text.
      padding: const EdgeInsets.only(
        left: viewerStripPadding - viewerMarkEdge,
        right: viewerStripPadding,
      ),
      // A Viewer docked narrow has less width than this bar wants, and an
      // overflow stripe is not a design: below the width the drawing needs,
      // the same row is laid out with plain gaps and scrolls sideways
      // (§12A.6's ladder, step 5).
      child: LayoutBuilder(
        builder: (context, constraints) {
          final width = constraints.maxWidth;
          final loose = width >= _barMinimum;
          // The rungs, in the order the owner ruled them (see [_barMinimum]).
          final keepsReading = width >= _barKeepsReading;
          final keepsLooking = width >= _barKeepsLooking;
          final keepsClock = width >= _barKeepsClock;
          final reading = _Readout(
            comp: comp,
            settings: settings,
            compSize: compSize,
            frame: frame,
            tier: tier,
            shownScale: shownScale,
          );
          final row = Row(
            // **The two gaps are the same gap, and they are what gives way
            // first** (§12A.6). The drawing sets the transport and the
            // reading each with a `margin-left: auto`, which in a flex row
            // splits whatever is left over equally between them — so the
            // reading is at its own natural width and the gaps take the rest.
            // `spaceBetween` over three groups is exactly that, and it is why
            // the reading is a plain child of the last group rather than one
            // of three equal flex shares: sharing the free space three ways
            // gave the reading a third of it and elided a line that fitted.
            mainAxisAlignment: loose
                ? MainAxisAlignment.spaceBetween
                : MainAxisAlignment.start,
            children: [
              Row(mainAxisSize: MainAxisSize.min, children: [
                ...leading,
                if (leading.isNotEmpty) viewerBarGapBox(viewerBarGap),
                if (keepsLooking)
                  ..._looking(context, t)
                else
                  // **Step 4 of the ladder**: a run of buttons that no longer
                  // fits collapses into one overflow mark at the end of its
                  // run rather than shrinking or clipping. The very same
                  // widgets stand inside it, so nothing here has a second
                  // implementation that can drift from the first.
                  _LookingOverflow(marks: () => _looking(context, t)),
              ]),
              if (!loose) const SizedBox(width: 24),
              Row(
                  mainAxisSize: MainAxisSize.min,
                  children: _transport(t, clock: keepsClock)),
              if (!loose && keepsReading) const SizedBox(width: 24),
              // The reading takes the room the two gaps are not using, and
              // sheds parts of itself before it elides — the ladder is in
              // [_Readout]. Flexible only where the bar is spread; where it
              // scrolls there is no width to be flexible against.
              if (loose)
                Flexible(
                  child: Row(mainAxisSize: MainAxisSize.min, children: [
                    Flexible(child: reading),
                    // Nothing at all while no frame is being waited on, so at
                    // rest the reading really is the bar's right-hand end.
                    ViewerProgressBar(
                      tracker: Provider.of<LumitUiState>(context, listen: false)
                          .previewProgress,
                    ),
                  ]),
                )
              else if (keepsReading) ...[
                reading,
                ViewerProgressBar(
                  tracker: Provider.of<LumitUiState>(context, listen: false)
                      .previewProgress,
                ),
              ],
            ],
          );
          return loose
              ? row
              : SingleChildScrollView(
                  scrollDirection: Axis.horizontal, child: row);
        },
      ),
    );
  }

  /// The ways of looking, and the snapshot behind its seam.
  List<Widget> _looking(BuildContext context, LumitTheme t) => [
        // The transparency board: the checkerboard itself rather than the word
        // "grid", which is also the overlay this is not.
        viewerBarMark(
          key: const ValueKey('viewer-grid'),
          icon: LumitIcon.checkerboard,
          colour: grid ? t.accent : t.textMuted,
          onPressed: onGrid,
          tip: l10n.tipTransparencyGrid,
        ),
        viewerBarGapBox(viewerBarGap),
        // Everything drawn *over* the picture, under one mark (docs/07 §2.2
        // items 5–6): the grid, the safe areas, the layer controls and the
        // region of interest — and the composition's own background, which is
        // the same question asked from behind.
        ViewerGuidesMenu(
          wireframes: wireframes,
          onWireframes: onWireframes,
          comp: comp,
          background: background,
        ),
        viewerBarGapBox(viewerBarGap),
        // The channel as a mark tinted by its own answer: the face is read at
        // a glance during a key, where "Green" spelled out is a word to read
        // and a green mark is a thing to see. The menu still lists the names.
        _ChannelPicker(channel: channel, onChannel: onChannel),
        viewerBarGapBox(viewerBarGap),
        // **The aperture names the number, and is the way back to nothing**
        // (owner ruling, superseding the appears-with-the-value reading):
        // the mark stands always, left of the stops, so the bare number has
        // its identity — and clicking it resets to 0. It brightens while a
        // value is engaged, so at rest it reads as a label rather than as an
        // armed control.
        viewerBarMark(
          key: const ValueKey('viewer-exposure-reset'),
          icon: LumitIcon.aperture,
          colour: look.stops != 0 ? t.textPrimary : t.textMuted,
          onPressed: () => onStops(0),
          tip: l10n.tipViewerExposureReset,
        ),
        // One edge to allow for rather than two: the exposure is text.
        SizedBox(width: viewerBarGap - viewerMarkEdge),
        // The exposure (docs/07 §2.2 item 12), bare: the number with
        // no well under it. Preview only, and the header's colour picker is
        // what says so while it is engaged.
        LumitTooltip(
          message: l10n.tipViewerExposure,
          child: DragValueField(
            key: const ValueKey('viewer-exposure'),
            value: look.stops,
            bare: true,
            // Ten stops each way: past that a picture is white or black
            // whatever is in it, so the drag has somewhere to stop.
            min: -10,
            max: 10,
            speed: 0.1,
            decimals: 1,
            signed: true,
            resetTo: 0,
            // Snapped to the tenth the box actually reads, so a drag cannot
            // leave a hair of exposure behind that shows as `+0.0` while the
            // engine treats the view as engaged.
            onChanged: (v) => onStops((v * 10).round() / 10),
          ),
        ),
        // One edge each side of the seam rather than two: the hairline is a
        // plain rule and carries none of its own.
        SizedBox(width: viewerBarGap - viewerMarkEdge),
        Container(
          width: 1,
          height: viewerBarDividerHeight,
          color: t.hairline,
        ),
        SizedBox(width: viewerBarGap - viewerMarkEdge),
        // Snapshots (§2.2 item 14): **two marks**, because a
        // snapshot nobody can see they have taken is a snapshot nobody uses.
        // Take photographs the picture on a plain click; Show, beside it, puts
        // the photograph back over the live one while it is held — and is
        // muted, saying why, until there is one to show.
        viewerBarMark(
          key: const ValueKey('viewer-snapshot'),
          icon: LumitIcon.snapshot,
          colour: t.textMuted,
          onPressed: onSnapshotTake,
          tip: l10n.tipViewerSnapshotTake,
        ),
        viewerBarGapBox(viewerBarGap),
        _SnapshotShowButton(
          hasSnapshot: hasSnapshot,
          onHold: onSnapshotHold,
        ),
      ];

  /// The five transport buttons and the clock, one instrument at one spacing.
  ///
  /// Round gathers them into a pill (§12.1); Sharp is handed the very
  /// same widgets with nothing wrapped round them.
  ///
  /// [clock] is the ladder's last step but one: on the narrowest bar the five
  /// buttons stand alone (see [_barMinimum]).
  List<Widget> _transport(LumitTheme t, {bool clock = true}) {
    final buttons = <Widget>[
      viewerBarMark(
        key: const ValueKey('viewer-home'),
        icon: LumitIcon.toStart,
        colour: t.textMuted,
        onPressed: () => onSeek(0),
        tip: l10n.tipTransportStart,
      ),
      viewerBarGapBox(viewerTransportGap),
      viewerBarMark(
        key: const ValueKey('viewer-step-back'),
        icon: LumitIcon.previousFrame,
        colour: t.textMuted,
        onPressed: () => onSeek(frame - 1),
        tip: l10n.tipTransportPrevious,
      ),
      viewerBarGapBox(viewerTransportGap),
      // The one lit mark on the bar: the control the eye goes to without
      // looking for it (the drawing's own `.ico.on`).
      viewerBarMark(
        key: const ValueKey('viewer-play'),
        icon: playing ? LumitIcon.pause : LumitIcon.play,
        colour: t.textPrimary,
        onPressed: onPlayPause,
        tip: playing ? l10n.tipTransportPause : l10n.tipTransportPlay,
      ),
      viewerBarGapBox(viewerTransportGap),
      viewerBarMark(
        key: const ValueKey('viewer-step-forward'),
        icon: LumitIcon.nextFrame,
        colour: t.textMuted,
        onPressed: () => onSeek(frame + 1),
        tip: l10n.tipTransportNext,
      ),
      viewerBarGapBox(viewerTransportGap),
      viewerBarMark(
        key: const ValueKey('viewer-end'),
        icon: LumitIcon.toEnd,
        colour: t.textMuted,
        onPressed: () => onSeek(comp.durationFrames() - 1),
        tip: l10n.tipTransportEnd,
      ),
    ];
    return [
      if (detached)
        Container(
          key: const ValueKey('viewer-transport-pill'),
          padding: const EdgeInsets.symmetric(horizontal: 2),
          decoration: BoxDecoration(
            color: t.surface3,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          child: Row(mainAxisSize: MainAxisSize.min, children: buttons),
        )
      else
        ...buttons,
      // One edge to allow for rather than two: the clock is text, and text
      // carries no button edge of its own.
      if (clock) SizedBox(width: viewerTransportGap - viewerMarkEdge),
      // The clock, in a slot wide enough for the longest time this comp can
      // show, and clickable to type one (docs/07 §2.2 item 11). A time past
      // either end of the composition lands on that end.
      if (clock)
        TimeReadout(
          key: const ValueKey('viewer-timecode'),
          frame: frame,
          format: (f) => timecodeOf(f, settings),
          widthChars: timecodeChars(settings.fpsNum, settings.fpsDen),
          style: t.mono
              .copyWith(fontSize: viewerTimecodeSize, color: t.textPrimary),
          parse: (text) =>
              framesOfTimecode(text, settings.fpsNum, settings.fpsDen),
          onCommit: onSeek,
          minFrame: 0,
          maxFrame: _lastFrameOf(settings),
          tooltip: l10n.tipFrameOnScreen,
        ),
    ];
  }
}

/// **The bar's shedding ladder, and what is left at the end of it** (§12A.6
/// and the owner's ruling on the order).
///
/// In plain terms: the bar cannot hold everything on a Viewer docked into a
/// sidebar, so things leave. **The transport is the last to go** — a person
/// who has narrowed the Viewer is still watching something, and a panel that
/// keeps the exposure field and loses Play has kept the wrong half. The clock
/// stands with it until the very end, because a picture with no time on it is
/// a picture you cannot say anything about.
///
/// Narrowing, in order:
///
/// 1. **the two gaps close** and the bar stops spreading ([_barMinimum]) — the
///    reading and the transport come together rather than a word being cut;
/// 2. **the reading sheds its own statements**, arrowed preview size then
///    composition name, which is the ladder inside [viewerReadoutLadder];
/// 3. **the reading goes entirely** ([_barKeepsReading]) — every one of its
///    facts is said again in the header, the tabs or the clock;
/// 4. **the ways of looking fold into one overflow mark**
///    ([_barKeepsLooking]), which is §12A.6's step 4 exactly: a toolbar
///    collapses into a menu rather than shrinking or clipping;
/// 5. **the clock goes** ([_barKeepsClock]);
/// 6. **the five transport buttons stand alone**, and only if the bar is
///    narrower than *those* does it finally slide sideways (step 5).
///
/// The numbers are the widths at which the pieces below them stop fitting,
/// rounded outward, and `viewer_metrics_test` walks the whole ladder.
const double _barMinimum = 560;

/// Below this the bar drops the reading and keeps the controls.
const double _barKeepsReading = 460;

/// Below this the ways of looking fold into the overflow mark.
const double _barKeepsLooking = 400;

/// Below this the clock goes and the transport stands alone.
const double _barKeepsClock = 280;

/// The one mark the ways of looking fold into on a narrow bar.
///
/// It opens the **same widgets** in a floating strip — not a menu written out
/// a second time, which is the version that goes stale. A control that works
/// on the bar works here, including the ones that are themselves menus.
class _LookingOverflow extends StatelessWidget {
  final List<Widget> Function() marks;
  const _LookingOverflow({required this.marks});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: l10n.tipViewerMoreControls,
      child: Builder(
        builder: (menuContext) => HouseButton(
          key: const ValueKey('viewer-overflow'),
          frameless: true,
          padding: EdgeInsets.zero,
          onPressed: () => _open(menuContext),
          child: SizedBox(
            width: viewerBarIconSize,
            height: viewerStripHeight - 2 * viewerMarkEdge,
            child: Center(child: Text('⋯', style: t.small)),
          ),
        ),
      ),
    );
  }

  void _open(BuildContext context) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    // Above the mark: the bar is at the bottom of the panel, so a strip hung
    // under it would be off the window.
    final over = box.localToGlobal(Offset(0, -viewerStripHeight - 6));
    showLumitPopup<void>(
      context: context,
      position: over,
      builder: (close) => FloatSurface(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
          child: Row(mainAxisSize: MainAxisSize.min, children: marks()),
        ),
      ),
    );
  }
}

/// **What is on screen, in one line**: the composition, the time, the
/// pixels the engine actually made, and the magnification they are drawn at.
///
/// It is the drawing's right-hand end, and it absorbs the degradation badge
/// (docs/07 §2.2 item 9) that used to come and go beside the transport: a
/// reading that always says `1920×1080 → 960×540` states the tier plainly, in
/// the one place a person already looks to ask what they are looking at, and
/// without a box appearing mid-playback and dragging the bar about.
/// **What it sheds, and in what order** (§12A.6's ladder). The reading is
/// four statements on one line, so step 1 — "flexible text ellipsises" — is not
/// one decision but four, and cutting the line at the ellipsis would take the
/// magnification, which is the part a person is most often watching.
///
/// So, narrowing:
///
/// 1. it **takes room from the two gaps** either side of the transport, which
///    slides the transport off centre rather than shortening a word;
/// 2. it drops the **arrowed preview size** (`→ 960×540`) — the tier is the
///    least of what the line says, and the picture itself shows it;
/// 3. it drops the **composition's name**, which the panel's header and the
///    composition tabs both still carry;
/// 4. and only then does what is left — the time, the size, the magnification —
///    **ellipsise**. In practice the bar reaches [_barMinimum] and scrolls
///    (step 5) before that, so a value is never cut.
List<String> viewerReadoutLadder({
  required String comp,
  required String time,
  required String source,
  required String preview,
  required String zoom,
}) =>
    [
      l10n.viewerReadout(comp, time, source, preview, zoom),
      l10n.viewerReadoutNoPreview(comp, time, source, zoom),
      l10n.viewerReadoutNoComp(time, source, zoom),
    ];

class _Readout extends StatelessWidget {
  final CompositionReference comp;
  final BridgeCompSettings settings;
  final BridgeCompSize compSize;
  final int frame;
  final int tier;
  final double shownScale;

  const _Readout({
    required this.comp,
    required this.settings,
    required this.compSize,
    required this.frame,
    required this.tier,
    required this.shownScale,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final divisor = tier < 1 ? 1 : tier;
    final style =
        t.mono.copyWith(fontSize: barValueTextSize, color: t.textMuted);
    final rungs = viewerReadoutLadder(
      comp: settings.name,
      time: timecodeOf(frame, settings),
      source: '${compSize.width}×${compSize.height}',
      preview: '${compSize.width ~/ divisor}×${compSize.height ~/ divisor}',
      zoom: '${(shownScale * 100).round()}%',
    );
    return LayoutBuilder(
      builder: (context, constraints) => Text(
        _widestThatFits(rungs, style, constraints.maxWidth, context),
        key: const ValueKey('viewer-readout'),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        softWrap: false,
        style: style,
      ),
    );
  }

  /// The first rung of the ladder that fits [maxWidth], or the last one — which
  /// is then left to the ellipsis. Measured rather than guessed at: the reading
  /// is mono, but the composition's name is not a fixed number of characters.
  static String _widestThatFits(
    List<String> rungs,
    TextStyle style,
    double maxWidth,
    BuildContext context,
  ) {
    if (!maxWidth.isFinite) return rungs.first;
    final scaler = MediaQuery.textScalerOf(context);
    for (final rung in rungs) {
      final painter = TextPainter(
        text: TextSpan(text: rung, style: style),
        textDirection: TextDirection.ltr,
        textScaler: scaler,
      )..layout();
      final width = painter.width;
      painter.dispose();
      if (width <= maxWidth) return rung;
    }
    return rungs.last;
  }
}

/// The channel picker's mark, and the menu of names behind it.
///
/// A bare mark rather than a boxed dropdown, which is what the drawing draws:
/// the answer is a colour, and a border round a colour is a box round a colour.
///
/// **The closed face is the answer, in the answer's own colour** (§5): the
/// Channels indicator is the one glyph in the set that carries real colour, and
/// it carries it here — the tri-colour mark for RGB, and a single circle in the
/// channel's own colour for R, G and B. Alpha is not a colour, so its circle is
/// the near-white a matte is drawn in, which is also the only light circle on
/// the bar and so tells itself apart from the three.
class _ChannelPicker extends StatelessWidget {
  final ViewerChannel channel;
  final ValueChanged<ViewerChannel> onChannel;

  const _ChannelPicker({required this.channel, required this.onChannel});

  static String _label(ViewerChannel c) => switch (c) {
        ViewerChannel.rgb => 'RGB',
        ViewerChannel.red => engineLabel('Red'),
        ViewerChannel.green => engineLabel('Green'),
        ViewerChannel.blue => engineLabel('Blue'),
        ViewerChannel.alpha => l10n.channelAlpha,
      };

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Builder(
      builder: (context) => LumitTooltip(
        message: l10n.tipViewerChannel,
        child: HouseButton(
          key: const ValueKey('viewer-channel'),
          frameless: true,
          padding: EdgeInsets.zero,
          onPressed: () {
            final box = context.findRenderObject();
            if (box is! RenderBox) return;
            showMenuAt<void>(
              context: context,
              position: box.localToGlobal(Offset(0, box.size.height + 2)),
              rows: (close) => [
                for (final c in ViewerChannel.values)
                  MenuRow(
                    key: ValueKey<String>('viewer-channel-${c.name}'),
                    onPressed: () {
                      close(null);
                      onChannel(c);
                    },
                    child: Row(children: [
                      menuTick(c == channel),
                      Text(_label(c)),
                    ]),
                  ),
              ],
            );
          },
          child: SizedBox(
            width: viewerBarIconSize,
            height: viewerStripHeight - 2 * viewerMarkEdge,
            child: Center(
              // Unkeyed: the bar's order is asserted by the keys of the
              // controls standing on it, and the face is part of one rather
              // than another. What finds it is its painter's own type.
              child: SizedBox(
                width: viewerBarIconSize,
                height: viewerBarIconSize,
                child: CustomPaint(
                  painter: ChannelFacePainter(channel: channel, theme: t),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The channel picker's closed face: a coloured circle for the view in force.
///
/// **Why it is painted rather than set from the icon set.** Every glyph in the
/// set is one colour, taken from the text colour around it (§5); this mark is
/// the set's one stated exception — three circles that fill per viewed channel —
/// and three colours cannot come out of one font glyph. The geometry is the set
/// glyph's own, so the mark is the same mark: three circles of r 3.8 on the 16
/// grid, at (8, 5.5), (6, 9.5) and (10, 9.5), and a centre dot of r 1.2.
///
/// The three colours are the Scopes panel's ([ScopeColours.standard]) — the one
/// place in the theme module that names a red, a green and a blue, and the right
/// ones by meaning: a scope's red trace and a red channel view are the same red
/// channel. Alpha takes `text_primary`, the near-white a matte reads as.
class ChannelFacePainter extends CustomPainter {
  final ViewerChannel channel;
  final LumitTheme theme;

  const ChannelFacePainter({required this.channel, required this.theme});

  /// The single circle's colour for a channel, or null for RGB — which is the
  /// tri-colour mark rather than one circle.
  static Color? single(LumitTheme t, ViewerChannel c) => switch (c) {
        ViewerChannel.rgb => null,
        ViewerChannel.red => ScopeColours.standard.red,
        ViewerChannel.green => ScopeColours.standard.green,
        ViewerChannel.blue => ScopeColours.standard.blue,
        ViewerChannel.alpha => t.textPrimary,
      };

  @override
  void paint(Canvas canvas, Size size) {
    // The set's own 16 grid, scaled to whatever the bar renders the mark at.
    final k = size.width / 16;
    final one = single(theme, channel);
    if (one != null) {
      // One circle, filling the cell as the three together do — a lone r 3.8
      // would read as a smaller mark than RGB rather than a different one.
      canvas.drawCircle(Offset(8 * k, 8 * k), 4.5 * k, Paint()..color = one);
      return;
    }
    const centres = [Offset(8, 5.5), Offset(6, 9.5), Offset(10, 9.5)];
    final colours = [
      ScopeColours.standard.red,
      ScopeColours.standard.green,
      ScopeColours.standard.blue,
    ];
    for (var i = 0; i < 3; i++) {
      canvas.drawCircle(
        centres[i] * k,
        3.8 * k,
        Paint()..color = colours[i].withValues(alpha: 0.9),
      );
    }
    canvas.drawCircle(
      Offset(8 * k, 8 * k),
      1.2 * k,
      Paint()..color = theme.textPrimary,
    );
  }

  @override
  bool shouldRepaint(ChannelFacePainter old) =>
      old.channel != channel || old.theme != theme;
}

/// **Show the snapshot**, the second half of the pair.
///
/// A **press and hold** puts the stored picture back over the live one for as
/// long as the button is down — the before/after read every grade leans on —
/// and releasing it is the whole of a comparison's life. Nothing crosses the
/// bridge: what is stored is what the stage's own boundary rasterised.
///
/// **Its own mark rather than a hold on Take**. Folding both gestures onto one
/// glyph left a taken snapshot with nothing on screen to say it existed or how
/// to see it: the only way to find the comparison was to hold a button that, as
/// far as anyone could tell, took photographs. A second mark states the
/// affordance — and states its absence, by standing muted with a tooltip saying
/// why, until one has been taken.
///
/// A raw [Listener] rather than a gesture recogniser: the comparison must last
/// exactly as long as the button is down, and a recogniser only reports once
/// the gesture is over.
class _SnapshotShowButton extends StatelessWidget {
  final bool hasSnapshot;
  final ValueChanged<bool> onHold;

  const _SnapshotShowButton({
    required this.hasSnapshot,
    required this.onHold,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Listener(
      onPointerDown: hasSnapshot ? (_) => onHold(true) : null,
      onPointerUp: hasSnapshot ? (_) => onHold(false) : null,
      onPointerCancel: hasSnapshot ? (_) => onHold(false) : null,
      child: viewerBarMark(
        key: const ValueKey('viewer-snapshot-show'),
        icon: LumitIcon.eye,
        colour: hasSnapshot ? t.textPrimary : t.textDisabled,
        // The press is the Listener's. This only says whether the control is
        // live — what mutes it, and what stops the pointer becoming a hand
        // over a button that does nothing.
        onPressed: hasSnapshot ? () {} : null,
        tip: hasSnapshot
            ? l10n.tipViewerSnapshotShow
            : l10n.tipViewerSnapshotNone,
      ),
    );
  }
}

/// `HH:MM:SS:FF` for `frame` at the comp's rate — the shared clock face in
/// state/timecode.dart, bound to this comp's settings.
String timecodeOf(int frame, BridgeCompSettings settings) =>
    timecodeOfRate(frame, settings.fpsNum, settings.fpsDen);

/// The last frame of a comp, from its settings alone.
///
/// Worked out here rather than asked of the engine: this is read while the bar
/// is being built, and the bar is built for every frame of playback.
/// Whole-integer arithmetic, so a long comp at 29.97 cannot drift the way a
/// double would.
int _lastFrameOf(BridgeCompSettings settings) {
  final den = settings.duration.den.toInt() * settings.fpsDen;
  if (den <= 0) return 0;
  final frames = settings.duration.num.toInt() * settings.fpsNum ~/ den;
  return frames > 0 ? frames - 1 : 0;
}
