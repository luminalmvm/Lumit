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
// `eyedropper.rs`. The Shape drag now maps its rect into comp pixels and commits
// real geometry through the bridge's `add_mask_geometry` (v0.9), so the drawn
// size/position is honoured. One honest gap carries over the bridge seam:
//   * the transform read-back gives Position (comp space); the full pan-behind
//     anchor maths and scale handles await the LayerMap port, so this moves the
//     layer by its Position and draws the anchor cross there.

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

  AppStateStub get app => widget.app;
  Rect get rect => widget.imageRect;

  /// Screen (local) → comp pixel coordinate.
  Offset _compOf(Offset local) {
    final fx = (local.dx - rect.left) / rect.width;
    final fy = (local.dy - rect.top) / rect.height;
    return Offset(fx * widget.compWidth, fy * widget.compHeight);
  }

  /// Comp pixel → screen (local) coordinate.
  Offset _screenOf(Offset comp) => Offset(
        rect.left + comp.dx / widget.compWidth * rect.width,
        rect.top + comp.dy / widget.compHeight * rect.height,
      );

  // --- Eyedropper --------------------------------------------------------

  /// Sample the shown frame at [local], averaging a (2r+1)² box of source
  /// pixels. Prefers the CPU frame the PreviewSource holds; on the shared path
  /// (no CPU pixels) it falls back to a one-off comp readback.
  List<double>? _sampleAt(Offset local) {
    final frame = widget.source.displayedFrame ?? app.sampleCompFrame();
    if (frame == null || frame.width <= 0 || frame.height <= 0) return null;
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

    // The transform gizmo: the selected 2D layer's anchor cross, draggable to
    // move it (Select tool). Non-interactive when no layer is selected.
    final layer = _selectedLayer();
    if (app.viewerTool == ToolMode.select && layer != null) {
      return _TransformGizmo(
        app: app,
        layer: layer,
        screenOf: _screenOf,
        compOf: _compOf,
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

/// The transform gizmo for a selected 2D layer: an anchor crosshair at the
/// layer's Position, draggable to move it (committing `position_x`/`position_y`
/// through `setTransform`). A 3D layer or a camera draws nothing (its gizmo
/// awaits the object-tools work, as in egui).
class _TransformGizmo extends StatefulWidget {
  final AppStateStub app;
  final BridgeLayer layer;
  final Offset Function(Offset comp) screenOf;
  final Offset Function(Offset local) compOf;
  const _TransformGizmo({
    required this.app,
    required this.layer,
    required this.screenOf,
    required this.compOf,
  });

  @override
  State<_TransformGizmo> createState() => _TransformGizmoState();
}

class _TransformGizmoState extends State<_TransformGizmo> {
  Offset? _drag; // the in-flight comp-space position while dragging

  double _prop(String name, double fallback) =>
      widget.app.transformValueFor(widget.layer.id, name) ?? fallback;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (widget.layer.switches.threeD ||
        widget.layer.kind == BridgeLayerKind.camera) {
      return const SizedBox.expand();
    }
    final position = _drag ??
        Offset(_prop('position_x', 0), _prop('position_y', 0));
    final centre = widget.screenOf(position);
    // A modest hit area around the cross.
    return Stack(
      children: [
        Positioned.fill(
          child: CustomPaint(painter: _AnchorPainter(centre: centre, theme: t)),
        ),
        Positioned(
          left: centre.dx - 12,
          top: centre.dy - 12,
          child: GestureDetector(
            key: const ValueKey('gizmo-anchor'),
            behavior: HitTestBehavior.opaque,
            onPanUpdate: (d) {
              setState(() => _drag = widget.compOf(
                  widget.screenOf(_drag ?? position) + d.delta));
            },
            onPanEnd: (_) {
              final p = _drag;
              if (p != null) {
                final comp = widget.app.frontCompIdResolved;
                if (comp != null) {
                  widget.app.setTransform(
                      comp, widget.layer.id, 'position_x', p.dx);
                  widget.app.setTransform(
                      comp, widget.layer.id, 'position_y', p.dy);
                }
              }
              setState(() => _drag = null);
            },
            child: MouseRegion(
              cursor: SystemMouseCursors.move,
              child: const SizedBox(width: 24, height: 24),
            ),
          ),
        ),
      ],
    );
  }
}

/// Draws the anchor crosshair (a circle with a cross), the egui `anchor_overlay`
/// mark.
class _AnchorPainter extends CustomPainter {
  final Offset centre;
  final LumitTheme theme;
  const _AnchorPainter({required this.centre, required this.theme});

  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()
      ..color = theme.accent
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    const r = 6.0;
    canvas.drawCircle(centre, r, p);
    canvas.drawLine(centre - const Offset(r + 4, 0), centre + const Offset(r + 4, 0), p);
    canvas.drawLine(centre - const Offset(0, r + 4), centre + const Offset(0, r + 4), p);
  }

  @override
  bool shouldRepaint(_AnchorPainter old) =>
      old.centre != centre || old.theme != theme;
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
