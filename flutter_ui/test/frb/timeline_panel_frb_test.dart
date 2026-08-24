// The Timeline panel on frb, tested against the real engine.
//
// New coverage: the v0 Timeline's tests are spread across several files and
// written against a fake bridge and a snapshot mirror, neither of which this
// panel has. What they assert about *behaviour* is reproduced here against the
// document itself — a switch that does not reach the engine is not a switch.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/time_readout.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/state/clipboard.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:uuid/uuid.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/panels/graph_editor_frb.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/icons/icons.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/transform_rows_frb.dart';
import 'package:lumit_flutter/panels/waveform_frb.dart';
import 'package:lumit_flutter/state/timeline_columns.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Timeline (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      // The outline alone is 800 px of columns; the default 800×600 test
      // surface would push its right edge (and the lanes) off screen.
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
    }

    /// Open the toolbar's ⋯ menu, where the layer/work-area/marker commands
    /// live now that the toolbar row belongs to the readouts and the search.
    Future<void> openMore(WidgetTester tester) async {
      await tester.tap(find.byKey(const ValueKey('tl-more')));
      await tester.pumpAndSettle();
    }

    /// The Razor tool (K-220). Clicking a bar cuts that layer **where the
    /// pointer is**, not at the playhead — the difference between a razor and
    /// the Cut-at-playhead command.
    testWidgets('the razor splits a layer in two where it is clicked',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.tools.select(ToolMode.razor);
      p.uiState.model.refresh();
      await mount(tester, p);

      expect(p.comp.getLayers().length, 1);
      final span = layer.getSpan();

      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      expect(bar, findsOneWidget);
      final box = tester.getRect(bar);
      // A third of the way along the bar, well inside it.
      await tester.tapAt(Offset(box.left + box.width / 3, box.center.dy));
      await tester.pumpAndSettle();

      final after = p.comp.getLayers();
      expect(after.length, 2, reason: 'one layer became two');
      // The halves meet: the first ends where the second begins, and together
      // they cover exactly what the layer covered.
      final spans = [for (final l in after) l.getSpan()];
      final ins = [for (final s in spans) s.inPoint.num / s.inPoint.den];
      final outs = [for (final s in spans) s.outPoint.num / s.outPoint.den];
      ins.sort();
      outs.sort();
      expect(ins.first, closeTo(span.inPoint.num / span.inPoint.den, 1e-9));
      expect(outs.last, closeTo(span.outPoint.num / span.outPoint.den, 1e-9));
      expect(outs.first, closeTo(ins.last, 1e-9),
          reason: 'no gap and no overlap at the cut');
    });

    /// **Cut at playhead is a command, not a tool (docs/07 §4.4).** The chord
    /// went nowhere: `layer.split` was bound in the Timeline context but no
    /// handler answered it, so the only way to cut was to arm the razor.
    testWidgets('Ctrl+Shift+D cuts the selected layer at the playhead',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      final span = layer.getSpan();
      p.uiState.setSelection([layer]);
      p.uiState.playheadFrame.value = 12;
      p.uiState.model.refresh();
      await mount(tester, p);
      expect(p.uiState.tools.tool.group, isNot(ToolGroup.razor),
          reason: 'no razor armed: this is a command');

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyD);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();

      final after = p.comp.getLayers();
      expect(after.length, 2, reason: 'one layer became two');
      final spans = [for (final l in after) l.getSpan()];
      final ins = [for (final s in spans) s.inPoint.num / s.inPoint.den];
      final outs = [for (final s in spans) s.outPoint.num / s.outPoint.den];
      ins.sort();
      outs.sort();
      expect(ins.first, closeTo(span.inPoint.num / span.inPoint.den, 1e-9));
      expect(outs.last, closeTo(span.outPoint.num / span.outPoint.den, 1e-9));
      expect(outs.first, closeTo(ins.last, 1e-9),
          reason: 'the halves meet at the cut');
      // The playhead is where they meet: this cut is at the playhead, not
      // wherever a pointer happened to be.
      expect(
          outs.first,
          closeTo(
              p.comp.timeOfFrame(frame: 12).num /
                  p.comp.timeOfFrame(frame: 12).den,
              1e-9));
    });

    /// A cut with nothing selected, or one the engine refuses, is silence.
    testWidgets('Ctrl+Shift+D with nothing selected cuts nothing',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.playheadFrame.value = 12;
      p.uiState.model.refresh();
      await mount(tester, p);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyD);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();

      expect(p.comp.getLayers().length, 1);
    });

    /// The field a readout turns into when it is clicked.
    Finder fieldIn(String key) => find.descendant(
          of: find.byKey(ValueKey<String>(key)),
          matching: find.byType(EditableText),
        );

    /// The toolbar's two readouts are typed into, not merely read (K-287),
    /// and neither can send the playhead out of the composition.
    testWidgets('typing a timecode moves the playhead, clamped to the comp',
        (tester) async {
      final p = withComp();
      p.uiState.playheadFrame.value = 0;
      p.uiState.model.refresh();
      await mount(tester, p);
      final last = p.comp.durationFrames() - 1;
      final (fpsNum, fpsDen) = p.uiState.model.fpsExact;

      await tester.tap(find.byKey(const ValueKey('tl-timecode')));
      await tester.pump();
      await tester.enterText(fieldIn('tl-timecode'), '00:00:01:00');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, (fpsNum / fpsDen).ceil(),
          reason: 'a second in, counted at this comp\'s rate');

      await tester.tap(find.byKey(const ValueKey('tl-timecode')));
      await tester.pump();
      await tester.enterText(fieldIn('tl-timecode'), '99:00:00:00');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, last,
          reason: 'past the end of the comp is the end of the comp');
    });

    testWidgets('typing a frame number moves the playhead, clamped',
        (tester) async {
      final p = withComp();
      p.uiState.playheadFrame.value = 0;
      p.uiState.model.refresh();
      await mount(tester, p);
      final last = p.comp.durationFrames() - 1;

      await tester.tap(find.byKey(const ValueKey('tl-frame')));
      await tester.pump();
      await tester.enterText(fieldIn('tl-frame'), 'f42');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, 42,
          reason: 'the f the readout wears is optional on the way back in');

      await tester.tap(find.byKey(const ValueKey('tl-frame')));
      await tester.pump();
      await tester.enterText(fieldIn('tl-frame'), '-8');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, 0,
          reason: 'before the start is the start');

      await tester.tap(find.byKey(const ValueKey('tl-frame')));
      await tester.pump();
      await tester.enterText(fieldIn('tl-frame'), '999999');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, last);
    });

    /// **Both readouts sit in value wells** (K-460): the inset `surface_0`
    /// face inside a hairline that every editable number in the editor wears,
    /// because that recess is the whole of what says "you may type here".
    /// They were bare text that happened to answer a click.
    testWidgets('the timecode and the frame count rest in wells',
        (tester) async {
      final p = withComp();
      p.uiState.playheadFrame.value = 48;
      p.uiState.model.refresh();
      await mount(tester, p);
      final t = LumitTheme.dark();

      for (final key in ['tl-timecode', 'tl-frame']) {
        final box = tester.widget<Container>(find
            .descendant(
                of: find.byKey(ValueKey<String>(key)),
                matching: find.byType(Container))
            .first);
        final face = box.decoration as BoxDecoration;
        expect(face.color, t.surface0,
            reason: '$key rests in the well\'s own recess');
        expect((face.border as Border).top.color, t.hairline,
            reason: 'inside a hairline, like every other value well');
        expect(tester.getRect(find.byKey(ValueKey<String>(key))).height,
            closeTo(readoutWellHeight, 0.5),
            reason: 'and both wells are one height, whatever size their own '
                'type is — 11 for the clock, 10 for the frame count');
      }

      // And the open editor speaks through the **edge**, never by lifting the
      // fill: a well that rose under the pointer would stop being a recess
      // (§2.1). `animated`, not `accent` — the ring that means "you are about
      // to change a value" (§6.5).
      for (final key in ['tl-timecode', 'tl-frame']) {
        await tester.tap(find.byKey(ValueKey<String>(key)));
        await tester.pump();
        final open = tester.widget<Container>(find
            .descendant(
                of: find.byKey(ValueKey<String>(key)),
                matching: find.byType(Container))
            .first);
        final face = open.decoration as BoxDecoration;
        expect(face.color, t.surface0, reason: '$key stays inset while typing');
        expect((face.border as Border).top.color, t.animated,
            reason: 'and says so on its edge');
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
      }

      // The total is not editable, so it wears no well: a recess round it
      // would be an invitation the readout cannot accept.
      expect(
          find.descendant(
              of: find.byKey(const ValueKey('tl-frame')),
              matching: find.text('/ ${p.comp.durationFrames()}')),
          findsNothing);
    });

    /// **The frame count drops its `f` while it is being typed** (K-460): the
    /// letter names the clock rather than counting in it, so an edit that
    /// began by stepping over it began wrong. It goes back on at commit.
    testWidgets('editing the frame count edits the bare number',
        (tester) async {
      final p = withComp();
      p.uiState.playheadFrame.value = 48;
      p.uiState.model.refresh();
      await mount(tester, p);
      expect(find.text('f48'), findsOneWidget,
          reason: 'at rest it wears the f');

      await tester.tap(find.byKey(const ValueKey('tl-frame')));
      await tester.pump();
      expect(tester.widget<EditableText>(fieldIn('tl-frame')).controller.text,
          '48',
          reason: 'the field holds the number alone');

      await tester.enterText(fieldIn('tl-frame'), '60');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, 60);
      expect(find.text('f60'), findsOneWidget,
          reason: 'and it wears the f again the moment the edit lands');

      // Escape puts it back, exactly as §12A.3 says an edit is abandoned.
      await tester.tap(find.byKey(const ValueKey('tl-frame')));
      await tester.pump();
      await tester.enterText(fieldIn('tl-frame'), '9');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, 60,
          reason: 'Escape reverts, and the playhead never moved');
      expect(find.text('f60'), findsOneWidget);
    });

    /// **The frame counter says how many frames there are** (§12A.1): the
    /// mockup's `f48 / 250`, the whole phrase in one muted colour. It said
    /// only `f48`, which left the reader with no idea how far in that was.
    testWidgets('the frame counter carries the comp\'s total', (tester) async {
      final p = withComp();
      p.uiState.playheadFrame.value = 3;
      p.uiState.model.refresh();
      await mount(tester, p);

      final total = p.comp.durationFrames();
      expect(find.text('f3'), findsOneWidget);
      expect(find.text('/ $total'), findsOneWidget,
          reason: 'the comp\'s whole length, after the frame in hand');
      expect(tester.widget<Text>(find.text('/ $total')).style?.color,
          LumitTheme.dark().textMuted,
          reason: 'in the same muted colour as the frame number it follows');
    });

    /// §12A.1's order for the row above the outline: the two readouts at the
    /// far left, the search well stretched across the middle, the Layers and
    /// Graph mode segments at the far right. The search field is the part that
    /// has been proposed for removal more than once — it stays.
    testWidgets('the search well sits between the readouts and the mode tabs',
        (tester) async {
      final p = withComp();
      await mount(tester, p);

      final search = find.byKey(const ValueKey('tl-search'));
      expect(search, findsOneWidget, reason: 'the layer search stays (§12A.1)');

      final timecode =
          tester.getRect(find.byKey(const ValueKey('tl-timecode')));
      final frame = tester.getRect(find.byKey(const ValueKey('tl-frame')));
      final well = tester.getRect(search);
      final layers =
          tester.getRect(find.byKey(const ValueKey('tl-view-lanes')));
      final graph = tester.getRect(find.byKey(const ValueKey('tl-graph')));

      // Left to right, no overlaps.
      expect(timecode.right, lessThanOrEqualTo(frame.left));
      expect(frame.right, lessThanOrEqualTo(well.left));
      expect(well.right, lessThanOrEqualTo(layers.left));
      expect(layers.right, lessThanOrEqualTo(graph.left));

      // All of them on one row, and the well is the part that stretches: it
      // takes what is left over, so it is wider than both readouts together.
      expect(well.center.dy, moreOrLessEquals(timecode.center.dy, epsilon: 2));
      expect(graph.center.dy, moreOrLessEquals(timecode.center.dy, epsilon: 2));
      expect(well.width, greaterThan(timecode.width + frame.width));
    });

    /// The two modes are words, not glyphs, and the one in force is the one
    /// wearing the frame (§12A.1). Clicking the other switches; §3.1 keeps the
    /// accent off both.
    testWidgets('the mode tabs read Layers and Graph, and switch',
        (tester) async {
      final p = withComp();
      await mount(tester, p);

      expect(find.text('LAYERS'), findsOneWidget);
      expect(find.text('GRAPH'), findsOneWidget);

      // Layers is in force to begin with, so the graph editor is not up.
      expect(find.byType(GraphEditorFrb), findsNothing);
      await tester.tap(find.byKey(const ValueKey('tl-graph')));
      await tester.pumpAndSettle();
      expect(find.byType(GraphEditorFrb), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('tl-view-lanes')));
      await tester.pumpAndSettle();
      expect(find.byType(GraphEditorFrb), findsNothing);
    });

    testWidgets('the razor is the toolbar tool, and undoes as one step',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.tools.select(ToolMode.razor);
      p.uiState.model.refresh();
      await mount(tester, p);

      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      final box = tester.getRect(bar);
      await tester.tapAt(Offset(box.left + box.width / 3, box.center.dy));
      await tester.pumpAndSettle();
      expect(p.comp.getLayers().length, 2);

      p.state.project!.undo();
      expect(p.comp.getLayers().length, 1,
          reason: 'a razor cut is one undo step (docs/07 §4.7)');
    });

    testWidgets(
        'with the Selection tool a click on a bar selects rather than'
        ' cutting', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      await tester.tap(bar);
      await tester.pumpAndSettle();

      expect(p.comp.getLayers().length, 1, reason: 'nothing was cut');
      expect(p.uiState.selectedLayer.value?.internallayerId,
          layer.internallayerId);
    });

    /// **Cutting a retimed layer gives each half an end of its own (K-221).**
    ///
    /// Both halves keep the whole speed map, so without a key at the cut the
    /// two ramps stay welded: bending one half's speed would bend the other's,
    /// because they are the same curve. The key goes in preserving the curve's
    /// shape, so the cut itself changes nothing that plays.
    /// Cut a layer at the middle of its bar with the razor, and hand back the
    /// halves.
    Future<List<LayerReference>> cutInHalf(
        WidgetTester tester, dynamic p, LayerReference layer) async {
      p.uiState.tools.select(ToolMode.razor);
      p.uiState.model.refresh();
      await mount(tester, p);

      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      final box = tester.getRect(bar);
      await tester.tapAt(Offset(box.left + box.width / 2, box.center.dy));
      await tester.pumpAndSettle();
      return (p.comp as CompositionReference).getLayers();
    }

    int keysOf(LayerReference layer) {
      final retime = layer.getRetimeProperty();
      return retime is BridgeScalar_Keyframed ? retime.field0.length : 0;
    }

    /// **A cut only keys a layer that has actually been retimed** (K-236).
    /// Switching Retime on installs the identity map, so the property being
    /// there says nothing about whether the layer has been retimed — and a cut
    /// that dropped keys into an untouched map left the user keys to notice and
    /// remove for a cut they had asked nothing else of.
    testWidgets('cutting a layer nobody retimed leaves its map alone',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      expect(layer.toggleRetimeProperty(), isTrue,
          reason: 'the identity map goes on');
      final keysBefore = keysOf(layer);

      final after = await cutInHalf(tester, p, layer);

      expect(after.length, 2, reason: 'it still cuts');
      for (final half in after) {
        expect(keysOf(half), keysBefore,
            reason: 'and puts no keys into a map nobody has shaped');
      }
    });

    testWidgets(
        'cutting a retimed layer puts a keyframe at the cut, on both'
        ' halves', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      expect(layer.toggleRetimeProperty(), isTrue);
      // Half speed, which is a map somebody has shaped: the layer's first
      // second shows the source's first half-second. Both halves of a cut
      // would otherwise share one curve, and bending one would bend the other.
      layer.setRetimeProperty(
        value: BridgeScalar.keyframed([
          for (final (frame, value) in [(0, 0.0), (60, 0.5)])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: frame),
              value: value,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      final keysBefore = keysOf(layer);
      expect(keysBefore, greaterThan(0));

      final after = await cutInHalf(tester, p, layer);

      expect(after.length, 2);
      for (final half in after) {
        expect(keysOf(half), keysBefore + 1,
            reason: 'both carry the key the cut added, so each half has an end '
                'of its own to hold');
      }
    });

    /// Masks appear in the fold-out under their own heading, and only once the
    /// layer has one — the same rule Effects follows (K-222).
    testWidgets('a masked layer grows a Masks heading in its twirl-down',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final twirl =
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}'));
      await tester.tap(twirl);
      await tester.pumpAndSettle();
      expect(find.text('Transform'), findsOneWidget);
      expect(find.text('Masks'), findsNothing,
          reason: 'an empty heading is a promise the row cannot keep');

      layer.addMask(
        mask: BridgeMask(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Ellipse',
          vertices: const [
            BridgeVertex(
                x: 0, y: 0, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 100, y: 0, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 100, y: 80, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
          ],
          closed: true,
          inverted: false,
          opacity: const BridgeScalar.static_(100),
          mode: BridgeMaskMode.add,
          feather: const BridgeScalar.static_(0),
          expansion: const BridgeScalar.static_(0),
          pathKeys: const [],
        ),
      );
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      expect(find.text('Masks'), findsOneWidget);
      // And it opens onto the mask itself.
      await tester.tap(find
          .byKey(ValueKey<String>('tl-group-${layer.internallayerId}/masks')));
      await tester.pumpAndSettle();
      expect(find.text('Ellipse'), findsOneWidget);

      // The invert switch writes through to the document.
      final masks = layer.getMasks();
      await tester.tap(
          find.byKey(ValueKey<String>('tl-mask-invert-${masks.single.id}')));
      await tester.pumpAndSettle();
      expect(layer.getMasks().single.inverted, isTrue);
    });

    /// Give [layer] a mask, mount, and open the twirls that show its row.
    Future<void> openMaskRow(WidgetTester tester, dynamic p,
        LayerReference layer, String name) async {
      layer.addMask(
        mask: BridgeMask(
          id: UuidValue.fromString(const Uuid().v4()),
          name: name,
          vertices: const [
            BridgeVertex(
                x: 0, y: 0, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 100, y: 0, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 100, y: 80, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
          ],
          closed: true,
          inverted: false,
          opacity: const BridgeScalar.static_(100),
          mode: BridgeMaskMode.add,
          feather: const BridgeScalar.static_(0),
          expansion: const BridgeScalar.static_(0),
          pathKeys: const [],
        ),
      );
      (p.uiState as LumitUiState).model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'masks', settle: true);
      expect(find.text(name), findsOneWidget);
    }

    /// **A mask's opacity was not undoable (K-234).** Its field wrote on every
    /// drag tick, so a drag left a stack of near-identical steps and one Ctrl+Z
    /// backed out a single percent — which looks like nothing happening. The
    /// drag is staged now, exactly as every other value row here stages its.
    testWidgets('dragging a mask opacity is ONE undo step', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');

      final id = layer.getMasks().single.id;
      final field = find.byKey(ValueKey<String>('tl-mask-opacity-$id'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(-3, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(stillValue(layer.getMasks().single.opacity), lessThan(100),
          reason: 'the drag reached the mask');

      p.state.project!.undo();
      expect(stillValue(layer.getMasks().single.opacity), 100,
          reason: 'ONE undo returns the opacity it had before the drag');
    });

    /// **A mask opacity drag shows while it is dragged** (K-240). The last of
    /// the three whole-list rows to preview: K-234 staged it so the drag was one
    /// undo step, which left the picture still until the button came up, and
    /// K-239 fixed exactly that for paint and shape art.
    testWidgets('a mask opacity drag shows before it commits', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');

      final id = layer.getMasks().single.id;
      final field = find.byKey(ValueKey<String>('tl-mask-opacity-$id'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(-3, 0));
        await tester.pump();
      }

      expect(stillValue(layer.getMasks().single.opacity), 100,
          reason: 'a drag in flight writes nothing');
      expect(find.descendant(of: field, matching: find.textContaining('100%')),
          findsNothing,
          reason: 'the row shows the value being dragged, not the stored one');
      expect(tester.takeException(), isNull,
          reason: 'the preview request is a courtesy and never a crash');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(stillValue(layer.getMasks().single.opacity), lessThan(100),
          reason: 'the release is what commits');
    });

    /// Drag [field] left by [ticks] steps, releasing unless told otherwise —
    /// the same gesture the opacity tests above make by hand.
    Future<TestGesture> dragLeft(WidgetTester tester, Finder field, int ticks,
        {bool release = true}) async {
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      for (var i = 0; i < ticks; i++) {
        await gesture.moveBy(const Offset(-3, 0));
        await tester.pump();
      }
      if (release) {
        await gesture.up();
        await tester.pumpAndSettle();
      }
      return gesture;
    }

    /// **A mask's mode is on its row (K-222).** How a mask combines with the
    /// ones above it is a document edit like any other: one pick, one op, one
    /// undo step.
    testWidgets('the mask mode dropdown writes through to the document',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');

      final id = layer.getMasks().single.id;
      expect(layer.getMasks().single.mode, BridgeMaskMode.add,
          reason: 'a new mask adds to what is already let through');

      await tester.tap(find.byKey(ValueKey<String>('tl-mask-mode-$id')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Subtract'));
      await tester.pumpAndSettle();

      expect(layer.getMasks().single.mode, BridgeMaskMode.subtract);

      p.state.project!.undo();
      expect(layer.getMasks().single.mode, BridgeMaskMode.add,
          reason: 'ONE undo puts the mode back');
    });

    /// **Feather is a row under the mask, in layer pixels.** Staged and
    /// previewed exactly as the opacity is, so the drag is one op and one undo
    /// step (K-234, K-240).
    testWidgets('dragging a mask feather is ONE undo step and previews first',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      layer.setMask(
          mask: maskWith(layer.getMasks().single,
              feather: const BridgeScalar.static_(20),
              expansion: const BridgeScalar.static_(5)));
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      final id = layer.getMasks().single.id;
      final field = find.byKey(ValueKey<String>('tl-mask-feather-$id'));
      expect(find.text('Feather'), findsOneWidget);

      final gesture = await dragLeft(tester, field, 20, release: false);
      expect(stillValue(layer.getMasks().single.feather), 20,
          reason: 'a drag in flight writes nothing');
      expect(tester.takeException(), isNull,
          reason: 'the preview request is a courtesy and never a crash');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(stillValue(layer.getMasks().single.feather), lessThan(20),
          reason: 'the release is what commits');

      p.state.project!.undo();
      expect(stillValue(layer.getMasks().single.feather), 20,
          reason: 'ONE undo returns the feather it had before the drag');
    });

    /// **A feather cannot go negative** — it is the width of a soft edge, so
    /// the field simply has no negative side to drag into.
    testWidgets('a mask feather stops at zero', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');

      final id = layer.getMasks().single.id;
      await dragLeft(
          tester, find.byKey(ValueKey<String>('tl-mask-feather-$id')), 40);

      expect(stillValue(layer.getMasks().single.feather), 0,
          reason: 'dragging past zero offers nothing below it');
    });

    /// **Expansion grows and shrinks the shape**, so unlike feather it is free
    /// to go negative — and it is the same one-op, previewed drag.
    testWidgets('dragging a mask expansion is ONE undo step and goes negative',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');

      final id = layer.getMasks().single.id;
      final field = find.byKey(ValueKey<String>('tl-mask-expansion-$id'));
      expect(find.text('Expansion'), findsOneWidget);

      final gesture = await dragLeft(tester, field, 20, release: false);
      expect(stillValue(layer.getMasks().single.expansion), 0,
          reason: 'a drag in flight writes nothing');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(stillValue(layer.getMasks().single.expansion), lessThan(0),
          reason: 'a mask can be shrunk as well as grown');

      p.state.project!.undo();
      expect(stillValue(layer.getMasks().single.expansion), 0,
          reason: 'ONE undo returns the expansion it had before the drag');
    });

    /// **Every mask value keyframes, with the same stopwatch as everything
    /// else** (K-340). The branch that added mask animation exposed none of it
    /// to the frontend: there was no Path row at all, and no mask property
    /// carried a clock.
    testWidgets('every mask property has a stopwatch and keys with it',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;

      // All four rows exist, shape first.
      expect(find.text('Path'), findsOneWidget);
      expect(find.text('Opacity'), findsOneWidget);
      expect(find.text('Feather'), findsOneWidget);
      expect(find.text('Expansion'), findsOneWidget);

      for (final name in ['opacity', 'feather', 'expansion']) {
        final stopwatch =
            find.byKey(ValueKey<String>('kf-stopwatch-tl-mask-$name-$id'));
        expect(stopwatch, findsOneWidget, reason: '$name has no stopwatch');
        await tester.tap(stopwatch);
        await tester.pumpAndSettle();
      }

      final mask = layer.getMasks().single;
      for (final scalar in [mask.opacity, mask.feather, mask.expansion]) {
        expect(scalar, isA<BridgeScalar_Keyframed>(),
            reason: 'the stopwatch planted a key holding what was there');
      }
      // Turning it on never moves the picture: the key holds the value that
      // was already showing.
      expect(
          sampleScalar(
              scalar: mask.opacity, time: p.comp.timeOfFrame(frame: 0)),
          100);
    });

    /// **The shape keyframes too, and its row is diamonds without a field**
    /// (K-339, K-340): a path has no number to put in one.
    testWidgets('the mask path row keys the shape and shows no value field',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;

      expect(find.byKey(ValueKey<String>('tl-mask-path-$id')), findsNothing,
          reason: 'a shape has no single number, so the row has no field');

      expect(layer.getMasks().single.pathKeys, isEmpty);
      await tester
          .tap(find.byKey(ValueKey<String>('kf-stopwatch-tl-mask-path-$id')));
      await tester.pumpAndSettle();
      expect(layer.getMasks().single.pathKeys, hasLength(1),
          reason: 'the stopwatch planted a key on the shape');

      // And off again keeps the shape rather than dropping it.
      final before = layer.getMasks().single.vertices.length;
      await tester
          .tap(find.byKey(ValueKey<String>('kf-stopwatch-tl-mask-path-$id')));
      await tester.pumpAndSettle();
      expect(layer.getMasks().single.pathKeys, isEmpty);
      expect(layer.getMasks().single.vertices, hasLength(before));
    });

    /// **A mask's rows select like every other property row** (K-341), and a
    /// keyed one puts its diamonds on the lane. Both were missing: a mask value
    /// row could not be picked at all, so its curve never reached the graph,
    /// and a key planted on one left the lane empty.
    testWidgets('mask rows select and show their keys on the lane',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;

      // Every one of the four rows picks itself when its name is clicked —
      // the same as a transform or an effect parameter row.
      final masks = masksPath(layer.internallayerId.toString());
      for (final value in MaskValue.values) {
        await tester.tap(find.text(maskValueLabel(value)));
        await tester.pump();
        expect(p.uiState.selectedProperties.value, ['$masks/$id/${value.name}'],
            reason: '${value.name} did not select when its row was clicked');
      }

      // And a key planted on one appears on its lane.
      expect(
          find.byKey(ValueKey<String>('tl-keys-${masksPath(
            layer.internallayerId.toString(),
          )}/$id/opacity')),
          findsNothing);
      await tester.tap(
          find.byKey(ValueKey<String>('kf-stopwatch-tl-mask-opacity-$id')));
      await tester.pumpAndSettle();
      expect(
          find.byKey(ValueKey<String>('tl-keys-${masksPath(
            layer.internallayerId.toString(),
          )}/$id/opacity')),
          findsOneWidget,
          reason:
              'the key planted by the stopwatch has no diamond on the lane');
    });

    /// **A mask row stays picked after the mouse comes up** (K-343). Reported
    /// from the app: "if I click one it briefly looks like it selects while the
    /// mouse is down then it resets." The row selected on the press and the
    /// ground under the outline took the *tap* on the release, clearing it —
    /// because a `Listener` never competes in the gesture arena, and a mask row
    /// had nothing else that did.
    testWidgets('a mask row stays picked when the press is released',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;
      final masks = masksPath(layer.internallayerId.toString());

      for (final value in MaskValue.values) {
        final gesture = await tester
            .startGesture(tester.getCenter(find.text(maskValueLabel(value))));
        await tester.pump();
        expect(p.uiState.selectedProperties.value, ['$masks/$id/${value.name}'],
            reason: '${value.name} did not pick on the press');
        await gesture.up();
        await tester.pump();
        expect(p.uiState.selectedProperties.value, ['$masks/$id/${value.name}'],
            reason: '${value.name} lost the selection when the mouse came up');
      }

      // The mask's own row, the same way.
      final gesture =
          await tester.startGesture(tester.getCenter(find.text('Ellipse')));
      await tester.pump();
      await gesture.up();
      await tester.pump();
      expect(p.uiState.selectedProperties.value, ['$masks/$id'],
          reason: 'the mask row lost the selection when the mouse came up');
    });

    /// **The Viewer picking a row, then a click on that row, keeps it picked**
    /// (K-343). Reported from the app: after a mask path drag planted a key the
    /// Path row lit up correctly, and clicking it then dropped the selection
    /// and would not take it back.
    testWidgets('a row the Viewer picked stays picked when clicked',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;
      final path = '${masksPath(layer.internallayerId.toString())}/$id/path';

      // What the gizmo does when a drag has written a path key.
      p.uiState.requestSelectProperty(path);
      await tester.pump();
      expect(p.uiState.selectedProperties.value, [path],
          reason: 'the Viewer request did not reach the Timeline');

      await tester.tap(find.text(maskValueLabel(MaskValue.path)));
      await tester.pump();
      expect(p.uiState.selectedProperties.value, [path],
          reason: 'clicking the row the Viewer picked dropped the selection');

      // And a second click still leaves it picked.
      await tester.tap(find.text(maskValueLabel(MaskValue.path)));
      await tester.pump();
      expect(p.uiState.selectedProperties.value, [path],
          reason: 'the row could not be picked again');
    });

    /// **Picking a property row says which layer it belongs to** (K-341), so
    /// the Viewer can outline that layer and its masks. Before this the
    /// selection stayed inside the Timeline and the picture showed nothing.
    testWidgets('picking a mask row publishes it to the shell', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');

      // Opening the fold already picked the Masks heading (K-300), so what
      // matters is that the pick *moves* to the mask when its row is clicked.
      await tester.tap(find.text('Ellipse'));
      await tester.pump();
      expect(p.uiState.selectedProperties.value, hasLength(1));
      expect(p.uiState.selectedProperties.value.single,
          '${masksPath(layer.internallayerId.toString())}/${layer.getMasks().single.id}');
    });

    /// **A mask row is a property row (K-234).** It joins the same selection
    /// every other row is in, so it lights up, the heading holding it marks
    /// itself, and Delete has something to act on.
    testWidgets('clicking a mask selects its row', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');

      final t = LumitTheme.dark();
      Color? fillOver(String text) {
        final box = find.ancestor(
            of: find.text(text), matching: find.byType(Container));
        return (tester.widget<Container>(box.first).decoration as BoxDecoration)
            .color;
      }

      expect(fillOver('Ellipse'), isNull, reason: 'nothing picked to start');

      await tester.tap(find.text('Ellipse'));
      await tester.pump();

      expect(fillOver('Ellipse'), t.selectionFill,
          reason: 'the mask row is the one selected');
      expect(fillOver('Masks'), t.selectionFill.withValues(alpha: 0.45),
          reason: 'the heading holding it marks itself, a shade dimmer');
    });

    /// **Delete removes the selected mask (K-234).** The shell's Delete deletes
    /// the selected *layers*; with a mask row picked it stands down and this
    /// claim runs instead, so the key acts on what is actually selected rather
    /// than on the layer the mask sits on.
    testWidgets('Delete removes a selected mask and leaves its layer',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      // The layer is selected too, which is the case that used to delete it.
      p.uiState.setSelection([layer]);
      await tester.pump();

      final claim = p.uiState.deleteClaim;
      expect(claim, isNotNull, reason: 'the Timeline claims Delete');
      expect(claim!(), isFalse,
          reason: 'with no mask picked the shell keeps the key');

      await tester.tap(find.text('Ellipse'));
      await tester.pump();
      expect(p.uiState.deleteClaim!(), isTrue,
          reason: 'with a mask picked the Timeline takes it');
      await tester.pumpAndSettle();

      expect(layer.getMasks(), isEmpty, reason: 'the mask is gone');
      expect(p.comp.getLayers(), hasLength(1),
          reason: 'and its layer is still there');
      expect(find.text('Masks'), findsNothing,
          reason: 'the heading goes with the last mask under it');
    });

    /// Two clicks inside the double-tap window, the way a person double-clicks
    /// a name. The rows count their own timestamps rather than taking an
    /// `onDoubleTap`, so nothing here waits for a recogniser.
    Future<void> doubleClick(WidgetTester tester, Finder target) async {
      await tester.tap(target);
      await tester.pump();
      await tester.tap(target);
      await tester.pump();
    }

    /// **A shape is named after the tool that drew it, and renamed by hand.**
    /// The default naming already worked; nothing could change it afterwards,
    /// so an ellipse and a second ellipse were both just "Ellipse". A
    /// double-click on the name opens the editor, and the commit is one write
    /// through `setMask` — one op, one undo step, like every other mask edit
    /// (K-234).
    testWidgets('double-clicking a mask name renames it in ONE undo step',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;

      await doubleClick(
          tester, find.byKey(ValueKey<String>('tl-mask-name-$id')));
      final editor = find.byKey(ValueKey<String>('tl-mask-rename-$id'));
      expect(editor, findsOneWidget, reason: 'the name became a field');

      await tester.enterText(editor, '  Left eye  ');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(layer.getMasks().single.name, 'Left eye',
          reason: 'the surrounding whitespace is not part of the name');
      expect(find.byKey(ValueKey<String>('tl-mask-rename-$id')), findsNothing,
          reason: 'submitting leaves the editor');

      p.state.project!.undo();
      expect(layer.getMasks().single.name, 'Ellipse',
          reason: 'ONE undo puts the tool default back');
    });

    /// Escape abandons the edit: the mask keeps the name it had, and nothing
    /// reaches the document.
    testWidgets('Escape cancels a mask rename', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;

      await doubleClick(
          tester, find.byKey(ValueKey<String>('tl-mask-name-$id')));
      final editor = find.byKey(ValueKey<String>('tl-mask-rename-$id'));
      await tester.enterText(editor, 'Never typed this');
      await tester.pump();

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();

      expect(layer.getMasks().single.name, 'Ellipse',
          reason: 'the abandoned edit was never written');
      expect(find.byKey(ValueKey<String>('tl-mask-rename-$id')), findsNothing,
          reason: 'Escape closes the editor');
    });

    /// A nameless mask is worse than one named after its tool, so an empty (or
    /// all-space) name is refused and the old name stands.
    testWidgets('an empty mask name is refused', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;

      await doubleClick(
          tester, find.byKey(ValueKey<String>('tl-mask-name-$id')));
      await tester.enterText(
          find.byKey(ValueKey<String>('tl-mask-rename-$id')), '   ');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(layer.getMasks().single.name, 'Ellipse',
          reason: 'the mask keeps the name it had');
      expect(find.text('Ellipse'), findsOneWidget);
    });

    /// **The regression that matters.** A single tap on the name still selects
    /// the row and nothing else — selection is what `Delete` acts on (K-234),
    /// so a rename that opened on one click would take the key away from it.
    testWidgets('a single tap on a mask name selects and does not rename',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;

      await tester.tap(find.byKey(ValueKey<String>('tl-mask-name-$id')));
      await tester.pump();

      expect(find.byKey(ValueKey<String>('tl-mask-rename-$id')), findsNothing,
          reason: 'one click is not a rename');
      expect(p.uiState.deleteClaim!(), isTrue,
          reason: 'the one click selected the mask, so Delete acts on it');
    });

    /// The rename is also on the row's own menu, beside Delete — a double-click
    /// is not discoverable on its own.
    testWidgets('the mask menu renames', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      final id = layer.getMasks().single.id;

      await tester.tapAt(
          tester.getCenter(find.byKey(ValueKey<String>('tl-mask-name-$id'))),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey<String>('tl-mask-rename-menu-$id')));
      await tester.pumpAndSettle();

      final editor = find.byKey(ValueKey<String>('tl-mask-rename-$id'));
      expect(editor, findsOneWidget, reason: 'the menu opened the editor');
      await tester.enterText(editor, 'Vignette');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(layer.getMasks().single.name, 'Vignette');
    });

    /// Paint strokes list under their own heading, between Masks and Effects —
    /// the order the picture is built in (K-227).
    testWidgets('a painted layer grows a Paint heading in its twirl-down',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final twirl =
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}'));
      await tester.tap(twirl);
      await tester.pumpAndSettle();
      expect(find.text('Paint'), findsNothing,
          reason: 'an empty heading is a promise the row cannot keep');

      layer.addStroke(
        stroke: BridgeStroke(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Brush 1',
          points: const [
            BridgeStrokePoint(x: 10, y: 10),
            BridgeStrokePoint(x: 40, y: 25),
          ],
          colour: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
          width: 20,
          hardness: 0.8,
          opacity: 100,
          mode: BridgePaintMode.paint,
          cloneOffsetX: 0,
          cloneOffsetY: 0,
        ),
      );
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      expect(find.text('Paint'), findsOneWidget);
      await tester.tap(find
          .byKey(ValueKey<String>('tl-group-${layer.internallayerId}/paint')));
      await tester.pumpAndSettle();
      expect(find.text('Brush 1'), findsOneWidget);

      // And the row's opacity writes through to the document.
      final stroke = layer.getPaint().single;
      await tester
          .tap(find.byKey(ValueKey<String>('tl-stroke-opacity-${stroke.id}')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(ValueKey<String>('tl-stroke-opacity-${stroke.id}')), '40');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(layer.getPaint().single.opacity, 40);
    });

    /// **A stroke's opacity was not undoable.** The same fault the mask row had
    /// under K-234, and for the same reason: the row was written from the mask
    /// row as it stood *before* that fix, so it committed on every tick of the
    /// drag. A drag left a stack of near-identical ops and one `Ctrl+Z` backed
    /// out a single percent, which reads as undo not working at all.
    testWidgets('dragging a stroke opacity is ONE undo step', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addStroke(
        stroke: BridgeStroke(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Brush 1',
          points: const [
            BridgeStrokePoint(x: 10, y: 10),
            BridgeStrokePoint(x: 40, y: 25),
          ],
          colour: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
          width: 20,
          hardness: 0.8,
          opacity: 100,
          mode: BridgePaintMode.paint,
          cloneOffsetX: 0,
          cloneOffsetY: 0,
        ),
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'paint', settle: true);

      final id = layer.getPaint().single.id;
      final field = find.byKey(ValueKey<String>('tl-stroke-opacity-$id'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(-3, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(layer.getPaint().single.opacity, lessThan(100),
          reason: 'the drag reached the stroke');

      p.state.project!.undo();
      expect(layer.getPaint().single.opacity, 100,
          reason: 'ONE undo returns the opacity it had before the drag');
    });

    /// **A dragged stroke opacity shows while it is dragged** (K-239).
    ///
    /// Staging the drag made it one undo step (K-238) but stopped the picture
    /// moving until the button came up — the wrong half of the bargain. The
    /// tick previews and the release commits, so the row reads the value under
    /// the pointer while the document still holds the old one.
    testWidgets('a stroke opacity drag shows before it commits',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addStroke(
        stroke: BridgeStroke(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Brush 1',
          points: const [
            BridgeStrokePoint(x: 10, y: 10),
            BridgeStrokePoint(x: 40, y: 25),
          ],
          colour: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
          width: 20,
          hardness: 0.8,
          opacity: 100,
          mode: BridgePaintMode.paint,
          cloneOffsetX: 0,
          cloneOffsetY: 0,
        ),
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'paint', settle: true);

      final id = layer.getPaint().single.id;
      final field = find.byKey(ValueKey<String>('tl-stroke-opacity-$id'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(-3, 0));
        await tester.pump();
      }

      // Mid-drag: the row is showing the value under the pointer, and asking
      // the engine to draw it — but nothing has been written.
      expect(layer.getPaint().single.opacity, 100,
          reason: 'a drag in flight writes nothing');
      expect(find.descendant(of: field, matching: find.textContaining('100%')),
          findsNothing,
          reason: 'the row shows the value being dragged, not the stored one');
      expect(tester.takeException(), isNull,
          reason: 'the preview request is a courtesy and never a crash');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(layer.getPaint().single.opacity, lessThan(100),
          reason: 'the release is what commits');
    });

    /// A shape layer lists its art under a Contents heading, above Masks and
    /// Effects — the order the picture is built in (K-237).
    testWidgets('a shape layer grows a Contents heading in its twirl-down',
        (tester) async {
      final p = withComp();
      BridgeVertex corner(double x, double y) => BridgeVertex(
          x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0);
      final layer = p.comp.addShapeLayer(
        name: 'Rectangle',
        contents: [
          BridgeShapeItem(
            id: UuidValue.fromString(const Uuid().v4()),
            name: 'Rectangle',
            vertices: [
              corner(0, 0),
              corner(60, 0),
              corner(60, 40),
              corner(0, 40),
            ],
            closed: true,
            fill: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
            stroke: null,
            strokeWidth: 0,
            opacity: 100,
          ),
        ],
      );
      p.uiState.model.refresh();
      await mount(tester, p);

      await openFold(tester, layer.internallayerId, settle: true);
      expect(find.text('Contents'), findsOneWidget);

      await tester.tap(find.byKey(
          ValueKey<String>('tl-group-${layer.internallayerId}/contents')));
      await tester.pumpAndSettle();
      expect(find.text('Rectangle'), findsWidgets);

      // The row's opacity writes through to the document.
      final item = layer.getShapeContents().single;
      await tester
          .tap(find.byKey(ValueKey<String>('tl-shape-opacity-${item.id}')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(ValueKey<String>('tl-shape-opacity-${item.id}')), '30');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.opacity, 30);
    });

    /// A shape layer's art gets the same rename as a mask: it too arrives named
    /// after the tool that drew it, and one write through `setShapeContents`
    /// makes the change one op and one undo step.
    testWidgets('a shape item renames from its name and its menu',
        (tester) async {
      final p = withComp();
      BridgeVertex corner(double x, double y) => BridgeVertex(
          x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0);
      final layer = p.comp.addShapeLayer(
        name: 'Ellipse',
        contents: [
          BridgeShapeItem(
            id: UuidValue.fromString(const Uuid().v4()),
            name: 'Ellipse',
            vertices: [corner(0, 0), corner(60, 0), corner(60, 40)],
            closed: true,
            fill: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
            stroke: null,
            strokeWidth: 0,
            opacity: 100,
          ),
        ],
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'contents', settle: true);

      final id = layer.getShapeContents().single.id;
      final name = find.byKey(ValueKey<String>('tl-shape-name-$id'));
      final editorKey = ValueKey<String>('tl-shape-rename-$id');

      // (That one click is not a rename is asserted on the mask row, which
      // shares this editor; a single tap here would sit inside the double-click
      // window of the next one and open it.)
      await doubleClick(tester, name);
      expect(find.byKey(editorKey), findsOneWidget,
          reason: 'the name became a field');
      await tester.enterText(find.byKey(editorKey), '  Iris  ');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.name, 'Iris');

      p.state.project!.undo();
      expect(layer.getShapeContents().single.name, 'Ellipse',
          reason: 'ONE undo puts the tool default back');
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      // Escape leaves the name alone, and the menu opens the same editor.
      await tester.tapAt(tester.getCenter(name), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(ValueKey<String>('tl-shape-rename-menu-$id')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(editorKey), 'Discarded');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.name, 'Ellipse',
          reason: 'Escape abandons the edit');
    });

    testWidgets('without a composition it says so', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.textContaining('Open a composition'), findsOneWidget);
    });

    /// Dropping footage with nothing open offers to make the composition it
    /// would go in, rather than dead-ending on the placeholder: the drag used
    /// to lift, show its feedback and drop into nothing.
    testWidgets('footage dropped on an empty Timeline offers a new comp',
        (tester) async {
      final p = freshProject();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      expect(p.uiState.selectedComp, isNull);

      await tester.pumpWidget(hostPanel(
        child: const Row(
          children: [
            SizedBox(width: 300, child: ProjectPanelFrb()),
            Expanded(child: TimelinePanelFrb()),
          ],
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();
      expect(find.textContaining('Open a composition'), findsOneWidget);

      final row =
          find.byKey(ValueKey<String>('project-row-${footage.internalid}'));
      final gesture = await tester.startGesture(tester.getCenter(row));
      await tester.pump(const Duration(milliseconds: 200));
      // Stepped, because one large move leaves the gesture arena resolving
      // the drag against the row's own recognisers.
      // 40 px a step: the test surface is 800 px wide whatever MediaQuery
      // says, so a bigger stride drops the drag off the edge of it.
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(40, 0));
        await tester.pump();
      }
      await gesture.up();
      // The dialog probes the dropped media before it opens, so it appears
      // after a real async round trip rather than on the next pump.
      await settleFrb(tester, minRounds: 8);

      expect(find.byKey(const ValueKey('comp-apply')), findsOneWidget,
          reason: 'the drop asks for the new comp settings');
      await tester.enterText(
          find.byKey(const ValueKey('comp-name')), 'From drop');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      final comp = p.uiState.selectedComp;
      expect(comp, isNotNull, reason: 'the new comp is fronted');
      expect(comp!.getSettings().name, 'From drop');
      expect(comp.getLayers(), hasLength(1),
          reason: 'the dropped footage landed in it as a layer');
    });

    testWidgets('New layer adds every kind, newest on top', (tester) async {
      final p = withComp();
      await mount(tester, p);

      for (final kind in [
        'Solid',
        'Text',
        'Camera',
        'Adjustment',
        'Null',
        'Sequence'
      ]) {
        await openMore(tester);
        await tester.tap(find.byKey(const ValueKey('tl-add-layer')));
        await tester.pumpAndSettle();
        await tester.tap(find.text(kind));
        await tester.pumpAndSettle();
      }

      final layers = p.comp.getLayers();
      expect(layers, hasLength(6));
      expect(layers.first.getKind(), BridgeLayerKind.sequence,
          reason: 'the newest layer is at the top of the stack');
      expect(
          find.byKey(
              ValueKey<String>('tl-row-${layers.first.internallayerId}')),
          findsOneWidget);
    });

    testWidgets('the switch column reaches the document', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final id = layer.internallayerId.toString();
      expect(layer.getSwitches().visible, isTrue);

      await tester.tap(find.byKey(ValueKey<String>('tl-visible-$id')));
      await tester.pump();
      expect(layer.getSwitches().visible, isFalse,
          reason: 'hiding a layer is a document edit, not a view state');

      await tester.tap(find.byKey(ValueKey<String>('tl-solo-$id')));
      await tester.pump();
      expect(layer.getSwitches().solo, isTrue);
      expect(layer.getSwitches().visible, isFalse,
          reason: 'one switch does not disturb another');
    });

    testWidgets('the blend dropdown commits by index', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      expect(layer.getBlend(), 0);
      final modes = listBlendModes();

      await tester.tap(
          find.byKey(ValueKey<String>('tl-blend-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.text(modes[2]).last);
      await tester.pumpAndSettle();

      expect(layer.getBlend(), 2,
          reason:
              'the index the dropdown shows is the index the engine stores');
    });

    testWidgets('the row menu duplicates, reorders and deletes',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final first = p.comp.getLayers().single;
      await tester.tapAt(
        tester.getCenter(
            find.byKey(ValueKey<String>('tl-row-${first.internallayerId}'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Duplicate'));
      await tester.pumpAndSettle();
      expect(p.comp.getLayers(), hasLength(2));

      // The bottom row can be brought forward but not sent back.
      final bottom = p.comp.getLayers()[1];
      await tester.tapAt(
        tester.getCenter(
            find.byKey(ValueKey<String>('tl-row-${bottom.internallayerId}'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      expect(find.text('Send backward'), findsNothing);
      await tester.tap(find.text('Bring forward'));
      await tester.pumpAndSettle();
      expect(p.comp.getLayers().first.internallayerId, bottom.internallayerId);

      await tester.tapAt(
        tester.getCenter(
            find.byKey(ValueKey<String>('tl-row-${bottom.internallayerId}'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();
      expect(p.comp.getLayers(), hasLength(1));
    });

    testWidgets('clicking the ruler scrubs the playhead', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      expect(p.uiState.playheadFrame.value, 0);
      final ruler = find.byKey(const ValueKey('tl-ruler'));
      final box = tester.getRect(ruler);
      await tester.tapAt(Offset(box.left + box.width * 0.5, box.center.dy));
      await tester.pump();

      final frames = p.comp.durationFrames();
      expect(p.uiState.playheadFrame.value, closeTo(frames * 0.5, 2),
          reason: 'the tap landed halfway along the comp');
      expect(p.uiState.playheadFrame.value, lessThan(frames),
          reason: 'the playhead never leaves the comp');
    });

    testWidgets('dragging a bar moves the layer as one op', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final before = layer.getSpan();
      final beforeIn = p.comp.frameAtTime(time: before.inPoint);

      final bar =
          find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
      final rect = tester.getRect(bar);
      // Well inside the bar, so this is a move rather than a trim.
      await tester.dragFrom(
        Offset(rect.left + rect.width * 0.5, rect.center.dy),
        const Offset(80, 0),
      );
      await tester.pumpAndSettle();

      final after = layer.getSpan();
      final afterIn = p.comp.frameAtTime(time: after.inPoint);
      expect(afterIn, greaterThan(beforeIn),
          reason: 'the bar moved later in the comp');

      // One op for the whole gesture: a single undo puts it back.
      p.state.project!.undo();
      expect(p.comp.frameAtTime(time: layer.getSpan().inPoint), beforeIn);
    });

    /// The mouse-acceleration bug: frames were rounded per pointer event and
    /// summed, so a slow drag's sub-frame deltas all rounded to nothing while
    /// a fast drag's rounded up — the bar moved a different distance than the
    /// pointer depending on speed. The frame delta must come from the pixel
    /// total. Fails without the `_deltaPx` accumulator.
    testWidgets('a slow drag moves the bar exactly as far as a fast one',
        (tester) async {
      final p = withComp();
      final fast = p.comp.addAdjustmentLayer();
      final slow = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      Future<void> dragBar(LayerReference layer, List<Offset> moves) async {
        final bar =
            find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
        final rect = tester.getRect(bar);
        final g = await tester
            .startGesture(Offset(rect.left + rect.width * 0.5, rect.center.dy));
        for (final m in moves) {
          await g.moveBy(m);
          await tester.pump();
        }
        await g.up();
        await tester.pumpAndSettle();
      }

      // Identical first events, so both gestures clear the touch slop the
      // same way — then the same 36 pixels: once in one event, once in 72
      // half-pixel events, the slow careful drag that used to fall behind
      // the pointer.
      await dragBar(fast, [const Offset(24, 0), const Offset(36, 0)]);
      await dragBar(slow, [
        const Offset(24, 0),
        for (var i = 0; i < 72; i++) const Offset(0.5, 0),
      ]);

      int inOf(LayerReference l) =>
          p.comp.frameAtTime(time: l.getSpan().inPoint);
      expect(inOf(fast), greaterThan(0), reason: 'the fast drag moved the bar');
      expect(inOf(slow), inOf(fast),
          reason: 'frames come from the pixel total, not per-event rounding');
    });

    /// Retime is an ordinary property row (K-197): hidden until the layer is
    /// given one, then sitting above Transform — outside it, not inside — and
    /// editable exactly like Opacity. Fails if it is filed under Transform, or
    /// if it shows on a layer with no Retime.
    testWidgets('Retime shows above Transform only once the layer has one',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId);
      expect(find.text('Retime'), findsNothing,
          reason: 'a layer with no Retime shows no row for it');

      layer.toggleRetimeProperty();
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.text('Retime'), findsOneWidget);
      expect(
        tester.getTopLeft(find.text('Retime')).dy,
        lessThan(tester.getTopLeft(find.text('Transform')).dy),
        reason: 'Retime sits above Transform, not inside it',
      );
      // Transform is still shut: a row that only appears when Transform is
      // twirled open would be inside it, whatever its indent says.
      expect(find.text('Opacity'), findsNothing);

      // The identity map is keyed, so the field edits the key at the playhead.
      List<BridgeKeyframe> keys() =>
          (layer.getRetimeProperty() as BridgeScalar_Keyframed).field0;
      expect(keys(), hasLength(2));
      p.uiState.playheadFrame.value = 0;
      await tester.pump();
      await tester.drag(
          find.byKey(const ValueKey('tl-retime-seconds')), const Offset(40, 0));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(2), reason: 'no key was added or lost');
      expect(keys().first.value, greaterThan(0),
          reason: 'the edit landed in the key under the playhead');
    });

    /// The Retime row reads as a clock, not as a decimal number of seconds
    /// (K-287, realising K-075) — and the Settings switch puts the seconds
    /// field back for anyone who wants sub-frame precision.
    testWidgets('Retime reads as a timecode, or as seconds when asked',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.toggleRetimeProperty();
      p.uiState.playheadFrame.value = 0;
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId);

      // Frame zero of the source at frame zero of the comp: an identity map
      // starts where the media does.
      Finder inRetimeRow(Finder matching) => find.descendant(
            of: find.byKey(const ValueKey('tl-retime-seconds')),
            matching: matching,
          );
      expect(inRetimeRow(find.text('00:00:00:00')), findsOneWidget,
          reason: 'the source position is a clock face');
      expect(inRetimeRow(find.textContaining(' s')), findsNothing,
          reason: 'and not a number of seconds');

      p.uiState.workspace.interface.retimeInSeconds = true;
      p.uiState.model.refresh();
      await tester.pump();
      expect(inRetimeRow(find.text('0.000 s')), findsOneWidget,
          reason: 'the setting puts the seconds field back');
    });

    /// The Retime clock counts *source* frames at the footage's own rate.
    /// Half a second into 600 fps footage is frame 300 — a clock at the comp's
    /// 60 fps would call the same moment frame 30, and could never say :599.
    testWidgets('the Retime clock runs at the footage rate, not the comp rate',
        (tester) async {
      final p = withComp();
      final footage =
          p.state.project!.importFootage(path: _highRateVideoFile('fast.y4m'));
      p.comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = p.comp.getLayers().single;
      layer.toggleRetimeProperty();
      // Half a second at the comp's 60 fps: the identity map reads 0.5 s of
      // source here.
      p.uiState.playheadFrame.value = 30;
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId);

      // The row probes the footage's rate over an async frb call; real
      // event-loop turns deliver the answer.
      await settleFrb(tester,
          until: () => find.text('00:00:00:300').evaluate().isNotEmpty);
      expect(find.text('00:00:00:300'), findsOneWidget,
          reason: 'the clock counts source frames at 600 fps, not comp frames');
    });

    /// An animated value stays editable in the outline (docs/07 §4.3): on a
    /// keyframe the edit lands in that key; between keyframes it plants one.
    /// Fails if the cell falls back to a read-only "animated" label, or if it
    /// writes a static value over the curve.
    testWidgets('editing an animated value edits the key under the playhead',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final (f, v) in [(0, 20.0), (60, 80.0)])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: v,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

      // On the first key: the drag edits that key, not the curve's shape.
      p.uiState.playheadFrame.value = 0;
      await tester.pump();
      await tester.drag(
          find.byKey(const ValueKey('tl-tf-opacity')), const Offset(40, 0));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(2), reason: 'no key was added or lost');
      expect(keys().first.value, greaterThan(20),
          reason: 'the edit landed in the key under the playhead');

      // Between keys: the drag plants a new one there.
      p.uiState.playheadFrame.value = 30;
      await tester.pump();
      await tester.drag(
          find.byKey(const ValueKey('tl-tf-opacity')), const Offset(40, 0));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(3),
          reason: 'editing between keys plants one at the playhead');
      expect(p.comp.frameAtTime(time: keys()[1].time), 30);
    });

    /// The ◆ button acts at the playhead's *current* frame — the diamond used
    /// to read the frame captured when the panel last drew, so after a scrub
    /// it removed the wrong key.
    testWidgets('the key diamond follows the playhead as it scrubs',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [0, 60])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

      // On the second key: ◆ removes it.
      p.uiState.playheadFrame.value = 60;
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('kf-toggle-tl-tf-opacity')));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(1), reason: 'the key under the playhead went');

      // Off any key: ◆ adds one exactly there.
      p.uiState.playheadFrame.value = 30;
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('kf-toggle-tl-tf-opacity')));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(2));
      expect(p.comp.frameAtTime(time: keys()[1].time), 30);
    });

    /// A shut layer says on its own row what is keyed inside it (§12A.1):
    /// the same diamonds, at half the scale, in `animated` rather than in
    /// `accent` — the accent's list is the playhead, the one filled button and
    /// the active tab tick, and nothing else (§3.1). Twirled open, the summary
    /// stands down and each property draws its own at full size.
    testWidgets('a shut layer shows its keys at half scale, in animated',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 1500])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);

      final summary =
          find.byKey(ValueKey<String>('tl-bar-keys-${layer.internallayerId}'));
      expect(summary, findsOneWidget,
          reason: 'a shut layer draws what is keyed inside it');

      final painter = tester
          .widget<CustomPaint>(
              find.descendant(of: summary, matching: find.byType(CustomPaint)))
          .painter as dynamic;
      expect((painter.frames as List).length, 2);
      expect(painter.colour, LumitTheme.dark().animated,
          reason: 'keys are animated, never accent');

      await openFold(tester, layer.internallayerId, group: 'Transform');
      expect(summary, findsNothing,
          reason: 'an open layer draws the real thing on each lane');

      final lane = tester
          .widget<CustomPaint>(find.descendant(
            of: find.byKey(ValueKey<String>(
                'tl-keys-${layer.internallayerId}/transform/opacity')),
            matching: find.byType(CustomPaint),
          ))
          .painter as dynamic;
      expect(painter.half, lessThan(lane.half),
          reason: 'the summary is smaller than the lane it summarises');
      expect(lane.colour, LumitTheme.dark().animated,
          reason: 'and both are animated');

      // The sizes themselves, not only their ratio: a diamond that shrank at
      // both ends would keep the ratio and lose the mark. A property's own
      // key is the drawing's 11 point to point in **both** modes (half 5.5,
      // K-459 — it was 8 here); the mockup's summary diamond is a 4px square
      // on its corner, 4√2 ≈ 5.7 point to point (half 2.8), and is unchanged.
      expect(lane.half, laneKeyHalf, reason: 'a lane key is 11 across');
      expect(painter.half, 2.8, reason: 'a summary diamond is 5.7 across');
    });

    /// Keyframes draw as diamonds on the lane (docs/07 §4.3), and a marquee
    /// dragged over empty lane space gathers them.
    testWidgets('lane diamonds appear and the marquee selects them',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      // Well apart on the axis, so the box can start on empty lane rather
      // than on a key's own drag handle.
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 1500])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      expect(find.byKey(laneKey), findsOneWidget,
          reason: 'an animated row draws its diamonds on the lane');

      Set<int> selected() {
        final paint = find.descendant(
          of: find.byKey(laneKey),
          matching: find.byType(CustomPaint),
        );
        return ((tester.widget<CustomPaint>(paint.first).painter as dynamic)
                .selected as Set<int>)
            .cast<int>();
      }

      expect(selected(), isEmpty);

      // A box over the whole lane row takes both keys.
      final rect = tester.getRect(find.byKey(laneKey));
      final gesture =
          await tester.startGesture(Offset(rect.left + 2, rect.top + 2));
      await tester.pump(const Duration(milliseconds: 100));
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(Offset(rect.width / 8, rect.height / 10));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();
      expect(selected(), hasLength(2),
          reason: 'the marquee gathered the keys it enclosed');
    });

    /// **Easing a key from the lanes.** Two things stopped F9 working in lane
    /// view: nothing selected a single diamond (only the marquee filled the
    /// catch), and the F9 family is bound in the *graph* context while the
    /// lookup only fell back the other way, so over the lanes the chord matched
    /// no action at all. Clicking a diamond and pressing F9 must ease that key.
    testWidgets('F9 eases a keyframe selected on the lane', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 2400])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;
      expect(keys().first.interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'the keys start linear');

      await tester.tap(find.byKey(ValueKey<String>(
          'tl-key-${layer.internallayerId}/transform/opacity#0')));
      await tester.pump();

      await tester.sendKeyEvent(LogicalKeyboardKey.f9);
      await tester.pumpAndSettle();

      expect(keys().first.interpOut, isA<BridgeSideInterp_Bezier>(),
          reason: 'F9 eased the key the lane click selected');
      expect(keys().last.interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'and only that one');
    });

    /// **The snap magnet's on-state is chrome, not accent** (§3.1): the accent
    /// list is closed — one filled button, the playhead, the workspace tick —
    /// and a tool toggle is not on it. On, the glyph reads at foreground
    /// strength on the button's own face; off, it is frameless and muted.
    testWidgets('the magnet lights in the foreground, never in the accent',
        (tester) async {
      final p = withComp();
      await mount(tester, p);
      final t = LumitTheme.dark();
      final magnet = find.byKey(const ValueKey('tl-magnet'));

      ColorFilter? tint() => tester
          .widget<SvgPicture>(
              find.descendant(of: magnet, matching: find.byType(SvgPicture)))
          .colorFilter;
      bool frameless() => tester.widget<HouseButton>(magnet).frameless;

      // On is the default.
      expect(tint(), ColorFilter.mode(t.textPrimary, BlendMode.srcIn),
          reason: 'the on glyph is text_primary');
      expect(tint(), isNot(ColorFilter.mode(t.accent, BlendMode.srcIn)));
      expect(frameless(), isFalse, reason: 'and it stands on its own face');

      await tester.tap(magnet);
      await tester.pumpAndSettle();
      expect(tint(), ColorFilter.mode(t.textMuted, BlendMode.srcIn),
          reason: 'off is muted');
      expect(frameless(), isTrue, reason: 'and frameless');
    });

    /// Dragging a lane diamond moves the keyframe in time — one op — and the
    /// magnet decides whether it lands on a whole frame or between two
    /// (docs/07 §4.5).
    testWidgets('a lane keyframe drags in time, and the magnet snaps it',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 2400])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;
      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      final handle = find.byKey(ValueKey<String>(
          'tl-key-${layer.internallayerId}/transform/opacity#0'));
      expect(handle, findsOneWidget, reason: 'each diamond is a drag handle');

      // Measured, not assumed: the axis is as wide as the panel leaves it,
      // and the columns can be resized, so the test asks how many pixels a
      // frame is worth rather than hard-coding one.
      // The row is the axis's whole width, padding included (§12A.1), so the
      // frames' own span is what one frame is worth in pixels.
      final perFrame =
          (tester.getRect(find.byKey(laneKey)).width - TimelineAxis.pad * 2) /
              p.comp.durationFrames();

      // Magnet on (the default): a drag of ten and a half frames still lands
      // on a whole one.
      await tester.drag(handle, Offset(perFrame * 10.5, 0));
      await tester.pumpAndSettle();
      final snapped = keys().first.time;
      expect(p.comp.frameAtTime(time: snapped), greaterThan(600),
          reason: 'the drag moved the key later');
      expect(snapped.num * 60 % snapped.den, 0,
          reason: 'with the magnet on it sits exactly on a frame');
      expect(keys(), hasLength(2), reason: 'no key added or lost');

      // One op for the gesture: a single undo puts it back.
      p.state.project!.undo();
      expect(p.comp.frameAtTime(time: keys().first.time), 600);

      // Magnet off: the same half-frame drag lands between two frames.
      await tester.tap(find.byKey(const ValueKey('tl-magnet')));
      await tester.pump();
      await tester.drag(handle, Offset(perFrame * 10.5, 0));
      await tester.pumpAndSettle();
      final free = keys().first.time;
      expect(free.num * 60 % free.den, isNot(0),
          reason: 'with the magnet off it may land between frames');
    });

    /// **A key lands on the marker it is dragged near** (docs/07 §4.5). The
    /// magnet used to cover exactly one snap — a whole frame — and the spec's
    /// other sources and targets were still to build. This is the one that
    /// matters most in use: beat-marker snapping is the beat-sync covenant's
    /// daily face, and a beat marker is an ordinary marker.
    testWidgets('a lane keyframe snaps onto a marker, and Ctrl lets it past',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 2400])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      // A marker a little past where a ten-frame drag would land, so the snap
      // has to reach *forwards* for it rather than the drag happening to hit.
      const markerFrame = 611;
      writeMarkers(p.comp, [
        BridgeMarker(
          id: UuidValue.fromString(const Uuid().v4()),
          time: p.comp.timeOfFrame(frame: markerFrame),
          label: 'Beat',
        ),
      ]);
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;
      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      final handle = find.byKey(ValueKey<String>(
          'tl-key-${layer.internallayerId}/transform/opacity#0'));
      // The row is the axis's whole width, padding included (§12A.1), so the
      // frames' own span is what one frame is worth in pixels.
      final perFrame =
          (tester.getRect(find.byKey(laneKey)).width - TimelineAxis.pad * 2) /
              p.comp.durationFrames();

      // Ten frames lands at 610 — one frame short of the marker, which at this
      // zoom is well inside the eight-pixel reach.
      await tester.drag(handle, Offset(perFrame * 10, 0));
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: keys().first.time), markerFrame,
          reason: 'the key landed ON the marker, not one frame short of it');

      p.state.project!.undo();
      expect(p.comp.frameAtTime(time: keys().first.time), 600);
      // The lane draws from the read model, so it has to be told the undo
      // happened before the next drag starts from where the key really is.
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      // Ctrl held suspends the snap, so the same drag lands where it was aimed
      // (docs/07 §4.5) — the way out when the wanted place is exactly where a
      // snap will not allow.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      addTearDown(
          () async => tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft));
      await tester.drag(handle, Offset(perFrame * 10, 0));
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: keys().first.time), 610,
          reason: 'Ctrl held let the key past the marker');
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    });

    /// **The one-frame regression.** A real drag is many pointer moves with a
    /// rebuild between each; the tests above are one move, which is the only
    /// reason they passed. Part-way through a real drag the snap indicator
    /// appears, and it used to be an unkeyed child inserted ahead of the
    /// diamonds — so Flutter paired it with the first diamond, the first
    /// diamond with the second, and rebuilt every gesture detector in the lane.
    /// The detector holding the pointer went with them, which ended the drag
    /// where it stood: the key committed the two or three pixels travelled so
    /// far and sat there however much further it was dragged, and a second drag
    /// died on the same target and put it back. Reported as "a keyframe can
    /// only be dragged one frame, and dragging again moves it back".
    testWidgets('a lane keyframe drags past a snap, over many pointer moves',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 2400])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      // A marker in the middle of the journey, so the drag is certain to be
      // caught by a snap on its way past — the moment the indicator appears.
      const markerFrame = 800;
      writeMarkers(p.comp, [
        BridgeMarker(
          id: UuidValue.fromString(const Uuid().v4()),
          time: p.comp.timeOfFrame(frame: markerFrame),
          label: 'Beat',
        ),
      ]);
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;
      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      final handle = find.byKey(ValueKey<String>(
          'tl-key-${layer.internallayerId}/transform/opacity#0'));
      // The row is the axis's whole width, padding included (§12A.1), so the
      // frames' own span is what one frame is worth in pixels.
      final perFrame =
          (tester.getRect(find.byKey(laneKey)).width - TimelineAxis.pad * 2) /
              p.comp.durationFrames();

      // The little push that gets the gesture past the pointer slop.
      const nudge = 3.0;

      // A drag as one really arrives: a nudge to start it, then a run of small
      // moves with a frame rendered between each. Returns the frame the key
      // ended on. A mouse, so the slop is a single pixel rather than a
      // finger's worth.
      Future<int> dragOn(double frames, {int steps = 18}) async {
        final gesture = await tester.startGesture(tester.getCenter(handle),
            kind: PointerDeviceKind.mouse);
        await gesture.moveBy(const Offset(nudge, 0));
        await tester.pump();
        for (var i = 0; i < steps; i++) {
          await gesture.moveBy(Offset(frames * perFrame / steps, 0));
          await tester.pump();
        }
        await gesture.up();
        await tester.pumpAndSettle();
        return p.comp.frameAtTime(time: keys().first.time);
      }

      // Four hundred frames of travel, measured in pixels from the axis so the
      // drag stays inside the comp whatever width the panel gives the lanes.
      const travel = 400.0;
      // The nudge that starts the drag is spent on the slop when something else
      // is in the gesture arena and counted when the diamond is alone in it, so
      // the landing is allowed its worth of frames either way. Either is a
      // world away from the fault, which left the key on the marker 200 frames
      // back.
      final slack = nudge / perFrame + 2;

      final landed = await dragOn(travel);
      expect(landed, isNot(markerFrame),
          reason: 'the drag went past the marker rather than dying on it');
      expect(landed.toDouble(), closeTo(600 + travel, slack),
          reason: 'the key travelled the whole drag, not its first moments');
      expect(keys(), hasLength(2), reason: 'no key added or lost');

      // And again from where it now is: the second drag carries on rather than
      // being pulled back to what caught the first.
      final again = await dragOn(travel);
      expect(again.toDouble(), closeTo(landed + travel, slack),
          reason: 'a second drag moves it on again, not back');
    });

    /// **The undo regression.** A drag on a *keyframed* value used to commit
    /// on every tick — [DragValueField] falls back to `onChanged` per tick
    /// when no `onChangeLive` is given — so the undo stack filled with a step
    /// per pixel and one undo moved the value back by a hair. The whole
    /// gesture must be a single step, back to the value before the drag.
    testWidgets('a drag on a keyframed value is one undo step', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final (f, v) in [(0, 20.0), (60, 80.0)])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: v,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

      // On the first key, dragged in many small steps — the shape that used
      // to write one op each.
      p.uiState.playheadFrame.value = 0;
      await tester.pump();
      final field = find.byKey(const ValueKey('tl-tf-opacity'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(3, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(keys().first.value, greaterThan(20),
          reason: 'the drag reached the key');
      expect(keys(), hasLength(2), reason: 'and planted nothing extra');

      p.state.project!.undo();
      expect(keys().first.value, 20,
          reason: 'ONE undo returns the value it had before the drag');
    });

    /// Clicking a property row selects it, and everything containing it —
    /// its group heading and its layer's row — marks itself, so switching to
    /// the graph knows which curve is meant (docs/07 §4.3).
    testWidgets('clicking a property selects it and marks its parents',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      await mount(tester, p);
      final id = layer.internallayerId;

      await openFold(tester, id, group: 'Effects');
      await tester.tap(find.text('Gaussian blur'));
      await tester.pump();

      final t = LumitTheme.dark();
      // The innermost Container over a row's label is that row's own.
      Color? fillOver(String text) {
        final box = find.ancestor(
            of: find.text(text), matching: find.byType(Container));
        return (tester.widget<Container>(box.first).decoration as BoxDecoration)
            .color;
      }

      expect(fillOver('Radius'), isNull,
          reason: 'nothing is picked to start with');

      await tester.tap(find.text('Radius'));
      await tester.pump();

      expect(fillOver('Radius'), t.selectionFill,
          reason: 'the property row is the one selected');
      expect(fillOver('Gaussian blur'), t.selectionFill.withValues(alpha: 0.45),
          reason: 'the effect holding it marks itself, a shade dimmer');
      expect(
          (tester
                  .widget<Container>(
                      find.byKey(ValueKey<String>('tl-rowbody-$id')))
                  .decoration as BoxDecoration)
              .color,
          t.selectionFill.withValues(alpha: 0.45),
          reason: "and so does the property's layer");
    });

    /// **Any press that acts on a row selects it** (K-334): the stopwatch, the
    /// navigator, a value drag. Touching a row's controls is choosing it — and
    /// it is what puts the channel in the graph before a drag's first tick.
    testWidgets('pressing a row control selects the row', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      await openFold(tester, id, group: 'Transform');

      final t = LumitTheme.dark();
      Color? fillOver(String text) {
        final box = find.ancestor(
            of: find.text(text), matching: find.byType(Container));
        return (tester.widget<Container>(box.first).decoration as BoxDecoration)
            .color;
      }

      expect(fillOver('Opacity'), isNull, reason: 'nothing picked to start');

      // The stopwatch, not the label.
      await tester.tap(
          find.byKey(const ValueKey<String>('kf-stopwatch-tl-tf-opacity')));
      await tester.pump();
      expect(fillOver('Opacity'), t.selectionFill,
          reason: 'pressing the stopwatch chose the row');
    });

    /// **Picking a layer on the picture reaches the Timeline** (K-275).
    ///
    /// The Viewer's click goes straight to the shell's selection
    /// (`setSelection`), never through this panel's own click path — so the
    /// property selection, the graph's keys and the row highlight, all of which
    /// belong to the layer that *was* chosen, stayed behind. The previous
    /// layer's rows kept their fill while a different layer was selected: two
    /// layers appearing chosen at once, which is what K-203 set out to remove.
    testWidgets('a selection made outside the panel clears the property one',
        (tester) async {
      final p = withComp();
      final first = p.comp.addSolidLayer();
      first.addEffect(name: 'blur');
      final second = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = first.internallayerId;

      // Select a property on the first layer, the ordinary way.
      await openFold(tester, id, group: 'Effects');
      await tester.tap(find.text('Gaussian blur'));
      await tester.pump();
      await tester.tap(find.text('Radius'));
      await tester.pump();

      final t = LumitTheme.dark();
      Color? fillOver(String text) {
        final box = find.ancestor(
            of: find.text(text), matching: find.byType(Container));
        return (tester.widget<Container>(box.first).decoration as BoxDecoration)
            .color;
      }

      expect(fillOver('Radius'), t.selectionFill, reason: 'picked to start');

      // Now the Viewer's path: the shell's selection changes under the panel.
      p.uiState.setSelection([second]);
      await tester.pump();

      expect(fillOver('Radius'), isNull,
          reason: 'the property belonged to the layer that was let go of');
      expect(fillOver('Gaussian blur'), isNull,
          reason: 'and so did the mark on the effect holding it');
      expect(
          (tester
                  .widget<Container>(
                      find.byKey(ValueKey<String>('tl-rowbody-$id')))
                  .decoration as BoxDecoration)
              .color,
          isNot(t.selectionFill.withValues(alpha: 0.45)),
          reason: 'the old layer stops looking chosen');
    });

    /// **The highlight with nowhere to sit (K-203).** A selected property
    /// stayed selected when its layer was twirled shut — invisible, but still
    /// the selection — so it came back lit when the layer reopened, and it
    /// went on colouring the layer's row while the user worked on a different
    /// layer entirely.
    testWidgets('closing a layer drops the selection inside it',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      final t = LumitTheme.dark();
      Color? fillOver(String text) {
        final box = find.ancestor(
            of: find.text(text), matching: find.byType(Container));
        return (tester.widget<Container>(box.first).decoration as BoxDecoration)
            .color;
      }

      await openFold(tester, id, group: 'Transform');
      await tester.tap(find.text('Opacity'));
      await tester.pump();
      expect(fillOver('Opacity'), t.selectionFill);

      // Shut the layer, open it again — the Transform twirl inside it is
      // remembered, so the rows come straight back. Nothing should be lit.
      await openFold(tester, id);
      await openFold(tester, id);
      expect(fillOver('Opacity'), isNull,
          reason: 'a selection you could not see is not a selection');
    });

    /// Clicking a layer means "this layer", not "this layer and whatever was
    /// picked on the last one" (K-203).
    testWidgets('selecting another layer clears the property selection',
        (tester) async {
      final p = withComp();
      final first = p.comp.addSolidLayer();
      final second = p.comp.addSolidLayer();
      await mount(tester, p);

      final t = LumitTheme.dark();
      Color? fillOver(String text) {
        final box = find.ancestor(
            of: find.text(text), matching: find.byType(Container));
        return (tester.widget<Container>(box.first).decoration as BoxDecoration)
            .color;
      }

      await openFold(tester, first.internallayerId, group: 'Transform');
      await tester.tap(find.text('Opacity'));
      await tester.pump();
      expect(fillOver('Opacity'), t.selectionFill);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-name-${second.internallayerId}')));
      // Past the double-tap window the name carries for renaming, so its timer
      // is spent rather than left pending.
      await tester.pump(const Duration(milliseconds: 400));
      expect(fillOver('Opacity'), isNull,
          reason: "the other layer's property is no longer the selection");
      expect(p.uiState.selectedLayer.value?.internallayerId,
          second.internallayerId);
    });

    /// **No way out of a selection (K-203).** Every command that reads the
    /// selection — Delete, the Retime chord, U — was stuck with whatever was
    /// picked last, because the only way to change it was to pick something
    /// else. A click on empty ground is the way out.
    testWidgets('clicking empty outline ground deselects everything',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}')));
      // Past the double-tap window the name carries for renaming.
      await tester.pump(const Duration(milliseconds: 400));
      expect(p.uiState.selectedLayer.value, isNotNull);

      // Well below the single layer row, which is empty outline.
      final ground =
          tester.getRect(find.byKey(const ValueKey('tl-outline-ground')));
      await tester.tapAt(Offset(ground.center.dx, ground.bottom - 20));
      await tester.pump();
      expect(p.uiState.selectedLayer.value, isNull);
    });

    /// With nothing selected, the reveal is the whole composition's: "show me
    /// what is animated" is a question about the comp as often as about one
    /// layer (K-203).
    testWidgets('U with nothing selected reveals every animated layer',
        (tester) async {
      final p = withComp();
      final animated = p.comp.addSolidLayer();
      animated.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [0, 30])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      final still = p.comp.addSolidLayer();
      await mount(tester, p);

      expect(p.uiState.selectedLayer.value, isNull,
          reason: 'nothing is picked');
      await tester.sendKeyEvent(LogicalKeyboardKey.keyU);
      await tester.pump();

      expect(
          find.byKey(ValueKey<String>('tl-lanes-${animated.internallayerId}')),
          findsOneWidget,
          reason: 'the keyed layer opened');
      expect(find.byKey(ValueKey<String>('tl-lanes-${still.internallayerId}')),
          findsNothing,
          reason: 'a layer with nothing animated stays shut');
    });

    /// **The reveal keys went nowhere.** `P`, `S`, `R`, `T` and `A` were bound
    /// in the Timeline context with no handler to answer them (docs/07 §4.3),
    /// so the only way to see one property was to twirl the whole Transform
    /// group open and read past the other four.
    testWidgets('P reveals Position alone, and a second press shuts the layer',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await tester.sendKeyEvent(LogicalKeyboardKey.keyP);
      await tester.pump();
      expect(find.text('Position'), findsAtLeastNWidgets(1));
      expect(find.text('Scale'), findsNothing,
          reason: 'a solo shows the one property it names');
      expect(find.text('Opacity'), findsNothing);

      await tester.sendKeyEvent(LogicalKeyboardKey.keyP);
      await tester.pump();
      expect(find.text('Position'), findsNothing,
          reason: 'the key is a toggle, as AE\'s is');
    });

    /// A reveal names one row, so the Retime row above Transform stands down
    /// with the rest — "show me Scale" that also showed Retime would be
    /// answering a question nobody asked.
    testWidgets('a reveal stands the Retime row down with the others',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.toggleRetimeProperty();
      p.uiState.setSelection([layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      // Twirled open the ordinary way, the Retime row is there (docs/07 §4.3).
      await openFold(tester, layer.internallayerId);
      expect(find.text('Retime'), findsAtLeastNWidgets(1));

      await tester.sendKeyEvent(LogicalKeyboardKey.keyS);
      await tester.pump();
      expect(find.text('Scale'), findsAtLeastNWidgets(1));
      expect(find.text('Retime'), findsNothing);
    });

    /// **`Ctrl+Shift+C` belongs to the shell now.** Precompose asks two
    /// questions before it packs anything (docs/07 §13.4), so the panel no
    /// longer answers the key itself — it declines, and the shell's dialogue
    /// takes it. What matters here is that the panel does not quietly pack the
    /// selection on its own, which is what it used to do.
    testWidgets('Ctrl+Shift+C is left for the shell to answer', (tester) async {
      final p = withComp();
      final lower = p.comp.addSolidLayer();
      final upper = p.comp.addSolidLayer();
      p.comp.addSolidLayer();
      p.uiState.setSelection([upper, lower]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyC);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();

      expect(p.comp.getLayers().length, 3,
          reason: 'nothing was packed without the dialogue being answered');
      expect(p.uiState.selectedLayers.value.length, 2,
          reason: 'and the selection is untouched');
    });

    /// **`[` and `]` were bound and unanswered too.** They move the layer so
    /// that end lands on the playhead; with `Alt` they trim it there instead,
    /// under the same rules the bar's own drag follows.
    testWidgets('[ moves the layer to the playhead and Alt+] trims it there',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      final before = layer.getSpan();
      final length = before.outPoint.num / before.outPoint.den -
          before.inPoint.num / before.inPoint.den;
      p.uiState.setSelection([layer]);
      p.uiState.playheadFrame.value = 20;
      p.uiState.model.refresh();
      await mount(tester, p);

      await tester.sendKeyEvent(LogicalKeyboardKey.bracketLeft);
      await tester.pumpAndSettle();
      final moved = layer.getSpan();
      final at20 = p.comp.timeOfFrame(frame: 20);
      expect(moved.inPoint.num / moved.inPoint.den,
          closeTo(at20.num / at20.den, 1e-9),
          reason: 'the in point is on the playhead');
      expect(
          moved.outPoint.num / moved.outPoint.den -
              moved.inPoint.num / moved.inPoint.den,
          closeTo(length, 1e-9),
          reason: 'a move keeps the length; only a trim changes it');

      p.uiState.playheadFrame.value = 30;
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.bracketRight);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pumpAndSettle();
      final trimmed = layer.getSpan();
      final at30 = p.comp.timeOfFrame(frame: 30);
      expect(trimmed.outPoint.num / trimmed.outPoint.den,
          closeTo(at30.num / at30.den, 1e-9),
          reason: 'the out point is on the playhead');
      expect(trimmed.inPoint, moved.inPoint,
          reason: 'a trim moves one end, not both');
    });

    /// **Ctrl+click toggled the layer in and straight back out.** Selection ran
    /// twice for one click — once on the row's pointer-down and once on its tap
    /// — which is invisible for a plain click and exactly wrong for a toggle.
    testWidgets('Ctrl+click adds a layer to the selection and takes it out',
        (tester) async {
      final p = withComp();
      final lower = p.comp.addSolidLayer();
      final upper = p.comp.addSolidLayer();
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-name-${upper.internallayerId}')));
      await tester.pump(const Duration(milliseconds: 400));
      expect(p.uiState.selectedLayers.value.length, 1);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-name-${lower.internallayerId}')));
      await tester.pump(const Duration(milliseconds: 400));
      expect(
          p.uiState.selectedLayerIds,
          containsAll(
              <UuidValue>[upper.internallayerId, lower.internallayerId]));

      // And out again: the same click on a chosen layer un-chooses it.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-name-${lower.internallayerId}')));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      expect(p.uiState.selectedLayerIds, <UuidValue>{upper.internallayerId});
    });

    /// Shift extends the selection along the stack, the way it extends a
    /// property selection along the visible rows (docs/07 §4.3).
    testWidgets('Shift+click extends the selection down the stack',
        (tester) async {
      final p = withComp();
      final bottom = p.comp.addSolidLayer();
      final middle = p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      await mount(tester, p);

      await tester
          .tap(find.byKey(ValueKey<String>('tl-name-${top.internallayerId}')));
      await tester.pump(const Duration(milliseconds: 400));

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-name-${bottom.internallayerId}')));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);

      expect(
          p.uiState.selectedLayerIds,
          <UuidValue>{
            top.internallayerId,
            middle.internallayerId,
            bottom.internallayerId
          },
          reason: 'everything between the two ends, inclusive');
      expect(p.uiState.selectedLayer.value?.internallayerId,
          bottom.internallayerId,
          reason: 'the layer just clicked is the one commands act on');
    });

    /// Twirling a layer open is not choosing it: the properties belong to that
    /// layer whether or not it is the one being worked on.
    testWidgets('the twirl opens a fold without taking the selection',
        (tester) async {
      final p = withComp();
      final lower = p.comp.addSolidLayer();
      final upper = p.comp.addSolidLayer();
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-name-${upper.internallayerId}')));
      await tester.pump(const Duration(milliseconds: 400));

      await openFold(tester, lower.internallayerId);
      expect(find.byKey(ValueKey<String>('tl-lanes-${lower.internallayerId}')),
          findsOneWidget,
          reason: 'the fold opened');
      expect(
          p.uiState.selectedLayer.value?.internallayerId, upper.internallayerId,
          reason: 'and the selection stayed where it was');

      // Nor does hiding a layer choose it: the switch groups are controls.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-visible-${lower.internallayerId}')));
      await tester.pump();
      expect(p.uiState.selectedLayer.value?.internallayerId,
          upper.internallayerId);
    });

    /// Selecting keyframes on a lane selects the property they belong to, so
    /// the outline follows what was boxed (docs/07 §4.3).
    testWidgets('boxing keyframes on a lane selects their property',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 1500])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      final rect = tester.getRect(find.byKey(laneKey));
      final gesture =
          await tester.startGesture(Offset(rect.left + 2, rect.top + 2));
      await tester.pump(const Duration(milliseconds: 100));
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(Offset(rect.width / 8, rect.height / 10));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final t = LumitTheme.dark();
      final row = find.ancestor(
        of: find.text('Opacity'),
        matching: find.byType(Container),
      );
      expect(
          (tester.widget<Container>(row.first).decoration as BoxDecoration)
              .color,
          t.selectionFill,
          reason: 'the boxed keys picked their own property row');
    });

    /// Dragging a header seam resizes that group and leaves the rest alone,
    /// so the outline grows by what the drag moved — and the fold-out's value
    /// cells, which span the render group, grow with it (docs/07 §4.2).
    testWidgets('dragging a header seam resizes just that group',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      await openFold(tester, id, group: 'Transform');

      double widthOf(String key) =>
          tester.getSize(find.byKey(ValueKey<String>(key))).width;
      final identityBefore = widthOf('tl-colgroup-identity');
      final composeBefore = widthOf('tl-blend-$id');
      // Modes is fixed at its five switch cells (owner, 2026-08-24), so the
      // seam dragged here is the layer-name group's — the one with something
      // inside it that gains from more room.
      final valueBefore = widthOf('tl-tf-opacity');

      await tester.drag(
          find.byKey(const ValueKey('tl-seam-identity')), const Offset(60, 0));
      await tester.pumpAndSettle();

      expect(widthOf('tl-colgroup-identity'), greaterThan(identityBefore),
          reason: 'the group the seam follows grew');
      expect(widthOf('tl-blend-$id'), composeBefore,
          reason: 'every other group kept its width');
      expect(widthOf('tl-tf-opacity'), valueBefore,
          reason: 'and so did the value cells under the fixed Modes column');
    });

    /// The bottom bar takes a column group away and gives it back (K-448,
    /// §12A.1), so the outline pares down to names and bars. What lines up
    /// with a hidden group has to go somewhere sensible: the fold-out's value
    /// cells stay on the panel rather than being pushed off its right edge.
    testWidgets('the bottom bar hides and restores a column group',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;
      await openFold(tester, id, group: 'Transform');

      final switches = find.byKey(ValueKey<String>('tl-visible-$id'));
      final toggle = find.byKey(const ValueKey('tl-column-switches'));
      expect(switches, findsOneWidget);
      final panelRight = tester.getRect(find.byType(TimelinePanelFrb)).right;

      await tester.tap(toggle);
      await tester.pumpAndSettle();
      expect(switches, findsNothing, reason: 'the A/V column stood down');
      expect(find.byKey(ValueKey<String>('tl-blend-$id')), findsOneWidget,
          reason: 'and took none of the others with it');

      await tester.tap(find.byKey(const ValueKey('tl-column-render')));
      await tester.pumpAndSettle();
      expect(tester.getRect(find.byKey(const ValueKey('tl-tf-opacity'))).right,
          lessThanOrEqualTo(panelRight),
          reason: 'the value cells the hidden group carried stay on the panel');

      await tester.tap(toggle);
      await tester.pumpAndSettle();
      expect(switches, findsOneWidget, reason: 'and it comes back');
    });

    /// **The column headers are words, not icons** (§12A.1, K-451) — and the
    /// bottom bar's toggles say the same words, because a toggle that named a
    /// column differently from the column's own heading would be naming two
    /// things.
    testWidgets('the column headers are kicker words', (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      // The mockup's own row: Switches · # · Layer · Matte · Blend · Parent.
      for (final word in [
        'SWITCHES',
        '#',
        'LAYER',
        'MATTE',
        'BLEND',
        'PARENT',
      ]) {
        expect(find.text(word), findsWidgets, reason: '$word heads a column');
      }

      // Every header is a kicker (§7.1), muted, in the mono face.
      final t = LumitTheme.dark();
      final layerHead = tester.widget<Text>(find.text('LAYER').first);
      expect(layerHead.style!.fontFamily, LumitTheme.monoFontFamily);
      expect(layerHead.style!.color, t.textMuted);
      expect(layerHead.style!.fontSize, t.kicker.fontSize);
    });

    /// **The bottom bar's zoom is a slider** (owner, 2026-08-06), between a
    /// small landscape glyph and a large one. Its left end is the whole
    /// composition; dragging right widens the time axis, and a slider zoom has
    /// no pointer to zoom about, so it holds the **playhead** still — the
    /// middle of the scrollbar, which it held first, is a place nobody is
    /// looking at (K-293).
    testWidgets('the zoom slider widens the lanes about the playhead',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      // Off the middle on purpose: holding the *centre* still would pass a
      // playhead test that only ever looked at the centre.
      p.uiState.playheadFrame.value = 20;
      await tester.pump();

      Rect barRect() => tester.getRect(
          find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}')));
      double playheadX() => tester.getRect(find.byType(PlayheadMarker)).left;
      final before = barRect().width;
      final playheadBefore = playheadX();

      final slider = find.byKey(const ValueKey('tl-zoom-slider'));
      expect(slider, findsOneWidget, reason: 'the buttons became a slider');
      // Drag the handle a third of the way along its track.
      final track = tester.getRect(slider);
      await tester.dragFrom(
        Offset(track.left + 2, track.center.dy),
        Offset(track.width / 3, 0),
      );
      await tester.pumpAndSettle();

      expect(barRect().width, greaterThan(before),
          reason: 'the comp takes more pixels when zoomed in');
      expect(playheadX(), moreOrLessEquals(playheadBefore, epsilon: 2),
          reason: 'the playhead kept the screen position it had');
    });

    /// **A dragged slider does not fly** (K-293). The flight fills the gap
    /// between zooms that arrive in steps; a drag is already the motion, and
    /// animating towards a target the finger keeps moving left the lanes
    /// trailing the handle by a whole flight — reported as the slider being
    /// laggy. So the lanes are already at the dragged width *before* anything
    /// settles.
    testWidgets('a dragged zoom lands at once, with no flight to wait for',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      double barWidth() => tester
          .getRect(
              find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}')))
          .width;
      final before = barWidth();

      final track =
          tester.getRect(find.byKey(const ValueKey('tl-zoom-slider')));
      final gesture =
          await tester.startGesture(Offset(track.left + 2, track.center.dy));
      await tester.pump();
      // Two moves: the first is spent crossing the drag slop, which is what
      // *starts* the drag; the second is the one the slider reads.
      await gesture.moveBy(const Offset(20, 0));
      await tester.pump();
      await gesture.moveBy(Offset(track.width / 3, 0));
      // One frame, not `pumpAndSettle`: this is the frame the finger is still
      // down for.
      await tester.pump();
      final duringDrag = barWidth();
      expect(duringDrag, greaterThan(before),
          reason: 'the drag was applied in the frame it arrived in');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(barWidth(), moreOrLessEquals(duringDrag, epsilon: 1),
          reason: 'and nothing was still flying towards it afterwards');
    });

    /// The slider's two ends are drawn, not looked up, and plainly different
    /// sizes — which is the whole of what says "less of this / more of this"
    /// (K-293, K-209).
    testWidgets('the slider is flanked by a small landscape and a large one',
        (tester) async {
      final p = withComp();
      await mount(tester, p);

      final glyphs = tester
          .widgetList<CustomPaint>(find.byType(CustomPaint))
          .where((w) => w.painter is ZoomExtentPainter)
          .toList();
      expect(glyphs.length, 2, reason: 'one at each end of the track');
      final sizes = glyphs.map((g) => g.size.width).toList()..sort();
      expect(sizes.first, lessThan(sizes.last),
          reason: 'the pair reads as small and large');
      expect(sizes.last, lessThan(16),
          reason: 'both fit the 20px bar, which is why they are painter-drawn '
              'rather than icon-set glyphs (K-209)');
    });

    /// The bar wears the layer's label colour (K-188) — **desaturated**
    /// (§12A.1): the chip thinned over the lane's ground, with the leading
    /// edge carrying it whole. Recolouring the label recolours both — and a solid starts on the solid chip.
    testWidgets('the bar wears the label colour, thinned', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      Color barColour() {
        final fill = find
            .byKey(ValueKey<String>('tl-bar-fill-${layer.internallayerId}'));
        final deco = tester.widget<Container>(fill).decoration as BoxDecoration;
        return deco.color!;
      }

      Color edgeColour() => tester
          .widget<ColoredBox>(find.descendant(
            of: find.byKey(
                ValueKey<String>('tl-bar-edge-${layer.internallayerId}')),
            matching: find.byType(ColoredBox),
          ))
          .color;

      final t = LumitTheme.dark();
      expect(barColour(), t.labelColour(2).withValues(alpha: clipFillAlpha),
          reason: 'a solid starts on its kind\'s chip, thinned');
      expect(edgeColour(), t.labelColour(2),
          reason: 'the leading edge carries the colour whole');

      layer.setLabel(label: 6);
      p.uiState.model.refresh();
      await tester.pump();
      expect(barColour(), t.labelColour(6).withValues(alpha: clipFillAlpha),
          reason: 'picking a label recolours the bar');
      expect(edgeColour(), t.labelColour(6),
          reason: 'and its leading edge with it');

      // Selection brightens the bar rather than outlining it (K-317): the
      // label colour lerps toward textPrimary, so the hue still says which
      // layer this is while the lit bar says it is the one in hand.
      p.uiState.setSelection([layer]);
      await tester.pump();
      expect(
          barColour(),
          Color.lerp(t.labelColour(6), t.textPrimary, 0.35)!
              .withValues(alpha: clipFillSelectedAlpha),
          reason: 'a selected bar is its label colour, lit');
      final deco = tester
          .widget<Container>(find
              .byKey(ValueKey<String>('tl-bar-fill-${layer.internallayerId}')))
          .decoration as BoxDecoration;
      expect(deco.border, isNull,
          reason: 'selection no longer draws an outline');
    });

    /// A stack taller than the panel scrolls rather than overflowing, and
    /// the two halves stay one table while it does.
    testWidgets('a tall stack scrolls without overflowing', (tester) async {
      final p = withComp();
      for (var i = 0; i < 40; i++) {
        p.comp.addSolidLayer();
      }
      await mount(tester, p);
      expect(tester.takeException(), isNull,
          reason: '40 rows in a 600px panel must scroll, not overflow');
    });

    /// The group reorder rule: the dragged group takes the target's slot,
    /// whichever side it came from, and dropping on itself changes nothing.
    test('reorderedGroups moves a group to the target slot', () {
      const g = TimelineGroup.values;
      expect(
        reorderedGroups(defaultGroupOrder, g[0], g[3]),
        [g[1], g[2], g[3], g[0], g[4]],
        reason: 'dragged right, it lands after the target',
      );
      expect(
        reorderedGroups(defaultGroupOrder, g[3], g[0]),
        [g[3], g[0], g[1], g[2], g[4]],
        reason: 'dragged left, it lands before the target',
      );
      expect(reorderedGroups(defaultGroupOrder, g[1], g[1]), defaultGroupOrder);
    });

    /// The value column sits under the render group: everything right of it
    /// in the order contributes its fixed width to the inset.
    test('valueColumnFor measures what sits right of the render group', () {
      expect(
          valueColumnFor(defaultGroupOrder, defaultGroupWidths).rightInset,
          groupDividerWidth +
              composeGroupWidth +
              groupDividerWidth +
              timingsGroupWidth);
      final renderLast = reorderedGroups(
          defaultGroupOrder, TimelineGroup.render, TimelineGroup.timings);
      expect(valueColumnFor(renderLast, defaultGroupWidths).rightInset, 0);

      // The value cells span the render group as it stands, so dragging that
      // group's seam widens the fields under it (K-192).
      final wider = {
        ...defaultGroupWidths,
        TimelineGroup.render: renderGroupWidth + 60,
      };
      expect(valueColumnFor(defaultGroupOrder, wider).width,
          renderGroupWidth + 60);
    });

    /// A group the outline is not drawing (its bottom-bar toggle is off,
    /// K-448) has nothing to its right — what lines up with it sits at the
    /// outline's own edge. The loop used to start from index zero for a group
    /// it could not find, counting *every* column as being to the right of one
    /// that was not there and pushing the value cells off the panel.
    test('a hidden group has no inset', () {
      final withoutRender = [
        for (final g in defaultGroupOrder)
          if (g != TimelineGroup.render) g
      ];
      expect(
          rightInsetOf(withoutRender, defaultGroupWidths, TimelineGroup.render),
          0);
      expect(valueColumnFor(withoutRender, defaultGroupWidths).rightInset, 0);
      expect(valueColumnFor(withoutRender, defaultGroupWidths).width,
          renderGroupWidth,
          reason: 'the cells keep their width; only where they sit changes');
    });

    /// The render-time readout on a twirled-open effect's heading has to sit
    /// under the same header the layer rows' numbers do, wherever that column
    /// has been dragged (docs/13 §7.1) — so a fold row measures its own inset
    /// rather than assuming the column is last.
    test('timingsColumnFor follows the render-time column', () {
      expect(
          timingsColumnFor(defaultGroupOrder, defaultGroupWidths).rightInset, 0,
          reason: 'shipped last, nothing sits to its right');
      expect(timingsColumnFor(defaultGroupOrder, defaultGroupWidths).width,
          timingsGroupWidth);
      final timingsFirst = reorderedGroups(
          defaultGroupOrder, TimelineGroup.timings, TimelineGroup.switches);
      expect(
        timingsColumnFor(timingsFirst, defaultGroupWidths).rightInset,
        rightInsetOf(timingsFirst, defaultGroupWidths, TimelineGroup.timings),
        reason: 'dragged to the front, the inset is everything after it',
      );
      expect(timingsColumnFor(timingsFirst, defaultGroupWidths).rightInset,
          greaterThan(0));
    });

    /// The ruler's label spacing thins as the comp zooms out, and its labels
    /// speak the familiar editor idiom.
    test('the ruler picks nice label steps and formats them', () {
      expect(rulerLabelStepSeconds(pixelsPerSecond: 100), 1);
      expect(rulerLabelStepSeconds(pixelsPerSecond: 20), 5);
      expect(rulerLabelStepSeconds(pixelsPerSecond: 2), 60);
      // Unpadded seconds, as the mockup's ruler reads them (K-451).
      expect(rulerLabelOf(0), '00s');
      expect(rulerLabelOf(5), '05s');
      expect(rulerLabelOf(0.5), '0.5s');
      expect(rulerLabelOf(0.25), '0.25s');
      expect(rulerLabelOf(2.5), '02.5s');
      expect(rulerLabelOf(60), '01:00s');
      expect(rulerLabelOf(90), '01:30s');
      expect(rulerLabelOf(3600), '1:00:00s');
    });

    /// Minor ticks subdivide as the zoom grows, and stop at one frame: a ruler
    /// that went on dividing would draw ticks nothing can land between
    /// (docs/15 §12A.1).
    test('the ruler subdivides down to one frame and no further', () {
      double minor(double pxPerSec, {double fps = 25}) => rulerMinorStepSeconds(
          pixelsPerSecond: pxPerSec,
          labelStep: rulerLabelStepSeconds(pixelsPerSecond: pxPerSec),
          fps: fps);

      // Wide open: one tick a frame, and the ladder has no rung below it —
      // whatever the rate, and however much further the zoom goes. Full zoom
      // shows twenty frames across the lanes, which is about 35px a frame, so
      // the finest rung is reached inside the zoom the panel offers.
      expect(minor(35 * 25), closeTo(1 / 25, 1e-9),
          reason: 'a frame 35px wide is one tick, as full zoom draws it');
      expect(minor(100000), closeTo(1 / 25, 1e-9),
          reason: 'a frame is the floor however far the zoom goes');
      expect(minor(100000, fps: 60), closeTo(1 / 60, 1e-9));

      // Zooming in only ever subdivides further, and every tick keeps a few
      // pixels to itself — a comb is not a ruler.
      var last = double.infinity;
      for (final px in [2.0, 8.0, 20.0, 60.0, 150.0, 600.0, 5000.0]) {
        final step = minor(px);
        final labels = rulerLabelStepSeconds(pixelsPerSecond: px);
        expect(step, lessThanOrEqualTo(last),
            reason: 'the ruler never coarsens as it is zoomed in');
        expect(step, lessThanOrEqualTo(labels),
            reason: 'ticks are never coarser than the labels they sit between');
        expect(step, greaterThanOrEqualTo(1 / 25 - 1e-9),
            reason: 'and never finer than a frame');
        if (step < labels) {
          expect(step * px, greaterThanOrEqualTo(30.0),
              reason: 'a drawn tick keeps the mockup\'s room beside it');
        }
        last = step;
      }

      // No room for anything at all: the labelled ticks stand alone.
      expect(minor(0.01), rulerLabelStepSeconds(pixelsPerSecond: 0.01));

      // **At the resting zoom, minor ticks are drawn** (§12A.1, and the
      // mockup, which shows them between the labelled seconds). At zoom 1 a
      // composition fits the lane area, so a ten-second comp across roughly
      // 700px is 70 pixels a second — and the subdivision has to land there,
      // not only deep in a zoom. The whole band around that is checked, so a
      // slightly different panel width cannot silently empty the ruler.
      for (final px in [40.0, 70.0, 120.0]) {
        final labels = rulerLabelStepSeconds(pixelsPerSecond: px);
        expect(minor(px), lessThan(labels),
            reason: 'the ruler subdivides at the resting zoom ($px px/s)');
      }

      // And it subdivides **at the mockup's density** (K-451): 70 px/s labels
      // every two seconds, with a tick on each half second between them —
      // three ticks 35px apart, not the comb a finer rung would draw. The rate
      // does not decide it: the ladder's rungs below a half second differ by
      // fps, and none of them has the room.
      for (final fps in [24.0, 25.0, 30.0, 60.0]) {
        expect(rulerLabelStepSeconds(pixelsPerSecond: 70), 2,
            reason: 'the resting zoom labels every two seconds');
        expect(minor(70, fps: fps), closeTo(0.5, 1e-9),
            reason: 'half-second minor ticks at the resting zoom ($fps fps)');
      }
    });

    /// What a grab does to the waveform's preview of the span — the mapping
    /// the lane draws while the gesture is still in flight (K-172).
    test('barDragPreview maps each grab onto the span', () {
      final move = barDragPreview('a', BarGrab.move, 5);
      expect((move.deltaIn, move.deltaOut, move.offsetShift), (5, 5, 5));
      final trimIn = barDragPreview('a', BarGrab.trimIn, -3);
      expect((trimIn.deltaIn, trimIn.deltaOut, trimIn.offsetShift), (-3, 0, 0));
      final trimOut = barDragPreview('a', BarGrab.trimOut, 7);
      expect(
          (trimOut.deltaIn, trimOut.deltaOut, trimOut.offsetShift), (0, 7, 0));
    });

    /// Which part of a bar a press takes hold of. The third-of-the-bar cap is
    /// the point: without it a short bar is all edge and cannot be moved.
    test('barGrabAt keeps a middle on even the shortest bar', () {
      expect(barGrabAt(2, 100), BarGrab.trimIn);
      expect(barGrabAt(50, 100), BarGrab.move);
      expect(barGrabAt(97, 100), BarGrab.trimOut);
      // Nine pixels wide: three each way, and a middle that still moves.
      expect(barGrabAt(1, 9), BarGrab.trimIn);
      expect(barGrabAt(4.5, 9), BarGrab.move);
      expect(barGrabAt(8, 9), BarGrab.trimOut);
    });

    /// The rule the ends obey (K-211): a source-backed layer stops where its
    /// media does, and Retime takes the limits off.
    test('barBounds pins a source-backed layer and frees a retimed one', () {
      expect(
        barBounds(startOffsetFrame: 10, sourceFrames: 50, retimed: false),
        const BarBounds(minIn: 10, maxOut: 60),
      );
      expect(
        barBounds(startOffsetFrame: 10, sourceFrames: 50, retimed: true),
        BarBounds.free,
        reason: 'Retime decides its own source times, so the ends are free',
      );
      expect(
        barBounds(startOffsetFrame: 10, sourceFrames: null, retimed: false),
        BarBounds.free,
        reason: 'a generated layer — or media that would not read — is free',
      );
    });

    /// What the clamp actually does to a gesture in flight.
    test('clampBarDelta holds a trim inside its source', () {
      const bounds = BarBounds(minIn: 10, maxOut: 60);
      int clamp(BarGrab grab, int delta,
              {int inFrame = 20, int outFrame = 50, BarBounds b = bounds}) =>
          clampBarDelta(
              grab: grab,
              delta: delta,
              inFrame: inFrame,
              outFrame: outFrame,
              bounds: b);

      expect(clamp(BarGrab.trimIn, -5), -5, reason: 'room to spare');
      expect(clamp(BarGrab.trimIn, -50), -10,
          reason: 'the head stops on the source\'s first frame');
      expect(clamp(BarGrab.trimOut, 50), 10,
          reason: 'the tail stops on the source\'s last frame');
      expect(clamp(BarGrab.trimOut, 500, b: BarBounds.free), 500,
          reason: 'a free end goes wherever it is dragged');
      // A move carries the start offset, so it can never leave the source.
      expect(clamp(BarGrab.move, 900), 900);
      // Never inside out: a bar always keeps at least one frame.
      expect(clamp(BarGrab.trimIn, 100), 29);
      expect(clamp(BarGrab.trimOut, -100), -29);
      // A layer already longer than its source keeps the length it has: the
      // bound holds it still rather than dragging it back.
      expect(clamp(BarGrab.trimOut, 5, inFrame: 20, outFrame: 80), 0);
      expect(clamp(BarGrab.trimOut, -5, inFrame: 20, outFrame: 80), -5,
          reason: 'pulling it back towards the source is always allowed');
    });

    /// Frames from exact times, in integers (K-184): the panel maps a start
    /// offset without asking the engine, and must floor the way `frame_at`
    /// does — including for a layer that starts before the comp.
    test('frameOfTime floors the way the engine does', () {
      BridgeRational r(int num, int den) => BridgeRational(num: num, den: den);
      expect(frameOfTime(r(1, 1), 30, 1), 30);
      expect(frameOfTime(r(1, 2), 30, 1), 15);
      expect(frameOfTime(r(1, 30), 24, 1), 0, reason: 'floors, never rounds');
      expect(frameOfTime(r(-1, 1), 30, 1), -30);
      expect(frameOfTime(r(-1, 30), 24, 1), -1,
          reason: 'negative times floor downwards, as div_euclid does');
      expect(frameOfTime(r(1, 1), 30000, 1001), 29,
          reason: '29.97: one second is 29 whole frames');
    });

    /// A Precomp layer cannot be trimmed past the comp it holds — and turning
    /// Retime on takes the limit off (K-211). Fails without the clamp: the
    /// tail simply followed the pointer.
    testWidgets('a precomp bar stops at the end of its source', (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Inner');
      // A short source, so the tail can reach the end of it in one drag.
      final settings = inner.getSettings();
      inner.setSettings(
        settings: BridgeCompSettings(
          name: settings.name,
          width: settings.width,
          height: settings.height,
          fpsNum: settings.fpsNum,
          fpsDen: settings.fpsDen,
          duration: const BridgeRational(num: 5, den: 1),
        ),
      );
      final layer = p.comp.addPrecompLayer(comp: inner);
      final sourceFrames = inner.durationFrames().toInt();
      // Well inside the source, so there is room to drag outward.
      layer.setSpan(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 0),
          outPoint: p.comp.timeOfFrame(frame: 200),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      await mount(tester, p);

      final fill =
          find.byKey(ValueKey<String>('tl-bar-fill-${layer.internallayerId}'));
      // Far more than the source has left: the tail must stop, not follow.
      await tester.dragFrom(
        Offset(tester.getRect(fill).right - 2, tester.getRect(fill).center.dy),
        const Offset(400, 0),
      );
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: layer.getSpan().outPoint), sourceFrames,
          reason: 'the tail landed on the source\'s last frame');

      // The corner mark says why it stopped.
      final marks = tester.widget<CustomPaint>(
          find.byKey(ValueKey<String>('tl-bar-ends-${layer.internallayerId}')));
      expect((marks.painter as BarEndMarksPainter).atOut, isTrue);

      // Retime on: the layer now decides its own source times, so it stretches.
      layer.toggleRetimeProperty();
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      await tester.dragFrom(
        Offset(tester.getRect(fill).right - 2, tester.getRect(fill).center.dy),
        const Offset(100, 0),
      );
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: layer.getSpan().outPoint),
          greaterThan(sourceFrames),
          reason: 'a retimed layer is any length the user drags it to');
      final retimedMarks = tester.widget<CustomPaint>(
          find.byKey(ValueKey<String>('tl-bar-ends-${layer.internallayerId}')));
      expect((retimedMarks.painter as BarEndMarksPainter).atOut, isFalse,
          reason: 'no limit, no mark');
    });

    /// Switching Retime on keys the layer where it *is* (K-213): the two
    /// diamonds land on its own start and end, not at the start of the
    /// composition. Fails without the comp-clock conversion at the seam — the
    /// keys drew at frames 0 and (duration) however far along the layer sat.
    testWidgets('the Retime keys land on the layer, not the comp start',
        (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Inner');
      final layer = p.comp.addPrecompLayer(comp: inner);
      // Moved along the timeline and trimmed a little off its head, so its own
      // zero is neither the comp's zero nor its in point.
      layer.setSpan(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 180),
          outPoint: p.comp.timeOfFrame(frame: 480),
          startOffset: p.comp.timeOfFrame(frame: 120),
        ),
      );
      layer.toggleRetimeProperty();
      p.uiState.model.refresh();
      await mount(tester, p);

      final keys = layer.getRetimeProperty()! as BridgeScalar_Keyframed;
      final frames = [
        for (final key in keys.field0) p.comp.frameAtTime(time: key.time)
      ];
      expect(frames, [180, 480],
          reason: 'the keys sit on the layer\'s own start and end');

      // And that is where the lane draws them, in the panel's own arithmetic.
      final fps = p.comp.fps();
      expect([for (final key in keys.field0) laneKeyFrame(key, fps).round()],
          [180, 480]);
    });

    /// A generated layer has no source to run out of: both ends go wherever
    /// they are dragged, and neither wears a corner mark.
    testWidgets('a solid bar trims freely and wears no marks', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setSpan(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 0),
          outPoint: p.comp.timeOfFrame(frame: 200),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      await mount(tester, p);

      final fill =
          find.byKey(ValueKey<String>('tl-bar-fill-${layer.internallayerId}'));
      await tester.dragFrom(
        Offset(tester.getRect(fill).right - 2, tester.getRect(fill).center.dy),
        const Offset(120, 0),
      );
      await tester.pumpAndSettle();
      expect(
          p.comp.frameAtTime(time: layer.getSpan().outPoint), greaterThan(200));

      final marks = tester.widget<CustomPaint>(
          find.byKey(ValueKey<String>('tl-bar-ends-${layer.internallayerId}')));
      expect((marks.painter as BarEndMarksPainter).atIn, isFalse);
      expect((marks.painter as BarEndMarksPainter).atOut, isFalse);
      expect(
          find.byKey(ValueKey<String>('tl-bar-ghost-${layer.internallayerId}')),
          findsNothing,
          reason: 'no source, nothing to show past the ends');
    });

    /// A trimmed source-backed layer shows where its media would reach — the
    /// faint outline behind the bar (K-212) — and stops showing it once the bar
    /// fills the source, or once Retime makes "the source's reach" meaningless.
    /// The one-frame bug: the ghost outline appearing part-way through a trim
    /// took the bar's place in its Stack, so the bar's element — and the
    /// recogniser holding the drag — was rebuilt mid-gesture. The bar moved by
    /// the first pointer event's frames and then went dead, which read as "the
    /// edge only moves one frame". Fails without the keys on the Stack's
    /// children; a single-event drag hides it, so this one moves in steps, as
    /// a hand does.
    testWidgets('a source-backed edge follows the pointer the whole way',
        (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Inner');
      final layer = p.comp.addPrecompLayer(comp: inner);
      await mount(tester, p);

      final fill =
          find.byKey(ValueKey<String>('tl-bar-fill-${layer.internallayerId}'));
      final rect = tester.getRect(fill);
      final before = p.comp.frameAtTime(time: layer.getSpan().outPoint);
      // A mouse: precise pointers have a one-pixel slop, so every step of this
      // counts as movement. Ten steps, the way a hand drags.
      final gesture = await tester.startGesture(
          Offset(rect.right - 2, rect.center.dy),
          kind: PointerDeviceKind.mouse);
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(-4, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      // Forty pixels of pointer, at this zoom, is far more than the couple of
      // frames one event carries.
      final after = p.comp.frameAtTime(time: layer.getSpan().outPoint);
      final perPixel = (before - after) / 40;
      expect(perPixel, greaterThan(3),
          reason: 'the edge tracked all forty pixels, not just the first four');
      expect(
          find.byKey(ValueKey<String>('tl-bar-ghost-${layer.internallayerId}')),
          findsOneWidget,
          reason: 'and the ghost that used to break it is on screen');
    });

    testWidgets('a trimmed precomp shows how far its source reaches',
        (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Inner');
      final layer = p.comp.addPrecompLayer(comp: inner);
      final sourceFrames = inner.durationFrames().toInt();
      final ghost =
          find.byKey(ValueKey<String>('tl-bar-ghost-${layer.internallayerId}'));

      await mount(tester, p);
      expect(ghost, findsNothing,
          reason: 'a layer filling its source has nothing left to show');

      // Crop the tail: the outline now reaches past it, to the source's end.
      layer.setSpan(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 0),
          outPoint: p.comp.timeOfFrame(frame: sourceFrames - 300),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect(ghost, findsOneWidget);
      final fill =
          find.byKey(ValueKey<String>('tl-bar-fill-${layer.internallayerId}'));
      expect(
          tester.getRect(ghost).right, greaterThan(tester.getRect(fill).right),
          reason: 'it reaches past the trimmed end');
      expect(
          tester.getRect(ghost).left, closeTo(tester.getRect(fill).left, 0.5),
          reason: 'and not past the end that is still at the source start');

      // Retime on: the source has no reach worth drawing any more.
      layer.toggleRetimeProperty();
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect(ghost, findsNothing);
    });

    /// The source-extent hint is a hairline and nothing else (§12A.1): a fill
    /// behind the bar read as a second, dimmer object rather than as this
    /// bar's own reach.
    testWidgets('the source-extent hint is an outline, not a fill',
        (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Inner');
      final layer = p.comp.addPrecompLayer(comp: inner);
      final sourceFrames = inner.durationFrames().toInt();
      await mount(tester, p);

      layer.setSpan(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 0),
          outPoint: p.comp.timeOfFrame(frame: sourceFrames - 300),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      final deco = tester
          .widget<Container>(find.descendant(
            of: find.byKey(
                ValueKey<String>('tl-bar-ghost-${layer.internallayerId}')),
            matching: find.byType(Container),
          ))
          .decoration as BoxDecoration;
      expect(deco.color, isNull, reason: 'nothing inside the outline');
      final side = (deco.border as Border).top;
      expect(side.width, 1);
      expect(side.color.a, closeTo(0.25, 0.001),
          reason: 'faint, and the label colour thinned rather than a new one');
    });

    /// A layer can start BEFORE the comp (docs/TODO: "re-introduce"): drag a
    /// bar left past frame zero and the span goes negative, carrying its
    /// content with it — the comp shows the part that overlaps.
    testWidgets('a bar dragged left of zero starts before the comp',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final bar =
          find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
      final rect = tester.getRect(bar);
      // From the middle (a move, not a trim), left by more than the bar's
      // distance to zero.
      await tester.dragFrom(
        Offset(rect.left + rect.width * 0.5, rect.center.dy),
        const Offset(-160, 0),
      );
      await tester.pumpAndSettle();

      final inFrame = p.comp.frameAtTime(time: layer.getSpan().inPoint);
      expect(inFrame, lessThan(0),
          reason: 'nothing pins a layer to the comp edge');
      // The offset travelled with it: layer time zero moved by the same
      // amount, so the content did not retime.
      expect(p.comp.frameAtTime(time: layer.getSpan().startOffset), inFrame);
    });

    /// Trimming by the bar edges (docs/TODO: "drag start/end to adjust/crop"):
    /// the in edge crops without moving the content, and the out edge crops
    /// the tail.
    testWidgets('the bar edges trim in and out', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final before = layer.getSpan();
      final beforeIn = p.comp.frameAtTime(time: before.inPoint);
      final beforeOut = p.comp.frameAtTime(time: before.outPoint);

      // The **body**, not the full-width row it sits in: the axis pads a few
      // pixels either side (§12A.1), so the row's edge is no longer the bar's.
      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      var rect = tester.getRect(bar);
      // Near the left edge: a trim of the in point, content unmoved.
      await tester.dragFrom(
          Offset(rect.left + 2, rect.center.dy), const Offset(60, 0));
      await tester.pumpAndSettle();
      final trimmedIn = p.comp.frameAtTime(time: layer.getSpan().inPoint);
      expect(trimmedIn, greaterThan(beforeIn), reason: 'the head is cropped');
      expect(p.comp.frameAtTime(time: layer.getSpan().startOffset),
          p.comp.frameAtTime(time: before.startOffset),
          reason: 'trimming never retimes the content');

      // Near the right edge: a trim of the out point.
      rect = tester.getRect(bar);
      await tester.dragFrom(
          Offset(rect.right - 2, rect.center.dy), const Offset(-60, 0));
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: layer.getSpan().outPoint),
          lessThan(beforeOut),
          reason: 'the tail is cropped');
    });

    testWidgets('the work area and markers draw on the ruler', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      p.comp.setWorkArea(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 10),
          outPoint: p.comp.timeOfFrame(frame: 40),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      await mount(tester, p);

      expect(find.byKey(const ValueKey('tl-work-area')), findsOneWidget);

      // Clearing it does not take the bar away: a comp that has not been
      // narrowed has a work area of the whole comp (K-203), which is what the
      // engine's null means and what leaves its ends there to grab.
      final narrowed =
          tester.getRect(find.byKey(const ValueKey('tl-work-area')));
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-clear-work-area')));
      await tester.pumpAndSettle();
      expect(p.comp.getWorkArea(), isNull);
      final whole = tester.getRect(find.byKey(const ValueKey('tl-work-area')));
      expect(whole.width, greaterThan(narrowed.width),
          reason: 'cleared, the work area spans the whole comp');
      expect(find.byKey(const ValueKey('tl-work-start')), findsOneWidget);
      expect(find.byKey(const ValueKey('tl-work-end')), findsOneWidget);
    });

    /// A work-area drag is staged: the document hears nothing until the
    /// pointer lifts, so the drag costs no writes while moving and one undo
    /// steps clean back over it (owner, 2026-08-21 — the mid-drag commits
    /// made the drag lag and undo walk back through every frame crossed).
    testWidgets('a work-area drag commits once, on release', (tester) async {
      final p = withComp();
      // A real span to move, so "unchanged mid-drag" is not vacuously true of
      // the whole-comp default.
      p.comp.setWorkArea(
          span: workAreaWith(
              comp: p.comp,
              current: null,
              wanted: p.comp.durationFrames() ~/ 2,
              isStart: false));
      await mount(tester, p);

      final before = workAreaFrames(p.comp);
      expect(before.whole, isFalse);

      final start =
          tester.getCenter(find.byKey(const ValueKey('tl-work-start')));
      final end = tester.getCenter(find.byKey(const ValueKey('tl-work-end')));
      final gesture = await tester.startGesture(end);
      await tester.pump();
      // Cross a good stretch of the span in steps, as a hand does.
      final step = Offset((start.dx - end.dx) / 12, 0);
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(step);
        await tester.pump();
      }
      expect(workAreaFrames(p.comp), equals(before),
          reason: 'mid-drag, the document has not been written');

      await gesture.up();
      await tester.pumpAndSettle();
      final after = workAreaFrames(p.comp);
      expect(after.end, lessThan(before.end),
          reason: 'the release is the one write');

      p.state.project!.undo();
      expect(workAreaFrames(p.comp), equals(before),
          reason: 'one undo returns to the span before the drag');
    });

    /// The lane area is rows all the way down. One layer used to leave the
    /// ground, the seams and the marquee stopping 22 px in, so most of the
    /// area was a hole: nothing to look at and nothing to click on.
    testWidgets('the lanes fill the viewport below the last layer',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      // The marquee is `Positioned.fill` inside the scrolled content, so its
      // height is the content's height.
      final lanes =
          tester.getSize(find.byKey(const ValueKey('tl-lane-marquee')));
      expect(lanes.height, greaterThan(400),
          reason: 'one 22 px row does not end the lane area');
    });
    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
    /// The gesture the whole Project panel drag exists for. It had no drop
    /// target at all: the drag lifted, showed feedback, and dropped into
    /// nothing, which reads as the app ignoring you.
    testWidgets('footage dragged from the Project panel becomes a layer',
        (tester) async {
      final p = withComp();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      // Both panels in one tree, so the drag is the real one rather than a
      // DragTarget poked directly.
      await tester.pumpWidget(hostPanel(
        child: const Row(
          children: [
            SizedBox(width: 300, child: ProjectPanelFrb()),
            Expanded(child: TimelinePanelFrb()),
          ],
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();

      expect(p.comp.getLayers(), isEmpty);

      final row =
          find.byKey(ValueKey<String>('project-row-${footage.internalid}'));
      expect(row, findsOneWidget, reason: 'the footage row is there to drag');

      final gesture = await tester.startGesture(tester.getCenter(row));
      await tester.pump(const Duration(milliseconds: 200));
      // Stepped, because one large move leaves the gesture arena resolving the
      // drag against the row's own recognisers.
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(40, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.comp.getLayers(), hasLength(1),
          reason: 'the drop reached the document');
      expect(p.comp.getLayers().single.getName(), contains('shot'));
    });

    /// **A drop used to ignore where it was aimed.** Footage always went on at
    /// the top of the stack, so building an order meant dragging every clip in
    /// and then re-sorting it by hand.
    testWidgets('footage lands where it was dropped, not at the top',
        (tester) async {
      final p = withComp();
      // Three solids to drop between, added bottom-up so the stack reads
      // Top, Middle, Bottom down the screen. The drop aims at the middle one.
      for (final name in ['Bottom', 'Middle', 'Top']) {
        p.comp.addSolidLayer().rename(name: name);
      }
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const Row(
          children: [
            SizedBox(width: 300, child: ProjectPanelFrb()),
            Expanded(child: TimelinePanelFrb()),
          ],
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();

      final middle = p.comp.getLayers()[1];
      final row =
          find.byKey(ValueKey<String>('tl-row-${middle.internallayerId}'));
      // The upper half of the row: a drop there goes above it, and the centre
      // is the midpoint the rule flips on.
      final target =
          tester.getTopLeft(row) + Offset(tester.getSize(row).width / 2, 5);
      final from = tester.getCenter(
          find.byKey(ValueKey<String>('project-row-${footage.internalid}')));

      final gesture = await tester.startGesture(from);
      await tester.pump(const Duration(milliseconds: 200));
      // Stepped, for the same arena reason as the drop test above.
      for (var i = 1; i <= 10; i++) {
        await gesture.moveTo(Offset.lerp(from, target, i / 10)!);
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect([
        for (final l in p.comp.getLayers()) l.getName()
      ], [
        'Top',
        'shot.mov',
        'Middle',
        'Bottom'
      ], reason: 'it went in above the row it was dropped on');
    });

    /// Comps nest by the same gesture: drag one from the Project panel onto
    /// another's Timeline and it lands as a Precomp layer.
    testWidgets('a comp dragged from the Project panel nests as a precomp',
        (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Titles');

      await tester.pumpWidget(hostPanel(
        child: const Row(
          children: [
            SizedBox(width: 300, child: ProjectPanelFrb()),
            Expanded(child: TimelinePanelFrb()),
          ],
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();

      expect(p.comp.getLayers(), isEmpty);

      final row =
          find.byKey(ValueKey<String>('project-row-${inner.internalid}'));
      expect(row, findsOneWidget, reason: 'the comp row is there to drag');

      final gesture = await tester.startGesture(tester.getCenter(row));
      await tester.pump(const Duration(milliseconds: 200));
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(40, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final layers = p.comp.getLayers();
      expect(layers, hasLength(1), reason: 'the drop reached the document');
      expect(layers.single.getKind(), BridgeLayerKind.precomp,
          reason: 'a dropped comp nests as a Precomp layer');
      // The inner comp itself is untouched — nesting places, never moves.
      expect(inner.getLayers(), isEmpty);
    });

    /// **The cut lands under the blade, not under the playhead (K-220).**
    ///
    /// The razor used to cut at the playhead wherever the bar was clicked,
    /// which made it a slower way of pressing Ctrl+Shift+D. docs/07 §4.4 has
    /// always said "click a clip to cut it at that time"; this is that,
    /// asserted by putting the playhead somewhere the cut must *not* land.
    testWidgets('the razor cuts under the pointer, not under the playhead',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.tools.select(ToolMode.razor);
      p.uiState.model.refresh();
      await mount(tester, p);

      // The playhead sits at the very start; the click lands well past it.
      p.uiState.playheadFrame.value = 0;
      await tester.pump();

      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      final box = tester.getRect(bar);
      await tester.tapAt(Offset(box.left + box.width / 2, box.center.dy));
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
      final after = p.comp.getLayers();
      expect(after.length, 2);
      // The seam is around the middle of the layer, nowhere near frame 0 —
      // which a cut at the playhead could not have produced at all (it would
      // have been refused as outside the span, leaving one layer).
      final seam = [
        for (final l in after)
          l.getSpan().inPoint.num / l.getSpan().inPoint.den,
      ].reduce((a, b) => a > b ? a : b);
      expect(seam, greaterThan(0.0));
      expect(p.uiState.playheadFrame.value, 0,
          reason: 'and the razor did not move the playhead to do it');
    });

    /// The twirl-down the port dropped. A layer opens onto its *section
    /// headings* — Transform always, Effects when it has any, Audio only when
    /// its source carries sound — and each heading opens onto its own rows
    /// (docs/07 §4.3).
    testWidgets('a layer opens onto its section headings', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      final twirl =
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}'));
      expect(twirl, findsOneWidget, reason: 'every layer row has one');
      expect(find.text('Transform'), findsNothing,
          reason: 'closed to start with, or a busy comp is a wall of numbers');

      await tester.tap(twirl);
      await tester.pump();
      expect(find.text('Transform'), findsOneWidget);
      expect(find.text('Position'), findsNothing,
          reason: 'the heading opens first, not every property under it');
      expect(find.text('Effects'), findsNothing,
          reason: 'a layer with no effects has no Effects group to offer');
      expect(find.text('Audio'), findsNothing,
          reason: 'a solid cannot be heard, so it has no volume to set');

      await tester.tap(find.text('Transform'));
      await tester.pump();
      for (final row in [
        'Anchor point',
        'Position',
        'Scale',
        'Rotation',
        'Opacity'
      ]) {
        expect(find.text(row), findsOneWidget);
      }

      await tester.tap(twirl);
      await tester.pump();
      expect(find.text('Transform'), findsNothing);
    });

    /// The four column groups in their shipped order (docs/07 §4.2):
    /// visibility · audio · solo · lock · shy, then twirl · label · number ·
    /// name, then fx · motion blur · 3D, then matte · blend · parent.
    testWidgets('the outline columns sit in their groups', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      double dx(String key) =>
          tester.getTopLeft(find.byKey(ValueKey<String>(key))).dx;
      // No `tl-audible` here: a solid has never made a sound, so since K-435 it
      // is offered no speaker. Its cell keeps its width, which is why the
      // switches after it still line up with the header's columns.
      final order = [
        'tl-visible-$id',
        'tl-solo-$id',
        'tl-locked-$id',
        'tl-shy-$id',
        'tl-twirl-$id',
        'tl-label-$id',
        'tl-name-$id',
        'tl-fx-$id',
        'tl-mb-$id',
        'tl-3d-$id',
        'tl-matte-$id',
        'tl-blend-$id',
        'tl-parent-$id',
      ];
      for (var i = 1; i < order.length; i++) {
        expect(dx(order[i]), greaterThan(dx(order[i - 1])),
            reason: '${order[i]} sits right of ${order[i - 1]}');
      }
    });

    /// Dragging a header group moves the whole cluster: dropping the
    /// switches group onto the compose group puts every switch cell right of
    /// the pickers, in one gesture.
    testWidgets('dragging a header group reorders the columns as a unit',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      double dx(String key) =>
          tester.getTopLeft(find.byKey(ValueKey<String>(key))).dx;
      expect(dx('tl-visible-$id'), lessThan(dx('tl-matte-$id')));

      final from =
          tester.getCenter(find.byKey(const ValueKey('tl-colgroup-switches')));
      final to =
          tester.getCenter(find.byKey(const ValueKey('tl-colgroup-compose')));
      final gesture = await tester.startGesture(from);
      await tester.pump(const Duration(milliseconds: 200));
      final step = (to - from) / 8;
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(step);
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      for (final key in [
        'tl-visible-$id',
        'tl-solo-$id',
        'tl-locked-$id',
        'tl-shy-$id',
      ]) {
        expect(dx(key), greaterThan(dx('tl-parent-$id')),
            reason: 'the whole switches cluster moved past the pickers');
      }
    });

    /// The render switches reach the document like the A/V ones do.
    testWidgets('the fx, motion-blur and 3D switches reach the document',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      expect(layer.getSwitches().fx, isTrue);
      await tester.tap(find.byKey(ValueKey<String>('tl-fx-$id')));
      await tester.pump();
      expect(layer.getSwitches().fx, isFalse);

      expect(layer.getSwitches().motionBlur, isFalse);
      await tester.tap(find.byKey(ValueKey<String>('tl-mb-$id')));
      await tester.pump();
      expect(layer.getSwitches().motionBlur, isTrue);

      expect(layer.getSwitches().threeD, isFalse);
      await tester.tap(find.byKey(ValueKey<String>('tl-3d-$id')));
      await tester.pump();
      expect(layer.getSwitches().threeD, isTrue);
    });

    /// The toolbar's readouts: the timecode counts frames at the comp's own
    /// rate and the frame count is zero-based, so frame 0 is 00:00:00:00.
    testWidgets('the timecode and frame readouts follow the playhead',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      expect(find.text('00:00:00:00'), findsOneWidget);
      expect(find.text('f0'), findsOneWidget);

      // 60 fps is the default comp rate: frame 90 is a second and a half in.
      p.uiState.playheadFrame.value = 90;
      await tester.pump();
      expect(find.text('00:00:01:30'), findsOneWidget);
      expect(find.text('f90'), findsOneWidget);
    });

    /// The master motion-blur button writes the comp's shutter enable — one
    /// op, undoable — and lights when it is on.
    testWidgets('the master motion-blur button toggles the comp shutter',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('tl-mb-master')));
      await tester.pump();
      expect(p.uiState.model.motionBlurEnabled, isTrue);

      await tester.tap(find.byKey(const ValueKey('tl-mb-master')));
      await tester.pump();
      expect(p.uiState.model.motionBlurEnabled, isFalse);
    });

    /// Shy (docs/07 §4.2): the row switch marks the layer, the toolbar's
    /// filter hides marked rows from the list — and only from the list.
    testWidgets('the shy filter hides shy rows without touching visibility',
        (tester) async {
      final p = withComp();
      final shy = p.comp.addSolidLayer();
      shy.rename(name: 'Backplate');
      final loud = p.comp.addSolidLayer();
      loud.rename(name: 'Hero');
      await mount(tester, p);

      await tester
          .tap(find.byKey(ValueKey<String>('tl-shy-${shy.internallayerId}')));
      await tester.pump();
      expect(shy.getSwitches().shy, isTrue,
          reason: 'shy is a document switch, so it survives the session');
      expect(find.text('Backplate'), findsNWidgets(2),
          reason: 'marking a layer shy does not hide it yet — the name is in '
              'the outline and on the bar (§12A.1)');

      await tester.tap(find.byKey(const ValueKey('tl-hide-shy')));
      await tester.pump();
      expect(find.text('Backplate'), findsNothing);
      expect(find.text('Hero'), findsNWidgets(2));
      expect(shy.getSwitches().visible, isTrue,
          reason: 'shy hides the row, never the picture');

      await tester.tap(find.byKey(const ValueKey('tl-hide-shy')));
      await tester.pump();
      expect(find.text('Backplate'), findsNWidgets(2));
    });

    /// Dragging a layer by its name moves it up or down the stack — layers
    /// used to be stuck in the order they were added, reorderable only from
    /// the row menu one place at a time (docs/07 §4.7).
    testWidgets('dragging a layer by its name reorders the stack',
        (tester) async {
      final p = withComp();
      for (final name in ['Bottom', 'Middle', 'Top']) {
        p.comp.addSolidLayer().rename(name: name);
      }
      p.uiState.model.refresh();
      await mount(tester, p);

      List<String> stack() => [for (final l in p.comp.getLayers()) l.getName()];
      expect(stack(), ['Top', 'Middle', 'Bottom'],
          reason: 'newest on top, as added');

      // Drag the top layer's name down onto the bottom row.
      final from = find.byKey(ValueKey<String>(
          'tl-name-${p.comp.getLayers().first.internallayerId}'));
      final onto = find.byKey(ValueKey<String>(
          'tl-row-${p.comp.getLayers().last.internallayerId}'));
      final start = tester.getCenter(from);
      final end = tester.getCenter(onto);
      final gesture = await tester.startGesture(start);
      await tester.pump(const Duration(milliseconds: 200));
      for (var i = 1; i <= 8; i++) {
        await gesture.moveTo(start + (end - start) * (i / 8));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(stack(), ['Middle', 'Bottom', 'Top'],
          reason: 'the dragged layer took the row it was dropped on');

      // One op: a single undo puts the stack back.
      p.state.project!.undo();
      p.uiState.model.refresh();
      expect(stack(), ['Top', 'Middle', 'Bottom']);
    });

    /// Mid-drag, **both halves of the table move** (K-208): the name in the
    /// outline and the bar in the lanes belong to one layer, and the lanes
    /// used to sit still while the names slid, because only the outline knew
    /// a drag was happening.
    testWidgets('a layer drag slides the outline and the lanes together',
        (tester) async {
      final p = withComp();
      for (final name in ['Bottom', 'Top']) {
        p.comp.addSolidLayer().rename(name: name);
      }
      p.uiState.model.refresh();
      await mount(tester, p);

      final top = p.comp.getLayers().first.internallayerId;
      final bottom = p.comp.getLayers().last.internallayerId;
      Rect rowOf(Object id) =>
          tester.getRect(find.byKey(ValueKey<String>('tl-rowbody-$id')));
      Rect barOf(Object id) =>
          tester.getRect(find.byKey(ValueKey<String>('tl-bar-$id')));

      final rowBefore = rowOf(top);
      final barBefore = barOf(top);
      final passedBefore = rowOf(bottom);
      // The two halves are level to begin with — the outline's headers and the
      // lane side's ruler come to the same height (docs/07 §4.1).
      expect(barBefore.top, closeTo(rowBefore.top, 0.5));

      // Lift the top layer's name and hold it over the bottom row — no drop.
      final from = find.byKey(ValueKey<String>('tl-name-$top'));
      final onto = find.byKey(ValueKey<String>('tl-row-$bottom'));
      final start = tester.getCenter(from);
      final end = tester.getCenter(onto);
      final gesture = await tester.startGesture(start);
      await tester.pump(const Duration(milliseconds: 200));
      for (var i = 1; i <= 4; i++) {
        await gesture.moveTo(start + (end - start) * (i / 4));
        await tester.pump();
      }
      // Past the slide's 120ms, so the rows have arrived rather than being
      // caught part-way.
      await tester.pump(const Duration(milliseconds: 200));

      final rowShift = rowOf(top).top - rowBefore.top;
      final barShift = barOf(top).top - barBefore.top;
      expect(rowShift, greaterThan(1),
          reason: 'the dragged layer is on its way down the outline');
      expect(barShift, closeTo(rowShift, 0.5),
          reason: 'its bar went exactly as far, at the same time');

      // And the layer it is passing moved the other way, in both halves,
      // still level with each other.
      expect(rowOf(bottom).top - passedBefore.top, lessThan(-1));
      expect(barOf(bottom).top, closeTo(rowOf(bottom).top, 0.5));

      await gesture.up();
      await tester.pumpAndSettle();
    });

    /// Lock (docs/07 §4.2): a locked layer's bar refuses the drag and its
    /// name refuses the rename, until it is unlocked.
    testWidgets('a locked layer cannot be dragged or renamed', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      await tester.tap(find.byKey(ValueKey<String>('tl-locked-$id')));
      await tester.pump();
      expect(layer.getSwitches().locked, isTrue);

      final before = p.comp.frameAtTime(time: layer.getSpan().inPoint);
      final bar = find.byKey(ValueKey<String>('tl-bar-$id'));
      final rect = tester.getRect(bar);
      await tester.dragFrom(
        Offset(rect.left + rect.width * 0.5, rect.center.dy),
        const Offset(80, 0),
      );
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: layer.getSpan().inPoint), before,
          reason: 'a locked bar holds still');

      await tester.tap(find.byKey(ValueKey<String>('tl-name-$id')));
      // Past the double-tap window, so the recognizer's countdown is not still
      // running when the test ends.
      await tester.pump(kDoubleTapTimeout);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      expect(find.byKey(ValueKey<String>('tl-rename-$id')), findsNothing,
          reason: 'a locked name does not open the editor');
    });

    /// **The Timeline's half of Copy effect** (K-275). An effect's heading in
    /// the fold-out offers it; the groupings around it — Transform, Effects,
    /// Masks, Audio — are not things that can be copied and offer nothing.
    testWidgets("an effect's heading in the fold-out copies that effect",
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      await mount(tester, p);
      final id = layer.internallayerId;

      await openFold(tester, id);
      await settleFrb(tester, minRounds: 4);
      // The effect's own heading sits inside the Effects group, so that has to
      // be open before there is a row to right-click.
      final effects = find.byKey(ValueKey<String>('tl-group-$id/effects'));
      final effectsRect = tester.getRect(effects);
      await tester.tapAt(Offset(effectsRect.left + 6, effectsRect.center.dy));
      await tester.pump();
      await settleFrb(tester, minRounds: 4);

      final effect = layer.getEffects().single;
      final heading =
          find.byKey(ValueKey<String>('tl-group-$id/effects/${effect.id()}'));
      expect(heading, findsOneWidget, reason: 'the effect has a heading row');

      expect(p.uiState.clipboard.kind, isNull);
      await tester.tapAt(tester.getCenter(heading), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(ValueKey<String>('tl-fx-menu-copy-${effect.id()}')));
      await tester.pumpAndSettle();
      expect(p.uiState.clipboard.kind, ClipboardKind.effects);

      // A grouping offers no menu: right-clicking Transform must not open one.
      await tester.tapAt(
        tester
            .getCenter(find.byKey(ValueKey<String>('tl-group-$id/transform'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      expect(find.textContaining('Copy effect'), findsNothing,
          reason: 'Transform is a grouping, not a thing that can be copied');
    });

    /// **Clicking an effect's heading picks it** (K-300). A heading only
    /// twirled before, so an effect could not be selected in the Timeline at
    /// all — and Copy, which acts on the selection, had nothing to take from
    /// here. The pick is the shell's, so the Effect controls panel shows the
    /// same one; the twirl beside the name still only twirls.
    testWidgets("clicking an effect's heading picks it for Copy",
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      p.uiState.setSelection([layer]);
      await mount(tester, p);
      final id = layer.internallayerId;

      await openFold(tester, id);
      await settleFrb(tester, minRounds: 4);
      final effects = find.byKey(ValueKey<String>('tl-group-$id/effects'));
      await tester.tapAt(Offset(
          tester.getRect(effects).left + 6, tester.getCenter(effects).dy));
      await tester.pump();
      await settleFrb(tester, minRounds: 4);

      final effect = layer.getEffects().single;
      expect(p.uiState.selectedEffects.value, isEmpty);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-group-$id/effects/${effect.id()}')));
      await tester.pump();
      await settleFrb(tester, minRounds: 4);
      expect(p.uiState.selectedEffects.value, [effect.id()],
          reason: 'the row is picked, and the shell knows which effect it is');

      expect(copySelectionFrb(p.uiState), isTrue);
      expect(p.uiState.clipboard.kind, ClipboardKind.effects,
          reason: 'Copy took the picked effect, not the layer under it');
    });

    /// **A locked layer's property rows are read-only too** (K-291). The lock
    /// used to guard only the *gestures* — the bar, the razor, rename, reorder,
    /// delete — while the fold-out's transform, effect and volume rows went on
    /// editing the layer, so the switch did not mean what it says.
    ///
    /// Two halves, and this is the interface one: the rows are shown, and their
    /// numbers are still the document's, but nothing on them can be touched. The
    /// engine refuses the edit as well (`OpError::LayerLocked`, covered in
    /// lumit-core), so this is what stops the interface offering a gesture that
    /// would only be refused.
    testWidgets("a locked layer's property rows cannot be touched",
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      // Twirl the layer open so its Transform rows are on screen.
      await openFold(tester, id);
      await settleFrb(tester, minRounds: 4);
      final transformGroup =
          find.byKey(ValueKey<String>('tl-group-$id/transform'));
      final groupRect = tester.getRect(transformGroup);
      await tester.tapAt(Offset(groupRect.left + 6, groupRect.center.dy));
      await tester.pump();
      await settleFrb(tester, minRounds: 4);

      final position = find.byType(TransformRowFrb);
      expect(position, findsWidgets,
          reason: 'the transform rows are on screen');
      expect(
        find.ancestor(of: position.first, matching: find.byType(AbsorbPointer)),
        findsNothing,
        reason: 'an unlocked layer\'s rows are live',
      );

      await tester.tap(find.byKey(ValueKey<String>('tl-locked-$id')));
      await tester.pump();
      await settleFrb(tester, minRounds: 4);
      expect(layer.getSwitches().locked, isTrue);

      expect(position, findsWidgets,
          reason: 'a locked row is shown, not hidden — the numbers still read');
      expect(
        find.ancestor(of: position.first, matching: find.byType(AbsorbPointer)),
        findsWidgets,
        reason: 'but nothing on it can be touched',
      );
      // The group heading stays live: twirling one open is navigation, not
      // editing, and a locked layer you could not look inside would be worse.
      final group = find.byKey(ValueKey<String>('tl-group-$id/transform'));
      expect(
        find.ancestor(of: group, matching: find.byType(AbsorbPointer)),
        findsNothing,
        reason: 'a group row is exempt',
      );
    });

    /// Enter turns the selected layer's name into an editor (K-243); submitting
    /// renames the layer through the document (one op, undoable like any
    /// other). It used to be a double-click, which now opens the layer.
    testWidgets('Enter renames the selected layer', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      await tester.tap(find.byKey(ValueKey<String>('tl-name-$id')));
      await tester.pump(kDoubleTapTimeout);
      expect(p.uiState.selectedLayer.value?.internallayerId, id,
          reason: 'the click picked it');

      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();

      final editor = find.byKey(ValueKey<String>('tl-rename-$id'));
      expect(editor, findsOneWidget, reason: 'the name became a field');

      await tester.enterText(editor, 'Hero solid');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(layer.getInfo().name, 'Hero solid');
      expect(find.byKey(ValueKey<String>('tl-rename-$id')), findsNothing,
          reason: 'submitting leaves the editor');

      // Escape leaves it the other way (K-323): editor shut, nothing written.
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      await tester.enterText(
          find.byKey(ValueKey<String>('tl-rename-$id')), 'Regretted');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(ValueKey<String>('tl-rename-$id')), findsNothing,
          reason: 'Escape closes the editor');
      expect(layer.getInfo().name, 'Hero solid',
          reason: 'and the layer keeps the name it had');
    });

    /// Clicking away from the rename editor finishes the edit and keeps what
    /// was typed (K-243). Pressing Enter is not the only way people leave a
    /// field, and the edit used to sit there open and then be lost.
    testWidgets('clicking elsewhere commits the rename', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      final other = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      await tester.tap(find.byKey(ValueKey<String>('tl-name-$id')));
      await tester.pump(kDoubleTapTimeout);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();

      final editor = find.byKey(ValueKey<String>('tl-rename-$id'));
      expect(editor, findsOneWidget);
      await tester.enterText(editor, 'Backplate');
      await tester.pump();

      // No Enter: click another row, the way a person would.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-name-${other.internallayerId}')));
      await tester.pump(kDoubleTapTimeout);

      expect(layer.getInfo().name, 'Backplate',
          reason: 'the edit was kept, not thrown away');
      expect(find.byKey(ValueKey<String>('tl-rename-$id')), findsNothing,
          reason: 'and the editor closed');
    });

    /// Double-clicking a Precomp layer opens the comp it draws (K-243) — the
    /// same thing the Project panel and the Hierarchy do, and what a
    /// double-click means everywhere else in the application.
    testWidgets('double-clicking a precomp layer opens its comp',
        (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Inner');
      final layer = p.comp.addPrecompLayer(comp: inner);
      await mount(tester, p);
      final id = layer.internallayerId;

      final name = find.byKey(ValueKey<String>('tl-name-$id'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      expect(p.uiState.selectedComp?.getSettings().name, 'Inner',
          reason: 'the nested comp is fronted');
      expect(find.byKey(ValueKey<String>('tl-rename-$id')), findsNothing,
          reason: 'and nothing is being renamed');
    });

    /// Every other kind has no window of its own yet, so a double-click on one
    /// does nothing at all — and in particular does not rename it.
    testWidgets('double-clicking any other layer does nothing', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;
      final before = p.uiState.selectedComp;

      final name = find.byKey(ValueKey<String>('tl-name-$id'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      expect(find.byKey(ValueKey<String>('tl-rename-$id')), findsNothing,
          reason: 'a double-click is not a rename any more');
      expect(p.uiState.selectedComp, before, reason: 'and fronts nothing');
    });

    /// Clicking anywhere on a layer selects it — including its bar in the
    /// lane area, which is most of what "the layer" is on screen.
    testWidgets('clicking a bar selects its layer', (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      await mount(tester, p);

      expect(p.uiState.selectedLayer.value, isNull);
      await tester
          .tap(find.byKey(ValueKey<String>('tl-bar-${top.internallayerId}')));
      await tester.pump();
      expect(
          p.uiState.selectedLayer.value?.internallayerId, top.internallayerId);
    });

    /// Selection happens on the pointer DOWN, not after the gesture arena
    /// settles: the name's rename double-tap holds the arena open for its
    /// whole ~300 ms window, so selecting through the row's tap made the
    /// Effect controls follow a click on the name a third of a second late.
    testWidgets('clicking a name selects before the double-tap window',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      await mount(tester, p);
      expect(p.uiState.selectedLayer.value, isNull);

      final gesture = await tester.startGesture(tester.getCenter(
          find.byKey(ValueKey<String>('tl-name-${top.internallayerId}'))));
      await tester.pump();

      // Still mid-press: the button has not even come up yet.
      expect(
          p.uiState.selectedLayer.value?.internallayerId, top.internallayerId,
          reason: 'selection lands on the down, before any arena resolves');

      await gesture.up();
      // Drain the rename recogniser's double-tap timer before teardown.
      await tester.pump(kDoubleTapTimeout * 2);
    });

    /// Touching a layer's fold-out highlights the layer a shade DIMMER than
    /// selection — "whose rows are these" answered at a glance, without the
    /// touch stealing the selection.
    testWidgets(
        'touching a fold row highlights its layer, dimmer than '
        'selection', (tester) async {
      final p = withComp();
      final below = p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      await mount(tester, p);

      // Select the top layer (a single click on the name selects once the
      // double-tap window has passed — the same click-and-a-beat AE has),
      // twirl open the one below and touch its fold.
      await tester
          .tap(find.byKey(ValueKey<String>('tl-name-${top.internallayerId}')));
      await tester.pump(kDoubleTapTimeout * 2);
      await openFold(tester, below.internallayerId, group: 'Transform');

      Color? rowColour(UuidValue id) {
        // The row's fill rides in the body's decoration, inside the drop
        // target that makes the row a reorder destination (K-193).
        final deco = tester
            .widget<Container>(find.byKey(ValueKey<String>('tl-rowbody-$id')))
            .decoration as BoxDecoration;
        return deco.color;
      }

      final t = LumitTheme.dark();
      expect(rowColour(top.internallayerId), t.selectionFill,
          reason: 'the selected layer keeps the full surface');
      expect(rowColour(below.internallayerId),
          t.selectionFill.withValues(alpha: 0.45),
          reason: 'the touched fold marks its layer at half strength');
      expect(
          p.uiState.selectedLayer.value?.internallayerId, top.internallayerId,
          reason: 'the highlight never steals the selection');
    });

    /// The matte cell: pick a source layer and the mode toggles appear; the
    /// choice reaches the document, luma and invert flip on their toggles.
    testWidgets('the matte cell sets, retargets and flips the matte',
        (tester) async {
      final p = withComp();
      final source = p.comp.addSolidLayer();
      source.rename(name: 'Matte source');
      final consumer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = consumer.internallayerId;

      expect(consumer.getMatte(), isNull);
      await tester.tap(find.byKey(ValueKey<String>('tl-matte-$id')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Matte source').last);
      await tester.pumpAndSettle();

      var matte = consumer.getMatte();
      expect(matte?.layer, source.internallayerId);
      expect(matte?.luma, isFalse, reason: 'alpha until asked otherwise');

      await tester.tap(find.byKey(ValueKey<String>('tl-matte-luma-$id')));
      await tester.pumpAndSettle();
      matte = consumer.getMatte();
      expect(matte?.luma, isTrue);

      await tester.tap(find.byKey(ValueKey<String>('tl-matte-invert-$id')));
      await tester.pumpAndSettle();
      expect(consumer.getMatte()?.inverted, isTrue);
    });

    /// The label swatch opens the eight-chip picker and the choice lands on
    /// the layer.
    testWidgets('the label swatch recolours the layer', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      expect(layer.getInfo().label, 2,
          reason: 'a solid starts on its kind\'s default chip (K-188)');
      await tester.tap(
          find.byKey(ValueKey<String>('tl-label-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tl-label-chip-3')));
      await tester.pumpAndSettle();
      expect(layer.getInfo().label, 3);
    });

    testWidgets(
        'dragging a transform value in the Timeline reaches the document',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      final before =
          (layer.getTransform().positionX as BridgeScalar_Static).field0;
      await tester.drag(
          find.byKey(const ValueKey('tl-tf-positionX')), const Offset(40, 0));
      await tester.pump();

      expect((layer.getTransform().positionX as BridgeScalar_Static).field0,
          greaterThan(before),
          reason: 'the drag committed, exactly as it does in Effect controls');
    });

    /// An effect adds its own group, and each effect in it opens onto its
    /// parameters — the same rows, and the same drag, the Effect controls panel
    /// shows.
    testWidgets('an effect adds a group whose parameters can be dragged',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      await mount(tester, p);

      await openFold(tester, layer.internallayerId);
      expect(find.text('Effects'), findsOneWidget,
          reason: 'the group appears because there is something in it');

      await tester.tap(find.text('Effects'));
      await tester.pump();
      expect(find.text('Gaussian blur'), findsOneWidget,
          reason: 'one row per effect, by label');
      expect(find.text('Radius'), findsNothing,
          reason: 'and its parameters wait until it is opened');

      await tester.tap(find.text('Gaussian blur'));
      await tester.pump();
      expect(find.text('Radius'), findsOneWidget);

      final id = layer.getEffects().single.id();
      double radius() => ((layer.getEffects().single.getValue(id: 'radius')
                  as BridgeEffectValue_Float)
              .field0 as BridgeScalar_Static)
          .field0;
      final before = radius();

      await tester.drag(
        find.byKey(ValueKey<String>('fx-float-$id-radius')),
        const Offset(50, 0),
      );
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
      expect(radius(), greaterThan(before),
          reason: 'the parameter drag reached the document');
    });

    /// **The switches column shows only what the layer can do (K-435).**
    ///
    /// A music track has never shown anything, so it is offered no eye; a solid
    /// has never made a sound, so it is offered no speaker. Both halves are
    /// asserted together, and each row is checked for the switch it *should*
    /// still have — a bug that drew nothing at all would otherwise pass.
    ///
    /// The wav is placed through the ordinary route, so this also pins that a
    /// file with no picture becomes an Audio layer on its own.
    testWidgets(
        'an audio row has no visibility switch and an image row has no audio switch',
        (tester) async {
      final p = withComp();
      final silent = p.comp.addSolidLayer();
      final music = p.state.project!.importFootage(path: _wavFile('row.wav'));
      p.comp.addFootageLayer(footage: music, asSequence: false);
      await mount(tester, p);

      final audioLayer = p.comp.getLayers().first;
      // The probe is a real trip into FFmpeg, so the answers arrive after a
      // frame or two rather than during the first build.
      await settleFrb(tester, minRounds: 8);

      expect(audioLayer.getKind(), BridgeLayerKind.audio,
          reason: 'a file with no picture is placed as an Audio layer');

      final audioId = audioLayer.internallayerId;
      expect(find.byKey(ValueKey<String>('tl-visible-$audioId')), findsNothing,
          reason:
              'an Audio layer has nothing to show, so it is offered no eye');
      expect(
          find.byKey(ValueKey<String>('tl-audible-$audioId')), findsOneWidget,
          reason: 'but it does have sound, so it keeps its speaker');

      final solidId = silent.internallayerId;
      expect(find.byKey(ValueKey<String>('tl-audible-$solidId')), findsNothing,
          reason: 'a solid can never be heard, so it is offered no speaker');
      expect(
          find.byKey(ValueKey<String>('tl-visible-$solidId')), findsOneWidget,
          reason: 'but it does draw, so it keeps its eye');

      // The cells still line up: the switches after the missing one sit in the
      // same column on both rows, because a hidden switch keeps its width.
      double dx(String key) =>
          tester.getTopLeft(find.byKey(ValueKey<String>(key))).dx;
      expect(dx('tl-solo-$audioId'), dx('tl-solo-$solidId'));
      expect(dx('tl-shy-$audioId'), dx('tl-shy-$solidId'));
    });

    /// The outline's switches are drawn from Lumit's own icon set (K-440,
    /// §12A.1) wherever the set has the mark, and each still flips its glyph
    /// rather than only dimming — a closed eye, a muted speaker, an open lock.
    /// Behaviour is untouched: the same key, the same write.
    testWidgets('the switches wear the Lumit glyphs and still flip',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      String markOn(String key) => tester
          .widget<glyph.LumitIcon>(find.descendant(
            of: find.byKey(ValueKey<String>('$key-$id')),
            matching: find.byType(glyph.LumitIcon),
          ))
          .glyph;

      expect(markOn('tl-visible'), LumitIcons.visible);
      expect(markOn('tl-locked'), LumitIcons.unlocked);
      expect(markOn('tl-solo'), LumitIcons.solo);

      await tester.tap(find.byKey(ValueKey<String>('tl-visible-$id')));
      await tester.pump();
      expect(layer.getSwitches().visible, isFalse,
          reason: 'the write is the one it always was');
      expect(markOn('tl-visible'), LumitIcons.hidden,
          reason: 'and the glyph flips rather than only dimming');

      await tester.tap(find.byKey(ValueKey<String>('tl-locked-$id')));
      await tester.pump();
      expect(layer.getSwitches().locked, isTrue);
      expect(markOn('tl-locked'), LumitIcons.lock);
    });

    /// The Audio group is offered only where there is sound to set. Both halves
    /// matter: a silent layer must not carry a volume control, and one with
    /// audio must.
    testWidgets('the Audio group follows whether the layer can be heard',
        (tester) async {
      final p = withComp();
      final silent = p.comp.addSolidLayer();
      final audible =
          p.state.project!.importFootage(path: _wavFile('tone.wav'));
      p.comp.addFootageLayer(footage: audible, asSequence: false);
      await mount(tester, p);

      final footageLayer = p.comp.getLayers().first;
      // The probe is a real trip into FFmpeg, so the answer arrives after a
      // frame or two rather than during the first build.
      await settleFrb(tester, minRounds: 8);

      await tester.tap(find
          .byKey(ValueKey<String>('tl-twirl-${footageLayer.internallayerId}')));
      await tester.pump();
      expect(find.text('Audio'), findsOneWidget,
          reason: 'the file carries an audio stream');

      await tester.tap(find.text('Audio'));
      await tester.pump();
      expect(find.text('Volume'), findsOneWidget);

      // The waveform lane (K-172): behind its own twirl under Audio, and its
      // lane paints once opened.
      expect(find.text('Waveform'), findsOneWidget);
      expect(
          find.byKey(
              ValueKey<String>('tl-wave-${footageLayer.internallayerId}')),
          findsNothing,
          reason: 'closed until asked — a busy comp only pays for open lanes');
      await tester.tap(find.text('Waveform'));
      await tester.pump();
      expect(
          find.byKey(
              ValueKey<String>('tl-wave-${footageLayer.internallayerId}')),
          findsOneWidget);

      // And the peaks themselves are real: the window asked for, bucketed to
      // the count asked for, with the source's true length beside it — the
      // data the lane maps through in/out/offset. `runAsync`, because a real
      // decode completes on real async, which the test's fake clock would
      // otherwise wait on for ever.
      final peaks = await tester.runAsync(() => footageLayer.audioPeaks(
            startSeconds: 0,
            endSeconds: 0.1,
            buckets: 64,
            multiwave: false,
          ));
      expect(peaks!.durationSeconds, greaterThan(0));
      expect(peaks.bands, 1, reason: 'one plain wave');
      expect(peaks.buckets, 64);
      expect(peaks.values, hasLength(64 * 3),
          reason: 'a (min, max, rms) per bucket');
      expect(peaks.values.any((v) => v.abs() > 0.01), isTrue,
          reason: 'a tone is not silence');

      // The multiwave stack: the same buckets three times over, bass, middle
      // and treble (K-280).
      final stack = await tester.runAsync(() => footageLayer.audioPeaks(
            startSeconds: 0,
            endSeconds: 0.1,
            buckets: 64,
            multiwave: true,
          ));
      expect(stack!.bands, 3);
      expect(stack.values, hasLength(3 * 64 * 3));
      // A 440 Hz square is a middle-band sound: its own band carries far more
      // than the treble one, which is the whole point of the stack.
      double loudest(int band) {
        var most = 0.0;
        for (var i = 0; i < 64; i++) {
          final v = stack.values[3 * (band * 64 + i) + 1].abs();
          if (v > most) most = v;
        }
        return most;
      }

      expect(loudest(1), greaterThan(loudest(2)),
          reason: 'the middle band hears the tone, the treble barely does');

      // Zooming in asks for a shorter window, and what comes back is a summary
      // of *that* window — which is what makes the drawn detail follow the
      // zoom instead of stretching one fixed summary (K-280).
      final zoomed = await tester.runAsync(() => footageLayer.audioPeaks(
            startSeconds: 0.02,
            endSeconds: 0.03,
            buckets: 64,
            multiwave: false,
          ));
      expect(zoomed!.startSeconds, closeTo(0.02, 1e-9));
      expect(zoomed.endSeconds, closeTo(0.03, 1e-9));
      expect(zoomed.buckets, 64,
          reason: 'a tenth of the audio, in the same number of buckets');

      await openFold(tester, silent.internallayerId);
      expect(find.text('Audio'), findsOneWidget,
          reason: 'still only the one — a solid has nothing to be heard');
    });

    /// The open lane is drawn over **both** rows — its own, and the empty one
    /// the Waveform twirl sits in directly above it (K-437). The row itself
    /// keeps its height; only the painting reaches up, so nothing in the
    /// outline or the lane stack moves.
    testWidgets('the waveform lane is drawn across both rows', (tester) async {
      final p = withComp();
      final audible =
          p.state.project!.importFootage(path: _wavFile('both-rows.wav'));
      p.comp.addFootageLayer(footage: audible, asSequence: false);
      await mount(tester, p);
      final layer = p.comp.getLayers().first;
      await settleFrb(tester, minRounds: 8);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Audio'));
      await tester.pump();
      await tester.tap(find.text('Waveform'));
      await tester.pump();

      final lane =
          find.byKey(ValueKey<String>('tl-wave-${layer.internallayerId}'));
      expect(lane, findsOneWidget);
      final row = tester.getRect(lane);
      final painter =
          (tester.widget<CustomPaint>(lane).painter) as WaveformPainter;
      expect(painter.height, closeTo(row.height * 2, 0.01),
          reason: 'the paint rect spans the pair of rows');
      // Anchored to the bottom of its own row, so what it borrows is the row
      // above — the empty one belonging to the twirl.
      expect(row.height, greaterThan(0));
    });

    /// A clip on a Sequence layer draws its own sound inside its own box, the
    /// way a Footage layer's lane does (K-280): a cut is aimed at what you can
    /// see, and on a Sequence layer what you are cutting is the clip.
    testWidgets('a Sequence layer\'s clips draw their sound', (tester) async {
      final p = withComp();
      // Two files, because no test can hand-write a container holding both a
      // picture and a soundtrack. The y4m is what makes this a Sequence layer
      // at all — only media that *runs* is placed as clips (K-246) — and the
      // relink then points that same item at a file with sound in it, which is
      // what the clip's waveform is drawn from.
      final footage =
          p.state.project!.importFootage(path: _highRateVideoFile('cut.y4m'));
      p.comp.addFootageLayer(footage: footage, asSequence: true);
      footage.relink(path: _wavFile('clip.wav'));
      await mount(tester, p);
      final layer = p.comp.getLayers().first;
      await settleFrb(tester, minRounds: 8);

      final clip = layer.getInfo().clips.single;
      final wave = find.byKey(ValueKey<String>('seq-wave-${clip.id}'));
      expect(wave, findsNothing, reason: 'the view is shut to begin with');

      // Double-clicking the bar opens the clip view. Retried: the first tap
      // selects and can rebuild the row under the second, which on a loaded
      // runner reads as two singles rather than a double.
      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      final view =
          find.byKey(ValueKey<String>('tl-seq-${layer.internallayerId}'));
      for (var attempt = 0; attempt < 3; attempt++) {
        await tester.tap(bar);
        await tester.pump(const Duration(milliseconds: 30));
        await tester.tap(bar);
        await tester.pumpAndSettle();
        await settleFrb(tester, until: () => view.evaluate().isNotEmpty);
        if (view.evaluate().isNotEmpty) break;
        await tester.pump(const Duration(milliseconds: 400));
      }
      expect(view, findsOneWidget, reason: 'the clip view opened');

      // The peaks are a real decode, so they arrive a frame or two later.
      await settleFrb(tester,
          until: () => wave.evaluate().isNotEmpty, maxRounds: 60);
      expect(wave, findsOneWidget, reason: 'the clip draws its waveform');
      final painter =
          (tester.widget<CustomPaint>(wave).painter) as WaveformPainter;
      expect(painter.peaks?.values.any((v) => v.abs() > 0.01), isTrue,
          reason: 'and what it draws is the sound, not silence');
    });

    /// **A retimed layer's wave stretches with its map** (K-436). The buckets
    /// are taken in the layer's own clock and mapped through its Retime, so a
    /// half-speed layer showing the first tenth of its bar is showing the
    /// first *twentieth* of its source — which for this file is the silent
    /// half. Bucketed evenly in source time instead, the tone would still be
    /// there and the transients would sit in the wrong columns.
    testWidgets('a retimed layer\'s waveform stretches with the map',
        (tester) async {
      final p = withComp();
      // Silence for the first half of the file, a tone for the second.
      final audible = p.state.project!
          .importFootage(path: _wavFile('ramp.wav', halfSilent: true));
      p.comp.addFootageLayer(footage: audible, asSequence: false);
      await mount(tester, p);
      final layer = p.comp.getLayers().first;
      await settleFrb(tester, minRounds: 8);

      Future<List<double>> band(double from, double to) async {
        final peaks = await tester.runAsync(() => layer.audioPeaks(
              startSeconds: from,
              endSeconds: to,
              buckets: 64,
              multiwave: false,
            ));
        return [for (final v in peaks!.values) v.abs()];
      }

      double loudest(List<double> v, int fromBucket, int toBucket) {
        var most = 0.0;
        for (var i = fromBucket * 3; i < toBucket * 3; i++) {
          if (v[i] > most) most = v[i];
        }
        return most;
      }

      // Un-retimed: the layer's clock is the source's, so the file's two
      // halves land in the lane's two halves.
      final plain = await band(0, 0.1);
      expect(loudest(plain, 0, 30), lessThan(0.05),
          reason: 'the first half of the file is silent');
      expect(loudest(plain, 34, 64), greaterThan(0.2),
          reason: 'and the second half carries the tone');

      // The identity map changes nothing: switching Retime on is not retiming.
      expect(layer.toggleRetimeProperty(), isTrue);
      final identity = await band(0, 0.1);
      expect(loudest(identity, 0, 30), lessThan(0.05));
      expect(loudest(identity, 34, 64), greaterThan(0.2));

      // Half speed: layer time 0.2 s reaches source time 0.1 s.
      layer.setRetimeProperty(
        value: BridgeScalar.keyframed([
          const BridgeKeyframe(
            time: BridgeRational(num: 0, den: 1),
            value: 0,
            interpIn: BridgeSideInterp.linear(),
            interpOut: BridgeSideInterp.linear(),
          ),
          const BridgeKeyframe(
            time: BridgeRational(num: 1, den: 5),
            value: 0.05,
            interpIn: BridgeSideInterp.linear(),
            interpOut: BridgeSideInterp.linear(),
          ),
        ]),
      );

      // The same tenth of a second of the bar now reads the first twentieth
      // of the file, which is silence all the way across.
      final slow = await band(0, 0.1);
      expect(loudest(slow, 0, 64), lessThan(0.05),
          reason: 'at half speed the whole window is still the silent half');
      // And twice the window reaches the tone again, in the same place the
      // un-retimed lane found it: the drawing has stretched, not moved.
      final wide = await band(0, 0.2);
      expect(loudest(wide, 0, 30), lessThan(0.05));
      expect(loudest(wide, 34, 64), greaterThan(0.2));
    });

    /// `L` opens a layer's sound, `LL` its waveform, `LLL` shuts it (K-281) —
    /// the same three-tap shape `U` has. A layer selected but silent is left
    /// alone rather than opened onto a group it has not got.
    testWidgets('L, LL and LLL cycle a layer\'s Audio open and shut',
        (tester) async {
      final p = withComp();
      final audible =
          p.state.project!.importFootage(path: _wavFile('cycle.wav'));
      p.comp.addFootageLayer(footage: audible, asSequence: false);
      await mount(tester, p);
      final layer = p.comp.getLayers().first;
      await settleFrb(tester, minRounds: 8);

      // Selected, and shut.
      p.uiState.setSelection([layer]);
      await tester.pump();
      expect(find.text('Audio'), findsNothing);

      await tester.sendKeyEvent(LogicalKeyboardKey.keyL);
      await tester.pump();
      expect(find.text('Volume'), findsOneWidget,
          reason: 'L opens the Audio group');
      expect(find.byKey(ValueKey<String>('tl-wave-${layer.internallayerId}')),
          findsNothing,
          reason: 'the lane waits for the second tap');

      await tester.sendKeyEvent(LogicalKeyboardKey.keyL);
      await tester.pump();
      expect(find.byKey(ValueKey<String>('tl-wave-${layer.internallayerId}')),
          findsOneWidget,
          reason: 'LL opens the waveform lane');

      await tester.sendKeyEvent(LogicalKeyboardKey.keyL);
      await tester.pump();
      expect(find.text('Volume'), findsNothing,
          reason: 'LLL shuts the audio stuff again');
      expect(find.text('Audio'), findsNothing);
    });

    /// The outline and the lanes are one table. A fold-out that pushed the names
    /// down without leaving the same room beside them would slide every bar
    /// below it away from its own layer.
    testWidgets('an open layer keeps its bars lined up with its names',
        (tester) async {
      final p = withComp();
      final upper = p.comp.addSolidLayer();
      final lower = p.comp.addSolidLayer();
      await mount(tester, p);

      Finder rowOf(LayerReference l) =>
          find.byKey(ValueKey<String>('tl-row-${l.internallayerId}'));
      Finder barOf(LayerReference l) =>
          find.byKey(ValueKey<String>('tl-bar-${l.internallayerId}'));

      for (final layer in [upper, lower]) {
        expect(tester.getTopLeft(rowOf(layer)).dy,
            closeTo(tester.getTopLeft(barOf(layer)).dy, 0.01));
      }

      await openFold(tester, upper.internallayerId, group: 'Transform');

      for (final layer in [upper, lower]) {
        expect(
          tester.getTopLeft(rowOf(layer)).dy,
          closeTo(tester.getTopLeft(barOf(layer)).dy, 0.01),
          reason: 'the layer below an open one still meets its own bar',
        );
      }
    });

    // ---------------------------------------------------------------------
    // Keys mode — the dope sheet (K-455, §12A.1).
    //
    // The mode adds no editing behaviour Layers lacks: it is a different
    // arrangement of the same parts, so most of what these claim is that the
    // *same* machinery is running under a different layout — the same key
    // paths, the same drag, the same selection, the same ruler.
    // ---------------------------------------------------------------------

    /// A solid whose Opacity is keyed at [frames], the fixture every claim
    /// below starts from.
    LayerReference keyedLayer(dynamic p, {List<int> frames = const [0, 60]}) {
      final layer = (p.comp as CompositionReference).addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in frames)
            BridgeKeyframe(
              time: (p.comp as CompositionReference).timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      (p.uiState as LumitUiState).model.refresh();
      return layer;
    }

    /// Switch the panel to Keys mode and twirl [layer] open, which is where
    /// its property rows appear.
    Future<void> openKeys(WidgetTester tester, LayerReference layer) async {
      await tester.tap(find.byKey(const ValueKey('tl-view-keys')));
      await tester.pumpAndSettle();
      await tester.tap(find
          .byKey(ValueKey<String>('tl-keys-twirl-${layer.internallayerId}')));
      await tester.pumpAndSettle();
    }

    /// The tab row gains a third segment, between its two (§12A.1, K-455), and
    /// the mode it opens is neither of the other two.
    testWidgets('the mode tabs read Layers, Keys and Graph, in that order',
        (tester) async {
      final p = withComp();
      await mount(tester, p);

      expect(find.text('KEYS'), findsOneWidget);
      final layers =
          tester.getRect(find.byKey(const ValueKey('tl-view-lanes')));
      final keys = tester.getRect(find.byKey(const ValueKey('tl-view-keys')));
      final graph = tester.getRect(find.byKey(const ValueKey('tl-graph')));
      expect(layers.right, lessThanOrEqualTo(keys.left));
      expect(keys.right, lessThanOrEqualTo(graph.left));
      expect(keys.center.dy, moreOrLessEquals(layers.center.dy, epsilon: 2));

      // Keys is a mode of its own: the graph does not come up with it, and
      // the columns Layers mode carries stand down.
      await tester.tap(find.byKey(const ValueKey('tl-view-keys')));
      await tester.pumpAndSettle();
      expect(find.byType(GraphEditorFrb), findsNothing);
      expect(find.byKey(const ValueKey('tl-keys-filters')), findsOneWidget,
          reason: 'the dope sheet\'s filter row replaces the column header');

      // And Layers is unchanged behind it.
      await tester.tap(find.byKey(const ValueKey('tl-view-lanes')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('tl-keys-filters')), findsNothing);
    });

    /// The sheet itself: a layer's own row saying how many properties it
    /// carries, then one flat row per keyed property named by the group it
    /// came out of — `Transform · Opacity` — with what it reads at the
    /// playhead beside it.
    testWidgets('Keys mode lists a layer\'s keyed properties, flat',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openKeys(tester, layer);

      expect(
          find.byKey(ValueKey<String>('tl-keys-row-${layer.internallayerId}')),
          findsOneWidget);
      expect(find.text('1 property'), findsOneWidget,
          reason: 'the layer says how much of it is listed');
      expect(
          find.byKey(ValueKey<String>(
              'tl-keys-prop-${layer.internallayerId}/transform/opacity')),
          findsOneWidget);
      expect(find.text('Transform'), findsOneWidget,
          reason: 'the group the property came out of names it');
      expect(find.text('Opacity'), findsOneWidget);
      // At frame 0 the curve reads 0, written the way the sheet writes whole
      // numbers.
      expect(find.text('0'), findsWidgets);

      // And a lane beside it, at the same path Layers mode uses.
      expect(
          find.byKey(ValueKey<String>(
              'tl-keys-${layer.internallayerId}/transform/opacity')),
          findsOneWidget);
    });

    /// The Animated filter (K-441) applies here as it does in Layers: on — the
    /// default — only keyframed properties are listed; All restores the rest.
    testWidgets('the Animated filter decides what the sheet lists',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openKeys(tester, layer);

      expect(find.text('Position'), findsNothing,
          reason: 'Position is not animated, so Animated leaves it out');

      await tester.tap(find.byKey(const ValueKey('tl-keys-all')));
      await tester.pumpAndSettle();
      expect(find.text('Position'), findsOneWidget,
          reason: 'All restores every property the layer has');
      expect(find.text('Opacity'), findsOneWidget,
          reason: 'and keeps the animated ones');

      await tester.tap(find.byKey(const ValueKey('tl-keys-animated')));
      await tester.pumpAndSettle();
      expect(find.text('Position'), findsNothing);
    });

    /// `U` is the same reveal it has always been: it opens a layer onto what
    /// is animated, and the dope sheet's rows are what it opens onto.
    testWidgets('U opens a layer in Keys mode as it does in Layers',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await tester.tap(find.byKey(const ValueKey('tl-view-keys')));
      await tester.pumpAndSettle();
      expect(
          find.byKey(ValueKey<String>(
              'tl-keys-prop-${layer.internallayerId}/transform/opacity')),
          findsNothing,
          reason: 'the layer starts shut');

      await tester.sendKeyEvent(LogicalKeyboardKey.keyU);
      await tester.pumpAndSettle();
      expect(
          find.byKey(ValueKey<String>(
              'tl-keys-prop-${layer.internallayerId}/transform/opacity')),
          findsOneWidget,
          reason: 'U reveals what is animated, in this mode too');
    });

    /// **The dope sheet's keys are the lanes' keys.** Clicking one selects it
    /// and dragging one moves it — through the very machinery Layers mode
    /// uses, at the same path, committing one op that undoes in one step.
    /// This is the claim K-455 rests on: were Keys a second implementation,
    /// this would need a second set of gestures to pass.
    testWidgets('a key in Keys mode selects and drags like a lane key',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p, frames: [600, 2400]);
      await mount(tester, p);
      await openKeys(tester, layer);

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;
      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      final handle = find.byKey(ValueKey<String>(
          'tl-key-${layer.internallayerId}/transform/opacity#0'));
      expect(handle, findsOneWidget,
          reason: 'the diamond is a drag handle here too');

      final perFrame =
          (tester.getRect(find.byKey(laneKey)).width - TimelineAxis.pad * 2) /
              p.comp.durationFrames();
      await tester.drag(handle, Offset(perFrame * 10.5, 0));
      await tester.pumpAndSettle();

      final moved = p.comp.frameAtTime(time: keys().first.time);
      expect(moved, greaterThan(600), reason: 'the drag moved the key later');
      expect(keys(), hasLength(2), reason: 'no key added or lost');

      p.state.project!.undo();
      expect(p.comp.frameAtTime(time: keys().first.time), 600,
          reason: 'one gesture, one undo step — the lane commit, unchanged');
    });

    /// The ruler, the work area, the cache bar and the playhead are the same
    /// ones Layers mode draws, not copies: a mode switch leaves them where
    /// they were, so the two views scroll the same range (§12A.1).
    testWidgets('Keys mode keeps the shared ruler and playhead',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      p.uiState.playheadFrame.value = 24;
      await mount(tester, p);

      final rulerBefore =
          tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final headBefore = tester.getRect(find.byType(PlayheadMarker).first);
      final zoomBefore = tester
          .widget<HouseSlider>(find.byKey(const ValueKey('tl-zoom-slider')))
          .value;

      await openKeys(tester, layer);

      expect(
          tester.getRect(find.byKey(const ValueKey('tl-ruler'))), rulerBefore,
          reason: 'the ruler does not move on a mode switch');
      expect(tester.getRect(find.byType(PlayheadMarker).first), headBefore,
          reason: 'nor does the playhead');
      // The zoom is the *same control*, at the same setting — one slider, not
      // two. Its x does move, and must: the Keys drawing puts a strip of
      // commands on this bar to the left of it (K-458), so the slider sits
      // along from where an empty Layers bar leaves it. What §12A.1a asks of
      // the zoom is that the two views share it, which is what this reads.
      expect(find.byKey(const ValueKey('tl-zoom-slider')), findsOneWidget,
          reason: 'one zoom slider, shared, not one per mode');
      expect(
          tester
              .widget<HouseSlider>(
                  find.byKey(const ValueKey('tl-zoom-slider')))
              .value,
          zoomBefore,
          reason: 'and it is still where it was left');
      expect(find.byType(TimelineCacheBar), findsWidgets);
    });

    /// The sheet's own metrics, from the approved Keys drawing: the rows are
    /// the density's lane row on both sides, and a key measures 11 point to
    /// point where a Layers lane draws 8 (the dope sheet is where the keys
    /// are the subject).
    testWidgets('the Keys sheet is built to the drawing\'s metrics',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openKeys(tester, layer);
      const d = DensityTokens.regular;

      final row = tester.getRect(
          find.byKey(ValueKey<String>('tl-keys-row-${layer.internallayerId}')));
      final lane = tester.getRect(find
          .byKey(ValueKey<String>('tl-keys-layer-${layer.internallayerId}')));
      final prop = tester.getRect(find.byKey(ValueKey<String>(
          'tl-keys-prop-${layer.internallayerId}/transform/opacity')));
      expect(row.height, closeTo(d.laneRow, 0.5),
          reason: 'a layer\'s row is the density\'s lane row (K-454)');
      expect(prop.height, closeTo(d.laneRow, 0.5),
          reason: 'and so is a property\'s');
      expect(lane.top, closeTo(row.top, 0.5),
          reason: 'the two halves are one table here as well');
      expect(lane.height, closeTo(row.height, 0.5));

      // The filter row is a secondary row, level with the column header it
      // replaces — which is what keeps the ruler opposite it.
      expect(
          tester.getRect(find.byKey(const ValueKey('tl-keys-filters'))).height,
          closeTo(d.secondaryRow, 0.5));

      // The property's value is mono at 10 in `animated`, and the layer's
      // count mono at 9 muted — the drawing's own two sizes.
      final value =
          tester.renderObject<RenderParagraph>(find.text('0').last).text.style!;
      expect(value.fontSize, 10);
      expect(value.color, LumitTheme.dark().animated);
      final count = tester
          .renderObject<RenderParagraph>(find.text('1 property'))
          .text
          .style!;
      expect(count.fontSize, 9);
      expect(count.color, LumitTheme.dark().textMuted);

      expect(keysNumberText(960), '960',
          reason: 'whole numbers plain, as the drawing writes them');
      expect(keysNumberText(1.6), '1.60');

      // The drawing's own two distances: a key spans 11 point to point, and a
      // property row starts 30 in, clear of the twirl and the colour dot.
      expect(laneKeyHalf * 2, 11);
      expect(keysPropertyIndent, 30);
      final group = tester.getRect(find.text('Transform'));
      expect(group.left - prop.left, closeTo(keysPropertyIndent, 0.5),
          reason: 'the group name starts at the drawing\'s indent');
    });

    /// Interpolation is drawn as shape (§6.2): diamond linear, square hold,
    /// circle bezier — the reading a dope sheet exists for.
    testWidgets('a key\'s shape says its interpolation', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 0),
            value: 0,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.hold(),
          ),
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 60),
            value: 100,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openKeys(tester, layer);

      final keys =
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;
      // Linear in, held out: the two halves disagree, and the mark says so.
      expect(keyShapeOf(keys[0]), (KeyShape.diamond, KeyShape.square),
          reason: 'a held key is a step, and a step is drawn square — but '
              'only on the side that holds');
      expect(keyShapeOf(keys[1]), (KeyShape.diamond, KeyShape.diamond));

      // And a bezier side is the hourglass, which supersedes the rounded
      // shape the Keys mode first drew (K-457).
      expect(
          keyShapeOfSide(const BridgeSideInterp.bezier(
              BridgeBezierSide(speed: 0, influence: 33))),
          KeyShape.hourglass);
      expect(keyShapeOfSide(const BridgeSideInterp.hold()), KeyShape.square);
      expect(keyShapeOfSide(const BridgeSideInterp.linear()), KeyShape.diamond);
    });
  }, skip: !engineAvailable);
}

/// Twirl a layer open, and optionally open one group heading under it — the
/// four-line block the fold-out tests were repeating everywhere. [group] taps
/// the heading by its visible label; [groupPath] by its key suffix
/// (`masks`, `paint`, ...). [settle] pumps each tap to rest, for the flows
/// whose fold has async follow-up to finish.
Future<void> openFold(
  WidgetTester tester,
  Object layerId, {
  String? group,
  String? groupPath,
  bool settle = false,
}) async {
  Future<void> pump() => settle ? tester.pumpAndSettle() : tester.pump();
  await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$layerId')));
  await pump();
  if (group != null) {
    await tester.tap(find.text(group));
    await pump();
  }
  if (groupPath != null) {
    await tester
        .tap(find.byKey(ValueKey<String>('tl-group-$layerId/$groupPath')));
    await pump();
  }
}

/// A real, probeable WAV: 16-bit mono PCM, a tenth of a second of silence.
///
/// Written to a temp file **synchronously** — an awaited async `dart:io` call in
/// a `testWidgets` body hangs the test outright (see frb_test_support.dart). The
/// point is only that FFmpeg reports an audio stream, so the samples can be
/// anything.
String _wavFile(String name, {bool halfSilent = false}) {
  final dir = Directory.systemTemp.createTempSync('lumit-audio');
  final file = File('${dir.path}/$name');
  file.writeAsBytesSync(_tinyWav(halfSilent: halfSilent));
  return file.path;
}

/// `halfSilent` puts the tone in the second half of the file and silence in
/// the first, so a test can tell *which stretch of the source* a lane is
/// showing — which is the whole question a retimed waveform asks.
Uint8List _tinyWav({bool halfSilent = false}) {
  const rate = 8000;
  const samples = 800;
  const dataBytes = samples * 2;
  final out = BytesBuilder();
  void ascii(String s) => out.add(s.codeUnits);
  void u16(int v) => out.add([v & 0xff, (v >> 8) & 0xff]);
  void u32(int v) =>
      out.add([v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff]);

  ascii('RIFF');
  u32(36 + dataBytes);
  ascii('WAVE');
  ascii('fmt ');
  u32(16); // PCM header length
  u16(1); // PCM
  u16(1); // mono
  u32(rate);
  u32(rate * 2); // byte rate
  u16(2); // block align
  u16(16); // bits per sample
  ascii('data');
  u32(dataBytes);
  // An actual tone, not silence: a ~440 Hz square wave at half amplitude, so
  // a test asking "does the waveform carry signal" has signal to find.
  final data = Uint8List(dataBytes);
  for (var i = 0; i < samples; i++) {
    if (halfSilent && i < samples ~/ 2) continue;
    final v = (i ~/ 25).isEven ? 16384 : -16384;
    data[i * 2] = v & 0xff;
    data[i * 2 + 1] = (v >> 8) & 0xff;
  }
  out.add(data);
  return out.toBytes();
}

/// A real, probeable 600 fps video on disk: one second of 2×2 YUV4MPEG.
///
/// y4m because it is the one video container a test can write by hand — a
/// plain-text header carrying the exact rational rate, then raw frames — and
/// FFmpeg reads it natively, so the engine's probe reports 600/1 for real.
String _highRateVideoFile(String name) {
  final dir = Directory.systemTemp.createTempSync('lumit-retime');
  final file = File('${dir.path}/$name');
  final out = BytesBuilder();
  out.add('YUV4MPEG2 W2 H2 F600:1 Ip A1:1 C420\n'.codeUnits);
  // 600 frames of grey: per frame, 4 luma bytes and one byte per chroma plane.
  final frame = Uint8List.fromList(
      [...'FRAME\n'.codeUnits, 128, 128, 128, 128, 128, 128]);
  for (var i = 0; i < 600; i++) {
    out.add(frame);
  }
  file.writeAsBytesSync(out.toBytes());
  return file.path;
}
