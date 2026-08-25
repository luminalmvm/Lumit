// Every panel, squeezed to nothing, must still draw.
//
// **Why this file exists.** The owner reports that dragging a dock seam far
// enough to the left crashes the editor, and calls it "a common issue with
// panel adjustments" — so this is not one panel's bug to fix, it is a claim
// about all of them. K-451's degradation ladder ends at step 5: below a
// panel's declared minimum width the panel *scrolls horizontally* rather than
// compressing further. A panel that throws instead has skipped that step.
//
// [sweepWidths] pumps a panel across the whole range a seam drag can produce
// — 40 px up to a comfortable 400 — and fails on the first exception. It is
// deliberately blunt: no layout numbers are asserted here, because what a
// panel looks like at 40 px is the metrics tests' business. All this file
// claims is that nothing throws, which is the claim the crash reports are
// about.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effects_presets_panel_frb.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/panels/node_panel.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

/// The widths a seam drag walks through, plus the extremes either side of it.
///
/// Every step, not a sample: the reported crashes were at particular widths,
/// and a sweep that skips 15 px at a time is a sweep that misses them.
const List<double> sweepRange = [
  40,
  48,
  56,
  64,
  72,
  80,
  90,
  100,
  110,
  120,
  140,
  160,
  180,
  200,
  240,
  280,
  320,
  360,
  400,
];

/// Pump [build]'s panel at every width in [sweepRange] and fail on any
/// exception.
///
/// The panel is hosted exactly as the dock hosts it — a fixed-width box, the
/// full height, and the same [PanelFloor] every pane is wrapped in — because
/// the dock's seam is what produces these widths in the first place. Testing
/// it bare would be testing an arrangement that never ships.
///
/// **A fresh tree per width, deliberately.** A render object reports its
/// overflow once in its lifetime, so a sweep that re-pumps into the same
/// element tree hears about the first width and nothing after it — which is
/// how this whole family of bugs stayed quiet.
Future<void> sweepWidths(
  WidgetTester tester, {
  required Panel panel,
  required Widget Function() build,
  required LumitState state,
  required LumitUiState uiState,
  double height = 600,
  List<double> widths = sweepRange,
}) async {
  for (final width in widths) {
    await tester.pumpWidget(const SizedBox.shrink());
    tester.view.physicalSize = Size(width, height);
    tester.view.devicePixelRatio = 1.0;
    await tester.pumpWidget(hostPanel(
      child: SizedBox(
        width: width,
        height: height,
        child: PanelFloor(
          minWidth: panelMinWidth(panel),
          child: build(),
        ),
      ),
      state: state,
      uiState: uiState,
      size: Size(width, height),
    ));
    await tester.pump();
    final thrown = tester.takeException();
    expect(thrown, isNull,
        reason: 'the ${panel.name} panel threw at ${width}px: below its '
            'minimum it must scroll horizontally, not fail (K-451 step 5)');
  }
  addTearDown(tester.view.reset);
}

void main() {
  setUpAll(initEngineForTests);

  group('Panels survive every width (frb)', () {
    /// A project with something in every panel, and with the **widest** row
    /// the Project panel can be asked to draw: a clip that is both placed in a
    /// comp and missing from disk, so it wears two badges beside its name at
    /// the same time. A row wearing none of them fits anywhere and proves
    /// nothing.
    ({LumitState state, LumitUiState uiState}) populated() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      // Nothing is at this path, which is what makes the row say `missing`.
      final clip = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: clip, asSequence: false);
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState);
    }

    testWidgets('Project', (tester) async {
      final p = populated();
      await sweepWidths(tester,
          panel: Panel.project,
          build: () => const ProjectPanelFrb(),
          state: p.state,
          uiState: p.uiState);
    });

    testWidgets('Viewer', (tester) async {
      final p = populated();
      await sweepWidths(tester,
          panel: Panel.viewer,
          build: () => const ViewerPanelFrb(),
          state: p.state,
          uiState: p.uiState);
    });

    testWidgets('Effect controls', (tester) async {
      final p = populated();
      await sweepWidths(tester,
          panel: Panel.effectControls,
          build: () => const EffectControlsPanelFrb(),
          state: p.state,
          uiState: p.uiState);
    });

    testWidgets('Effects and presets', (tester) async {
      final p = populated();
      await sweepWidths(tester,
          panel: Panel.effectsAndPresets,
          build: () => const EffectsPresetsPanelFrb(),
          state: p.state,
          uiState: p.uiState);
    });

    testWidgets('Timeline', (tester) async {
      final p = populated();
      await sweepWidths(tester,
          panel: Panel.timeline,
          build: () => const TimelinePanelFrb(),
          state: p.state,
          uiState: p.uiState);
    });

    testWidgets('Graph', (tester) async {
      final p = populated();
      await sweepWidths(tester,
          panel: Panel.graph,
          build: () => const GraphPanelFrb(),
          state: p.state,
          uiState: p.uiState);
    });

    testWidgets('Node', (tester) async {
      final p = populated();
      await sweepWidths(tester,
          panel: Panel.node,
          build: () => const NodePanelFrb(),
          state: p.state,
          uiState: p.uiState);
    });
  }, skip: !engineAvailable);
}
