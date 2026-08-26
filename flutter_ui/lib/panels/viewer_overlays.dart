// The Viewer's grid and safe-area overlays, and the bar menu that turns them
// on (K-416, docs/07 §2.2 items 5–6).
//
// **In plain terms.** Two sets of thin lines drawn on top of the picture to
// help you place things: a grid that divides the frame into eighths, and the
// two rectangles broadcast has always used — 90 % of the frame for anything
// that matters, 80 % for anything with words in it. Neither is in the picture:
// they are drawn by the display, over it, and no export has ever seen them.
//
// Everything here works in **comp space**: the marks are worked out from the
// rectangle the picture is drawn in, so they zoom and pan with it rather than
// sitting still on the panel while the shot moves underneath.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_panel_frb.dart'
    show
        showViewerBackgroundPicker,
        viewerBackgroundColour,
        viewerBarIconSize,
        viewerMarkEdge,
        viewerStripHeight;

/// The bar's **view menu** (K-416, K-466): one mark, and everything that is
/// drawn over the picture behind it.
///
/// It is a *menu* rather than a row of toggles because the approved drawing
/// gives the bottom bar one overlay glyph, and because §2.2 items 5–6 have more
/// to come — rulers, draggable guides, snapping — which land as entries here
/// rather than as more chrome on a bar that is one row. What it governs:
///
/// * the **grid** and the **safe areas** (K-416), the two marks the display
///   draws over the shot;
/// * the **layer controls** (K-217) — the wireframe boxes, the handles and the
///   hover highlight, which are marks over the picture like any other;
/// * the **region of interest** (K-362), armed or cleared from the same place;
/// * the composition's own **background** (K-357), which is the same question
///   asked from behind the picture rather than in front of it.
///
/// The face reads in the accent while anything it governs is drawn, which is
/// the promise every toggle on this bar makes: a mark over the picture is never
/// a state you can be in without being told.
class ViewerGuidesMenu extends StatelessWidget {
  /// Whether the layer controls are drawn. Panel state, so it is handed in
  /// rather than read here.
  final bool wireframes;
  final VoidCallback onWireframes;

  /// The fronted composition, and its background off the held read model —
  /// what the background row shows and edits.
  final CompositionReference comp;
  final F32Array4? background;

  const ViewerGuidesMenu({
    super.key,
    required this.wireframes,
    required this.onWireframes,
    required this.comp,
    required this.background,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Watched, not read: the menu closes on a pick, so the face has to catch up
    // from the state rather than from its own rebuild.
    final ui = context.watch<LumitUiState>();
    final on = ui.viewerOverlays;
    final lit = on.grid ||
        on.safeAreas ||
        ui.regionOfInterest != null ||
        ui.armingRegion;
    return LumitTooltip(
      message: l10n.tipViewerGuides,
      child: Builder(
        builder: (context) => HouseButton(
          key: const ValueKey('viewer-guides-menu'),
          frameless: true,
          padding: EdgeInsets.zero,
          onPressed: () => _open(context, ui),
          child: SizedBox(
            width: viewerBarIconSize,
            height: viewerStripHeight - 2 * viewerMarkEdge,
            child: Center(
              child: lumitIcon(
                LumitIcon.grid,
                size: viewerBarIconSize,
                color: lit ? t.accent : t.textMuted,
              ),
            ),
          ),
        ),
      ),
    );
  }

  void _open(BuildContext context, LumitUiState ui) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    final t = ThemeScope.of(context).theme;
    final region = ui.regionOfInterest != null;
    // The bar sits at the bottom of the panel, so a menu anchored under it
    // would hang off the window — showLumitPopup's layout pulls it back on
    // screen, which is the same thing the background picker relies on.
    final under = box.localToGlobal(Offset(0, box.size.height + 6));
    showMenuAt<void>(
      context: context,
      position: under,
      rows: (close) => [
        for (final row
            in <({String key, String text, bool on, void Function() pick})>[
          (
            key: 'viewer-guides-grid',
            text: l10n.viewerOverlayGrid,
            on: ui.viewerOverlays.grid,
            pick: () => ui.setViewerOverlays(grid: !ui.viewerOverlays.grid),
          ),
          (
            key: 'viewer-guides-safe',
            text: l10n.viewerOverlaySafeAreas,
            on: ui.viewerOverlays.safeAreas,
            pick: () =>
                ui.setViewerOverlays(safeAreas: !ui.viewerOverlays.safeAreas),
          ),
          (
            key: 'viewer-wireframes',
            text: l10n.viewerOverlayLayerControls,
            on: wireframes,
            pick: onWireframes,
          ),
          (
            key: 'viewer-region',
            // A region that exists is cleared; otherwise the next drag on the
            // picture is armed to sweep one out (K-362).
            text: region
                ? l10n.viewerOverlayClearRegion
                : l10n.viewerOverlayRegion,
            on: region || ui.armingRegion,
            pick: () => region
                ? ui.setRegionOfInterest(null)
                : ui.armingRegion = !ui.armingRegion,
          ),
        ])
          MenuRow(
            key: ValueKey<String>(row.key),
            onPressed: () {
              close(null);
              row.pick();
            },
            child: Row(
              children: [
                menuTick(row.on),
                Text(row.text),
              ],
            ),
          ),
        // The one row that is a document edit rather than a way of looking, so
        // it carries the colour it would write.
        MenuRow(
          key: const ValueKey('viewer-background'),
          onPressed: () {
            close(null);
            showViewerBackgroundPicker(
              context: context,
              comp: comp,
              background: background,
              position: under,
            );
          },
          child: Row(
            children: [
              SizedBox(
                width: 16,
                child: Center(
                  child: Container(
                    width: 10,
                    height: 10,
                    decoration: BoxDecoration(
                      color: viewerBackgroundColour(background),
                      border: Border.all(color: t.hairlineStrong),
                      borderRadius:
                          BorderRadius.circular(t.tokens.controlRadius),
                    ),
                  ),
                ),
              ),
              Text(l10n.viewerOverlayBackground),
            ],
          ),
        ),
      ],
    );
  }
}

/// The grid and the safe rectangles, over the picture.
///
/// [picture] is where the picture is drawn in the panel, so every mark below is
/// a fraction of *it* — which is the whole of why the overlays zoom with the
/// shot instead of floating over it.
class ViewerOverlayPainter extends CustomPainter {
  final Rect picture;
  final bool grid;
  final bool safeAreas;

  /// The grid's line and the safe rectangles' line. Two colours because they
  /// are two weights of statement: the grid is scaffolding to place things
  /// against, the safe areas are a boundary that means something.
  final Color gridLine;
  final Color safeLine;

  const ViewerOverlayPainter({
    required this.picture,
    required this.grid,
    required this.safeAreas,
    required this.gridLine,
    required this.safeLine,
  });

  /// How many parts the grid cuts the frame into, each way.
  ///
  /// **Eight, and proportional** — the frame's own eighths, not a spacing in
  /// pixels. docs/07 §2.2 item 6 asks for "grid" and no more; a proportional
  /// grid is the one After Effects draws and the only kind that means the same
  /// thing on a 4K comp as on a 720p one. Eight includes the halves and the
  /// quarters, which are the lines anyone actually places against, and stays
  /// legible at the size a Viewer is usually docked at.
  static const int divisions = 8;

  @override
  void paint(Canvas canvas, Size size) {
    // Bounded by the panel, exactly as the transparency board is (K-230): at a
    // high magnification the picture is far bigger than the panel, and there is
    // no reason to draw a line where nobody can see it.
    final area = picture.intersect(Offset.zero & size);
    if (area.isEmpty || picture.isEmpty) return;
    canvas.save();
    canvas.clipRect(area);
    if (grid) _paintGrid(canvas);
    if (safeAreas) _paintSafeAreas(canvas);
    canvas.restore();
  }

  void _paintGrid(Canvas canvas) {
    final paint = Paint()
      ..color = gridLine
      ..strokeWidth = 1;
    for (var i = 1; i < divisions; i++) {
      final x = picture.left + picture.width * i / divisions;
      final y = picture.top + picture.height * i / divisions;
      canvas.drawLine(Offset(x, picture.top), Offset(x, picture.bottom), paint);
      canvas.drawLine(Offset(picture.left, y), Offset(picture.right, y), paint);
    }
  }

  /// The two broadcast rectangles: **action safe** at 90 % of the frame and
  /// **title safe** at 80 %, centred, drawn as plain hairlines with square
  /// corners. No rounded corners and no labels — the shape is the whole of what
  /// they say, and a rounded corner would be a design where a boundary is
  /// wanted.
  void _paintSafeAreas(Canvas canvas) {
    final paint = Paint()
      ..color = safeLine
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    for (final fraction in const [0.9, 0.8]) {
      final inset = (1 - fraction) / 2;
      canvas.drawRect(
        Rect.fromLTRB(
          picture.left + picture.width * inset,
          picture.top + picture.height * inset,
          picture.right - picture.width * inset,
          picture.bottom - picture.height * inset,
        ),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(ViewerOverlayPainter old) =>
      old.picture != picture ||
      old.grid != grid ||
      old.safeAreas != safeAreas ||
      old.gridLine != gridLine ||
      old.safeLine != safeLine;
}
