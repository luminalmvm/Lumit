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

// **Where the rest of it is.** The Viewer is four files, split along the seams
// it already had (K-007): this one holds the panel — the magnification, the
// pan, the snapshot slot and the transport's two calls — and hands the rest
// down. `viewer_stage.dart` is the picture and everything drawn over it,
// `viewer_strips.dart` the header strip and the vocabulary both strips are made
// of, `viewer_bar.dart` the bottom bar, and `viewer_dropper_layer.dart` the
// armed dropper. They are re-exported below, so anything that imported this
// file for a piece of the Viewer still finds it here.

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
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../shell/welcome_frb.dart' show EmptyStageFrb;
import '../state/settings.dart';
import '../state/viewer_view.dart';
import '../widgets/controls.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import 'viewer_bar.dart';
import 'viewer_stage.dart';
import 'viewer_strips.dart';
import 'viewer_zoom.dart';

// The Viewer used to be one file, and every panel, test and tool that wanted a
// piece of it imported this one. Re-exported rather than made to chase the
// split: the pieces moved, the address did not.
export 'viewer_bar.dart';
export 'viewer_dropper_layer.dart';
export 'viewer_stage.dart';
export 'viewer_strips.dart';

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

/// Which part of [picture] the [panel] around it can actually show, in the
/// picture's own coordinates (K-612).
///
/// The whole of it while the picture fits; the panel's own rectangle, slid onto
/// the picture, once magnification has taken the rest off screen. Empty when the
/// two do not meet at all — a picture panned right off the panel.
///
/// Only the offset between the two is used, because between the picture and the
/// panel there is nothing but [Positioned]: the magnification is in the
/// picture's *size*, not in a transform over it.
Rect visiblePictureCrop(RenderRepaintBoundary picture, RenderBox panel) {
  final origin = picture.localToGlobal(Offset.zero, ancestor: panel);
  return (Offset.zero & panel.size)
      .shift(-origin)
      .intersect(Offset.zero & picture.size);
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

  /// Which slice of the picture that snapshot is, as fractions of the picture's
  /// own rectangle — null while there is no snapshot. Held as fractions rather
  /// than pixels because the picture it goes back over is the one on screen
  /// *now*, at whatever magnification has happened since.
  Rect? _snapshotArea;

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

  /// Photograph the picture as it stands — the part of it on screen (K-612).
  ///
  /// At the device's own pixel ratio, so a snapshot held against the live
  /// picture is the same sharpness rather than a softer copy of it, and **never
  /// more pixels than the panel itself has**, because the bounds are the panel's
  /// and not the picture's. The boundary is the picture's rectangle, which is
  /// the *comp* at this magnification and not the panel: at 800 % on an HD comp
  /// it is 15360 pixels across (K-230's number), and photographing that whole
  /// would ask for gigabytes of pixels nobody can see, on a button that must
  /// never be a risk to press. What is off screen is what is dropped; what is on
  /// screen keeps every pixel it is drawn with.
  Future<void> _takeSnapshot() async {
    final boundary = _pictureKey.currentContext?.findRenderObject();
    final panel = context.findRenderObject();
    if (boundary is! RenderRepaintBoundary || panel is! RenderBox) return;
    // The layer rather than the render object, because only the layer takes a
    // rectangle: `RenderRepaintBoundary.toImage` is this same call with the
    // whole of the boundary passed in, and there is no other way to ask for
    // part of it. The field is protected for subclasses of [RenderObject]; the
    // cast to [OffsetLayer] is the one the framework's own `toImage` makes.
    // ignore: invalid_use_of_protected_member
    final layer = boundary.layer;
    final picture = boundary.size;
    if (layer is! OffsetLayer || !panel.hasSize || picture.isEmpty) return;
    final crop = visiblePictureCrop(boundary, panel);
    if (crop.isEmpty) return;
    final ratio = MediaQuery.devicePixelRatioOf(context);
    final dartui.Image shot;
    try {
      shot = await layer.toImage(crop, pixelRatio: ratio);
    } catch (_) {
      // A boundary that has not been painted yet has nothing to hand over.
      return;
    }
    if (!mounted) {
      shot.dispose();
      return;
    }
    final old = _snapshot;
    setState(() {
      _snapshot = shot;
      _snapshotArea = Rect.fromLTRB(
        crop.left / picture.width,
        crop.top / picture.height,
        crop.right / picture.width,
        crop.bottom / picture.height,
      );
    });
    // After the frame, not during the swap: the old image may be on screen this
    // very instant (Show held while Take is pressed), and disposing an image a
    // live [RawImage] still points at is a crash rather than a saving.
    WidgetsBinding.instance.addPostFrameCallback((_) => old?.dispose());
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
        ? ViewerHeader(
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
          builder: (context, tier, _) => ViewerBar(
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
            builder: (context, frame, _) => ViewerStage(
              comp: comp,
              uiState: ui,
              fitted: fitted,
              grid: ui.viewerGrid,
              overlays: ui.viewerOverlays,
              pictureKey: _pictureKey,
              snapshot: _showingSnapshot ? _snapshot : null,
              snapshotArea: _snapshotArea,
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
