// Dragging the Viewer's split, in the Nodes workspace, with the wireframes on.
//
// **The report.** Widening the Viewer's share of the split made the layer
// wireframe flicker over the *node graph* beside it; a moment later the editor
// froze and the process was gone. Two separate faults, and this file is about
// the first: a mark belonging to the Viewer painted outside the Viewer.
//
// The marks over the picture — wireframes, handles, mask outlines, every tool
// layer — are painters filling the stage's stack, and a painter may draw
// wherever it likes. A `Stack` does not stop it: the clip a stack carries is
// applied only when a *positioned child* is measured overflowing, which a
// `Positioned.fill` painter never is. So the only thing that ever kept those
// marks inside the panel was the rounded-tile wrapper the Round theme shape
// puts round the picture — and under Sharp there was nothing at all. That is a
// guarantee resting on a coincidence, which is what the first test here ends.
//
// The second test is the gesture itself. `panel_width_sweep_test` walks the
// same widths but builds a *fresh tree* for each one, on purpose, so that each
// width gets a render object that has not reported its overflow yet. A seam
// drag is the opposite: one tree, laid out again and again as the pointer
// moves, with frames arriving in between. Nothing covered that until now.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('The Viewer stage during a split drag (frb)', () {
    /// A comp with a layer selected, which is what puts a wireframe on the
    /// picture at all.
    ({LumitState state, LumitUiState uiState}) selected() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState);
    }

    Widget host(
      ({LumitState state, LumitUiState uiState}) p, {
      required double width,
      required ThemeShape shape,
    }) =>
        hostPanel(
          child: SizedBox(
            width: width,
            height: 600,
            child: const ViewerPanelFrb(),
          ),
          state: p.state,
          uiState: p.uiState,
          size: Size(width, 600),
          shape: shape,
        );

    /// **The regression.** Under Sharp there was no clip anywhere above the
    /// stage, so a wireframe wider than the panel landed on whatever sat next
    /// to the Viewer. The clip is the stage's own now, so both shapes have it.
    for (final shape in ThemeShape.values) {
      testWidgets('nothing the stage draws leaves it (${shape.name})',
          (tester) async {
        final p = selected();
        await tester.pumpWidget(host(p, width: 480, shape: shape));
        await tester.pump();
        expect(
          find.descendant(
            of: find.byType(ViewerStage),
            matching: find.byType(ClipRect),
          ),
          findsWidgets,
          reason: 'the stage must clip its own marks by construction, not by '
              'whichever wrapper the theme shape happens to put round it',
        );
      });
    }

    /// The drag itself: one tree, re-laid-out at every width the pointer
    /// passes through. Nothing may throw, and the clip must survive every one
    /// of them — a clip that is there at rest and gone mid-gesture is no clip.
    testWidgets('thrashing the split width in one tree stays sound',
        (tester) async {
      final p = selected();
      // Widening, as the report describes, then back — a seam drag rarely goes
      // one way only, and the way back is where the layout is asked for sizes
      // it has already seen.
      final widths = <double>[
        for (var w = 200.0; w <= 900.0; w += 17) w,
        for (var w = 900.0; w >= 200.0; w -= 23) w,
      ];
      for (final width in widths) {
        await tester.pumpWidget(
          host(p, width: width, shape: ThemeShape.sharp),
        );
        await tester.pump(const Duration(milliseconds: 16));
        expect(tester.takeException(), isNull,
            reason: 'the Viewer threw at ${width}px during a seam drag');
        expect(
          find.descendant(
            of: find.byType(ViewerStage),
            matching: find.byType(ClipRect),
          ),
          findsWidgets,
          reason: 'the stage lost its clip at ${width}px',
        );
      }
    });
  }, skip: !engineAvailable);
}
