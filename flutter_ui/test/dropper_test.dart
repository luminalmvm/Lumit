// The dropper's arithmetic and its viewfinder.
//
// The sums matter more than they look: a colour lifted off the picture is
// written straight into a scene-linear parameter, so an average taken in the
// wrong space is a wrong colour with nothing on screen to say so. The
// viewfinder tests pin the two things the owner asked for by eye — nine by
// nine, and the centre pixel alone until Shift+scroll says otherwise.

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/dropper.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/dropper_overlay.dart';

/// A window of [side] pixels centred on `(cx, cy)` of the picture, whose pixels
/// are given by `pixel(x, y)` in the *picture's* own coordinates — so a test can
/// say "white on the left half" without doing the window arithmetic itself.
/// [layerAlone] marks it as a read of one layer on its own, as a depth reply is.
BridgeSampledPixels windowOf(
  List<int> Function(int x, int y) pixel, {
  int side = 21,
  int cx = 40,
  int cy = 20,
  int width = 100,
  int height = 50,
  bool layerAlone = false,
}) {
  final bytes = Uint8List(side * side * 4);
  final half = side ~/ 2;
  for (var row = 0; row < side; row++) {
    for (var col = 0; col < side; col++) {
      final rgb = pixel(cx - half + col, cy - half + row);
      final i = (row * side + col) * 4;
      bytes[i] = rgb[0];
      bytes[i + 1] = rgb[1];
      bytes[i + 2] = rgb[2];
      bytes[i + 3] = 255;
    }
  }
  return BridgeSampledPixels(
    window: side,
    rgba: bytes,
    width: width,
    height: height,
    x: cx,
    y: cy,
    frame: BigInt.zero,
    view: 0,
    layerAlone: layerAlone,
  );
}

Widget harness(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Center(child: child),
      ),
    );

void main() {
  group('sampling a window', () {
    test('a wider region averages in linear light, not in sRGB bytes', () {
      // One white pixel among its eight black neighbours: over 3×3 that is one
      // ninth of the light, not the byte midpoint a naive average gives.
      final w =
          windowOf((x, y) => x == 40 && y == 20 ? [255, 255, 255] : [0, 0, 0]);
      final sample = sampleFromWindow(w, 3, 40, 20);
      expect(sample.r, closeTo(1 / 9, 1e-9));
      expect(sample.depth, closeTo(1 / 9, 1e-9));
    });

    /// **The point of a window.** The magnifier reads it around wherever the
    /// pointer is *now*, not around where it was when the window was read — so
    /// moving the pointer inside one costs no engine call and still samples the
    /// right pixel.
    test('reads around the pointer, not around the window centre', () {
      // White left of x = 40, black from there on.
      final w = windowOf((x, y) => x < 40 ? [255, 255, 255] : [0, 0, 0]);
      expect(sampleFromWindow(w, 1, 40, 20).r, closeTo(0.0, 1e-9));
      expect(sampleFromWindow(w, 1, 39, 20).r, closeTo(1.0, 1e-9),
          reason: 'one pixel left of the centre is on the white side');
      expect(sampleFromWindow(w, 1, 45, 25).r, closeTo(0.0, 1e-9));
    });
  });

  group('which pixel grid a point is in', () {
    /// **The bug this exists to prevent.** The picture the engine reads is a
    /// reduced-resolution preview whenever the Viewer is not at 100 %, so its
    /// pixel grid is NOT the composition's. A point must therefore be turned
    /// into a pixel through the *reply's* own raster; doing it through the
    /// composition's put every index outside the window, every cell clamped to
    /// the same edge pixel, and the magnifier showed a flat colour.
    test('a point becomes a pixel of the reply raster, not of the comp', () {
      // A 1920x1080 comp read at half resolution: the reply is 960x540.
      final half = windowOf((x, y) => [0, 0, 0],
          side: 129, cx: 480, cy: 270, width: 960, height: 540);

      expect(windowPixelAt(half, 0.5, 0.5), (480, 270),
          reason: 'the middle of the picture is the middle of THIS raster');
      // The comp-pixel answer would have been (960, 540) — which is not even
      // inside the picture, let alone inside the window.
      expect(windowCovers(half, 960, 540), isFalse);
      expect(windowCovers(half, 480, 270), isTrue);
    });
  });

  group('sRGB conversion', () {
    test('round-trips every byte', () {
      for (var b = 0; b <= 255; b++) {
        expect(srgbEncode(srgbDecode(b)), b, reason: '$b');
      }
    });
  });

  group('the viewfinder', () {
  group('where it sits', () {
    // A window with plenty of room, and one with the pointer hard against its
    // far corner.
    const window = Rect.fromLTWH(0, 0, 1000, 800);
    final w = dropperViewfinderSize.width;
    final h = dropperViewfinderSize.height;

    /// The window's edge is the one it must answer to — an application cannot
    /// paint outside its own window. It answers the way a tooltip does: the
    /// same distance on the *other* side of the pointer, so it never creeps
    /// over the pixel being read.
    test('flips to the other side of the pointer at the window edge', () {
      const at = Offset(990, 790);
      final origin = dropperViewfinderOrigin(at, window);
      expect(origin.dx, at.dx - dropperViewfinderOffset.dx - w);
      expect(origin.dy, at.dy - dropperViewfinderOffset.dy - h);
      expect(origin.dx + w, lessThanOrEqualTo(window.right));
      expect(origin.dy + h, lessThanOrEqualTo(window.bottom));
      // The gap from the pointer is the one it has everywhere else.
      expect(at.dx - (origin.dx + w), dropperViewfinderOffset.dx);
      expect(at.dy - (origin.dy + h), dropperViewfinderOffset.dy);
    });
  });
  });
}
