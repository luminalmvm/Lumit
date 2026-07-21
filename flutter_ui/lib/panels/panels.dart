// The panel dispatcher: one file per panel (file-length hygiene, K-007 spirit)
// — this module only routes a Panel to its widget. Panels still waiting on a
// phase render the shared PlaceholderPanel naming that phase.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../state/app_state.dart';
import '../state/dock.dart';
import 'effect_controls_panel.dart';
import 'effects_presets_panel.dart';
import 'hierarchy_panel.dart';
import 'placeholder.dart';
import 'project_panel.dart';
import 'scopes_panel.dart';
import 'timeline_panel.dart';
import 'viewer_panel.dart';

Widget buildPanelBody(BuildContext context, Panel panel, AppStateStub app) =>
    switch (panel) {
      // The Project panel goes live when the bridge is present: it renders the
      // real document tree instead of the placeholder (phase F1). Without a
      // bridge the placeholder stays.
      Panel.project => (app.bridge != null && app.snapshot != null)
          ? ProjectPanel(app: app)
          : const PlaceholderPanel(
              icon: LumitIcon.folder,
              title: 'Project',
              hint:
                  'Project items, thumbnails and relink arrive with the engine bridge (phase F1).',
            ),
      Panel.viewer => ViewerPanel(app: app),
      Panel.timeline => TimelinePanel(app: app),
      // The Effect controls panel goes live with a comp in the snapshot: it
      // shows the selected layer's Transform rows (phase F4). Without a
      // bridge/comp the placeholder stays.
      Panel.effectControls => (app.bridge != null && app.frontComp != null)
          ? EffectControlsPanel(app: app)
          : const PlaceholderPanel(
              icon: LumitIcon.fx,
              title: 'Effect controls',
              hint:
                  'Transform and effect property rows arrive in phase F4; select a layer to edit it here.',
            ),
      // The Effects & presets panel goes live with a bridge: the searchable
      // built-in effect registry, applied to the selected layer (phase F4).
      // Without a bridge the placeholder stays. The .lumfx presets wait on the
      // file + preset bridge ops (a placeholder row names that inside).
      Panel.effectsAndPresets => app.bridge != null
          ? EffectsPresetsPanel(app: app)
          : const PlaceholderPanel(
              icon: LumitIcon.star,
              title: 'Effects & presets',
              hint:
                  'The searchable effect list and .lumfx presets arrive in phase F4.',
            ),
      Panel.scopes => ScopesPanel(app: app),
      // The Hierarchy panel goes live with a comp in the snapshot: the front
      // comp's layer tree, precomps expandable (phase F4). Without a
      // bridge/comp the placeholder stays.
      Panel.hierarchy => (app.bridge != null && app.frontComp != null)
          ? HierarchyPanel(app: app)
          : const PlaceholderPanel(
              icon: LumitIcon.nodes,
              title: 'Hierarchy',
              hint: 'The composition tree arrives in phase F4.',
            ),
    };
