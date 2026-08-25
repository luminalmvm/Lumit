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
import 'package:lumit_flutter/src/rust/api/colour.dart';
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
import '../shell/welcome_frb.dart' show EmptyStageFrb;
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
import '../widgets/escape_ladder.dart';
import '../widgets/time_readout.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
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
import 'viewer_prefix_chip.dart';
import 'viewer_progress_bar.dart';
import 'viewer_type.dart';
import 'viewer_region.dart';
import 'viewer_zoom.dart';

/// The magnifications the picker offers. `null` means fit-to-panel, which is
/// the default and the only one that changes as the panel is resized.
const List<double?> _zoomSteps = [null, 0.25, 0.5, 1.0, 2.0, 4.0];

/// Which channel the picture shows.
enum ViewerChannel { rgb, red, green, blue, alpha }

// --- The project's own picture (K-468) --------------------------------------
//
// In plain terms: when a project is saved, the welcome screen wants a small
// picture of it to show on its recent row next time. The picture it wants is
// the composition as it looks right now — which is exactly what the Viewer is
// already showing, inside the very [RepaintBoundary] the Snapshot button
// photographs.
//
// **So no engine call is involved, and none exists.** A composition frame never
// crosses the bridge as pixels: zero-copy is the only Viewer transport (K-183),
// so a rendered frame reaches Dart as a texture handle and the read-back path
// was deleted. Photographing the boundary is not a workaround around a call
// that exists — it is the only place in the process where those pixels are
// addressable at all. If the engine ever grows a call that renders a
// composition to bytes off the playback path, this is the one function that
// should change.

/// The picture boundary of the Viewer currently on screen, or null when there
/// is no Viewer up — the welcome screen's window, or a workspace with the panel
/// closed. Set by the panel while it is mounted; see `_lendPicture`.
GlobalKey? viewerPictureKey;

/// How many pixels across a project thumbnail is captured at.
///
/// The welcome row draws it 64 wide, so this is that at 200 % and no more: a
/// picture nobody will ever see at full size is bytes on somebody's disk and
/// milliseconds on every save.
const double projectThumbnailPixels = 128;

/// Photograph the composition on screen as a small PNG, for the welcome
/// screen's recent rows (K-468).
///
/// Null whenever there is nothing honest to hand back: no Viewer up, a boundary
/// that has not painted yet, or a driver that will not read the picture back.
/// A missing thumbnail is an ordinary state and the row is built for it.
Future<Uint8List?> captureViewerPicturePng() async {
  final boundary = viewerPictureKey?.currentContext?.findRenderObject();
  if (boundary is! RenderRepaintBoundary) return null;
  final size = boundary.size;
  if (size.isEmpty) return null;
  // The boundary is the *whole* picture at the current magnification, not the
  // visible part of it (see the stack in `_stage`), so this is the composition
  // frame however the Viewer happens to be zoomed or panned.
  final ratio = (projectThumbnailPixels / size.width).clamp(0.001, 1.0);
  final shot = await boundary.toImage(pixelRatio: ratio);
  try {
    final png = await shot.toByteData(format: dartui.ImageByteFormat.png);
    return png?.buffer.asUint8List();
  } finally {
    shot.dispose();
  }
}

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
    _lendPicture();
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

  /// Lend that boundary to [captureViewerPicturePng] while this Viewer is up.
  ///
  /// A field rather than a `GlobalKey` constant because a global key must be
  /// unique in the tree, and nothing stops a workspace from carrying two
  /// Viewers; the last one built is the one photographed, and a Viewer only
  /// gives the slot up if it is still holding it.
  void _lendPicture() => viewerPictureKey = _pictureKey;

  void _returnPicture() {
    if (identical(viewerPictureKey, _pictureKey)) viewerPictureKey = null;
  }

  /// The one slot. AE's four-slot family can follow on this same mechanism if
  /// it is ever asked for (K-416); one is what a before/after actually needs.
  dartui.Image? _snapshot;

  /// Whether the Show button is being held down this instant.
  bool _showingSnapshot = false;

  @override
  void dispose() {
    _returnPicture();
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
    // Nothing to show: the ways to start work, or this panel's ordinary empty
    // line when the project does have compositions (K-481, shell/welcome_frb).
    if (comp == null) return const EmptyStageFrb();
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

    // How the chrome is arranged round the picture (K-448's setting, K-466's
    // drawing): the drawing's split by default, or everything gathered into
    // one strip at whichever end is asked for.
    final arrangement = ui.workspace.interface.viewerBars;
    final split = arrangement == ViewerBars.split;

    // The panel's own header strip (K-466): the Viewer's kicker, and the three
    // pickers the drawing puts at its right — the magnification, the preview
    // quality and the colour pipeline. The Viewer docks as a pane of its own,
    // so the dock draws no strip above it and this is the panel's only title.
    //
    // The magnification menu is a jump to a named place, so it flies there
    // like every other zoom (K-218) — from whatever is on screen, which is
    // what the measured rectangle in the layout builder knows.
    void goToNamedZoom(double? z) =>
        _goToZoom(z, Offset.zero, from: _shownScale);
    final header = split
        ? _ViewerHeader(
            zoom: _zoom,
            shownScale: _shownScale,
            look: ui.viewerLook,
            showToneMap: ui.workspace.interface.showToneMap,
            onToneMap: ui.toggleViewerToneMap,
            onZoom: goToNamedZoom,
            detached: round,
          )
        : null;

    // Both notifiers, because the transport shows two things the engine owns:
    // where the playhead is, and whether it is running.
    final bar = ValueListenableBuilder<bool>(
      valueListenable: ui.playing,
      builder: (context, playing, _) => ValueListenableBuilder<int>(
        valueListenable: ui.playheadFrame,
        builder: (context, frame, _) => ValueListenableBuilder<int>(
          valueListenable: ui.previewTier,
          builder: (context, tier, _) => _ViewerBar(
            channel: _channel,
            // Session state rather than panel state (K-352): the engine has to
            // be told when it flips, and [LumitUiState] is what talks to it.
            grid: ui.viewerGrid,
            wireframes: _wireframes,
            look: ui.viewerLook,
            onStops: ui.setViewerStops,
            playing: playing,
            frame: frame,
            settings: settings,
            comp: comp,
            compSize: facts.size,
            tier: tier,
            shownScale: _shownScale,
            background: ui.model.heldBackground,
            onChannel: (c) => setState(() => _channel = c),
            onGrid: () => ui.setViewerGrid(!ui.viewerGrid),
            onWireframes: () => setState(() => _wireframes = !_wireframes),
            onPlayPause: _togglePlay,
            onSeek: (f) => _seek(comp, ui, f),
            hasSnapshot: _snapshot != null,
            onSnapshotTake: _takeSnapshot,
            onSnapshotHold: _holdSnapshot,
            detached: round,
            // Gathered: the header's contents lead the one strip, in the
            // order the two strips read.
            leading: split
                ? const []
                : [
                    Text(l10n.panelViewer.toUpperCase(), style: t.kickerOn),
                    viewerBarGapBox(viewerBarGap),
                    ...viewerPickers(
                      zoom: _zoom,
                      shownScale: _shownScale,
                      look: ui.viewerLook,
                      showToneMap: ui.workspace.interface.showToneMap,
                      onToneMap: ui.toggleViewerToneMap,
                      onZoom: goToNamedZoom,
                    ),
                  ],
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
    // The strips above and below the picture, in whichever arrangement is set:
    // the drawing's header above and bar below, or the one gathered strip at
    // the top or at the bottom.
    final above = <Widget>[
      if (header != null) header,
      if (!split && arrangement == ViewerBars.top) bar
    ];
    final below = <Widget>[
      if (split || arrangement == ViewerBars.bottom) bar,
    ];

    if (!round) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [...above, Expanded(child: stage), ...below],
      );
    }
    return ColoredBox(
      color: t.surface0,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final strip in above) ...[
            strip,
            SizedBox(height: t.tokens.tileGap)
          ],
          Expanded(
            child: ClipRRect(
              borderRadius: BorderRadius.circular(t.tokens.cardRadius),
              child: stage,
            ),
          ),
          for (final strip in below) ...[
            SizedBox(height: t.tokens.tileGap),
            strip
          ],
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
    final shown = fitted.width / size.width;
    if (shown != _shownScale) {
      _shownScale = shown;
      // The bar and the header are built *above* the layout builder that
      // measures the picture, so the magnification they read is last frame's.
      // Nothing else redraws them when only the panel's size changed, and the
      // percentage in the readout would sit at whatever it was until a frame
      // happened to arrive. After the frame, never during it: this runs inside
      // a layout, where a `setState` is an assertion rather than a rebuild.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) setState(() {});
      });
    }
    state.reportViewerScale(
      _shownScale > fit ? _shownScale : fit,
      // A zoom in flight lays out on every tick of it. The scale it passes
      // through on the way is not one to ask the engine for a frame at
      // (K-430); the one it lands on is.
      settled: !_zoomMotion.isAnimating,
    );
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
            // The selection's name, over the stage's own corner (K-466). Last
            // in the stack because it is chrome: it names what is selected
            // whatever is being drawn underneath, including a held snapshot.
            _ViewerTag(uiState: uiState),
            // And what is being *looked at*, when that is not the finished
            // composition (K-528). Its own file, so this is one line: the chip
            // is the Viewer's, but it follows the effect selection rather than
            // anything this panel knows.
            ViewerPrefixChip(uiState: uiState),
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
/// and the **drag** that picks (docs/07 §6.1, K-532).
///
/// **A pick is a drag.** The press does not write anything. It starts a
/// gesture: every move stages the sample under the pointer and previews it, so
/// a colour is *swept* and a point is *slid* into place while the picture
/// answers; the release commits that last sample once, which is the one undo
/// step. Escape puts back what the drag was previewing. This is the same
/// stage/preview/commit every value field uses — a pick that wrote on
/// mouse-down was the one gesture in the application that decided before you
/// could see what you had chosen.
///
/// **Why it lives here.** The pixels being picked are the Viewer's, and only
/// this panel knows where the picture actually sits on screen at the current
/// magnification and pan. What is *done* with the pick is not this panel's
/// business at all: the parameter that armed the tool handed over closures,
/// and this calls them.
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

  /// The previews the pick drag itself sends, bounded separately from the reads
  /// above: a render is a great deal more work than a 66 KiB window, and the
  /// two rates have nothing to do with each other.
  final PreviewThrottle _previews = PreviewThrottle();

  /// The sample the drag has staged — what a release would commit, and what a
  /// preview shows. Null before the first covered sample of a gesture, which is
  /// what makes a press on a picture nothing has been read of commit nothing.
  DropperSample? _staged;

  /// Whether a press is down and the pick is being dragged.
  bool _dragging = false;

  @override
  void initState() {
    super.initState();
    // Escape puts the tool away wherever the focus happens to be — a tool armed
    // by accident must never need a click on the picture to get rid of.
    _escapeRelease = EscapeLadder.register(EscapeRung.gesture, _escape);
    widget.uiState.dropper.addListener(_onArmChanged);
  }

  @override
  void dispose() {
    _escapeRelease?.call();
    _escapeRelease = null;
    widget.uiState.dropper.removeListener(_onArmChanged);
    _hideViewfinder();
    _throttle.cancel();
    // A held preview tick must not fire into a panel that has gone. Nothing is
    // reverted here: reverting renders, and rendering from `dispose` is the
    // setState-while-tearing-down fault transform_rows had to defer round.
    _previews.cancel();
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
    // Whatever a drag had staged belongs to the arm that is going, not to the
    // one arriving. No revert here: disarming is *also* what a committed pick
    // does, and putting the old value back after a commit would undo it.
    _previews.cancel();
    _staged = null;
    _dragging = false;
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

  /// How to stand down from the ladder.
  VoidCallback? _escapeRelease;

  bool _escape() {
    if (widget.uiState.dropper.value == null) return false;
    // Escape mid-drag puts back what was being previewed *and* puts the tool
    // away — the convention every staged gesture keeps (docs/07 §4).
    _abandon();
    widget.uiState.disarmDropper();
    return true;
  }

  /// Throw away a drag in progress: stop the previews, and ask whatever armed
  /// the tool to put its own value back. Nothing was ever committed, so there
  /// is no undo step to unwind — only a picture to correct.
  void _abandon() {
    _previews.cancel();
    final staged = _staged;
    _staged = null;
    _dragging = false;
    if (staged != null) widget.uiState.dropper.value?.onRevert?.call();
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
          onPointerDown: (e) => _pressed(arm, e.localPosition, e.position),
          onPointerUp: (e) => _released(arm, e.localPosition),
          onPointerCancel: (_) => _abandon(),
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
    // A move with the button down is the pick itself moving: stage the sample
    // under the pointer and show it. The read below still happens when the
    // window has run out, so a sweep across the picture keeps answering.
    if (_dragging && arm != null) _stage(arm, local);
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

  /// A press **starts** a pick when it lands on the picture, and puts the tool
  /// away when it lands anywhere else — the same escape the egui build gave, so
  /// a dropper armed in error is dismissed by clicking away from the frame.
  ///
  /// Nothing is written here. The press only stages what is under it, so that a
  /// click that never moves still has a value to commit on release.
  ///
  /// **A press is a position too.** The magnifier used to be put up by the
  /// hover alone, which is fine for a mouse and nothing at all for a pointer
  /// that has no hover — a touch, a stylus, or a pointer that arrives over the
  /// picture already down. The pick then ran with no grid to aim by. The press
  /// says where it is like any other movement does.
  void _pressed(DropperArm arm, Offset local, Offset global) {
    if (!widget.fitted.contains(local)) {
      widget.uiState.disarmDropper();
      return;
    }
    _noteOverlayPosition(global);
    setState(() => _cursor = local);
    _syncViewfinder(arm);
    _dragging = true;
    _staged = null;
    _stage(arm, local);
  }

  /// One tick of the pick drag: the sample under the pointer, staged and
  /// previewed.
  ///
  /// Nothing is staged off a window that does not answer for this pixel — a
  /// frame the playhead has since left, or one the pointer has outrun. Another
  /// read is asked for instead, and the next move stages off the reply; a
  /// release with nothing staged commits nothing at all rather than a value
  /// lifted from a picture that is not the one on screen.
  void _stage(DropperArm arm, Offset local) {
    final window = widget.uiState.dropperPatch.value;
    if (window == null || !_covered(local)) {
      _request(local);
      return;
    }
    final (x, y) = windowPixelAt(window, _u(local), _v(local));
    _staged = sampleFromWindow(window, _region, x, y);
    if (arm.onPreview == null) return;
    // Built inside the closure, so a held tick sends where the pointer is now
    // rather than where it was when the interval started ([PreviewThrottle]).
    _previews.request(() {
      final staged = _staged;
      if (staged != null) arm.onPreview!(staged);
    });
  }

  /// The release: **one** commit, of the last sample the drag staged, and the
  /// tool goes away. A press on a picture nothing has been read of stages
  /// nothing, so it commits nothing and stays armed for the next attempt.
  void _released(DropperArm arm, Offset local) {
    if (!_dragging) return;
    _dragging = false;
    // A held preview would otherwise render provisional values *after* the
    // commit and put the pre-commit picture back on screen.
    _previews.cancel();
    final staged = _staged;
    _staged = null;
    if (staged == null) return;
    arm.onPick(staged);
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

/// **Every glyph on the Viewer's bars is 14** (K-456, K-466): the size the
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

/// Where the selection's name sits over the picture — the drawing's 16 from the
/// left edge of the stage and 8 down from its top.
const double viewerTagLeft = 16;
const double viewerTagTop = 8;

/// The 1px transparent edge every [HouseButton] carries so that a hover cannot
/// grow it and shuffle the row beside it. It is not drawn, but it is laid out,
/// so a mark's box is 2 wider and 2 taller than its glyph's cell — and the gaps
/// between marks, which the drawing measures between the *glyphs*, are stated
/// 2 short of the drawing's number for the same reason.
const double viewerMarkEdge = 1;

/// One mark on the Viewer's bars: the glyph at its drawn size (K-456), in a
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
/// and a tile of its own — rounded, outlined, shadowed — under Round (K-394).
BoxDecoration _stripDecoration(LumitTheme t, bool detached) => BoxDecoration(
      color: t.surface2,
      borderRadius:
          detached ? BorderRadius.circular(t.tokens.floatRadius) : null,
      border: detached ? Border.all(color: t.hairline) : null,
      boxShadow: detached ? t.tokens.cardShadow : null,
    );

/// The Viewer's **panel header strip** (K-466, §12A.6: 22 tall): the panel's
/// own kicker, then the three pickers the approved drawing puts at its right —
/// the magnification, the preview quality, and the colour pipeline.
///
/// **Why the Viewer draws its own strip.** It docks as a pane of its own rather
/// than as a tab in a group, so the dock puts no header above it; without this
/// the one panel whose drawing shows a title had none at all.
class _ViewerHeader extends StatelessWidget {
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

  const _ViewerHeader({
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
      decoration: _stripDecoration(t, detached),
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

/// The three pickers the drawing puts at the header's right-hand end, 6 apart.
///
/// A list rather than a widget because the strip they sit in is not always the
/// header: with the bars gathered into one (K-448's setting) they lead the
/// bottom bar instead, in this same order.
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

/// A tick in a menu row, the mark the guides menu already uses.
///
/// ponytail: the tick is a character, as it is in the menu bar; a drawn
/// checkmark wants a glyph of our own.
Widget menuTick(LumitTheme t, bool on) =>
    SizedBox(width: 16, child: on ? Text('✓', style: t.bodyPrimary) : null);

/// **How good the preview is** — the header's middle picker (K-466).
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
          onPressed: () => _open(context, t, ui, adaptive),
          face: dropdownFace(t, ui.previewResolution.title),
        ),
      ),
    );
  }

  void _open(
      BuildContext context, LumitTheme t, LumitUiState ui, bool adaptive) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    showMenuAt<void>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      rows: (close) => [
        _menuHeading(t, l10n.viewerQualityResolution),
        for (final resolution in PreviewResolution.values)
          MenuRow(
            key: ValueKey<String>('viewer-quality-${resolution.name}'),
            onPressed: () {
              close(null);
              ui.setPreviewResolution(resolution);
            },
            child: Row(children: [
              menuTick(t, resolution == ui.previewResolution),
              Text(resolution.title),
            ]),
          ),
        _menuHeading(t, l10n.viewerQualityPlayback),
        for (final mode in PlaybackMode.values)
          MenuRow(
            key: ValueKey<String>('viewer-playback-${mode.name}'),
            onPressed: () {
              close(null);
              ui.workspace.performance.playback = mode;
              ui.workspace.touch();
            },
            child: Row(children: [
              menuTick(
                  t,
                  mode ==
                      (adaptive
                          ? PlaybackMode.adaptive
                          : PlaybackMode.everyFrame)),
              Text(mode == PlaybackMode.adaptive
                  ? l10n.playbackAdaptiveShort
                  : l10n.playbackEveryFrame),
            ]),
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
/// (docs/07 §2.2 item 8, K-466).
///
/// It always names the display transform the picture is being shown through:
/// working space to display. With no colour config that is the one built-in
/// pair, scene-linear to sRGB (docs/06 §3.3); with one loaded the menu grows a
/// section per display the config declares, each of its views a row, and the
/// face names the view in force (K-490, docs/impl/ocio.md §6.2).
///
/// **And while either preview-only control is engaged, it says so.** Exposure
/// and the tone map live inside that same display transform (K-314) and change
/// nothing the export will ever see. The statement that *the picture is not the
/// export* belongs here, stated calmly rather than warned about (15-DESIGN) —
/// a reading you can take without leaving the picture.
///
/// **A config that is not in force is said, not hidden.** A missing or refused
/// one leaves the picture on the built-in transform (the calm half of K-490's
/// asymmetry), so the face says the config is not in force and the menu carries
/// the reason in one quiet line, in the same words the Project settings row
/// uses.
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
    // document changes and read from Dart here (K-183).
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
            menuTick(t, view == null),
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
        // words, in the config's own order, never translated (K-303).
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
                menuTick(
                    t,
                    view != null &&
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
              menuTick(t, look.toneMap),
              Text(l10n.viewerColourToneMap),
            ]),
          ),
      ],
    );
  }
}

/// The Viewer's **bottom bar** (K-466, §12A.6: 22 tall).
///
/// Left to right, and this is the drawing's own order: the ways of *looking* —
/// the transparency board, the view menu, the channel, the exposure — then a
/// hairline seam and the snapshot; the transport with its clock in the middle;
/// and at the right-hand end the composition's own reading, which says what is
/// being shown, at what time, at how many pixels, and how big.
class _ViewerBar extends StatelessWidget {
  final ViewerChannel channel;
  final bool grid;
  final bool wireframes;

  /// How the fronted comp is being looked at (K-314). Passed down rather than
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

  /// What leads the strip when the setting has gathered both bars into one
  /// (K-448): the panel's kicker and the three pickers the header would
  /// otherwise carry, in that same order. Empty in the drawing's own split,
  /// where the header carries them.
  final List<Widget> leading;

  const _ViewerBar({
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
      decoration: _stripDecoration(t, detached),
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
            // first** (§12A.6, K-451). The drawing sets the transport and the
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
                    // Nothing at all while no frame is being waited on
                    // (K-287), so at rest the reading really is the bar's
                    // right-hand end.
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
        // The exposure (K-314, docs/07 §2.2 item 12), bare: the number with
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
        // Snapshots (K-416, K-532, §2.2 item 14): **two marks**, because a
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
  /// Round gathers them into a pill (K-394, §12.1); Sharp is handed the very
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

/// **The bar's shedding ladder, and what is left at the end of it** (§12A.6,
/// K-451, and the owner's ruling on the order).
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

/// **What is on screen, in one line** (K-466): the composition, the time, the
/// pixels the engine actually made, and the magnification they are drawn at.
///
/// It is the drawing's right-hand end, and it absorbs the degradation badge
/// (docs/07 §2.2 item 9) that used to come and go beside the transport: a
/// reading that always says `1920×1080 → 960×540` states the tier plainly, in
/// the one place a person already looks to ask what they are looking at, and
/// without a box appearing mid-playback and dragging the bar about.
/// **What it sheds, and in what order** (§12A.6's ladder, K-451). The reading is
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

/// The channel picker's mark, and the menu of names behind it (K-411, K-466).
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
                      menuTick(t, c == channel),
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

/// **Show the snapshot**, the second half of the pair (K-416, K-532).
///
/// A **press and hold** puts the stored picture back over the live one for as
/// long as the button is down — the before/after read every grade leans on —
/// and releasing it is the whole of a comparison's life. Nothing crosses the
/// bridge: what is stored is what the stage's own boundary rasterised.
///
/// **Its own mark rather than a hold on Take** (K-532, superseding that half of
/// K-466). Folding both gestures onto one glyph left a taken snapshot with
/// nothing on screen to say it existed or how to see it: the only way to find
/// the comparison was to hold a button that, as far as anyone could tell, took
/// photographs. A second mark states the affordance — and states its absence,
/// by standing muted with a tooltip saying why, until one has been taken.
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
class _ViewerTag extends StatelessWidget {
  final LumitUiState uiState;
  const _ViewerTag({required this.uiState});

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

/// Which route frames take from the engine to the Viewer, in words — the
/// quality picker's tooltip, where the two playback behaviours are chosen
/// (K-466).
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

/// **The composition's background colour** (docs/07 §2.2 item 10, K-357), and
/// the picker that changes it.
///
/// **A document edit, unlike every other way of looking the Viewer offers.**
/// The exposure, the tone map and the transparency board are ways of *looking*;
/// this is what the comp is actually drawn onto and what an export writes
/// there, so it goes through an op and Ctrl+Z undoes it.
///
/// The drawing gives it no seat on the bar, so it is a row in the view menu
/// (K-466) — beside the transparency board, because the two answer the same
/// question from opposite sides and finding one without the other is what makes
/// a black comp confusing.
///
/// [background] is the colour to show, off the held read model and never asked
/// for here (K-184).
Color viewerBackgroundColour(F32Array4? background) {
  final List<double> rgba = background ?? const [0.0, 0.0, 0.0, 1.0];
  int byte(double v) => (v.clamp(0.0, 1.0) * 255).round();
  return documentColour(byte(rgba[0]), byte(rgba[1]), byte(rgba[2]), 255);
}

Future<void> showViewerBackgroundPicker({
  required BuildContext context,
  required CompositionReference comp,
  required F32Array4? background,
  required Offset position,
}) async {
  final t = ThemeScope.of(context).theme;
  final state = Provider.of<LumitState>(context, listen: false);
  await showColourPicker(
    context: context,
    position: position,
    initial: PickedColour.of(viewerBackgroundColour(background)),
    // Chosen as a display colour, like a solid's, so the fields read 0–255
    // rather than scene-linear floats.
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
}
