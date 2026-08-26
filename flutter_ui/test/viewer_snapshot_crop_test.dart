// A Viewer snapshot photographs the visible region, at full detail (K-612).
//
// The boundary a snapshot is taken from is the picture's rectangle, which at
// high magnification is the *comp* and not the panel — an HD comp at 400 % is
// 7680 logical pixels across. Photographing that whole at the device's own
// ratio asks for a few hundred million pixels, so the capture used to be scaled
// down to the panel's own resolution instead: safe, and softer than the live
// picture it is meant to be held against.
//
// The bound moves rather than the number. The bounds handed to the layer are
// what the panel can actually show, so a snapshot is still never more pixels
// than the panel has, and every one of them is at the sharpness the picture is
// drawn with.
//
// These mount the same arrangement the stage builds — a large picture inside a
// small panel — and read the pixels back.

import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';

/// One-pixel vertical stripes: detail exactly as fine as a logical pixel, so a
/// capture at anything under 1:1 has to average it away.
class _Stripes extends CustomPainter {
  const _Stripes();

  @override
  void paint(Canvas canvas, Size size) {
    final brush = Paint()..isAntiAlias = false;
    for (var x = 0; x < size.width; x++) {
      brush.color = x.isEven ? const Color(0xFFFFFFFF) : const Color(0xFF000000);
      canvas.drawRect(Rect.fromLTWH(x.toDouble(), 0, 1, size.height), brush);
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}

/// A [picture] of the given size, offset by [at] inside a 100 × 100 panel —
/// which is the stage: the magnification is in the picture's size, and the pan
/// is in its offset.
Future<({RenderRepaintBoundary picture, RenderBox panel})> _mount(
  WidgetTester tester, {
  required Size picture,
  required Offset at,
}) async {
  final panelKey = GlobalKey();
  final pictureKey = GlobalKey();
  await tester.pumpWidget(Center(
    child: SizedBox(
      key: panelKey,
      width: 100,
      height: 100,
      child: Stack(
        clipBehavior: Clip.none,
        textDirection: TextDirection.ltr,
        children: [
          Positioned(
            left: at.dx,
            top: at.dy,
            width: picture.width,
            height: picture.height,
            child: RepaintBoundary(
              key: pictureKey,
              child: const CustomPaint(painter: _Stripes()),
            ),
          ),
        ],
      ),
    ),
  ));
  return (
    picture: pictureKey.currentContext!.findRenderObject()!
        as RenderRepaintBoundary,
    panel: panelKey.currentContext!.findRenderObject()! as RenderBox,
  );
}

/// Read [image] back as a colour per pixel.
Future<int Function(int, int)> _pixels(ui.Image image) async {
  final data = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
  final bytes = data!.buffer.asUint8List();
  final width = image.width;
  return (int x, int y) {
    final i = (y * width + x) * 4;
    return Color.fromARGB(bytes[i + 3], bytes[i], bytes[i + 1], bytes[i + 2])
        .toARGB32();
  };
}

/// Whether every neighbouring pixel along row [y], from [from] to [to], differs
/// from the last — which is what a row of one-pixel stripes looks like when
/// nothing has been averaged away.
bool _alternates(int Function(int, int) at, int from, int to, int y) {
  for (var x = from + 1; x < to; x++) {
    if (at(x, y) == at(x - 1, y)) return false;
  }
  return true;
}

void main() {
  testWidgets('a zoomed snapshot keeps the detail a panel-sized one lost',
      (tester) async {
    // The picture is four times the panel and panned so its middle is on
    // screen: 400 % with the corners off, which is the case that used to cost
    // the detail.
    final stage = await _mount(
      tester,
      picture: const Size(400, 400),
      at: const Offset(-150, -150),
    );

    final crop = visiblePictureCrop(stage.picture, stage.panel);
    expect(crop, const Rect.fromLTWH(150, 150, 100, 100),
        reason: 'the visible region is the panel, slid onto the picture');

    final shots = await tester.runAsync(() async {
      final layer = stage.picture.debugLayer! as OffsetLayer;
      final visible = await layer.toImage(crop, pixelRatio: 1);
      // What the panel-sized capture did: the whole picture, scaled down until
      // it fits the panel's own resolution.
      final whole = await stage.picture.toImage(pixelRatio: 100 / 400);
      return (
        visible: (visible.width, await _pixels(visible)),
        whole: (whole.width, await _pixels(whole)),
      );
    });

    final (visibleWidth, visible) = shots!.visible;
    final (wholeWidth, whole) = shots.whole;
    expect(visibleWidth, 100, reason: 'still only the panel worth of pixels');
    expect(wholeWidth, 100, reason: 'and the same count as the old capture');

    // Column 20 of the crop is logical x = 170, an even column and so white,
    // with black either side of it. At full detail those are the three colours
    // read back, and every stripe across the row is there.
    expect(visible(19, 50), 0xFF000000);
    expect(visible(20, 50), 0xFFFFFFFF);
    expect(visible(21, 50), 0xFF000000);
    expect(_alternates(visible, 0, 100, 50), isTrue,
        reason: 'every one of the hundred stripes survives the capture');

    // The same region of the picture in the old capture: columns 37 to 62,
    // twenty-five pixels standing for a hundred logical ones. Whatever the
    // rasteriser makes of that, the stripes are not in it — this is the
    // softening the crop does away with.
    expect(_alternates(whole, 37, 62, 50), isFalse);
  });

  testWidgets('a snapshot of a picture that fits is the whole of it, unchanged',
      (tester) async {
    final stage = await _mount(
      tester,
      picture: const Size(100, 100),
      at: Offset.zero,
    );

    final crop = visiblePictureCrop(stage.picture, stage.panel);
    expect(crop, const Rect.fromLTWH(0, 0, 100, 100),
        reason: 'nothing is off screen, so nothing is cropped away');

    final shot = await tester.runAsync(() async {
      final layer = stage.picture.debugLayer! as OffsetLayer;
      final image = await layer.toImage(crop, pixelRatio: 1);
      return (image.width, image.height, await _pixels(image));
    });
    expect(shot!.$1, 100);
    expect(shot.$2, 100);
    // Corner to corner, the picture itself: the fitted case takes the same
    // photograph it always did.
    expect(shot.$3(0, 0), 0xFFFFFFFF);
    expect(shot.$3(99, 99), 0xFF000000);
  });

  testWidgets('a picture panned right off the panel crops to nothing',
      (tester) async {
    final stage = await _mount(
      tester,
      picture: const Size(400, 400),
      at: const Offset(-500, 0),
    );
    expect(visiblePictureCrop(stage.picture, stage.panel).isEmpty, isTrue,
        reason: 'and Take stands down rather than asking for an empty image');
  });
}
