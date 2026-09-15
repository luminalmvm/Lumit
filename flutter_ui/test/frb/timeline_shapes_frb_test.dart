// The Timeline under its three shapes: what each one draws differently, as
// geometry a user could point at. Studio's own numbers are pinned by
// timeline_alignment_test and shape_frb_test; this file is for what
// Desk and Lantern change.

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_zoom_dial.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
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

    BorderRadius barRadius(WidgetTester tester, LayerReference l) {
      final fill = find.byKey(ValueKey<String>('tl-bar-fill-${idOf(l)}'));
      return (tester.widget<Container>(fill).decoration! as BoxDecoration)
          .borderRadius! as BorderRadius;
    }

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

    /// A bar's corner is the shape's content radius on Desk and Lantern; the
    /// selection is an outline on Desk and a ring outside the bar on Lantern,
    /// and the bar itself keeps its rectangle either way.
    testWidgets('a bar wears the content radius and the shape\'s selection',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();

      for (final shape in [ThemeShape.desk, ThemeShape.lantern]) {
        await mount(tester, p, shape);
        final t = LumitTheme.forScheme(LumitColorScheme.dark, shape);
        expect(barRadius(tester, layer),
            BorderRadius.circular(t.tokens.contentRadius),
            reason: 'the corner is the token under $shape');
        final before = laneBar(tester, layer);
        await tester.tapAt(before.center);
        await tester.pump();
        expect(laneBar(tester, layer), before,
            reason: 'selecting does not move the bar under $shape');
        final fill = tester.widget<Container>(
            find.byKey(ValueKey<String>('tl-bar-fill-${idOf(layer)}')));
        final border = (fill.decoration! as BoxDecoration).border;
        final ring = find.byKey(ValueKey<String>('tl-bar-ring-${idOf(layer)}'));
        if (shape == ThemeShape.desk) {
          expect(border, isNotNull, reason: 'Desk outlines the bar');
          expect((border! as Border).top.color, t.accent);
          expect(ring, findsNothing);
        } else {
          expect(border, isNull, reason: 'Lantern draws no outline on it');
          final ringRect = tester.getRect(ring);
          final body = tester.getRect(
              find.byKey(ValueKey<String>('tl-bar-body-${idOf(layer)}')));
          expect(ringRect.left, closeTo(body.left - 2, 0.5),
              reason: 'the ring stands 2px outside the bar');
          expect(ringRect.width, closeTo(body.width + 4, 0.5));
        }
      }
    });

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

    /// Lantern's playhead is a pin: a 12px round head on a 2px stem, still
    /// centred on its frame; and its rows stand on a 28 pitch with no rule
    /// between them.
    testWidgets('Lantern draws the pin head and the 28 pitch', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p, ThemeShape.lantern);

      final marker = find.byType(PlayheadMarker).first;
      expect(tester.getSize(marker).width,
          PlayheadMarker.halfWidthFor(ThemeShape.lantern) * 2);
      final head =
          find.descendant(of: marker, matching: find.byType(CustomPaint));
      expect(tester.getSize(head.first), const Size(12, 12),
          reason: 'a 12px round head');
      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      expect(tester.getCenter(marker).dx,
          closeTo(ruler.left + TimelineAxis.pad, 1.0),
          reason: 'centred on frame zero');

      const d = DensityTokens.lanternRegular;
      expect(d.laneRow, 28);
      expect(
          tester
              .getRect(find.byKey(ValueKey<String>('tl-row-${idOf(layer)}')))
              .height,
          closeTo(28, 0.5),
          reason: 'the outline row is the 28 pitch');
      expect(
          tester
              .getRect(
                  find.byKey(ValueKey<String>('tl-rowbody-${idOf(layer)}')))
              .height,
          closeTo(26, 0.5),
          reason: 'and draws 26 of it, a pixel of ground at each edge');
      expect(laneBar(tester, layer).height, closeTo(28, 0.5));
      final seams = tester
          .widgetList<CustomPaint>(find.byWidgetPredicate(
              (w) => w is CustomPaint && w.painter is RowDividerPainter))
          .map((w) => w.painter! as RowDividerPainter);
      expect(seams, isNotEmpty);
      for (final seam in seams) {
        expect(seam.colour.a, 0, reason: 'no rule between lanes');
      }
    });

    /// Desk's rail and header: every row stands on a surface2 band, 22 in
    /// the 24 pitch; the bar is the hue mixed into the band and outlined in
    /// it; Export is the one accent fill; the fronted comp tab and the mode
    /// in force each stand over a 2px accent rule with no fill.
    testWidgets('Desk draws the rail, the outlined bar and the ruled tabs',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [10, 20])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p, ThemeShape.desk);
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.desk);
      final hue = t.labelColour(p.uiState.model.layers.single.info.label);

      final row = laneBar(tester, layer);
      final band = find.byKey(ValueKey<String>('tl-bar-band-${idOf(layer)}'));
      final bandRect = tester.getRect(band);
      expect(row.height, closeTo(24, 0.5));
      expect(bandRect.height, closeTo(22, 0.5),
          reason: 'a pixel of ground above and below the band');
      expect(bandRect.top, closeTo(row.top + 1, 0.5));
      expect(bandRect.width, closeTo(row.width, 0.5),
          reason: 'the band runs the row\'s full width');
      expect(
          tester
              .widget<ColoredBox>(
                  find.descendant(of: band, matching: find.byType(ColoredBox)))
              .color,
          t.surface2);

      final fill = tester.widget<Container>(
          find.byKey(ValueKey<String>('tl-bar-fill-${idOf(layer)}')));
      final deco = fill.decoration! as BoxDecoration;
      expect(deco.color,
          Color.alphaBlend(hue.withValues(alpha: deskBarMix), t.surface2),
          reason: 'the hue mixed 30 percent into the band');
      expect((deco.border! as Border).top.color, hue,
          reason: 'outlined 1px in the layer hue');
      expect((deco.border! as Border).top.width, 1);
      expect(deco.borderRadius,
          BorderRadius.circular(ShapeTokens.desk.contentRadius));

      final keys = tester.widget<CustomPaint>(find.descendant(
          of: find.byKey(ValueKey<String>('tl-bar-keys-${idOf(layer)}')),
          matching: find.byType(CustomPaint)));
      expect(keys.painter, isA<OutlinedKeysPainter>(),
          reason: 'the shut layer\'s keys are outlined diamonds');
      expect((keys.painter! as OutlinedKeysPainter).colour, t.textSecondary);

      final export = find.byKey(const ValueKey('tl-export'));
      final exportBox = tester.widget<AnimatedContainer>(find.descendant(
          of: export, matching: find.byType(AnimatedContainer)));
      expect((exportBox.decoration! as BoxDecoration).color, t.accent,
          reason: 'Export is the one accent fill');
      expect(
          tester
              .renderObject<RenderParagraph>(
                  find.descendant(of: export, matching: find.byType(RichText)))
              .text
              .style!
              .color,
          t.surface0,
          reason: 'its word in surface0');

      final tab = find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}'));
      final rule = find.descendant(
          of: tab, matching: find.byKey(const ValueKey('tl-tab-rule')));
      expect(rule, findsOneWidget, reason: 'the fronted tab wears the rule');
      final ruleRect = tester.getRect(rule);
      expect(ruleRect.height, closeTo(2, 0.5));
      expect(ruleRect.bottom, closeTo(tester.getRect(tab).bottom, 0.5));
      expect(
          tester
              .widget<ColoredBox>(
                  find.descendant(of: rule, matching: find.byType(ColoredBox)))
              .color,
          t.accent);
      final tabBox = tester.widget<Container>(
          find.descendant(of: tab, matching: find.byType(Container)).first);
      expect((tabBox.decoration! as BoxDecoration).color, isNull,
          reason: 'a word over a rule, no pill and no fill');
      expect(find.byKey(const ValueKey('tl-mode-rule')), findsOneWidget,
          reason: 'and the mode in force wears the same rule');
    });

    /// Lantern's header is one 36 line: the comp tabs as a segmented pill
    /// at the left, the title centred, the Layers / Graph pill and Export at
    /// the right, and nothing between it and the chrome row. Its bar is 18
    /// tall, carries the layer's name, and a selected row fills accent_soft
    /// behind it.
    testWidgets('Lantern draws one header line and named 18px bars',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p, ThemeShape.lantern);
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.lantern);
      final hue = t.labelColour(p.uiState.model.layers.single.info.label);

      final header = tester.getRect(find.byType(CompTabsFrb));
      expect(header.height, closeTo(lanternTimelineLine, 0.5),
          reason: 'one 28 line: the 22 pill and a 3px margin either side');
      final tab = find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}'));
      final segment =
          tester.getRect(find.byKey(const ValueKey('tl-comp-tabs')));
      expect(segment.height, closeTo(Segment.height, 0.5),
          reason: 'the tabs stand in a 22 tall segment');
      for (final (name, finder) in [
        ('the comp tab', tab),
        ('Export', find.byKey(const ValueKey('tl-export'))),
        ('the Layers mode', find.byKey(const ValueKey('tl-view-lanes'))),
        ('the Graph mode', find.byKey(const ValueKey('tl-graph'))),
        ('the title', find.byKey(const ValueKey('tl-title'))),
      ]) {
        final r = tester.getRect(finder);
        expect(r.top, greaterThanOrEqualTo(header.top - 0.5),
            reason: '$name stands on the header line');
        expect(r.bottom, lessThanOrEqualTo(header.bottom + 0.5),
            reason: '$name stands on the header line');
      }
      final toolbar = tester.getRect(find.byKey(const ValueKey('tl-toolbar')));
      expect(toolbar.top, closeTo(header.bottom, 0.5),
          reason: 'no second strip between the header and the chrome row');
      final tabBox = tester.widget<Container>(
          find.descendant(of: tab, matching: find.byType(Container)).first);
      expect((tabBox.decoration! as BoxDecoration).color, t.surface2,
          reason: 'the fronted tab is a surface2 pill');

      final row = laneBar(tester, layer);
      final bodyKey = ValueKey<String>('tl-bar-body-${idOf(layer)}');
      final body = tester.getRect(find.byKey(bodyKey));
      expect(body.height, closeTo(18, 0.5), reason: 'the bar is 18 tall');
      expect(body.center.dy, closeTo(row.center.dy, 0.5),
          reason: 'centred in its row');
      final fill = tester.widget<Container>(
          find.byKey(ValueKey<String>('tl-bar-fill-${idOf(layer)}')));
      expect((fill.decoration! as BoxDecoration).color,
          hue.withValues(alpha: lanternBarAlpha),
          reason: 'the hue at 55 percent');
      final name = find.byKey(ValueKey<String>('tl-bar-name-${idOf(layer)}'));
      expect(name, findsOneWidget, reason: 'the name rides inside the bar');
      final nameRect = tester.getRect(name);
      expect(nameRect.left, greaterThanOrEqualTo(body.left));
      expect(nameRect.right, lessThanOrEqualTo(body.right + 0.5));
      expect(nameRect.top, greaterThanOrEqualTo(body.top - 0.5));
      expect(nameRect.bottom, lessThanOrEqualTo(body.bottom + 0.5));
      final style = tester.renderObject<RenderParagraph>(name).text.style!;
      expect(style.fontSize, 11);
      expect(style.fontWeight, FontWeight.w600);
      expect(style.color, t.textPrimary);

      await tester.tapAt(body.center);
      await tester.pump();
      final rowFill =
          find.byKey(ValueKey<String>('tl-bar-rowfill-${idOf(layer)}'));
      expect(rowFill, findsOneWidget);
      expect(tester.getRect(rowFill).height, closeTo(26, 0.5),
          reason: 'the row\'s 26 inside the 28 pitch');
      final soft = t.accent.withValues(alpha: accentSoftAlpha);
      expect(
          (tester
                  .widget<DecoratedBox>(find.descendant(
                      of: rowFill, matching: find.byType(DecoratedBox)))
                  .decoration as BoxDecoration)
              .color,
          soft,
          reason: 'accent_soft behind the bar');
      final outlineRow = tester.widget<Container>(
          find.byKey(ValueKey<String>('tl-rowbody-${idOf(layer)}')));
      expect((outlineRow.decoration! as BoxDecoration).color, soft,
          reason: 'and under the outline row beside it');
    });
  });
}
