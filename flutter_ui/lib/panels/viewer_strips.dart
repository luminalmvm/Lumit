// The Viewer's **strip vocabulary**, and the header strip built out of it: the
// shared sizes both strips are measured in, the mark and the gap box every
// control on them is made of, and the three pickers — magnification, preview
// quality, colour pipeline — the drawing puts at the header's right.
//
// Split out of viewer_panel_frb.dart. The bottom bar is a file of its own
// (viewer_bar.dart) and is built from these same pieces, which is why they
// live here rather than in either strip.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/settings.dart';
import '../state/viewer_view.dart';
import '../state/workspace.dart' show ViewerLook;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show showMenuAt;

/// The magnifications the picker offers. `null` means fit-to-panel, which is
/// the default and the only one that changes as the panel is resized.
const List<double?> _zoomSteps = [null, 0.25, 0.5, 1.0, 2.0, 4.0];

/// The Viewer's two strips are the same height as every other panel header and
/// bottom bar (§12A.6): 22, whichever density is set.
const double viewerStripHeight = 22;

/// The room either end of both strips — the drawing's `padding: 0 10px`.
const double viewerStripPadding = 10;

/// The gap between the marks on the bottom bar, and between the three pickers
/// in the header. Two numbers because the drawing draws two.
const double viewerBarGap = 8;
const double viewerHeaderGap = 6;

/// The gap inside the transport, which is one instrument and is spaced as one.
const double viewerTransportGap = 10;

/// **Every glyph on the Viewer's bars is 14**: the size the
/// approved drawing computes for each of them, rather than the 16 a panel icon
/// takes or the 20 the transport used to. A 22px strip has 14 of room in it
/// once the mark is given air above and below.
const double viewerBarIconSize = 14;

/// The seam between the ways of looking and the snapshot beside them: a
/// hairline 12 tall, standing in the middle of a 22px bar.
const double viewerBarDividerHeight = 12;

/// The clock on the bar, and the composition's own reading at its right-hand
/// end: 11px mono for the time, 10 for the reading (the drawing's sizes).
const double viewerTimecodeSize = 11;

/// The 1px transparent edge every [HouseButton] carries so that a hover cannot
/// grow it and shuffle the row beside it. It is not drawn, but it is laid out,
/// so a mark's box is 2 wider and 2 taller than its glyph's cell — and the gaps
/// between marks, which the drawing measures between the *glyphs*, are stated
/// 2 short of the drawing's number for the same reason.
const double viewerMarkEdge = 1;

/// One mark on the Viewer's bars: the glyph at its drawn size, in a
/// cell as tall as the strip so the aim is a bar's worth of target rather than
/// a 14px square (§7.2).
Widget viewerBarMark({
  required Key key,
  required LumitIcon icon,
  required Color colour,
  required VoidCallback? onPressed,
  required String tip,
}) =>
    LumitTooltip(
      message: tip,
      child: HouseButton(
        key: key,
        frameless: true,
        padding: EdgeInsets.zero,
        onPressed: onPressed,
        child: SizedBox(
          width: viewerBarIconSize,
          height: viewerStripHeight - 2 * viewerMarkEdge,
          child: Center(
            child: lumitIcon(icon, size: viewerBarIconSize, color: colour),
          ),
        ),
      ),
    );

/// The room between two marks' boxes that leaves [glyphGap] between the glyphs
/// themselves — the number the drawing states.
Widget viewerBarGapBox(double glyphGap) =>
    SizedBox(width: glyphGap - 2 * viewerMarkEdge);

/// The strip's own ground: `surface_2` welded to the panel edge under Sharp,
/// and a tile of its own — rounded, outlined, shadowed — under Round.
BoxDecoration viewerStripDecoration(LumitTheme t, bool detached) =>
    BoxDecoration(
      color: t.surface2,
      borderRadius:
          detached ? BorderRadius.circular(t.tokens.floatRadius) : null,
      border: detached ? Border.all(color: t.hairline) : null,
      boxShadow: detached ? t.tokens.cardShadow : null,
    );

/// The Viewer's **panel header strip** (§12A.6: 22 tall): the panel's
/// own kicker, then the three pickers the approved drawing puts at its right —
/// the magnification, the preview quality, and the colour pipeline.
///
/// **Why the Viewer draws its own strip.** It docks as a pane of its own rather
/// than as a tab in a group, so the dock puts no header above it; without this
/// the one panel whose drawing shows a title had none at all.
class ViewerHeader extends StatelessWidget {
  /// The magnification being *headed for* — null for fit, which is a rule
  /// rather than a number.
  final double? zoom;

  /// The magnification actually on screen, which is what the face reads when a
  /// wheel notch has left the listed steps behind.
  final double shownScale;

  final ViewerLook look;

  /// Whether the tone map is offered at all (Settings → Interface). [look] is
  /// already gated to match, so hiding it never strands an engaged one.
  final bool showToneMap;
  final VoidCallback onToneMap;
  final ValueChanged<double?> onZoom;
  final bool detached;

  const ViewerHeader({
    super.key,
    required this.zoom,
    required this.shownScale,
    required this.look,
    required this.showToneMap,
    required this.onToneMap,
    required this.onZoom,
    required this.detached,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey('viewer-header'),
      height: viewerStripHeight,
      decoration: viewerStripDecoration(t, detached),
      padding: const EdgeInsets.symmetric(horizontal: viewerStripPadding),
      // The header strip narrows exactly as the bottom bar does (§12A.6's
      // ladder): the panel's name ellipsises first, and below the width the
      // three pickers themselves need, the strip slides sideways rather than
      // painting over its own edge. Before this it was a plain `Row` with a
      // `Spacer`, and a Viewer docked narrower than the pickers — which is
      // most of a 1080p sidebar — overflowed on every frame.
      child: LayoutBuilder(
        builder: (context, constraints) {
          // The panel's name, a kicker like every other container label
          // (§7.1), and lit because this is the container rather than one of
          // several tabs in it.
          final title = Text(l10n.panelViewer.toUpperCase(),
              style: t.kickerOn, maxLines: 1, overflow: TextOverflow.ellipsis);
          final pickers = viewerPickers(
            zoom: zoom,
            shownScale: shownScale,
            look: look,
            showToneMap: showToneMap,
            onToneMap: onToneMap,
            onZoom: onZoom,
          );
          if (constraints.maxWidth >= _headerMinimum) {
            // The title is not flexible here: it and the `Spacer` would then
            // share the free space between them, and the pickers would stop
            // at the strip's right-hand *padding*. Above the minimum there is
            // room for the whole word anyway — below it, the strip slides.
            return Row(children: [title, const Spacer(), ...pickers]);
          }
          return SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                title,
                const SizedBox(width: _headerGatheredGap),
                ...pickers,
              ],
            ),
          );
        },
      ),
    );
  }
}

/// Below this the header strip stops spreading and starts scrolling: the three
/// pickers at their own widths, the panel's name, and air between them.
const double _headerMinimum = 360;

/// What stands between the name and the pickers once the strip is sliding and
/// there is no free space left to hold them apart.
const double _headerGatheredGap = 24;

/// Which route frames take from the engine to the Viewer, in words — the
/// quality picker's tooltip, where the two playback behaviours are chosen.
///
/// The bridge is asked once and kept — it reports what this build compiled to,
/// and it was asked for in a `build()` that runs for each frame of playback.
/// The wording is a getter over that answer, so it follows the language.
final BridgeViewerTransport _transport = viewerTransport();
String get _transportName => switch (_transport) {
      BridgeViewerTransport.sharedTexture => l10n.transportSharedTexture,
      BridgeViewerTransport.dmaBuf => l10n.transportDmaBuf,
      BridgeViewerTransport.readBack => l10n.transportReadBack,
    };

/// The three pickers the drawing puts at the header's right-hand end, 6 apart.
///
/// A list rather than a widget because the strip they sit in is not always the
/// header: with the bars gathered into one they lead the bottom bar instead,
/// in this same order.
List<Widget> viewerPickers({
  required double? zoom,
  required double shownScale,
  required ViewerLook look,
  required bool showToneMap,
  required VoidCallback onToneMap,
  required ValueChanged<double?> onZoom,
}) =>
    [
      // The picture's scale. The face hugs its own label: "Fit" and "400%"
      // are different widths, and a common box left a gap that read as a
      // missing control.
      BareDropdown<int>(
        key: const ValueKey('viewer-zoom'),
        dense: true,
        // -1: a wheel zoom between the listed steps; the face then reads the
        // true percentage and the menu still offers the steps.
        value: _zoomSteps.indexOf(zoom),
        options: [for (var i = 0; i < _zoomSteps.length; i++) i],
        label: (i) => i == -1
            ? '${(shownScale * 100).round()}%'
            : _zoomSteps[i] == null
                ? l10n.menuFit
                : '${(_zoomSteps[i]! * 100).round()}%',
        onChanged: (i) => onZoom(_zoomSteps[i]),
      ),
      const SizedBox(width: viewerHeaderGap),
      const _QualityDropdown(key: ValueKey('viewer-resolution')),
      const SizedBox(width: viewerHeaderGap),
      _ColourDropdown(
        key: const ValueKey('viewer-colour'),
        look: look,
        showToneMap: showToneMap,
        onToneMap: onToneMap,
      ),
    ];

/// **How good the preview is** — the header's middle picker.
///
/// It carries two answers that used to sit apart: the preview resolution
/// (docs/07 §2.2 item 2), whose name the closed face reads, and the playback
/// behaviour, whose button the drawing takes off the bar. They belong in one
/// menu because they are one question — how much quality this preview is
/// allowed to spend — and asking it in two places was how a soft picture and a
/// slow transport came to look like two unrelated faults.
class _QualityDropdown extends StatelessWidget {
  const _QualityDropdown({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context);
    final adaptive = ui.workspace.performance.playback == PlaybackMode.adaptive;
    return LumitTooltip(
      // Which route frames take to get here rides in the tooltip: a build
      // without a zero-copy path copies every pixel down and uploads it
      // again, which is the difference between playback feeling immediate and
      // feeling heavy, so it is worth being able to read off the screen.
      message: adaptive
          ? l10n.tipPlaybackAdaptive(_transportName)
          : l10n.tipPlaybackEveryFrame(_transportName),
      child: Builder(
        builder: (context) => dropdownButton(
          t: t,
          dense: true,
          onPressed: () => _open(context, t, ui),
          face: dropdownFace(t, ui.previewResolution.title),
        ),
      ),
    );
  }

  void _open(BuildContext context, LumitTheme t, LumitUiState ui) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    showMenuAt<void>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      // **Every row here is an option row**: both halves of this menu
      // change the picture in front of you, and picking one is usually
      // comparing it with the last. So the menu stays until the pointer
      // leaves it, and the ticks are redrawn in place — which is what the
      // builder is for, since a row that stays has to be able to change its
      // mind about which one is ticked.
      rows: (close) => [
        StatefulBuilder(
          builder: (context, redraw) => Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _menuHeading(t, l10n.viewerQualityResolution),
              for (final resolution in PreviewResolution.values)
                MenuRow.option(
                  key: ValueKey<String>('viewer-quality-${resolution.name}'),
                  onPressed: () {
                    ui.setPreviewResolution(resolution);
                    redraw(() {});
                  },
                  child: Row(children: [
                    menuTick(resolution == ui.previewResolution),
                    Text(resolution.title),
                  ]),
                ),
              _menuHeading(t, l10n.viewerQualityPlayback),
              for (final mode in PlaybackMode.values)
                MenuRow.option(
                  key: ValueKey<String>('viewer-playback-${mode.name}'),
                  onPressed: () {
                    ui.workspace.performance.playback = mode;
                    ui.workspace.touch();
                    redraw(() {});
                  },
                  child: Row(children: [
                    // Read live rather than from the face the menu opened
                    // with: a row that stays open outlives it.
                    menuTick(mode == ui.workspace.performance.playback),
                    Text(mode == PlaybackMode.adaptive
                        ? l10n.playbackAdaptiveShort
                        : l10n.playbackEveryFrame),
                  ]),
                ),
            ],
          ),
        ),
      ],
    );
  }
}

/// A heading over a run of menu rows — the same aside a grouped dropdown draws.
Widget _menuHeading(LumitTheme t, String text) => Padding(
      padding: const EdgeInsets.fromLTRB(10, 6, 10, 2),
      child: Text(text, style: t.small.copyWith(color: t.textMuted)),
    );

/// **What am I looking at?** — the colour pipeline, the header's third picker
/// (docs/07 §2.2 item 8).
///
/// It always names the display transform the picture is being shown through:
/// working space to display. With no colour config that is the one built-in
/// pair, scene-linear to sRGB (docs/06 §3.3); with one loaded the menu grows a
/// section per display the config declares, each of its views a row, and the
/// face names the view in force (docs/impl/ocio.md §6.2).
///
/// **And while either preview-only control is engaged, it says so.** Exposure
/// and the tone map live inside that same display transform and change
/// nothing the export will ever see. The statement that *the picture is not the
/// export* belongs here, stated calmly rather than warned about (15-DESIGN) —
/// a reading you can take without leaving the picture.
///
/// **A config that is not in force is said, not hidden.** A missing or refused
/// one leaves the picture on the built-in transform, so the face says the
/// config is not in force and the menu carries the reason in one quiet line, in
/// the same words the Project settings row uses.
///
/// It was a read-only badge at the right-hand end of the bar until the drawing
/// made it a picker; the tone map came with it, off a bar seat the drawing does
/// not have and into the menu of the transform it lives inside.
class _ColourDropdown extends StatelessWidget {
  final ViewerLook look;
  final bool showToneMap;
  final VoidCallback onToneMap;

  const _ColourDropdown({
    super.key,
    required this.look,
    required this.showToneMap,
    required this.onToneMap,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context);
    // A held answer, never a bridge call: the summary is fetched when the
    // document changes and read from Dart here.
    final summary = ui.colourSummary;
    final engaged = look.stops != 0 || look.toneMap;
    final name = _faceName(summary, ui.colourView);
    return LumitTooltip(
      message: engaged ? l10n.tipViewerPreviewView : l10n.tipDisplayTransform,
      child: Builder(
        builder: (context) => dropdownButton(
          t: t,
          dense: true,
          onPressed: () => _open(context, t, ui, summary),
          face: dropdownFace(
            t,
            '',
            face: Flexible(
              child: Text(
                engaged ? l10n.viewerDisplayTransformPreview(name) : name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(color: engaged ? t.accent : null),
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// What the closed face reads: the view in force, the built-in transform, or
  /// — where a config is named but not usable — that it is not in force.
  static String _faceName(BridgeColourSummary summary, List<String>? view) {
    if (view != null && view.length == 2) {
      return l10n.viewerColourViewFace(view.last, view.first);
    }
    if (summary.path.isNotEmpty && !summary.loaded) {
      return l10n.viewerColourConfigOff;
    }
    return l10n.viewerDisplayTransform;
  }

  void _open(BuildContext context, LumitTheme t, LumitUiState ui,
      BridgeColourSummary summary) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    final view = ui.colourView;
    final problem = summary.path.isEmpty || summary.loaded
        ? null
        : colourProblem(summary.problem, {
              for (final arg in summary.problemArgs) arg.name: arg.value,
            }) ??
            summary.problemEnglish;
    showMenuAt<void>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      rows: (close) => [
        // The built-in transform: the no-config face, and where a view is in
        // force this is how to come back to it.
        MenuRow(
          key: const ValueKey('viewer-colour-transform'),
          onPressed: () {
            close(null);
            ui.setColourView(null);
          },
          child: Row(children: [
            menuTick(view == null),
            Text(l10n.viewerDisplayTransform),
          ]),
        ),
        // Why the config is not doing anything, said where the picture is
        // named rather than left for the user to find in the settings.
        if (problem != null && problem.isNotEmpty)
          Padding(
            key: const ValueKey('viewer-colour-problem'),
            padding: const EdgeInsets.fromLTRB(10, 6, 10, 2),
            child: SizedBox(
              width: 260,
              child: Text(problem, style: t.small.copyWith(color: t.textMuted)),
            ),
          ),
        // One section per display, its views the rows — the config's own
        // words, in the config's own order, never translated.
        for (final display in summary.displays) ...[
          _menuHeading(t, display.name),
          for (final name in display.views)
            MenuRow(
              key: ValueKey<String>('viewer-colour-view-${display.name}-$name'),
              onPressed: () {
                close(null);
                ui.setColourView([display.name, name]);
              },
              child: Row(children: [
                menuTick(view != null &&
                    view.first == display.name &&
                    view.last == name),
                Text(name),
              ]),
            ),
        ],
        if (showToneMap)
          MenuRow(
            key: const ValueKey('viewer-tone-map'),
            onPressed: () {
              close(null);
              onToneMap();
            },
            child: Row(children: [
              menuTick(look.toneMap),
              Text(l10n.viewerColourToneMap),
            ]),
          ),
      ],
    );
  }
}
