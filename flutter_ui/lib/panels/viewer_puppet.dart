// The Puppet tools over the picture: the mesh ghost, the pins, and the gestures
// that place and move them (K-704, docs/impl/puppet.md §5, docs/07 §1.7).
//
// **In plain terms.** With a puppet tool in hand, a thin wireframe appears over
// the **selected layer**: that is the mesh, a net of small triangles laid over
// everything the layer draws. Clicking on it drives a pin in. Dragging a pin
// takes that spot of the picture with it, and the mesh bends to follow — every
// triangle turning and sliding rather than stretching, which is what makes an
// arm bend at the elbow instead of smearing.
//
// Four kinds of pin, one per tool. A **position** pin moves a spot. A **starch**
// pin stiffens the region round it. An **overlap** pin says which part draws in
// front where the picture folds over itself. A **bend** pin turns and scales the
// region round it without travelling — so a hand waves from the wrist.
//
// **The wireframe is the engine's own mesh, not a copy.** No triangle is in the
// document or in a project file; the render builds the mesh from the layer's
// alpha and leaves the one it just used where this can read it
// ([PuppetGhosts]). So the wireframe cannot disagree with the pixels, and a
// hover asks the engine nothing at all (K-681).
//
// **Two refusals, both calm.** A click on a layer with nothing opaque under it,
// and a click outside the mesh, are refused with a line in the status area and
// no dialogue — and no block is made, so nothing has to be undone.
//
// **One gesture is one undo step.** A pin is placed with one op, which for the
// first pin is the block and the pin together; a drag commits once on release,
// through the ordinary property rules, so with the stopwatch on a keyframe
// lands where every other keyframe does.

import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/puppet_ghost.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show puppetPinCopy;
import 'viewer_gizmo.dart';
import 'viewer_layer_map.dart';
import 'viewer_tool_cursor.dart';

/// How near, in screen pixels, the pointer has to be to take hold of a pin.
const double puppetPinGrab = 9;

/// The radius a pin's dot is drawn at.
const double puppetPinRadius = 4;

/// Which kind of pin each puppet tool places.
BridgePuppetPinKind puppetKindFor(ToolMode tool) => switch (tool) {
      ToolMode.puppetStarch => BridgePuppetPinKind.starch,
      ToolMode.puppetOverlap => BridgePuppetPinKind.overlap,
      ToolMode.puppetBend => BridgePuppetPinKind.bend,
      _ => BridgePuppetPinKind.position,
    };

/// What a pin of that kind is called — its default name, and the word its
/// Timeline row is headed with.
String puppetKindLabel(BridgePuppetPinKind kind) => switch (kind) {
      BridgePuppetPinKind.position => l10n.puppetPinPosition,
      BridgePuppetPinKind.starch => l10n.puppetPinStarch,
      BridgePuppetPinKind.overlap => l10n.puppetPinOverlap,
      BridgePuppetPinKind.bend => l10n.puppetPinBend,
    };

/// The still value of a scalar, or null when it is keyed or an expression.
double? _still(BridgeScalar s) => s is BridgeScalar_Static ? s.field0 : null;

/// Where a pin stands at the frame on screen, in layer pixels.
///
/// The read model evaluates every scalar at the playhead, so a *keyed* pin's
/// `x`/`y` still arrive as the numbers it holds now — but a keyed scalar comes
/// across as its curve, so the value has to be read out of it. Falling back to
/// the first key rather than to zero: a pin at the origin would be drawn in the
/// corner of the layer, which reads as a bug rather than as a missing value.
Offset puppetPinAt(BridgePuppetPin pin) => Offset(
      _still(pin.x) ?? _firstKey(pin.x),
      _still(pin.y) ?? _firstKey(pin.y),
    );

double _firstKey(BridgeScalar s) => switch (s) {
      BridgeScalar_Keyframed(:final field0) =>
        field0.isEmpty ? 0 : field0.first.value,
      _ => 0,
    };

/// The puppet tools over the picture.
class ViewerPuppetLayer extends StatefulWidget {
  /// Whether a puppet tool is armed. Inert otherwise — and standing down is
  /// what takes the mesh preview off the render.
  final bool active;

  final ToolMode tool;
  final LumitState state;
  final LumitUiState uiState;
  final CompositionReference comp;

  /// Every layer with its box, top first — for the layer being pinned and the
  /// map that turns the pointer into layer coordinates.
  final List<LayerBox> boxes;

  final VoidCallback onChanged;

  const ViewerPuppetLayer({
    super.key,
    required this.active,
    required this.tool,
    required this.state,
    required this.uiState,
    required this.comp,
    required this.boxes,
    required this.onChanged,
  });

  @override
  State<ViewerPuppetLayer> createState() => _ViewerPuppetLayerState();
}

class _ViewerPuppetLayerState extends State<ViewerPuppetLayer> {
  /// The engine's mesh for the layer being pinned, held against the frame and
  /// the document so a hover asks nothing.
  final PuppetGhosts _ghosts = PuppetGhosts();

  /// Where the pointer is, for the tool's own mark.
  Offset? _pointer;

  /// The pin being dragged, and where the press landed. The framework only
  /// reports a drag once it has travelled its slop, and a drag that began 18 px
  /// along is the wrong drag (K-217's trap, and every tool since).
  UuidValue? _dragging;
  Offset? _downAt;

  /// Where the dragged pin is now, in layer pixels — what the overlay draws
  /// while the pointer is down. The document still holds where it was.
  Offset? _dragTo;

  /// A bend pin's turn and size in flight, when the bend tool is dragging one.
  double? _dragRotation;
  double? _dragScale;

  /// What was last asked of the render's mesh preview, so arming it is a state
  /// change rather than something a rebuild does (K-681: no bridge calls in a
  /// rebuild path).
  ///
  /// It starts at "no layer", which is the truth: this widget is mounted with
  /// every Viewer whether or not a puppet tool is in hand, and standing down a
  /// preview nobody armed would be a bridge call for nothing on every panel that
  /// opens.
  ({UuidValue? layer, double density, double expansion}) _armed =
      (layer: null, density: 0, expansion: 0);

  VoidCallback? _escapeRelease;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_onKey);
    _escapeRelease = EscapeLadder.register(EscapeRung.gesture, _escape);
    widget.uiState.tools.addListener(_onToolsChanged);
    WidgetsBinding.instance.addPostFrameCallback((_) => _syncPreview());
  }

  @override
  void didUpdateWidget(ViewerPuppetLayer old) {
    super.didUpdateWidget(old);
    _syncPreview();
  }

  @override
  void dispose() {
    widget.uiState.tools.removeListener(_onToolsChanged);
    HardwareKeyboard.instance.removeHandler(_onKey);
    _escapeRelease?.call();
    _escapeRelease = null;
    // The preview is one flag in the engine, and leaving it set would keep a
    // layer's mesh being rebuilt for an overlay nobody is drawing.
    if (_armed.layer != null) {
      try {
        disarmPuppetPreview();
      } catch (_) {
        // The engine went away first; nothing to stand down.
      }
    }
    super.dispose();
  }

  /// Tell the render which layer wants a mesh, and at what density.
  ///
  /// This is what makes the **first** pin placeable: until a layer has a puppet
  /// there is no mesh to aim at, and the render builds one only because it was
  /// asked to. Guarded by what was last asked, so the rebuild that follows every
  /// movement of the pointer crosses the bridge exactly zero times.
  void _syncPreview() {
    if (!mounted) return;
    final tools = widget.uiState.tools;
    final box = widget.active ? _target : null;
    final next = box == null
        ? (layer: null, density: 0.0, expansion: 0.0)
        : (
            layer: box.id,
            density: tools.puppetDensity,
            expansion: tools.puppetExpansion,
          );
    if (_armed == next) return;
    _armed = next;
    try {
      if (box == null) {
        disarmPuppetPreview();
      } else {
        box.layer.armPuppetPreview(
          density: tools.puppetDensity,
          expansion: tools.puppetExpansion,
        );
      }
    } catch (_) {
      // No layer, or it went away between the selection and the ask. The
      // overlay simply draws nothing.
    }
  }

  /// A tool option moved, or a tool was picked.
  ///
  /// The document edit lives **here and not in [_syncPreview]**, which
  /// `didUpdateWidget` calls in the middle of the parent's build: an op
  /// committed there would refresh the read model mid-build, which is the one
  /// thing Flutter will not have. A toolbar drag is never in a build.
  void _onToolsChanged() {
    _syncPreview();
    final box = widget.active ? _target : null;
    if (box == null) return;
    // The mesh a puppeted layer already has is built from the numbers on its own
    // block, so the options are only live on it if they are written there — the
    // "re-meshing on commit" half of K-225. One-way: the block takes what the
    // toolbar says, and the toolbar never reads it back.
    try {
      _pushMeshOptions(box, widget.uiState.tools);
    } catch (_) {
      // The layer went away between the drag and the commit.
    }
  }

  void _pushMeshOptions(LayerBox box, ToolsState tools) {
    final puppet = box.puppet;
    if (puppet == null) return;
    if (puppet.density == tools.puppetDensity &&
        puppet.expansion == tools.puppetExpansion) {
      return;
    }
    box.layer.setPuppet(
      puppet: BridgePuppet(
        referenceTime: puppet.referenceTime,
        density: tools.puppetDensity,
        expansion: tools.puppetExpansion,
        pins: puppet.pins,
      ),
    );
    widget.onChanged();
  }

  /// The layer being pinned: the primary selection, as every other drawing tool
  /// uses.
  LayerBox? get _target =>
      primarySelectedBox(widget.boxes, widget.uiState.selectedLayerIds);

  /// Escape abandons the drag in flight — the ladder's gesture rung, so it is
  /// taken back before a menu closes.
  bool _escape() {
    if (!widget.active || _dragging == null) return false;
    setState(_clearDrag);
    return true;
  }

  void _clearDrag() {
    _dragging = null;
    _dragTo = null;
    _dragRotation = null;
    _dragScale = null;
  }

  /// `Delete` removes the pin under the cursor — the note's own rule, and the
  /// reason it is the pointer rather than a selection that decides: a pin is
  /// aimed at on the picture, and there is nowhere else to aim from.
  bool _onKey(KeyEvent event) {
    if (!widget.active || event is! KeyDownEvent) return false;
    if (event.logicalKey != LogicalKeyboardKey.delete) return false;
    final box = _target;
    final at = _pointer;
    if (box == null || at == null) return false;
    final pin = _pinNear(box, at);
    if (pin == null) return false;
    try {
      box.layer.deletePuppetPin(id: pin.id);
      widget.onChanged();
      return true;
    } catch (_) {
      return false;
    }
  }

  /// The pin whose dot is under [screen], nearest first.
  BridgePuppetPin? _pinNear(LayerBox box, Offset screen) {
    BridgePuppetPin? best;
    var nearest = puppetPinGrab;
    for (final pin in box.puppet?.pins ?? const <BridgePuppetPin>[]) {
      final at = puppetPinAt(pin);
      final d = (box.map.toScreen(at.dx, at.dy) - screen).distance;
      if (d <= nearest) {
        nearest = d;
        best = pin;
      }
    }
    return best;
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    final box = _target;
    return Positioned.fill(
      // The hardware crosshair leads (K-724): a pin is driven into a pixel,
      // and the OS pointer is the one that moves at input rate.
      child: DrawnPointerRegion(
        cursor: SystemMouseCursors.precise,
        onPointer: (at) => setState(() => _pointer = at),
        child: Listener(
          onPointerDown: (event) => _downAt = event.localPosition,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapUp: _onTapUp,
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: (_) => _onPanEnd(),
            onPanCancel: () => setState(_clearDrag),
            child: Stack(
              children: [
                // **Under a listener of its own**, as the solved point cloud
                // is (K-430). The mesh moves when a *frame* lands, and a frame
                // landing moves neither the document's revision nor the
                // playhead — so without this the wireframe would hold the pose
                // it had at the last edit and only catch up when something
                // else happened to rebuild the panel.
                Positioned.fill(
                  child: ListenableBuilder(
                    listenable: widget.uiState.frameArrived,
                    builder: (context, _) {
                      // Held against the layer, the frame the engine last
                      // delivered and the document's revision — the three
                      // things that can move the mesh — so this costs a
                      // comparison on a hover and a call on none of them
                      // (K-184, K-681).
                      _ghosts.refresh(
                        layer: box?.layer,
                        generation: widget.uiState.frameArrived.value,
                        revision: widget.uiState.model.heldRevision,
                      );
                      return CustomPaint(
                        painter: PuppetOverlayPainter(
                          ghost: _ghosts.ghost,
                          map: box?.map,
                          pins: box?.puppet?.pins ?? const [],
                          dragging: _dragging,
                          dragTo: _dragTo,
                          mesh: t.hairline,
                          pin: t.accent,
                          outline: t.surface0,
                          mark: t.textPrimary,
                        ),
                      );
                    },
                  ),
                ),
                ToolPointer(
                  at: _pointer,
                  tool: widget.tool,
                  mark: t.textPrimary,
                  outline: t.surface0,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  // --- The gestures ---------------------------------------------------------

  /// A click drives a pin of the armed tool's kind in.
  ///
  /// The engine decides whether it may: it looks the point up in the *deformed*
  /// mesh — the one the user aimed at — and carries it back to where that spot
  /// sits at rest, which is where a pin is stored. A click with no mesh under
  /// it, or outside the one there is, comes back as a refusal, and the status
  /// line says which.
  void _onTapUp(TapUpDetails details) {
    final box = _target;
    if (box == null) {
      widget.state.postNotice(l10n.selectALayerToPin);
      return;
    }
    // A click on a pin takes hold of it rather than stacking a second one on
    // top: two pins at the same spot fight each other in the solve for ever.
    if (_pinNear(box, details.localPosition) != null) return;
    final at = box.map.layerOf(details.localPosition);
    final kind = puppetKindFor(widget.tool);
    final number = (box.puppet?.pins.length ?? 0) + 1;
    try {
      box.layer.addPuppetPinAt(
        frame: widget.uiState.playheadFrame.value,
        kind: kind,
        name: '${puppetKindLabel(kind)} $number',
        x: at.dx,
        y: at.dy,
      );
      widget.onChanged();
    } catch (_) {
      // The refusals of §6, in the reader's own language. Which of the two it
      // is, is read off the wireframe rather than off the error: a
      // `BridgeError` reaches Dart as an opaque handle with nothing readable on
      // it, and the engine's two answers — no mesh at all, and a point outside
      // the one there is — are exactly "is there a wireframe" and "there is".
      // The overlay is holding that answer already, so nothing is asked and
      // nothing is scraped (K-303).
      //
      // ponytail: the held wireframe is a frame behind the engine's, so on the
      // one frame where the layer's alpha has just emptied the message can be
      // the wrong one of two calm sentences. The upgrade is a typed placement
      // result across the seam. Observable trigger: a refusal naming the mesh
      // on a layer that visibly has none.
      widget.state.postNotice(
        _ghosts.ghost == null ? l10n.puppetNoMesh : l10n.puppetOutsideMesh,
      );
    }
  }

  void _onPanStart(DragStartDetails details) {
    final box = _target;
    if (box == null) return;
    final from = _downAt ?? details.localPosition;
    final pin = _pinNear(box, from);
    if (pin == null) return;
    setState(() {
      _dragging = pin.id;
      _dragTo = puppetPinAt(pin);
      _dragRotation = _still(pin.rotation);
      _dragScale = _still(pin.scale);
    });
  }

  void _onPanUpdate(DragUpdateDetails details) {
    final box = _target;
    final id = _dragging;
    if (box == null || id == null) return;
    final pin = box.puppet?.pins.where((p) => p.id == id).firstOrNull;
    if (pin == null) return;
    final centre = puppetPinAt(pin);
    // The bend tool on a bend pin turns the region round the pin instead of
    // moving it: the angle is the pointer's about the pin, and `Alt` swaps the
    // turn for a size (docs/impl/puppet.md §5).
    if (widget.tool == ToolMode.puppetBend &&
        pin.kind == BridgePuppetPinKind.bend) {
      final at = box.map.layerOf(details.localPosition);
      final was = box.map.layerOf(details.localPosition - details.delta);
      if (HardwareKeyboard.instance.isAltPressed) {
        final wasR = (was - centre).distance;
        final nowR = (at - centre).distance;
        if (wasR > 1e-6) {
          setState(() => _dragScale =
              ((_dragScale ?? 100) * nowR / wasR).clamp(1.0, 1000.0));
        }
        return;
      }
      final turn = (math.atan2(at.dy - centre.dy, at.dx - centre.dx) -
              math.atan2(was.dy - centre.dy, was.dx - centre.dx)) *
          180 /
          math.pi;
      setState(() => _dragRotation = (_dragRotation ?? 0) + turn);
      return;
    }
    setState(() => _dragTo = box.map.layerOf(details.localPosition));
  }

  /// One drag, one op, one undo step.
  ///
  /// ponytail: the picture does not deform until the pointer is released — the
  /// wireframe and the dot follow it, the pixels catch up on the commit. The
  /// upgrade is a puppet-preview render call beside the transform one the gizmo
  /// drags through, which is a bridge surface and a solve per throttled tick.
  /// Observable trigger: pin drags reading as blind on a real character.
  void _onPanEnd() {
    final box = _target;
    final id = _dragging;
    final to = _dragTo;
    final rotation = _dragRotation;
    final scale = _dragScale;
    setState(_clearDrag);
    if (box == null || id == null) return;
    final puppet = box.puppet;
    if (puppet == null) return;
    final was = puppet.pins.where((p) => p.id == id).firstOrNull;
    if (was == null) return;
    final frame = widget.uiState.playheadFrame.value;
    // Through `scalarWithValueAt`, so a pin with its stopwatch on lands a
    // keyframe exactly where a mask's opacity would, and a still one just takes
    // the new number (docs/07 §4.3).
    BridgeScalar at(BridgeScalar s, double v) =>
        scalarWithValueAt(s, v, widget.comp, frame);
    final next = puppetPinCopy(
      was,
      x: to == null ? null : at(was.x, to.dx),
      y: to == null ? null : at(was.y, to.dy),
      rotation: rotation == null ? null : at(was.rotation, rotation),
      scale: scale == null ? null : at(was.scale, scale),
    );
    if (next == was) return;
    try {
      box.layer.setPuppetPin(pin: next);
      widget.onChanged();
    } catch (_) {
      // The pin went away mid-drag. Not worth a dialogue.
    }
  }
}

/// The mesh ghost and the pins.
///
/// Everything is drawn from the theme (15-DESIGN: a hex literal in widget code
/// is a defect). The wireframe is one path rather than one line per edge: at the
/// engine's vertex cap that is nearly three thousand triangles, and nine
/// thousand `drawLine` calls a frame is a redraw budget spent on scaffolding.
class PuppetOverlayPainter extends CustomPainter {
  /// The engine's mesh at this frame, or null when there is none to draw.
  final BridgePuppetGhost? ghost;

  /// The layer↔screen map, or null when nothing is selected.
  final ViewerLayerMap? map;

  final List<BridgePuppetPin> pins;

  /// The pin under the pointer, and where it has been dragged to in layer
  /// pixels — drawn there rather than where the document still has it.
  final UuidValue? dragging;
  final Offset? dragTo;

  final Color mesh;
  final Color pin;
  final Color outline;
  final Color mark;

  const PuppetOverlayPainter({
    required this.ghost,
    required this.map,
    required this.pins,
    required this.dragging,
    required this.dragTo,
    required this.mesh,
    required this.pin,
    required this.outline,
    required this.mark,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final m = map;
    if (m == null) return;
    _paintMesh(canvas, m);
    for (final p in pins) {
      final held = p.id == dragging;
      final at = held ? (dragTo ?? puppetPinAt(p)) : puppetPinAt(p);
      final screen = m.toScreen(at.dx, at.dy);
      // The reach of a starch, overlap or bend pin, while it is being dragged
      // (docs/impl/puppet.md §5): a faint circle in the layer's own pixels, so
      // it turns and scales with the layer as the mesh does.
      if (held && p.kind != BridgePuppetPinKind.position) {
        final edge = m.toScreen(at.dx + p.extent, at.dy);
        canvas.drawCircle(
          screen,
          (edge - screen).distance,
          Paint()
            ..color = mesh
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1,
        );
      }
      _paintPin(canvas, screen, p.kind,
          inert: ghost?.inert.contains(p.id) ?? false);
    }
  }

  void _paintMesh(Canvas canvas, ViewerLayerMap m) {
    final g = ghost;
    if (g == null || g.triangles.isEmpty) return;
    final path = Path();
    for (var i = 0; i + 2 < g.triangles.length; i += 3) {
      Offset? corner(int slot) {
        final v = g.triangles[i + slot] * 2;
        if (v + 1 >= g.vertices.length) return null;
        return m.toScreen(g.vertices[v], g.vertices[v + 1]);
      }

      final a = corner(0);
      final b = corner(1);
      final c = corner(2);
      if (a == null || b == null || c == null) continue;
      path
        ..moveTo(a.dx, a.dy)
        ..lineTo(b.dx, b.dy)
        ..lineTo(c.dx, c.dy)
        ..close();
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = mesh
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  /// One pin: a filled dot, and the mark that says which kind it is.
  ///
  /// An **inert** pin — one whose rest position fell outside the mesh after it
  /// was rebuilt — is drawn hollow (§6). It is still in the document and comes
  /// back by itself if the mesh grows back, so it is shown rather than hidden.
  void _paintPin(Canvas canvas, Offset at, BridgePuppetPinKind kind,
      {required bool inert}) {
    final body = Paint()
      ..color = pin
      ..style = inert ? PaintingStyle.stroke : PaintingStyle.fill
      ..strokeWidth = 1.5;
    canvas.drawCircle(
      at,
      puppetPinRadius,
      Paint()
        ..color = outline
        ..style = PaintingStyle.stroke
        ..strokeWidth = 3,
    );
    canvas.drawCircle(at, puppetPinRadius, body);
    if (kind == BridgePuppetPinKind.position) return;
    // The three seasoning kinds each carry one extra mark, in the theme's ink
    // so it reads over the dot: starch a square (a region held rigid), overlap
    // a bar across (one side in front of the other), bend an arm (the turn it
    // makes).
    final ink = Paint()
      ..color = mark
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.25;
    const r = puppetPinRadius + 3;
    switch (kind) {
      case BridgePuppetPinKind.starch:
        canvas.drawRect(Rect.fromCircle(center: at, radius: r), ink);
      case BridgePuppetPinKind.overlap:
        canvas.drawLine(at + const Offset(-r, 0), at + const Offset(r, 0), ink);
      case BridgePuppetPinKind.bend:
        canvas.drawLine(at, at + const Offset(r, -r), ink);
      case BridgePuppetPinKind.position:
        break;
    }
  }

  @override
  bool shouldRepaint(PuppetOverlayPainter old) =>
      !identical(old.ghost, ghost) ||
      old.map != map ||
      old.pins != pins ||
      old.dragging != dragging ||
      old.dragTo != dragTo ||
      old.mesh != mesh ||
      old.pin != pin;
}
