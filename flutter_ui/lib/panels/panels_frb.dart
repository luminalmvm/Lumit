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
import 'package:lumit_flutter/main.dart' show LumitUiState;
import 'package:lumit_flutter/panels/debug_panel.dart';
import 'package:provider/provider.dart';

import '../state/dock.dart';
import 'align_panel.dart';
import 'audio_panel_frb.dart';
import 'audio_timeline_panel_frb.dart';
import 'cut_timeline_panel_frb.dart';
import 'easing_panel_frb.dart';
import 'effect_controls_panel_frb.dart';
import 'effects_presets_panel_frb.dart';
import 'expressions_panel_frb.dart';
import 'grade_panel_frb.dart';
import 'graph_panel.dart';
import 'hierarchy_panel_frb.dart';
import 'mixer_panel_frb.dart';
import 'node_panel.dart';
import 'notes_panel_frb.dart';
import 'captions_panel_frb.dart';
import 'project_panel_frb.dart';
import 'retime_panel_frb.dart';
import 'paragraph_panel_frb.dart';
import 'scopes_panel_frb.dart';
import 'stagger_panel_frb.dart';
import 'template_panel_frb.dart';
import 'extension_panel.dart';
import 'text_panel_frb.dart';
import 'timeline_panel_frb.dart';
import 'transcript_panel_frb.dart';
import 'viewer_panel_frb.dart';

/// The panels whose whole business is changing the project. The Viewer,
/// the timelines and the Project panel are where a person looks, plays and
/// chooses a composition, so they are not among them.
const Set<Panel> _changing = {
  Panel.effectControls,
  Panel.effectsAndPresets,
  Panel.grade,
  Panel.easing,
  Panel.node,
  Panel.mixer,
  Panel.audio,
  Panel.text,
  Panel.paragraph,
  Panel.expressions,
  Panel.stagger,
  Panel.retime,
  Panel.notes,
  Panel.align,
  Panel.captions,
  Panel.graphicsTemplate,
};

/// The panels laid out along a time or space axis, or holding code. They read
/// left to right in every language, as numbers and timecode do.
const Set<Panel> _leftToRight = {
  Panel.viewer,
  Panel.timeline,
  Panel.scopes,
  Panel.easing,
  Panel.graph,
  Panel.mixer,
  Panel.audio,
  Panel.audioTimeline,
  Panel.cutTimeline,
  Panel.retime,
  Panel.expressions,
  Panel.align,
  Panel.paragraph,
};

Widget buildPanelBodyFrb(BuildContext context, PaneId pane) {
  final body = _changing.contains(pane.panel)
      ? LookHeld(child: _panelBody(pane))
      : _panelBody(pane);
  return _leftToRight.contains(pane.panel)
      ? Directionality(textDirection: TextDirection.ltr, child: body)
      : body;
}

/// A panel of controls on a machine that can only look at the shared project
/// it is in: dimmed, and taking no pointer and no keyboard focus. It is as it
/// was the moment that ends. `ToolsState.lookOnly` is the rule.
class LookHeld extends StatelessWidget {
  final Widget child;
  const LookHeld({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    final tools = context.read<LumitUiState>().tools;
    return ListenableBuilder(
      listenable: tools,
      // Its own layer under the dim, so a meter ticking inside a dimmed panel
      // repaints the meter, not the whole panel.
      child: RepaintBoundary(child: child),
      builder: (context, child) => IgnorePointer(
        ignoring: tools.lookOnly,
        child: ExcludeFocus(
          excluding: tools.lookOnly,
          child: Opacity(opacity: tools.lookOnly ? 0.4 : 1, child: child),
        ),
      ),
    );
  }
}

Widget _panelBody(PaneId pane) => switch (pane.panel) {
      Panel.project => const ProjectPanelFrb(),
      // The one panel that can be in the arrangement more than once, so the
      // pane is what tells it which views it holds.
      Panel.viewer => ViewerPanelFrb(pane: pane),
      Panel.timeline => const TimelinePanelFrb(),
      Panel.effectControls => const EffectControlsPanelFrb(),
      Panel.effectsAndPresets => const EffectsPresetsPanelFrb(),
      Panel.scopes => const ScopesPanelFrb(),
      Panel.grade => const GradePanelFrb(),
      Panel.hierarchy => const HierarchyPanelFrb(),
      Panel.easing => const EasingPanelFrb(),
      Panel.graph => const GraphPanelFrb(),
      Panel.node => const NodePanelFrb(),
      Panel.mixer => const MixerPanelFrb(),
      Panel.audio => const AudioPanelFrb(),
      Panel.audioTimeline => const AudioTimelinePanelFrb(),
      Panel.cutTimeline => const CutTimelinePanelFrb(),
      Panel.text => const TextPanelFrb(),
      Panel.paragraph => const ParagraphPanelFrb(),
      Panel.expressions => const ExpressionsPanelFrb(),
      Panel.stagger => const StaggerPanelFrb(),
      Panel.retime => const RetimePanelFrb(),
      Panel.notes => const NotesPanelFrb(),
      Panel.captions => const CaptionsPanelFrb(),
      Panel.graphicsTemplate => const TemplatePanelFrb(),
      Panel.transcript => const TranscriptPanelFrb(),
      Panel.align => const AlignPanel(),
      Panel.debug => const DebugPanel(),
      Panel.extension => ExtensionPanel(slot: pane.instance),
    };
