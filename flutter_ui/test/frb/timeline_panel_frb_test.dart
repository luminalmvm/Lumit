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
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/state/clipboard.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:uuid/uuid.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/panels/comp_graph_panel.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/panels/graph_editor_frb.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_navigator.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
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

    /// Dragging the window pans and nothing else. Each pointer move used to
    /// zoom out a little, so the window crept wider for as long as it was
    /// dragged.
    testWidgets('dragging the navigator window pans without zooming',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      double laneWidth() =>
          tester.widget<TimelineRuler>(find.byType(TimelineRuler)).axis.width;

      final box = tester.getRect(find.byKey(const ValueKey('tl-navigator')));
      // Zoom in to about a third of the comp first, so there is room to pan.
      await tester.dragFrom(
        Offset(box.right - TimelineNavigator.handleGrab / 2, box.center.dy),
        Offset(-box.width * 2 / 3, 0),
      );
      await tester.pumpAndSettle();
      final zoomed = laneWidth();

      // Take hold of the window's middle and drag it along in small steps,
      // the way a mouse reports a drag, then back.
      final gesture = await tester.startGesture(
          Offset(box.left + box.width / 6, box.center.dy),
          kind: PointerDeviceKind.mouse);
      for (var i = 0; i < 80; i++) {
        await gesture.moveBy(const Offset(3, 0));
        await tester.pump();
      }
      for (var i = 0; i < 80; i++) {
        await gesture.moveBy(const Offset(-3, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(laneWidth(), closeTo(zoomed, 0.01),
          reason: 'a pan is not a zoom, however many moves it is made of');
    });

    /// The Razor tool. Clicking a bar cuts that layer **where the
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

    /// The field a readout turns into when it is clicked.
    Finder fieldIn(String key) => find.descendant(
          of: find.byKey(ValueKey<String>(key)),
          matching: find.byType(EditableText),
        );

    /// The toolbar's two readouts are typed into, not merely read,
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

    /// The two modes are words, not glyphs, and the one in force is the one
    /// wearing the frame (§12A.1). Clicking the other switches; §3.1 keeps the
    /// accent off both.
    testWidgets('the mode tabs read Layers and Graph, and switch',
        (tester) async {
      final p = withComp();
      await mount(tester, p);

      expect(find.text('Layers'), findsOneWidget);
      expect(find.text('Graph'), findsOneWidget);

      // Layers is in force to begin with, so the graph editor is not up.
      expect(find.byType(GraphEditorFrb), findsNothing);
      await tester.tap(find.byKey(const ValueKey('tl-graph')));
      await tester.pumpAndSettle();
      expect(find.byType(GraphEditorFrb), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('tl-view-lanes')));
      await tester.pumpAndSettle();
      expect(find.byType(GraphEditorFrb), findsNothing);
    });

    /// **Cutting a retimed layer gives each half an end of its own.**
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
    /// layer has one — the same rule Effects follows.
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

    /// **A mask's opacity was not undoable.** Its field wrote on every
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

    /// **Every mask value keyframes, with the same stopwatch as everything
    /// else.** The branch that added mask animation exposed none of it
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

    /// **Delete removes the selected mask.** The shell's Delete deletes
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
      p.uiState.activePane.value = Panel.timeline.pane();
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

    /// **A feather per point is switched on from the mask's own menu, and
    /// gives every point a row.**
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
    /// the order the picture is built in.
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

    /// A shape layer lists its art under a Contents heading, above Masks and
    /// Effects — the order the picture is built in.
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

      // The offset applies before the trim, and reads as one length.
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

    /// Retime is an ordinary property row: hidden until the layer is
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

    /// **The bottom bar's zoom is a slider** (owner, 2026-08-06), between a
    /// small landscape glyph and a large one. Its left end is the whole
    /// composition; dragging right widens the time axis, and a slider zoom has
    /// no pointer to zoom about, so it holds the **playhead** still — the
    /// middle of the scrollbar, which it held first, is a place nobody is
    /// looking at.
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

    /// Frames from exact times, in integers: the panel maps a start
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
    /// Retime on takes the limit off. Fails without the clamp: the
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

    /// A trimmed source-backed layer shows where its media would reach — the
    /// faint outline behind the bar — and stops showing it once the bar
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
    /// holds: the playhead is painted through a transform behind its own
    /// layer, so what a test that read the notifier alone would prove is that
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

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
    /// The gesture the whole Project panel drag exists for. It had no drop
    /// **One drop is one undo step.** The layer used to go on at the top and
    /// then be walked down to the row it was dropped on, so Ctrl+Z put it back
    /// at the top before a second press took it away.
    testWidgets('a drop below the top takes one undo, not two', (tester) async {
      final p = withComp();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      for (var i = 0; i < 3; i++) {
        p.comp.addNullLayer();
      }
      final before = [
        for (final l in p.comp.getLayers()) l.internallayerId,
      ];
      p.uiState.model.refresh();

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

      // Let go over the second layer's row, which is not the top of the stack.
      final onto = find.byKey(ValueKey<String>('tl-row-${before[1]}'));
      expect(onto, findsOneWidget);
      final target = tester.getCenter(onto);
      final row =
          find.byKey(ValueKey<String>('project-row-${footage.internalid}'));
      final gesture = await tester.startGesture(tester.getCenter(row));
      await tester.pump(const Duration(milliseconds: 200));
      final from = tester.getCenter(row);
      for (var i = 1; i <= 10; i++) {
        await gesture.moveTo(Offset(
          from.dx + (target.dx - from.dx) * i / 10,
          from.dy + (target.dy - from.dy) * i / 10,
        ));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final after = p.comp.getLayers();
      expect(after, hasLength(4), reason: 'the drop reached the document');
      expect(after.first.internallayerId, before.first,
          reason: 'it landed where it was dropped, not at the top');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(
        [for (final l in p.comp.getLayers()) l.internallayerId],
        before,
        reason: 'one press put the whole drop back',
      );
    });

    /// **The Timeline on a node graph** (docs/impl/node-graph-comp.md §4.4):
    /// there are no layers to draw, so it draws the ruler and the canvas.
    testWidgets('a node graph shows the ruler and the canvas', (tester) async {
      final p = withComp();
      final graph = p.state.project!.newNodeGraph(name: 'Wires');
      p.uiState.setSelectedComp(graph);
      p.uiState.model.refresh();
      await mount(tester, p);

      expect(find.byType(TimelineRuler), findsOneWidget);
      expect(find.byType(CompGraphPanel), findsOneWidget);
      expect(
          tester.widget<CompGraphPanel>(find.byType(CompGraphPanel)).host,
          Panel.timeline);
      expect(find.byKey(const ValueKey('tl-navigator')), findsNothing,
          reason: 'nothing of the layer table is drawn');
      // The tabs stay: they are the way back out of a node graph, and the
      // Export button belongs to the comp rather than to the layer table.
      expect(find.byKey(ValueKey<String>('tl-tab-${graph.internalid}')),
          findsOneWidget);
      expect(find.byKey(const ValueKey('tl-export')), findsOneWidget);

      // The ruler spans the panel and scrubs the playhead.
      final box = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      expect(box.width, 1280);
      await tester.tapAt(Offset(box.left + box.width * 0.5, box.center.dy));
      await tester.pump();
      final frames = graph.durationFrames();
      expect(p.uiState.playheadFrame.value, closeTo(frames * 0.5, 2));
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

    /// A drop lands the layer where the row under it stands **in the stack**,
    /// not where it stands on screen. With a row hidden by the shy filter, the
    /// search box or the Sound mix fold, every slot below it counted short and
    /// the drop put the layer somewhere else.
    testWidgets('a drop counts the stack, not the rows on screen',
        (tester) async {
      final p = withComp();
      for (final name in ['Bottom', 'Middle', 'Top', 'Backplate']) {
        p.comp.addSolidLayer().rename(name: name);
      }
      p.uiState.model.refresh();
      await mount(tester, p);

      List<String> stack() => [for (final l in p.comp.getLayers()) l.getName()];
      expect(stack(), ['Backplate', 'Top', 'Middle', 'Bottom'],
          reason: 'newest on top, as added');

      // The top row goes shy and the filter takes it off the screen, so the
      // rows on show start one below the top of the stack. Set through the
      // engine: the Switches column opens with its shy cell put away.
      p.comp
          .getLayers()
          .first
          .setSwitch(switch_: BridgeLayerSwitch.shy, on_: true);
      p.uiState.model.refresh();
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('tl-hide-shy')));
      await tester.pump();
      expect(find.text('Backplate'), findsNothing);

      // The topmost row on show, dropped on the bottom one.
      final visible = p.comp.getLayers().sublist(1);
      final from = find.byKey(
          ValueKey<String>('tl-name-${visible.first.internallayerId}'));
      final onto = find
          .byKey(ValueKey<String>('tl-row-${visible.last.internallayerId}'));
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

      expect(stack(), ['Backplate', 'Middle', 'Bottom', 'Top'],
          reason: 'the drop landed the layer at the slot it had on screen, '
              'which is one place short of the stack with a row hidden');
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

    /// **Clicking an effect's heading picks it.** A heading only
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

    /// Delete on picked effect rows removes those effects and leaves the layer.
    testWidgets('Delete removes picked effects and leaves their layer',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      layer.addEffect(name: 'blur');
      layer.addEffect(name: 'blur');
      p.uiState.setSelection([layer]);
      await mount(tester, p);
      final [first, second, kept] = layer.getEffects();

      p.uiState.activePane.value = Panel.timeline.pane();
      p.uiState.setEffectSelection(layer, [first.id(), second.id()]);
      await tester.pump();
      await settleFrb(tester, minRounds: 4);

      expect(p.uiState.deleteClaim!(), isTrue,
          reason: 'with effects picked the Timeline takes Delete');
      await settleFrb(tester, minRounds: 4);

      expect([for (final e in layer.getEffects()) e.id()], [kept.id()],
          reason: 'both picked effects are gone, the other stays');
      expect(p.comp.getLayers(), hasLength(1),
          reason: 'and the layer is still there');
      expect(p.uiState.selectedEffects.value, isEmpty);
    });

    /// Enter turns the selected layer's name into an editor; submitting
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

      // Escape leaves it the other way: editor shut, nothing written.
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

    /// Double-clicking a Precomp layer opens the comp it draws — the
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

      // The waveform lane: behind its own twirl under Audio, and its
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
      // and treble.
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
      // zoom instead of stretching one fixed summary.
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

    /// **A retimed layer's wave stretches with its map.** The buckets
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

    // ---------------------------------------------------------------------
    // The keyframe block and the bottom bar's strip, in Layers mode.
    //
    // Keys mode — the dope sheet these were written against — is gone, and
    // the strip came with them to the Layers bar. The machinery
    // never was the sheet's: the block box, its stretch handles, the Ease
    // popover and the seven commands all act on the lane key selection, which
    // is the same selection in every view.
    // ---------------------------------------------------------------------

    /// Twirl a layer open onto its Transform group, which is how a keyed
    /// property's lane is reached (it used to be a tap on the Keys
    /// tab, and the sheet opened every layer for you).
    Future<void> openKeyLane(WidgetTester tester, LayerReference layer) async {
      await openFold(tester, layer.internallayerId, group: 'Transform');
      await tester.pumpAndSettle();
    }

    // ---------------------------------------------------------------------
    // The block tools: the selection box with its stretch handles and
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

    /// Interpolation, from the strip: the selected keys' two sides, set at a
    /// press, using the keys' own vocabulary — and drawn with their shapes.
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

    // Reverse's and the Copy/Paste buttons' widget tests went with the
    // buttons (owner, 2026-08-31): the copy/paste road that remains is
    // the chord, round-tripped below; Reverse's arithmetic keeps its pins in
    // key_block_test.dart.

    // ---------------------------------------------------------------------
    // The owner's desktop-testing batch.
    // ---------------------------------------------------------------------

    /// **`Ctrl+C` then `Ctrl+V` round-trips a block of keys — in Layers.**
    /// The chord goes through the shell's own copy/paste, which hands it to
    /// whichever panel has claimed it.
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

    // ---------------------------------------------------------------------
    // An action on a multi-selection applies to every selected layer.
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

      // **One edit, not one per layer** (the owner's Ctrl+A undo walked back
      // through fifty-three separate steps): a single undo restores every
      // layer at once.
      p.state.project!.undo();
      expect(upper.getSwitches().visible, isTrue);
      expect(lower.getSwitches().visible, isTrue,
          reason: 'one undo restored the whole selection');
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
    // Dragging one selected bar drags the whole selection: every
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
          reason: 'the mate\'s bar previews the same travel');
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
          reason: 'one undo restored the whole selection');
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

