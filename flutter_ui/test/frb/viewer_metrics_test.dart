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

    Future<void> mount(WidgetTester tester, dynamic p) async {
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(900, 520),
      ));
      await tester.pump();
    }

    Rect rectOf(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

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

      const looking = [
        'viewer-grid',
        'viewer-guides-menu',
        'viewer-channel',
      ];
      for (final key in looking) {
        expect(glyphOf(tester, key).size, const Size(14, 14), reason: key);
      }
      expect(glyphOf(tester, 'viewer-snapshot').size, const Size(14, 14));

      expect(glyphOf(tester, 'viewer-grid').left, closeTo(bar.left + 10, 0.5),
          reason: 'the strip is padded 10 before its first mark');
      for (var i = 1; i < looking.length; i++) {
        expect(
            glyphOf(tester, looking[i]).left -
                glyphOf(tester, looking[i - 1]).right,
            closeTo(8, 0.5),
            reason: '${looking[i]} stands 8 from the mark before it');
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
      await mount(tester, p);

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
