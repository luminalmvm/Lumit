// Round v2's shape-conditional geometry (K-394, 15-DESIGN §12.1), on the two
// surfaces where it is a branch in a panel rather than a token read.
//
// The stadium controls, the bigger cards and the filled-pill active state are
// covered where they live — `ShapeTokens` in theme_test, `HouseButton.active`
// in controls_hover_test. What is left, and what this file guards, is the
// handful of places a widget has to *ask* which shape it is in: the Viewer's
// transport cluster, which Round gathers into one pill, and the Timeline's
// layer bars, which Round draws as capsules. Each is asserted both ways, so a
// change that reaches Sharp fails here rather than in the running app.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Round v2 geometry (K-394)', () {
    /// A comp with one solid on it — the Timeline needs a layer before it has
    /// a bar to draw, and the Viewer does not mind either way.
    ({LumitState state, LumitUiState uiState}) withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState);
    }

    Future<void> mount(WidgetTester tester, Widget panel, ThemeShape shape,
        {Size size = const Size(1280, 600)}) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = withComp();
      await tester.pumpWidget(hostPanel(
        child: panel,
        state: p.state,
        uiState: p.uiState,
        size: size,
        shape: shape,
      ));
      await tester.pump();
    }

    /// The five transport buttons are one instrument under Round, and a
    /// container round them says so. Sharp gets the identical buttons loose on
    /// the bar — no wrapper at all, which is what "Sharp is untouched" means
    /// here in the widget tree and not only on screen.
    final pill = find.byKey(const ValueKey('viewer-transport-pill'));
    final play = find.byKey(const ValueKey('viewer-play'));

    testWidgets('the Viewer transport sits in one pill under Round',
        (tester) async {
      await mount(tester, const ViewerPanelFrb(), ThemeShape.round);
      expect(pill, findsOneWidget);
      expect(
        find.descendant(of: pill, matching: play),
        findsOneWidget,
        reason: 'the transport buttons are inside the pill, not beside it',
      );
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.round);
      final d = tester.widget<Container>(pill).decoration! as BoxDecoration;
      expect(d.borderRadius, BorderRadius.circular(t.tokens.controlRadius));
    });

    testWidgets('Sharp keeps the transport loose on the bar', (tester) async {
      await mount(tester, const ViewerPanelFrb(), ThemeShape.sharp);
      expect(pill, findsNothing);
      expect(play, findsOneWidget, reason: 'the same buttons, unwrapped');
    });

    /// **Both shapes follow K-411's arrangement.** Round's pill is a container
    /// around the transport, not a re-ordering of the bar: the same controls
    /// come in the same order, with the pill's own key falling where the
    /// transport starts. The Sharp order is asserted in full in
    /// `viewer_panel_frb_test`; this is the shape half of it.
    testWidgets('Round keeps the K-411 order, pill and all', (tester) async {
      await mount(tester, const ViewerPanelFrb(), ThemeShape.round);
      expect(barKeys(tester), [
        'viewer-zoom',
        'viewer-resolution',
        'viewer-region',
        'viewer-grid',
        'viewer-guides-menu',
        'viewer-wireframes',
        'viewer-background',
        'viewer-channel',
        'viewer-exposure',
        'viewer-snapshot-take',
        'viewer-snapshot-show',
        'viewer-timecode',
        'viewer-playback-mode',
        'viewer-transport-pill',
        'viewer-home',
        'viewer-step-back',
        'viewer-play',
        'viewer-step-forward',
        'viewer-end',
        'viewer-colour-badge',
      ]);
    });

    /// The bar is *below* the picture under Round, parted from it by the tile
    /// gap — not laid over the bottom of the frame, which is what v2 first
    /// shipped. Two things are asserted because both are the point: nothing
    /// overlaps, and the picture's own box is the one that shrank, so fit,
    /// zoom and hit-testing are measured against a picture with no bar on it.
    final stage = find.byKey(const ValueKey('viewer-stage'));
    final viewerBar = find.byKey(const ValueKey('viewer-bar'));

    testWidgets(
        'Round puts the Viewer bar below the picture, Sharp welds it '
        'on', (tester) async {
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.round);

      await mount(tester, const ViewerPanelFrb(), ThemeShape.round);
      final picture = tester.getRect(stage);
      final strip = tester.getRect(viewerBar);
      expect(strip.top, greaterThanOrEqualTo(picture.bottom),
          reason: 'the bar starts where the picture ends, at the earliest');
      expect(strip.top - picture.bottom, closeTo(t.tokens.tileGap, 0.5),
          reason: 'and the canvas shows through a tile gap between them');
      expect(strip.bottom,
          closeTo(tester.getRect(find.byType(ViewerPanelFrb)).bottom, 0.5),
          reason: 'the bar rides in the panel, so the panel carries it');

      await mount(tester, const ViewerPanelFrb(), ThemeShape.sharp);
      expect(
        tester.getRect(viewerBar).top,
        closeTo(tester.getRect(stage).bottom, 0.5),
        reason: 'Sharp keeps the welded strip — no gap at all',
      );
    });

    /// A layer bar draws stadium ends under Round: the control radius is the
    /// sentinel that clamps to half the bar's own height, so the same read
    /// gives a capsule whatever the row height turns out to be. Sharp keeps
    /// the 2 px it always had.
    BorderRadius barRadius(WidgetTester tester) {
      final fill = find.byWidgetPredicate((w) =>
          w is Container &&
          w.key is ValueKey<String> &&
          (w.key! as ValueKey<String>).value.startsWith('tl-bar-fill-'));
      expect(fill, findsWidgets);
      return (tester.widget<Container>(fill.first).decoration! as BoxDecoration)
          .borderRadius! as BorderRadius;
    }

    testWidgets(
        'Timeline layer bars are capsules under Round and rectangles '
        'under Sharp', (tester) async {
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.round);

      await mount(tester, const TimelinePanelFrb(), ThemeShape.round);
      expect(barRadius(tester), BorderRadius.circular(t.tokens.controlRadius));

      await mount(tester, const TimelinePanelFrb(), ThemeShape.sharp);
      expect(barRadius(tester), BorderRadius.circular(2));
    });
  });
}
