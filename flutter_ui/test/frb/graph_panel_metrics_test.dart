// The Graph panel measured against the approved NodeGraph drawing, band by
// band and box by box.
//
// **Why this file exists.** `graph_panel_frb_test` is about what the panel
// *does* — what a drag wires, what a delete takes with it. This one is about
// what it *looks like*: the numbers the drawing's own computed styles resolved
// to (K-451, K-454, K-456, K-458). Nothing here names a private widget class,
// because none of these claims is about how the panel is built — each is
// something a person could point at on screen and measure with a ruler.
//
// A value that disagrees with the drawing is a defect, so each expectation
// carries the drawing's own number in its reason.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/fx_section.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  group('Graph panel metrics (frb)', () {
    final theme = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withBlur() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      const size = Size(900, 600);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const GraphPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
      ));
      await tester.pump();
    }

    Rect at(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

    TextStyle styleOf(WidgetTester tester, String key) =>
        tester.widget<Text>(find.byKey(ValueKey<String>(key))).style!;

    String effectKey(LayerReference layer) => graphNodeKey(
        layer.getGraph().nodes.firstWhere((n) => n.matchName == 'blur').node);

    /// 1. **The toolbar is the drawing's 22px band**, on `surface_1`, with the
    /// two switches, frame-all at 13 and the zoom readout in Geist Mono 10.
    testWidgets('the toolbar is built to the drawing\'s band', (tester) async {
      final p = withBlur();
      await mount(tester, p);

      expect(at(tester, 'graph-toolbar').height, graphToolbarHeight,
          reason: 'the drawing\'s 22px header band');
      expect(
        tester
            .widget<Container>(
                find.byKey(const ValueKey<String>('graph-toolbar')))
            .color,
        theme.surface1,
        reason: 'the header strip is surface_1, as every panel\'s is',
      );

      // Auto-wire and Heal are `HouseToggle`s, on, which is `animated`
      // everywhere a pill switch is (K-465, K-473's note).
      expect(find.byKey(const ValueKey<String>('graph-auto-wire')),
          findsOneWidget);
      expect(find.byKey(const ValueKey<String>('graph-heal')), findsOneWidget);

      final zoom = styleOf(tester, 'graph-zoom');
      expect(zoom.fontFamily, LumitTheme.monoFontFamily,
          reason: 'the readout is mono in the drawing');
      expect(zoom.fontSize, 10, reason: 'the drawing\'s 10px readout');
      expect(zoom.color, theme.textMuted);
      expect(find.text('100%'), findsOneWidget);

      expect(at(tester, 'graph-frame-all').width, graphIconSize,
          reason: 'the drawing\'s 13x13 glyph (K-456)');
    });

    /// 2. **A node card is 150 wide inside a hairline**, with a 21px header
    /// strip and 18px port rows — 152 x 59 for a two-row box, which is what
    /// the manifest measures the Source box at.
    testWidgets('a node card is the drawing\'s box', (tester) async {
      final p = withBlur();
      await mount(tester, p);

      final source = at(tester, 'graph-node-source');
      expect(source.width, graphNodeWidth + 2,
          reason: '150 of content inside a 1px border each side');
      expect(source.height, 2 + graphNodeHeaderHeight + 2 * graphPortRowHeight,
          reason: 'the manifest\'s 152x59: a header and two port rows');

      // The Layer out box is the narrower one, with Image and Audio.
      final out = at(tester, 'graph-node-out');
      expect(out.width, graphOutNodeWidth + 2);
      expect(out.height, 2 + graphNodeHeaderHeight + 2 * graphPortRowHeight);
    });

    /// 3. **A port row is 18 high and its socket 9 across**, sitting on the
    /// card's edge — filled when wired, hollow when not.
    testWidgets('port rows and sockets are the drawing\'s', (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);

      final image = at(tester, 'graph-socket-source-image');
      expect(image.width, graphSocketSize);
      expect(image.height, graphSocketSize);
      final card = at(tester, 'graph-node-source');
      expect(image.center.dx, closeTo(card.right, 0.01),
          reason: 'an output socket sits on the card\'s right edge');
      expect(
          image.center.dy,
          closeTo(card.top + 1 + graphNodeHeaderHeight + graphPortRowHeight / 2,
              0.01),
          reason: 'centred on its own 18px row, under the header');

      final input = at(tester, 'graph-socket-$key-input');
      expect(input.center.dx, closeTo(at(tester, 'graph-node-$key').left, 0.01),
          reason: 'an input socket sits on the card\'s left edge');
    });

    /// 4. **A socket and its word take the port's type colour**, which is the
    /// whole of the canvas's colour coding (K-472). Five colours, seven types.
    testWidgets('a port draws in its type\'s token', (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
      await tester.pump();

      expect(
          styleOf(tester, 'graph-port-$key-in-input').color, theme.port.image,
          reason: 'the picture\'s own path is the image token');
      expect(
          styleOf(tester, 'graph-port-$key-in-radius').color, theme.port.number,
          reason: 'a number parameter is the number token');
      expect(styleOf(tester, 'graph-port-out-in-audio').color, theme.port.audio,
          reason: 'the Layer out box\'s audio socket');

      // A filled socket is wired, a hollow one is not.
      final wired = tester
          .widget<Container>(
              find.byKey(ValueKey<String>('graph-socket-$key-input')))
          .decoration! as BoxDecoration;
      final loose = tester
          .widget<Container>(
              find.byKey(ValueKey<String>('graph-socket-$key-radius')))
          .decoration! as BoxDecoration;
      expect(wired.color, theme.port.image);
      expect(loose.color, theme.surface1,
          reason: 'hollow: the card\'s own ground shows through');
    });

    /// 5. **The header wears the application's own grammar** (K-645): an
    /// enable tick left of the name — the Effect controls heading's switch
    /// face, the same widget at the same scale — and a twirl beside it that
    /// opens the box up. No lettered badges: the canvas says what it means
    /// with the marks the rest of the application already uses.
    testWidgets('a node header is a tick, a twirl and a name', (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);

      // The tick IS the Effect controls switch: `fxEnableMark`'s checkbox,
      // scaled by the same number the heading scales it by.
      final tick = tester.widget<HouseCheckbox>(
          find.byKey(ValueKey<String>('graph-enable-$key')));
      expect(tick.value, isTrue, reason: 'a live box reads as switched on');
      expect(
          tester
              .widget<Transform>(find
                  .ancestor(
                    of: find.byKey(ValueKey<String>('graph-enable-$key')),
                    matching: find.byType(Transform),
                  )
                  .first)
              .transform
              .getMaxScaleOnAxis(),
          closeTo(fxEnableMarkScale, 0.001));

      // Each mark fills its own cell: the tick's is the scaled checkbox's
      // size, the twirl's is the header's ordinary one.
      expect(at(tester, 'graph-enable-$key').width,
          closeTo(graphEnableSize, 0.01));
      expect(at(tester, 'graph-enable-$key').height,
          closeTo(graphEnableSize, 0.01));
      expect(at(tester, 'graph-twirl-$key').width, graphBadgeSize);
      expect(at(tester, 'graph-twirl-$key').height, graphBadgeSize);

      // Shut, the twirl points the way it will open and draws muted; open, it
      // points down in the primary text colour.
      glyph.LumitIcon twirl() => tester.widget<glyph.LumitIcon>(find.descendant(
            of: find.byKey(ValueKey<String>('graph-twirl-$key')),
            matching: find.byType(glyph.LumitIcon),
          ));
      expect(twirl().glyph, LumitIcons.expand);
      expect(twirl().colour, theme.textMuted);
      expect(twirl().size, graphTwirlSize);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
      await tester.pump();
      expect(twirl().glyph, LumitIcons.collapse);
      expect(twirl().colour, theme.textPrimary);

      // Switching the tick off bypasses the box, which is what `B` did.
      await tester.tap(find.byKey(ValueKey<String>('graph-enable-$key')));
      await tester.pump();
      expect(
          tester
              .widget<HouseCheckbox>(
                  find.byKey(ValueKey<String>('graph-enable-$key')))
              .value,
          isFalse);

      // The derived boxes can be neither bypassed nor opened, so they carry
      // neither control. A driver draws every socket it has whatever its
      // exposure says, so it carries a tick and no twirl.
      expect(find.byKey(const ValueKey<String>('graph-enable-source')),
          findsNothing);
      expect(
          find.byKey(const ValueKey<String>('graph-twirl-out')), findsNothing);
    });

    /// 5b. **A name runs the whole width the controls leave** (owner, desk
    /// test): they cut short with half the header standing empty beside them.
    /// The header was a `Flexible` name next to a `Spacer`, and a `Spacer` is
    /// an `Expanded` of flex 1 — so the two split the free space half each and
    /// the name ellipsised at half a card.
    testWidgets('a node name takes the header\'s whole remaining width',
        (tester) async {
      final p = withBlur();
      p.layer.rename(
          name: 'A layer named far past anything a 152px card could hold');
      p.uiState.model.refresh();
      await mount(tester, p);

      // The Source box wears no controls, so its name has the header entire:
      // the card less its 1px border either side and the header's 8 of inset.
      final card = at(tester, 'graph-node-source');
      final name = at(tester, 'graph-node-name-source');
      expect(name.width, card.width - 2 - 16,
          reason: 'the drawing\'s 152 card, less its border and its insets');

      // And on a box that does wear them, they are hard left and the name gets
      // the whole remainder — not a share of it.
      final key = effectKey(p.layer);
      final effect = at(tester, 'graph-node-$key');
      expect(at(tester, 'graph-enable-$key').left,
          closeTo(effect.left + 1 + 8, 0.01),
          reason: 'the tick starts at the header\'s own inset');
      final controls = graphEnableSize + graphBadgeSize + 2 * 2;
      expect(at(tester, 'graph-node-name-$key').left,
          closeTo(effect.left + 1 + 8 + controls, 0.01),
          reason: 'and the name starts where the twirl ends');
    });

    /// 6. **A node's name is a kicker**, primary while the box is live and
    /// muted while it is bypassed — the drawing's own reading.
    testWidgets('a node header is set in the kicker', (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);

      TextStyle name(String node) => tester
          .widget<Text>(find
              .descendant(
                of: find.byKey(ValueKey<String>('graph-node-$node')),
                matching: find.byType(Text),
              )
              .first)
          .style!;
      expect(name(key).fontFamily, LumitTheme.monoFontFamily);
      expect(name(key).fontSize, theme.kicker.fontSize);
      expect(name(key).color, theme.textPrimary);

      await tester.tap(find.byKey(ValueKey<String>('graph-enable-$key')));
      await tester.pump();
      expect(name(key).color, theme.textMuted,
          reason: 'a bypassed box\'s name goes quiet, as the drawing draws it');
    });

    /// 7. **The legend is the canvas's only key**: the heading and five
    /// swatches, each in its own token, lower case and lightly tracked.
    testWidgets('the legend names five colours for seven types',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);

      expect(
          find.byKey(const ValueKey<String>('graph-legend')), findsOneWidget);
      for (final word in [
        'image · matte',
        'number',
        'colour',
        'shape · points',
        'audio',
      ]) {
        expect(find.text(word), findsOneWidget,
            reason: 'the drawing\'s legend groups, lower case');
      }
      expect(find.text('Types'), findsOneWidget);
    });

    /// 8. **Ctrl+Space opens the console over the graph** (K-645, K-673),
    /// wearing the canvas's own words: the one key in the head's kicker, and a
    /// foot line saying what picking a row will do. The popover's own shape is
    /// the console's now, so its numbers are asserted where it lives rather
    /// than here.
    testWidgets('the graph\'s add surface is the console, in its own words',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);

      await tester.tapAt(const Offset(600, 400));
      await tester.pump();
      p.uiState.activePanel.value = Panel.graph;
      expect(p.uiState.consoleClaim!(), isTrue,
          reason: 'the graph claims Ctrl+Space while it is the focused panel');
      await tester.pump();

      expect(
          find.byKey(const ValueKey<String>('fx-console-bar')), findsOneWidget);
      expect(find.text('Ctrl+Space'), findsOneWidget,
          reason: 'one surface, one key — the Tab door went (K-673)');
      expect(find.text('Adds a box to the graph'), findsOneWidget,
          reason: 'no wire in hand, so the foot says what a pick will do');
      expect(find.byKey(const ValueKey<String>('fx-console-item-Wiggle')),
          findsOneWidget,
          reason: 'and with no ring to offer the list stands open');
    });

    /// 9. **A wire always leaves its socket** (owner, desk test). The cubic's
    /// handles used to be ±dx/2, so a consumer dragged left of — or on top of
    /// — its producer collapsed them to nothing and the wire vanished behind
    /// the two cards. [graphWireStub] is the floor that keeps the classic
    /// node-editor S-curve: the curve reaches past the output socket to the
    /// right and past the input socket to the left, whichever way round the
    /// boxes sit.
    test('a backwards wire still leaves both sockets', () {
      // The consumer is 60 px LEFT of the producer, and level with it.
      const from = Offset(400, 200);
      const to = Offset(340, 200);
      final bounds = graphWirePath(from, to).getBounds();

      expect(bounds.right, greaterThan(from.dx + graphWireStub / 2),
          reason: 'the curve reaches out to the right of the output socket');
      expect(bounds.left, lessThan(to.dx - graphWireStub / 2),
          reason: 'and out to the left of the input socket');
      expect(bounds.width, greaterThan((from.dx - to.dx).abs()),
          reason: 'so it is wider than the gap, not hidden inside it');

      // Stacked one on the other is the same story, and the degenerate case
      // that used to draw a zero-length path.
      final stacked = graphWirePath(from, from).getBounds();
      expect(stacked.width, greaterThanOrEqualTo(graphWireStub),
          reason: 'even a wire onto itself is a visible loop out and back');

      // And the ordinary left-to-right wire is unchanged where the gap is
      // already wider than the stub.
      final wide = graphWirePath(const Offset(0, 0), const Offset(400, 0));
      expect(wide.getBounds().width, 400,
          reason: 'a long wire still runs socket to socket with no overshoot');
    });
  });
}
