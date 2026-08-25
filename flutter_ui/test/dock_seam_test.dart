// The seam between two docked panes is painted by the dock, edge to edge.
//
// **The defect this pins.** A split's divider reserves a 7px hit area so a
// 1px hairline is still grabbable, but it only ever painted the hairline: the
// 3px either side were transparent, and whatever sat under the dock composited
// through them. Under Sharp that is `surface_0` — the same value the Graph
// panel grounds its canvas in — so the seam between the small Viewer and the
// Node panel of the Nodes workspace read as a slot with the graph showing
// through it (owner, desk test).
//
// Every split is the same divider widget, so the test walks both axes: a
// horizontal seam and a vertical one, sampled a pixel in from each pane's edge.

import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// The two pane fills, chosen so neither can be mistaken for a theme colour.
const Color _paneA = Color(0xFFFF00FF);
const Color _paneB = Color(0xFF00FF00);

final GlobalKey _shot = GlobalKey();

Widget _harness(DockSplit root, LumitTheme theme) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: theme,
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: RepaintBoundary(
          key: _shot,
          child: Overlay(
            initialEntries: [
              OverlayEntry(
                builder: (context) => DockWidget(
                  root: root,
                  buildPanel: (context, panel) => Container(
                    key: ValueKey<String>('pane-${panel.name}'),
                    color: panel == Panel.graph ? _paneA : _paneB,
                  ),
                  onLayoutChanged: () {},
                  activePanel: ValueNotifier<Panel?>(null),
                ),
              ),
            ],
          ),
        ),
      ),
    );

/// The rendered pixels of the whole harness, so a seam can be read off it.
Future<(ByteData, int)> _pixels(WidgetTester tester) async {
  final image = await tester.runAsync(() async {
    final boundary =
        _shot.currentContext!.findRenderObject()! as RenderRepaintBoundary;
    return boundary.toImage();
  });
  final bytes = await tester
      .runAsync(() => image!.toByteData(format: ui.ImageByteFormat.rawRgba));
  return (bytes!, image!.width);
}

Color _at(ByteData bytes, int rowStride, int x, int y) {
  final i = (y * rowStride + x) * 4;
  return Color.fromARGB(
    bytes.getUint8(i + 3),
    bytes.getUint8(i),
    bytes.getUint8(i + 1),
    bytes.getUint8(i + 2),
  );
}

void main() {
  final theme = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

  Future<void> mount(WidgetTester tester, DockSplit root) async {
    tester.view.physicalSize = const Size(800, 600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(_harness(root, theme));
    await tester.pump();
  }

  testWidgets('a horizontal seam is the dock ground for its whole width',
      (tester) async {
    await mount(
      tester,
      DockSplit(
        DockAxis.horizontal,
        [DockPane(Panel.graph), DockPane(Panel.viewer)],
        [0.5, 0.5],
      ),
    );

    final left = tester.getRect(find.byKey(const ValueKey('pane-graph')));
    final right = tester.getRect(find.byKey(const ValueKey('pane-viewer')));
    final (bytes, stride) = await _pixels(tester);
    final y = left.center.dy.round();

    // Every column strictly inside the seam. Before the fix the outer three of
    // the seven were transparent and read `surface_0` — the Graph panel's own
    // canvas value, which is what made the seam look like a hole through to the
    // graph. The one pixel against each pane is skipped: a split's shares put
    // the pane edges on fractional pixels, so that column is a blend of the
    // pane and the seam whatever the seam is painted in.
    for (var x = left.right.ceil() + 1; x < right.left.floor() - 1; x++) {
      final pixel = _at(bytes, stride, x, y);
      expect(pixel, isNot(_paneA), reason: 'x=$x is not the left pane');
      expect(pixel, isNot(_paneB), reason: 'x=$x is not the right pane');
      expect(pixel, theme.surface2,
          reason: 'x=$x is the dock ground, not whatever sits behind it');
    }
    expect(right.left - left.right, greaterThan(theme.tokens.tileGap),
        reason: 'the seam is wider than the token: the hit padding is layout');
  });

  testWidgets('a vertical seam is the dock ground for its whole height',
      (tester) async {
    // The Nodes workspace's right-hand column, which is where the owner saw it.
    await mount(
      tester,
      DockSplit(
        DockAxis.vertical,
        [DockPane(Panel.viewer), DockPane(Panel.node)],
        [0.8, 0.2],
      ),
    );

    final top = tester.getRect(find.byKey(const ValueKey('pane-viewer')));
    final bottom = tester.getRect(find.byKey(const ValueKey('pane-node')));
    final (bytes, stride) = await _pixels(tester);
    final x = top.center.dx.round();

    var sampled = 0;
    for (var y = top.bottom.ceil() + 1; y < bottom.top.floor() - 1; y++) {
      sampled++;
      expect(_at(bytes, stride, x, y), theme.surface2,
          reason: 'y=$y is the dock ground, not the pane behind it');
    }
    expect(sampled, greaterThan(1), reason: 'the seam was actually sampled');
  });
}
