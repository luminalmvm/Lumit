// The Viewer, on the flutter_rust_bridge API.
//
// A toolbar under the picture: magnification, channel view, the transparency
// grid, the transport and the timecode. Sharp welds it to the panel's bottom
// edge; Round parts it from the picture by a tile gap and draws it as a tile
// of its own (K-394). The picture itself is whatever the
// render worker last published — always a platform `Texture` on a zero-copy
// path (K-183) — drawn at the chosen zoom over a checkerboard, pannable when
// it is larger than the panel.
//
// **What the overlay does.** The selected layer gets a bounding box with a
// centre handle. Dragging the handle moves the layer: the drag previews through
// `renderFrameWithTransformPreview`, which patches a clone of the document
// engine-side, and commits one `set_transform` pair on release. So dragging in
// the Viewer costs the same one undo step that dragging the number in Effect
// controls does.
//
// **What the transport does, and does not, do.** It says play, stop and seek,
// and it draws whichever frame last arrived. It runs no clock and schedules
// nothing: the engine chooses frames, paces them against the audio, and stops
// itself at the end, and every frame it publishes says which frame it is — so
// the playhead follows the picture rather than predicting it (K-181). This panel
// used to hold a `Ticker` polling the audio clock, an every-frame pump two deep,
// an in-flight counter and a staleness flag, which is a scheduler sitting on the
// far side of an FFI boundary from everything it had to schedule against.
//
// **What is not here.** The scale and rotate gizmo handles, motion paths and
// masks; rulers and draggable guides, and the wireframe/overlay menu. Recorded
// in docs/TODO.md — none is blocked on the engine. The grid and the safe areas
// *are* here (K-416, `viewer_overlays.dart`), as is the region of interest.

import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';
// Aliased, and not as `ui`: half this file already calls the session state
// `ui`, and a local of that name would shadow the prefix where it is needed.
import 'dart:ui' as dartui;

import 'package:flutter/gestures.dart';
// For [RenderRepaintBoundary], which is what a snapshot is photographed from.
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/tool_bar_frb.dart';
import '../state/dropper.dart';
import '../state/layer_bounds.dart' show shapeContentsRect, textLayerBounds;
import '../state/preview_throttle.dart';
import '../state/settings.dart';
import '../state/tools.dart';
import '../state/timecode.dart';
import '../state/viewer_view.dart';
import '../state/workspace.dart' show ViewerLook;
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import '../widgets/dropper_overlay.dart';
import '../widgets/time_readout.dart';
import 'placeholder.dart';
import 'viewer_anchor.dart';
import 'viewer_gizmo.dart';
import 'viewer_layer_map.dart';
import 'viewer_overlays.dart';
import 'viewer_rotate.dart';
import 'viewer_shape_layer.dart';
import 'viewer_shapes.dart';
import 'viewer_tool_cursor.dart';
import 'viewer_track.dart';
import 'viewer_camera.dart';
import 'viewer_paint.dart';
import 'viewer_progress_bar.dart';
import 'viewer_type.dart';
import 'viewer_region.dart';
import 'viewer_zoom.dart';

/// The magnifications the picker offers. `null` means fit-to-panel, which is
/// the default and the only one that changes as the panel is resized.
const List<double?> _zoomSteps = [null, 0.25, 0.5, 1.0, 2.0, 4.0];

/// Which channel the picture shows.
enum ViewerChannel { rgb, red, green, blue, alpha }

class ViewerPanelFrb extends StatefulWidget {
  const ViewerPanelFrb({super.key});

  @override
  State<ViewerPanelFrb> createState() => _ViewerPanelFrbState();
}

class _ViewerPanelFrbState extends State<ViewerPanelFrb>
    with SingleTickerProviderStateMixin {
  /// The magnification the Viewer is *heading for*: a multiple of comp
  /// resolution, or null for fit-to-panel, which is the only mode that follows
  /// the panel as it is resized.
  double? _zoom;
  ViewerChannel _channel = ViewerChannel.rgb;

  /// Whether the layer controls — the wireframe boxes, the handles and the
  /// hover highlight — are drawn over the picture (K-217). On by default,
  /// because a selected layer with no box is a layer you cannot see the extent
  /// of; the switch exists for judging the picture itself, where any mark over
  /// it is in the way.
  bool _wireframes = true;
  Offset _pan = Offset.zero;

  /// The composition this Viewer has already asked for a frame of — so
  /// fronting another one asks once, not on every rebuild.
  UuidValue? _askedFor;

  /// Edits made anywhere in the document, which are the other reason the
  /// picture has to be asked for again.
  StreamSubscription<ScopedChange>? _changes;

  /// The zoom's own motion (K-218).
  ///
  /// A magnification change is a *place* changing, not a value being nudged, so
  /// it is worth animating: jumping the picture from one magnification to
  /// another loses the reader's place, and the whole point of anchored zooming
  /// is that the place is kept. Held here rather than in an implicitly animated
  /// widget because the two things being animated — the magnification and the
  /// pan — have to move together or the anchor point drifts mid-flight.
  late final AnimationController _zoomMotion;

  @override
  void initState() {
    super.initState();
    // Built here rather than lazily on first use: a `late final` field is
    // constructed the first time it is *read*, and the first read on a Viewer
    // that was never zoomed is `dispose` — which builds a ticker against a
    // widget that has already left the tree, and throws.
    _zoomMotion = AnimationController(vsync: this)
      ..addListener(() => setState(() {}))
      ..addStatusListener((status) {
        // The picture is rendered at the size it is *shown* at, so the frame in
        // hand is the wrong resolution once the magnification has changed.
        // Asked for at the end rather than per tick: a render per frame of a
        // 120 ms animation is a render per frame for no visible gain.
        if (status == AnimationStatus.completed) _boundUi?.requestFrame();
      });
  }

  /// Where the animation started from, resolved to real numbers: the target may
  /// be "fit", which is a rule rather than a number, and a lerp needs both ends.
  double? _zoomFrom;
  Offset _panFrom = Offset.zero;

  /// How much motion the shell is set to show, read in [build] because that is
  /// where the theme scope is in reach.
  AnimationLevel _animationLevel = AnimationLevel.all;

  // --- Snapshots (K-416, docs/07 §2.2 item 14) ------------------------------
  //
  // A snapshot is a *display* affordance: the picture on screen, kept, and put
  // back over the picture while a button is held, for the before/after read
  // every grade leans on. Nothing crosses the bridge — no engine copy, no cache
  // entry, no export path anywhere near it. What is stored is exactly what the
  // stage's own [RepaintBoundary] rasterised, which is the picture and not the
  // marks over it: the wireframes, the region and every tool layer are siblings
  // of that boundary in the stack rather than children of it.

  /// The boundary the camera photographs: the picture alone.
  final GlobalKey _pictureKey = GlobalKey();

  /// The one slot. AE's four-slot family can follow on this same mechanism if
  /// it is ever asked for (K-416); one is what a before/after actually needs.
  dartui.Image? _snapshot;

  /// Whether the Show button is being held down this instant.
  bool _showingSnapshot = false;

  @override
  void dispose() {
    _unbind();
    _changes?.cancel();
    _zoomMotion.dispose();
    _snapshot?.dispose();
    super.dispose();
  }

  /// Photograph the picture as it stands.
  ///
  /// At the device's own pixel ratio, so a snapshot held against the live
  /// picture is the same sharpness rather than a softer copy of it — **but
  /// never at more pixels than the panel itself has**. The boundary is the
  /// picture's rectangle, which is the *comp* at this magnification and not the
  /// panel: at 800 % on an HD comp it is 15360 pixels across (K-230's number),
  /// and photographing that whole would ask for gigabytes of pixels nobody can
  /// see, on a button that must never be a risk to press.
  Future<void> _takeSnapshot() async {
    final boundary = _pictureKey.currentContext?.findRenderObject();
    if (boundary is! RenderRepaintBoundary) return;
    final ratio =
        MediaQuery.devicePixelRatioOf(context) * _snapshotFit(boundary);
    final dartui.Image shot;
    try {
      shot = await boundary.toImage(pixelRatio: ratio);
    } catch (_) {
      // A boundary that has not been painted yet has nothing to hand over.
      return;
    }
    if (!mounted) {
      shot.dispose();
      return;
    }
    final old = _snapshot;
    setState(() => _snapshot = shot);
    // After the frame, not during the swap: the old image may be on screen this
    // very instant (Show held while Take is pressed), and disposing an image a
    // live [RawImage] still points at is a crash rather than a saving.
    WidgetsBinding.instance.addPostFrameCallback((_) => old?.dispose());
  }

  /// How much of the device pixel ratio a snapshot may use: 1 while the picture
  /// fits the panel, and less once it is bigger, so the stored image is never
  /// larger than the panel could show. Both edges are covered, so nothing on
  /// screen is sampled below the resolution it is drawn at.
  ///
  /// ponytail: a zoomed-in snapshot is therefore the panel's worth of detail
  /// and no more — held against a 800 % live picture it is the softer of the
  /// two. Photographing the visible region instead of the whole picture is the
  /// upgrade, and it wants the boundary moved rather than a number changed.
  double _snapshotFit(RenderRepaintBoundary boundary) {
    final shot = boundary.size;
    final panel = context.size;
    if (panel == null || panel.isEmpty || shot.width <= 0 || shot.height <= 0) {
      return 1;
    }
    return math
        .max(panel.width / shot.width, panel.height / shot.height)
        .clamp(0.001, 1.0);
  }

  /// Show the stored picture while the button is down, and stop the moment it
  /// is up. Releasing the button is the whole of its lifecycle (K-416).
  void _holdSnapshot(bool held) {
    if (_snapshot == null && held) return;
    if (_showingSnapshot == held) return;
    setState(() => _showingSnapshot = held);
  }

  /// Take the Viewer to [scale] (null = fit) and [pan], smoothly when the shell
  /// animates at all.
  ///
  /// [from] is the magnification being left, which the caller already knows
  /// from the rectangle it measured — asking for it again here would need the
  /// constraints, which live in the layout builder.
  void _goToZoom(double? scale, Offset pan, {required double from}) {
    setState(() {
      _zoomFrom = from;
      _panFrom = _pan;
      _zoom = scale;
      _pan = pan;
    });
    final duration = animationDuration(_animationLevel);
    if (duration == Duration.zero) {
      _zoomMotion.value = 1;
      _boundUi?.requestFrame();
      return;
    }
    _zoomMotion.duration = duration;
    _zoomMotion.forward(from: 0);
  }

  /// Something changed the document: tell the engine, which decides what to do
  /// about it. Every commit comes through here — this panel makes no edits
  /// itself, so there is no local shortcut to take.
  void _onDocumentChanged() {
    _facts = null;
    _boundUi?.requestFrame();
  }

  /// What this panel has to ask the *engine* about the composition, as against
  /// what it reads from the model: its settings, its pixel size, and which of
  /// its layers have a file behind them.
  ///
  /// Asked once and held until an edit lands (K-230). None of the three can
  /// change without one, and they were being re-asked on every rebuild — which
  /// meant every pointer movement of a Hand-tool pan crossed the bridge four
  /// times and more, one of them walking every layer in the composition. A pan
  /// changes where the picture is drawn and nothing else; it must ask the
  /// engine nothing at all.
  ({
    BridgeCompSettings settings,
    BridgeCompSize size,
    List<FootageReference> footage,
  })? _facts;

  ({
    BridgeCompSettings settings,
    BridgeCompSize size,
    List<FootageReference> footage,
  }) _factsOf(CompositionReference comp) {
    final held = _facts;
    if (held != null) return held;
    final next = (
      settings: comp.getSettings(),
      size: comp.getSize(),
      footage: <FootageReference>[
        for (final layer in comp.getLayers())
          if (layer.getSourceItem() case ItemReference_Footage(:final field0))
            field0,
      ],
    );
    _facts = next;
    return next;
  }

  /// The shell's transport intent (the space bar). Subscribed here rather than
  /// exposed as a callback so the key is a quiet no-op when no Viewer is
  /// mounted.
  LumitUiState? _boundUi;

  void _unbind() {
    final ui = _boundUi;
    if (ui == null) return;
    ui.togglePlayRequest.removeListener(_onTogglePlayRequest);
    ui.playheadFrame.removeListener(_onPlayheadChanged);
    ui.viewerZoomRequest.removeListener(_onZoomRequest);
  }

  void _onTogglePlayRequest() => _togglePlay();

  /// The View menu, a chord or the command palette asked for a magnification
  /// (docs/07 §2.2, §15). The panel answers because only it knows where the
  /// magnification is now and what "fit" would mean at this size.
  ///
  /// A step is taken about the middle of the panel rather than about the
  /// pointer: there is no pointer in a menu choice, and the middle is the one
  /// point a keyboard zoom can promise to keep. That is why the pan goes back
  /// to zero — the picture is re-centred, not left where a drag had pushed it.
  void _onZoomRequest() {
    final request = _boundUi?.viewerZoomRequest.value;
    if (request == null || !mounted) return;
    final from = _shownScale;
    switch (request.$2) {
      case ViewerZoomCommand.fit:
        _goToZoom(null, Offset.zero, from: from);
      case ViewerZoomCommand.zoomIn:
        _goToZoom(_clampZoom(from * zoomToolStep), Offset.zero, from: from);
      case ViewerZoomCommand.zoomOut:
        _goToZoom(_clampZoom(from / zoomToolStep), Offset.zero, from: from);
    }
  }

  static double _clampZoom(double scale) =>
      scale.clamp(minViewerZoom, maxViewerZoom).toDouble();

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    if (!identical(_boundUi, ui)) {
      _unbind();
      _boundUi = ui;
      ui.togglePlayRequest.addListener(_onTogglePlayRequest);
      ui.playheadFrame.addListener(_onPlayheadChanged);
      ui.viewerZoomRequest.addListener(_onZoomRequest);
      _changes?.cancel();
      _changes = Provider.of<LumitState>(context, listen: false)
          .onChange
          .listen((_) => _onDocumentChanged());
      // The frame under the playhead as it stands: without this the Viewer
      // shows nothing at all until something moves the playhead. After the
      // frame, so the scale this asks at is the one just measured.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _onPlayheadChanged();
      });
    }
    final comp = ui.selectedComp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.footage,
        title: l10n.panelViewer,
        hint: l10n.selectACompositionFirst,
      );
    }
    // A newly fronted composition is a new picture to ask for. Nothing else
    // asks: the playhead has not moved and no edit has landed, so without this
    // the Viewer sat on the last comp's frame and — because the engine's idle
    // fill is anchored on the frame last *shown* — the new comp banked nothing
    // ahead of the playhead until the first edit happened to ask for a frame.
    if (_askedFor != comp.internalid) {
      _askedFor = comp.internalid;
      // Another composition is another set of facts.
      _facts = null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _onPlayheadChanged();
      });
    }

    final facts = _factsOf(comp);
    final settings = facts.settings;
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    _animationLevel = scope.animationLevel;
    final round = t.shape == ThemeShape.round;

    // Both notifiers, because the transport shows two things the engine owns:
    // where the playhead is, and whether it is running.
    final bar = ValueListenableBuilder<bool>(
      valueListenable: ui.playing,
      builder: (context, playing, _) => ValueListenableBuilder<int>(
        valueListenable: ui.playheadFrame,
        builder: (context, frame, _) => ValueListenableBuilder<int>(
          valueListenable: ui.previewTier,
          builder: (context, tier, _) => _Toolbar(
            zoom: _zoom,
            channel: _channel,
            // Session state rather than panel state (K-352): the engine has to
            // be told when it flips, and [LumitUiState] is what talks to it.
            grid: ui.viewerGrid,
            wireframes: _wireframes,
            look: ui.viewerLook,
            showToneMap: ui.workspace.interface.showToneMap,
            onStops: ui.setViewerStops,
            onToneMap: ui.toggleViewerToneMap,
            playing: playing,
            frame: frame,
            settings: settings,
            comp: comp,
            tier: tier,
            background: ui.model.heldBackground,
            // The magnification menu is a jump to a named place, so it flies
            // there like every other zoom (K-218) — from whatever is on screen,
            // which is what the measured rectangle in the layout builder knows.
            onZoom: (z) => _goToZoom(z, Offset.zero, from: _shownScale),
            onChannel: (c) => setState(() => _channel = c),
            onGrid: () => ui.setViewerGrid(!ui.viewerGrid),
            onWireframes: () => setState(() => _wireframes = !_wireframes),
            onPlayPause: _togglePlay,
            onSeek: (f) => _seek(comp, ui, f),
            hasSnapshot: _snapshot != null,
            onSnapshotTake: _takeSnapshot,
            onSnapshotHold: _holdSnapshot,
            detached: round,
          ),
        ),
      ),
    );

    // The picture. The preview progress bar used to float over the bottom of
    // it; it now rides on the right of the transport instead (K-287), where it
    // covers nothing and has a place of its own that is always the same size.
    final stage = LayoutBuilder(
      key: const ValueKey('viewer-stage'),
      builder: (context, constraints) {
        final size = facts.size;
        final fitted = _fittedRect(constraints, size);
        _reportScale(ui, fitted, size, _fitScale(constraints, size));

        // Which layers might be missing their file. Off the held facts, not
        // re-asked here: this used to live in the stage, which rebuilds per
        // frame during playback, so `getLayers` plus a `getSourceItem` per
        // layer crossed the bridge sixty times a second to re-answer a
        // question edits change and playback never does.
        final footage = facts.footage;

        void applyZoom(ViewerZoom next) => _goToZoom(
              next.scale,
              next.pan,
              from: size.width == 0 ? 1 : fitted.width / size.width,
            );

        return Listener(
          // The wheel zooms about the cursor (docs/07 §2.2): the comp point
          // under the pointer stays under the pointer, which is what makes
          // zooming feel like leaning in rather than teleporting.
          onPointerSignal: (event) {
            if (event is PointerScrollEvent) {
              // Shift+scroll belongs to the armed dropper, which is sizing
              // its sample region with it — zooming as well would move the
              // picture out from under the pixel being aimed at.
              if (ui.dropper.value != null &&
                  HardwareKeyboard.instance.isShiftPressed) {
                return;
              }
              _scrollZoom(event.localPosition, event.scrollDelta.dy,
                  constraints, size, fitted);
            }
          },
          child: ValueListenableBuilder<int>(
            valueListenable: ui.playheadFrame,
            builder: (context, frame, _) => _Stage(
              comp: comp,
              uiState: ui,
              fitted: fitted,
              grid: ui.viewerGrid,
              overlays: ui.viewerOverlays,
              pictureKey: _pictureKey,
              snapshot: _showingSnapshot ? _snapshot : null,
              wireframes: _wireframes,
              channel: _channel,
              compSize: size,
              footage: footage,
              onPan: (delta) => setState(() {
                // A pan during a zoom flight would be fighting it, so the
                // flight ends where it is and the drag takes over.
                _zoomFrom = null;
                _zoomMotion.value = 1;
                _pan += delta;
              }),
              // The model is *told* an edit landed, rather than the boxes
              // checking for themselves as they draw (K-230): the Viewer
              // commits its own edits, and the drawing path reads the held
              // copy now, so this is what puts the new document on screen
              // without waiting for the change stream's round trip. The same
              // thing every other panel does after committing.
              onChanged: () {
                ui.model.refresh();
                setState(() {});
              },
              onZoomAt: (at, {required bool out}) => applyZoom(zoomAboutPoint(
                cursor: at,
                factor: out ? 1 / zoomToolStep : zoomToolStep,
                fitted: fitted,
                compSize: Size(size.width.toDouble(), size.height.toDouble()),
                panel: Size(constraints.maxWidth, constraints.maxHeight),
              )),
              onZoomBox: (box, {required bool out}) => applyZoom(zoomToBox(
                box: box,
                out: out,
                fitted: fitted,
                compSize: Size(size.width.toDouble(), size.height.toDouble()),
                panel: Size(constraints.maxWidth, constraints.maxHeight),
              )),
            ),
          ),
        );
      },
    );

    // The transport belongs under the picture, where a transport goes. Round
    // makes that literal (K-394, docs/15 §12.1): the picture is one tile and
    // the transport another, parted by the same tile gap that parts the panes
    // themselves (K-092) with the canvas showing through, so the bar sits
    // *below* the picture instead of over it. It is still a child of this
    // panel's own column, so docking or dragging the Viewer carries the
    // transport with it. Sharp keeps the strip welded to the panel edge, and
    // the two shapes read as two deliberate designs rather than one with a gap.
    //
    // The picture's own box shrinks by the bar and the gap under Round, which
    // is the point: the layout builder above measures what is left, so fit,
    // zoom and every hit-test are against a picture that no longer has a bar
    // sitting on it.
    if (!round) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [Expanded(child: stage), bar],
      );
    }
    return ColoredBox(
      color: t.surface0,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: ClipRRect(
              borderRadius: BorderRadius.circular(t.tokens.cardRadius),
              child: stage,
            ),
          ),
          SizedBox(height: t.tokens.tileGap),
          bar,
        ],
      ),
    );
  }

  /// Where the picture sits in the panel, at the current magnification.
  ///
  /// Fit is the default and the only mode that follows the panel; a fixed
  /// magnification draws at that multiple of comp resolution and pans, which is
  /// what "100%" has to mean for it to be worth having.
  Rect _fittedRect(BoxConstraints constraints, BridgeCompSize size) {
    final w = size.width.toDouble();
    final h = size.height.toDouble();
    if (w <= 0 || h <= 0) return Rect.zero;

    // The target, resolved: "fit" is re-resolved every frame rather than
    // captured, so a panel resized mid-animation still lands on its own fit.
    final target = _zoom ?? _fitScale(constraints, size);
    var scale = target;
    var pan = _pan;
    final from = _zoomFrom;
    if (from != null && _zoomMotion.value < 1) {
      final t = Curves.easeOutCubic.transform(_zoomMotion.value);
      // Geometric, not linear: magnification is a *ratio*, and lerping the
      // number itself makes the second half of a big zoom crawl while the
      // first half bolts. Interpolating the logarithm is what makes a 1x → 8x
      // flight look like one steady move.
      scale = from * math.pow(target / from, t);
      pan = Offset.lerp(_panFrom, _pan, t) ?? _pan;
    }
    final drawn = Size(w * scale, h * scale);
    final centre = Offset(
      (constraints.maxWidth - drawn.width) / 2,
      (constraints.maxHeight - drawn.height) / 2,
    );
    // Snapped to the device-pixel grid before anyone sees it: the
    // checkerboard is painted anti-aliased while the platform texture is not,
    // so a fractional edge bled a soft row of board out under the picture at
    // some zooms. Snapping here rather than at a call site keeps the
    // invariant wherever the rect travels.
    return snapToDevicePixels(
      (centre + pan) & drawn,
      MediaQuery.devicePixelRatioOf(context),
    );
  }

  /// One wheel notch is ~12 % in or out, smooth on a trackpad (the delta is
  /// per-pixel there), anchored so the comp point under the cursor does not
  /// move. The anchoring itself is [zoomAboutPoint], shared with the Zoom tool
  /// so the wheel and the tool cannot drift apart.
  ///
  /// Not animated: the wheel already arrives as a stream of small steps, and
  /// animating each of them would make the picture lag the fingers.
  void _scrollZoom(Offset cursor, double dy, BoxConstraints constraints,
      BridgeCompSize size, Rect fitted) {
    if (size.width == 0 || fitted.width <= 0) return;
    final next = zoomAboutPoint(
      cursor: cursor,
      factor: math.pow(1.0012, -dy).toDouble(),
      fitted: fitted,
      compSize: Size(size.width.toDouble(), size.height.toDouble()),
      panel: Size(constraints.maxWidth, constraints.maxHeight),
    );
    setState(() {
      _zoomFrom = null;
      _zoomMotion.value = 1;
      _zoom = next.scale;
      _pan = next.pan;
    });
  }

  /// The magnification "Fit" means here: the whole picture in the panel.
  double _fitScale(BoxConstraints constraints, BridgeCompSize size) {
    final w = size.width.toDouble();
    final h = size.height.toDouble();
    if (w <= 0 || h <= 0) return 1;
    return constraints.maxWidth / w < constraints.maxHeight / h
        ? constraints.maxWidth / w
        : constraints.maxHeight / h;
  }

  /// Tell the engine what fraction of comp resolution the next render should be
  /// made at.
  ///
  /// **Not simply what is on screen** (K-230). The magnification the *panel*
  /// implies is what governs it — a Viewer docked small is cheap, which is the
  /// whole point of reporting anything — but zooming inside that panel does
  /// not: zooming out used to lower the preview resolution, which threw away
  /// every cached frame and made the picture visibly coarser for a gesture that
  /// only meant "let me see more of it". Zooming in cannot raise it either;
  /// above comp resolution there is nothing left to render (the clamp lives in
  /// [LumitUiState.reportViewerScale]).
  void _reportScale(
      LumitUiState state, Rect fitted, BridgeCompSize size, double fit) {
    if (size.width == 0) return;
    _shownScale = fitted.width / size.width;
    state.reportViewerScale(_shownScale > fit ? _shownScale : fit);
  }

  /// The magnification actually on screen, last time the picture was laid out.
  ///
  /// Kept because the bar is built outside the layout builder that measures it,
  /// and a zoom has to know where it is flying *from*.
  double _shownScale = 1;

  /// The playhead moved — from anywhere. The Timeline ruler, an arrow key and
  /// the transport all just set it, and this is what tells the engine.
  ///
  /// It is only ever *told*: moving the playhead is the user's own gesture and
  /// happens the instant they make it, with no round trip to wait on. What
  /// happens next — which frame to render, whether the one in flight is now
  /// worthless — is the engine's to decide.
  void _onPlayheadChanged() => _boundUi?.requestFrame();

  /// Move the playhead, taking the sound with it. Seeking while playing keeps
  /// playing, which is what makes scrubbing during playback usable rather than
  /// a stutter.
  void _seek(CompositionReference comp, LumitUiState state, int frame) {
    final last = comp.durationFrames() - 1;
    state.playheadFrame.value = frame.clamp(0, last < 0 ? 0 : last);
    audioSeek(secs: state.playheadFrame.value / comp.fps());
  }

  /// Start or stop playback. Two calls, because that is all a transport is:
  /// the clock, the frame choice, the sound and the end of the composition are
  /// all the engine's (K-181).
  void _togglePlay() {
    final ui = _boundUi;
    if (ui == null) return;
    ui.playing.value ? ui.stopPlayback() : ui.play();
  }
}

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
class _Stage extends StatelessWidget {
  final CompositionReference comp;
  final LumitUiState uiState;
  final Rect fitted;
  final bool grid;

  /// Which of the guides menu's marks are drawn over the picture (K-416).
  final ({bool grid, bool safeAreas}) overlays;

  /// The boundary the panel photographs for a snapshot — round the picture
  /// alone, so the marks over it are not in the photograph.
  final GlobalKey pictureKey;

  /// The stored snapshot, while the Show button is held; null the rest of the
  /// time, which is nearly always.
  final dartui.Image? snapshot;

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

  const _Stage({
    required this.comp,
    required this.uiState,
    required this.fitted,
    required this.grid,
    required this.overlays,
    required this.pictureKey,
    required this.snapshot,
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
    // and re-asks only when one of the two has moved — and only when some
    // layer actually has a mask to draw. The read model already knows that
    // without a call, and on a maskless comp the old unconditional ask was a
    // bridge call per playhead move for an answer that is always empty, which
    // is exactly what this method's budget (K-184) exists to stop.
    if (model.heldLayers.any((entry) => entry.info.masks.isNotEmpty)) {
      uiState.animatedMaskPaths.refresh(
        comp: comp,
        frame: uiState.playheadFrame.value,
        revision: revision,
      );
    }
    final viewScale = compSize.width == 0 ? 1.0 : fitted.width / compSize.width;
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
            ? uiState.layerBounds
                .boundsOf(entry, compSize: compSize, revision: revision)
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
            shapeContentsRect(entry.info.shapeContents)?.topLeft ?? Offset.zero,
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
    return ListenableBuilder(
      listenable: uiState.tools,
      builder: (context, _) => MouseRegion(
        // Which pointer the armed tool wears over the picture.
        cursor: viewerCursorFor(uiState.tools.tool),
        child: _stage(context, t),
      ),
    );
  }

  Widget _stage(BuildContext context, LumitTheme t) {
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      // Panning the picture, not the layer: the overlay's own handle takes
      // the gesture first when it is hit, so this only fires on empty space.
      onPanUpdate: (d) => onPan(d.delta),
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
                child: _Picture(uiState: uiState, channel: channel),
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
            if (_cloud() case final cloud?)
              // Listened to rather than read: the playhead moving does not
              // rebuild this panel by itself (it asks the engine for a frame,
              // and the picture arriving is what redraws), and the cloud has to
              // follow the frame it is drawn over.
              Positioned.fill(
                child: ValueListenableBuilder<int>(
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
                    accent: t.accent,
                    mark: t.textPrimary,
                    onChanged: onChanged,
                  ),
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
            // The held snapshot (K-416), over everything: while it is up the
            // Viewer is showing a second picture, and a wireframe belonging to
            // the live one drawn on top of it would be a lie about both. Fitted
            // to the picture's rectangle as it is *now*, so a zoom taken since
            // the snapshot compares like with like.
            if (snapshot case final shot?)
              Positioned.fromRect(
                rect: fitted,
                child: IgnorePointer(
                  child: RawImage(
                    key: const ValueKey('viewer-snapshot-overlay'),
                    image: shot,
                    fit: BoxFit.fill,
                  ),
                ),
              ),
          ],
        ),
      ),
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

/// The armed dropper over the picture: the magnifier, the sample-size wheel,
/// and the click that picks (docs/07 §6.1).
///
/// **Why it lives here.** The pixels being picked are the Viewer's, and only
/// this panel knows where the picture actually sits on screen at the current
/// magnification and pan. What is *done* with the pick is not this panel's
/// business at all: the parameter that armed the tool handed over a closure,
/// and this calls it.
///
/// Nothing at all while the tool is not armed — not a hit-test, not a listener.
class DropperLayer extends StatefulWidget {
  final CompositionReference comp;
  final LumitUiState uiState;

  /// Where the picture is drawn in this panel, at the current magnification.
  final Rect fitted;

  const DropperLayer({
    super.key,
    required this.comp,
    required this.uiState,
    required this.fitted,
  });

  @override
  State<DropperLayer> createState() => _DropperLayerState();
}

class _DropperLayerState extends State<DropperLayer> {
  /// Where the pointer is, in this layer's own coordinates, or null when it is
  /// not over the picture — which is where every arm starts, whatever the
  /// pointer did last time.
  Offset? _cursor;

  /// The viewfinder, while it is on screen. It lives in the application's
  /// overlay rather than in this panel's own stack, so it can hang over
  /// whatever is beside the Viewer instead of being pushed back inside it near
  /// a corner — the pointer keeps it at one fixed offset everywhere.
  OverlayEntry? _viewfinderEntry;

  /// Where the pointer is in the *overlay's* coordinates, and how much room the
  /// overlay has — both worked out when the pointer moves, and used afterwards
  /// as plain numbers.
  ///
  /// **Never worked out while building.** Placing the magnifier means asking
  /// render objects where they are, and a scroll over the Viewer zooms the
  /// picture, which relays this panel out: asking mid-rebuild asserts
  /// `attached` and takes the window red. A pointer event is the one moment
  /// both trees are settled, so that is when it is asked.
  Offset? _overlayCursor;
  Rect _overlayBounds = Rect.zero;

  /// How many pixels a side are averaged. One — this pixel and no other —
  /// until Shift+scroll says otherwise, and remembered for as long as the tool
  /// stays armed.
  int _region = dropperRegions.first;

  /// The reads that do go out are bounded like a drag's previews: crossing a
  /// window's edge at speed is not worth a read per frame, and the newest
  /// position is the only one worth answering.
  final PreviewThrottle _throttle = PreviewThrottle();

  @override
  void initState() {
    super.initState();
    // Escape puts the tool away wherever the focus happens to be — a tool armed
    // by accident must never need a click on the picture to get rid of.
    HardwareKeyboard.instance.addHandler(_onKey);
    widget.uiState.dropper.addListener(_onArmChanged);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    widget.uiState.dropper.removeListener(_onArmChanged);
    _hideViewfinder();
    _throttle.cancel();
    super.dispose();
  }

  /// Armed or disarmed: forget where the pointer was.
  ///
  /// Without this the *previous* pick's last pointer position survived, so
  /// arming the tool again put the magnifier on screen straight away, sitting
  /// wherever the last pick happened — before the pointer had gone anywhere
  /// near the Viewer. The magnifier belongs to the pointer being over the
  /// picture, and nothing else.
  void _onArmChanged() {
    _hideViewfinder();
    if (mounted) {
      setState(() {
        _cursor = null;
        _overlayCursor = null;
      });
    }
  }

  @override
  void didUpdateWidget(DropperLayer old) {
    super.didUpdateWidget(old);
    // A different composition is a different picture; a window read against the
    // old one is meaningless now.
    if (old.comp.internalid != widget.comp.internalid) {
      widget.uiState.dropperPatch.value = null;
      _hideViewfinder();
    }
    // The picture moved under the pointer (a zoom, a pan, the panel resized):
    // which pixel is under the pointer has changed, so the magnifier has to be
    // redrawn — but AFTER this build, never during it. Marking an overlay entry
    // dirty from inside a build is the "setState() called during build" error,
    // and it is what an ordinary scroll over the Viewer used to do.
    if (old.fitted != widget.fitted) _refreshViewfinderAfterFrame();
  }

  /// Redraw the magnifier once this frame is over.
  void _refreshViewfinderAfterFrame() {
    if (_viewfinderEntry == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _viewfinderEntry?.markNeedsBuild();
    });
  }

  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent) return false;
    if (event.logicalKey != LogicalKeyboardKey.escape) return false;
    if (widget.uiState.dropper.value == null) return false;
    widget.uiState.disarmDropper();
    return true;
  }

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<DropperArm?>(
      valueListenable: widget.uiState.dropper,
      builder: (context, arm, _) =>
          arm == null ? const SizedBox.shrink() : _armed(context, arm),
    );
  }

  Widget _armed(BuildContext context, DropperArm arm) {
    return Positioned.fill(
      child: MouseRegion(
        cursor: SystemMouseCursors.precise,
        onExit: (_) {
          setState(() {
            _cursor = null;
            _overlayCursor = null;
          });
          _hideViewfinder();
        },
        child: Listener(
          behavior: HitTestBehavior.opaque,
          onPointerHover: (e) => _moved(e.localPosition, e.position),
          onPointerMove: (e) => _moved(e.localPosition, e.position),
          onPointerSignal: (e) {
            if (e is! PointerScrollEvent) return;
            if (!HardwareKeyboard.instance.isShiftPressed) return;
            // Shift turns the wheel horizontal on most platforms, so take
            // whichever axis actually carries the motion — reading only the
            // vertical delta is why the egui build's size never changed.
            final d = e.scrollDelta;
            final scroll = d.dy.abs() >= d.dx.abs() ? d.dy : d.dx;
            if (scroll.abs() < 0.5) return;
            // Nothing is asked of the engine here: the window in hand already
            // holds every pixel a wider region could want.
            setState(() =>
                _region = nextDropperRegion(_region, scroll < 0 ? 1 : -1));
            _viewfinderEntry?.markNeedsBuild();
          },
          onPointerDown: (e) => _pressed(arm, e.localPosition),
          child: const SizedBox.expand(),
        ),
      ),
    );
  }

  /// Put the magnifier on screen, or take it off, and keep it beside the
  /// pointer while it is there.
  ///
  /// On screen only while the pointer is over the picture — there is nothing
  /// under it to magnify anywhere else — and in the application's overlay, so
  /// it keeps one fixed offset from the pointer everywhere on the picture
  /// instead of being pushed back inside the panel near an edge.
  void _syncViewfinder(DropperArm arm) {
    final at = _cursor;
    if (at == null || !widget.fitted.contains(at) || _overlayCursor == null) {
      _hideViewfinder();
      return;
    }
    if (_viewfinderEntry != null) {
      _viewfinderEntry!.markNeedsBuild();
      return;
    }
    final overlay = Overlay.maybeOf(context);
    if (overlay == null) return;
    _viewfinderEntry = OverlayEntry(builder: (_) => _viewfinderAt(arm));
    overlay.insert(_viewfinderEntry!);
  }

  void _hideViewfinder() {
    _viewfinderEntry?.remove();
    _viewfinderEntry = null;
  }

  /// The pointer's global position in the overlay's own coordinates, and the
  /// room the overlay has — remembered for the builder, which must not go
  /// looking for render objects itself (see [_overlayCursor]).
  ///
  /// The overlay's box is what carries the UI-scale transform, so a global
  /// pointer position is put through it rather than assumed to match.
  void _noteOverlayPosition(Offset global) {
    final overlayBox = Overlay.maybeOf(context)?.context.findRenderObject();
    if (overlayBox is! RenderBox ||
        !overlayBox.attached ||
        !overlayBox.hasSize) {
      _overlayCursor = null;
      return;
    }
    _overlayCursor = overlayBox.globalToLocal(global);
    _overlayBounds = Offset.zero & overlayBox.size;
  }

  /// The magnifier, placed at a fixed offset from the pointer — from numbers
  /// worked out when the pointer last moved, so this touches no render object
  /// and is safe to run in any frame.
  Widget _viewfinderAt(DropperArm arm) {
    final at = _cursor;
    final overlayAt = _overlayCursor;
    if (at == null || overlayAt == null) return const SizedBox.shrink();
    final origin = dropperViewfinderOrigin(
      overlayAt,
      // The window's content area: what the application can actually paint on,
      // and so the only edge the viewfinder has to answer to.
      _overlayBounds,
    );
    return Positioned(
      left: origin.dx,
      top: origin.dy,
      child: IgnorePointer(
        child: ValueListenableBuilder<BridgeSampledPixels?>(
          valueListenable: widget.uiState.dropperPatch,
          builder: (context, window, _) => DropperViewfinder(
            arm: arm,
            window: window,
            // In the window's own raster, which the reply describes — the
            // magnifier cannot be indexed in any other grid.
            centre:
                window == null ? (0, 0) : windowPixelAt(window, _u(at), _v(at)),
            region: _region,
          ),
        ),
      ),
    );
  }

  /// The pointer moved. Redrawing is free — the magnifier reads the window
  /// already in hand — so the engine is only asked when that window has run out
  /// of pixels under the pointer.
  void _moved(Offset local, Offset global) {
    _noteOverlayPosition(global);
    setState(() => _cursor = local);
    final arm = widget.uiState.dropper.value;
    if (arm != null) _syncViewfinder(arm);
    if (!widget.fitted.contains(local)) return;
    if (_covered(local)) return;
    _throttle.request(() => _request(local));
  }

  /// Whether the window in hand answers for the pointer where it now is: same
  /// frame, same source, and far enough from its edge.
  bool _covered(Offset local) {
    final window = widget.uiState.dropperPatch.value;
    if (window == null) return false;
    if (window.frame.toInt() != widget.uiState.playheadFrame.value) {
      return false;
    }
    if (window.layerAlone !=
        (widget.uiState.dropper.value?.sampleLayer != null)) {
      return false;
    }
    final (x, y) = windowPixelAt(window, _u(local), _v(local));
    return windowCovers(window, x, y);
  }

  /// A press picks when it lands on the picture, and puts the tool away when it
  /// lands anywhere else — the same escape the egui build gave, so a dropper
  /// armed in error is dismissed by clicking away from the frame.
  void _pressed(DropperArm arm, Offset local) {
    if (!widget.fitted.contains(local)) {
      widget.uiState.disarmDropper();
      return;
    }
    final window = widget.uiState.dropperPatch.value;
    // Nothing read yet, or nothing that answers for this pixel — a frame the
    // playhead has since left, or a window the pointer has outrun. Ask again
    // rather than committing a value off a picture that is not the one on
    // screen; the next reply lands before the pointer can click twice.
    if (window == null || !_covered(local)) {
      _request(local);
      return;
    }
    final (x, y) = windowPixelAt(window, _u(local), _v(local));
    arm.onPick(sampleFromWindow(window, _region, x, y));
    widget.uiState.disarmDropper();
  }

  /// Where the pointer is *inside the drawn picture*, as a fraction from 0 to 1.
  ///
  /// The only thing this panel actually knows, and deliberately all it says:
  /// which pixel that is depends on the raster the engine ends up reading, which
  /// is a reduced-resolution preview whenever the Viewer is showing one. The
  /// reply carries that raster, and every pixel is named in it.
  double _u(Offset local) => widget.fitted.width <= 0
      ? 0
      : ((local.dx - widget.fitted.left) / widget.fitted.width).clamp(0.0, 1.0);

  double _v(Offset local) => widget.fitted.height <= 0
      ? 0
      : ((local.dy - widget.fitted.top) / widget.fitted.height).clamp(0.0, 1.0);

  /// Ask the engine for a window around the point under `local`.
  void _request(Offset local) =>
      widget.uiState.requestDropperSample(_u(local), _v(local));
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

/// Whatever the worker last published, in the chosen channel — always a
/// platform texture (K-183): frames only ever arrive as GPU handles.
class _Picture extends StatelessWidget {
  final LumitUiState uiState;
  final ViewerChannel channel;
  const _Picture({required this.uiState, required this.channel});

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<int?>(
      valueListenable: uiState.viewerFrameid,
      builder: (context, textureId, _) {
        // Nearest by default: Flutter's `Texture` filters bilinearly unless
        // told otherwise, which softens every pixel once the zoom is past
        // 1:1 — the opposite of what zooming in is usually for. The setting
        // hands the smoothing back to anyone who wants it.
        final picture = textureId != null
            ? Texture(
                textureId: textureId,
                filterQuality: uiState.workspace.smoothZoomedViewer
                    ? FilterQuality.low
                    : FilterQuality.none,
              )
            : const SizedBox.expand();
        return pictureChannelFilter(channel, picture);
      },
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

/// Magnification, channel, grid, transport and timecode.
class _Toolbar extends StatelessWidget {
  final double? zoom;
  final ViewerChannel channel;
  final bool grid;
  final bool wireframes;

  /// How the fronted comp is being looked at (K-314). Passed down rather than
  /// read here: this bar rebuilds for every frame that arrives, and a control
  /// that asks the engine what it is set to would cross the boundary sixty
  /// times a second to be told what the frontend already knows.
  final ViewerLook look;

  /// Whether the tone map button is on the bar at all (Settings → Interface).
  /// Off by default; [look] is already gated to match, so a hidden button
  /// never leaves an engaged tone map behind it.
  final bool showToneMap;
  final bool playing;
  final int frame;
  final BridgeCompSettings settings;
  final CompositionReference comp;
  final ValueChanged<double?> onZoom;
  final ValueChanged<ViewerChannel> onChannel;
  final VoidCallback onGrid;
  final VoidCallback onWireframes;

  final ValueChanged<double> onStops;
  final VoidCallback onToneMap;
  final VoidCallback onPlayPause;
  final ValueChanged<int> onSeek;

  /// Whether a snapshot has been taken — the only thing that makes Show
  /// anything but a muted mark (K-416).
  final bool hasSnapshot;
  final VoidCallback onSnapshotTake;

  /// Held down, or let go.
  final ValueChanged<bool> onSnapshotHold;

  /// Drawn as a tile of its own — rounded, outlined and shadowed, sitting
  /// below the picture with the canvas showing between (round mode) — rather
  /// than a strip welded to the panel's bottom edge (sharp mode).
  final bool detached;

  /// The preview tier the last frame was made at, off the frame itself
  /// ([LumitUiState.previewTier]). Given rather than asked for: this bar
  /// rebuilds for each shown frame, and asking the engine here cost one call
  /// across the boundary for each of them.
  final int tier;

  /// The comp's background colour, off the held read model
  /// ([CompModel.heldBackground]) for the same reason as [tier]: given, not
  /// asked for, because this bar rebuilds for each shown frame. Null before
  /// the model's first read, which the swatch draws as black.
  final F32Array4? background;

  const _Toolbar({
    required this.zoom,
    required this.channel,
    required this.grid,
    required this.wireframes,
    required this.look,
    required this.showToneMap,
    required this.onStops,
    required this.onToneMap,
    required this.playing,
    required this.frame,
    required this.settings,
    required this.comp,
    required this.tier,
    required this.background,
    required this.onZoom,
    required this.onChannel,
    required this.onGrid,
    required this.onWireframes,
    required this.onPlayPause,
    required this.onSeek,
    required this.hasSnapshot,
    required this.onSnapshotTake,
    required this.onSnapshotHold,
    this.detached = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey('viewer-bar'),
      height: 26,
      decoration: BoxDecoration(
        color: t.surface1,
        borderRadius:
            detached ? BorderRadius.circular(t.tokens.floatRadius) : null,
        border: detached ? Border.all(color: t.hairline) : null,
        boxShadow: detached ? t.tokens.cardShadow : null,
      ),
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
          // The controls, and the preview progress on the right of the same bar
          // (K-287). The controls take the space that is left over, so the
          // progress appearing and going never moves any of them.
          Expanded(child: _controls(context, t)),
          ViewerProgressBar(
            tracker: Provider.of<LumitUiState>(context, listen: false)
                .previewProgress,
          ),
        ],
      ),
    );
  }

  /// Everything on the left of the bar, in K-411's instruments: the picture's
  /// scale, the view toggles, how the pixels read, the clock, the transport,
  /// then the right-edge readouts. Small gaps inside a group, wide ones
  /// between — the arrangement is the whole of what tells them apart.
  Widget _controls(BuildContext context, LumitTheme t) {
    // Scrolls rather than overflowing: a Viewer docked narrow has less width
    // than this bar wants, and an overflow stripe is not a design.
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: Row(
        children: [
          // --- The picture's scale (K-411 item 1). Both dropdowns hug their
          // own label: a magnification reading "Fit" and one reading "400%"
          // are different widths, and boxing them to a common one left a
          // gap that read as a missing control.
          BareDropdown<int>(
            key: const ValueKey('viewer-zoom'),
            // -1: a wheel zoom between the listed steps; the button shows
            // the true percentage and the menu still offers the steps.
            value: _zoomSteps.indexOf(zoom),
            options: [for (var i = 0; i < _zoomSteps.length; i++) i],
            label: (i) => i == -1
                ? '${((zoom ?? 1) * 100).round()}%'
                : _zoomSteps[i] == null
                    ? l10n.menuFit
                    : '${(_zoomSteps[i]! * 100).round()}%',
            onChanged: (i) => onZoom(_zoomSteps[i]),
          ),
          const SizedBox(width: _itemGap),
          // The preview resolution (docs/07 §2.2 item 2): how many pixels the
          // engine is asked to make, beside the magnification it is so easily
          // mistaken for. Muted while adaptive playback is choosing the tier
          // itself — a choice something else is making is not yours to make.
          const _ResolutionDropdown(),

          // --- The view toggles (K-411 item 2): one tight cluster of icons,
          // each of which changes what is drawn over or behind the picture
          // and nothing about the picture itself.
          const SizedBox(width: _groupGap),
          // Region of interest (K-362). Lit while a region is in force or a
          // drag is armed for one, so working on a corner of a shot is never
          // a state you can be in without being told; the same click clears a
          // region that exists.
          Builder(builder: (context) {
            final ui = context.watch<LumitUiState>();
            final set = ui.regionOfInterest != null;
            final arming = ui.armingRegion;
            return LumitTooltip(
              message: set
                  ? l10n.tipClearRegionOfInterest
                  : arming
                      ? l10n.tipDragRegionOfInterest
                      : l10n.tipRegionOfInterest,
              child: HouseButton(
                key: const ValueKey('viewer-region'),
                small: true,
                frameless: true,
                // A region that exists is cleared; otherwise the next drag on
                // the picture is armed to sweep one out.
                onPressed: () => set
                    ? ui.setRegionOfInterest(null)
                    : ui.armingRegion = !arming,
                child: lumitIcon(
                  LumitIcon.rectangle,
                  size: iconSize,
                  color: set || arming ? t.accent : t.textSecondary,
                ),
              ),
            );
          }),
          const SizedBox(width: _itemGap),
          // The transparency grid: the checkerboard itself rather than the
          // word "Grid" (K-411 item 2). The word was the odd one out in a row
          // of marks, and it named the thing least well — "grid" is also the
          // guide overlay, which this is not.
          LumitTooltip(
            message: l10n.tipTransparencyGrid,
            child: HouseButton(
              key: const ValueKey('viewer-grid'),
              small: true,
              frameless: true,
              onPressed: onGrid,
              child: lumitIcon(
                LumitIcon.checkerboard,
                size: iconSize,
                color: grid ? t.accent : t.textSecondary,
              ),
            ),
          ),
          const SizedBox(width: _itemGap),
          // The grid-and-guides menu (K-416, §2.2 items 5-6), beside the
          // transparency grid because the two are the pair most easily
          // confused: one is what shows *through* the picture, the other is
          // what is drawn *over* it.
          const ViewerGuidesMenu(),
          const SizedBox(width: _itemGap),
          // The layer controls switch (K-217): the boxes, handles and hover
          // highlight over the picture. An icon rather than a word, because
          // what it governs is a *mark* — and the mark is what it draws.
          LumitTooltip(
            message: wireframes
                ? l10n.tipHideLayerControls
                : l10n.tipShowLayerControls,
            child: HouseButton(
              key: const ValueKey('viewer-wireframes'),
              small: true,
              frameless: true,
              onPressed: onWireframes,
              child: lumitIcon(
                LumitIcon.wireframe,
                size: iconSize,
                color: wireframes ? t.accent : t.textSecondary,
              ),
            ),
          ),
          const SizedBox(width: _itemGap),
          _BackgroundSwatch(comp: comp, background: background),

          // --- How the pixels read (K-411 item 3): the three controls that
          // change what the numbers on screen mean.
          const SizedBox(width: _groupGap),
          // The channel picker as a mark tinted by its own answer: the face
          // is read at a glance during a key, and "Green" spelled out is a
          // word to read where a green mark is a thing to see. The menu still
          // lists the names.
          LumitTooltip(
            message: l10n.tipViewerChannel,
            child: BareDropdown<ViewerChannel>(
              key: const ValueKey('viewer-channel'),
              value: channel,
              options: ViewerChannel.values,
              label: _channelLabel,
              onChanged: onChannel,
              face: _channelFace(t, channel),
            ),
          ),
          const SizedBox(width: _itemGap),
          // Exposure and the tone map (K-314, docs/07 §2.2 items 12-13): the
          // two preview-only controls. Each reads in the accent while it is
          // engaged, which says "this control is on" and nothing more — that
          // the *picture* is not the export is the colour-management badge's
          // to say (item 8, further along this bar).
          lumitIcon(
            LumitIcon.aperture,
            size: iconSize,
            color: look.stops == 0 ? t.textSecondary : t.accent,
          ),
          const SizedBox(width: _itemGap),
          LumitTooltip(
            message: l10n.tipViewerExposure,
            child: DragValueField(
              key: const ValueKey('viewer-exposure'),
              value: look.stops,
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
              // engine treats the view as engaged. Since K-346 that no longer
              // costs the caches — a look names its frames rather than leaving
              // them nameless — but a hair nobody asked for would still bank a
              // whole second set of frames under a look that reads as neutral.
              onChanged: (v) => onStops((v * 10).round() / 10),
            ),
          ),
          // The tone map button is asked for rather than given (K-314): most
          // work never reads a picture that way, so the bar stays shorter
          // until somebody turns it on in Settings → Interface.
          if (showToneMap) ...[
            const SizedBox(width: _itemGap),
            LumitTooltip(
              message: l10n.tipViewerToneMap,
              child: HouseButton(
                key: const ValueKey('viewer-tone-map'),
                small: true,
                frameless: true,
                onPressed: onToneMap,
                child: lumitIcon(
                  LumitIcon.toneMap,
                  size: iconSize,
                  color: look.toneMap ? t.accent : t.textSecondary,
                ),
              ),
            ),
          ],

          // --- Snapshots (K-416, §2.2 item 14): a pair of its own, next to
          // the exposure group because a snapshot is what the exposure is
          // usually being judged against. Take photographs the picture; Show
          // is held down, and puts the photograph back over it.
          const SizedBox(width: _groupGap),
          LumitTooltip(
            message: l10n.tipViewerSnapshotTake,
            child: HouseButton(
              key: const ValueKey('viewer-snapshot-take'),
              small: true,
              frameless: true,
              onPressed: onSnapshotTake,
              child: lumitIcon(
                LumitIcon.snapshot,
                size: iconSize,
                color: t.textSecondary,
              ),
            ),
          ),
          const SizedBox(width: _itemGap),
          LumitTooltip(
            message: hasSnapshot
                ? l10n.tipViewerSnapshotShow
                : l10n.tipViewerSnapshotNone,
            // A press and hold, so the raw pointer rather than a tap: the
            // comparison lasts exactly as long as the button is down, and a
            // gesture recogniser only reports once it is over.
            child: Listener(
              onPointerDown: hasSnapshot ? (_) => onSnapshotHold(true) : null,
              onPointerUp: hasSnapshot ? (_) => onSnapshotHold(false) : null,
              onPointerCancel:
                  hasSnapshot ? (_) => onSnapshotHold(false) : null,
              child: HouseButton(
                key: const ValueKey('viewer-snapshot-show'),
                small: true,
                frameless: true,
                // The press is the Listener's; this only says whether the
                // button is live, which is what mutes it and what stops the
                // pointer becoming a hand over a control that does nothing.
                onPressed: hasSnapshot ? () {} : null,
                child: lumitIcon(
                  LumitIcon.eye,
                  size: iconSize,
                  color: hasSnapshot ? t.textSecondary : t.textDisabled,
                ),
              ),
            ),
          ),

          // --- The clock (K-411 item 4), a field of its own rather than
          // something to find between the transport and a badge. In a slot
          // wide enough for the longest time this comp can show, and
          // clickable to type one (docs/07 §2.2 item 11). A time past either
          // end of the composition lands on that end: the playhead cannot
          // leave the comp, so asking for somewhere outside it means the
          // nearest place inside.
          const SizedBox(width: _groupGap),
          TimeReadout(
            key: const ValueKey('viewer-timecode'),
            frame: frame,
            format: (f) => timecodeOf(f, settings),
            widthChars: timecodeChars(settings.fpsNum, settings.fpsDen),
            style: t.mono,
            parse: (text) =>
                framesOfTimecode(text, settings.fpsNum, settings.fpsDen),
            onCommit: onSeek,
            minFrame: 0,
            maxFrame: _lastFrameOf(settings),
            tooltip: l10n.tipFrameOnScreen,
          ),

          // --- The transport and the mode it runs in (K-411 item 5). A fixed
          // gap, not a Spacer: the bar scrolls when the panel is narrow, and
          // a flex child cannot live inside a scroll view.
          const SizedBox(width: _groupGap),
          // A slot rather than a button sized to its own label: the two modes
          // are two different words, and letting them size themselves moved
          // the whole transport across whenever the mode changed.
          const SizedBox(
            width: _playbackModeWidth,
            child: _PlaybackModeButton(),
          ),
          const SizedBox(width: _itemGap),
          // Round gathers the transport into one pill (K-394, §12.1): the five
          // buttons are one instrument, and a container round them says so.
          // Sharp is handed the very same widgets with nothing wrapped round
          // them, so its bar is unchanged down to the widget tree.
          if (t.shape == ThemeShape.round)
            Container(
              key: const ValueKey('viewer-transport-pill'),
              padding: const EdgeInsets.symmetric(horizontal: 2),
              decoration: BoxDecoration(
                color: t.surface2,
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              ),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: _transport(t),
              ),
            )
          else
            ..._transport(t),

          // --- The right edge (K-411 item 6): readouts, not controls, which
          // is why they live apart from everything above.
          const SizedBox(width: _groupGap),
          _ColourManagementBadge(look: look),
          // The degradation badge (docs/13 §B5, docs/07 §2.2): when adaptive
          // playback has dropped below Full, say so on the bar — a softer
          // picture must never be a mystery. The tier rides in on the frame,
          // so the bar draws it without asking anything.
          //
          // Its slot is there whether or not the badge is: a box that comes
          // and goes mid-playback would drag the bar about at the very
          // moment the picture is being watched.
          const SizedBox(width: _itemGap),
          SizedBox(
            width: _tierBadgeWidth,
            child: playing && tier > 1
                ? Center(
                    child: Container(
                      key: const ValueKey('viewer-tier-badge'),
                      padding: const EdgeInsets.symmetric(
                          horizontal: 6, vertical: 1),
                      decoration: BoxDecoration(
                        color: t.surface2,
                        borderRadius:
                            BorderRadius.circular(t.tokens.controlRadius),
                      ),
                      child: Text(
                        _PlaybackModeButton._tierNames[tier.clamp(0, 4)],
                        style: t.small.copyWith(color: t.warning),
                      ),
                    ),
                  )
                : null,
          ),
        ],
      ),
    );
  }

  /// The five transport buttons, in order. Pulled out so both shapes get the
  /// identical set — Round inside its pill, Sharp loose on the bar.
  List<Widget> _transport(LumitTheme t) => [
        HouseButton(
          key: const ValueKey('viewer-home'),
          small: true,
          frameless: true,
          onPressed: () => onSeek(0),
          child: Text('|◀', style: t.small),
        ),
        HouseButton(
          key: const ValueKey('viewer-step-back'),
          small: true,
          frameless: true,
          onPressed: () => onSeek(frame - 1),
          child: Text('◀', style: t.small),
        ),
        HouseButton(
          key: const ValueKey('viewer-play'),
          small: true,
          onPressed: onPlayPause,
          // The transport is the one place the spec asks for 20 (§5): it
          // is the control the eye goes to without looking for it.
          child: lumitIcon(playing ? LumitIcon.pause : LumitIcon.play,
              size: iconSizeTransport, color: t.textPrimary),
        ),
        HouseButton(
          key: const ValueKey('viewer-step-forward'),
          small: true,
          frameless: true,
          onPressed: () => onSeek(frame + 1),
          child: Text('▶', style: t.small),
        ),
        HouseButton(
          key: const ValueKey('viewer-end'),
          small: true,
          frameless: true,
          onPressed: () => onSeek(comp.durationFrames() - 1),
          child: Text('▶|', style: t.small),
        ),
      ];

  /// The channel picker's closed face (K-411 item 3): one mark, tinted by
  /// whichever channel is being shown.
  ///
  /// The tints are the Scopes panel's own red, green and blue
  /// ([ScopeColours.standard]) — the only place in the theme module that names
  /// the three, and the right ones by meaning: a scope's red trace and a red
  /// channel view are the same red channel. They do not vary by theme, which
  /// is also correct here, because what they stand for does not either.
  ///
  /// Alpha is not a colour, so it gets a different mark rather than a tint: a
  /// matte, which is what an alpha view is drawn as.
  static Widget _channelFace(LumitTheme t, ViewerChannel c) => lumitIcon(
        c == ViewerChannel.alpha ? LumitIcon.matte : LumitIcon.channels,
        size: iconSize,
        color: switch (c) {
          ViewerChannel.rgb || ViewerChannel.alpha => t.textSecondary,
          ViewerChannel.red => ScopeColours.standard.red,
          ViewerChannel.green => ScopeColours.standard.green,
          ViewerChannel.blue => ScopeColours.standard.blue,
        },
      );

  static String _channelLabel(ViewerChannel c) => switch (c) {
        ViewerChannel.rgb => 'RGB',
        ViewerChannel.red => engineLabel('Red'),
        ViewerChannel.green => engineLabel('Green'),
        ViewerChannel.blue => engineLabel('Blue'),
        ViewerChannel.alpha => l10n.channelAlpha,
      };
}

/// `HH:MM:SS:FF` for `frame` at the comp's rate — the shared clock face in
/// state/timecode.dart, bound to this comp's settings.
String timecodeOf(int frame, BridgeCompSettings settings) =>
    timecodeOfRate(frame, settings.fpsNum, settings.fpsDen);

/// The last frame of a comp, from its settings alone.
///
/// Worked out here rather than asked of the engine: this is read while the bar
/// is being built, and the bar is built for every frame of playback (K-184).
/// Whole-integer arithmetic, so a long comp at 29.97 cannot drift the way a
/// double would.
int _lastFrameOf(BridgeCompSettings settings) {
  final den = settings.duration.den.toInt() * settings.fpsDen;
  if (den <= 0) return 0;
  final frames = settings.duration.num.toInt() * settings.fpsNum ~/ den;
  return frames > 0 ? frames - 1 : 0;
}

/// The two gaps the bar is built out of (K-411): controls doing one job sit at
/// icon spacing, and the groups themselves are parted by three times as much.
/// The bar used to be an even queue at 6, which is neither — near enough to
/// touching that nothing grouped, far enough apart that nothing was tight.
const double _itemGap = 4;
const double _groupGap = 12;

/// The slot the playback-mode button sits in — wide enough for either of the
/// two labels, so changing mode does not shuffle the transport sideways.
const double _playbackModeWidth = 92;

/// The slot the degradation badge sits in, kept whether or not the badge is
/// showing.
const double _tierBadgeWidth = 52;

/// The slot the colour-management badge sits in — wide enough for the longer of
/// its two readings, so engaging the exposure cannot shove the transport along.
const double _colourBadgeWidth = 148;

/// **What am I looking at?** — the colour-management badge (docs/07 §2.2 item 8).
///
/// It is always on the bar, and it always names the display transform the
/// picture is being shown through: working space to display, which for now is
/// the one built-in pair, scene-linear to sRGB (docs/06 §3.3 — OCIO slots in
/// here later and this is the readout it will feed).
///
/// **And while either preview-only control is engaged, it says so.** Exposure
/// and the tone map live inside that same display transform (K-314) and change
/// nothing the export will ever see; the two controls draw themselves in the
/// accent while they are on, but that only says "this control is on" and only
/// says it where you are already looking. The statement that *the picture is
/// not the export* belongs here, stated calmly rather than warned about
/// (15-DESIGN) — a badge you can read without leaving the picture.
///
/// A readout, not a control. §2.2 asks that clicking it open colour settings;
/// there are none to open yet (docs/TODO.md), and a badge that looked pressable
/// and did nothing would be worse than one that plainly is not.
class _ColourManagementBadge extends StatelessWidget {
  final ViewerLook look;

  const _ColourManagementBadge({required this.look});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final engaged = look.stops != 0 || look.toneMap;
    return SizedBox(
      width: _colourBadgeWidth,
      child: LumitTooltip(
        message: engaged ? l10n.tipViewerPreviewView : l10n.tipDisplayTransform,
        child: Center(
          child: Container(
            key: const ValueKey('viewer-colour-badge'),
            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
            decoration: BoxDecoration(
              color: t.surface2,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
            child: Text(
              engaged
                  ? l10n.viewerDisplayTransformPreview(
                      l10n.viewerDisplayTransform)
                  : l10n.viewerDisplayTransform,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style:
                  t.small.copyWith(color: engaged ? t.accent : t.textSecondary),
            ),
          ),
        ),
      ),
    );
  }
}

/// Which playback behaviour is in force, and a click to change it.
///
/// **Why this is on the bar rather than buried in Settings.** The two modes
/// disagree about what playback *is* — one keeps time and lets the picture go
/// soft, the other shows every frame and takes as long as it takes — so a
/// picture that looks wrong or a transport that runs slow is explained by which
/// one you are in. Being unable to see that from the Viewer is what makes it
/// feel broken rather than chosen.
///
/// Which route frames take from the engine to the Viewer, in words.
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

/// The composition's background colour, on the bar (docs/07 §2.2 item 10,
/// K-357).
///
/// **A document edit, unlike everything else on this half of the bar.** The
/// exposure, the tone map and the transparency grid are ways of *looking*;
/// this is what the comp is actually drawn onto and what an export writes
/// there, so it goes through an op and Ctrl+Z undoes it. It sits beside the
/// grid button because the two answer the same question from opposite sides —
/// what is behind the picture — and finding one without the other is what
/// makes a black comp confusing.
class _BackgroundSwatch extends StatelessWidget {
  final CompositionReference comp;

  /// The colour to show, off the held read model — never asked for here:
  /// this swatch rebuilds with the bar, once per arriving frame (K-184).
  final F32Array4? background;
  const _BackgroundSwatch({required this.comp, required this.background});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final state = Provider.of<LumitState>(context, listen: false);
    // Off the read model, handed down by the bar — a document change
    // refreshes the model and rebuilds this from the new colour.
    final List<double> rgba = background ?? const [0.0, 0.0, 0.0, 1.0];
    int byte(double v) => (v.clamp(0.0, 1.0) * 255).round();
    final shown =
        documentColour(byte(rgba[0]), byte(rgba[1]), byte(rgba[2]), 255);

    return LumitTooltip(
      message: l10n.tipCompBackground,
      child: GestureDetector(
        key: const ValueKey('viewer-background'),
        behavior: HitTestBehavior.opaque,
        onTap: () async {
          final box = context.findRenderObject();
          if (box is! RenderBox) return;
          await showColourPicker(
            context: context,
            position: box.localToGlobal(Offset(0, box.size.height + 6)),
            initial: PickedColour.of(shown),
            // Chosen as a display colour, like a solid's, so the fields read
            // 0–255 rather than scene-linear floats.
            scale: ColourScale.bytes,
            presets: t.backgroundPresets,
            onCommit: (picked) {
              try {
                comp.setBackground(
                  rgba: F32Array4(Float32List.fromList([
                    picked.r.toDouble(),
                    picked.g.toDouble(),
                    picked.b.toDouble(),
                    1.0,
                  ])),
                );
              } catch (_) {
                return;
              }
              state.notifyDocumentChanged();
            },
          );
        },
        child: MouseRegion(
          cursor: SystemMouseCursors.click,
          child: Container(
            width: 22,
            height: 14,
            decoration: BoxDecoration(
              color: shown,
              border: Border.all(color: t.hairlineStrong),
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
          ),
        ),
      ),
    );
  }
}

/// The preview resolution, on the bar (docs/07 §2.2 item 2).
///
/// The same choice the View menu's Resolution rows make — [LumitUiState] holds
/// it, so the menu tick and this face cannot disagree. Disabled while playback
/// is adaptive: the engine is walking the degradation ladder itself, and a
/// dropdown that claimed the choice while something else made it would be a
/// control that lies.
class _ResolutionDropdown extends StatelessWidget {
  const _ResolutionDropdown();

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    final adaptive = ui.workspace.performance.playback == PlaybackMode.adaptive;
    return LumitTooltip(
      message: adaptive
          ? l10n.tipPreviewResolutionAdaptive
          : l10n.tipPreviewResolution,
      child: BareDropdown<PreviewResolution>(
        key: const ValueKey('viewer-resolution'),
        value: ui.previewResolution,
        options: PreviewResolution.values,
        label: (resolution) => resolution.title,
        onChanged: adaptive ? null : ui.setPreviewResolution,
      ),
    );
  }
}

/// Which of the two playback behaviours is in force — the name of the mode and
/// nothing else (K-287).
///
/// It used to carry the settled tier beside the name ("Adaptive · Half"), which
/// meant the button re-lettered itself as the engine felt its way up and down
/// the ladder — a word changing under the pointer, in the corner of the eye,
/// through every second of playback. Which tier a frame was made at is the
/// degradation badge's job, and the badge already says it while it matters.
class _PlaybackModeButton extends StatelessWidget {
  const _PlaybackModeButton();

  static List<String> get _tierNames => [
        l10n.menuFull,
        l10n.menuFull,
        l10n.menuHalf,
        l10n.resolutionThird,
        l10n.menuQuarter,
      ];

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context);
    final adaptive = ui.workspace.performance.playback == PlaybackMode.adaptive;
    final label =
        adaptive ? l10n.playbackAdaptiveShort : l10n.playbackEveryFrame;

    // Which route frames take to get here. A build without a zero-copy path
    // copies every pixel down, serialises it a byte at a time and uploads it
    // again, which is the difference between playback feeling immediate and
    // feeling heavy — so it is worth being able to read off the screen.
    final transport = _transportName;

    return LumitTooltip(
      message: adaptive
          ? l10n.tipPlaybackAdaptive(transport)
          : l10n.tipPlaybackEveryFrame(transport),
      child: HouseButton(
        key: const ValueKey('viewer-playback-mode'),
        small: true,
        onPressed: () {
          ui.workspace.performance.playback =
              adaptive ? PlaybackMode.everyFrame : PlaybackMode.adaptive;
          ui.workspace.touch();
        },
        child: Text(
          label,
          style: t.small.copyWith(color: adaptive ? null : t.accent),
        ),
      ),
    );
  }
}
