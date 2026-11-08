// The Viewer's grid and safe-area overlays, and the bar menu that turns them
// on (docs/07 §2.2 items 5–6).
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
import '../state/workspace.dart' show ViewerReach;
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_menus.dart';
import 'viewer_panel_frb.dart'
    show
        showViewerBackgroundPicker,
        viewerBackgroundColour,
        viewerBarIconSize,
        viewerMarkEdge,
        viewerOffers3d,
        viewerStripHeight;

/// The bar's **view menu**: one mark, and everything that is
/// drawn over the picture behind it.
///
/// It is a *menu* rather than a row of toggles because the approved drawing
/// gives the bottom bar one overlay glyph, and because §2.2 items 5–6 have more
/// to come — rulers, draggable guides, snapping — which land as entries here
/// rather than as more chrome on a bar that is one row. What it governs:
///
/// * the **grid** and the **safe areas**, the two marks the display
///   draws over the shot;
/// * the **layer controls** — the wireframe boxes, the handles and the
///   hover highlight, which are marks over the picture like any other;
/// * the **region of interest**, armed or cleared from the same place;
/// * the composition's own **background**, which is the same question
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

  /// The Viewer view this bar belongs to, whose own 3D ground and axes the
  /// menu shows and switches.
  final int viewId;
  final ViewerMenuHost host;

  const ViewerGuidesMenu({
    super.key,
    required this.wireframes,
    required this.onWireframes,
    required this.comp,
    required this.background,
    required this.viewId,
    required this.host,
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
        on.rulers ||
        (on.reach && ui.model.isNodeGraph) ||
        // The ground through the camera is the one 3D aid that is off until
        // asked for.
        (_offers3d(ui) &&
            ui.viewPoseOf(viewId) == null &&
            ui.viewerAidsOf(viewId).ground) ||
        ui.guides.isNotEmpty ||
        ui.regionOfInterest != null ||
        ui.armingRegion;
    return LumitTooltip(
      message: l10n.tipViewerGuides,
      child: Builder(
        builder: (context) => ViewerRightClick(
          rows: (close) => _rows(context, ui, close),
          child: HouseButton(
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
      ),
    );
  }

  bool _offers3d(LumitUiState ui) => viewerOffers3d(ui, viewId);

  void _open(BuildContext context, LumitUiState ui) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    // The bar sits at the bottom of the panel, so a menu anchored under it
    // would hang off the window — showLumitPopup's layout pulls it back on
    // screen, which is the same thing the background picker relies on.
    showMenuAt<void>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height + 6)),
      rows: (close) => _rows(context, ui, () => close(null)),
    );
  }

  List<Widget> _rows(
          BuildContext context, LumitUiState ui, VoidCallback close) =>
      [
        ...viewerShowRows(ui, host, close),
        const MenuDivider(),
        ...viewerGuideRows(ui, host, close),
        const MenuDivider(),
        viewerRegionRow(ui, close),
        // The one row that is a document edit rather than a way of looking, so
        // it carries the colour it would write.
        MenuRow(
          key: const ValueKey('viewer-background'),
          onPressed: () {
            close();
            final box = context.findRenderObject();
            showViewerBackgroundPicker(
              context: context,
              comp: comp,
              background: background,
              position:
                  box is RenderBox ? box.localToGlobal(Offset.zero) : Offset.zero,
            );
          },
          child: Row(
            children: [
              SizedBox(
                width: 16,
                child: Center(
                  child: Builder(builder: (context) {
                    final t = ThemeScope.of(context).theme;
                    return Container(
                      width: 10,
                      height: 10,
                      decoration: BoxDecoration(
                        color: viewerBackgroundColour(background),
                        border: Border.all(color: t.hairlineStrong),
                        borderRadius:
                            BorderRadius.circular(t.tokens.controlRadius),
                      ),
                    );
                  }),
                ),
              ),
              Text(l10n.viewerOverlayBackground),
            ],
          ),
        ),
      ];
}

/// How far a node graph's picture reaches past the frame: a dotted outline
/// round it, and its size in comp pixels under the bottom right corner. The
/// outline turns to the warning colour where the picture reached as far as a
/// box may keep, and was cut there.
class ViewerReachPainter extends CustomPainter {
  /// Where the frame is drawn in the panel.
  final Rect picture;

  /// The comp's own pixel size, which [reach] is measured in.
  final Size comp;
  final ViewerReach reach;
  final Color line;
  final Color limitLine;

  const ViewerReachPainter({
    required this.picture,
    required this.comp,
    required this.reach,
    required this.line,
    required this.limitLine,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (comp.isEmpty || picture.isEmpty) return;
    final sx = picture.width / comp.width;
    final sy = picture.height / comp.height;
    final r = reach.rect;
    final outline = Rect.fromLTRB(
      picture.left + r.left * sx,
      picture.top + r.top * sy,
      picture.left + r.right * sx,
      picture.top + r.bottom * sy,
    );
    final colour = reach.atLimit ? limitLine : line;
    // Bounded by the panel, which the outline is often bigger than.
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    final paint = Paint()
      ..color = colour
      ..strokeWidth = 1;
    const dash = 4.0;
    void dashed(Offset a, Offset b) {
      final length = (b - a).distance;
      if (length == 0) return;
      final step = (b - a) / length;
      for (var d = 0.0; d < length; d += dash * 2) {
        final end = d + dash < length ? d + dash : length;
        canvas.drawLine(a + step * d, a + step * end, paint);
      }
    }

    dashed(outline.topLeft, outline.topRight);
    dashed(outline.topRight, outline.bottomRight);
    dashed(outline.bottomRight, outline.bottomLeft);
    dashed(outline.bottomLeft, outline.topLeft);
    final text = TextPainter(
      text: TextSpan(
        text: '${r.width.round()}×${r.height.round()}',
        style: TextStyle(color: colour, fontSize: 9),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    // Under the bottom right corner, or as near it as the panel shows.
    double within(double v, double room) => v > room ? room : (v < 2 ? 2 : v);
    final at = Offset(
      within(outline.right - text.width, size.width - text.width - 2),
      within(outline.bottom + 2, size.height - text.height - 2),
    );
    text.paint(canvas, at);
    text.dispose();
    canvas.restore();
  }

  @override
  bool shouldRepaint(ViewerReachPainter old) =>
      old.picture != picture ||
      old.comp != comp ||
      old.reach != reach ||
      old.line != line ||
      old.limitLine != limitLine;
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
    // Bounded by the panel, exactly as the transparency board is: at a
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
