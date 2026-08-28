// What the Graph panel's ground costs to draw, at every zoom it offers.
//
// **Why this file exists.** The dots on the canvas ground sit on the canvas's
// own grid, so zooming out used to pull more of them into view: at the smallest
// zoom the panel offers, a 900x600 canvas drew 15,000 dots against 1,350 at
// 100%, one `drawCircle` apiece, on every single repaint — and the ground
// repainted with the wires, which is every pointer move of a pan, a hover or a
// wire drag. That is the whole of the owner's "laggier and laggier as you zoom
// out".
//
// The two rules pinned below: **the dot count does not grow as the canvas is
// zoomed out** — the grid skips every other line each time the zoom halves, so
// the spacing on screen stays put — and **the ground does not repaint for
// anything but a pan, a zoom or a theme change**. Each is paired with the guard
// that stops it being satisfied by a ground that has quietly stopped drawing:
// the dots must still be there, and a real zoom must still redraw them.

import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';

import 'frb_test_support.dart';

/// A canvas that draws nothing and counts the dots it was asked for, in both
/// the shapes a dot grid could plausibly take.
class _DotCount implements Canvas {
  int dots = 0;
  int calls = 0;

  @override
  void drawCircle(Offset c, double radius, Paint paint) {
    dots++;
    calls++;
  }

  @override
  void drawRawPoints(ui.PointMode mode, Float32List points, Paint paint) {
    dots += points.length ~/ 2;
    calls++;
  }

  @override
  void drawPoints(ui.PointMode mode, List<Offset> points, Paint paint) {
    dots += points.length;
    calls++;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  group('Graph ground budget (frb)', () {
    const size = Size(900, 600);

    Future<({LumitState state, LumitUiState uiState})> mount(
        WidgetTester tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const GraphPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: size,
      ));
      await tester.pump();
      return p;
    }

    CustomPainter ground(WidgetTester tester) => tester
        .widget<CustomPaint>(find.byKey(const ValueKey('graph-ground')))
        .painter!;

    /// Count the ground's dots as it stands.
    _DotCount count(WidgetTester tester) {
      final counter = _DotCount();
      ground(tester).paint(counter, size);
      return counter;
    }

    /// Zoom the canvas out one wheel notch at a time. The panel's own handler
    /// clamps at 0.2, so twenty notches lands on the floor whatever it starts
    /// from.
    Future<void> zoomOut(WidgetTester tester, int notches) async {
      final centre = tester.getCenter(find.byKey(const ValueKey('graph-canvas')));
      for (var i = 0; i < notches; i++) {
        await tester.sendEventToBinding(PointerScrollEvent(
          position: centre,
          scrollDelta: const Offset(0, 40),
        ));
      }
      await tester.pump();
    }

    testWidgets('the dot count holds as the canvas zooms out', (tester) async {
      await mount(tester);
      final atFull = count(tester);
      // The guard: a ground that draws nothing would pass every budget below.
      expect(atFull.dots, greaterThan(500),
          reason: 'a 900x600 canvas at 100% is a 45x30 grid of dots');
      expect(atFull.calls, 1,
          reason: 'the whole grid is one call, not one per dot');

      await zoomOut(tester, 20);
      expect(find.text('20%'), findsOneWidget,
          reason: 'the wheel really did drive the zoom to its floor');

      final atFloor = count(tester);
      expect(atFloor.dots, lessThanOrEqualTo(atFull.dots),
          reason: 'zooming out thins the grid; it never multiplies it');
      // The guard again, at the far end: the old code dropped the grid entirely
      // below a 6px pitch, which is exactly where this zoom lands.
      expect(atFloor.dots, greaterThan(500),
          reason: 'the ground still has its grid at the smallest zoom');
      expect(atFloor.calls, 1);
    });

    testWidgets('the ground repaints for the zoom and not for a hover',
        (tester) async {
      await mount(tester);
      final before = ground(tester);

      // A hover over the canvas rebuilds the panel and redraws every wire.
      final centre = tester.getCenter(find.byKey(const ValueKey('graph-canvas')));
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(pointer.hover(centre));
      await tester.sendEventToBinding(pointer.hover(centre + const Offset(9, 9)));
      await tester.pump();
      expect(ground(tester).shouldRepaint(before), isFalse,
          reason: 'the ground answers to the pan, the zoom and the theme only');

      // The guard: it must still repaint for something that actually moves it.
      await zoomOut(tester, 1);
      expect(ground(tester).shouldRepaint(before), isTrue,
          reason: 'a zoom moves the grid, so the ground redraws');
    });
  });
}
