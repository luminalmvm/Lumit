// The colour picker's eyedropper: a colour lifted off anything in the window
// by its pixels on screen.
//
// **In plain terms.** Press the pipette in a colour picker and the pointer
// becomes a crosshair over the whole of Lumit: the Viewer, a swatch in a panel,
// a thumbnail, the picker itself. A little tag beside the pointer shows the
// colour under it. A click takes that colour; a right-click or Escape puts the
// pipette away and changes nothing.
//
// **Why the screen and not the engine.** The Viewer's own dropper (an effect
// colour row's, state/dropper.dart) reads the composition through the engine,
// and only over the Viewer. A picker is often open where that one cannot
// reach: over a dialogue such as New Solid, or wanting a colour that is only
// on a panel. Reading the window's own picture works everywhere a colour can
// be seen, and what it reads is exactly the colour seen.

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart' show kSecondaryMouseButton;
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../theme/theme.dart';
import 'colour_picker.dart' show formatHex;
import 'controls.dart';
import 'escape_ladder.dart';

/// The boundary around the whole window's picture, popups and dialogues
/// included. The shell puts it on its outermost repaint boundary.
final GlobalKey windowPictureKey = GlobalKey(debugLabel: 'window-picture');

/// The colour of the window's pixel under [global], a window position, or
/// null when there is no picture to read there.
Future<Color?> readWindowPixel(Offset global, double pixelRatio) async {
  final boundary = windowPictureKey.currentContext?.findRenderObject();
  if (boundary is! RenderRepaintBoundary || !boundary.hasSize) return null;
  // The layer rather than the render object, because only the layer takes a
  // rectangle (the Viewer snapshot's note, viewer_panel_frb.dart).
  // ignore: invalid_use_of_protected_member
  final layer = boundary.layer;
  if (layer is! OffsetLayer) return null;
  final local = boundary.globalToLocal(global);
  if (!(Offset.zero & boundary.size).contains(local)) return null;
  // One device pixel: a logical pixel is several of them on a scaled display.
  final side = 1 / pixelRatio;
  ui.Image? image;
  try {
    image = await layer.toImage(Rect.fromLTWH(local.dx, local.dy, side, side),
        pixelRatio: pixelRatio);
    final bytes = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
    if (bytes == null || bytes.lengthInBytes < 4) return null;
    final a = bytes.getUint8(3);
    // Premultiplied, though the window is opaque wherever anything is drawn.
    int un(int c) => a == 0 ? 0 : (c * 255 / a).round().clamp(0, 255);
    return documentColour(un(bytes.getUint8(0)), un(bytes.getUint8(1)),
        un(bytes.getUint8(2)), 0xff);
  } catch (_) {
    // A boundary not painted yet, or a driver that will not read back.
    return null;
  } finally {
    image?.dispose();
  }
}

/// Arm the eyedropper over the whole window, and complete with the colour
/// clicked, or null when it was put away.
Future<Color?> pickColourOnScreen(BuildContext context) {
  final overlay = Overlay.of(context);
  final completer = Completer<Color?>();
  late OverlayEntry entry;
  void finish(Color? colour) {
    if (completer.isCompleted) return;
    completer.complete(colour);
    entry.remove();
  }

  // Over everything, popups and dialogues included, so the click is the
  // pipette's and never the click-away of the picker it came from.
  entry = OverlayEntry(builder: (_) => _ScreenDropper(onDone: finish));
  overlay.insert(entry);
  return completer.future;
}

class _ScreenDropper extends StatefulWidget {
  final void Function(Color? colour) onDone;
  const _ScreenDropper({required this.onDone});

  @override
  State<_ScreenDropper> createState() => _ScreenDropperState();
}

class _ScreenDropperState extends State<_ScreenDropper> {
  /// Where the pointer is, in this overlay's own coordinates, for the tag.
  Offset? _at;

  /// The colour under the pointer, as last read.
  Color? _under;

  /// A read is in flight; [_next] is where the pointer went meanwhile.
  bool _reading = false;
  Offset? _next;

  VoidCallback? _escapeRelease;

  @override
  void initState() {
    super.initState();
    // The gesture rung, above the popup's, so Escape puts the pipette away
    // and leaves the picker open.
    _escapeRelease = EscapeLadder.register(EscapeRung.gesture, () {
      widget.onDone(null);
      return true;
    });
  }

  @override
  void dispose() {
    _escapeRelease?.call();
    super.dispose();
  }

  double get _ratio => View.of(context).devicePixelRatio;

  /// Read the pixel under [global] for the tag, one read at a time: a move
  /// that arrives during a read is read next, and any in between are skipped.
  Future<void> _hover(Offset global) async {
    if (_reading) {
      _next = global;
      return;
    }
    _reading = true;
    final colour = await readWindowPixel(global, _ratio);
    _reading = false;
    if (!mounted) return;
    setState(() => _under = colour);
    final next = _next;
    _next = null;
    if (next != null) unawaited(_hover(next));
  }

  Future<void> _pick(Offset global) async {
    final colour = await readWindowPixel(global, _ratio);
    widget.onDone(colour ?? _under);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final at = _at;
    final colour = _under;
    return Stack(
      children: [
        Positioned.fill(
          child: MouseRegion(
            cursor: SystemMouseCursors.precise,
            child: Listener(
              behavior: HitTestBehavior.opaque,
              onPointerHover: (e) {
                setState(() => _at = e.localPosition);
                _hover(e.position);
              },
              onPointerDown: (e) {
                if (e.buttons == kSecondaryMouseButton) {
                  widget.onDone(null);
                  return;
                }
                _pick(e.position);
              },
              child: const SizedBox.expand(),
            ),
          ),
        ),
        // The colour under the pointer, beside it rather than under it, so it
        // never covers the pixel being read.
        if (at != null && colour != null)
          Positioned(
            left: at.dx + 18,
            top: at.dy + 18,
            child: IgnorePointer(
              child: Container(
                padding: const EdgeInsets.all(4),
                decoration: BoxDecoration(
                  color: t.surface3,
                  borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                  border: Border.all(color: t.hairlineStrong),
                  boxShadow: t.tokens.cardShadow,
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Container(
                      width: 18,
                      height: 18,
                      decoration: BoxDecoration(
                        color: colour,
                        borderRadius:
                            BorderRadius.circular(t.tokens.controlRadius),
                        border: Border.all(color: t.hairlineStrong),
                      ),
                    ),
                    const SizedBox(width: 6),
                    Text('#${formatHex(colour)}',
                        style: t.mono.copyWith(color: t.textSecondary)),
                  ],
                ),
              ),
            ),
          ),
      ],
    );
  }
}
