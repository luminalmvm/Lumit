// The Timeline under its three shapes: what each one draws differently, as
// geometry a user could point at. Studio's own numbers are pinned by
// timeline_alignment_test; this file is for what Desk and Lantern change.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_zoom_dial.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Timeline shapes (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p, ThemeShape shape) async {
      const size = Size(1280, 600);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
        shape: shape,
        density: DensityTokens.forShape(shape, false),
      ));
      await tester.pump();
    }

    String idOf(LayerReference l) => l.internallayerId.toString();

    Rect laneBar(WidgetTester tester, LayerReference l) =>
        tester.getRect(find.byKey(ValueKey<String>('tl-bar-${idOf(l)}')));

    /// Ctrl and the wheel zoom time, so the lanes have somewhere to scroll.
    Future<void> zoomIn(WidgetTester tester, Offset at) async {
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendEventToBinding(pointer.hover(at));
      for (var i = 0; i < 6; i++) {
        await tester.sendEventToBinding(pointer.scroll(const Offset(0, -120)));
        await tester.pump();
      }
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
    }

    /// Desk's zoom is a dial with its reading beside it, and turning it
    /// writes the same zoom the slider does.
    testWidgets('Desk turns the zoom on a dial and reads a tick in frames',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p, ThemeShape.desk);

      expect(find.byKey(const ValueKey('tl-zoom-slider')), findsNothing);
      final dial = find.byKey(const ValueKey('tl-zoom-dial'));
      expect(dial, findsOneWidget);
      expect(tester.getSize(dial), const Size.square(TimelineZoomDial.size));
      final readout =
          tester.widget<Text>(find.byKey(const ValueKey('tl-zoom-readout')));
      expect(readout.data, matches(RegExp(r'^1 tick = \d+ f$')),
          reason: 'the reading says what one ruler tick spans');

      // Round the dial from its start at the bottom left to its end at the
      // bottom right: the whole comp, then twenty frames across the lanes.
      final centre = tester.getCenter(dial);
      final before = laneBar(tester, layer).width;
      final gesture = await tester.startGesture(centre + const Offset(-5, 5));
      await tester.pump();
      await gesture.moveTo(centre + const Offset(0, -7));
      await tester.pump();
      await gesture.moveTo(centre + const Offset(5, 5));
      await tester.pump();
      await gesture.up();
      await tester.pump();
      expect(laneBar(tester, layer).width, greaterThan(before * 2),
          reason: 'the bar grew with the zoom the dial wrote');
      expect(TimelineZoomDial.valueAt(const Offset(-5, 5)), closeTo(0, 0.01));
      expect(TimelineZoomDial.valueAt(const Offset(5, 5)), closeTo(1, 0.01));
    });

    /// Lantern's minimap under the lanes: a 10px strip that scrolls the lanes
    /// when its window is dragged.
    testWidgets('Lantern draws a minimap that scrolls the lanes',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p, ThemeShape.studio);
      expect(find.byKey(const ValueKey('tl-minimap')), findsNothing,
          reason: 'Studio has no minimap');

      await mount(tester, p, ThemeShape.lantern);
      final minimap = find.byKey(const ValueKey('tl-minimap'));
      expect(minimap, findsOneWidget);
      final strip = tester.getRect(minimap);
      expect(strip.height, closeTo(9, 0.5),
          reason: 'a 10px strip with its hairline under it');
      final foot =
          tester.getRect(find.byKey(const ValueKey('tl-lane-bottom-bar')));
      expect(strip.bottom, lessThanOrEqualTo(foot.top + 0.5),
          reason: 'it stands under the lanes, above the foot');

      await zoomIn(tester, laneBar(tester, layer).center);
      // A press on the strip brings the window to the pointer, at the
      // comp's start; dragging it on from there scrolls the lanes on.
      final gesture =
          await tester.startGesture(Offset(strip.left + 10, strip.center.dy));
      await tester.pump();
      final before = laneBar(tester, layer).left;
      await gesture.moveBy(const Offset(120, 0));
      await tester.pump();
      await gesture.up();
      await tester.pump();
      expect(laneBar(tester, layer).left, lessThan(before),
          reason: 'dragging the window scrolled the lanes');
    });
  });
}
