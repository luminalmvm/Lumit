// The panel dispatcher: one file per panel (file-length hygiene, K-007 spirit)
// — this module only routes a Panel to its widget. Panels still waiting on a
// phase render the shared PlaceholderPanel naming that phase.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../state/app_state.dart';
import '../state/dock.dart';
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
      Panel.effectControls => const PlaceholderPanel(
          icon: LumitIcon.fx,
          title: 'Effect controls',
          hint:
              'Transform and effect property rows arrive in phase F4; select a layer to edit it here.',
        ),
      Panel.effectsAndPresets => const PlaceholderPanel(
          icon: LumitIcon.star,
          title: 'Effects & presets',
          hint:
              'The searchable effect list and .lumfx presets arrive in phase F4.',
        ),
      Panel.scopes => const ScopesPanel(),
      Panel.hierarchy => const PlaceholderPanel(
          icon: LumitIcon.nodes,
          title: 'Hierarchy',
          hint: 'The composition tree arrives in phase F4.',
        ),
    };
