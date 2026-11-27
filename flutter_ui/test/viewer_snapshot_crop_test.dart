// A Viewer snapshot photographs the visible region, at full detail.
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

bool _alternates(int Function(int, int) at, int from, int to, int y) {
  for (var x = from + 1; x < to; x++) {
    if (at(x, y) == at(x - 1, y)) return false;
  }
  return true;
}

void main() {
  testWidgets('a zoomed snapshot keeps the detail a panel-sized one lost',
      (tester) async {
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

    expect(visible(19, 50), 0xFF000000);
    expect(visible(20, 50), 0xFFFFFFFF);
    expect(visible(21, 50), 0xFF000000);
    expect(_alternates(visible, 0, 100, 50), isTrue,
        reason: 'every one of the hundred stripes survives the capture');

    expect(_alternates(whole, 37, 62, 50), isFalse);
  });
}
