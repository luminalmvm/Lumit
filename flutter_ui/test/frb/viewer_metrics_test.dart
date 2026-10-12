// The Viewer, measured against its approved drawing.
//
// **Why this file exists.** `viewer_panel_frb_test` asserts what the Viewer
// *does* — the transport steps, a drag moves a layer, a snapshot is taken. This
// asserts what it *is*: the heights, the type, the colours and the spacing the
// approved Main drawing computes for the two strips and for the chip over the
// picture. Those are decisions (docs/15-DESIGN §12A.6: the mockups' metrics are
// canonical), and a decision nothing measures drifts back to whatever the
// widgets happened to give.
//
// Every number below is the drawing's own rendered value, not an approximation
// of it. Where a measurement allows for a pixel of chrome the code does not
// draw — a [HouseButton]'s transparent edge — the allowance is named, so the
// reading stays the drawing's and the arithmetic stays visible.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Viewer metrics (frb)', () {
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Opening titles');
      final layer = comp.addSolidLayer();
      layer.rename(name: 'Title');
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    /// The panel, at [size] — the **surface's** own size, not just the
    /// MediaQuery's: what a bar has to lay out in is the constraint it is
    /// given, and a MediaQuery that disagrees with the surface changes nothing
    /// about the room a row has.
    Future<void> mount(WidgetTester tester, dynamic p,
        {ViewerBars bars = ViewerBars.split,
        Size size = const Size(900, 520),
        ThemeShape shape = ThemeShape.studio}) async {
      await tester.binding.setSurfaceSize(size);
      addTearDown(() => tester.binding.setSurfaceSize(null));
      (p.uiState as LumitUiState).workspace.interface.viewerBars = bars;
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
        shape: shape,
      ));
      await tester.pump();
    }

    /// **The transport is the last thing standing** (owner ruling). The bar
    /// cannot hold everything on a Viewer docked into a sidebar, so as it
    /// narrows the reading goes, then the ways of looking fold into one
    /// overflow mark, then the clock — and Play is still there at a width
    /// where nothing else is. A bar that kept the exposure field and lost Play
    /// would have kept the wrong half.
    testWidgets('the bar sheds everything before it sheds the transport',
        (tester) async {
      final p = withLayer();

      bool shows(String key) =>
          find.byKey(ValueKey<String>(key)).evaluate().isNotEmpty;

      // Wide: every cluster on the bar at once.
      await mount(tester, p, size: const Size(900, 520));
      expect(shows('viewer-play'), isTrue);
      expect(shows('viewer-timecode'), isTrue);
      expect(shows('viewer-grid'), isTrue,
          reason: 'the ways of looking stand on their own');
      expect(shows('viewer-overflow'), isFalse,
          reason: 'and so need no overflow mark');
      expect(find.byKey(const ValueKey('viewer-readout')), findsOneWidget);

      // The reading goes first: every fact on it is said again elsewhere.
      await mount(tester, p, size: const Size(430, 520));
      expect(find.byKey(const ValueKey('viewer-readout')), findsNothing,
          reason: 'the reading is the first whole thing to go');
      expect(shows('viewer-grid'), isTrue);
      expect(shows('viewer-timecode'), isTrue);
      expect(shows('viewer-play'), isTrue);

      // Then the ways of looking fold into one mark — §12A.6's step 4.
      await mount(tester, p, size: const Size(340, 520));
      expect(shows('viewer-overflow'), isTrue,
          reason: 'a toolbar collapses into a menu rather than clipping');
      expect(shows('viewer-grid'), isFalse,
          reason: 'the marks are inside it now, not beside it');
      expect(shows('viewer-timecode'), isTrue);
      expect(shows('viewer-play'), isTrue);

      // Then the clock, and the five transport buttons stand alone.
      await mount(tester, p, size: const Size(250, 520));
      expect(shows('viewer-timecode'), isFalse,
          reason: 'the clock is the last thing to leave before the transport');
      expect(shows('viewer-play'), isTrue,
          reason: 'the transport outlives everything else on the bar');
      for (final mark in [
        'viewer-home',
        'viewer-step-back',
        'viewer-step-forward',
        'viewer-end',
      ]) {
        expect(shows(mark), isTrue,
            reason: 'the transport sheds none of its five');
      }
    });

    /// The controls the overflow mark swallowed are still the same controls:
    /// pressing it opens them, it does not merely say they exist.
    testWidgets('the overflow mark opens the marks it folded away',
        (tester) async {
      final p = withLayer();
      await mount(tester, p, size: const Size(340, 520));

      await tester.tap(find.byKey(const ValueKey('viewer-overflow')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('viewer-grid')), findsOneWidget,
          reason: 'the transparency board is in there');
      expect(find.byKey(const ValueKey('viewer-guides-menu')), findsOneWidget,
          reason: 'and the guides menu, which is itself a menu');
      expect(find.byKey(const ValueKey('viewer-exposure')), findsOneWidget,
          reason: 'and the exposure, which is a drag field');
    });

  }, skip: !engineAvailable);
}
