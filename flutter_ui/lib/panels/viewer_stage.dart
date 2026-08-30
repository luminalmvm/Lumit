// The Viewer's **stage**: the picture, its checkerboard, and every mark drawn
// over it — the layer controls, the tool layers, the guides, the region, a held
// snapshot, and the chips in the corner.
//
// Split out of viewer_panel_frb.dart (K-007): the panel above it holds the
// magnification, the pan and the transport, and hands this one a rectangle to
// draw in. Nothing here changed in the move.

// Aliased, and not as `ui`: this file already calls the session state `ui`, and
// a local of that name would shadow the prefix where it is needed.
import 'dart:ui' as dartui;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/layer_bounds.dart' show shapeContentsRect, textLayerBounds;
import '../shell/tool_bar_frb.dart';
import '../state/tools.dart';
import '../state/workspace.dart' show ViewerOverlays;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'viewer_anchor.dart';
import 'viewer_camera.dart';
import 'viewer_dropper_layer.dart';
import 'viewer_gizmo.dart';
import 'viewer_layer_map.dart';
import 'viewer_overlays.dart';
import 'viewer_paint.dart';
import 'viewer_prefix_chip.dart';
import 'viewer_region.dart';
import 'viewer_rotate.dart';
import 'viewer_rulers.dart';
import 'viewer_shape_layer.dart';
import 'viewer_shapes.dart';
import 'viewer_tool_cursor.dart';
import 'viewer_track.dart';
import 'viewer_type.dart';
import 'viewer_zoom.dart';

/// Which channel the picture shows.
enum ViewerChannel { rgb, red, green, blue, alpha }

/// What is painted around the picture (K-203).
///
/// Neutral by default, and deliberately so: a grade cannot be judged against a
/// tinted surround, which is why the theme carries `viewerSurround` as a grey
/// no scheme colours (docs/15-DESIGN §2.1/§11). The Appearance toggle exists
/// because a neutral rectangle in the middle of a themed shell is something
/// people reasonably want to switch off — the same shape of answer the scopes
/// toggle gives.
Color viewerSurroundFor(LumitTheme t, {bool themed = false}) =>
    themed ? t.surface0 : t.viewerSurround;

/// The picture, its checkerboard, and the layer controls.
class ViewerStage extends StatelessWidget {
  final CompositionReference comp;
  final LumitUiState uiState;
  final Rect fitted;
  final bool grid;

  /// Which of the guides menu's marks are drawn over the picture (K-416,
  /// K-689): the grid, the safe rectangles and the rulers.
  final ViewerOverlays overlays;

  /// The boundary the panel photographs for a snapshot — round the picture
  /// alone, so the marks over it are not in the photograph.
  final GlobalKey pictureKey;

  /// The stored snapshot, while the Show button is held; null the rest of the
  /// time, which is nearly always.
  final dartui.Image? snapshot;

  /// Which slice of the picture that snapshot is, in fractions of the picture's
  /// rectangle (K-612): the whole of it unless it was taken while zoomed in,
  /// where it is the part that was on screen. Null when nothing is stored.
  final Rect? snapshotArea;

  /// Whether the layer controls are drawn (the bar's wireframe switch).
  final bool wireframes;
  final ViewerChannel channel;

  /// The comp's own pixel size, measured once by the panel.
  final BridgeCompSize compSize;

  /// The comp's footage layers' sources, read by the panel once per rebuild —
  /// not here, where playback would re-read them per frame.
  final List<FootageReference> footage;
  final ValueChanged<Offset> onPan;
  final VoidCallback onChanged;

  /// The Zoom tool's two gestures (K-218), applied by the panel because only it
  /// holds the magnification.
  final void Function(Offset at, {required bool out}) onZoomAt;
  final void Function(Rect box, {required bool out}) onZoomBox;

  const ViewerStage({
    super.key,
    required this.comp,
    required this.uiState,
    required this.fitted,
    required this.grid,
    required this.overlays,
    required this.pictureKey,
    required this.snapshot,
    required this.snapshotArea,
    required this.wireframes,
    required this.channel,
    required this.compSize,
    required this.footage,
    required this.onPan,
    required this.onChanged,
    required this.onZoomAt,
    required this.onZoomBox,
  });

  /// The tracked layer whose solved point cloud is drawn, and whether it is
  /// also the one taking clicks (K-417, docs/07 §2.3.6).
  ///
  /// Found in the read model the panel already holds (K-184): the first layer
  /// carrying an **enabled** Camera track whose Show points is on. No bridge
  /// call, and no per-paint walk of anything the engine owns.
  ({LayerReference layer, bool selecting})? _cloud() {
    final picked = uiState.selectedLayerIds;
    for (final entry in uiState.model.heldLayers) {
      for (final fx in entry.info.effects) {
        if (fx.name != 'camera_track' || !fx.enabled) continue;
        var show = false;
        for (final v in fx.values) {
          if (v.id != 'show_points') continue;
          if (v.value case BridgeEffectValue_Bool(:final field0)) show = field0;
        }
        if (!show) continue;
        return (
          layer: entry.layer,
          selecting: picked.contains(entry.layer.internallayerId),
        );
      }
    }
    return null;
  }

  /// Every layer of the comp with its box, top of the stack first — what the
  /// gizmo hit-tests, outlines and drags (K-217).
  ///
  /// Built from the read model (K-184), so this costs no bridge calls per
  /// paint. Three kinds are left out on purpose: a Camera has no picture to put
  /// a box round; a layer whose position is a curve has no single point a drag
  /// could add to — it would be a box drawn in the wrong place, which is worse
  /// than none; and **a layer switched off is not on the picture at all**
  /// (K-230), so it gets no wireframe and takes no click. Switching a layer's
  /// eye off is how you get it out of the way; a box round something invisible,
  /// and a click that selected it, put it right back in the way.
  List<LayerBox> _boxes() {
    if (fitted.isEmpty) return const [];
    final model = uiState.model;
    // The held copy, not a checked one (K-230): this runs on every rebuild, and
    // a pan rebuilds on every movement of the pointer. A change to the document
    // refreshes the model and repaints this from the new one, so checking here
    // only asked the engine a question the answer to which was always no.
    final revision = model.heldRevision;
    // Where the keyed masks actually are at the frame on screen (K-342). Held
    // against the document and the playhead, so this costs nothing on a hover
    // and re-asks only when one of the two has moved — and only when a mask
    // whose shape is **drawn** is actually path-animated. Both halves of that
    // are known here for free: the read model carries every mask's `pathKeys`,
    // and a mask outline is drawn only on an outlined layer
    // ([LumitUiState.outlinedLayerIds], which the gizmo picks its boxes with).
    //
    // Asking regardless was ~0.7 ms of *every frame of every scrub* on the
    // owner's project — a sixth of a frame's budget spent interpolating three
    // masks nothing on screen was going to draw (ui-performance §3.4, §4.5).
    // A keyed mask on an outlined layer is still asked per frame, because its
    // vertices genuinely differ frame by frame; that is the whole point.
    final outlined = uiState.outlinedLayerIds;
    uiState.animatedMaskPaths.refresh(
      comp: comp,
      frame: uiState.playheadFrame.value,
      revision: revision,
      anyAnimated: model.heldLayers.any((entry) =>
          outlined.contains(entry.layer.internallayerId.toString()) &&
          entry.info.masks.any((mask) => mask.pathKeys.isNotEmpty)),
    );
    final viewScale = compSize.width == 0 ? 1.0 : fitted.width / compSize.width;
    // Where the playhead is in seconds, which is the clock a shape item's keys
    // cross the bridge on: a keyed repeater's copies are part of the layer's
    // box, so the wireframe has to be measured at the frame on screen (K-553).
    final playheadSeconds = uiState.playheadFrame.value / model.heldFps;
    double? still(BridgeScalar s) => s is BridgeScalar_Static ? s.field0 : null;

    final out = <LayerBox>[];
    for (final entry in model.heldLayers) {
      if (entry.info.kind == BridgeLayerKind.camera) continue;
      if (!entry.info.switches.visible) continue;
      // A value scrub in the property rows previews the picture at a
      // provisional transform while the document still holds the old one, so
      // the box is drawn from that same provisional value and the two move
      // together (the reasoning [LumitUiState.liveRotations] sets out, for the
      // rows rather than the on-picture tools). Absent whenever nothing is
      // being dragged, which is nearly always.
      final tf = uiState.liveTransforms.value[entry.layer.internallayerId] ??
          entry.info.transform;
      final px = still(tf.positionX);
      final py = still(tf.positionY);
      if (px == null || py == null) continue;
      final sx = still(tf.scaleX);
      final sy = still(tf.scaleY);
      final rotation = still(tf.rotation);
      final live = uiState.liveText.value[entry.layer.internallayerId];
      out.add(LayerBox(
        layer: entry.layer,
        id: entry.layer.internallayerId,
        map: ViewerLayerMap.of(
          positionX: px,
          positionY: py,
          anchorX: still(tf.anchorX) ?? 0,
          anchorY: still(tf.anchorY) ?? 0,
          scaleXPercent: sx ?? 100,
          scaleYPercent: sy ?? 100,
          rotationDegrees: rotation ?? 0,
          origin: fitted.topLeft,
          viewScale: viewScale,
        ),
        // A line being typed measures what is being typed (K-232): the
        // document holds the old one until the edit ends, so a box measured
        // from it would not grow with the words.
        bounds: live == null
            ? uiState.layerBounds.boundsOf(entry,
                compSize: compSize, revision: revision, t: playheadSeconds)
            : textLayerBounds(live.text, live.size),
        draggable: true,
        scalable: sx != null && sy != null && rotation != null,
        rotationDegrees: rotation ?? 0,
        // An animated mask draws where the picture has it, not where its
        // still path was last written (K-342).
        masks: [
          for (final mask in entry.info.masks)
            switch (uiState.animatedMaskPaths
                .pathOf(entry.layer.internallayerId, mask.id)) {
              final live? => maskWithVertices(mask, live),
              _ => mask,
            }
        ],
        shapeContents: entry.info.shapeContents,
        // Where the art's box starts, which is where the layer's pixels do
        // (K-308) — without it every drawn point sat a box away from its art.
        artOrigin:
            shapeContentsRect(entry.info.shapeContents, t: playheadSeconds)
                    ?.topLeft ??
                Offset.zero,
      ));
    }
    return out;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Listened to rather than read: the tool is armed from the toolbar, which
    // is not in this panel's rebuild path, so without this neither the pointer
    // nor the tool overlays would catch up until something else redrew the
    // Viewer. The whole stage is inside the builder for that reason — handing
    // it in as a cached `child` kept the *pointer* current while every tool
    // layer under it stayed armed for whichever tool was in hand when the panel
    // last rebuilt (K-225).
    //
    // The dropper is listened to beside the tools, and for the same reason: it
    // is armed from a parameter row in another panel, and arming it takes the
    // drag away from the pan below (see [_stage]).
    return ListenableBuilder(
      listenable: Listenable.merge([uiState.tools, uiState.dropper]),
      builder: (context, _) => MouseRegion(
        // Which pointer the armed tool wears over the picture.
        cursor: viewerCursorFor(uiState.tools.tool),
        child: _stage(context, t),
      ),
    );
  }

  /// Where a held snapshot goes: the slice of the picture it was taken from,
  /// measured against the picture as it stands now (K-612). A snapshot taken
  /// while the whole picture was on screen covers the whole of it, as it always
  /// did; one taken zoomed in covers the part it photographed, wherever that
  /// part has since moved to.
  Rect _snapshotRect() {
    final area = snapshotArea;
    if (area == null) return fitted;
    return Rect.fromLTRB(
      fitted.left + area.left * fitted.width,
      fitted.top + area.top * fitted.height,
      fitted.left + area.right * fitted.width,
      fitted.top + area.bottom * fitted.height,
    );
  }

  Widget _stage(BuildContext context, LumitTheme t) {
    // **While a pick is armed, the drag is the dropper's** (K-532, docs/07
    // §6.1). Every tool layer in the stack below settles this by sitting above
    // the pan and taking the hit; the dropper cannot, because it reads raw
    // pointer events rather than recognising a gesture — a [Listener] never
    // joins the arena, so this recogniser went on winning it underneath and
    // picking a colour dragged the whole preview about. The one place the two
    // can be told apart is here, where the pan is declared: an armed pick
    // means no pan recogniser exists at all, and disarming brings it back.
    final picking = uiState.dropper.value != null;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      // Panning the picture, not the layer: the overlay's own handle takes
      // the gesture first when it is hit, so this only fires on empty space.
      onPanUpdate: picking ? null : (d) => onPan(d.delta),
      // **Nothing the stage draws may leave the stage.** Every mark over the
      // picture — the wireframes, the handles, the mask outlines, the tool
      // layers — is a [CustomPaint] filling the stack, and a painter is free
      // to draw outside the box it was given: a layer parked off the edge of
      // the comp, or a picture zoomed past the panel, puts its box beyond the
      // panel's own rectangle. A [Stack] does not stop it, because the clip it
      // carries is only applied when a *positioned child* is measured
      // overflowing, which a `Positioned.fill` painter never is. So the marks
      // landed on whatever sat next to the Viewer — the node graph, during a
      // split drag, which is where this was reported. The panel's rounded-tile
      // wrapper happened to clip them under one theme shape and not the other,
      // which is exactly the kind of coincidence a guarantee should not rest
      // on. Here it holds whatever the shape, the zoom or the layout is doing.
      child: ClipRect(
          child: Container(
        color: viewerSurroundFor(
          t,
          themed: uiState.workspace.themedViewerSurround,
        ),
        child: Stack(
          children: [
            // The checkerboard covers the panel and is clipped to the
            // picture, rather than being a widget the size of the picture
            // (K-230): at 800 % on an HD composition that widget was 15360
            // pixels across, and painting an 8-pixel grid over it meant half
            // a million rectangles for the few thousand actually on screen.
            // That, and not the rendering, is what made zooming in seize the
            // whole window.
            if (grid)
              Positioned.fill(
                child: CustomPaint(painter: _CheckerPainter(t, fitted)),
              ),
            // The picture, inside the boundary a snapshot is taken of
            // (K-416). Everything below in this stack is a sibling of it, so
            // the photograph is the picture and not the marks over it.
            Positioned.fromRect(
              rect: fitted,
              child: RepaintBoundary(
                key: pictureKey,
                child: _Picture(
                  uiState: uiState,
                  channel: channel,
                  shownScale:
                      compSize.width == 0 ? 1 : fitted.width / compSize.width,
                ),
              ),
            ),
            // The grid and the safe areas (K-416): over the picture, under the
            // layer controls, and worked out from the picture's own rectangle
            // so they zoom and pan with the shot.
            if (overlays.grid || overlays.safeAreas)
              Positioned.fill(
                key: const ValueKey('viewer-overlay-guides'),
                child: IgnorePointer(
                  child: CustomPaint(
                    painter: ViewerOverlayPainter(
                      picture: fitted,
                      grid: overlays.grid,
                      safeAreas: overlays.safeAreas,
                      gridLine: t.hairline,
                      safeLine: t.hairlineStrong,
                    ),
                  ),
                ),
              ),
            _missingSlate(),
            // The layer controls and every tool that reads the boxes, under
            // one builder: [_boxes] walks the whole model, and it used to be
            // called once per tool layer — six times per build, sixty times a
            // second during playback. One walk now serves all of them.
            Positioned.fill(
              child: ListenableBuilder(
                // Four things move the boxes without the panel being rebuilt:
                // the selection (a Timeline click), a probe landing with a
                // clip's real size, an edit changing a transform, and a turn
                // in flight from the Rotation tool (K-230).
                listenable: Listenable.merge([
                  uiState.selectedLayers,
                  // Picking a mask's **Path** row outlines its layer without
                  // the layer ever being clicked (K-341), so it is half of
                  // `outlinedLayerIds` — and therefore half of whether the
                  // masks below are asked for at the frame on screen. Without
                  // it the outline would appear and draw the stored shape.
                  uiState.selectedProperties,
                  uiState.layerBounds,
                  uiState.model,
                  uiState.liveRotations,
                  uiState.liveText,
                  uiState.liveTransforms,
                ]),
                builder: (context, _) {
                  final boxes = _boxes();
                  final state = Provider.of<LumitState>(context, listen: false);
                  return Stack(
                    children: [
                      // The layer controls. With the Hand tool armed this only
                      // draws — it lets every gesture through to the pan above,
                      // which is the whole difference between the two tools
                      // over the picture.
                      ViewerGizmoLayer(
                        comp: comp,
                        uiState: uiState,
                        boxes: boxes,
                        showControls: wireframes,
                        tool: uiState.tools.tool,
                        // The pivot, while the tool that turns about it is in
                        // hand.
                        showAnchors:
                            uiState.tools.tool.group == ToolGroup.rotate,
                        // What a dragged layer reaches for (K-689): a guide is
                        // kept in comp pixels, and these two put it on screen.
                        picture: fitted,
                        compSize: Size(
                          compSize.width.toDouble(),
                          compSize.height.toDouble(),
                        ),
                        onChanged: onChanged,
                      ),
                      // The shape tools and the Pen: a drag draws a mask on
                      // the selected layer, and the Pen builds one point by
                      // point (K-222, K-223).
                      ViewerShapeLayer(
                        active: uiState.tools.tool.group == ToolGroup.shape ||
                            uiState.tools.tool == ToolMode.pen,
                        tool: uiState.tools.tool,
                        state: state,
                        uiState: uiState,
                        boxes: boxes,
                        comp: comp,
                        fitted: fitted,
                        compSize: Size(
                          compSize.width.toDouble(),
                          compSize.height.toDouble(),
                        ),
                        accent: t.accent,
                        onChanged: onChanged,
                      ),
                      // The Type tool: a click makes or edits a text layer,
                      // and what is typed is previewed until the edit ends
                      // (K-225).
                      ViewerTypeLayer(
                        active: uiState.tools.tool.group == ToolGroup.type,
                        tool: uiState.tools.tool,
                        comp: comp,
                        state: state,
                        uiState: uiState,
                        boxes: boxes,
                        fitted: fitted,
                        compSize: Size(
                          compSize.width.toDouble(),
                          compSize.height.toDouble(),
                        ),
                        accent: t.accent,
                        onChanged: onChanged,
                      ),
                      // The painting tools: a drag paints a stroke on the
                      // selected layer (K-227), under the brush ring K-226
                      // gave them.
                      ViewerPaintLayer(
                        active: uiState.tools.tool.group == ToolGroup.paint,
                        tool: uiState.tools.tool,
                        state: state,
                        uiState: uiState,
                        boxes: boxes,
                        viewScale: compSize.width == 0
                            ? 1.0
                            : fitted.width / compSize.width,
                        onChanged: onChanged,
                      ),
                      // The Anchor point tool: its own pointer, and a drag
                      // that slides the pivot while the picture stays still
                      // (K-220).
                      ViewerAnchorLayer(
                        active: uiState.tools.tool.group == ToolGroup.anchor,
                        comp: comp,
                        uiState: uiState,
                        boxes: boxes,
                        mark: t.textPrimary,
                        outline: t.surface0,
                        accent: t.accent,
                        onChanged: onChanged,
                      ),
                      // The Rotation tool: its own pointer, and a drag that
                      // turns the selection about each layer's anchor (K-219).
                      ViewerRotateLayer(
                        active: uiState.tools.tool.group == ToolGroup.rotate,
                        comp: comp,
                        uiState: uiState,
                        boxes: boxes,
                        mark: t.textPrimary,
                        outline: t.surface0,
                        onChanged: onChanged,
                      ),
                    ],
                  );
                },
              ),
            ),
            // The camera tools: a drag orbits, tracks or dollies the comp's
            // active camera (K-229).
            ViewerCameraLayer(
              active: uiState.tools.tool.group == ToolGroup.camera,
              tool: uiState.tools.tool,
              comp: comp,
              state: Provider.of<LumitState>(context, listen: false),
              uiState: uiState,
              fitted: fitted,
              compSize: Size(
                compSize.width.toDouble(),
                compSize.height.toDouble(),
              ),
              mark: t.textPrimary,
              outline: t.surface0,
              accent: t.accent,
              onChanged: onChanged,
            ),
            // The solved point cloud on the tracked layer (K-417): drawn
            // whenever Show points is on and a solve exists, and taking the
            // pointer only while that layer is the selected one — a cloud that
            // always took clicks would make the whole shot unselectable.
            //
            // **Under a listener of its own** (K-430). Two things decide
            // whether there is a cloud at all, and neither of them rebuilds
            // this panel: switching the effect off, which is a change to the
            // model, and an analysis landing, which is a change to nothing the
            // document holds. Read outside a builder, the cloud stayed on the
            // picture after the effect was disabled and stayed off it after a
            // solve arrived, in both cases until the frame happened to change.
            Positioned.fill(
              child: ListenableBuilder(
                listenable: Listenable.merge(
                  [uiState.model, uiState.solveLanded],
                ),
                builder: (context, _) {
                  final cloud = _cloud();
                  // An empty box takes no hit of its own, so the picture under
                  // it stays clickable.
                  if (cloud == null) return const SizedBox.shrink();
                  // Listened to rather than read: the playhead moving does not
                  // rebuild this panel by itself (it asks the engine for a
                  // frame, and the picture arriving is what redraws), and the
                  // cloud has to follow the frame it is drawn over.
                  return ValueListenableBuilder<int>(
                    valueListenable: uiState.playheadFrame,
                    builder: (context, frame, _) => ViewerTrackLayer(
                      key: ValueKey<String>(
                          'viewer-track-${cloud.layer.internallayerId}'),
                      tracked: cloud.layer,
                      selecting: cloud.selecting,
                      fitted: fitted,
                      compSize: Size(
                        compSize.width.toDouble(),
                        compSize.height.toDouble(),
                      ),
                      playheadFrame: frame,
                      revision: uiState.model.heldRevision,
                      generation: uiState.solveLanded.value,
                      accent: t.accent,
                      mark: t.textPrimary,
                      onChanged: onChanged,
                    ),
                  );
                },
              ),
            ),
            // The region of interest (K-362): the outline whenever one is set,
            // and — only while armed — the drag that sweeps a new one. Above
            // the layer controls for the same reason the Zoom tool is: while a
            // region is being swept, the whole picture is the target.
            ViewerRegionLayer(
              arming: uiState.armingRegion,
              fitted: fitted,
              region: uiState.regionOfInterest,
              accent: t.accent,
              onRegion: uiState.setRegionOfInterest,
            ),
            // Over the layer controls, and inert unless the Zoom tool is
            // armed: while it is, the whole picture is its target and no
            // handle underneath may take a click meant for a magnification.
            ViewerZoomLayer(
              active: uiState.tools.tool.group == ToolGroup.zoom,
              onZoomAt: onZoomAt,
              onZoomBox: onZoomBox,
              accent: t.accent,
              mark: t.textPrimary,
              outline: t.surface0,
            ),
            // The Hand tool: the drawn hand, and the drag that pans (K-230).
            // It takes the drag rather than leaving it to the stage beneath,
            // so the hand keeps following the pointer while the button is
            // down — which is when it matters most.
            ViewerHandLayer(
              active: uiState.tools.tool.group == ToolGroup.hand,
              onPan: onPan,
              mark: t.textPrimary,
              outline: t.surface0,
            ),
            // Above both, because while the dropper is armed the whole
            // picture is a target: a drag handle under the pointer must not
            // take the click that was meant to pick a pixel.
            DropperLayer(comp: comp, uiState: uiState, fitted: fitted),
            // The rulers along the top and left edges, and the guides dragged
            // out of them (K-689). Over the tool layers, because a guide is
            // grabbed by its own thin strip and a handle underneath must not
            // take a press aimed at one; under the snapshot, because a guide
            // belonging to the live picture drawn over a held one would be a
            // mark about the wrong picture. Nothing is built at all when there
            // are neither rulers up nor guides placed.
            if (overlays.rulers || uiState.guides.isNotEmpty)
              ViewerRulers(
                rulers: overlays.rulers,
                picture: fitted,
                compSize: Size(
                  compSize.width.toDouble(),
                  compSize.height.toDouble(),
                ),
                guides: uiState.guides,
                onGuides: uiState.setGuides,
                band: t.surface2,
                line: t.hairlineStrong,
                label: t.textMuted,
                // The one saturated mark the neutrality zone allows over the
                // picture: §3.2 names guides in its own exemption.
                guideColour: t.accent,
              ),
            // The held snapshot (K-416), over everything: while it is up the
            // Viewer is showing a second picture, and a wireframe belonging to
            // the live one drawn on top of it would be a lie about both. Fitted
            // to the picture's rectangle as it is *now*, so a zoom taken since
            // the snapshot compares like with like.
            if (snapshot case final shot?)
              Positioned.fromRect(
                rect: _snapshotRect(),
                child: IgnorePointer(
                  child: RawImage(
                    key: const ValueKey('viewer-snapshot-overlay'),
                    image: shot,
                    fit: BoxFit.fill,
                  ),
                ),
              ),
            // The selection's name, over the stage's own corner (K-466). Last
            // in the stack because it is chrome: it names what is selected
            // whatever is being drawn underneath, including a held snapshot.
            ViewerTag(uiState: uiState),
            // And what is being *looked at*, when that is not the finished
            // composition (K-528). Its own file, so this is one line: the chip
            // is the Viewer's, but it follows the effect selection rather than
            // anything this panel knows.
            ViewerPrefixChip(uiState: uiState),
          ],
        ),
      )),
    );
  }

  /// A notice when a footage layer in this comp has lost its file. The probe
  /// itself happens in the badge; this only places it.
  Widget _missingSlate() {
    if (footage.isEmpty) return const SizedBox.shrink();
    return Positioned(
      left: 8,
      bottom: 8,
      child: _MissingBadge(footage: footage),
    );
  }
}

/// The badge that appears only once a probe has actually found a file gone.
class _MissingBadge extends StatefulWidget {
  final List<FootageReference> footage;
  const _MissingBadge({required this.footage});

  @override
  State<_MissingBadge> createState() => _MissingBadgeState();
}

class _MissingBadgeState extends State<_MissingBadge> {
  int _missing = 0;

  @override
  void initState() {
    super.initState();
    _probe();
  }

  @override
  void didUpdateWidget(_MissingBadge old) {
    super.didUpdateWidget(old);
    if (old.footage.length != widget.footage.length) _probe();
  }

  Future<void> _probe() async {
    var count = 0;
    for (final f in widget.footage) {
      // A probe outlives the document it was started for: opening a project
      // clears the engine's registry, and every reference held from the
      // outgoing one throws from here on. There is no missing media in a
      // document that is gone — drop the count and wait to be rebuilt with the
      // new project's footage.
      try {
        if (await f.getStatus() == LumitMediaStatus.missing) count++;
      } catch (_) {
        return;
      }
    }
    if (mounted && count != _missing) setState(() => _missing = count);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (_missing == 0) return const SizedBox.shrink();
    return Container(
      key: const ValueKey('viewer-missing'),
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          lumitIcon(LumitIcon.unlink, size: iconSize, color: t.warning),
          const SizedBox(width: 6),
          Text(
            l10n.missingFileCount(_missing),
            style: t.small.copyWith(color: t.warning),
          ),
        ],
      ),
    );
  }
}

/// How the picture's texture is sampled, given how much of it lands on how
/// many screen pixels (K-631).
///
/// Three sizes decide this, and only their product matters. [shownScale] is the
/// picture's width on screen as a share of the comp's — the percentage the bar
/// reads out. [tier] is the preview divisor the frame was actually made at
/// (1 Full, 2 Half, 4 Quarter), so the texture is that many times smaller than
/// the comp. [devicePixelRatio] turns the first, which is in logical pixels,
/// into the real ones the rasteriser samples into. Multiply them and the answer
/// is how many screen pixels each texture pixel gets.
///
/// **Below one, the texture is being minified, and nearest sampling is wrong.**
/// It keeps one source pixel out of every few and throws the rest away, which
/// is not a smaller picture but a different one — edges break up, fine detail
/// crawls, and the whole frame reads as soft and faintly busy. That is what a
/// Viewer below 100 % was doing. [FilterQuality.medium] mipmaps instead, which
/// is the cheap, clean equivalent of the Lanczos the exporter resizes with
/// (K-498) — one prefiltered sample rather than a pile of taps.
///
/// At or past 1:1 the picture is *magnified*, and nearest is the honest answer:
/// a zoomed pixel should be a square, because the reason to zoom in is to look
/// at the pixels. [smooth] is the Settings toggle that hands the blending back.
///
/// Note the product: a half-resolution frame shown at 80 % is not 0.56 of
/// native, because the engine renders to the *panel's* fit rather than the
/// zoom. It is 0.8 × 2 = 1.6 texture pixels per screen pixel — magnified, and
/// filtered as such. The two never multiply behind the bar's back.
FilterQuality viewerPictureFilter({
  required double shownScale,
  required int tier,
  required double devicePixelRatio,
  required bool smooth,
}) {
  final perTexel = shownScale * (tier < 1 ? 1 : tier) * devicePixelRatio;
  if (perTexel < 0.999) return FilterQuality.medium;
  return smooth ? FilterQuality.low : FilterQuality.none;
}

/// Whatever the worker last published, in the chosen channel — always a
/// platform texture (K-183): frames only ever arrive as GPU handles.
class _Picture extends StatelessWidget {
  final LumitUiState uiState;
  final ViewerChannel channel;

  /// The picture's width on screen as a share of the comp's — the bar's
  /// magnification, and half of what decides the filter.
  final double shownScale;

  const _Picture({
    required this.uiState,
    required this.channel,
    required this.shownScale,
  });

  @override
  Widget build(BuildContext context) {
    final dpr = MediaQuery.devicePixelRatioOf(context);
    return ValueListenableBuilder<int?>(
      valueListenable: uiState.viewerFrameid,
      builder: (context, textureId, _) => ValueListenableBuilder<int>(
        valueListenable: uiState.previewTier,
        builder: (context, tier, _) {
          final picture = textureId != null
              ? Texture(
                  textureId: textureId,
                  filterQuality: viewerPictureFilter(
                    shownScale: shownScale,
                    tier: tier,
                    devicePixelRatio: dpr,
                    smooth: uiState.workspace.smoothZoomedViewer,
                  ),
                )
              : const SizedBox.expand();
          return pictureChannelFilter(channel, picture);
        },
      ),
    );
  }
}

/// [picture] shown in [channel] — clipped, which is the whole point.
///
/// The channel matrices below force alpha opaque, so they turn transparent
/// black into solid black. A filter that changes transparent pixels cannot be
/// confined to where its child painted — the rasteriser has to run it over
/// every pixel the current clip allows. Without a clip of its own that is the
/// whole window, which is why picking Red painted the toolbar and the side
/// panel flat black along with the Viewer's surround.
Widget pictureChannelFilter(ViewerChannel channel, Widget picture) {
  final filter = channelFilterFor(channel);
  if (filter == null) return picture;
  return ClipRect(child: ColorFiltered(colorFilter: filter, child: picture));
}

/// The matrix that isolates one channel, or null for the full picture.
///
/// A single channel is shown as grey rather than tinted — the point of looking
/// at one is to judge its *values*, and a red picture is harder to read than a
/// grey one. Alpha is copied into all three, which is what makes a matte
/// legible.
ColorFilter? channelFilterFor(ViewerChannel channel) => switch (channel) {
      ViewerChannel.rgb => null,
      ViewerChannel.red => const ColorFilter.matrix(<double>[
          1, 0, 0, 0, 0, //
          1, 0, 0, 0, 0, //
          1, 0, 0, 0, 0, //
          0, 0, 0, 0, 255,
        ]),
      ViewerChannel.green => const ColorFilter.matrix(<double>[
          0, 1, 0, 0, 0, //
          0, 1, 0, 0, 0, //
          0, 1, 0, 0, 0, //
          0, 0, 0, 0, 255,
        ]),
      ViewerChannel.blue => const ColorFilter.matrix(<double>[
          0, 0, 1, 0, 0, //
          0, 0, 1, 0, 0, //
          0, 0, 1, 0, 0, //
          0, 0, 0, 0, 255,
        ]),
      ViewerChannel.alpha => const ColorFilter.matrix(<double>[
          0, 0, 0, 1, 0, //
          0, 0, 0, 1, 0, //
          0, 0, 0, 1, 0, //
          0, 0, 0, 0, 255,
        ]),
    };

/// The part of the transparency board worth painting: what is both picture and
/// panel (K-230).
///
/// The board used to be a widget the size of the *picture*, which at 800 % on an
/// HD composition is 15360 pixels across — an 8-pixel grid over that is half a
/// million rectangles a paint, for the few thousand that are on screen. Bounding
/// it by the panel is what keeps the cost of the board the same at every
/// magnification.
Rect checkerArea(Rect picture, Size panel) =>
    picture.intersect(Offset.zero & panel);

/// [rect] with every edge on a whole device pixel.
///
/// The picture and its checkerboard are given the same rectangle, but they
/// rasterise it differently — the board through an anti-aliased canvas, the
/// platform texture without — so a fractional edge showed as a soft row of
/// board sticking out under the picture. Snapping the shared rectangle is
/// what makes "the same rectangle" true on screen and not just in the layout.
Rect snapToDevicePixels(Rect rect, double dpr) {
  double snap(double v) => (v * dpr).roundToDouble() / dpr;
  return Rect.fromLTRB(
      snap(rect.left), snap(rect.top), snap(rect.right), snap(rect.bottom));
}

/// The transparency checkerboard behind the picture.
///
/// [picture] is where the picture is drawn in the panel; the board fills that
/// and no more, and only the part of it that is on screen is ever painted. The
/// squares stay pinned to the picture's own top-left, so panning slides the
/// board with the picture instead of the picture swimming over a fixed grid.
class _CheckerPainter extends CustomPainter {
  final LumitTheme theme;
  final Rect picture;
  const _CheckerPainter(this.theme, this.picture);

  static const double _square = 8;

  @override
  void paint(Canvas canvas, Size size) {
    final area = checkerArea(picture, size);
    if (area.isEmpty) return;
    final light = Paint()..color = theme.surface2;
    final dark = Paint()..color = theme.surface1;
    canvas.save();
    canvas.clipRect(area);
    canvas.drawRect(area, dark);
    // Start on the square the picture's own grid has at this corner, so the
    // pattern does not shift as the picture is panned across the panel.
    double alignedStart(double edge, double origin) =>
        origin + ((edge - origin) / _square).floorToDouble() * _square;
    final startX = alignedStart(area.left, picture.left);
    final startY = alignedStart(area.top, picture.top);
    for (var y = startY; y < area.bottom; y += _square) {
      for (var x = startX; x < area.right; x += _square) {
        final odd = (((x - picture.left) / _square).round() +
                ((y - picture.top) / _square).round())
            .isOdd;
        if (odd) continue;
        canvas.drawRect(Rect.fromLTWH(x, y, _square, _square), light);
      }
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_CheckerPainter old) =>
      old.theme != theme || old.picture != picture;
}

/// Where the selection's name sits over the picture — the drawing's 16 from the
/// left edge of the stage and 8 down from its top.
const double viewerTagLeft = 16;
const double viewerTagTop = 8;

/// The selection's name over the picture's corner (K-466, the mockup's TITLE
/// chip).
///
/// **Why it is worth a mark on the picture.** Selection is agreed in four
/// places — the Timeline, the graph, the properties and the Viewer — and until
/// now the Viewer was the one that only showed it as a box. A box says *where*;
/// the chip says *what*, which is the question a comp with six similar layers
/// actually raises.
///
/// It is `animated`, not `accent` (§3.1): the closed list gives that colour to
/// "this is selected or in hand", which is exactly what this says, and it is
/// the colour of the box it names.
class ViewerTag extends StatelessWidget {
  final LumitUiState uiState;
  const ViewerTag({super.key, required this.uiState});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Off the read model and the selection, both of which move without this
    // panel being rebuilt (K-230).
    return ListenableBuilder(
      listenable: Listenable.merge([uiState.model, uiState.selectedLayers]),
      builder: (context, _) {
        final picked = uiState.selectedLayerIds;
        String? name;
        for (final entry in uiState.model.heldLayers) {
          if (picked.contains(entry.layer.internallayerId)) {
            name = entry.info.name;
            break;
          }
        }
        if (name == null || name.isEmpty) return const SizedBox.shrink();
        return Positioned(
          left: viewerTagLeft,
          top: viewerTagTop,
          child: IgnorePointer(
            child: Container(
              key: const ValueKey('viewer-tag'),
              padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
              decoration: BoxDecoration(
                border: Border.all(color: t.animated),
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              ),
              child: Text(
                name.toUpperCase(),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: t.kicker.copyWith(
                  color: t.animated,
                  letterSpacing: viewerTagTracking,
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

/// The chip's tracking: 0.08em at 9px, the drawing's own — a shade tighter than
/// the 0.12em a kicker carries, because it names a layer rather than a
/// container.
const double viewerTagTracking = 0.72;
