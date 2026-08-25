// The panel dispatcher of the flutter_rust_bridge shell — what `LumitAppNew` in
// main.dart docks.
//
// This routes a Panel to its widget and nothing else; each ported panel lives in
// a file of its own. All of them are ported.
//
// The dispatcher for what remains on the v0 JSON bridge is panels.dart. Panels
// move across as the frb API grows to cover what they need — see docs/TODO.md,
// "Bridge".

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/panels/debug_panel.dart';

import '../state/dock.dart';
import 'easing_panel_frb.dart';
import 'effect_controls_panel_frb.dart';
import 'effects_presets_panel_frb.dart';
import 'graph_panel.dart';
import 'hierarchy_panel_frb.dart';
import 'node_panel.dart';
import 'node_preview_panel.dart';
import 'project_panel_frb.dart';
import 'scopes_panel_frb.dart';
import 'timeline_panel_frb.dart';
import 'viewer_panel_frb.dart';

Widget buildPanelBodyFrb(BuildContext context, Panel panel) => switch (panel) {
      Panel.project => const ProjectPanelFrb(),
      Panel.viewer => const ViewerPanelFrb(),
      Panel.timeline => const TimelinePanelFrb(),
      Panel.effectControls => const EffectControlsPanelFrb(),
      Panel.effectsAndPresets => const EffectsPresetsPanelFrb(),
      Panel.scopes => const ScopesPanelFrb(),
      Panel.hierarchy => const HierarchyPanelFrb(),
      Panel.easing => const EasingPanelFrb(),
      Panel.graph => const GraphPanelFrb(),
      Panel.node => const NodePanelFrb(),
      Panel.nodePreview => const NodePreviewPanelFrb(),
      Panel.debug => const DebugPanel(),
    };
