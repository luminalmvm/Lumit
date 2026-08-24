// The Viewer, measured against its approved drawing (K-466).
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

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Viewer metrics (frb)', () {
    final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

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
        Size size = const Size(900, 520)}) async {
      await tester.binding.setSurfaceSize(size);
      addTearDown(() => tester.binding.setSurfaceSize(null));
      (p.uiState as LumitUiState).workspace.interface.viewerBars = bars;
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
      ));
      await tester.pump();
    }

    Rect rectOf(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

    /// The channel picker's painted face. It carries no key of its own — the
    /// bar's order is asserted by the keys of the controls on it — so what
    /// finds it is the one painter that draws it.
    final channelFace = find.byWidgetPredicate(
        (w) => w is CustomPaint && w.painter is ChannelFacePainter);

    /// The **glyph** inside a bar mark, which is what the drawing measures —
    /// never the button's box, which carries a transparent edge so that hover
    /// cannot shift the row.
    Rect glyphOf(WidgetTester tester, String key) => tester.getRect(find
        .descendant(
          of: find.byKey(ValueKey<String>(key)),
          matching: find.byType(glyph.LumitIcon),
        )
        .first);

    /// The style a piece of text is actually painted in, after the default
    /// styles above it have merged.
    TextStyle styleOf(WidgetTester tester, Finder text) =>
        (tester.renderObject<RenderParagraph>(text).text as TextSpan).style!;

    BoxDecoration decorationOf(WidgetTester tester, String key) =>
        tester.widget<Container>(find.byKey(ValueKey<String>(key))).decoration!
            as BoxDecoration;

    /// **The header strip is a panel header** (§12A.6's table: 22 under both
    /// densities), on the faint surface every header and bottom bar takes.
    testWidgets('the header strip is 22 of surface2, padded 10',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final header = rectOf(tester, 'viewer-header');
      expect(header.height, 22);
      expect(decorationOf(tester, 'viewer-header').color, t.surface2);

      expect(rectOf(tester, 'viewer-zoom').height, t.density.inRowPicker,
          reason: 'the drawing computes 18 for all three: a `.dd` of 16 with '
              'its 1px border either side, which is the in-row face');
      expect(rectOf(tester, 'viewer-colour').right,
          closeTo(header.right - 10, 0.5),
          reason: 'the strip is padded 10 at its right-hand end');

      final title = find.text('VIEWER');
      expect(tester.getRect(title).left, closeTo(header.left + 10, 0.5),
          reason: 'and 10 at its left');
      final kicker = styleOf(tester, title);
      expect(kicker.fontFamily, LumitTheme.monoFontFamily);
      expect(kicker.fontSize, 9);
      expect(kicker.letterSpacing, closeTo(1.08, 1e-9));
      expect(kicker.color, t.textPrimary,
          reason: 'the panel is the container these controls belong to, so its '
              'own kicker is lit');
    });

    /// The three pickers sit 6 apart with a 10px label, on `surface_2` inside a
    /// plain hairline — the `.dd` the drawing draws everywhere.
    testWidgets('the header pickers are 6 apart, at a 10px label',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final zoom = rectOf(tester, 'viewer-zoom');
      final quality = rectOf(tester, 'viewer-resolution');
      final colour = rectOf(tester, 'viewer-colour');
      expect(quality.left - zoom.right, closeTo(6, 0.5));
      expect(colour.left - quality.right, closeTo(6, 0.5));

      expect(styleOf(tester, find.text('Fit')).fontSize, inRowDropdownTextSize);
      expect(styleOf(tester, find.text('Linear → sRGB')).fontSize,
          inRowDropdownTextSize);

      final face = tester
          .widget<Container>(find
              .descendant(
                of: find.byKey(const ValueKey('viewer-zoom')),
                matching: find.byType(Container),
              )
              .first)
          .decoration! as BoxDecoration;
      expect(face.color, t.surface2);
      expect((face.border! as Border).top.color, t.hairline);
    });

    /// **The bottom bar is 22 as well**, and its marks are 14 — the drawing's
    /// own glyph size (K-456), not a panel icon's 16 nor the 20 the transport
    /// used to take.
    testWidgets('the bottom bar is 22, its glyphs 14, its gaps 8 and 10',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final bar = rectOf(tester, 'viewer-bar');
      expect(bar.height, 22);
      expect(decorationOf(tester, 'viewer-bar').color, t.surface2);

      const looking = ['viewer-grid', 'viewer-guides-menu'];
      for (final key in looking) {
        expect(glyphOf(tester, key).size, const Size(14, 14), reason: key);
      }
      expect(glyphOf(tester, 'viewer-snapshot').size, const Size(14, 14));
      // The channel's face is painted rather than set from the icon set (it is
      // the one mark in colour, §5), so it is measured by its own key — at the
      // same 14 as every glyph beside it.
      expect(tester.getRect(channelFace).size, const Size(14, 14));

      expect(glyphOf(tester, 'viewer-grid').left, closeTo(bar.left + 10, 0.5),
          reason: 'the strip is padded 10 before its first mark');
      final marks = [
        glyphOf(tester, 'viewer-grid'),
        glyphOf(tester, 'viewer-guides-menu'),
        tester.getRect(channelFace),
      ];
      for (var i = 1; i < marks.length; i++) {
        expect(marks[i].left - marks[i - 1].right, closeTo(8, 0.5),
            reason: 'mark $i stands 8 from the mark before it');
      }

      const transport = [
        'viewer-home',
        'viewer-step-back',
        'viewer-play',
        'viewer-step-forward',
        'viewer-end',
      ];
      for (final key in transport) {
        expect(glyphOf(tester, key).size, const Size(14, 14), reason: key);
      }
      for (var i = 1; i < transport.length; i++) {
        expect(
            glyphOf(tester, transport[i]).left -
                glyphOf(tester, transport[i - 1]).right,
            closeTo(10, 0.5),
            reason: 'the transport is one instrument, spaced 10');
      }
    });

    /// The bar's three readings, each at the size the drawing sets it: the
    /// exposure bare at 10, the clock at 11 and lit, the composition's own
    /// reading at 10 and muted.
    testWidgets('the exposure is bare at 10, the clock 11, the reading 10',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final exposure = styleOf(tester, find.text('+0.0'));
      expect(exposure.fontFamily, LumitTheme.monoFontFamily);
      expect(exposure.fontSize, barValueTextSize);
      expect(exposure.color, t.textSecondary);
      final well = tester
          .widget<Container>(find
              .descendant(
                of: find.byKey(const ValueKey('viewer-exposure')),
                matching: find.byType(Container),
              )
              .first)
          .decoration! as BoxDecoration;
      expect(well.color, isNull,
          reason: 'the drawing sets the exposure as the number alone — no '
              'inset behind it, unlike every value in a panel row');

      final clock = styleOf(tester, find.text('00:00:00:00'));
      expect(clock.fontFamily, LumitTheme.monoFontFamily);
      expect(clock.fontSize, 11);
      expect(clock.color, t.textPrimary);

      final reading =
          styleOf(tester, find.byKey(const ValueKey('viewer-readout')));
      expect(reading.fontFamily, LumitTheme.monoFontFamily);
      expect(reading.fontSize, barValueTextSize);
      expect(reading.color, t.textMuted);
      expect(rectOf(tester, 'viewer-readout').right,
          closeTo(rectOf(tester, 'viewer-bar').right - 10, 0.5),
          reason: 'the reading is the bar\'s right-hand end');
    });

    /// **The reading says what is on screen**: the composition, the time, the
    /// pixels the engine actually made, and the magnification.
    testWidgets('the reading names the comp, the time, the pixels and the zoom',
        (tester) async {
      final p = withLayer();
      // Wide enough that the ladder below has shed nothing — the test font
      // gives every character a full em, so the drawing's own width is not it.
      await mount(tester, p, size: const Size(1400, 520));

      final text = tester
          .widget<Text>(find.byKey(const ValueKey('viewer-readout')))
          .data!;
      expect(text, contains('Opening titles'));
      expect(text, contains('00:00:00:00'));
      expect(text, contains('1920×1080'));
      expect(text, contains('%'));
      expect(text, contains('·'), reason: 'the drawing parts them with a dot');
    });

    /// **The selection's name over the picture** (the drawing's TITLE chip):
    /// 16 from the stage's left edge, 8 down from its top, in `animated` inside
    /// an `animated` hairline — the colour of the box it names (§3.1).
    testWidgets('the selection chip sits 16/8 into the stage, in animated',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final stage = rectOf(tester, 'viewer-stage');
      final chip = rectOf(tester, 'viewer-tag');
      expect(chip.left - stage.left, closeTo(16, 0.5));
      expect(chip.top - stage.top, closeTo(8, 0.5));

      final decoration = decorationOf(tester, 'viewer-tag');
      final border = decoration.border! as Border;
      expect(border.top.color, t.animated);
      expect(border.top.width, 1);
      expect(decoration.color, isNull, reason: 'an outline, not a fill');

      final label = find.text('TITLE');
      expect(label, findsOneWidget, reason: 'the selected layer, in capitals');
      final style = styleOf(tester, label);
      expect(style.fontFamily, LumitTheme.monoFontFamily);
      expect(style.fontSize, 9);
      expect(style.letterSpacing, closeTo(viewerTagTracking, 1e-9));
      expect(style.color, t.animated);
      // The drawing's 1 above and below, 5 either side.
      expect(tester.getRect(label).left - chip.left, closeTo(6, 0.5),
          reason: '5 of padding plus the 1px border');
    });

    /// Nothing selected, nothing named: the chip is a statement about a
    /// selection and goes with it rather than standing empty.
    testWidgets('the chip goes when the selection does', (tester) async {
      final p = withLayer();
      await mount(tester, p);
      expect(find.byKey(const ValueKey('viewer-tag')), findsOneWidget);

      p.uiState.clearSelection();
      await tester.pump();
      expect(find.byKey(const ValueKey('viewer-tag')), findsNothing);
    });

    /// **The three arrangements** (K-448's setting, K-466's drawing). Split is
    /// the drawing's: a header above the picture and the bar below it. The
    /// other two gather everything into one strip, which then carries the
    /// panel's kicker and the three pickers ahead of the bar's own marks —
    /// the same controls in the same order, on one row instead of two.
    testWidgets('the setting splits the bars, or gathers them top or bottom',
        (tester) async {
      final p = withLayer();

      await mount(tester, p);
      var stage = rectOf(tester, 'viewer-stage');
      expect(rectOf(tester, 'viewer-header').bottom, closeTo(stage.top, 0.5),
          reason: 'split: the header is above the picture');
      expect(rectOf(tester, 'viewer-bar').top, closeTo(stage.bottom, 0.5),
          reason: 'and the bar below it');

      await mount(tester, p, bars: ViewerBars.top);
      stage = rectOf(tester, 'viewer-stage');
      expect(find.byKey(const ValueKey('viewer-header')), findsNothing,
          reason: 'gathered: there is one strip, not two');
      expect(rectOf(tester, 'viewer-bar').bottom, closeTo(stage.top, 0.5),
          reason: 'and it is above the picture');
      expect(
          barKeys(tester).take(4),
          [
            'viewer-zoom',
            'viewer-resolution',
            'viewer-colour',
            'viewer-grid',
          ],
          reason: "the pickers lead the strip, in the header's own order");
      expect(find.text('VIEWER'), findsOneWidget,
          reason: 'the panel keeps its name when the header goes');

      await mount(tester, p, bars: ViewerBars.bottom);
      stage = rectOf(tester, 'viewer-stage');
      expect(find.byKey(const ValueKey('viewer-header')), findsNothing);
      expect(rectOf(tester, 'viewer-bar').top, closeTo(stage.bottom, 0.5),
          reason: 'gathered at the bottom, under the picture');
    });

    /// **The channel's face is the answer's own colour** (§5, owner review):
    /// the tri-colour mark for RGB, a single circle in the channel's colour for
    /// R, G and B, and the near-white a matte reads as for alpha. It is the one
    /// mark in the set that carries colour, so it is painted rather than set
    /// from a font glyph.
    testWidgets('the channel face is a coloured circle for the view in force',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      ChannelFacePainter faceNow() =>
          tester.widget<CustomPaint>(channelFace).painter!
              as ChannelFacePainter;

      expect(faceNow().channel, ViewerChannel.rgb);
      expect(ChannelFacePainter.single(t, ViewerChannel.rgb), isNull,
          reason: 'RGB is the tri-colour mark, not one circle');
      expect(ChannelFacePainter.single(t, ViewerChannel.red),
          ScopeColours.standard.red);
      expect(ChannelFacePainter.single(t, ViewerChannel.green),
          ScopeColours.standard.green);
      expect(ChannelFacePainter.single(t, ViewerChannel.blue),
          ScopeColours.standard.blue);
      expect(ChannelFacePainter.single(t, ViewerChannel.alpha), t.textPrimary,
          reason: 'alpha is not a colour, so its circle is the matte white');

      // And the face follows the pick rather than a name appearing beside it.
      await tester.tap(find.byKey(const ValueKey('viewer-channel')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('viewer-channel-green')));
      await tester.pumpAndSettle();
      expect(faceNow().channel, ViewerChannel.green);
    });

    /// **The exposure's way back to nothing** (owner review): a reset mark to
    /// the left of the number, there only while there is something to undo.
    testWidgets('the exposure reset appears with a value and clears it',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);
      final reset = find.byKey(const ValueKey('viewer-exposure-reset'));

      expect(reset, findsNothing,
          reason: 'the drawing puts no reset beside a resting exposure');

      p.uiState.setViewerStops(1.5);
      await tester.pump();
      expect(reset, findsOneWidget);
      expect(find.text('+1.5'), findsOneWidget);
      expect(tester.getRect(reset).right,
          lessThanOrEqualTo(tester.getRect(find.text('+1.5')).left),
          reason: 'it stands to the left of the number it undoes');

      await tester.tap(reset);
      await tester.pump();
      expect(p.uiState.viewerLook.stops, 0);
      expect(reset, findsNothing, reason: 'and goes with the value');
    });

    /// **What the reading sheds, and in what order** (§12A.6, K-451). The two
    /// gaps give way first — the reading keeps its full line at widths that
    /// used to elide it — then the arrowed preview size, then the composition's
    /// name. The time, the size and the magnification are the last to stand.
    ///
    /// The widths below are wider than the drawing's because the test font
    /// gives every character a full em: what is asserted is the **order** of
    /// the shedding, which the font cannot change.
    testWidgets('the reading sheds the arrowed size, then the comp name',
        (tester) async {
      final p = withLayer();
      final widths = [1400.0, 1200.0, 1050.0, 950.0, 860.0];
      final seen = <double, String>{};
      for (final width in widths) {
        await mount(tester, p, size: Size(width, 520));
        seen[width] = tester
            .widget<Text>(find.byKey(const ValueKey('viewer-readout')))
            .data!;
      }

      expect(seen[1400]!, contains('→'),
          reason: 'given the room, the reading says the whole of it');
      expect(seen[1400]!, contains('Opening titles'));

      // Every rung keeps the values: the time, the comp's size, the zoom.
      for (final text in seen.values) {
        expect(text, contains('00:00:00:00'));
        expect(text, contains('1920×1080'));
        expect(text, contains('%'));
      }

      // Shedding is monotone and in the stated order: the arrowed preview size
      // goes first, the name second, and neither comes back as the bar narrows.
      var arrow = true, name = true;
      for (final width in widths) {
        final text = seen[width]!;
        if (!text.contains('→')) arrow = false;
        if (!text.contains('Opening titles')) name = false;
        expect(text.contains('→'), arrow, reason: 'the arrow, at $width');
        expect(text.contains('Opening titles'), name,
            reason: 'the name, at $width');
        expect(arrow && !name, isFalse,
            reason: 'the name never goes while the arrow is still there');
      }
      expect(seen[860]!.contains('→'), isFalse,
          reason: 'by the narrowest width the arrowed preview size has gone');
      expect(arrow, isFalse, reason: 'the ladder was actually exercised');
    });

    /// The surround is the neutral grey no scheme colours (§3.2), and the
    /// drawing's own `#121212`.
    testWidgets('the surround is the neutral viewer grey', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final surround = tester.widget<Container>(find
          .descendant(
            of: find.byKey(const ValueKey('viewer-stage')),
            matching: find.byType(Container),
          )
          .first);
      expect((surround.decoration as BoxDecoration?)?.color ?? surround.color,
          t.viewerSurround);
    });
  }, skip: !engineAvailable);
}
