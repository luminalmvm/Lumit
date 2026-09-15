// The shape-conditional geometry (15-DESIGN §12.1) on the surfaces where a
// widget has to *ask* which shape it is in, rather than read a token: the
// Viewer's transport cluster, which a roomed shape gathers into one pill and
// parts from the picture, and the Timeline's layer bars, whose corner is the
// shape's own. Every rule runs once for Studio, Desk and Lantern, reading the
// mounted shape's tokens, so a change that reaches the wrong shape fails here
// rather than in the running app.
//
// The stadium controls, the bigger cards and the filled-pill active state are
// covered where they live: `ShapeTokens` in theme_test, `HouseButton.active`
// in controls_hover_test.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Shape geometry', () {
    /// A comp with one solid on it: the Timeline needs a layer before it has
    /// a bar to draw, and the Viewer does not mind either way.
    ({LumitState state, LumitUiState uiState}) withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState);
    }

    Future<LumitTheme> mount(
        WidgetTester tester, Widget panel, ThemeShape shape,
        {Size size = const Size(1280, 600)}) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = withComp();
      // These are the split drawing's geometry claims; which arrangement a
      // style chooses by default is settings_test's business.
      p.uiState.workspace.interface.viewerBars = ViewerBars.split;
      await tester.pumpWidget(hostPanel(
        child: panel,
        state: p.state,
        uiState: p.uiState,
        size: size,
        shape: shape,
      ));
      await tester.pump();
      return LumitTheme.forScheme(LumitColorScheme.dark, shape);
    }

    /// The five transport buttons are one instrument on a roomed shape, and a
    /// container round them says so. The flat shapes get the identical
    /// buttons loose on the bar, no wrapper at all, which is what "untouched"
    /// means here in the widget tree and not only on screen.
    final pill = find.byKey(const ValueKey('viewer-transport-pill'));
    final play = find.byKey(const ValueKey('viewer-play'));

    testWidgets('the Viewer transport sits in one pill only on a roomed shape',
        (tester) async {
      for (final shape in ThemeShape.values) {
        final t = await mount(tester, const ViewerPanelFrb(), shape);
        if (!t.tokens.roomed) {
          expect(pill, findsNothing, reason: '$shape keeps the bar flat');
          expect(play, findsOneWidget, reason: 'the same buttons, unwrapped');
          continue;
        }
        expect(pill, findsOneWidget, reason: '$shape gathers the transport');
        expect(
          find.descendant(of: pill, matching: play),
          findsOneWidget,
          reason: 'the transport buttons are inside the pill, not beside it',
        );
        final d = tester.widget<Container>(pill).decoration! as BoxDecoration;
        expect(d.borderRadius, BorderRadius.circular(t.tokens.actionRadius));
      }
    });

    /// **Every shape follows the drawing's arrangement**. The pill is a
    /// container around the transport, not a re-ordering of the bar: the same
    /// controls come in the same order, with the pill's own key falling where
    /// the transport starts. The full Studio order is asserted in
    /// `viewer_panel_frb_test`; this is the shape half of it.
    testWidgets("every shape keeps the drawing's order, pill and all",
        (tester) async {
      for (final shape in ThemeShape.values) {
        final t = await mount(tester, const ViewerPanelFrb(), shape);
        expect(
            barKeys(tester),
            [
              'viewer-grid',
              'viewer-guides-menu',
              'viewer-channel',
              'viewer-view',
              'viewer-exposure-reset',
              'viewer-exposure',
              'viewer-snapshot',
              'viewer-snapshot-show',
              if (t.tokens.roomed) 'viewer-transport-pill',
              'viewer-home',
              'viewer-step-back',
              'viewer-play',
              'viewer-step-forward',
              'viewer-end',
              'viewer-timecode',
              'viewer-readout',
            ],
            reason: 'under $shape');
        expect(headerKeys(tester),
            ['viewer-zoom', 'viewer-resolution', 'viewer-colour']);
      }
    });

    /// The bar is *below* the picture on a roomed shape, parted from it by the
    /// tile gap, not laid over the bottom of the frame. Two things are
    /// asserted because both are the point: nothing overlaps, and the
    /// picture's own box is the one that shrank, so fit, zoom and hit-testing
    /// are measured against a picture with no bar on it. The flat shapes weld
    /// the strip on, no gap at all.
    final stage = find.byKey(const ValueKey('viewer-stage'));
    final viewerBar = find.byKey(const ValueKey('viewer-bar'));

    testWidgets(
        'the Viewer bar stands a tile gap below the picture, or is '
        'welded on', (tester) async {
      for (final shape in ThemeShape.values) {
        final t = await mount(tester, const ViewerPanelFrb(), shape);
        final picture = tester.getRect(stage);
        final strip = tester.getRect(viewerBar);
        final gap = t.tokens.roomed ? t.tokens.tileGap : 0.0;
        expect(strip.top, greaterThanOrEqualTo(picture.bottom),
            reason: '$shape: the bar starts where the picture ends');
        expect(strip.top - picture.bottom, closeTo(gap, 0.5),
            reason: '$shape: the ground shows through the shape\'s gap');
        expect(strip.bottom,
            closeTo(tester.getRect(find.byType(ViewerPanelFrb)).bottom, 0.5),
            reason: 'the bar rides in the panel, so the panel carries it');
      }
    });

    /// A layer bar's corner is [clipRadius]: the shape's content radius, a
    /// small corner that keeps a bar reading as content and not as a button,
    /// except that Studio draws square ends, the mockup's own bar.
    BorderRadius barRadius(WidgetTester tester) {
      final fill = find.byWidgetPredicate((w) =>
          w is Container &&
          w.key is ValueKey<String> &&
          (w.key! as ValueKey<String>).value.startsWith('tl-bar-fill-'));
      expect(fill, findsWidgets);
      return (tester.widget<Container>(fill.first).decoration! as BoxDecoration)
          .borderRadius! as BorderRadius;
    }

    /// **The filled state stands off its pill by the shape's inset**, the
    /// same margin on every side, and its corner is the outer one less the
    /// inset. Lantern's 3 keeps a filled segment clear of the pill it sits
    /// in; Studio and Desk have 0, so their buttons draw as they always did.
    testWidgets('the active fill is inset by the pill inset under every shape',
        (tester) async {
      const probe = ValueKey<String>('probe');
      for (final shape in ThemeShape.values) {
        final t = await mount(
          tester,
          Center(
            child: HouseButton(
                key: probe,
                active: true,
                onPressed: () {},
                child: const Text('Mask')),
          ),
          shape,
        );
        final inset = t.tokens.pillInset;
        expect(inset, shape == ThemeShape.lantern ? 3 : 0, reason: '$shape');
        final button = tester.getRect(find.byKey(probe));
        final fill = tester.getRect(find
            .descendant(
                of: find.byKey(probe), matching: find.byType(DecoratedBox))
            .first);
        expect(fill.left - button.left, closeTo(inset, 0.01),
            reason: '$shape: left');
        expect(button.right - fill.right, closeTo(inset, 0.01),
            reason: '$shape: right');
        expect(fill.top - button.top, closeTo(inset, 0.01),
            reason: '$shape: top');
        expect(button.bottom - fill.bottom, closeTo(inset, 0.01),
            reason: '$shape: bottom');
        final d = tester
            .widget<AnimatedContainer>(find.descendant(
                of: find.byKey(probe),
                matching: find.byType(AnimatedContainer)))
            .decoration! as BoxDecoration;
        expect(
            d.borderRadius,
            BorderRadius.circular(
                math.max(0, t.tokens.actionRadius - inset).toDouble()),
            reason: '$shape: the inner corner is the outer less the inset');
        final word = tester.getRect(find.text('Mask'));
        expect(word.center.dy, closeTo(button.center.dy, 0.5),
            reason: '$shape: the label is centred in the button');
        expect(word.center.dx, closeTo(button.center.dx, 0.5),
            reason: '$shape: the label is centred in the button');
      }
    });

    testWidgets('Timeline layer bars wear the shape\'s clip radius',
        (tester) async {
      for (final shape in ThemeShape.values) {
        final t = await mount(tester, const TimelinePanelFrb(), shape);
        expect(barRadius(tester), BorderRadius.circular(clipRadius(t)),
            reason: 'under $shape');
      }
      expect(
          clipRadius(
              LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.studio)),
          sharpClipRadius,
          reason: 'Studio keeps its square ends whatever its token says');
    });
  });
}
