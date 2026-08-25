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
  group('the sample sizes', () {
    test('start at one pixel and step 1, 3, 5, 7, 9 without wrapping', () {
      expect(dropperRegions.first, 1, reason: 'the centre pixel alone');
      expect(dropperRegions, [1, 3, 5, 7, 9]);
      expect(nextDropperRegion(1, 1), 3);
      expect(nextDropperRegion(3, 1), 5);
      expect(nextDropperRegion(9, 1), 9, reason: 'the top holds, never wraps');
      expect(nextDropperRegion(1, -1), 1, reason: 'and so does the bottom');
      expect(nextDropperRegion(5, -1), 3);
    });

    test('every size is odd, so there is always one centre pixel', () {
      for (final n in dropperRegions) {
        expect(n.isOdd, isTrue, reason: '$n');
        expect(n <= dropperGrid, isTrue, reason: 'never wider than the grid');
      }
    });
  });

  group('sampling a window', () {
    test('one pixel is that pixel, decoded to scene-linear', () {
      // Pure red at (40, 20), black everywhere else.
      final w = windowOf((x, y) => x == 40 && y == 20 ? [255, 0, 0] : [0, 0, 0]);
      final sample = sampleFromWindow(w, 1, 40, 20);
      expect(sample.r, closeTo(1.0, 1e-9), reason: 'sRGB 255 is linear 1.0');
      expect(sample.g, closeTo(0.0, 1e-9));
      expect(sample.b, closeTo(0.0, 1e-9));
      expect(sample.region, 1);
      expect([sample.x, sample.y], [40, 20], reason: 'the pixel it came from');
    });

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

    test('a region wider than the magnifier is clamped rather than taken', () {
      final w = windowOf((x, y) => [255, 255, 255]);
      final sample = sampleFromWindow(w, 99, 40, 20);
      expect(sample.region, dropperGrid);
      expect(sample.r, closeTo(1.0, 1e-9));
    });

    /// **The flat-magnifier regression.** A position outside the window is
    /// answered with nothing, not with the nearest edge pixel. The window
    /// already carries the picture's own edge repeats, so a pixel past the
    /// picture's border is *inside* it and answers normally; a position outside
    /// the window means the caller is asking in the wrong grid, and clamping
    /// answered that with a plausible colour — which is how a whole magnifier
    /// of one repeated pixel, looking like a flat average, went unnoticed.
    test('a position outside the window answers nothing, not its edge', () {
      final w = windowOf((x, y) => [10, 20, 30], side: 11);
      expect(windowPixel(w, 4000, 4000), isNull);
      expect(windowPixel(w, 40, 20), isNotNull, reason: 'the centre is in it');
      expect(windowPixel(w, 45, 25), isNotNull, reason: 'and so is its corner');
      expect(windowPixel(w, 46, 20), isNull, reason: 'one past it is not');
    });

    test('depth is Rec. 709 luma in linear light', () {
      expect(sampleFromWindow(windowOf((x, y) => [0, 255, 0]), 1, 40, 20).depth,
          closeTo(0.7152, 1e-6));
      expect(
          sampleFromWindow(windowOf((x, y) => [0, 0, 0]), 1, 40, 20).depth, 0);
    });
  });

  group('when a window has to be re-read', () {
    // 21 a side centred on (40, 20): it covers the magnifier's grid anywhere
    // within ten pixels of that centre, less the four the grid itself reaches.
    final w = windowOf((x, y) => [0, 0, 0]);

    test('covers the pointer while the whole grid still fits inside it', () {
      expect(windowCovers(w, 40, 20), isTrue, reason: 'dead centre');
      expect(windowCovers(w, 46, 26), isTrue);
      expect(windowCovers(w, 34, 14), isTrue, reason: 'the far corner, just');
    });

    test('stops covering once the grid would reach past its edge', () {
      expect(windowCovers(w, 47, 20), isFalse);
      expect(windowCovers(w, 40, 33), isFalse);
    });

    /// The whole point of the size: a pointer can travel most of a window
    /// before another read is needed, so a sweep across the picture is a
    /// handful of reads rather than one per mouse move.
    test('a full-size window lasts a long pointer travel', () {
      final full = windowOf((x, y) => [0, 0, 0], side: dropperWindow);
      final reach = dropperWindow ~/ 2 - dropperGrid ~/ 2;
      expect(reach, greaterThan(50));
      expect(windowCovers(full, 40 + reach, 20), isTrue);
      expect(windowCovers(full, 40 + reach + 1, 20), isFalse);
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

    test('the ends of the picture are its first and last pixel', () {
      final w = windowOf((x, y) => [0, 0, 0], width: 960, height: 540);
      expect(windowPixelAt(w, 0, 0), (0, 0));
      expect(windowPixelAt(w, 1, 1), (959, 539), reason: 'never one past the end');
      expect(windowPixelAt(w, 2, -1), (959, 0), reason: 'and clamped either way');
    });

    /// With the grids agreeing again, the magnifier shows nine *different*
    /// pixels rather than one repeated: the whole complaint.
    test('the magnifier reads nine distinct pixels across the grid', () {
      // A vertical stripe every other pixel, in a half-resolution raster.
      final w = windowOf((x, y) => x.isEven ? [255, 255, 255] : [0, 0, 0],
          side: 129, cx: 480, cy: 270, width: 960, height: 540);
      final (cx, cy) = windowPixelAt(w, 0.5, 0.5);
      final row = [
        for (var dx = -4; dx <= 4; dx++) windowPixel(w, cx + dx, cy)?.r,
      ];
      expect(row.contains(null), isFalse, reason: 'every cell has a pixel');
      expect(row.toSet().length, 2, reason: 'stripes, not one flat colour');
      expect(row.first, isNot(row[1]), reason: 'neighbours differ');
    });
  });

  group('sRGB conversion', () {
    test('round-trips every byte', () {
      for (var b = 0; b <= 255; b++) {
        expect(srgbEncode(srgbDecode(b)), b, reason: '$b');
      }
    });

    test('the ends are exact and the middle is not the byte midpoint', () {
      expect(srgbDecode(0), 0);
      expect(srgbDecode(255), closeTo(1.0, 1e-9));
      expect(srgbDecode(128), lessThan(0.25),
          reason: 'mid-grey is about a fifth of the light, not half');
    });
  });

  group('the viewfinder', () {
    testWidgets('shows the colour and its numbers for a colour pick',
        (tester) async {
      final arm = DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: (_) {},
      );
      await tester.pumpWidget(harness(DropperViewfinder(
        arm: arm,
        window: windowOf((x, y) => [255, 128, 0]),
        centre: (40, 20),
        region: 1,
      )));
      expect(find.text('255 128 0'), findsOneWidget);
      expect(find.text('1×1'), findsOneWidget);
    });

    testWidgets('names the layer it is reading for a pick that is not a colour',
        (tester) async {
      final arm = DropperArm(
        id: 'test',
        reads: DropperReads.depth,
        label: 'Focus distance',
        sampleLayerName: 'Depth pass',
        onPick: (_) {},
      );
      await tester.pumpWidget(harness(DropperViewfinder(
        arm: arm,
        window: windowOf((x, y) => [255, 255, 255], layerAlone: true),
        centre: (40, 20),
        region: 3,
      )));
      // The layer the numbers come from, and the value — no colour swatch,
      // because no colour is being chosen.
      expect(find.textContaining('Depth pass'), findsOneWidget);
      expect(find.textContaining('1.000'), findsOneWidget);
      expect(find.text('3×3'), findsOneWidget);
    });

    testWidgets('says Composite when the pixels are not of that layer alone',
        (tester) async {
      final arm = DropperArm(
        id: 'test',
        reads: DropperReads.depth,
        label: 'Focus distance',
        sampleLayerName: 'Depth pass',
        onPick: (_) {},
      );
      await tester.pumpWidget(harness(DropperViewfinder(
        arm: arm,
        // layerAlone false: the reply is of the composite, so naming the layer
        // would claim the number came from somewhere it did not.
        window: windowOf((x, y) => [0, 0, 0]),
        centre: (40, 20),
        region: 1,
      )));
      expect(find.textContaining('Composite'), findsOneWidget);
      expect(find.textContaining('Depth pass'), findsNothing);
    });

    testWidgets('says so when the pointer has outrun the window it holds',
        (tester) async {
      final arm = DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: (_) {},
      );
      await tester.pumpWidget(harness(DropperViewfinder(
        arm: arm,
        window: windowOf((x, y) => [255, 128, 0], side: 11),
        // Well outside the 11-wide window centred on (40, 20): a value read
        // from the cells that happen to be inside it would be a lie.
        centre: (90, 20),
        region: 1,
      )));
      expect(find.text('Reading…'), findsOneWidget);
      expect(find.text('255 128 0'), findsNothing);
    });

    testWidgets('says so while the first read is still in flight',
        (tester) async {
      final arm = DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: (_) {},
      );
      await tester.pumpWidget(harness(DropperViewfinder(
          arm: arm, window: null, centre: (0, 0), region: 1)));
      expect(find.text('Reading…'), findsOneWidget);
    });

  group('where it sits', () {
    // A window with plenty of room, and one with the pointer hard against its
    // far corner.
    const window = Rect.fromLTWH(0, 0, 1000, 800);
    final w = dropperViewfinderSize.width;
    final h = dropperViewfinderSize.height;

    /// **The corner regression.** The viewfinder used to be pulled back to stay
    /// inside the *Viewer*, so near the bottom-right corner it crept over the
    /// very pixels being aimed at and stopped following the pointer. It is
    /// drawn in the application's overlay now, so a panel edge means nothing
    /// to it: the offset is the same wherever the pointer is on the picture.
    test('keeps the same offset from the pointer with room to spare', () {
      expect(dropperViewfinderOrigin(const Offset(100, 100), window),
          const Offset(100, 100) + dropperViewfinderOffset);
      expect(dropperViewfinderOrigin(const Offset(700, 500), window),
          const Offset(700, 500) + dropperViewfinderOffset);
      expect(dropperViewfinderOrigin(Offset.zero, window),
          dropperViewfinderOffset);
    });

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

    test('flips each axis on its own, not both together', () {
      // Hard against the bottom, plenty of room to the right.
      final low = dropperViewfinderOrigin(const Offset(100, 790), window);
      expect(low.dx, 100 + dropperViewfinderOffset.dx, reason: 'still right');
      expect(low.dy, 790 - dropperViewfinderOffset.dy - h, reason: 'now above');

      // Hard against the right, plenty of room below.
      final right = dropperViewfinderOrigin(const Offset(990, 100), window);
      expect(right.dx, 990 - dropperViewfinderOffset.dx - w);
      expect(right.dy, 100 + dropperViewfinderOffset.dy);
    });

    test('a window with room for neither side shows what it can', () {
      const tiny = Rect.fromLTWH(0, 0, 60, 60);
      final origin = dropperViewfinderOrigin(const Offset(30, 30), tiny);
      expect(origin, Offset.zero,
          reason: 'a magnifier half on screen beats none at all');
    });
  });
  });
}
