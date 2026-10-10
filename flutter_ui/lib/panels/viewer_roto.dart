// The Roto tools over the picture: the scribbles, the edge they produce, and
// the gestures that lay them down (docs/impl/roto.md §6, docs/07 §2.3.7).
//
// **In plain terms.** Cutting a moving thing out of a shot means saying, for
// every frame, which pixels are the subject. The Roto brush shortens that to a
// scribble: drag through the subject and it is claimed as foreground; hold
// `Alt` and drag through what is behind it and that is claimed as background.
// The engine works the edge out from the colours between the two, and
// **Propagate** — on the effect's own card — carries it through the rest of the
// shot. Where it goes wrong on some later frame, scribble there and it is put
// right from that frame onward. **Refine edge** paints the band where the edge
// is allowed to be soft: hair, motion blur, smoke.
//
// **A stroke is stored in the file's own pixels, not the composition's**. That
// is the one piece of arithmetic here that matters: the pointer arrives in
// panel coordinates, and it is carried back through the Viewer's fit, the
// layer's position, anchor, scale and rotation — `LayerBox.map` — to land on
// the picture as the file holds it. So a matte survives every transform, every
// retime and every preview tier, and one shot's mattes serve every composition
// that cuts it. **Which** frame of the file is on screen is the engine's answer
// too ([rotoSourceFrame]), because the layer's start offset and its Retime map
// both live in the document.
//
// **The overlay asks once per frame, never per rebuild**. Three things move
// what is drawn — the frame, the document, and a propagation landing — and the
// read is held against all three, so a pointer travelling across the picture
// crosses the bridge exactly zero times.
//
// **One drag is one stroke is one undo step.** The scribble is drawn on the
// overlay while the pointer is down and committed once on release, through the
// ordinary whole-stack effect commit. `Escape` abandons a scribble in flight.
// A first scribble on a layer with no Roto brush **brings the brush with it**:
// the stroke rides inside the new instance, so effect and stroke land as one op
// and one undo step.
//
// **In a node graph the strokes belong to a box.** A node graph has no layers,
// so there is no selected layer to scribble on. The brush is the Roto brush
// box picked on the canvas, the file is the footage wired into it, and a
// scribble is carried onto that file through where its Read laid it in the
// graph's frame: centred, at its own size. Everything else is the same gesture,
// committed through the graph's own op.
//
// **A tap is a tap or a prompt, and the effect's own Seed from row decides
// which.** On Strokes it is the dab it has always been. On Segment, and on the
// frame the shot is decided from, it is a point handed to a segmentation model
// instead: tap the subject, hold `Alt` and tap what is behind it, and the model
// proposes the cut-out that the scribbles then correct. A drag is a scribble
// either way.
//
// **Release shows the frame it touched.** Committing a stroke asks the engine
// to solve that one frame's matte now — the same job Propagate runs, stopped
// at the scribbled frame — and a small poll waits for it to land so the
// picture refreshes the moment it does. Propagate stays the road to the rest
// of the shot.

import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' show PointMode;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/roto.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'viewer_gizmo.dart';
import 'viewer_layer_map.dart';
import 'status_poller.dart' show statusPoll;
import 'viewer_paint.dart' show strokePaint, strokePath, thinStroke;
import 'viewer_tool_cursor.dart';

/// The Roto brush's `view` choice, by index — the order
/// `roto_brush::VIEW_OPTIONS` declares. Only **Boundary** concerns this file:
/// Result and Matte are pictures the stack draws, and Boundary is the one that
/// keeps the picture and asks the overlay to draw the edge over it.
const int rotoViewBoundary = 2;

/// The Roto brush's `seed` choice, by index, which is what
/// `roto_brush::SEED_SEGMENT` names. **Strokes** is 0 and is what every
/// project reads on a machine with no addon; **Segment** hands the base frame
/// and the taps made on it to a segmentation model, which is what turns a tap
/// from a dab into a prompt.
const int rotoSeedSegment = 1;

/// Which kind of stroke a gesture lays down: the tool in hand, and whether
/// `Alt` is held.
///
/// `Alt` for background is After Effects' convention and the reason it is a
/// modifier rather than a second tool — the two claims are made in one
/// continuous act of scribbling, and reaching for a different tool between them
/// would break the rhythm the whole loop depends on.
BridgeRotoStrokeKind rotoKindFor(ToolMode tool, {required bool alt}) {
  if (tool == ToolMode.refineEdge) return BridgeRotoStrokeKind.refine;
  return alt
      ? BridgeRotoStrokeKind.background
      : BridgeRotoStrokeKind.foreground;
}

/// What each kind of stroke is drawn in — all three from the theme struct, as
/// every colour in the application is (15-DESIGN §1: a hex literal in widget
/// code is a defect).
///
/// The subject in the success role and the background in the error one, which
/// is the green/red pair every rotoscoping tool has used since the technique
/// had a name — here they say "kept" and "cut" rather than "good" and "bad",
/// which is the same distinction those two roles already carry. Refine takes the
/// accent: it is neither claim, it is the band where the answer is allowed to be
/// soft.
Color rotoStrokeColour(BridgeRotoStrokeKind kind, LumitTheme t) =>
    switch (kind) {
      BridgeRotoStrokeKind.foreground => t.success,
      BridgeRotoStrokeKind.background => t.error,
      BridgeRotoStrokeKind.refine => t.accent,
    };

/// The Roto brush the selected layer carries, as the overlay needs it.
///
/// Built from the read model by the Viewer, so finding it costs no
/// bridge call: which instance to write to, which picture it is drawing, and
/// what its base frame is seeded from.
typedef RotoTarget = ({UuidValue effect, int view, int seed});

/// What a Roto tool writes on: a footage layer's stack, or a Roto brush box in
/// a node graph. Everything the gesture asks of either is here, so the code
/// that lays a stroke down reads the same for both.
class _Surface {
  final LayerReference? layer;
  final CompositionReference? graph;

  /// Screen to the file's own pixels and back.
  final ViewerLayerMap map;

  const _Surface.layer(LayerReference this.layer, this.map) : graph = null;
  const _Surface.graph(CompositionReference this.graph, this.map)
      : layer = null;

  /// The instances a stroke is staged on, freshly read.
  List<BridgeEffectInstance> instances() =>
      graph?.getNodeGraphInstances() ?? layer!.getEffects();

  /// One commit, one undo step: the stack's own op, or the graph's.
  void commit(List<BridgeEffectInstance> staged) {
    final graph = this.graph;
    if (graph == null) {
      layer!.setEffects(effects: staged);
    } else {
      graph.setNodeGraph(
          instances: staged, wiring: graph.getNodeGraph().wiring);
    }
  }

  BridgeRotoStatus status(UuidValue effect) =>
      graph?.graphRotoStatus(effect: effect) ??
      rotoStatus(layer: layer!, effect: effect);
}

/// The Roto tools over the picture.
class ViewerRotoLayer extends StatefulWidget {
  /// Whether a Roto tool is armed. Inert otherwise.
  final bool active;

  final ToolMode tool;
  final LumitState state;
  final LumitUiState uiState;

  /// Every layer with its box, top first — for the layer being scribbled on and
  /// the map that carries the pointer into the file's own pixels.
  final List<LayerBox> boxes;

  /// The Roto brush on the selected layer, or null when it has none. Found by
  /// the Viewer from the read model it already holds.
  final RotoTarget? target;

  /// The node graph on screen, when the fronted composition is one, and the
  /// box picked on its canvas. A graph has no layers, so [boxes] is empty and
  /// [target] is null there: the brush is the picked box, found by this
  /// overlay's own read.
  final CompositionReference? graph;
  final UuidValue? graphBox;

  /// The fitted picture's top-left and the composition's size in pixels: what
  /// carries a pointer into a node graph's frame, which a layer's own map
  /// already holds for a layer.
  final Offset origin;
  final Size compSize;

  /// The picture's magnification, so a scribble width in source pixels draws
  /// the ring it would really leave.
  final double viewScale;

  /// The frame on screen, the document's revision, and a counter bumped when a
  /// propagation lands — the three things that move what is drawn, and the only
  /// three that make it worth asking again.
  final int playheadFrame;
  final BigInt? revision;
  final int generation;

  final VoidCallback onChanged;

  /// Where the two engine answers come from. Null is the engine itself, which
  /// is the only thing that ever runs in the application; a test hands one in,
  /// which is the seam `ViewerTrackLayer.fetch` already is — a propagated matte
  /// cannot be produced from Dart, and mounting an engine that could not have
  /// one would assert nothing.
  final int Function(LayerReference layer, int frame)? sourceFrameOf;
  final Float32List Function(UuidValue effect, int frame)? boundaryOf;

  /// Where the release-time solve of the scribbled frame is asked for. Null is
  /// the engine; a test hands one in — solving a real frame needs a real file
  /// and a minute of machinery, and what this side owes is only that the ask is
  /// made, with the right frame, on release.
  final bool Function(LayerReference layer, UuidValue effect, int frame)?
      solveFrameOf;

  const ViewerRotoLayer({
    super.key,
    required this.active,
    required this.tool,
    required this.state,
    required this.uiState,
    required this.boxes,
    required this.target,
    this.graph,
    this.graphBox,
    this.origin = Offset.zero,
    this.compSize = Size.zero,
    required this.viewScale,
    required this.playheadFrame,
    required this.revision,
    this.generation = 0,
    required this.onChanged,
    this.sourceFrameOf,
    this.boundaryOf,
    this.solveFrameOf,
  });

  @override
  State<ViewerRotoLayer> createState() => _ViewerRotoLayerState();
}

class _ViewerRotoLayerState extends State<ViewerRotoLayer> {
  /// Where the pointer is, for the ring.
  Offset? _pointer;

  /// The scribble in flight, in screen coordinates.
  final List<Offset> _stroke = [];

  /// Where the press landed. The framework only reports a drag once it has
  /// travelled its slop, and a scribble that began 18 px along is the wrong
  /// scribble.
  Offset? _downAt;

  /// Which frame of the **file** is on screen, or null when the engine cannot
  /// say — a layer that is not footage, or media that will not probe.
  int? _sourceFrame;

  /// Every stroke and every prompt this brush holds, the frame it propagates
  /// outward from, and the propagated matte's edge at this frame. All held
  /// against [_asked], so a rebuild draws them again and asks nothing.
  List<BridgeRotoStroke> _strokes = const [];
  List<BridgeRotoPrompt> _prompts = const [];
  int? _baseFrame;
  Float32List _boundary = Float32List(0);

  /// What the last read was for.
  ({
    int frame,
    BigInt? revision,
    int generation,
    UuidValue? effect,
    int view,
    bool armed,
  })? _asked;

  VoidCallback? _escapeRelease;

  /// The poll waiting for a release-time solve to land, or null.
  Timer? _solveTimer;

  @override
  void initState() {
    super.initState();
    _escapeRelease = EscapeLadder.register(EscapeRung.gesture, _escape);
    WidgetsBinding.instance.addPostFrameCallback((_) => _read());
  }

  @override
  void didUpdateWidget(ViewerRotoLayer old) {
    super.didUpdateWidget(old);
    _read();
  }

  @override
  void dispose() {
    _solveTimer?.cancel();
    _escapeRelease?.call();
    _escapeRelease = null;
    super.dispose();
  }

  /// Escape abandons the scribble being drawn — the ladder's gesture rung, so
  /// it is taken back before a menu closes.
  bool _escape() {
    if (!widget.active || _stroke.isEmpty) return false;
    setState(_stroke.clear);
    return true;
  }

  /// The layer being scribbled on: the primary selection, as every other
  /// drawing tool uses.
  LayerBox? get _target =>
      primarySelectedBox(widget.boxes, widget.uiState.selectedLayerIds);

  /// The picked box's brush in a node graph, and the size of the file wired
  /// into it. Both from [_read], and both null until a Roto brush box with
  /// footage behind it is picked.
  RotoTarget? _graphBrush;
  Size? _graphFile;

  /// Whether footage is wired into the picked brush at all, which is what
  /// tells an unwired box from one whose file will not read.
  bool _graphWired = false;

  /// The brush in hand: the selected layer's, or the picked box's.
  RotoTarget? get _brush => widget.graph == null ? widget.target : _graphBrush;

  /// What a stroke would be written on, or null when there is nothing to
  /// stroke: no layer selected, or in a node graph no brush box picked with
  /// footage wired into it.
  _Surface? get _surface {
    final graph = widget.graph;
    if (graph == null) {
      final box = _target;
      return box == null ? null : _Surface.layer(box.layer, box.map);
    }
    final file = _graphFile;
    if (_graphBrush == null || file == null) return null;
    // Where the Read laid the file in the graph's frame: centred, at its own
    // size, so the file's middle sits on the frame's middle.
    return _Surface.graph(
      graph,
      ViewerLayerMap(
        px: widget.compSize.width / 2,
        py: widget.compSize.height / 2,
        ax: file.width / 2,
        ay: file.height / 2,
        sx: 1,
        sy: 1,
        sin: 0,
        cos: 1,
        origin: widget.origin,
        viewScale: widget.viewScale,
      ),
    );
  }

  /// Why there is nothing to stroke on, in the words for where the user is.
  String get _noSurface {
    if (widget.graph == null) return l10n.rotoSelectALayer;
    if (_graphBrush == null) return l10n.rotoPickABox;
    return _graphWired ? l10n.planeFailedUnreadable : l10n.graphBoxNoFootage;
  }

  /// The three engine answers the overlay draws from, asked for once per frame,
  /// per document revision and per propagation landing — never per rebuild
  /// (`bridge_call_budget_test` is the gate).
  void _read() {
    if (!mounted) return;
    final graph = widget.graph;
    final box = widget.active && graph == null ? _target : null;
    final brush = widget.target;
    final next = (
      frame: widget.playheadFrame,
      revision: widget.revision,
      generation: widget.generation,
      effect: graph == null ? brush?.effect : widget.graphBox,
      view: brush?.view ?? 0,
      // A node graph's brush is found by the read itself, so arming the tool
      // is a reason to read there. A layer's arrives from the Viewer.
      armed: graph != null && widget.active,
    );
    if (_asked == next) return;
    _asked = next;
    if (graph != null) {
      _readBox(graph, widget.active ? widget.graphBox : null);
      return;
    }
    if (box == null) {
      if (_strokes.isNotEmpty ||
          _prompts.isNotEmpty ||
          _boundary.isNotEmpty ||
          _sourceFrame != null) {
        setState(() {
          _strokes = const [];
          _prompts = const [];
          _baseFrame = null;
          _boundary = Float32List(0);
          _sourceFrame = null;
        });
      }
      return;
    }
    int? frame;
    var strokes = const <BridgeRotoStroke>[];
    var prompts = const <BridgeRotoPrompt>[];
    int? base;
    var boundary = Float32List(0);
    try {
      // Asked whether or not the layer carries a brush yet: the first scribble
      // on a bare layer is the one that *adds* it, and it needs the
      // file's frame as much as any correction does.
      frame = (widget.sourceFrameOf ?? _sourceFrameFromEngine)(
          box.layer, widget.playheadFrame);
      if (brush != null) {
        // One walk of the stack, three readings off the instance it finds: the
        // strokes and the prompts the overlay draws, and the base frame, which
        // is what says whether a tap here may seed the model at all.
        final instance = box.layer
            .getEffects()
            .where((e) => e.id() == brush.effect)
            .firstOrNull;
        if (instance != null) {
          strokes = instance.rotoStrokes();
          prompts = instance.rotoPrompts();
          base = instance.rotoBaseFrame();
        }
        // Only in the Boundary view: the edge is a scan of the whole matte,
        // and running it for a picture nobody is showing would be work for
        // nothing.
        if (brush.view == rotoViewBoundary) {
          boundary = (widget.boundaryOf ?? _boundaryFromEngine)(
              brush.effect, frame);
        }
      }
    } catch (_) {
      // The layer went away under the overlay, or its media will not probe.
      // Nothing is drawn, and nothing about that is a fault.
    }
    if (!mounted) return;
    setState(() {
      _sourceFrame = frame;
      _strokes = strokes;
      _prompts = prompts;
      _baseFrame = base;
      _boundary = boundary;
    });
  }

  /// [_read] for a node graph: the picked box's brush, the file wired into it
  /// and which frame of that file is on screen, then the same three readings
  /// the overlay draws. One walk of the graph's boxes per frame and per
  /// document revision, as the layer's read is one walk of its stack.
  void _readBox(CompositionReference graph, UuidValue? picked) {
    // Nothing picked and nothing held: a frame going by with the tool put
    // away asks nothing and redraws nothing.
    if (picked == null &&
        _graphBrush == null &&
        _sourceFrame == null &&
        _strokes.isEmpty &&
        _prompts.isEmpty &&
        _boundary.isEmpty) {
      return;
    }
    RotoTarget? brush;
    Size? file;
    var wired = false;
    int? frame;
    var strokes = const <BridgeRotoStroke>[];
    var prompts = const <BridgeRotoPrompt>[];
    int? base;
    var boundary = Float32List(0);
    try {
      final instance = picked == null
          ? null
          : graph
              .getNodeGraphInstances()
              .where((e) => e.id() == picked)
              .firstOrNull;
      final info = instance?.getInfo();
      if (instance != null && info != null && info.name == 'roto_brush') {
        var view = 0;
        var seed = 0;
        for (final v in info.values) {
          if (v.value case BridgeEffectValue_Choice(:final field0)) {
            if (v.id == 'view') view = field0;
            if (v.id == 'seed') seed = field0;
          }
        }
        brush = (effect: info.id, view: view, seed: seed);
        strokes = instance.rotoStrokes();
        prompts = instance.rotoPrompts();
        base = instance.rotoBaseFrame();
        // Refused where no footage is wired into the box: the brush is still
        // the brush, with no file yet to stroke on.
        final source =
            graph.graphBoxSource(effect: info.id, frame: widget.playheadFrame);
        wired = true;
        frame = source.frame;
        if (frame != null) {
          file = Size(source.width.toDouble(), source.height.toDouble());
          if (view == rotoViewBoundary) {
            boundary =
                (widget.boundaryOf ?? _boundaryFromEngine)(info.id, frame);
          }
        }
      }
    } catch (_) {
      // The box went away under the overlay, or it has no footage behind it.
      // What was read before the refusal stands.
    }
    if (!mounted) return;
    setState(() {
      _graphBrush = brush;
      _graphFile = file;
      _graphWired = wired;
      _sourceFrame = frame;
      _strokes = strokes;
      _prompts = prompts;
      _baseFrame = base;
      _boundary = boundary;
    });
  }

  /// Whether a tap on the picture seeds the model instead of laying a one-point
  /// stroke (docs/impl/addons.md §6.2).
  ///
  /// Three things have to hold. The seed row has to say **Segment**, because a
  /// brush seeded by its strokes has nothing to read the taps with. The tool in
  /// hand has to be the brush: Refine edge paints the band where the edge may
  /// be soft, which is not a claim about what the subject is. And the frame on
  /// screen has to be the base frame, or there has to be no base yet, because
  /// that is the only frame the model is asked about. A tap anywhere else is
  /// the correction dab it has always been.
  bool get _tapSeedsTheModel {
    final brush = _brush;
    if (brush == null || brush.seed != rotoSeedSegment) return false;
    if (widget.tool != ToolMode.rotoBrush) return false;
    final base = _baseFrame;
    return base == null || base == _sourceFrame;
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    final map = _surface?.map;
    final alt = HardwareKeyboard.instance.isAltPressed;
    return Positioned.fill(
      // The hardware crosshair leads: the OS moves it at input rate
      // whatever the application's frame rate is doing, so it is the thing to
      // aim with. The ring below is decoration — the size of the claim, drawn
      // by the app and honestly a frame behind.
      child: DrawnPointerRegion(
        cursor: SystemMouseCursors.precise,
        onPointer: (at) => setState(() => _pointer = at),
        child: Listener(
          onPointerDown: (event) => _downAt = event.localPosition,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapUp: (d) => _tapSeedsTheModel
                ? _prompt(d.localPosition)
                : _commit([d.localPosition]),
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: (_) => _onPanEnd(),
            onPanCancel: () => setState(_stroke.clear),
            child: Stack(
              children: [
                Positioned.fill(
                  child: CustomPaint(
                    painter: RotoOverlayPainter(
                      map: map,
                      strokes: _strokes,
                      prompts: _prompts,
                      sourceFrame: _sourceFrame,
                      boundary: _boundary,
                      foreground: t.success,
                      background: t.error,
                      refine: t.accent,
                      edge: t.marker,
                      outline: t.surface0,
                    ),
                  ),
                ),
                // The scribble in flight on its own layer above the edge: a
                // drag adds a point per pointer move, and the boundary below is
                // up to twelve thousand points that have to be mapped one by one
                // to be drawn. Keeping the two apart means a drag redraws only
                // the scribble.
                Positioned.fill(
                  child: CustomPaint(
                    painter: _ScribblePainter(
                      points: _stroke,
                      colour: rotoStrokeColour(
                          rotoKindFor(widget.tool, alt: alt), t),
                      width: widget.uiState.tools.rotoSize * widget.viewScale,
                    ),
                  ),
                ),
                ToolPointer(
                  at: _pointer,
                  tool: widget.tool,
                  mark: t.textPrimary,
                  outline: t.surface0,
                  ringRadius: brushRingRadius(
                      widget.uiState.tools.rotoSize, widget.viewScale),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  // --- The gesture ----------------------------------------------------------

  void _onPanStart(DragStartDetails details) {
    setState(() {
      _stroke
        ..clear()
        ..add(_downAt ?? details.localPosition)
        ..add(details.localPosition);
    });
  }

  void _onPanUpdate(DragUpdateDetails details) =>
      setState(() => _stroke.add(details.localPosition));

  void _onPanEnd() {
    final points = List.of(_stroke);
    setState(_stroke.clear);
    if (points.isNotEmpty) _commit(points);
  }

  /// One scribble, one whole-stack commit, one undo step.
  ///
  /// The points are thinned exactly as a paint stroke's are — two screen pixels,
  /// [thinStroke] — because a stroke is a record of a gesture and not a designed
  /// shape, and a path with a thousand points in it costs the seeding pass for
  /// no extra claim. Pressures are ignored: a roto stroke says *what a region
  /// is*, and how hard the pen was pressed says nothing about that.
  void _commit(List<Offset> screenPoints) {
    final box = _surface;
    if (box == null) {
      widget.state.postNotice(_noSurface);
      return;
    }
    final frame = _sourceFrame;
    if (frame == null) {
      // The engine could not say which frame of the file is on screen: the
      // layer is not footage, or its media will not probe. A stroke filed
      // against a guess would seed a frame nobody looked at.
      widget.state.postNotice(l10n.rotoNoSourceFrame);
      return;
    }
    final brush = _brush;
    if (brush == null && widget.tool == ToolMode.refineEdge) {
      // A refine stroke widens the band around an answer, and a layer with no
      // Roto brush has no answer to widen — said plainly. The brush itself is
      // different: its first scribble below brings the effect with it.
      widget.state.postNotice(l10n.rotoAddTheEffect);
      return;
    }
    // **The conversion.** Screen → the layer's own pixels through the whole
    // comp chain, which for a footage layer at natural size is the file's own
    // raster. Nothing downstream re-reads the comp, which is what makes
    // the matte survive every transform above it.
    final points = <double>[
      for (final p in thinStroke(screenPoints))
        ...() {
          final at = box.map.layerOf(p);
          return [at.dx, at.dy];
        }(),
    ];
    if (points.isEmpty) return;
    final kind = rotoKindFor(widget.tool,
        alt: HardwareKeyboard.instance.isAltPressed);
    // In source pixels, as the points are: the width of the claim has to be
    // in the same ruler as where the claim was made.
    final radius = widget.uiState.tools.rotoSize / 2;
    try {
      final UuidValue effect;
      final layer = box.layer;
      if (brush == null) {
        // A node graph's surface is a picked brush, so only a layer gets here.
        if (layer == null) return;
        // First scribble on a bare layer: the Roto brush and the stroke land
        // in one commit — one op, one undo step — instead of a refusal that
        // read as the tool doing nothing.
        effect = layer.rotoFirstStroke(
          points: Float32List.fromList(points),
          radius: radius,
          kind: kind,
          frame: frame,
        );
      } else {
        final staged = box.instances();
        final instance =
            staged.where((e) => e.id() == brush.effect).firstOrNull;
        if (instance == null) return;
        instance.rotoAddStroke(
          points: Float32List.fromList(points),
          radius: radius,
          kind: kind,
          frame: frame,
        );
        box.commit(staged);
        effect = brush.effect;
      }
      // The strokes moved, so the held copy is stale; the document's revision
      // has moved too, which is what re-reads it.
      widget.onChanged();
      _solveNow(box, effect, frame);
    } catch (_) {
      // The layer or the effect went away mid-scribble, or the stroke had
      // nothing in it after thinning. Neither is worth a dialogue.
    }
  }

  /// One tap, one prompt for the segmentation model (docs/impl/addons.md §6.2).
  ///
  /// The same commit a scribble makes and the same conversion: the tap goes
  /// back through `LayerBox.map` into the file's own pixels, so what the model
  /// is asked about is a place on the picture as the file holds it. `Alt` makes
  /// the tap a negative one, which is the same modifier that claims the
  /// background for a stroke and says the same thing.
  ///
  /// A second tap refines the first rather than proposing a second subject:
  /// every prompt on the base frame is handed to the decoder together.
  void _prompt(Offset screenPoint) {
    final box = _surface;
    if (box == null) {
      widget.state.postNotice(_noSurface);
      return;
    }
    final frame = _sourceFrame;
    if (frame == null) {
      widget.state.postNotice(l10n.rotoNoSourceFrame);
      return;
    }
    final brush = _brush;
    if (brush == null) return;
    final at = box.map.layerOf(screenPoint);
    final subject = !HardwareKeyboard.instance.isAltPressed;
    try {
      final staged = box.instances();
      final instance =
          staged.where((e) => e.id() == brush.effect).firstOrNull;
      if (instance == null) return;
      instance.rotoAddPrompt(
        points: Float32List.fromList([at.dx, at.dy]),
        labels: Uint8List.fromList([subject ? 1 : 0]),
        frame: frame,
      );
      box.commit(staged);
      widget.onChanged();
      _solveNow(box, brush.effect, frame);
    } catch (_) {
      // The layer or the effect went away under the tap, or the point landed
      // somewhere the engine would not read. Neither is worth a dialogue.
    }
  }

  /// Ask for the scribbled frame's own matte, now (docs/impl/roto.md
  /// §6 step 1) — the same job Propagate runs, stopped at this frame.
  ///
  /// Best-effort on the way out of a gesture that already succeeded: a quiet
  /// `false` (another job holding the slot, offline media) leaves the stroke
  /// filed and visible, with Propagate as the press that reports refusals.
  void _solveNow(_Surface on, UuidValue effect, int frame) {
    final bool started;
    try {
      final layer = on.layer;
      started = layer == null
          ? on.graph!.graphRotoSolveFrame(effect: effect, frame: frame)
          : (widget.solveFrameOf ?? _solveFrameFromEngine)(
              layer, effect, frame);
    } catch (_) {
      return;
    }
    if (started) _watchSolve(on, effect);
  }

  /// Wait for the release-time solve to land, then tell the Viewer.
  ///
  /// The same twice-a-second sampling the status cards do ([statusPoll]), for
  /// the same reason: a job landing moves neither the playhead nor the
  /// document's revision, so the picture would stay the one banked before it
  /// unless somebody who knows says so. The effect card says so too when it is
  /// open — but a scribble must not need a panel open to show its result.
  void _watchSolve(_Surface on, UuidValue effect) {
    _solveTimer?.cancel();
    _solveTimer = Timer.periodic(statusPoll, (timer) {
      final BridgeRotoStatus s;
      try {
        s = on.status(effect);
      } catch (_) {
        // The layer or the effect went away under the poll.
        timer.cancel();
        return;
      }
      final moving = s.stage == BridgeRotoStage.queued ||
          s.stage == BridgeRotoStage.solving;
      if (moving) return;
      timer.cancel();
      if (!mounted) return;
      widget.uiState.solveLanded.value++;
      widget.uiState.requestFrame();
      widget.onChanged();
    });
  }
}

/// The engine's own answers — what the application always uses.
int _sourceFrameFromEngine(LayerReference layer, int frame) =>
    rotoSourceFrame(layer: layer, frame: frame);

Float32List _boundaryFromEngine(UuidValue effect, int frame) =>
    rotoBoundary(effect: effect, frame: frame);

bool _solveFrameFromEngine(LayerReference layer, UuidValue effect, int frame) =>
    rotoSolveFrame(layer: layer, effect: effect, frame: frame);

/// The scribble under the pointer, on its own layer so a drag does not redraw
/// the edge underneath it.
class _ScribblePainter extends CustomPainter {
  /// The scribble in screen coordinates, and the colour of the claim it is
  /// about to make.
  final List<Offset> points;
  final Color colour;

  /// How wide it draws, in screen pixels.
  final double width;

  const _ScribblePainter({
    required this.points,
    required this.colour,
    required this.width,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (points.isEmpty) return;
    canvas.drawPath(strokePath(points), strokePaint(colour, width));
  }

  @override
  bool shouldRepaint(_ScribblePainter old) =>
      old.points.length != points.length ||
      old.points.lastOrNull != points.lastOrNull ||
      old.colour != colour ||
      old.width != width;
}

/// How wide a prompt's ring is drawn, in screen pixels. Big enough to find and
/// small enough to see what is under it.
const double _promptRadius = 5;

/// The scribbles and the taps on this frame and, in the Boundary view, the
/// propagated matte's edge.
///
/// Every colour arrives from the theme struct; nothing here chooses one.
class RotoOverlayPainter extends CustomPainter {
  /// The layer↔screen map, or null when nothing is selected.
  final ViewerLayerMap? map;

  /// Every stroke the brush holds, in source pixels, and which frame of the
  /// file is on screen — only that frame's strokes are drawn, because a stroke
  /// is a claim about one frame.
  final List<BridgeRotoStroke> strokes;

  /// Every segmentation prompt the brush holds, on the same terms: taps in
  /// source pixels, and only this frame's are drawn.
  final List<BridgeRotoPrompt> prompts;

  final int? sourceFrame;

  /// The matte's edge at this frame, `[x0, y0, …]` in source pixels. Empty in
  /// every view but Boundary, and outside the propagated span.
  final Float32List boundary;

  final Color foreground;
  final Color background;
  final Color refine;
  final Color edge;
  final Color outline;

  const RotoOverlayPainter({
    required this.map,
    required this.strokes,
    this.prompts = const [],
    required this.sourceFrame,
    required this.boundary,
    required this.foreground,
    required this.background,
    required this.refine,
    required this.edge,
    required this.outline,
  });

  Color _colourOf(BridgeRotoStrokeKind kind) => switch (kind) {
        BridgeRotoStrokeKind.foreground => foreground,
        BridgeRotoStrokeKind.background => background,
        BridgeRotoStrokeKind.refine => refine,
      };

  @override
  void paint(Canvas canvas, Size size) {
    final m = map;
    if (m == null) return;
    _paintBoundary(canvas, m);
    _paintStrokes(canvas, m);
    _paintPrompts(canvas, m);
  }

  /// The taps that seed the model: a small ring each, green for the subject and
  /// red for what is against it, the same two roles a stroke's colours carry,
  /// so a tap and a scribble that make the same claim look alike.
  ///
  /// Rings rather than dots, and at a fixed size on screen rather than in the
  /// file's pixels, because a tap has no width: it names one place and says
  /// which side of the edge it is on, and it has to stay findable however far
  /// the picture is zoomed out.
  void _paintPrompts(Canvas canvas, ViewerLayerMap m) {
    for (final p in prompts) {
      if (p.frame != sourceFrame) continue;
      for (var i = 0; i + 1 < p.points.length; i += 2) {
        final at = m.toScreen(p.points[i], p.points[i + 1]);
        final subject = (i ~/ 2) < p.labels.length && p.labels[i ~/ 2] != 0;
        canvas.drawCircle(
          at,
          _promptRadius,
          Paint()
            ..color = outline
            ..style = PaintingStyle.stroke
            ..strokeWidth = 3,
        );
        canvas.drawCircle(
          at,
          _promptRadius,
          Paint()
            ..color = subject ? foreground : background
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1.5,
        );
      }
    }
  }

  /// The strokes already in the document, at their stored width, faded — they
  /// are what the answer was *made from* and not the answer, so they must not
  /// out-shout the matte edge over the same pixels.
  void _paintStrokes(Canvas canvas, ViewerLayerMap m) {
    for (final s in strokes) {
      if (s.frame != sourceFrame || s.points.length < 2) continue;
      final at = <Offset>[
        for (var i = 0; i + 1 < s.points.length; i += 2)
          m.toScreen(s.points[i], s.points[i + 1]),
      ];
      // The stored radius is in source pixels; on screen it is that through the
      // same map the points went through, measured rather than assumed so a
      // scaled or rotated layer draws the width it will really claim.
      final rim = m.toScreen(s.points[0] + s.radius, s.points[1]);
      canvas.drawPath(
        strokePath(at),
        strokePaint(_colourOf(s.kind).withValues(alpha: 0.45),
            (rim - at.first).distance * 2),
      );
    }
  }

  /// The matte's edge: two passes of points, the halo first, so the outline is
  /// legible over a white subject and a black one alike.
  ///
  /// `drawPoints` rather than a path: the engine sends edge *pixels* rather than
  /// a traced contour, and joining them in scan order would draw a zigzag across
  /// the picture rather than an outline.
  void _paintBoundary(Canvas canvas, ViewerLayerMap m) {
    if (boundary.length < 2) return;
    final at = <Offset>[
      for (var i = 0; i + 1 < boundary.length; i += 2)
        m.toScreen(boundary[i], boundary[i + 1]),
    ];
    canvas.drawPoints(
      PointMode.points,
      at,
      Paint()
        ..color = outline
        ..strokeWidth = 3
        ..strokeCap = StrokeCap.round,
    );
    canvas.drawPoints(
      PointMode.points,
      at,
      Paint()
        ..color = edge
        ..strokeWidth = 1.5
        ..strokeCap = StrokeCap.round,
    );
  }

  @override
  bool shouldRepaint(RotoOverlayPainter old) =>
      old.map != map ||
      !identical(old.strokes, strokes) ||
      !identical(old.prompts, prompts) ||
      old.sourceFrame != sourceFrame ||
      !identical(old.boundary, boundary) ||
      old.foreground != foreground ||
      old.background != background ||
      old.refine != refine ||
      old.edge != edge ||
      old.outline != outline;
}
