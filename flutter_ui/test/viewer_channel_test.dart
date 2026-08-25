// The channel view must colour the picture and nothing else.
//
// Picking Red/Green/Blue/Alpha runs the picture through a colour matrix whose
// alpha row is a constant — that is what makes a single channel readable as
// grey rather than as a tinted ghost. A matrix like that turns transparent
// black into opaque black, and a filter that changes transparent pixels has to
// be applied to every pixel the current clip allows, not just to the ones its
// child painted. The picture is a platform texture, a composited layer the
// enclosing paint cannot bound, so without a clip of its own "everywhere" was
// the window: picking Red painted the toolbar and the side panel flat black.
//
// These render the same arrangement — a filtered picture inside a larger
// surface — and read the pixels back.

import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';

/// The surface: a green field with a 20 × 20 filtered [picture] in the middle.
Future<int Function(int, int)> _render(
  WidgetTester tester,
  ViewerChannel channel,
  Widget picture,
) async {
  final key = GlobalKey();
  await tester.pumpWidget(RepaintBoundary(
    key: key,
    child: SizedBox(
      width: 100,
      height: 100,
      child: Stack(
        textDirection: TextDirection.ltr,
        children: [
          Positioned.fill(child: Container(color: const Color(0xFF00FF00))),
          Positioned.fromRect(
            rect: const Rect.fromLTWH(40, 40, 20, 20),
            child: pictureChannelFilter(channel, picture),
          ),
        ],
      ),
    ),
  ));

  // The shot is the whole test view, not the 100 × 100 box, so its own width is
  // what the row arithmetic below has to use.
  final shot = await tester.runAsync(() async {
    final boundary =
        key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
    final image = await boundary.toImage();
    return (
      image.width,
      await image.toByteData(format: ui.ImageByteFormat.rawRgba)
    );
  });
  final width = shot!.$1;
  final bytes = shot.$2!.buffer.asUint8List();
  return (int x, int y) {
    final i = (y * width + x) * 4;
    return Color.fromARGB(bytes[i + 3], bytes[i], bytes[i + 1], bytes[i + 2])
        .toARGB32();
  };
}

void main() {
  for (final channel
      in ViewerChannel.values.where((c) => c != ViewerChannel.rgb)) {
    testWidgets('the $channel filter does not paint outside the picture',
        (tester) async {
      final at = await _render(tester, channel, const Texture(textureId: 0));
      // Two far corners, standing in for the toolbar and the side panel.
      expect(at(5, 5), 0xFF00FF00, reason: 'the surround must keep its colour');
      expect(at(700, 500), 0xFF00FF00);
    });
  }

  testWidgets('the filter still reaches the picture', (tester) async {
    // The red channel of #3366FF is 0x33, shown grey — so a clip that swallowed
    // the filter along with the escape would fail here.
    final at = await _render(
        tester, ViewerChannel.red, Container(color: const Color(0xFF3366FF)));
    expect(at(50, 50), 0xFF333333);
    expect(at(5, 5), 0xFF00FF00);
  });
}
