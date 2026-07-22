// The Viewer interaction overlays (phase F-D): the Shape-tool rubber-band, the
// transform gizmo (bounding box + anchor cross + drag-to-move), and the
// eyedropper magnifier. Drawn over the fitted image on the Viewer stage.
//
// In plain terms: this is the layer of the Viewer you interact WITH. Depending
// on the active tool it lets you drag a mask shape, drag the selected layer's
// anchor to move it, or (when the eyedropper is armed) pick a colour off the
// picture. It maps between comp pixels and screen pixels using the fitted-image
// rectangle the stage computed.
//
// Grounding: the egui overlays (crates/lumit-ui/src/shell/overlays.rs) draw the
// anchor crosshair (`anchor_overlay`) and the shape rubber-band (`shape_overlay`
// → `SetLayerMasks` with real geometry), and the eyedropper magnifier lives in
// `eyedropper.rs`. The Shape drag maps its rect into comp pixels and commits
// real geometry through the bridge's `add_mask_geometry` (v0.9). The transform
// gizmo is now the full manipulator: the LayerMap maths is ported into
// `viewer_layer_map.dart` and drives the bounding box, the corner/edge scale
// handles and the anchor pan-behind drag (the exact egui `anchor_overlay`
// behaviour) plus a body drag. egui's overlays.rs draws only the anchor cross —
// the box, handles and body drag are the Flutter "full manipulator" (06 §D);
// there is no rotation affordance (verified absent in overlays.rs).

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../bridge/bridge.dart';
import '../state/app_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'preview_source.dart';
import 'viewer_layer_map.dart';

/// The interaction layer, sized to the whole stage. [imageRect] is the fitted
/// picture's rectangle within the stage (in local coordinates); [compWidth] /
/// [compHeight] are the front comp's pixel size, for the comp↔screen mapping.
class ViewerInteractionLayer extends StatefulWidget {
  final AppStateStub app;
  final PreviewSource source;
  final Rect imageRect;
  final int compWidth;
  final int compHeight;
  const ViewerInteractionLayer({
    super.key,
    required this.app,
    required this.source,
    required this.imageRect,
    required this.compWidth,
    required this.compHeight,
  });

  @override
  State<ViewerInteractionLayer> createState() => _ViewerInteractionLayerState();
}

class _ViewerInteractionLayerState extends State<ViewerInteractionLayer> {
  // Shape-drag rubber-band (screen coords), non-null mid-drag.
  Offset? _shapeStart;
  Offset? _shapeNow;

  // Eyedropper cursor position (screen coords) and the sampled colour.
  Offset? _dropperPos;
  List<double>? _dropperRgba;

  /// The eyedropper average radius, in source pixels (Shift+scroll widens it —
  /// the egui `eyedropper` sample-radius behaviour).
  int _sampleRadius = 0;

  /// The one-off comp readback for the eyedropper on the shared-texture path
  /// (no CPU pixels to sample), fetched asynchronously through the render
  /// worker. One request per arm session ([_sampleFrameRequested]); the
  /// magnifier shows no swatch until it lands.
  DecodedFrame? _sampleFrame;
  bool _sampleFrameRequested = false;

  AppStateStub get app => widget.app;
  Rect get rect => widget.imageRect;

  /// Screen (local) → comp pixel coordinate.
  Offset _compOf(Offset local) {
    final fx = (local.dx - rect.left) / rect.width;
    final fy = (local.dy - rect.top) / rect.height;
    return Offset(fx * widget.compWidth, fy * widget.compHeight);
  }

  // --- Eyedropper --------------------------------------------------------

  /// Sample the shown frame at [local], averaging a (2r+1)² box of source
  /// pixels. Prefers the CPU frame the PreviewSource holds — pixels ALREADY
  /// read back, so sampling is free; only when none exists (the shared-texture
  /// path before its first throttled readback) does it kick off ONE async comp
  /// readback through the render worker and sample nothing until it lands.
  /// The choice over an always-async render: honest and simple — the shown
  /// frame IS what the user is picking from, and a synchronous full-scale
  /// render here froze the UI for the render's whole length (TF round 5).
  List<double>? _sampleAt(Offset local) {
    final frame = widget.source.displayedFrame ?? _sampleFrame;
    if (frame == null) {
      _requestSampleFrame(local);
      return null;
    }
    if (frame.width <= 0 || frame.height <= 0) return null;
    final fx = ((local.dx - rect.left) / rect.width).clamp(0.0, 1.0);
    final fy = ((local.dy - rect.top) / rect.height).clamp(0.0, 1.0);
    final cx = (fx * (frame.width - 1)).round();
    final cy = (fy * (frame.height - 1)).round();
    var r = 0.0, g = 0.0, b = 0.0, n = 0;
    for (var dy = -_sampleRadius; dy <= _sampleRadius; dy++) {
      for (var dx = -_sampleRadius; dx <= _sampleRadius; dx++) {
        final x = cx + dx, y = cy + dy;
        if (x < 0 || y < 0 || x >= frame.width || y >= frame.height) continue;
        final i = (y * frame.width + x) * 4;
        r += frame.rgba[i];
        g += frame.rgba[i + 1];
        b += frame.rgba[i + 2];
        n++;
      }
    }
    if (n == 0) return null;
    // The engine's Colour parameter holds scene-linear 0..1; the decoded frame
    // is straight 8-bit, so normalise (gamma is not re-applied, matching the
    // effect colour rows' convention).
    return [r / n / 255.0, g / n / 255.0, b / n / 255.0];
  }

  /// Kick off the one-off async readback that backs [_sampleAt] on the
  /// shared-texture path. At most one per arm session, whatever it returns —
  /// pointer moves must never queue up renders.
  void _requestSampleFrame(Offset local) {
    if (_sampleFrameRequested) return;
    _sampleFrameRequested = true;
    widget.source.requestSampleFrame((frame) {
      if (!mounted || frame == null) return;
      setState(() {
        _sampleFrame = frame;
        // Refresh the magnifier swatch where the cursor last was.
        final pos = _dropperPos;
        if (pos != null) _dropperRgba = _sampleAt(pos);
      });
    });
  }

  void _dropperMove(Offset local) {
    setState(() {
      _dropperPos = local;
      _dropperRgba = _sampleAt(local);
    });
  }

  void _dropperCommit(Offset local) {
    final rgba = _sampleAt(local);
    if (rgba != null) {
      app.commitEyedropper(rgba[0], rgba[1], rgba[2]);
    } else {
      app.disarmEyedropper();
    }
    setState(() {
      _dropperPos = null;
      _dropperRgba = null;
      // The arm session is over; the next arm re-fetches its own readback.
      _sampleFrame = null;
      _sampleFrameRequested = false;
    });
  }

  // --- Build -------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;

    // The eyedropper owns every pointer while armed (over the whole stage).
    if (app.eyedropperArmed) {
      return Listener(
        onPointerHover: (e) => _dropperMove(e.localPosition),
        onPointerMove: (e) => _dropperMove(e.localPosition),
        onPointerSignal: (e) {
          // Shift+scroll widens the sample average (the egui behaviour).
          if (e is PointerScrollEvent) {
            final shift = HardwareKeyboard.instance.isShiftPressed;
            if (shift) {
              setState(() {
                _sampleRadius =
                    (_sampleRadius - e.scrollDelta.dy.sign.toInt())
                        .clamp(0, 16);
                if (_dropperPos != null) _dropperRgba = _sampleAt(_dropperPos!);
              });
            }
          }
        },
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapUp: (d) => _dropperCommit(d.localPosition),
          child: MouseRegion(
            cursor: SystemMouseCursors.precise,
            child: CustomPaint(
              painter: _EyedropperPainter(
                pos: _dropperPos,
                rgba: _dropperRgba,
                theme: t,
                radius: _sampleRadius,
              ),
              child: const SizedBox.expand(),
            ),
          ),
        ),
      );
    }

    // The Shape tool: drag a rubber-band; on release commit the default mask.
    if (app.viewerTool == ToolMode.shape) {
      return GestureDetector(
        behavior: HitTestBehavior.opaque,
        onPanStart: (d) => setState(() {
          _shapeStart = d.localPosition;
          _shapeNow = d.localPosition;
        }),
        onPanUpdate: (d) => setState(() => _shapeNow = d.localPosition),
        onPanEnd: (_) {
          final start = _shapeStart, now = _shapeNow;
          setState(() {
            _shapeStart = null;
            _shapeNow = null;
          });
          // Only a real drag draws (the egui > 2px gate). The dragged rect is
          // mapped into comp pixels and committed as real geometry (bridge v0.9
          // `add_mask_geometry`), so the drawn size/position is honoured.
          if (start != null && now != null && (start - now).distance > 2) {
            final a = _compOf(start);
            final b = _compOf(now);
            final x = math.min(a.dx, b.dx);
            final y = math.min(a.dy, b.dy);
            final w = (a.dx - b.dx).abs();
            final h = (a.dy - b.dy).abs();
            app.drawShapeMask(x, y, w, h);
          }
        },
        child: MouseRegion(
          cursor: SystemMouseCursors.precise,
          child: CustomPaint(
            painter: _ShapeDragPainter(
              start: _shapeStart,
              now: _shapeNow,
              shape: app.viewerShape,
              theme: t,
            ),
            child: const SizedBox.expand(),
          ),
        ),
      );
    }

    // The transform gizmo: the selected 2D layer's bounding box, scale handles,
    // anchor cross and body drag, all mapped through the ported LayerMap (Select
    // tool). Non-interactive when no layer is selected.
    final layer = _selectedLayer();
    if (app.viewerTool == ToolMode.select && layer != null) {
      return _TransformGizmo(
        app: app,
        layer: layer,
        imageRect: rect,
        compWidth: widget.compWidth,
        compHeight: widget.compHeight,
      );
    }

    return const SizedBox.expand();
  }

  BridgeLayer? _selectedLayer() {
    final comp = app.frontComp;
    final id = app.selectedLayer;
    if (comp == null || id == null) return null;
    for (final l in comp.layers) {
      if (l.id == id) return l;
    }
    return null;
  }
}

/// One scale handle: its layer-space anchor-relative point and which axes it
/// drives. A corner drives both; an edge drives one (the perpendicular axis).
class _ScaleHandleSpec {
  final String id;
  final double lx, ly; // the handle's point in layer space
  final bool scaleX, scaleY;
  const _ScaleHandleSpec(this.id, this.lx, this.ly, this.scaleX, this.scaleY);
}

/// The full transform gizmo for a selected 2D layer, mapped through the ported
/// [ViewerLayerMap] (the egui `LayerMap`):
///   * the layer's bounding box (its native raster corners, mapped through the
///     transform),
///   * corner + edge scale handles (a drag commits `scale_x`/`scale_y` on
///     release, animation-aware),
///   * the anchor crosshair — dragging it is the egui `anchor_overlay`
///     pan-behind: the anchor moves and Position compensates so the layer stays
///     visually fixed, committed as one gesture (`anchor_x`/`anchor_y` +
///     `position_x`/`position_y`),
///   * a body drag anywhere inside the box committing `position_x`/`position_y`.
///
/// Live preview redraws the box/handles at the dragged transform while dragging;
/// the picture itself re-renders on release (the ops commit then), matching
/// egui's commit-on-`drag_stopped`. A 3D layer or a camera draws nothing (its
/// gizmo awaits the object-tools work, as in egui `anchor_overlay`).
///
/// Grounding: egui's viewer overlays (`overlays.rs`) draw ONLY the anchor cross
/// pan-behind drag (`anchor_overlay`, lines 143-242) — there is no bounding box,
/// no scale handles and no rotation affordance in egui. The box, scale handles
/// and body drag are the Flutter "full manipulator" (06 §D) the LayerMap port
/// unblocks, built on the exact ported mapping maths and committing through the
/// same animation-aware transform ops egui's `mk` closure uses. Rotation is not
/// offered (verified absent in `overlays.rs`).
class _TransformGizmo extends StatefulWidget {
  final AppStateStub app;
  final BridgeLayer layer;
  final Rect imageRect;
  final int compWidth;
  final int compHeight;
  const _TransformGizmo({
    required this.app,
    required this.layer,
    required this.imageRect,
    required this.compWidth,
    required this.compHeight,
  });

  @override
  State<_TransformGizmo> createState() => _TransformGizmoState();
}

class _TransformGizmoState extends State<_TransformGizmo> {
  /// The handle hit slop (K-116 spirit): a generous radius so a small handle is
  /// easy to grab.
  static const double _slop = 11;

  final GlobalKey _stackKey = GlobalKey();

  // In-flight drag preview state (null when idle). Exactly one preview channel
  // is live at a time, driven by the grabbed part.
  Offset? _previewAnchor; // layer space
  Offset? _previewPosition; // comp pixels
  double? _previewScaleX, _previewScaleY; // percentages

  AppStateStub get app => widget.app;
  BridgeLayer get layer => widget.layer;

  double _prop(String name, double fallback) =>
      app.transformValueFor(layer.id, name) ?? fallback;

  bool _animated(String name) => layer.transform?[name]?.animated ?? false;

  /// The layer's native raster size — a solid's own size, else the comp size.
  (double, double) get _native {
    final s = layer.solidSize;
    if (s != null && s.length == 2 && s[0] > 0 && s[1] > 0) {
      return (s[0].toDouble(), s[1].toDouble());
    }
    return (widget.compWidth.toDouble(), widget.compHeight.toDouble());
  }

  Offset _stageLocal(Offset global) {
    final box = _stackKey.currentContext?.findRenderObject() as RenderBox?;
    if (box == null) return global;
    return box.globalToLocal(global);
  }

  /// The committed transform values (no preview overrides) — the fixed map a
  /// drag inverts through, so the anchor/scale solve stays stable mid-gesture.
  ViewerLayerMap _committedMap() {
    final (nw, nh) = _native;
    return ViewerLayerMap.of(
      positionX: _prop('position_x', widget.compWidth / 2),
      positionY: _prop('position_y', widget.compHeight / 2),
      anchorX: _prop('anchor_x', nw / 2),
      anchorY: _prop('anchor_y', nh / 2),
      scaleXPercent: _prop('scale_x', 100),
      scaleYPercent: _prop('scale_y', 100),
      rotationDegrees: _prop('rotation', 0),
      origin: widget.imageRect.topLeft,
      viewScale: _viewScale,
    );
  }

  /// The map with the in-flight preview overrides folded in, for drawing.
  ViewerLayerMap _previewMap() {
    final (nw, nh) = _native;
    return ViewerLayerMap.of(
      positionX: _previewPosition?.dx ?? _prop('position_x', widget.compWidth / 2),
      positionY:
          _previewPosition?.dy ?? _prop('position_y', widget.compHeight / 2),
      anchorX: _previewAnchor?.dx ?? _prop('anchor_x', nw / 2),
      anchorY: _previewAnchor?.dy ?? _prop('anchor_y', nh / 2),
      scaleXPercent: _previewScaleX ?? _prop('scale_x', 100),
      scaleYPercent: _previewScaleY ?? _prop('scale_y', 100),
      rotationDegrees: _prop('rotation', 0),
      origin: widget.imageRect.topLeft,
      viewScale: _viewScale,
    );
  }

  double get _viewScale =>
      widget.compWidth <= 0 ? 1 : widget.imageRect.width / widget.compWidth;

  /// The eight scale handles in layer space, relative to the native raster.
  List<_ScaleHandleSpec> get _handles {
    final (w, h) = _native;
    return [
      _ScaleHandleSpec('tl', 0, 0, true, true),
      _ScaleHandleSpec('tr', w, 0, true, true),
      _ScaleHandleSpec('br', w, h, true, true),
      _ScaleHandleSpec('bl', 0, h, true, true),
      _ScaleHandleSpec('t', w / 2, 0, false, true),
      _ScaleHandleSpec('r', w, h / 2, true, false),
      _ScaleHandleSpec('b', w / 2, h, false, true),
      _ScaleHandleSpec('l', 0, h / 2, true, false),
    ];
  }

  String? get _compId => app.frontCompIdResolved;

  /// Commit one property, keeping keyed properties keyed (a key at the playhead)
  /// and static properties static — egui's `mk` closure, per property.
  void _commit(String name, double value) {
    final comp = _compId;
    if (comp == null) return;
    if (_animated(name)) {
      app.addKeyframe(comp, layer.id, name, app.previewFrame, value);
    } else {
      app.setTransform(comp, layer.id, name, value);
    }
  }

  // --- Anchor (pan-behind) ------------------------------------------------

  void _anchorUpdate(Offset global) {
    final anchor = _committedMap().layerOf(_stageLocal(global));
    setState(() => _previewAnchor = anchor);
  }

  void _anchorEnd() {
    final newAnchor = _previewAnchor;
    if (newAnchor != null) {
      final (nw, nh) = _native;
      final oldAnchor =
          Offset(_prop('anchor_x', nw / 2), _prop('anchor_y', nh / 2));
      final position = Offset(
          _prop('position_x', widget.compWidth / 2),
          _prop('position_y', widget.compHeight / 2));
      final newPos = panBehindPosition(
        oldAnchor: oldAnchor,
        newAnchor: newAnchor,
        position: position,
        scaleXPercent: _prop('scale_x', 100),
        scaleYPercent: _prop('scale_y', 100),
        rotationDegrees: _prop('rotation', 0),
      );
      _commit('anchor_x', newAnchor.dx);
      _commit('anchor_y', newAnchor.dy);
      _commit('position_x', newPos.dx);
      _commit('position_y', newPos.dy);
    }
    setState(_reset);
  }

  // --- Scale --------------------------------------------------------------

  void _scaleUpdate(_ScaleHandleSpec h, Offset global) {
    final map = _committedMap();
    final (sx, sy) = map.scaleForHandle(
      dxFromAnchor: h.scaleX ? h.lx - map.ax : 0.0,
      dyFromAnchor: h.scaleY ? h.ly - map.ay : 0.0,
      pointer: _stageLocal(global),
    );
    setState(() {
      if (h.scaleX) _previewScaleX = sx;
      if (h.scaleY) _previewScaleY = sy;
    });
  }

  void _scaleEnd(_ScaleHandleSpec h) {
    if (h.scaleX && _previewScaleX != null) {
      _commit('scale_x', _previewScaleX!);
    }
    if (h.scaleY && _previewScaleY != null) {
      _commit('scale_y', _previewScaleY!);
    }
    setState(_reset);
  }

  // --- Body (position) ----------------------------------------------------

  void _bodyStart() {
    _previewPosition = Offset(
      _prop('position_x', widget.compWidth / 2),
      _prop('position_y', widget.compHeight / 2),
    );
  }

  void _bodyUpdate(Offset delta) {
    setState(() => _previewPosition =
        (_previewPosition ?? Offset.zero) + delta / _viewScale);
  }

  void _bodyEnd() {
    final p = _previewPosition;
    if (p != null) {
      _commit('position_x', p.dx);
      _commit('position_y', p.dy);
    }
    setState(_reset);
  }

  void _reset() {
    _previewAnchor = null;
    _previewPosition = null;
    _previewScaleX = null;
    _previewScaleY = null;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (layer.switches.threeD || layer.kind == BridgeLayerKind.camera) {
      return const SizedBox.expand();
    }
    // No picture to map against yet (a zero fitted rect) — draw nothing.
    if (_viewScale <= 0 || widget.imageRect.width <= 0) {
      return const SizedBox.expand();
    }
    final map = _previewMap();
    final (w, h) = _native;
    final corners = [
      map.toScreen(0, 0),
      map.toScreen(w, 0),
      map.toScreen(w, h),
      map.toScreen(0, h),
    ];
    // Build the handle list once so the drawn positions and the hit areas share
    // the same specs.
    final handles = _handles;
    final handleScreens = <_ScaleHandleSpec, Offset>{
      for (final s in handles) s: map.toScreen(s.lx, s.ly),
    };
    final anchorScreen = map.toScreen(map.ax, map.ay);

    // The body hit area is the axis-aligned bounds of the (possibly rotated)
    // box — a generous target for the move drag, behind the handles.
    final aabb = _bounds(corners);

    return Stack(
      key: _stackKey,
      children: [
        // The box + handles + anchor cross (drawn, non-interactive).
        Positioned.fill(
          child: IgnorePointer(
            child: CustomPaint(
              painter: _GizmoPainter(
                corners: corners,
                handles: handleScreens.values.toList(),
                anchor: anchorScreen,
                theme: t,
              ),
            ),
          ),
        ),
        // Body drag (position) — under the handles so a handle wins the grab.
        Positioned.fromRect(
          rect: aabb,
          child: GestureDetector(
            key: const ValueKey('gizmo-body'),
            behavior: HitTestBehavior.translucent,
            onPanStart: (_) => setState(_bodyStart),
            onPanUpdate: (d) => _bodyUpdate(d.delta),
            onPanEnd: (_) => _bodyEnd(),
            onPanCancel: () => setState(_reset),
            child: MouseRegion(
              cursor: SystemMouseCursors.move,
              child: const SizedBox.expand(),
            ),
          ),
        ),
        // Scale handles.
        for (final s in handles)
          _handleHitArea(
            key: ValueKey('gizmo-scale-${s.id}'),
            centre: handleScreens[s]!,
            cursor: _cursorForHandle(s),
            onStart: () {},
            onUpdate: (g) => _scaleUpdate(s, g),
            onEnd: () => _scaleEnd(s),
          ),
        // Anchor cross (topmost so it wins over a corner it overlaps).
        _handleHitArea(
          key: const ValueKey('gizmo-anchor'),
          centre: anchorScreen,
          cursor: SystemMouseCursors.move,
          size: 24,
          onStart: () {},
          onUpdate: _anchorUpdate,
          onEnd: _anchorEnd,
        ),
      ],
    );
  }

  Rect _bounds(List<Offset> pts) {
    var minX = pts.first.dx, maxX = pts.first.dx;
    var minY = pts.first.dy, maxY = pts.first.dy;
    for (final p in pts) {
      minX = math.min(minX, p.dx);
      maxX = math.max(maxX, p.dx);
      minY = math.min(minY, p.dy);
      maxY = math.max(maxY, p.dy);
    }
    return Rect.fromLTRB(minX, minY, maxX, maxY);
  }

  MouseCursor _cursorForHandle(_ScaleHandleSpec s) {
    if (s.scaleX && s.scaleY) return SystemMouseCursors.resizeUpLeftDownRight;
    if (s.scaleX) return SystemMouseCursors.resizeLeftRight;
    return SystemMouseCursors.resizeUpDown;
  }

  /// A square, generous (K-116 slop) hit area centred on [centre] driving a
  /// pan-style drag through the given callbacks (globalPosition-based, so the
  /// handler can invert through the stage map).
  Widget _handleHitArea({
    required Key key,
    required Offset centre,
    required MouseCursor cursor,
    required VoidCallback onStart,
    required void Function(Offset global) onUpdate,
    required VoidCallback onEnd,
    double size = 8,
  }) {
    final hit = size + _slop * 2;
    return Positioned(
      left: centre.dx - hit / 2,
      top: centre.dy - hit / 2,
      width: hit,
      height: hit,
      child: GestureDetector(
        key: key,
        behavior: HitTestBehavior.opaque,
        onPanStart: (_) => onStart(),
        onPanUpdate: (d) => onUpdate(d.globalPosition),
        onPanEnd: (_) => onEnd(),
        onPanCancel: () => setState(_reset),
        child: MouseRegion(
          cursor: cursor,
          child: const SizedBox.expand(),
        ),
      ),
    );
  }
}

/// Draws the gizmo: the bounding box outline, the eight scale handles (small
/// hollow squares) and the anchor crosshair (a circle with a cross) — the egui
/// `anchor_overlay` mark for the cross, the box/handles a Flutter addition.
class _GizmoPainter extends CustomPainter {
  final List<Offset> corners;
  final List<Offset> handles;
  final Offset anchor;
  final LumitTheme theme;
  const _GizmoPainter({
    required this.corners,
    required this.handles,
    required this.anchor,
    required this.theme,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final line = Paint()
      ..color = theme.accent
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.0;
    if (corners.length == 4) {
      final path = Path()..moveTo(corners[0].dx, corners[0].dy);
      for (var i = 1; i < corners.length; i++) {
        path.lineTo(corners[i].dx, corners[i].dy);
      }
      path.close();
      canvas.drawPath(path, line);
    }
    // Scale handles: hollow squares filled with the surface so they read on the
    // picture, ringed in the accent.
    final fill = Paint()
      ..color = theme.surface0
      ..style = PaintingStyle.fill;
    for (final hpt in handles) {
      final r = Rect.fromCenter(center: hpt, width: 8, height: 8);
      canvas.drawRect(r, fill);
      canvas.drawRect(r, line);
    }
    // The anchor crosshair (egui `anchor_overlay`): a ringed cross.
    final cross = Paint()
      ..color = theme.accent
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    const rr = 6.0;
    canvas.drawCircle(anchor, rr, cross);
    canvas.drawLine(
        anchor - const Offset(rr + 4, 0), anchor + const Offset(rr + 4, 0), cross);
    canvas.drawLine(
        anchor - const Offset(0, rr + 4), anchor + const Offset(0, rr + 4), cross);
  }

  @override
  bool shouldRepaint(_GizmoPainter old) =>
      !_listEq(old.corners, corners) ||
      !_listEq(old.handles, handles) ||
      old.anchor != anchor ||
      old.theme != theme;

  static bool _listEq(List<Offset> a, List<Offset> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}

/// Previews the Shape-tool rubber-band outline (rectangle/ellipse/star bound) in
/// the accent while dragging — the egui `shape_overlay` clay preview.
class _ShapeDragPainter extends CustomPainter {
  final Offset? start;
  final Offset? now;
  final ShapeKind shape;
  final LumitTheme theme;
  const _ShapeDragPainter({
    required this.start,
    required this.now,
    required this.shape,
    required this.theme,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final s = start, n = now;
    if (s == null || n == null) return;
    final r = Rect.fromPoints(s, n);
    final paint = Paint()
      ..color = theme.accent
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.0;
    switch (shape) {
      case ShapeKind.rectangle:
        canvas.drawRect(r, paint);
      case ShapeKind.ellipse:
        canvas.drawOval(r, paint);
      case ShapeKind.star:
        _drawStar(canvas, r, paint);
    }
  }

  void _drawStar(Canvas canvas, Rect r, Paint paint) {
    final cx = r.center.dx, cy = r.center.dy;
    final outer = math.min(r.width, r.height) / 2;
    final inner = outer * 0.42;
    final path = Path();
    for (var i = 0; i < 10; i++) {
      final radius = i.isEven ? outer : inner;
      final a = -math.pi / 2 + i * math.pi / 5;
      final x = cx + radius * math.cos(a);
      final y = cy + radius * math.sin(a);
      i == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
    }
    path.close();
    canvas.drawPath(path, paint);
  }

  @override
  bool shouldRepaint(_ShapeDragPainter old) =>
      old.start != start || old.now != now || old.shape != shape;
}

/// The eyedropper magnifier: a ring at the cursor filled with the sampled
/// colour, the egui `eyedropper` mark. [radius] is the sample-average radius (in
/// source pixels) so a wider Shift+scroll average reads on the label.
class _EyedropperPainter extends CustomPainter {
  final Offset? pos;
  final List<double>? rgba;
  final LumitTheme theme;
  final int radius;
  const _EyedropperPainter({
    required this.pos,
    required this.rgba,
    required this.theme,
    required this.radius,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final p = pos;
    if (p == null) return;
    final ring = Paint()
      ..color = theme.textPrimary
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2.0;
    const rr = 22.0;
    if (rgba != null) {
      int ch(double f) => (f.clamp(0.0, 1.0) * 255).round();
      final fill = Paint()
        ..color = ui.Color.fromARGB(255, ch(rgba![0]), ch(rgba![1]), ch(rgba![2]));
      canvas.drawCircle(p + const Offset(28, -28), rr, fill);
    }
    canvas.drawCircle(p + const Offset(28, -28), rr, ring);
    // The crosshair at the exact sampled pixel.
    final cross = Paint()
      ..color = theme.accent
      ..strokeWidth = 1.0;
    canvas.drawLine(p - const Offset(8, 0), p + const Offset(8, 0), cross);
    canvas.drawLine(p - const Offset(0, 8), p + const Offset(0, 8), cross);
  }

  @override
  bool shouldRepaint(_EyedropperPainter old) =>
      old.pos != pos || old.rgba != rgba || old.radius != radius;
}
