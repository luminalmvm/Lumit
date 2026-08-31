// The Timeline panel on frb, tested against the real engine.
//
// New coverage: the v0 Timeline's tests are spread across several files and
// written against a fake bridge and a snapshot mirror, neither of which this
// panel has. What they assert about *behaviour* is reproduced here against the
// document itself — a switch that does not reach the engine is not a switch.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
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
import 'package:lumit_flutter/panels/ease_popover.dart';
import 'package:lumit_flutter/panels/timeline_navigator.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/transform_rows_frb.dart';
import 'package:lumit_flutter/panels/waveform_frb.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/state/timeline_columns.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

/// A ticked menu row's mark: the set's checkmark, not the character it used to
/// be (K-440's tick).
final Finder _tick = find.byWidgetPredicate(
    (w) => w is glyph.LumitIcon && w.glyph == LumitIcons.tick);

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

    /// **The time navigator** (T5, K-648): the whole comp as a strip, with the
    /// visible span as a window on it. Dragging the window's right-hand end
    /// inward narrows the view — which is a zoom in — about its left-hand end,
    /// and the lanes widen to match.
    testWidgets('the navigator window zooms the lanes', (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      double laneWidth() =>
          tester.widget<TimelineRuler>(find.byType(TimelineRuler)).axis.width;

      final strip = find.byKey(const ValueKey('tl-navigator'));
      expect(strip, findsOneWidget);
      final box = tester.getRect(strip);
      final fitted = laneWidth();

      // At fit the window is the whole comp, so its right-hand handle sits at
      // the strip's right-hand end. Half the strip's width in is half the comp.
      await tester.dragFrom(
        Offset(box.right - TimelineNavigator.handleGrab / 2, box.center.dy),
        Offset(-box.width / 2, 0),
      );
      await tester.pumpAndSettle();
      expect(laneWidth(), greaterThan(fitted * 1.5),
          reason: 'half the comp across the same panel is about twice the '
              'pixels per frame');
    });

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
        // One height, whatever size their own type is — 11 for the clock, 10
        // for the frame count — and under Regular that height is the one the
        // Timeline's chrome row states for everything standing in it (K-512),
        // not the readout's own [readoutWellHeight]. Compact states none and
        // the well measures itself.
        expect(tester.getRect(find.byKey(ValueKey<String>(key))).height,
            closeTo(DensityTokens.regular.timelineChromeControl!, 0.5),
            reason: 'a well in the chrome row grows with the row');
        expect(readoutWellHeight,
            lessThan(DensityTokens.regular.timelineChromeControl!),
            reason: 'which is taller than the well would size itself to');
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
      expect(find.text('F48'), findsOneWidget,
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
      expect(find.text('F60'), findsOneWidget,
          reason: 'and it wears the f again the moment the edit lands');

      // Escape puts it back, exactly as §12A.3 says an edit is abandoned.
      await tester.tap(find.byKey(const ValueKey('tl-frame')));
      await tester.pump();
      await tester.enterText(fieldIn('tl-frame'), '9');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, 60,
          reason: 'Escape reverts, and the playhead never moved');
      expect(find.text('F60'), findsOneWidget);
    });

    /// **The frame counter says how many frames there are** (§12A.1): the
    /// mockup's `F48 / 250`, the whole phrase in one muted colour. It said
    /// only `F48`, which left the reader with no idea how far in that was.
    ///
    /// **No space after the slash** (K-529, the owner after desktop testing):
    /// the mockup writes one, and on a real composition the phrase then broke
    /// into three marks — a number, a lone stroke, another number — where it
    /// is one reading. The stroke binds to the count it introduces.
    testWidgets('the frame counter carries the comp\'s total', (tester) async {
      final p = withComp();
      p.uiState.playheadFrame.value = 3;
      p.uiState.model.refresh();
      await mount(tester, p);

      final total = p.comp.durationFrames();
      expect(find.text('F3'), findsOneWidget);
      expect(find.text('/$total'), findsOneWidget,
          reason: 'the comp\'s whole length, after the frame in hand');
      expect(find.text('/ $total'), findsNothing,
          reason: 'and nothing between the stroke and the number');
      expect(tester.widget<Text>(find.text('/$total')).style?.color,
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
          vertexFeather: const [],
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
          vertexFeather: const [],
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
      // Not the per-point feather (K-545): this mask has one width, so it
      // has no per-point rows to click.
      for (final value in MaskValue.values) {
        if (value == MaskValue.vertexFeather) continue;
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

      // Not the per-point feather (K-545): this mask has one width, so it
      // has no per-point rows to click.
      for (final value in MaskValue.values) {
        if (value == MaskValue.vertexFeather) continue;
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

    /// **A feather per point is switched on from the mask's own menu, and
    /// gives every point a row** (K-545).
    ///
    /// Switching it on must not move the picture: each point starts at the
    /// width the mask already had, so the change is an offer of control and
    /// not an edit of the shape. The rows that appear key like any other
    /// number, which is what makes one edge animatable soft and another sharp.
    testWidgets('a mask gains a feather row per point, and gives them back',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await openMaskRow(tester, p, layer, 'Ellipse');
      layer.setMask(
          mask: maskWith(layer.getMasks().single,
              feather: const BridgeScalar.static_(12)));
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      final id = layer.getMasks().single.id;

      expect(find.text('Point 1 feather'), findsNothing,
          reason: 'an ordinary mask shows the four rows it always did');

      Future<void> toggleFromMenu() async {
        await tester.tapAt(
            tester.getCenter(find.byKey(ValueKey<String>('tl-mask-name-$id'))),
            buttons: kSecondaryButton);
        await tester.pumpAndSettle();
        await tester
            .tap(find.byKey(ValueKey<String>('tl-mask-vary-feather-$id')));
        await tester.pumpAndSettle();
      }

      await toggleFromMenu();
      final varied = layer.getMasks().single;
      expect(varied.vertexFeather.length, varied.vertices.length,
          reason: 'one width per point of the shape');
      expect(varied.vertexFeather.map(stillValue), everyElement(12),
          reason: 'each point starts at the width the mask already had, so '
              'switching this on does not move the picture');
      expect(find.text('Point 1 feather'), findsOneWidget);
      expect(find.text('Point 3 feather'), findsOneWidget);

      // A per-point row is a value row like any other: its field writes
      // through to that point alone.
      await dragLeft(tester,
          find.byKey(ValueKey<String>('tl-mask-vertexFeather-$id-0')), 20);
      final dragged = layer.getMasks().single;
      expect(stillValue(dragged.vertexFeather[0]), lessThan(12),
          reason: 'the drag reached point 1');
      expect(stillValue(dragged.vertexFeather[1]), 12,
          reason: 'and nothing else');

      await toggleFromMenu();
      expect(layer.getMasks().single.vertexFeather, isEmpty,
          reason: 'switching it off puts the one width back');
      expect(find.text('Point 1 feather'), findsNothing);
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
            BridgeStrokePoint(x: 10, y: 10, pressure: 1),
            BridgeStrokePoint(x: 40, y: 25, pressure: 1),
          ],
          colour: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
          width: 20,
          hardness: 0.8,
          shape: BridgeBrushShape.round,
          opacity: 100,
          start: const BridgeScalar.static_(0),
          end: const BridgeScalar.static_(100),
          mode: BridgePaintMode.paint,
          blend: 0,
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

    /// A stroke's Start and End (K-549) get rows of their own under it, and
    /// both write through — the pair that makes a stroke draw itself on.
    testWidgets('a stroke grows Start and End rows that write through',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addStroke(
        stroke: BridgeStroke(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Brush 1',
          points: const [
            BridgeStrokePoint(x: 10, y: 10, pressure: 1),
            BridgeStrokePoint(x: 40, y: 25, pressure: 1),
          ],
          colour: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
          width: 20,
          hardness: 0.8,
          shape: BridgeBrushShape.round,
          opacity: 100,
          start: const BridgeScalar.static_(0),
          end: const BridgeScalar.static_(100),
          mode: BridgePaintMode.paint,
          blend: 0,
          cloneOffsetX: 0,
          cloneOffsetY: 0,
        ),
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'paint', settle: true);

      expect(find.text('Start'), findsOneWidget);
      expect(find.text('End'), findsOneWidget);

      final id = layer.getPaint().single.id;
      final end = find.byKey(ValueKey<String>('tl-stroke-end-$id'));
      await tester.tap(end);
      await tester.pumpAndSettle();
      await tester.enterText(end, '40');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      final written = layer.getPaint().single.end;
      expect(written, isA<BridgeScalar_Static>());
      expect((written as BridgeScalar_Static).field0, 40);
      expect(layer.getPaint().single.start, isA<BridgeScalar_Static>(),
          reason: 'and Start is left where it was');
    });

    /// A stroke's blend mode is the layer blend list, on its own row (K-550).
    testWidgets('a stroke row picks a blend mode from the layer list',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addStroke(
        stroke: BridgeStroke(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Brush 1',
          points: const [
            BridgeStrokePoint(x: 10, y: 10, pressure: 1),
            BridgeStrokePoint(x: 40, y: 25, pressure: 1),
          ],
          colour: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
          width: 20,
          hardness: 0.8,
          shape: BridgeBrushShape.round,
          opacity: 100,
          start: const BridgeScalar.static_(0),
          end: const BridgeScalar.static_(100),
          mode: BridgePaintMode.paint,
          blend: 0,
          cloneOffsetX: 0,
          cloneOffsetY: 0,
        ),
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'paint', settle: true);

      final id = layer.getPaint().single.id;
      final picker = find.byKey(ValueKey<String>('tl-stroke-blend-$id'));
      expect(picker, findsOneWidget);
      await tester.tap(picker);
      await tester.pumpAndSettle();
      // The same words the layer's own picker offers, from the engine's table.
      await tester.tap(find.text('Multiply').last);
      await tester.pumpAndSettle();

      final modes = listBlendModes();
      expect(layer.getPaint().single.blend, modes.indexOf('Multiply'));
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
            BridgeStrokePoint(x: 10, y: 10, pressure: 1),
            BridgeStrokePoint(x: 40, y: 25, pressure: 1),
          ],
          colour: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
          width: 20,
          hardness: 0.8,
          shape: BridgeBrushShape.round,
          opacity: 100,
          start: const BridgeScalar.static_(0),
          end: const BridgeScalar.static_(100),
          mode: BridgePaintMode.paint,
          blend: 0,
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
            BridgeStrokePoint(x: 10, y: 10, pressure: 1),
            BridgeStrokePoint(x: 40, y: 25, pressure: 1),
          ],
          colour: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
          width: 20,
          hardness: 0.8,
          shape: BridgeBrushShape.round,
          opacity: 100,
          start: const BridgeScalar.static_(0),
          end: const BridgeScalar.static_(100),
          mode: BridgePaintMode.paint,
          blend: 0,
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
            trimStart: const BridgeScalar.static_(0),
            trimEnd: const BridgeScalar.static_(100),
            trimOffset: const BridgeScalar.static_(0),
            dashes: const [],
            dashOffset: const BridgeScalar.static_(0),
            gradient: 0,
            gradientColour: null,
            gradientStartX: const BridgeScalar.static_(0),
            gradientStartY: const BridgeScalar.static_(0),
            gradientEndX: const BridgeScalar.static_(0),
            gradientEndY: const BridgeScalar.static_(0),
            combine: 0,
            pathKeys: const [],
            offsetAmount: const BridgeScalar.static_(0),
            repeatCopies: const BridgeScalar.static_(1),
            repeatOffset: const BridgeScalar.static_(0),
            repeatAnchorX: const BridgeScalar.static_(0),
            repeatAnchorY: const BridgeScalar.static_(0),
            repeatPositionX: const BridgeScalar.static_(0),
            repeatPositionY: const BridgeScalar.static_(0),
            repeatRotation: const BridgeScalar.static_(0),
            repeatScale: const BridgeScalar.static_(100),
            repeatStartOpacity: const BridgeScalar.static_(100),
            repeatEndOpacity: const BridgeScalar.static_(100),
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

      // The trim's three rows sit under the item, and each writes through.
      expect(find.text('Trim start'), findsOneWidget);
      expect(find.text('Trim end'), findsOneWidget);
      expect(find.text('Trim offset'), findsOneWidget);
      final field = find.byKey(ValueKey<String>('tl-shape-trimEnd-${item.id}'));
      await tester.tap(field);
      await tester.pumpAndSettle();
      await tester.enterText(field, '40');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.trimEnd,
          const BridgeScalar.static_(40));

      // This item has no outline, so it has no dashes to set.
      expect(find.text('Dash'), findsNothing);

      // The offset applies before the trim, and reads as one length (K-554).
      expect(find.text('Offset path'), findsOneWidget);
      final offset =
          find.byKey(ValueKey<String>('tl-shape-offsetPath-${item.id}'));
      await tester.tap(offset);
      await tester.pumpAndSettle();
      await tester.enterText(offset, '-4');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.offsetAmount,
          const BridgeScalar.static_(-4));
    });

    /// The dashes belong to the outline: their rows appear under an item that
    /// has one, and writing either half makes the pair (K-552).
    testWidgets('a stroked shape item carries the dash rows', (tester) async {
      final p = withComp();
      BridgeVertex corner(double x, double y) => BridgeVertex(
          x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0);
      final layer = p.comp.addShapeLayer(
        name: 'Outline',
        contents: [
          BridgeShapeItem(
            id: UuidValue.fromString(const Uuid().v4()),
            name: 'Outline',
            vertices: [
              corner(0, 0),
              corner(60, 0),
              corner(60, 40),
              corner(0, 40),
            ],
            closed: true,
            fill: null,
            stroke: const BridgeColourRgba(r: 0, g: 1, b: 0, a: 1),
            strokeWidth: 3,
            opacity: 100,
            trimStart: const BridgeScalar.static_(0),
            trimEnd: const BridgeScalar.static_(100),
            trimOffset: const BridgeScalar.static_(0),
            dashes: const [],
            dashOffset: const BridgeScalar.static_(0),
            gradient: 0,
            gradientColour: null,
            gradientStartX: const BridgeScalar.static_(0),
            gradientStartY: const BridgeScalar.static_(0),
            gradientEndX: const BridgeScalar.static_(0),
            gradientEndY: const BridgeScalar.static_(0),
            combine: 0,
            pathKeys: const [],
            offsetAmount: const BridgeScalar.static_(0),
            repeatCopies: const BridgeScalar.static_(1),
            repeatOffset: const BridgeScalar.static_(0),
            repeatAnchorX: const BridgeScalar.static_(0),
            repeatAnchorY: const BridgeScalar.static_(0),
            repeatPositionX: const BridgeScalar.static_(0),
            repeatPositionY: const BridgeScalar.static_(0),
            repeatRotation: const BridgeScalar.static_(0),
            repeatScale: const BridgeScalar.static_(100),
            repeatStartOpacity: const BridgeScalar.static_(100),
            repeatEndOpacity: const BridgeScalar.static_(100),
          ),
        ],
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'contents', settle: true);

      expect(find.text('Dash'), findsOneWidget);
      expect(find.text('Gap'), findsOneWidget);
      expect(find.text('Dash offset'), findsOneWidget);

      final item = layer.getShapeContents().single;
      expect(item.dashes, isEmpty, reason: 'solid until it is dashed');
      final field = find.byKey(ValueKey<String>('tl-shape-dash-${item.id}'));
      await tester.tap(field);
      await tester.pumpAndSettle();
      await tester.enterText(field, '8');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.dashes,
          const [BridgeScalar.static_(8), BridgeScalar.static_(0)],
          reason: 'writing one half makes the pair');
    });

    /// A gradient fill is a choice, a second colour and two points, and none of
    /// them is on screen until there is a fill to ramp (K-555).
    testWidgets('a filled shape item carries the gradient rows',
        (tester) async {
      final p = withComp();
      BridgeVertex corner(double x, double y) => BridgeVertex(
          x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0);
      final layer = p.comp.addShapeLayer(
        name: 'Panel',
        contents: [
          BridgeShapeItem(
            id: UuidValue.fromString(const Uuid().v4()),
            name: 'Panel',
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
            trimStart: const BridgeScalar.static_(0),
            trimEnd: const BridgeScalar.static_(100),
            trimOffset: const BridgeScalar.static_(0),
            dashes: const [],
            dashOffset: const BridgeScalar.static_(0),
            gradient: 0,
            gradientColour: null,
            gradientStartX: const BridgeScalar.static_(0),
            gradientStartY: const BridgeScalar.static_(0),
            gradientEndX: const BridgeScalar.static_(0),
            gradientEndY: const BridgeScalar.static_(0),
            combine: 0,
            pathKeys: const [],
            offsetAmount: const BridgeScalar.static_(0),
            repeatCopies: const BridgeScalar.static_(1),
            repeatOffset: const BridgeScalar.static_(0),
            repeatAnchorX: const BridgeScalar.static_(0),
            repeatAnchorY: const BridgeScalar.static_(0),
            repeatPositionX: const BridgeScalar.static_(0),
            repeatPositionY: const BridgeScalar.static_(0),
            repeatRotation: const BridgeScalar.static_(0),
            repeatScale: const BridgeScalar.static_(100),
            repeatStartOpacity: const BridgeScalar.static_(100),
            repeatEndOpacity: const BridgeScalar.static_(100),
          ),
        ],
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'contents', settle: true);

      // Flat: the fill's colour and the choice, and nothing to aim.
      expect(find.text('Fill'), findsOneWidget);
      expect(find.text('Gradient'), findsOneWidget);
      expect(find.text('Gradient colour'), findsNothing);
      expect(find.text('Gradient start x'), findsNothing);

      final item = layer.getShapeContents().single;
      final dropdown =
          find.byKey(ValueKey<String>('tl-shape-gradient-${item.id}'));
      await tester.tap(dropdown);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Linear').last);
      await tester.pumpAndSettle();

      final ramped = layer.getShapeContents().single;
      expect(ramped.gradient, 1);
      // Switched on unaimed, it aims itself down the art's own box — a ramp
      // that read as one flat colour would look broken rather than unaimed.
      expect(ramped.gradientStartY, const BridgeScalar.static_(0));
      expect(ramped.gradientEndY, const BridgeScalar.static_(40));
      expect(find.text('Gradient colour'), findsOneWidget);
      expect(find.text('Gradient start x'), findsOneWidget);
    });

    /// A shape item's own path keys, so a drawn shape can morph into another
    /// one (K-606). The row is a stopwatch and its diamonds and nothing else:
    /// a shape has no single number to show.
    testWidgets('a shape item carries a Path row that keys its shape',
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
            trimStart: const BridgeScalar.static_(0),
            trimEnd: const BridgeScalar.static_(100),
            trimOffset: const BridgeScalar.static_(0),
            dashes: const [],
            dashOffset: const BridgeScalar.static_(0),
            gradient: 0,
            gradientColour: null,
            gradientStartX: const BridgeScalar.static_(0),
            gradientStartY: const BridgeScalar.static_(0),
            gradientEndX: const BridgeScalar.static_(0),
            gradientEndY: const BridgeScalar.static_(0),
            combine: 0,
            pathKeys: const [],
            offsetAmount: const BridgeScalar.static_(0),
            repeatCopies: const BridgeScalar.static_(1),
            repeatOffset: const BridgeScalar.static_(0),
            repeatAnchorX: const BridgeScalar.static_(0),
            repeatAnchorY: const BridgeScalar.static_(0),
            repeatPositionX: const BridgeScalar.static_(0),
            repeatPositionY: const BridgeScalar.static_(0),
            repeatRotation: const BridgeScalar.static_(0),
            repeatScale: const BridgeScalar.static_(100),
            repeatStartOpacity: const BridgeScalar.static_(100),
            repeatEndOpacity: const BridgeScalar.static_(100),
          ),
        ],
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'contents', settle: true);

      final id = layer.getShapeContents().single.id;
      final rowKey = 'tl-shape-path-$id';
      expect(find.text('Path'), findsOneWidget);
      expect(find.byKey(ValueKey<String>(rowKey)), findsNothing,
          reason: 'a shape has no single number, so the row has no field');

      expect(layer.getShapeContents().single.pathKeys, isEmpty);
      await tester.tap(find.byKey(ValueKey<String>('kf-stopwatch-$rowKey')));
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.pathKeys, hasLength(1),
          reason: 'the stopwatch planted a key on the shape');
      // The keys reach the lane, so the diamonds have somewhere to be drawn.
      expect(
          laneKeysOf(FoldShapeValueRow(
              layer.getShapeContents().single, ShapeValue.path,
              depth: 3)),
          hasLength(1));

      // And off again keeps the shape rather than dropping it.
      final before = layer.getShapeContents().single.vertices.length;
      await tester.tap(find.byKey(ValueKey<String>('kf-stopwatch-$rowKey')));
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.pathKeys, isEmpty);
      expect(layer.getShapeContents().single.vertices, hasLength(before));
    });

    /// The Combine row belongs to the item **above**: the first piece of art
    /// in the list has nothing in front of it to join, and an item that joins
    /// the run lends its path and nothing else (K-605).
    testWidgets('a shape item after the first carries the Combine row',
        (tester) async {
      final p = withComp();
      BridgeVertex corner(double x, double y) => BridgeVertex(
          x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0);
      BridgeShapeItem art(String name, double x, double y) => BridgeShapeItem(
            id: UuidValue.fromString(const Uuid().v4()),
            name: name,
            vertices: [
              corner(x, y),
              corner(x + 40, y),
              corner(x + 40, y + 40),
              corner(x, y + 40),
            ],
            closed: true,
            fill: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
            stroke: null,
            strokeWidth: 0,
            opacity: 100,
            trimStart: const BridgeScalar.static_(0),
            trimEnd: const BridgeScalar.static_(100),
            trimOffset: const BridgeScalar.static_(0),
            dashes: const [],
            dashOffset: const BridgeScalar.static_(0),
            gradient: 0,
            gradientColour: null,
            gradientStartX: const BridgeScalar.static_(0),
            gradientStartY: const BridgeScalar.static_(0),
            gradientEndX: const BridgeScalar.static_(0),
            gradientEndY: const BridgeScalar.static_(0),
            combine: 0,
            pathKeys: const [],
            offsetAmount: const BridgeScalar.static_(0),
            repeatCopies: const BridgeScalar.static_(1),
            repeatOffset: const BridgeScalar.static_(0),
            repeatAnchorX: const BridgeScalar.static_(0),
            repeatAnchorY: const BridgeScalar.static_(0),
            repeatPositionX: const BridgeScalar.static_(0),
            repeatPositionY: const BridgeScalar.static_(0),
            repeatRotation: const BridgeScalar.static_(0),
            repeatScale: const BridgeScalar.static_(100),
            repeatStartOpacity: const BridgeScalar.static_(100),
            repeatEndOpacity: const BridgeScalar.static_(100),
          );
      final layer = p.comp.addShapeLayer(
        name: 'Art',
        contents: [art('Base', 0, 0), art('Cutter', 20, 20)],
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'contents', settle: true);

      // One row, on the second item — the first has nothing above it to join.
      expect(find.text('Combine'), findsOneWidget);
      final second = layer.getShapeContents()[1];
      final dropdown =
          find.byKey(ValueKey<String>('tl-shape-combine-${second.id}'));
      await tester.tap(dropdown);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Subtract').last);
      await tester.pumpAndSettle();

      expect(layer.getShapeContents()[1].combine, 2);
      // Joined, the second item lends its path and nothing else: the run wears
      // the first item's paint and modifiers, so only one set of rows is left.
      expect(find.text('Trim start'), findsOneWidget);
      expect(find.text('Fill'), findsOneWidget);
    });

    /// The repeater's step is nine rows of nothing until there is more than
    /// one copy to step between, so Copies is the row that opens them (K-553).
    testWidgets('a repeated shape item carries the repeater rows',
        (tester) async {
      final p = withComp();
      BridgeVertex corner(double x, double y) => BridgeVertex(
          x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0);
      final layer = p.comp.addShapeLayer(
        name: 'Tick',
        contents: [
          BridgeShapeItem(
            id: UuidValue.fromString(const Uuid().v4()),
            name: 'Tick',
            vertices: [
              corner(0, 0),
              corner(10, 0),
              corner(10, 40),
              corner(0, 40),
            ],
            closed: true,
            fill: const BridgeColourRgba(r: 1, g: 0, b: 0, a: 1),
            stroke: null,
            strokeWidth: 0,
            opacity: 100,
            trimStart: const BridgeScalar.static_(0),
            trimEnd: const BridgeScalar.static_(100),
            trimOffset: const BridgeScalar.static_(0),
            dashes: const [],
            dashOffset: const BridgeScalar.static_(0),
            gradient: 0,
            gradientColour: null,
            gradientStartX: const BridgeScalar.static_(0),
            gradientStartY: const BridgeScalar.static_(0),
            gradientEndX: const BridgeScalar.static_(0),
            gradientEndY: const BridgeScalar.static_(0),
            combine: 0,
            pathKeys: const [],
            offsetAmount: const BridgeScalar.static_(0),
            repeatCopies: const BridgeScalar.static_(1),
            repeatOffset: const BridgeScalar.static_(0),
            repeatAnchorX: const BridgeScalar.static_(0),
            repeatAnchorY: const BridgeScalar.static_(0),
            repeatPositionX: const BridgeScalar.static_(0),
            repeatPositionY: const BridgeScalar.static_(0),
            repeatRotation: const BridgeScalar.static_(0),
            repeatScale: const BridgeScalar.static_(100),
            repeatStartOpacity: const BridgeScalar.static_(100),
            repeatEndOpacity: const BridgeScalar.static_(100),
          ),
        ],
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openFold(tester, layer.internallayerId,
          groupPath: 'contents', settle: true);

      // Drawn once: the count is there to find, and nothing else is.
      expect(find.text('Copies'), findsOneWidget);
      expect(find.text('Repeater rotation'), findsNothing);

      final item = layer.getShapeContents().single;
      final field =
          find.byKey(ValueKey<String>('tl-shape-repeatCopies-${item.id}'));
      await tester.tap(field);
      await tester.pumpAndSettle();
      await tester.enterText(field, '5');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(layer.getShapeContents().single.repeatCopies,
          const BridgeScalar.static_(5));

      // And now there is a step to describe.
      expect(find.text('Repeater rotation'), findsOneWidget);
      expect(find.text('Repeater position x'), findsOneWidget);
      expect(find.text('Start opacity'), findsOneWidget);
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
            trimStart: const BridgeScalar.static_(0),
            trimEnd: const BridgeScalar.static_(100),
            trimOffset: const BridgeScalar.static_(0),
            dashes: const [],
            dashOffset: const BridgeScalar.static_(0),
            gradient: 0,
            gradientColour: null,
            gradientStartX: const BridgeScalar.static_(0),
            gradientStartY: const BridgeScalar.static_(0),
            gradientEndX: const BridgeScalar.static_(0),
            gradientEndY: const BridgeScalar.static_(0),
            combine: 0,
            pathKeys: const [],
            offsetAmount: const BridgeScalar.static_(0),
            repeatCopies: const BridgeScalar.static_(1),
            repeatOffset: const BridgeScalar.static_(0),
            repeatAnchorX: const BridgeScalar.static_(0),
            repeatAnchorY: const BridgeScalar.static_(0),
            repeatPositionX: const BridgeScalar.static_(0),
            repeatPositionY: const BridgeScalar.static_(0),
            repeatRotation: const BridgeScalar.static_(0),
            repeatScale: const BridgeScalar.static_(100),
            repeatStartOpacity: const BridgeScalar.static_(100),
            repeatEndOpacity: const BridgeScalar.static_(100),
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

    /// **An outline switch is a bare glyph** (owner, 2026-08-24). Every
    /// toggle in a layer's row — the eye, solo, lock, shy, and every mode
    /// switch beside them — used to sit on a small outlined face, which the
    /// drawing puts on none of them; ten boxed marks at the head of a row
    /// turned two quiet columns into a grid of buttons.
    ///
    /// **And no accent, and no `animated`, in the Modes column** (the owner's
    /// longstanding ruling, and §3.1's closed accent list). On is
    /// `text_primary` and off is `text_muted` — the two strengths the drawing
    /// lights a row switch at; `animated` means "this is keyed", which a
    /// motion-blur switch is not.
    testWidgets('a row switch is a bare glyph, lit in the foreground',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      final t = LumitTheme.dark();
      final id = layer.internallayerId;

      for (final name in ['visible', 'solo', 'locked', 'shy', 'fx', 'mb']) {
        final cell = find.byKey(ValueKey<String>('tl-$name-$id'));
        expect(cell, findsOneWidget, reason: '$name is on the row');
        // No face: nothing inside the cell paints a fill or a border.
        for (final box in tester.widgetList<DecoratedBox>(
            find.descendant(of: cell, matching: find.byType(DecoratedBox)))) {
          final d = box.decoration as BoxDecoration;
          expect(d.color, isNull, reason: '$name wears no face');
          expect(d.border, isNull, reason: '$name wears no outline');
        }
      }

      // The Modes column's on-state, read off the glyph itself.
      ColorFilter? tintOf(String name) => tester
          .widget<SvgPicture>(find.descendant(
              of: find.byKey(ValueKey<String>('tl-$name-$id')),
              matching: find.byType(SvgPicture)))
          .colorFilter;

      expect(tintOf('fx'), ColorFilter.mode(t.textPrimary, BlendMode.srcIn),
          reason: 'effects are on, and an on switch is text_primary');
      expect(tintOf('fx'), isNot(ColorFilter.mode(t.accent, BlendMode.srcIn)),
          reason: 'never the accent — §3.1\'s list is closed');
      expect(tintOf('fx'), isNot(ColorFilter.mode(t.animated, BlendMode.srcIn)),
          reason: 'and never animated, which means "this is keyed"');
      expect(tintOf('mb'), ColorFilter.mode(t.textMuted, BlendMode.srcIn),
          reason: 'motion blur is off, and an off switch is text_muted');
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
      // K-459 — it was 8 here); the summary diamond is the **8** the drawing
      // renders (2026-08-24), a 4px square with a 1px border stood on its
      // corner, and it had been drawn at 5.
      expect(lane.half, laneKeyHalf, reason: 'a lane key is 11 across');
      expect(painter.half, 4, reason: 'a summary diamond is 8 across');
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
          isBeat: false,
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
          isBeat: false,
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

    /// **The live readout** (docs/impl/timeline-interaction.md §4.2): while a
    /// lane key is dragged a small pill rides beside it saying what frame it
    /// has reached and what it holds there — and it is gone the moment the
    /// pointer comes up, leaving nothing at rest (P1).
    testWidgets('a lane key drag carries a frame and value badge',
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

      final handle = find.byKey(ValueKey<String>(
          'tl-key-${layer.internallayerId}/transform/opacity#0'));
      final hint = find.byKey(const ValueKey('tl-key-drag-hint'));
      expect(hint, findsNothing, reason: 'nothing at rest');

      final gesture = await tester.startGesture(tester.getCenter(handle),
          kind: PointerDeviceKind.mouse);
      await gesture.moveBy(const Offset(3, 0));
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(4, 0));
        await tester.pump();
      }
      expect(hint, findsOneWidget,
          reason: 'the readout is under the hand while the drag runs');
      expect(
          tester
              .widget<Text>(
                  find.descendant(of: hint, matching: find.byType(Text)))
              .data,
          matches(RegExp(r'^f\d+ · 20$')),
          reason: 'the frame it has reached, and the value it holds');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(hint, findsNothing, reason: 'and leaves nothing behind');
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

    /// **The fold-out says *driven* too** (K-471, K-627). The Timeline builds
    /// the same parameter row Effect controls does, so a wire feeding a
    /// parameter has to reach it here as well: the mark stands where the
    /// stopwatch was, and the value field is deaf.
    testWidgets('a driven parameter marks the fold-out row', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      final effect = layer.getEffects().single.id();
      final made = layer.newDriver(name: 'wiggle');
      layer.setGraph(
        drivers: [made],
        wiring: BridgeGraphWiring(
          edges: [
            BridgeGraphEdge(
              from: BridgeOutputRef.driver(node: made.id(), port: 'value'),
              to: BridgeInputRef.param(
                  node: BridgeNodeRef.effect(effect), port: 'radius'),
            ),
          ],
          layout: const [],
          exposed: const [],
          groups: const [],
        ),
      );
      await mount(tester, p);

      await openFold(tester, layer.internallayerId, group: 'Effects');
      await tester.tap(find.text('Gaussian blur'));
      await tester.pump();

      final mark = find.byKey(ValueKey<String>('fx-driven-$effect-radius'));
      final field = find.byKey(ValueKey<String>('fx-float-$effect-radius'));
      expect(mark, findsOneWidget);
      expect(find.byKey(ValueKey<String>('kf-stopwatch-$effect-radius')),
          findsNothing,
          reason: 'the stopwatch has nothing to switch on here either');
      expect(tester.getTopLeft(mark).dx, lessThan(tester.getTopLeft(field).dx),
          reason: 'the mark is on the left of the lane, not in the value');
      expect(find.ancestor(of: field, matching: find.byType(IgnorePointer)),
          findsWidgets,
          reason: 'and the number it shows cannot be dragged');
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

    /// **The Modes column is a minimum, not a cage** (owner, desk test). An
    /// unlinked Scale draws two boxes, and half the Modes column each is
    /// narrower than a well holding `100.0%` — so the per-cent sign dropped to
    /// a second line inside a lane that is one line tall. The pair runs on to
    /// the right instead, into room the property row has going spare.
    testWidgets('an unlinked Scale keeps its per-cent sign on the line',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      // Unlinked, not separated: Scale ships linked and draws one box for
      // both axes (K-571); unlinking it puts x and y in the same row.
      layer.setAxisMode(
          pair: BridgeTransformPair.scale, mode: BridgeAxisMode.combined);
      await mount(tester, p);
      await openFold(tester, layer.internallayerId, group: 'Transform');

      final x = tester.getRect(find.byKey(const ValueKey('tl-tf-scaleX')));
      final y = tester.getRect(find.byKey(const ValueKey('tl-tf-scaleY')));
      expect(x.width, greaterThanOrEqualTo(transformCellWidth),
          reason: 'each box is the width it is drawn at in Effect controls — '
              'half the Modes column each was 53, and a reading does not fit');
      expect(y.width, greaterThanOrEqualTo(transformCellWidth));
      expect(y.right - x.left, greaterThan(renderGroupWidth),
          reason: 'so the pair is wider than the Modes column, and allowed '
              'to be: it runs on to the right rather than squeezing');

      // And it still lands on the panel rather than off its right edge.
      expect(
          y.right,
          lessThanOrEqualTo(
              tester.getRect(find.byType(TimelinePanelFrb)).right));

      // Each reading is one line, which is what the room was for.
      final readings = find.text('100.0%');
      expect(readings, findsNWidgets(2), reason: 'a box per axis');
      for (var i = 0; i < 2; i++) {
        expect(tester.getRect(readings.at(i)).height, lessThan(24),
            reason: 'one line of 11px mono, not the % wrapped under the '
                'digits inside a lane one line tall');
      }
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
    ///
    /// The label-colour column is the one exception, and it is a narrow one:
    /// see the test below it.
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

    /// **The label-colour column is headed by the set's Label glyph** (owner,
    /// 2026-08-24). The mockup's header row names six columns in words and
    /// leaves the dot column bare; the owner's ruling fills it with the glyph
    /// rather than a seventh word, because the column is 16 wide and every
    /// word for it is wider than that.
    ///
    /// What this pins is that it stands **over the dots**: the heading's cell
    /// and the row's swatch cell are the same 16, and the heading sits over
    /// its column by exactly the shift every other heading has. The header row
    /// is inset 10 where the layer rows are inset 8 (K-454), so *no* heading
    /// is centred on its column to the pixel — LAYER stands two right of the
    /// names too. Comparing the two shifts says the real thing without
    /// re-stating that 2, and still catches a heading that has come off its
    /// column, which is a change nobody would see in either half alone.
    testWidgets('the label column is headed by the Label glyph',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      final head = find.byKey(const ValueKey<String>('tl-colhead-label'));
      expect(head, findsOneWidget, reason: 'the dot column has a heading');

      // The set's own Label glyph, muted like every kicker beside it — not a
      // word, and not the accent.
      final icon = tester.widget<glyph.LumitIcon>(
          find.descendant(of: head, matching: find.byType(glyph.LumitIcon)));
      expect(icon.glyph, LumitIcons.label);
      expect(icon.colour, LumitTheme.dark().textMuted);

      // The heading's cell is the swatch's cell: 16 either side, so the glyph
      // centres on the dot rather than on some slice of the column.
      final swatch =
          find.byKey(ValueKey<String>('tl-label-${layer.internallayerId}'));
      expect(swatch, findsOneWidget);
      expect(tester.getRect(head).width, 16);
      expect(tester.getRect(swatch).width, 16);

      // And it stands over its column exactly as LAYER stands over the names.
      final shift = tester.getRect(head).left - tester.getRect(swatch).left;
      final nameShift = tester.getRect(find.text('LAYER').first).left -
          tester
              .getRect(find
                  .byKey(ValueKey<String>('tl-name-${layer.internallayerId}')))
              .left;
      expect(shift, closeTo(nameShift, 0.5),
          reason: 'the heading stands over the dots it names, by the same '
              'inset every other heading has');
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
        [g[1], g[2], g[3], g[0], g[4], g[5]],
        reason: 'dragged right, it lands after the target',
      );
      expect(
        reorderedGroups(defaultGroupOrder, g[3], g[0]),
        [g[3], g[0], g[1], g[2], g[4], g[5]],
        reason: 'dragged left, it lands before the target',
      );
      expect(reorderedGroups(defaultGroupOrder, g[1], g[1]), defaultGroupOrder);
    });

    /// **The hide ladders** (K-633). A switch column narrowed by a drag gives
    /// its cells up in a named order rather than clipping whichever happened
    /// to be last, and two of them are never in the ladder at all.
    test('a narrowed switch column sheds its cells in order', () {
      Set<SwitchCell> at(int cells) =>
          switchCellsFor(cells * switchCellWidth + 1);
      expect(at(6), SwitchCell.values.toSet(), reason: 'all six at full width');
      expect(at(5).contains(SwitchCell.guide), isFalse,
          reason: 'the grid mark goes first');
      expect(at(4).intersection({SwitchCell.guide, SwitchCell.shy}), isEmpty,
          reason: 'then shy');
      expect(at(3).contains(SwitchCell.locked), isFalse, reason: 'then lock');
      expect(at(2), {SwitchCell.visible, SwitchCell.audible},
          reason: 'then solo, and the eye and the speaker are what is left');
      expect(at(0), {SwitchCell.visible, SwitchCell.audible},
          reason: 'the ladder ends there however hard the seam is pulled');
      expect(switchCellsFor(400), SwitchCell.values.toSet(),
          reason: 'and a column dragged past its cells gains none');

      Set<ModeCell> modes(int cells) =>
          modeCellsFor(cells * switchCellWidth + 1);
      expect(modes(6), ModeCell.values.toSet());
      expect(modes(5).contains(ModeCell.flow), isFalse, reason: 'flow first');
      expect(modes(4).contains(ModeCell.adjustment), isFalse,
          reason: 'then adjustment');
      expect(modes(3), {ModeCell.fx, ModeCell.threeD, ModeCell.collapse},
          reason: 'then motion blur, leaving fx, 3D and collapse');
      expect(modes(0), modes(3),
          reason:
              'those three are the floor, and minGroupWidth is their width');
      expect(minGroupWidth(TimelineGroup.render), 3 * switchCellWidth);
      expect(minGroupWidth(TimelineGroup.switches), 2 * switchCellWidth);
    });

    /// **Seams snap** (K-633): a switch column to whole cells, everything else
    /// back to the width it shipped at when the drag passes near it.
    test('snapGroupWidth settles a dragged seam', () {
      expect(snapGroupWidth(TimelineGroup.switches, 4 * switchCellWidth + 4),
          4 * switchCellWidth,
          reason: 'rounded down to whole cells');
      expect(snapGroupWidth(TimelineGroup.switches, 4 * switchCellWidth - 4),
          4 * switchCellWidth,
          reason: 'and up');
      expect(snapGroupWidth(TimelineGroup.switches, 1000), switchesGroupWidth,
          reason: 'a switch column stops at its own cells: no blank space');
      expect(snapGroupWidth(TimelineGroup.render, 0),
          minGroupWidth(TimelineGroup.render));

      final home = defaultGroupWidths[TimelineGroup.identity]!;
      expect(snapGroupWidth(TimelineGroup.identity, home + snapGrab - 1), home,
          reason: 'near its shipped width, the seam takes it');
      expect(snapGroupWidth(TimelineGroup.identity, home + 40), home + 40,
          reason:
              'away from it, the width you dragged to is the width you get');
      expect(snapGroupWidth(TimelineGroup.identity, 10),
          minGroupWidth(TimelineGroup.identity),
          reason: 'and never below what the column needs');
    });

    /// The value column sits under the render group: everything right of it
    /// in the order contributes its fixed width to the inset.
    test('valueColumnFor measures what sits right of the render group', () {
      expect(
          valueColumnFor(defaultGroupOrder, defaultGroupWidths).rightInset,
          groupDividerWidth +
              composeGroupWidth +
              groupDividerWidth +
              parentGroupWidth +
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
          background: settings.background,
          shutterAngle: settings.shutterAngle,
          motionBlurSamples: settings.motionBlurSamples,
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
      // The lane ground's wash, which must follow the hand even though the
      // document does not — it reads the panel's one span, so it stands for
      // the graph highlight and the snap targets too (owner, 2026-08-25:
      // the highlight sat still until the release).
      double laneWashEnd() => tester
          .widgetList<CustomPaint>(find.byType(CustomPaint))
          .map((w) => w.painter)
          .whereType<WorkAreaGroundPainter>()
          .first
          .endX!;
      final washBefore = laneWashEnd();

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
      expect(laneWashEnd(), lessThan(washBefore),
          reason: 'mid-drag, the lane highlight is already at the staged span');

      await gesture.up();
      await tester.pumpAndSettle();
      final after = workAreaFrames(p.comp);
      expect(after.end, lessThan(before.end),
          reason: 'the release is the one write');

      p.state.project!.undo();
      expect(workAreaFrames(p.comp), equals(before),
          reason: 'one undo returns to the span before the drag');
    });

    /// **Grabbing an edge is not scrubbing** (owner, desk test). The ruler's
    /// own tap recognizer joins the arena from the same press as the handle's
    /// drag and fires on the press deadline whether or not it goes on to win,
    /// so taking hold of a work-area edge dragged the playhead to the pointer
    /// before the resize had even begun.
    testWidgets('grabbing a work-area edge leaves the playhead alone',
        (tester) async {
      final p = withComp();
      p.comp.setWorkArea(
          span: workAreaWith(
              comp: p.comp,
              current: null,
              wanted: p.comp.durationFrames() ~/ 2,
              isStart: false));
      await mount(tester, p);
      p.uiState.playheadFrame.value = 3;
      await tester.pump();

      final gesture = await tester.startGesture(
          tester.getCenter(find.byKey(const ValueKey('tl-work-end'))));
      // Past the press deadline, which is when the playhead used to jump.
      await tester.pump(const Duration(milliseconds: 300));
      expect(p.uiState.playheadFrame.value, 3,
          reason: 'the press belongs to the handle');

      // Crossed in steps, as a hand does.
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(-12, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, 3,
          reason: 'and so does the drag that followed it');
      expect(workAreaFrames(p.comp).end, lessThan(p.comp.durationFrames() ~/ 2),
          reason: 'the edge itself did move');
    });

    /// **The line is the pointer's, never the picture's** (P1's rule, applied
    /// to the playhead itself; the owner's "dragging the playhead over
    /// uncached areas is visually laggy").
    ///
    /// A scrub tells the engine where the playhead now is and paints the line
    /// there; what the engine does about it — a frame that may take half a
    /// second on ground nothing has rendered — is the picture's business and
    /// never the line's. Nothing here is mounted that asks for a frame, so no
    /// frame can arrive: **every pointer event must still move the line**, and
    /// move it on screen and not only in the notifier.
    ///
    /// The line's own drawn position is read, not only the frame the notifier
    /// holds: the playhead is painted through a transform behind its own layer
    /// (K-649), so what a test that read the notifier alone would prove is that
    /// a number changed.
    testWidgets(
        'a scrub moves the line on every pointer event, with no frame '
        'served', (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      p.uiState.playheadFrame.value = 0;
      await tester.pump();

      final served = p.uiState.frameArrived.value;
      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      double lineX() => tester.getTopLeft(find.byType(PlayheadMarker).first).dx;

      // A mouse, which is what a scrub is made with; near the ruler's left end
      // so ten steps stay inside the composition, but clear of the work area's
      // start handle, whose ten pixels are its own.
      final gesture = await tester.startGesture(
          Offset(ruler.left + 60, ruler.top + 4),
          kind: PointerDeviceKind.mouse);
      await tester.pump();
      final frames = <int>[];
      final drawn = <double>[];
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(8, 0));
        await tester.pump(const Duration(milliseconds: 16));
        frames.add(p.uiState.playheadFrame.value);
        drawn.add(lineX());
      }
      await gesture.up();
      await tester.pump();

      expect(p.uiState.frameArrived.value, served,
          reason: 'a frame arrived, so this measured nothing');
      expect(frames.first, greaterThan(0),
          reason: 'the first pointer event moved the pointer and not the line');
      for (var i = 1; i < frames.length; i++) {
        expect(frames[i], greaterThan(frames[i - 1]),
            reason: 'pointer event $i left the playhead where it was');
        expect(drawn[i], greaterThan(drawn[i - 1]),
            reason: 'pointer event $i left the line drawn where it was');
      }
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
    /// name, then fx · motion blur · 3D · adjustment, then matte · blend ·
    /// parent.
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
        'tl-guide-$id',
        'tl-twirl-$id',
        'tl-label-$id',
        'tl-name-$id',
        'tl-fx-$id',
        'tl-mb-$id',
        'tl-3d-$id',
        // A solid draws the adjustment cell (K-484); most kinds do not.
        'tl-adjust-$id',
        'tl-matte-$id',
        'tl-blend-$id',
        'tl-parent-$id',
      ];
      for (var i = 1; i < order.length; i++) {
        expect(dx(order[i]), greaterThan(dx(order[i - 1])),
            reason: '${order[i]} sits right of ${order[i - 1]}');
      }
    });

    /// **Flow and collapse have a cell each** (K-632). They shared one slot,
    /// which meant the same square was a frame-interpolation policy on footage
    /// and a rasterisation rule on a Precomp — and a Precomp made from retimed
    /// footage had nowhere to say both. Each is drawn only on the kind it acts
    /// on, and each stands at the end of the Modes column: flow, then collapse.
    testWidgets('flow and collapse stand in cells of their own',
        (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Inner');
      final precomp = p.comp.addPrecompLayer(comp: inner);
      final solid = p.comp.addSolidLayer();
      await mount(tester, p);
      final precompId = precomp.internallayerId;
      final solidId = solid.internallayerId;

      expect(find.byKey(ValueKey<String>('tl-collapse-$precompId')),
          findsOneWidget,
          reason: 'a Precomp collapses');
      expect(find.byKey(ValueKey<String>('tl-collapse-$solidId')), findsNothing,
          reason: 'a solid has no nested comp to collapse');
      expect(find.byKey(ValueKey<String>('tl-flow-$precompId')), findsNothing,
          reason: 'and it has no footage rate to interpolate, so no flow — but '
              'the cell it does not draw is no longer the cell collapse needs');

      // The L6 order, read across the Precomp's own row: fx leads, and the two
      // kind-gated cells end the column with flow left of collapse.
      double dx(String key) =>
          tester.getTopLeft(find.byKey(ValueKey<String>(key))).dx;
      for (final pair in [
        ('tl-fx-$precompId', 'tl-mb-$precompId'),
        ('tl-mb-$precompId', 'tl-3d-$precompId'),
        ('tl-3d-$precompId', 'tl-adjust-$precompId'),
        ('tl-adjust-$precompId', 'tl-collapse-$precompId'),
      ]) {
        expect(dx(pair.$2), greaterThan(dx(pair.$1)),
            reason: '${pair.$2} sits right of ${pair.$1}');
      }
      // Collapse is last, so it stands a whole cell right of where the flow
      // cell is drawn — the two are columns, not one square taking turns.
      expect(dx('tl-collapse-$precompId') - dx('tl-adjust-$precompId'),
          closeTo(2 * switchCellWidth, 0.5),
          reason: 'the blank flow cell keeps its place between them');
      expect(renderGroupWidth, 6 * switchCellWidth,
          reason: 'six cells: fx, motion blur, 3D, adjustment, flow, collapse');
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
          tester.getCenter(find.byKey(const ValueKey('tl-colgroup-parent')));
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
        'tl-guide-$id',
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

    /// **Accepts lights has no cell in the Modes column** (owner's ruling): the
    /// column is four switches, and the setting (K-361) is reached from the
    /// layer's own right-click menu, ticked when it is on.
    testWidgets('accepts lights left the Modes column for the row menu',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      expect(find.byKey(ValueKey<String>('tl-lit-$id')), findsNothing,
          reason: 'no accepts-lights cell in the Modes column');

      Future<void> openMenu() async {
        await tester.tapAt(
          tester.getCenter(find.byKey(ValueKey<String>('tl-row-$id'))),
          buttons: kSecondaryButton,
        );
        await tester.pumpAndSettle();
      }

      // On by default, so the entry opens ticked.
      expect(layer.getSwitches().acceptsLights, isTrue);
      await openMenu();
      final entry = find.byKey(const ValueKey('tl-row-accepts-lights'));
      expect(entry, findsOneWidget);
      expect(find.descendant(of: entry, matching: _tick), findsOneWidget,
          reason: 'the entry says which way the setting is set');
      await tester.tap(entry);
      await tester.pumpAndSettle();
      expect(layer.getSwitches().acceptsLights, isFalse,
          reason: 'and picking it writes the switch to the document');

      // Off now, so the tick is gone and picking it again puts it back.
      await openMenu();
      final again = find.byKey(const ValueKey('tl-row-accepts-lights'));
      expect(find.descendant(of: again, matching: _tick), findsNothing);
      await tester.tap(again);
      await tester.pumpAndSettle();
      expect(layer.getSwitches().acceptsLights, isTrue);
    });

    /// **Retime's own commands live on the layer's right-click** (docs/04
    /// §12.1): switching Retime on or off, Stretch, and a freeze at the
    /// playhead. A Sequence layer is offered none of them — its clips carry
    /// the maps (K-075) and are commanded from the sequence view.
    testWidgets('the row menu carries the Retime commands', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      await tester.tapAt(
        tester.getCenter(find.byKey(ValueKey<String>('tl-row-$id'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('tl-row-retime')), findsOneWidget);
      expect(find.byKey(const ValueKey('tl-row-stretch')), findsOneWidget);
      expect(find.byKey(const ValueKey('tl-row-freeze')), findsOneWidget);
      expect(find.text('Enable Retime'), findsOneWidget,
          reason: 'the command names what it will do');
    });

    /// **Stretch asks once and writes both halves** (docs/04 §11.2): half speed
    /// is twice as long, the in point is the anchor, and the map comes with it.
    testWidgets('Stretch asks for a speed and lengthens the layer',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;
      final before = layer.getInfo();

      await tester.tapAt(
        tester.getCenter(find.byKey(ValueKey<String>('tl-row-$id'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tl-row-stretch')));
      await tester.pumpAndSettle();

      // The two wells are one number seen twice: asking for half speed puts
      // twice the frames in the duration well before anything is committed.
      expect(find.byKey(const ValueKey('stretch-duration')), findsOneWidget);
      tester
          .widget<DragValueField>(find.byKey(const ValueKey('stretch-speed')))
          .onChanged(50);
      await tester.pumpAndSettle();
      expect(
          tester
              .widget<DragValueField>(
                  find.byKey(const ValueKey('stretch-duration')))
              .value,
          (before.outFrame - before.inFrame) * 2,
          reason: 'the duration well follows the speed well');

      await tester.tap(find.byKey(const ValueKey('stretch-confirm')));
      await tester.pumpAndSettle();

      final after = layer.getInfo();
      expect(after.inFrame, before.inFrame, reason: 'anchored at the in point');
      expect(after.outFrame - after.inFrame,
          (before.outFrame - before.inFrame) * 2);
      expect(layer.getRetimeProperty(), isNotNull,
          reason: 'the stretch is the map, not a hidden multiplier');
    });

    /// **Freeze at the playhead holds a second and leaves the length alone**
    /// (docs/04 §7.3, K-022).
    testWidgets('the row menu freezes the frame at the playhead',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      final before = layer.getInfo();
      p.uiState.playheadFrame.value = (before.inFrame + before.outFrame) ~/ 2;
      await mount(tester, p);
      final id = layer.internallayerId;

      await tester.tapAt(
        tester.getCenter(find.byKey(ValueKey<String>('tl-row-$id'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tl-row-freeze')));
      await tester.pumpAndSettle();

      final after = layer.getInfo();
      expect(after.outFrame, before.outFrame, reason: 'the length never moved');
      expect(layer.getRetimeProperty(), isNotNull);
    });

    /// **The adjustment switch is the Modes column's fifth cell** (K-537), and
    /// it is drawn on **every row that shows something in the Viewer** —
    /// footage, solid, precomp, text, and a layer born an adjustment. Only the
    /// four kinds with no picture to set aside leave it empty. A hidden layer
    /// keeps it: what a layer *is* and whether it is being shown are two
    /// answers, and hiding one must not hide the other.
    testWidgets('the adjustment cell is drawn on every visual kind',
        (tester) async {
      final p = withComp();
      final solid = p.comp.addSolidLayer();
      final adjustment = p.comp.addAdjustmentLayer();
      final text = p.comp.addTextLayer();
      final camera = p.comp.addCameraLayer();
      final nul = p.comp.addNullLayer();
      final hidden = p.comp.addSolidLayer();
      hidden.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
      p.uiState.model.refresh();
      await mount(tester, p);

      Finder cell(LayerReference l) =>
          find.byKey(ValueKey<String>('tl-adjust-${l.internallayerId}'));

      expect(cell(solid), findsOneWidget, reason: 'a solid can become one');
      expect(cell(adjustment), findsOneWidget, reason: 'and so can this one');
      expect(cell(text), findsOneWidget,
          reason: 'text draws, so text takes it');
      expect(cell(hidden), findsOneWidget,
          reason: 'a hidden layer is still a layer that draws');
      expect(cell(camera), findsNothing, reason: 'a camera shows nothing');
      expect(cell(nul), findsNothing, reason: 'nor does a null');
    });

    /// The cell lights the way the rest of the column does — `text_primary`
    /// when the layer is acting as an adjustment, `text_muted` when it is not —
    /// and it writes both ways, one click each. On a **footage** layer, which
    /// is the case the K-484 kind flip could not express at all.
    testWidgets('the adjustment cell lights by state and writes both ways',
        (tester) async {
      final p = withComp();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = p.comp.getLayers()[0];
      p.uiState.model.refresh();
      await mount(tester, p);
      final t = LumitTheme.dark();
      final id = layer.internallayerId;
      final cell = find.byKey(ValueKey<String>('tl-adjust-$id'));

      ColorFilter? tint() => tester
          .widget<SvgPicture>(
              find.descendant(of: cell, matching: find.byType(SvgPicture)))
          .colorFilter;

      expect(layer.getSwitches().adjustment, isFalse);
      expect(tint(), ColorFilter.mode(t.textMuted, BlendMode.srcIn),
          reason: 'an ordinary layer rests at text_muted');

      await tester.tap(cell);
      await tester.pumpAndSettle();
      expect(layer.getSwitches().adjustment, isTrue,
          reason: 'the click reached the document');
      expect(layer.getKind(), BridgeLayerKind.footage,
          reason: 'and it is still the shot — a switch, not a conversion');
      expect(tint(), ColorFilter.mode(t.textPrimary, BlendMode.srcIn),
          reason: 'and it lights at text_primary');

      // And back: the same cell writes the other direction, and the layer is
      // itself again with its source where it was.
      await tester.tap(cell);
      await tester.pumpAndSettle();
      expect(layer.getSwitches().adjustment, isFalse);
      expect(layer.getSourceItem(), isNotNull);
      expect(tint(), ColorFilter.mode(t.textMuted, BlendMode.srcIn));
    });

    /// **The switch applies to the whole selection** (K-523): it is an ordinary
    /// `_switch` cell, so it joins the choke point the other five already pass
    /// through rather than writing the clicked row alone.
    testWidgets('the adjustment cell applies to every selected layer',
        (tester) async {
      final p = withComp();
      final a = p.comp.addSolidLayer();
      final b = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      p.uiState.setSelection([a, b]);
      await mount(tester, p);

      await tester
          .tap(find.byKey(ValueKey<String>('tl-adjust-${a.internallayerId}')));
      await tester.pumpAndSettle();
      expect(a.getSwitches().adjustment, isTrue);
      expect(b.getSwitches().adjustment, isTrue,
          reason: 'the unclicked selected row went with it');
    });

    /// **The guide switch (K-497)**, the sixth cell in the A/V column. Unlike
    /// the two kind-gated cells in the Modes column it is drawn on every row —
    /// any layer can be reference-only — and it lights the way the rest of the
    /// column does, writing the document both ways.
    testWidgets('the guide switch is drawn on every kind and writes both ways',
        (tester) async {
      final p = withComp();
      final solid = p.comp.addSolidLayer();
      final text = p.comp.addTextLayer();
      final camera = p.comp.addCameraLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      final t = LumitTheme.dark();

      for (final layer in [solid, text, camera]) {
        expect(
            find.byKey(ValueKey<String>('tl-guide-${layer.internallayerId}')),
            findsOneWidget,
            reason: 'every kind can be reference-only');
      }

      final cell =
          find.byKey(ValueKey<String>('tl-guide-${solid.internallayerId}'));
      ColorFilter? tint() => tester
          .widget<SvgPicture>(
              find.descendant(of: cell, matching: find.byType(SvgPicture)))
          .colorFilter;

      expect(solid.getSwitches().guide, isFalse);
      expect(tint(), ColorFilter.mode(t.textMuted, BlendMode.srcIn),
          reason: 'off rests at text_muted');

      await tester.tap(cell);
      await tester.pumpAndSettle();
      expect(solid.getSwitches().guide, isTrue,
          reason: 'the click reached the document');
      expect(tint(), ColorFilter.mode(t.textPrimary, BlendMode.srcIn),
          reason: 'on lights at text_primary, and never the accent');

      await tester.tap(cell);
      await tester.pumpAndSettle();
      expect(solid.getSwitches().guide, isFalse);
    });

    /// The toolbar's readouts: the timecode counts frames at the comp's own
    /// rate and the frame count is zero-based, so frame 0 is 00:00:00:00.
    testWidgets('the timecode and frame readouts follow the playhead',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      expect(find.text('00:00:00:00'), findsOneWidget);
      expect(find.text('F0'), findsOneWidget);

      // 60 fps is the default comp rate: frame 90 is a second and a half in.
      p.uiState.playheadFrame.value = 90;
      await tester.pump();
      expect(find.text('00:00:01:30'), findsOneWidget);
      expect(find.text('F90'), findsOneWidget);
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
      expect(find.text('Backplate'), findsOneWidget,
          reason: 'marking a layer shy does not hide it yet — the name is in '
              'the outline, and on the bar only if the setting asks (K-514)');

      await tester.tap(find.byKey(const ValueKey('tl-hide-shy')));
      await tester.pump();
      expect(find.text('Backplate'), findsNothing);
      expect(find.text('Hero'), findsOneWidget);
      expect(shy.getSwitches().visible, isTrue,
          reason: 'shy hides the row, never the picture');

      await tester.tap(find.byKey(const ValueKey('tl-hide-shy')));
      await tester.pump();
      expect(find.text('Backplate'), findsOneWidget);
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
      // Numbered by place in the composition since item 6.13.
      await tester.tap(find.textContaining('Matte source').last);
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
    // The keyframe block and the bottom bar's strip (K-458), in Layers mode.
    //
    // Keys mode — the dope sheet these were written against — is gone
    // (K-529), and the strip came with them to the Layers bar. The machinery
    // never was the sheet's: the block box, its stretch handles, the Ease
    // popover and the seven commands all act on the lane key selection, which
    // is the same selection in every view.
    // ---------------------------------------------------------------------

    /// Twirl a layer open onto its Transform group, which is how a keyed
    /// property's lane is reached (K-529: it used to be a tap on the Keys
    /// tab, and the sheet opened every layer for you).
    Future<void> openKeyLane(WidgetTester tester, LayerReference layer) async {
      await openFold(tester, layer.internallayerId, group: 'Transform');
      await tester.pumpAndSettle();
    }

    /// **Two mode tabs, not three** (K-529): Keys is gone, and the tab that
    /// opened it with it.
    testWidgets('the mode tabs read Layers and Graph, in that order',
        (tester) async {
      final p = withComp();
      await mount(tester, p);

      expect(find.byKey(const ValueKey('tl-view-keys')), findsNothing);
      expect(find.text('KEYS'), findsNothing);
      final layers =
          tester.getRect(find.byKey(const ValueKey('tl-view-lanes')));
      final graph = tester.getRect(find.byKey(const ValueKey('tl-graph')));
      expect(layers.right, lessThanOrEqualTo(graph.left));
      expect(graph.center.dy, moreOrLessEquals(layers.center.dy, epsilon: 2));

      // The sheet's own surfaces went with the mode, not dormant behind it.
      expect(find.byKey(const ValueKey('tl-keys-filters')), findsNothing);
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
      await openKeyLane(tester, layer);

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
    // ---------------------------------------------------------------------
    // The block tools (K-458): the selection box with its stretch handles and
    // badge, the Ease popover, and the Keys bottom bar's strip.
    //
    // All of it lives in the machinery both modes share, so the claims below
    // are made in Keys mode — where the drawing puts them — and one of them is
    // made again in Layers mode, which is the claim that they are *shared*
    // rather than copied.
    // ---------------------------------------------------------------------

    /// A solid with [frames] keyed on Opacity, each key's value its own frame
    /// number — so a test can tell whether a value travelled with its key.
    LayerReference blockLayer(dynamic p, List<int> frames) {
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

    /// Box the whole of one lane row, which is how a block is made: the
    /// marquee sits behind the keys, so a drag that starts on empty lane
    /// gathers everything it encloses (docs/07 §4.3).
    Future<void> boxRow(WidgetTester tester, Key laneKey) async {
      final rect = tester.getRect(find.byKey(laneKey));
      final gesture =
          await tester.startGesture(Offset(rect.left + 1, rect.top + 1));
      await tester.pump(const Duration(milliseconds: 100));
      await gesture.moveTo(Offset(rect.left + 6, rect.top + 4));
      await tester.pump();
      await gesture.moveTo(Offset(rect.right - 1, rect.bottom - 1));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
    }

    /// Press one of the Keys bottom bar's words.
    ///
    /// Scrolled into view first: the strip scrolls sideways when the panel is
    /// narrow — the same answer the toolbar gives, an overflow stripe being a
    /// layout fault — so in a test-sized window its right-hand end starts off
    /// the edge of the bar.
    Future<void> tapStrip(WidgetTester tester, String key) async {
      final button = find.byKey(ValueKey<String>(key));
      await tester.ensureVisible(button);
      await tester.pumpAndSettle();
      await tester.tap(button);
      await tester.pumpAndSettle();
    }

    /// The frames a layer's Opacity keys read, in order.
    List<int> framesOf(dynamic p, LayerReference layer) => [
          for (final k
              in (layer.getTransform().opacity as BridgeScalar_Keyframed)
                  .field0)
            (p.comp as CompositionReference).frameAtTime(time: k.time)
        ];

    List<double> valuesOf(LayerReference layer) => [
          for (final k
              in (layer.getTransform().opacity as BridgeScalar_Keyframed)
                  .field0)
            k.value
        ];

    /// Pixels per frame on the lane axis, for turning a wanted frame move into
    /// a drag.
    double perFrameOf(dynamic p, WidgetTester tester, Key laneKey) =>
        (tester.getRect(find.byKey(laneKey)).width - TimelineAxis.pad * 2) /
        (p.comp as CompositionReference).durationFrames();

    /// The box, its two handles and its badge, at the drawing's own
    /// measurements: a 14px box inside a 22px lane, 3×6 handle marks, and the
    /// badge counting the keys and the frames it spans.
    testWidgets('a block of selected keys draws the drawing\'s box and badge',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 1500]);
      await mount(tester, p);
      await openKeyLane(tester, layer);

      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      expect(find.byKey(const ValueKey('tl-block-box')), findsNothing,
          reason: 'nothing selected, nothing to box');

      await boxRow(tester, laneKey);

      expect(find.byKey(const ValueKey('tl-block-box')), findsOneWidget);
      expect(
          find.byKey(const ValueKey('tl-block-handle-start')), findsOneWidget);
      expect(find.byKey(const ValueKey('tl-block-handle-end')), findsOneWidget);

      // The badge says what the block holds: two keys, and the 900 frames
      // between 600 and 1500.
      expect(find.text('2 keys · 900 f'), findsOneWidget,
          reason: 'the drawing\'s `n keys · n f`');

      const d = DensityTokens.regular;
      final box = tester.getRect(find.byKey(const ValueKey('tl-block-box')));
      expect(box.height, closeTo(d.laneRow - 8, 0.5),
          reason: 'the drawing\'s 14 inside a 22px lane — 4 above and below');
      final lane = tester.getRect(find.byKey(laneKey));
      expect(box.top, closeTo(lane.top + 4, 0.5));

      // The box reaches from the first key to the last.
      final perFrame = perFrameOf(p, tester, laneKey);
      expect(box.width, closeTo(perFrame * 900, 1.5),
          reason: 'the box spans the block, not the row');

      // The handle marks themselves are the drawing's 3 × 6, inside a hit
      // target wide enough to aim at (K-452).
      final mark = tester.getRect(find.descendant(
        of: find.byKey(const ValueKey('tl-block-handle-start')),
        matching: find.byType(ColoredBox),
      ));
      expect(mark.width, closeTo(3, 0.01));
      expect(mark.height, closeTo(6, 0.01));
      expect(
          tester
              .getRect(find.byKey(const ValueKey('tl-block-handle-start')))
              .width,
          closeTo(11, 0.01),
          reason: 'a 3px mark is a thing to see, not a thing to aim at');
    });

    /// The block box and the Ease popover at the **manifest's** own numbers —
    /// the computed styles taken off the approved Keys drawing, which is what
    /// stops the two drifting apart one careless pixel at a time.
    testWidgets(
        'the block box and the Ease popover match the drawing\'s styles',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 1500]);
      await mount(tester, p);
      await openKeyLane(tester, layer);
      await boxRow(
          tester,
          ValueKey<String>(
              'tl-keys-${layer.internallayerId}/transform/opacity'));
      final t = LumitTheme.dark();

      // The box: a hairline in text_primary, and nothing else.
      final boxDecoration = tester
          .widget<DecoratedBox>(find.descendant(
            of: find.byKey(const ValueKey('tl-block-box')),
            matching: find.byType(DecoratedBox),
          ))
          .decoration as BoxDecoration;
      expect(boxDecoration.border!.top.color, t.textPrimary,
          reason: 'the drawing\'s eef1f2 edge');
      expect(boxDecoration.border!.top.width, 1);
      expect(boxDecoration.color, isNull,
          reason: 'the box is an outline, not a wash — the keys show through');

      // The handle marks are the same foreground, filled.
      expect(
          tester
              .widget<ColoredBox>(find.descendant(
                of: find.byKey(const ValueKey('tl-block-handle-end')),
                matching: find.byType(ColoredBox),
              ))
              .color,
          t.textPrimary);

      // The badge: 8px mono in text_primary on surface_4, 1/4 padding, 2 round.
      final badgeText = tester.widget<Text>(find.text('2 keys · 900 f')).style!;
      expect(badgeText.fontSize, 8, reason: 'the manifest\'s 8px');
      expect(badgeText.color, t.textPrimary);
      expect(badgeText.fontFamily, t.mono.fontFamily,
          reason: 'mono — this is a count');
      final badgeBox = tester.widget<Container>(find.descendant(
        of: find.byKey(const ValueKey('tl-block-badge')),
        matching: find.byType(Container),
      ));
      expect((badgeBox.decoration as BoxDecoration).color, t.surface4,
          reason: 'the drawing\'s 2b3034');
      expect(badgeBox.padding,
          const EdgeInsets.symmetric(horizontal: 4, vertical: 1));

      // The popover: the drawing's 190 face, its own hairline, and a header
      // strip a lane row tall on surface_2.
      await tester.tap(find.byKey(const ValueKey('tl-block-badge')));
      await tester.pumpAndSettle();
      expect(easePopoverWidth, 190, reason: 'the drawing\'s own width');
      final popover = tester.getRect(find.byKey(const ValueKey('ease-apply')));
      expect(popover.width, greaterThan(0));
      final curve = tester.getRect(find.byKey(const ValueKey('ease-curve')));
      final influence =
          tester.getRect(find.byKey(const ValueKey('ease-influence-out')));
      final stagger =
          tester.getRect(find.byKey(const ValueKey('ease-stagger')));
      expect(curve.left, closeTo(influence.left, 0.5),
          reason: 'the three controls line up under one another');
      expect(curve.top, lessThan(influence.top));
      expect(influence.top, lessThan(stagger.top),
          reason: 'Curve, Influence, Stagger — the drawing\'s order');
    });

    /// One key is a key, not a block: it has its own drag, and a box round it
    /// would say "0 f".
    testWidgets('one selected key draws no block', (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 1500]);
      await mount(tester, p);
      await openKeyLane(tester, layer);

      await tester.tap(find.byKey(ValueKey<String>(
          'tl-key-${layer.internallayerId}/transform/opacity#0')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('tl-block-box')), findsNothing);
    });

    /// The gesture the whole box exists for: the anchored end stays put, the
    /// dragged end lands where it was put, and the key between keeps its share
    /// of the span — landed on whole frames, and undone in one step.
    testWidgets('dragging a handle stretches the block proportionally',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 900, 1500]);
      await mount(tester, p);
      await openKeyLane(tester, layer);

      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      await boxRow(tester, laneKey);
      expect(find.text('3 keys · 900 f'), findsOneWidget);

      // Drag the later end 450 frames further out: the span goes 900 → 1350,
      // a scale of 1.5 about the anchored first key.
      final perFrame = perFrameOf(p, tester, laneKey);
      await tester.drag(find.byKey(const ValueKey('tl-block-handle-end')),
          Offset(perFrame * 450, 0));
      await tester.pumpAndSettle();

      expect(framesOf(p, layer), [600, 1050, 1950],
          reason: '600 holds; 900 is a third along and stays a third along; '
              '1500 lands where it was dragged');
      expect(valuesOf(layer), [600, 900, 1500],
          reason: 'a stretch moves keys in time and nothing else');

      p.state.project!.undo();
      p.uiState.model.refresh();
      expect(framesOf(p, layer), [600, 900, 1500],
          reason: 'one gesture, one undo step — every row of it');
    });

    /// The other end, and the other direction: dragging the *earlier* handle
    /// anchors the later one, which is the half that is easy to get backwards.
    testWidgets('dragging the earlier handle anchors the later one',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 900, 1500]);
      await mount(tester, p);
      await openKeyLane(tester, layer);

      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      await boxRow(tester, laneKey);
      final perFrame = perFrameOf(p, tester, laneKey);
      // 600 → 1050: the span halves about 1500.
      await tester.drag(find.byKey(const ValueKey('tl-block-handle-start')),
          Offset(perFrame * 450, 0));
      await tester.pumpAndSettle();

      expect(framesOf(p, layer), [1050, 1200, 1500]);
    });

    /// The claim K-458 makes about *where* these tools live: the box is drawn
    /// by the lane area, which is one widget serving both modes, so Layers
    /// mode has it without a line of its own.
    testWidgets('Layers mode has the same block box and stretch',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 1500]);
      await mount(tester, p);
      // Layers mode — no Keys tab tapped — twirled open onto Transform.
      await openFold(tester, layer.internallayerId, group: 'Transform');

      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      await boxRow(tester, laneKey);

      expect(find.byKey(const ValueKey('tl-block-box')), findsOneWidget,
          reason: 'the block tools are the lanes\', not the sheet\'s');
      expect(find.text('2 keys · 900 f'), findsOneWidget);

      final perFrame = perFrameOf(p, tester, laneKey);
      await tester.drag(find.byKey(const ValueKey('tl-block-handle-end')),
          Offset(perFrame * 900, 0));
      await tester.pumpAndSettle();
      expect(framesOf(p, layer), [600, 2400],
          reason: 'and the stretch is the same stretch');
    });

    /// The Ease popover, opened where the drawing anchors it — on the block's
    /// own badge — and applied to every span the selection covers, in one step.
    testWidgets('the badge opens the Ease popover, which eases the block',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 1500]);
      await mount(tester, p);
      await openKeyLane(tester, layer);

      await boxRow(
          tester,
          ValueKey<String>(
              'tl-keys-${layer.internallayerId}/transform/opacity'));
      await tester.tap(find.byKey(const ValueKey('tl-block-badge')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('ease-apply')), findsOneWidget,
          reason: 'the popover is up');
      expect(find.byKey(const ValueKey('ease-count')), findsOneWidget);
      expect(find.text('2 keys'), findsWidgets,
          reason: 'it says back what it has hold of');
      // The drawing's four lines.
      expect(find.byKey(const ValueKey('ease-curve')), findsOneWidget);
      expect(find.byKey(const ValueKey('ease-influence-out')), findsOneWidget);
      expect(find.byKey(const ValueKey('ease-influence-in')), findsOneWidget);
      expect(find.byKey(const ValueKey('ease-stagger')), findsOneWidget);
      expect(find.byKey(const ValueKey('ease-stagger-order')), findsOneWidget);
      expect(find.byKey(const ValueKey('ease-open-graph')), findsOneWidget);

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;
      expect(keys().first.interpOut, const BridgeSideInterp.linear());

      await tester.tap(find.byKey(const ValueKey('ease-apply')));
      await tester.pumpAndSettle();

      expect(keys().first.interpOut, isA<BridgeSideInterp_Bezier>(),
          reason: 'the span between the two selected keys took the shape');
      expect(keys().last.interpIn, isA<BridgeSideInterp_Bezier>());
      expect(framesOf(p, layer), [600, 1500],
          reason: 'a shape is not a move: the keys stayed where they were');

      p.state.project!.undo();
      p.uiState.model.refresh();
      expect(keys().first.interpOut, const BridgeSideInterp.linear(),
          reason: 'one press, one undo step');
    });

    /// **The strip lives at the outline's foot**. It was drawn for Keys mode,
    /// came to the lane bar when that mode went (K-529), and moved again when
    /// the lane bar was pared back to the zoom and the scrollbar: the commands
    /// act on a key selection, which is made in the outline above them. It is
    /// there from the moment the panel is, because a selection is not a view.
    testWidgets('the outline\'s foot carries the keyframe strip',
        (tester) async {
      final p = withComp();
      blockLayer(p, [600, 1500]);
      await mount(tester, p);

      for (final key in const [
        'keys-interp-linear',
        'keys-interp-hold',
        'keys-interp-ease',
        'keys-interp-bezier',
        'keys-reverse',
        'keys-copy',
        'keys-paste',
      ]) {
        expect(find.byKey(ValueKey<String>(key)), findsOneWidget, reason: key);
      }
      // In the drawing's order, left to right.
      double x(String key) =>
          tester.getRect(find.byKey(ValueKey<String>(key))).left;
      expect(x('keys-interp-linear'), lessThan(x('keys-interp-hold')));
      expect(x('keys-interp-hold'), lessThan(x('keys-interp-ease')));
      expect(x('keys-interp-ease'), lessThan(x('keys-interp-bezier')));
      expect(x('keys-interp-bezier'), lessThan(x('keys-reverse')));
      expect(x('keys-reverse'), lessThan(x('keys-copy')));
      expect(x('keys-copy'), lessThan(x('keys-paste')));

      // And the whole run stands under the outline, not under the lanes: its
      // last button ends before the lane bar begins.
      expect(
          tester.getRect(find.byKey(const ValueKey('keys-paste'))).right,
          lessThanOrEqualTo(tester
              .getRect(find.byKey(const ValueKey('tl-lane-bottom-bar')))
              .left),
          reason: 'the commands sit at the outline\'s foot');
    });

    /// Interpolation, from the strip: the selected keys' two sides, set at a
    /// press, using K-457's vocabulary — and drawn with K-457's shapes.
    testWidgets(
        'the strip\'s Interpolation words set the selected keys\' sides',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 1500]);
      await mount(tester, p);
      await openKeyLane(tester, layer);
      await boxRow(
          tester,
          ValueKey<String>(
              'tl-keys-${layer.internallayerId}/transform/opacity'));

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

      await tapStrip(tester, 'keys-interp-hold');
      expect(keys().first.interpOut, const BridgeSideInterp.hold());
      expect(keyShapeOf(keys().first), (KeyShape.square, KeyShape.square));

      await tapStrip(tester, 'keys-interp-bezier');
      expect(keys().first.interpOut, isA<BridgeSideInterp_Bezier>());
      expect(
          keyShapeOf(keys().first), (KeyShape.hourglass, KeyShape.hourglass));

      await tapStrip(tester, 'keys-interp-linear');
      expect(keys().first.interpOut, const BridgeSideInterp.linear());
      expect(keyShapeOf(keys().first), (KeyShape.diamond, KeyShape.diamond));
    });

    /// Reverse: the block plays backwards where it stands. The times mirror
    /// through the middle of the selection, and **each value travels with its
    /// own key** — this re-times keys, it does not shuffle values under fixed
    /// times.
    testWidgets('Reverse mirrors the selected keys within their span',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 900, 1500]);
      await mount(tester, p);
      await openKeyLane(tester, layer);
      await boxRow(
          tester,
          ValueKey<String>(
              'tl-keys-${layer.internallayerId}/transform/opacity'));

      await tapStrip(tester, 'keys-reverse');

      expect(framesOf(p, layer), [600, 1200, 1500],
          reason: '900 reflects through the middle of 600…1500');
      expect(valuesOf(layer), [1500, 900, 600],
          reason: 'the values travelled with their keys, so the run reads '
              'back to front where it stands');

      p.state.project!.undo();
      p.uiState.model.refresh();
      expect(framesOf(p, layer), [600, 900, 1500]);
      expect(valuesOf(layer), [600, 900, 1500],
          reason: 'one press, one undo step');
    });

    /// Copy, then Paste at playhead: the block lands with its first key on the
    /// playhead, on the same property it came off.
    testWidgets('Copy and Paste at playhead put the block under the playhead',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 900]);
      await mount(tester, p);
      await openKeyLane(tester, layer);
      await boxRow(
          tester,
          ValueKey<String>(
              'tl-keys-${layer.internallayerId}/transform/opacity'));

      await tapStrip(tester, 'keys-copy');

      p.uiState.playheadFrame.value = 1800;
      await tapStrip(tester, 'keys-paste');

      final frames = framesOf(p, layer);
      expect(frames, contains(1800),
          reason: 'the block\'s first key landed on the playhead');
      expect(frames, contains(2100),
          reason: 'and the second kept its 300-frame gap');
      expect(frames, containsAll(<int>[600, 900]),
          reason: 'a paste adds; it does not move what was there');
    });

    // ---------------------------------------------------------------------
    // The owner's desktop-testing batch (K-529, K-530).
    // ---------------------------------------------------------------------

    /// **Each column toggle hides the columns its own word names.** The bug
    /// the owner found: pressing PARENT took the matte and the blend with it,
    /// because the three pickers were one draggable cluster and the toggle was
    /// mapped to the cluster rather than to the column it is named after.
    testWidgets('a column toggle hides its own columns and no others',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      final id = layer.internallayerId;

      Finder cell(String name) => find.byKey(ValueKey<String>('tl-$name-$id'));

      // What each toggle owns, and what it must leave alone.
      const scopes = <String, List<String>>{
        'switches': ['visible', 'solo', 'locked', 'shy', 'guide'],
        'render': ['fx', 'mb', '3d'],
        'parent': ['parent'],
      };
      // The matte and the blend answer to no toggle at all: the mockup's
      // bottom bar carries three words and these are not among them.
      const untoggleable = ['matte', 'blend'];

      for (final entry in scopes.entries) {
        for (final name in [...entry.value, ...untoggleable]) {
          expect(cell(name), findsOneWidget,
              reason: '$name is drawn before anything is hidden');
        }

        await tester
            .tap(find.byKey(ValueKey<String>('tl-column-${entry.key}')));
        await tester.pumpAndSettle();

        for (final name in entry.value) {
          expect(cell(name), findsNothing,
              reason: '${entry.key} hides $name, which it names');
        }
        for (final other in scopes.entries) {
          if (other.key == entry.key) continue;
          for (final name in other.value) {
            expect(cell(name), findsOneWidget,
                reason: '${entry.key} must not touch ${other.key}\'s $name');
          }
        }
        for (final name in untoggleable) {
          expect(cell(name), findsOneWidget,
              reason: '${entry.key} must not touch $name, which no toggle '
                  'owns');
        }

        await tester
            .tap(find.byKey(ValueKey<String>('tl-column-${entry.key}')));
        await tester.pumpAndSettle();
      }
    });

    /// **The three toggles are glyphs by default and words on request**
    /// (K-440's Chrome labels setting, consumed here for the first time —
    /// K-530). A tooltip carries the word in either mode, which is what makes
    /// the glyph readable at all.
    testWidgets('the column toggles are icons by default, words when asked',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      expect(p.uiState.workspace.interface.chromeLabels, ChromeLabels.icons,
          reason: 'the shipped default');
      for (final group in const ['switches', 'render', 'parent']) {
        final toggle = find.byKey(ValueKey<String>('tl-column-$group'));
        expect(
            find.descendant(of: toggle, matching: find.byType(glyph.LumitIcon)),
            findsOneWidget,
            reason: '$group draws the set\'s own glyph');
        expect(find.descendant(of: toggle, matching: find.byType(Text)),
            findsNothing);
      }

      p.uiState.workspace.interface.chromeLabels = ChromeLabels.words;
      p.uiState.workspace.settingsChanged();
      await tester.pumpAndSettle();

      for (final (group, word) in const [
        ('switches', 'SWITCHES'),
        ('render', 'MODES'),
        ('parent', 'PARENT'),
      ]) {
        final toggle = find.byKey(ValueKey<String>('tl-column-$group'));
        expect(find.descendant(of: toggle, matching: find.text(word)),
            findsOneWidget,
            reason: 'Words gives $group its word back');
        expect(
            find.descendant(of: toggle, matching: find.byType(glyph.LumitIcon)),
            findsNothing);
      }
    });

    /// **The floating tag rides the cursor** (K-529). Flutter's default anchors
    /// the feedback where the pointer sat inside the *child*, and the child is
    /// a header a couple of hundred pixels wide — so a header grabbed anywhere
    /// but its left edge drew its own name back at the header's x.
    testWidgets('a dragged column header\'s tag follows the pointer',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final header =
          tester.getRect(find.byKey(const ValueKey('tl-colgroup-identity')));
      // Grabbed well to the right of the header's left edge, which is the case
      // the old anchor got wrong.
      final from = Offset(header.right - 20, header.center.dy);
      final gesture = await tester.startGesture(from);
      await tester.pump(const Duration(milliseconds: 200));
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();

      final at = from + const Offset(30, 0);
      // The tag carries the column's word as written, where the header above
      // it is set in capitals — so this finds the tag and never the header.
      final tag = tester.getRect(find.text('Layer'));
      expect((tag.left - at.dx).abs(), lessThan(20),
          reason: 'the tag is under the hand, not back at the header\'s x');
      expect(tag.left, greaterThan(header.left + 40),
          reason: 'and nowhere near where the column starts');

      await gesture.up();
      await tester.pumpAndSettle();
    });

    /// **The lane bar is the zoom, the magnet and the scrollbar, in every
    /// view** (K-529 put the zoom first; the commands that used to follow it
    /// have gone to the outline's foot). Nothing that acts on a key stands
    /// under the lanes any more, in either view.
    testWidgets('the zoom and the magnet lead the bottom bar in both views',
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
      p.uiState.model.refresh();
      await mount(tester, p);

      double leadingEdge() =>
          tester.getRect(find.byKey(const ValueKey('tl-lane-bottom-bar'))).left;
      double zoomLeft() =>
          tester.getRect(find.byKey(const ValueKey('tl-zoom-slider'))).left;

      // Layers: the zoom leads, and the key commands are not on this bar.
      expect(zoomLeft() - leadingEdge(), lessThan(40));
      expect(
          tester
              .getRect(find.byKey(const ValueKey('keys-interp-linear')))
              .right,
          lessThanOrEqualTo(leadingEdge()));

      await tester.tap(find.byKey(const ValueKey('tl-graph')));
      await tester.pumpAndSettle();

      expect(zoomLeft() - leadingEdge(), lessThan(40),
          reason: 'the graph puts them in the same place');
      expect(
          tester
              .getRect(find.byKey(const ValueKey('graph-interp-linear')))
              .right,
          lessThanOrEqualTo(leadingEdge()),
          reason: 'the graph\'s own commands are at the outline\'s foot too');
    });

    /// **A seam drag moves the columns as it goes** (K-633, moving K-529's
    /// pin). Column widths are pure view state, so nothing reaches the document
    /// either way — the lag K-529 was answering was the *panel* rebuilding
    /// whole on every pointer move, and the live width now reaches the outline
    /// alone. The gesture still draws its own line, where the pointer is rather
    /// than where the snapped column has settled, and the travel lands once:
    /// releasing does not apply it a second time.
    testWidgets('a column seam moves the columns as it is dragged',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      final seam = find.byKey(const ValueKey('tl-seam-identity'));
      final before = tester.getRect(seam);

      final gesture = await tester.startGesture(before.center);
      await tester.pump(const Duration(milliseconds: 100));
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(10, 0));
        await tester.pump();
      }

      expect(find.byKey(const ValueKey('tl-seam-preview')), findsOneWidget,
          reason: 'the gesture draws where the pointer is');
      expect(tester.getRect(seam).left, closeTo(before.left + 60, 1),
          reason: 'and the column has already followed it');

      await gesture.up();
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('tl-seam-preview')), findsNothing);
      expect(tester.getRect(seam).left, closeTo(before.left + 60, 1),
          reason: 'the travel landed once, not twice');
    });

    /// **`Ctrl+C` then `Ctrl+V` round-trips a block of keys — in Layers.**
    /// The chord goes through the shell's own copy/paste, which hands it to
    /// whichever panel has claimed it (K-300).
    testWidgets('Ctrl+C and Ctrl+V round-trip keys in Layers mode',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 900]);
      await mount(tester, p);
      await openKeyLane(tester, layer);
      await boxRow(
          tester,
          ValueKey<String>(
              'tl-keys-${layer.internallayerId}/transform/opacity'));

      graphKeyClipboard = const [];
      expect(copySelectionFrb(p.uiState), isTrue,
          reason: 'the Timeline claims the chord and takes the keys');
      expect(graphKeyClipboard, hasLength(1));
      expect(graphKeyClipboard.single.keys, hasLength(2));

      p.uiState.playheadFrame.value = 1800;
      expect(
          await pasteSelectionFrb(p.state, p.uiState, p.comp, layer), isTrue);
      await tester.pumpAndSettle();

      expect(framesOf(p, layer), containsAll(<int>[600, 900, 1800, 2100]),
          reason: 'the block landed with its first key on the playhead, and '
              'the paste added rather than moved');
    });

    /// The same round trip in **Graph** mode, where the selection speaks in
    /// channel ids rather than in row paths.
    testWidgets('Ctrl+C and Ctrl+V round-trip keys in Graph mode',
        (tester) async {
      final p = withComp();
      final layer = blockLayer(p, [600, 900]);
      await mount(tester, p);
      await openKeyLane(tester, layer);
      await tester.tap(find.text('Opacity'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tl-graph')));
      await tester.pumpAndSettle();

      graphKeyClipboard = const [];
      expect(copySelectionFrb(p.uiState), isTrue);
      expect(graphKeyClipboard.single.keys, hasLength(2));

      p.uiState.playheadFrame.value = 1800;
      expect(
          await pasteSelectionFrb(p.state, p.uiState, p.comp, layer), isTrue);
      await tester.pumpAndSettle();

      expect(framesOf(p, layer), containsAll(<int>[600, 900, 1800, 2100]));
    });

    /// **A copy that took nothing says so** (K-529). It used to claim the
    /// chord anyway, which swallowed `Ctrl+C` and left the *previous* copy on
    /// the clipboard for the next paste to put down — the shape "Copy does
    /// nothing" took on the owner's desktop.
    testWidgets('a copy with no keys and no rows does not claim the chord',
        (tester) async {
      final p = withComp();
      blockLayer(p, [600, 900]);
      await mount(tester, p);

      graphKeyClipboard = const [];
      // Nothing picked at all: no keys, no property rows.
      expect(p.uiState.copyClaim?.call(), isFalse,
          reason: 'the chord falls through to the layer copy below it');
      expect(graphKeyClipboard, isEmpty);
    });

    // ---------------------------------------------------------------------
    // An action on a multi-selection applies to every selected layer (K-523).
    //
    // Every one of these was the same typo in a different cell: the row widget
    // holds a handle to *its* layer and calls the document with it, never
    // asking the shell what is picked. They route through `_menuTargets()`
    // now, which is the Project panel's `_targets` rule - the whole selection
    // when this row is in it, this row alone when it is not.
    // ---------------------------------------------------------------------

    /// Open a row's context menu.
    Future<void> openRowMenu(WidgetTester tester, LayerReference l) async {
      await tester.tapAt(
        tester.getCenter(
            find.byKey(ValueKey<String>('tl-row-${l.internallayerId}'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
    }

    testWidgets('the label swatch recolours every selected layer',
        (tester) async {
      final p = withComp();
      final upper = p.comp.addSolidLayer();
      final lower = p.comp.addSolidLayer();
      p.uiState.setSelection([upper, lower]);
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-label-${upper.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tl-label-chip-5')));
      await tester.pumpAndSettle();

      expect(upper.getInfo().label, 5);
      expect(lower.getInfo().label, 5,
          reason: 'the other picked layer took the colour too (K-523)');
    });

    testWidgets('a switch cell flips every selected layer', (tester) async {
      final p = withComp();
      final upper = p.comp.addSolidLayer();
      final lower = p.comp.addSolidLayer();
      p.uiState.setSelection([upper, lower]);
      await mount(tester, p);

      expect(upper.getSwitches().visible, isTrue);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-visible-${upper.internallayerId}')));
      await tester.pumpAndSettle();

      expect(upper.getSwitches().visible, isFalse);
      expect(lower.getSwitches().visible, isFalse,
          reason: 'the six switches share one choke point, so all six do this');

      // **One edit, not one per layer** (K-720 — the owner's Ctrl+A undo
      // walked back through fifty-three separate steps): a single undo
      // restores every layer at once.
      p.state.project!.undo();
      expect(upper.getSwitches().visible, isTrue);
      expect(lower.getSwitches().visible, isTrue,
          reason: 'one undo restored the whole selection (K-720)');
    });

    /// The batch keeps the loop's manners about locks: a locked *sibling*
    /// silently sits its share out, and the rest of the selection still flips.
    testWidgets('a locked sibling sits out of the switch batch',
        (tester) async {
      final p = withComp();
      final upper = p.comp.addSolidLayer();
      final lower = p.comp.addSolidLayer();
      lower.setSwitch(switch_: BridgeLayerSwitch.locked, on_: true);
      p.uiState.setSelection([upper, lower]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-visible-${upper.internallayerId}')));
      await tester.pumpAndSettle();

      expect(upper.getSwitches().visible, isFalse);
      expect(lower.getSwitches().visible, isTrue,
          reason: 'the locked sibling silently refused its share');
    });

    testWidgets('the row menu\'s Delete takes the whole selection',
        (tester) async {
      final p = withComp();
      final upper = p.comp.addSolidLayer();
      final lower = p.comp.addSolidLayer();
      final spare = p.comp.addSolidLayer();
      p.uiState.setSelection([upper, lower]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await openRowMenu(tester, upper);
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();

      final left = [
        for (final e in p.comp.getLayers()) e.internallayerId,
      ];
      expect(left, [spare.internallayerId],
          reason: 'both picked layers went, and the unpicked one stayed');
    });

    /// The other half of the rule, and the half that keeps a right-click
    /// honest: a menu opened on a row that is *not* picked is about that row.
    testWidgets('a row menu on an unpicked row acts on that row alone',
        (tester) async {
      final p = withComp();
      final picked = p.comp.addSolidLayer();
      final clicked = p.comp.addSolidLayer();
      p.uiState.setSelection([picked]);
      p.uiState.model.refresh();
      await mount(tester, p);

      await openRowMenu(tester, clicked);
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();

      final left = [
        for (final e in p.comp.getLayers()) e.internallayerId,
      ];
      expect(left, [picked.internallayerId]);
    });

    // ---------------------------------------------------------------------
    // Dragging one selected bar drags the whole selection (K-720): every
    // selected bar previews the travel live, and release commits ONE batched
    // slide — so one undo puts the whole set back.
    // ---------------------------------------------------------------------

    testWidgets('dragging a selected bar moves the whole selection as one step',
        (tester) async {
      final p = withComp();
      final upper = p.comp.addSolidLayer();
      final lower = p.comp.addSolidLayer();
      await mount(tester, p);
      p.uiState.setSelection([upper, lower]);
      await tester.pumpAndSettle();

      int inOf(LayerReference l) =>
          p.comp.frameAtTime(time: l.getSpan().inPoint);
      final upperIn = inOf(upper);
      final lowerIn = inOf(lower);

      final bar = find
          .byKey(ValueKey<String>('tl-bar-body-${upper.internallayerId}'));
      final rect = tester.getRect(bar);
      final mate = find
          .byKey(ValueKey<String>('tl-bar-body-${lower.internallayerId}'));
      final mateBefore = tester.getRect(mate);

      final gesture = await tester
          .startGesture(Offset(rect.left + rect.width / 2, rect.center.dy));
      await tester.pump();
      // In steps, as a hand moves — the arena needs real movement to give the
      // bar's recogniser the gesture before anything previews.
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(20, 0));
        await tester.pump();
      }
      // Mid-gesture, the selection-mate's bar travels live with the drag —
      // not on release, and not only the grabbed bar.
      expect(tester.getRect(mate).left, greaterThan(mateBefore.left + 60),
          reason: 'the mate\'s bar previews the same travel (K-720)');
      await gesture.up();
      await tester.pumpAndSettle();

      final moved = inOf(upper) - upperIn;
      expect(moved, greaterThan(0), reason: 'the drag moved the grabbed layer');
      expect(inOf(lower) - lowerIn, moved,
          reason: 'the mate moved by exactly the same frames');

      // One undo puts the whole selection back: the release was one batched
      // commit, never one write per layer.
      p.state.project!.undo();
      expect(inOf(upper), upperIn);
      expect(inOf(lower), lowerIn,
          reason: 'one undo restored the whole selection (K-720)');
    });

    /// The other half of the rule: grabbing a bar **outside** the selection
    /// acts on that bar alone, exactly as it always has — the press makes it
    /// the selection, and the drag carries only it.
    testWidgets('dragging an unselected bar moves it alone', (tester) async {
      final p = withComp();
      final upper = p.comp.addSolidLayer();
      final lower = p.comp.addSolidLayer();
      await mount(tester, p);
      p.uiState.setSelection([lower]);
      await tester.pumpAndSettle();

      int inOf(LayerReference l) =>
          p.comp.frameAtTime(time: l.getSpan().inPoint);
      final lowerIn = inOf(lower);

      final bar = find
          .byKey(ValueKey<String>('tl-bar-body-${upper.internallayerId}'));
      final rect = tester.getRect(bar);
      await tester.dragFrom(
          Offset(rect.left + rect.width / 2, rect.center.dy),
          const Offset(120, 0));
      await tester.pumpAndSettle();

      expect(inOf(upper), greaterThan(0), reason: 'the grabbed bar moved');
      expect(inOf(lower), lowerIn,
          reason: 'the previously selected layer sat still');
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
