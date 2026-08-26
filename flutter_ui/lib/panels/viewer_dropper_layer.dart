// The Viewer's armed dropper: the magnifier over the picture, the sample-size
// wheel, and the drag that picks a colour or a point.
//
// Split out of viewer_panel_frb.dart, which had grown past the length anyone
// can hold in their head (K-007). Nothing here changed in the move: it is the
// same widget, with the same state, called from the same one place — the
// Viewer's stage, which is the only thing that knows where the picture is.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';

import '../state/dropper.dart';
import '../state/preview_throttle.dart';
import '../widgets/dropper_overlay.dart';
import '../widgets/escape_ladder.dart';

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
