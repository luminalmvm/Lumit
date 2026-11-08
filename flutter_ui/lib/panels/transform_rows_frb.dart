// A layer's Transform properties as editable rows — the one implementation of
// them, used by both panels that show them.
//
// The Effect controls panel wraps these in its Transform card; the Timeline
// twirls them open under a layer. They were the Effect controls card's private
// business first, and the Timeline's fold-out would have been a second copy of
// the same eleven properties, the same staging, and the same preview throttle —
// which is exactly the kind of copy that drifts. So they moved here whole.
//
// **What a row is.** Keyframe controls (the stopwatch and the ◄ ◆ ► navigator),
// a label, and one draggable value per axis. A property group that has more than
// one axis — Position is x and y — is *one* row with one stopwatch, because a
// control that says "Position" has to act on Position; the axes are separate
// properties underneath (which is what makes a per-axis curve possible) and are
// committed together as one op.
//
// **What a drag costs.** One undo step, not one per tick. A tick stages the new
// value locally and renders it through `renderFrameWithTransformPreview`, which
// patches a clone of the document engine-side and never touches the document;
// only the release commits. An animated property is not draggable at all —
// writing a static value over a curve would delete it — and says so instead.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/menu_animation_frb.dart' show toggleKeysAtPlayhead;
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../shell/menu_layer_frb.dart'
    show freshTransform, onSelection, transformRows;
import '../state/comp_time.dart';
import '../state/preview_throttle.dart';
import '../state/timeline_columns.dart';
import '../widgets/angle_dial.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'drive_chain.dart' show wiggleExpression;
import 'graph_editor_frb.dart';
import 'fx_key_strip.dart';
import 'fx_section.dart';
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart' show transformGroupPath;
import 'light_rows_frb.dart';
import 'param_row_menu.dart';
import 'timeline_layer_menu_frb.dart' show showMenuEntriesAt;
import '../state/expression_language.dart' show PropertyRef;
import 'property_link.dart';

/// How wide one value cell is. Fixed rather than flexible so the columns line
/// up down the card, which is what makes a stack of numbers readable.
const double transformCellWidth = 74;

/// One axis of a transform row: which property it edits, and the display hints
/// that make its drag feel right.
class TransformAxis {
  final BridgeTransformProp prop;
  final String? suffix;
  final double min;
  final double max;
  final int decimals;
  final double speed;

  /// A rotation counts whole turns beside its degrees; an angle that is not
  /// a turn, a cone's, does not.
  final bool turns;
  const TransformAxis(
    this.prop, {
    this.suffix,
    this.min = -100000,
    this.max = 100000,
    this.decimals = 1,
    this.speed = 1,
    this.turns = true,
  });
}

/// Which heading a transform row sits under in the fold-out: the Transform
/// group itself, one of the option groups a camera or a light has inside it,
/// or a 3D layer's Material options beside it.
enum TransformSection { transform, camera, light, material }

/// One row: its label, the axes it edits, and — on the three properties that
/// have more than one axis — which pair it belongs to and how that pair is
/// being shown.
class TransformGroup {
  final String label;
  final List<TransformAxis> axes;

  /// The pair this row came out of, or null on Rotation and Opacity, which have
  /// nothing to separate. It is what the row's menu acts on.
  final BridgeTransformPair? pair;

  /// The pair's mode. [BridgeAxisMode.separated] means this row is one axis of
  /// it; [BridgeAxisMode.linked] means the row draws one box for two axes.
  final BridgeAxisMode mode;

  /// The heading the row sits under: a camera's lens numbers and a light's
  /// own are options rather than placement (docs/impl/camera.md §1), so the
  /// fold-out puts their own heading in front of them.
  final TransformSection section;

  /// Three axes that are one colour, drawn as a swatch rather than as three
  /// numbers.
  final bool colour;

  const TransformGroup(
    this.label,
    this.axes, {
    this.pair,
    this.mode = BridgeAxisMode.combined,
    this.section = TransformSection.transform,
    this.colour = false,
  });

  /// A linked row draws one box and carries the other axis with it.
  bool get isLinked => mode == BridgeAxisMode.linked && axes.length > 1;
}

/// Whether a layer of this kind has a 3D switch to flip.
///
/// A Camera, a Light, a Model layer and a 3D shape are three-dimensional by
/// being what they are (docs/impl/camera.md §1), so the switch would be one you
/// cannot turn off: the Timeline leaves their cell blank and the Layer menu
/// skips them.
bool hasThreeDSwitch(BridgeLayerKind kind) =>
    kind != BridgeLayerKind.camera &&
    kind != BridgeLayerKind.light &&
    kind != BridgeLayerKind.model &&
    kind != BridgeLayerKind.mesh;

/// Whether a layer of this kind carries the visibility eye.
///
/// A camera puts no pixels on the screen and keeps the eye all the same: on a
/// camera the switch is what makes it the composition's active one
/// (docs/impl/camera.md §6), which is the most useful thing on the row.
bool hasVisibilitySwitch(BridgeLayerKind kind, {required bool hasPicture}) =>
    hasPicture || kind == BridgeLayerKind.camera;

/// The rows a layer shows, in order.
///
/// The 3D rows (Position z, Rotation x, Rotation y) appear only on a 3D layer: a
/// 2D layer showing controls that cannot do anything is worse than not showing
/// them. Exposed as a list rather than built inline because the Timeline has to
/// know *how many* rows a layer will take before it draws them — its lanes have
/// to leave exactly that much room or the bars stop lining up with the names.
///
/// **Separated axes are more rows, not different data**. The axes are
/// separate scalar properties in the document whatever the mode says; a
/// separated pair simply hands each of them a row of its own, with its own
/// stopwatch, its own lane and its own curve. Which is why every surface that
/// walks this list — the fold-out, the lanes, the graph editor, the Effect
/// controls card — follows without knowing the feature exists.
///
/// **A camera and a light show a different list.** Neither has an anchor, a
/// scale or an opacity - they are viewpoints, not pictures - and both carry z
/// whatever the 3D switch says, because being three-dimensional is what they
/// are (docs/impl/camera.md §1). A camera adds its point of interest when it
/// is two-node, and the four lens numbers the fold-out heads with Camera
/// options. A light adds its point of interest when it is aimed, and the
/// numbers its kind has under Light options.
List<TransformGroup> transformGroups({
  required bool threeD,
  required BridgeAxisModes modes,
  BridgeLayerKind kind = BridgeLayerKind.footage,
  bool twoNode = false,
  BridgeLightSettings? light,
  BridgeMaterialSettings? material,
}) {
  List<TransformGroup> pairRows(
    BridgeTransformPair pair,
    String label,
    List<String> axisLabels,
    List<TransformAxis> axes,
  ) {
    final mode = switch (pair) {
      BridgeTransformPair.anchor => modes.anchor,
      BridgeTransformPair.position => modes.position,
      BridgeTransformPair.scale => modes.scale,
    };
    if (mode != BridgeAxisMode.separated) {
      return [TransformGroup(label, axes, pair: pair, mode: mode)];
    }
    return [
      for (var i = 0; i < axes.length; i++)
        TransformGroup(axisLabels[i], [axes[i]], pair: pair, mode: mode),
    ];
  }

  // After Effects' Orientation: three angles on one row, turning as one.
  final orientation = TransformGroup(l10n.transformOrientation, const [
    TransformAxis(BridgeTransformProp.orientationX,
        suffix: '°', speed: 0.5, turns: false),
    TransformAxis(BridgeTransformProp.orientationY,
        suffix: '°', speed: 0.5, turns: false),
    TransformAxis(BridgeTransformProp.orientationZ,
        suffix: '°', speed: 0.5, turns: false),
  ]);

  // The placement a viewpoint has: where it stands, and which way it faces.
  // An environment light is nowhere, so it only faces.
  List<TransformGroup> placement({bool placed = true}) => [
        if (placed)
          ...pairRows(
          BridgeTransformPair.position,
          l10n.transformPosition,
          [
            l10n.transformPositionX,
            l10n.transformPositionY,
            l10n.transformPositionZ,
          ],
          const [
            TransformAxis(BridgeTransformProp.positionX),
            TransformAxis(BridgeTransformProp.positionY),
            TransformAxis(BridgeTransformProp.positionZ),
          ],
        ),
        orientation,
        TransformGroup(l10n.transformRotationX, const [
          TransformAxis(BridgeTransformProp.rotationX, suffix: '°', speed: 0.5),
        ]),
        TransformGroup(l10n.transformRotationY, const [
          TransformAxis(BridgeTransformProp.rotationY, suffix: '°', speed: 0.5),
        ]),
        TransformGroup(l10n.transformRotation, const [
          TransformAxis(BridgeTransformProp.rotation, suffix: '°', speed: 0.5),
        ]),
      ];

  if (kind == BridgeLayerKind.camera) {
    return [
      // Inert on a one-node camera, so it is not shown there: the rows a
      // camera has follow how it aims.
      if (twoNode)
        TransformGroup(l10n.transformPointOfInterest, const [
          TransformAxis(BridgeTransformProp.poiX),
          TransformAxis(BridgeTransformProp.poiY),
          TransformAxis(BridgeTransformProp.poiZ),
        ]),
      ...placement(),
      TransformGroup(l10n.sourceZoom, const [
        TransformAxis(BridgeTransformProp.zoom,
            min: 1, decimals: 0, speed: 4),
      ], section: TransformSection.camera),
      TransformGroup(l10n.cameraFocusDistance, const [
        TransformAxis(BridgeTransformProp.focusDistance,
            min: 1, decimals: 0, speed: 4),
      ], section: TransformSection.camera),
      TransformGroup(l10n.cameraAperture, const [
        TransformAxis(BridgeTransformProp.aperture, min: 0),
      ], section: TransformSection.camera),
      TransformGroup(l10n.cameraBlurLevel, const [
        TransformAxis(BridgeTransformProp.blurLevel,
            suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
      ], section: TransformSection.camera),
    ];
  }
  if (kind == BridgeLayerKind.light) {
    final s = light ?? plainLight;
    const option = TransformSection.light;
    return [
      // The point it is aimed at, while it is aimed and has a way to face.
      if (s.twoNode && lightAims(s.kind))
        TransformGroup(l10n.transformPointOfInterest, const [
          TransformAxis(BridgeTransformProp.poiX),
          TransformAxis(BridgeTransformProp.poiY),
          TransformAxis(BridgeTransformProp.poiZ),
        ]),
      ...placement(placed: s.kind != BridgeLightKind.environment),
      TransformGroup(l10n.lightIntensity, const [
        TransformAxis(BridgeTransformProp.lightIntensity,
            suffix: '%', decimals: 0, speed: 0.5),
      ], section: option),
      TransformGroup(
          l10n.lightColour,
          const [
            TransformAxis(BridgeTransformProp.lightColourR, min: 0),
            TransformAxis(BridgeTransformProp.lightColourG, min: 0),
            TransformAxis(BridgeTransformProp.lightColourB, min: 0),
          ],
          section: option,
          colour: true),
      if (s.kind == BridgeLightKind.spot) ...[
        TransformGroup(l10n.lightConeAngle, const [
          TransformAxis(BridgeTransformProp.coneAngle,
              suffix: '°', min: 0, max: 180, speed: 0.5, turns: false),
        ], section: option),
        TransformGroup(l10n.lightConeFeather, const [
          TransformAxis(BridgeTransformProp.coneFeather,
              suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
        ], section: option),
      ],
      if (s.kind == BridgeLightKind.area)
        TransformGroup(l10n.size, [
          TransformAxis(BridgeTransformProp.lightWidth,
              suffix: l10n.unitSymbolPx, min: 0, decimals: 0),
          TransformAxis(BridgeTransformProp.lightHeight,
              suffix: l10n.unitSymbolPx, min: 0, decimals: 0),
        ], section: option),
      if (lightFallsOff(s.kind) && s.falloff != BridgeLightFalloff.none)
        TransformGroup(l10n.lightRadius, [
          TransformAxis(BridgeTransformProp.lightRadius,
              suffix: l10n.unitSymbolPx, min: 0, decimals: 0, speed: 2),
        ], section: option),
      if (lightFallsOff(s.kind) && s.falloff == BridgeLightFalloff.smooth)
        TransformGroup(l10n.lightFalloffDistance, [
          TransformAxis(BridgeTransformProp.falloffDistance,
              suffix: l10n.unitSymbolPx, min: 0, decimals: 0, speed: 2),
        ], section: option),
      if (s.kind == BridgeLightKind.environment && s.background)
        TransformGroup(l10n.lightBackgroundBlur, const [
          TransformAxis(BridgeTransformProp.backgroundBlur,
              suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
        ], section: option),
      if (s.castsShadows && lightCasts(s.kind)) ...[
        TransformGroup(l10n.lightShadowDarkness, const [
          TransformAxis(BridgeTransformProp.shadowDarkness,
              suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
        ], section: option),
        TransformGroup(l10n.lightShadowDiffusion, [
          TransformAxis(BridgeTransformProp.shadowDiffusion,
              suffix: l10n.unitSymbolPx, min: 0, decimals: 0, speed: 0.5),
        ], section: option),
      ],
      // How it shows in the air, and once it does, what the air is like. An
      // ambient or environment light comes from nowhere, so it has no beam
      // to show.
      if (lightCasts(s.kind))
        TransformGroup(l10n.lightVolumetric, const [
          TransformAxis(BridgeTransformProp.hazeAmount,
              suffix: '%', min: 0, decimals: 0, speed: 0.5),
        ], section: option),
      if (lightCasts(s.kind) && s.inAir) ...[
        TransformGroup(l10n.lightHazeDensity, const [
          TransformAxis(BridgeTransformProp.hazeDensity,
              suffix: '%', min: 0, decimals: 0, speed: 0.5),
        ], section: option),
        TransformGroup(
            l10n.lightHazeColour,
            const [
              TransformAxis(BridgeTransformProp.hazeColourR, min: 0),
              TransformAxis(BridgeTransformProp.hazeColourG, min: 0),
              TransformAxis(BridgeTransformProp.hazeColourB, min: 0),
            ],
            section: option,
            colour: true),
        TransformGroup(l10n.lightHeightFalloff, const [
          TransformAxis(BridgeTransformProp.hazeFalloff,
              suffix: '%', min: 0, decimals: 0, speed: 0.5),
        ], section: option),
        TransformGroup(l10n.lightForwardScatter, const [
          TransformAxis(BridgeTransformProp.hazeScatter,
              suffix: '%', min: -100, max: 100, decimals: 0, speed: 0.5),
        ], section: option),
        TransformGroup(l10n.lightNoise, const [
          TransformAxis(BridgeTransformProp.hazeNoise,
              suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
        ], section: option),
      ],
      if (lightCasts(s.kind) && s.inAir && s.noisy) ...[
        TransformGroup(l10n.lightNoiseSize, [
          TransformAxis(BridgeTransformProp.hazeNoiseSize,
              suffix: l10n.unitSymbolPx, min: 0, decimals: 0, speed: 2),
        ], section: option),
        TransformGroup(l10n.lightNoiseSpeed, [
          TransformAxis(BridgeTransformProp.hazeNoiseSpeed,
              suffix: l10n.unitPerSecond(l10n.unitSymbolPx),
              decimals: 0,
              speed: 0.5),
        ], section: option),
      ],
    ];
  }

  return [
    ...pairRows(
      BridgeTransformPair.anchor,
      l10n.transformAnchorPoint,
      [
        l10n.transformAnchorPointX,
        l10n.transformAnchorPointY,
        if (threeD) l10n.transformAnchorPointZ,
      ],
      [
        const TransformAxis(BridgeTransformProp.anchorX),
        const TransformAxis(BridgeTransformProp.anchorY),
        if (threeD) const TransformAxis(BridgeTransformProp.anchorZ),
      ],
    ),
    ...pairRows(
      BridgeTransformPair.position,
      l10n.transformPosition,
      [
        l10n.transformPositionX,
        l10n.transformPositionY,
        if (threeD) l10n.transformPositionZ,
      ],
      [
        const TransformAxis(BridgeTransformProp.positionX),
        const TransformAxis(BridgeTransformProp.positionY),
        if (threeD) const TransformAxis(BridgeTransformProp.positionZ),
      ],
    ),
    ...pairRows(
      BridgeTransformPair.scale,
      l10n.transformScale,
      [
        l10n.transformScaleX,
        l10n.transformScaleY,
        if (threeD) l10n.transformScaleZ,
      ],
      [
        const TransformAxis(BridgeTransformProp.scaleX, suffix: '%'),
        const TransformAxis(BridgeTransformProp.scaleY, suffix: '%'),
        if (threeD)
          const TransformAxis(BridgeTransformProp.scaleZ, suffix: '%'),
      ],
    ),
    if (threeD) orientation,
    TransformGroup(l10n.transformRotation, const [
      TransformAxis(BridgeTransformProp.rotation, suffix: '°', speed: 0.5),
    ]),
    if (threeD) ...[
      TransformGroup(l10n.transformRotationX, const [
        TransformAxis(BridgeTransformProp.rotationX, suffix: '°', speed: 0.5),
      ]),
      TransformGroup(l10n.transformRotationY, const [
        TransformAxis(BridgeTransformProp.rotationY, suffix: '°', speed: 0.5),
      ]),
    ],
    TransformGroup(l10n.transformOpacity, [
      TransformAxis(BridgeTransformProp.opacity,
          suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
    ]),
    // How a 3D layer takes the comp's lights, as After Effects has it. A null
    // has no picture to light, and a model's or a 3D shape's materials have
    // their own rows.
    if (threeD &&
        kind != BridgeLayerKind.nullLayer &&
        kind != BridgeLayerKind.model &&
        kind != BridgeLayerKind.mesh)
      for (final (label, prop) in [
        // Light passes through a layer only where it casts a shadow.
        if (material?.castsShadows != BridgeShadowMode.off)
          (l10n.materialLightTransmission, BridgeTransformProp.lightTransmission),
        (l10n.materialAmbient, BridgeTransformProp.ambient),
        (l10n.materialDiffuse, BridgeTransformProp.diffuse),
        (l10n.materialSpecular, BridgeTransformProp.specular),
        (l10n.materialShininess, BridgeTransformProp.shininess),
        (l10n.materialMetal, BridgeTransformProp.metal),
        (l10n.materialReflection, BridgeTransformProp.reflection),
      ])
        TransformGroup(label, [
          TransformAxis(prop,
              suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
        ], section: TransformSection.material),
  ];
}

/// A layer's transform rows, all of them.
///
/// The Effect controls card shows the whole set; the Timeline's fold-out draws
/// them one at a time (its lanes are per row), so the row itself is the widget
/// that carries the behaviour and this is a Column of them.
class TransformRowsFrb extends StatelessWidget {
  final CompositionReference comp;
  final LayerReference layer;

  /// The layer's transform and 3D flag, from the read model — so
  /// drawing the rows costs no bridge calls.
  final BridgeTransform transform;
  final bool threeD;

  /// What the layer is, and - on a camera - how it aims: the two other things
  /// that decide which rows there are. A light's settings decide its own.
  final BridgeLayerKind kind;
  final bool twoNode;
  final BridgeLightSettings? light;

  /// The layer's Accepts lights switch and its shadow settings, for the
  /// Material options' own rows.
  final bool acceptsLights;
  final BridgeMaterialSettings? material;

  /// How each pair is shown — which decides how many rows there are.
  final BridgeAxisModes axisModes;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  /// Prefixes every row's widget key, so the same rows in two panels do not
  /// collide when both are on screen.
  final String keyPrefix;

  /// A fixed height per row, for a caller that has to line something up beside
  /// them (the Timeline's lanes). Null lets each row take what it needs.
  final double? rowHeight;

  /// Padding inside each row.
  final EdgeInsets rowPadding;

  /// Lay the rows out as the Effect controls panel's two columns.
  final bool twoColumn;

  const TransformRowsFrb({
    super.key,
    required this.comp,
    required this.layer,
    required this.transform,
    required this.threeD,
    this.kind = BridgeLayerKind.footage,
    this.twoNode = false,
    this.light,
    this.acceptsLights = true,
    this.material,
    required this.axisModes,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.keyPrefix = 'tf',
    this.rowHeight,
    this.rowPadding = const EdgeInsets.symmetric(vertical: 2),
    this.twoColumn = false,
    this.filter = '',
  });

  /// The panel's row filter, lowercase: only the groups whose name holds it
  /// are drawn. Empty draws them all.
  final String filter;

  /// One widget per transform row — for a caller that has to put each row in its
  /// own chrome (the Effect controls panel's hairline-separated rows). A
  /// section's choice rows come first in it, as they do in the fold-out.
  List<Widget> rows(BuildContext context) {
    final settings = layerSettings(
        kind: kind, light: light, threeD: threeD, twoNode: twoNode);
    final out = <Widget>[];
    TransformSection? last;
    final seen = <TransformSection>{};
    // The Effect controls card names each options group as the Timeline's
    // headings do.
    void options(TransformSection section) {
      seen.add(section);
      final heading = switch (section) {
        TransformSection.transform => null,
        TransformSection.camera => l10n.cameraOptions,
        TransformSection.light => l10n.lightOptions,
        TransformSection.material => l10n.materialOptions,
      };
      if (twoColumn && heading != null) out.add(_OptionsHeading(heading));
      for (final setting in settings) {
        if (setting.section != section) continue;
        out.add(LayerSettingRowFrb(
          layer: layer,
          setting: setting,
          light: light,
          material: material,
          acceptsLights: acceptsLights,
          onChanged: onChanged,
          orient: transform.orient,
          threeD: threeD,
          kind: kind,
          keyPrefix: keyPrefix,
        ));
      }
    }

    for (final group in transformGroups(
        threeD: threeD,
        modes: axisModes,
        kind: kind,
        twoNode: twoNode,
        light: light,
        material: material)) {
      if (filter.isNotEmpty &&
          !engineLabel(group.label).toLowerCase().contains(filter)) {
        continue;
      }
      if (group.section != last) {
        last = group.section;
        options(group.section);
      }
      out.add(TransformRowFrb(
        comp: comp,
        layer: layer,
        transform: transform,
        group: group,
        playheadFrame: playheadFrame,
        onSeek: onSeek,
        onChanged: onChanged,
        keyPrefix: keyPrefix,
        rowHeight: rowHeight,
        rowPadding: rowPadding,
        twoColumn: twoColumn,
      ));
    }
    // A section of switches with no numbers, as a Model layer's materials.
    for (final section in {for (final s in settings) s.section}) {
      if (!seen.contains(section)) options(section);
    }
    return out;
  }

  @override
  Widget build(BuildContext context) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: rows(context),
      );
}

/// The name over a camera's, a light's or a layer's option rows in the Effect
/// controls card, in the label column.
class _OptionsHeading extends StatelessWidget {
  final String label;
  const _OptionsHeading(this.label);

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return SizedBox(
      height: fxRowHeight(t),
      child: Padding(
        padding: const EdgeInsetsDirectional.only(start: fxKeyColumnWidth + 4),
        child: Align(
          alignment: AlignmentDirectional.centerStart,
          child: Text(t.kickerCase(label), style: t.kicker),
        ),
      ),
    );
  }
}

/// One transform property group as a row.
class TransformRowFrb extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;

  /// The layer's transform as the owner last read it — one read shared by all
  /// the rows, rather than one crossing per row.
  final BridgeTransform transform;
  final TransformGroup group;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;
  final String keyPrefix;
  final double? rowHeight;
  final EdgeInsets rowPadding;

  /// When set (the Timeline's fold-out), the value cells share this fixed
  /// span — aligned to both its edges — instead of each taking
  /// [transformCellWidth], so the values sit exactly under the render-switch
  /// column group whatever order the groups are dragged into (docs/07 §4.3).
  final ValueColumn? valueColumn;

  /// Clicking the property's *name* selects it for the graph editor
  /// (docs/07 §4.3) — the name, not the whole row, so grabbing a value field
  /// or a stopwatch never re-aims the graph.
  final VoidCallback? onLabelTap;

  /// The property's graph line colours while it is selected — one per axis,
  /// so Position reads as its x and y strokes (docs/07 §5). The label takes
  /// the first; a multi-axis row shows one dot per axis beside it.
  final List<Color>? graphColours;

  /// Lay the row out as the Effect controls panel's two columns — name left,
  /// axes left-aligned in the rest. Ignored when [valueColumn] is set.
  final bool twoColumn;

  /// The group this row came out of, drawn before its name on a **flat
  /// sheet**: the dope sheet lists `Transform · Position`, where the
  /// fold-out draws `Position` under the Transform twirl. Null everywhere the
  /// row sits inside its own group.
  final String? nameGroup;

  const TransformRowFrb({
    super.key,
    required this.comp,
    required this.layer,
    required this.transform,
    required this.group,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.keyPrefix = 'tf',
    this.rowHeight,
    this.rowPadding = const EdgeInsets.symmetric(vertical: 2),
    this.valueColumn,
    this.onLabelTap,
    this.graphColours,
    this.twoColumn = false,
    this.nameGroup,
  });

  @override
  State<TransformRowFrb> createState() => _TransformRowFrbState();
}

class _TransformRowFrbState extends State<TransformRowFrb> {
  /// The transform being dragged, held only for the length of one drag, so the
  /// preview renders the other ten properties as the document has them.
  BridgeTransform? _staged;

  /// Bounded preview rate, holding rather than dropping the ticks in between,
  /// so the pointer's last position always reaches the picture.
  final PreviewThrottle _throttle = PreviewThrottle();

  /// A linked row's ratio of each other axis to x, taken once per gesture.
  /// Once, because re-reading it every tick would read the value the tick
  /// before had just written — the ratio would hold trivially and the link
  /// would do nothing — and because sampling costs a crossing the drag does
  /// not need to pay sixty times a second.
  final Map<BridgeTransformProp, double> _linkRatios = {};

  /// The partner axis's value for `value` on the leading one, holding the
  /// ratio the axes already had. A leading axis sitting at zero has no ratio to
  /// keep, so the partner simply matches it.
  double _linked(BridgeTransformProp lead, BridgeTransformProp partner,
      double value, int frame) {
    final ratio = _linkRatios[partner] ??= () {
      final time = timeOfFrame(widget.comp, frame);
      final x = sampledScalar(read(widget.transform, lead), time);
      final y = sampledScalar(read(widget.transform, partner), time);
      return x == 0 ? 1.0 : y / x;
    }();
    return value * ratio;
  }

  /// Held rather than read through the context on the way past, because
  /// [dispose] needs it after the context is no longer a place to look things
  /// up — and the notifier itself outlives every row.
  ///
  /// Filled in [initState] rather than lazily: a row that is never dragged
  /// would otherwise run the lookup for the first time *in* [dispose], where
  /// the element is already being taken down and an ancestor lookup throws.
  late final LumitUiState _ui;

  @override
  void initState() {
    super.initState();
    _ui = Provider.of<LumitUiState>(context, listen: false);
  }

  /// Tell the Viewer's boxes what this drag is doing. The picture is already
  /// previewed at `staged`; without this the wireframe drawn from the document
  /// sits still until the drag is let go (see [LumitUiState.liveTransforms]).
  void _publishLive(BridgeTransform staged) {
    _ui.liveTransforms.value = {widget.layer.internallayerId: staged};
  }

  /// The gesture is over: the document is the truth again.
  void _clearLive() {
    if (_ui.liveTransforms.value.isNotEmpty) {
      _ui.liveTransforms.value = const {};
    }
  }

  @override
  void dispose() {
    _throttle.cancel();
    // A drag cut short by the panel closing or the selection changing must not
    // leave the box frozen at a provisional value nothing will ever replace.
    // After the frame, not in it: dispose can run inside a build, and the
    // Viewer's boxes builder listens to this notifier — firing it mid-build
    // marks that builder dirty while it is building. The notifier belongs to
    // the shell, so it is safe to touch once this row has gone.
    final live = _ui.liveTransforms;
    if (live.value.isNotEmpty) {
      WidgetsBinding.instance
          .addPostFrameCallback((_) => live.value = const {});
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    // The live playhead: an animated cell shows (and edits) the value under
    // the playhead, so it must follow a scrub rather than hold the frame the
    // panel last drew at.
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) =>
          _row(_staged ?? widget.transform, widget.group, frame),
    );
  }

  Widget _row(BridgeTransform transform, TransformGroup group, int frame) {
    final t = ThemeScope.of(context).theme;
    // A linked row draws one box for two axes: the second follows the
    // first through [_ratio], so a box for it would be a box that can only ever
    // be told what it already knows. The stopwatch above still covers both. A
    // 3D layer's linked Scale shows all three, as After Effects does, so a z
    // off the others' ratio can be seen and typed.
    final cells = (group.isLinked && group.axes.length < 3) || group.colour
        ? group.axes.sublist(0, 1)
        : group.axes;
    // One stopwatch, every axis in the row — and one undo step, because
    // `setTransforms` commits them as a batch.
    final keyframes = KeyframeControlsFrb(
      scalars: [for (final axis in group.axes) read(transform, axis.prop)],
      comp: widget.comp,
      onSeek: widget.onSeek,
      rowKey: '${widget.keyPrefix}-${group.axes.first.prop.name}',
      // The Effect controls panel's fixed columns; the Timeline's
      // fold-out draws the same rows against its own column group and keeps
      // the loose layout.
      fixedColumns: widget.twoColumn && widget.valueColumn == null,
      navigatorApart: widget.valueColumn != null,
      // An Orientation keys the turn shown, not each angle off its own curve.
      valuesAt: _isOrientation(group.axes.first.prop)
          ? (time) {
              final d = transform.depth;
              final o = d == null
                  ? null
                  : sampledOrientation(
                      d.orientationX, d.orientationY, d.orientationZ, time);
              return o == null ? null : [o.$1, o.$2, o.$3];
            }
          : null,
      onWrite: (next) {
        widget.layer.setTransforms(
          props: [for (final axis in group.axes) axis.prop],
          values: next,
        );
        setState(() => _staged = null);
        widget.onChanged();
      },
    );

    // The name is the row's handle for the graph editor, so it is built once
    // and drawn by whichever layout the row takes.
    final menu = _menu(group, keyframes, cells, transform);
    Widget name(double lit) => GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: widget.onLabelTap,
      child: Row(
        children: [
          ...flatGroupPrefix(t, widget.nameGroup),
          Flexible(
            child: Text(
              t.propertyCase(group.label),
              style: widget.graphColours?.isNotEmpty ?? false
                  ? t.body.copyWith(color: widget.graphColours!.first)
                  : t.body.copyWith(
                      color: Color.lerp(t.body.color, t.textPrimary, lit)),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          // One dot per axis in its stroke colour, so a two-axis property names
          // both of its curves.
          if ((widget.graphColours?.length ?? 0) > 1)
            for (final colour in widget.graphColours!)
              Padding(
                padding: const EdgeInsetsDirectional.only(start: 3),
                child: Container(
                  width: 5,
                  height: 5,
                  decoration: BoxDecoration(
                    color: colour,
                    borderRadius: BorderRadius.circular(3),
                  ),
                ),
              ),
        ],
      ),
    );
    // Only a name that picks its property lights.
    final label = widget.onLabelTap == null
        ? name(0)
        : HoverMark(builder: (context, lit) => name(lit));

    if (widget.twoColumn && widget.valueColumn == null) {
      // No padding of its own: the Effect controls panel gives every row the
      // same fixed height ([fxRowHeight]), and padding on top of that would
      // eat into the room the controls sit in.
      final row = fxTwoColumnRow(
        context: context,
        name: label,
        keyframeControls: keyframes,
        lane: FxKeyLane(
          rowKey: keyframes.rowKey,
          scalars: keyframes.scalars,
          onWrite: keyframes.onWrite,
        ),
        control: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (var i = 0; i < cells.length; i++) ...[
              if (i > 0) const SizedBox(width: 6),
              // Flexible so an expression's field gives way in a narrow
              // panel; a fixed-width value box ignores the loose constraint.
              Flexible(child: _cell(transform, cells[i], frame, group: group)),
            ],
          ],
        ),
        trailing: _linkMark(group, transform),
      );
      final height = widget.rowHeight;
      return LinkDrop(
        ref: (link) => _linkRef(link, group, cells),
        child: ParamRowMenu(
          menu: menu,
          child: height == null ? row : SizedBox(height: height, child: row),
        ),
      );
    }

    // The Timeline's fold: the link mark ends the name, so the value boxes
    // stay where the columns above put them.
    final mark =
        widget.valueColumn == null ? null : _linkMark(group, transform);
    final row = Padding(
      padding: widget.rowPadding,
      child: Row(
        children: [
          keyframes,
          const SizedBox(width: 4),
          Expanded(child: label),
          if (widget.valueColumn != null)
            if (keyframes.trailingNavigator case final nav?) nav,
          if (mark != null) mark,
          if (widget.valueColumn case final col?) ...[
            SizedBox(
              width: _valueAreaWidth(col, cells.length),
              child: Row(
                children: [
                  for (var i = 0; i < cells.length; i++) ...[
                    if (i > 0) const SizedBox(width: 4),
                    Expanded(
                        child: _cell(transform, cells[i], frame,
                            fixed: false, group: group)),
                  ],
                ],
              ),
            ),
            SizedBox(
                width: col.width +
                    col.rightInset -
                    _valueAreaWidth(col, cells.length)),
          ] else
            for (final axis in cells) ...[
              const SizedBox(width: 6),
              Flexible(child: _cell(transform, axis, frame, group: group)),
            ],
        ],
      ),
    );
    final height = widget.rowHeight;
    final menued = ParamRowMenu(
      menu: menu,
      child: height == null ? row : SizedBox(height: height, child: row),
    );
    if (widget.valueColumn == null) return menued;
    return LinkDrop(
      ref: (link) => _linkRef(link, group, cells),
      child: FxRowHoverRegion(child: menued),
    );
  }

  /// How much room the row's value boxes take in the Timeline's fold-out.
  ///
  /// **The Modes column is a minimum, not a cage** (owner, desk test). One box
  /// fills the column exactly, as it always has. Two — an unlinked Scale, a
  /// Position — were being squeezed into half of it each, which is narrower
  /// than a value well can hold `100.0%`: the unit dropped to a second line
  /// inside a lane one line tall. A row of more than one box therefore asks
  /// for the width those same boxes are drawn at in the Effect controls panel,
  /// [transformCellWidth] each, and **runs on to the right** rather than
  /// squeezing — into room a property row has going spare anyway, since it
  /// draws no matte, no blend and no parent under those columns.
  ///
  /// Capped at what the outline actually holds ([ValueColumn.rightInset] is
  /// everything to the right of the Modes column), so a narrow outline gives
  /// back the squeeze rather than pushing the boxes off the panel.
  double _valueAreaWidth(ValueColumn col, int cells) {
    final wanted = cells * transformCellWidth + (cells - 1) * 4;
    if (wanted <= col.width) return col.width;
    return math.min(wanted, col.width + col.rightInset);
  }

  Widget _cell(BridgeTransform transform, TransformAxis axis, int frame,
      {bool fixed = true, required TransformGroup group}) {
    if (group.colour) return _swatch(transform, group, frame);
    final scalar = read(transform, axis.prop);
    // The axes this box drags along with it, none when it drags alone.
    final partners = [
      if (group.isLinked)
        for (final other in group.axes)
          if (other.prop != axis.prop) other.prop
    ];

    // An animated property stays editable (docs/07 §4.3): the field shows the
    // value under the playhead, and a change writes it into the key sitting
    // there — or plants a new one — never flattening the curve.

    // No Flexible of its own: two of the three call sites place the cell
    // straight into a Row and wrap it themselves, and the third wraps it in
    // Expanded — a Flexible here fought that Expanded over the same slot and
    // took the whole Reveal fold down with a ParentDataWidget error.
    if (scalar case BridgeScalar_Expression scalar) {
      return EffectParamRowExpression(
          value: scalar,
          row: BridgeExpressionRow.transform(axis.prop),
          set: (value) {
            final field = (value as BridgeEffectValue_Float).field0;

            if (field is BridgeScalar_Expression) {
              _commitExpression(axis.prop, field);
            }

            if (field is BridgeScalar_Static) {
              _commit(axis.prop, field.field0);
            }
          },
          setLive: (value) {
            _liveExpression(
                axis.prop, (value as BridgeEffectValue_Float).field0);
          },
          comp: widget.comp,
          frame: frame,
          layer: widget.layer);
    }

    // A rotation shows its whole turns beside its degrees (docs/07 §6.1): 30°
    // and 390° are the same picture but not the same animation, and a single
    // box cannot say which of the two a key holds. The value written is still
    // the one angle — see `TurnsAndDegreesField`.
    final isRotation = axis.suffix == '°' && axis.turns;

    if (scalar is! BridgeScalar_Keyframed) {
      final static_ = (scalar as BridgeScalar_Static).field0;
      if (isRotation) {
        return TurnsAndDegreesField(
          keyName: '${widget.keyPrefix}-${axis.prop.name}',
          degrees: static_,
          onChanged: (v) => _live(axis.prop, v),
          onCommit: (v) => _commit(axis.prop, v),
          setExpression: () => _commitExpression(
              axis.prop,
              BridgeScalar.expression(
                  static_.toString(), newExpressionLanguage(context))),
        );
      }
      return SizedBox(
        width: fixed ? transformCellWidth : null,
        child: DragValueField(
          key: ValueKey<String>('${widget.keyPrefix}-${axis.prop.name}'),
          value: static_,
          min: axis.min,
          max: axis.max,
          speed: axis.speed,
          decimals: axis.decimals,
          suffix: axis.suffix,
          onChanged: (v) => _commit(axis.prop, v.toDouble(), partners, frame),
          onChangeStart: () => _staged = transform,
          onChangeLive: (v) => _live(axis.prop, v.toDouble(), partners, frame),
          onChangeEnd: (v) =>
              _commit(axis.prop, v.toDouble(), partners, frame),
          onDragCancel: () {
            _clearLive();
            _linkRatios.clear();
            setState(() => _staged = null);
          },
          setExpression: () {
            _commitExpression(
                axis.prop,
                BridgeScalar.expression(
                    static_.toString(), newExpressionLanguage(context)));
          },
        ),
      );
    }

    final time = timeOfFrame(widget.comp, frame);
    var sampled = sampledScalar(scalar, time);
    // An Orientation turns from key to key as one, so its angles read off
    // that turn rather than each off its own curve.
    if (transform.depth case final d? when _isOrientation(axis.prop)) {
      final o = sampledOrientation(
          d.orientationX, d.orientationY, d.orientationZ, time);
      if (o != null) {
        sampled = switch (axis.prop) {
          BridgeTransformProp.orientationX => o.$1,
          BridgeTransformProp.orientationY => o.$2,
          _ => o.$3,
        };
      }
    }
    // No live preview mid-drag: staging a keyframed transform through the
    // static-preview path would lie about the curve. The release commits one
    // op — the key at the playhead updated or planted.
    if (isRotation) {
      return TurnsAndDegreesField(
        keyName: '${widget.keyPrefix}-${axis.prop.name}',
        degrees: sampled,
        onCommit: (v) => _commitKeyed(axis.prop, scalar, v, frame, partners),
      );
    }
    return SizedBox(
      width: fixed ? transformCellWidth : null,
      child: KeyedValueField(
        fieldKey: ValueKey<String>('${widget.keyPrefix}-${axis.prop.name}'),
        value: sampled,
        min: axis.min,
        max: axis.max,
        speed: axis.speed,
        decimals: axis.decimals,
        suffix: axis.suffix,
        onCommit: (v) => _commitKeyed(axis.prop, scalar, v, frame, partners),
        onLive: (v) => _liveKeyed(axis.prop, scalar, v, frame, partners),
        onStart: () => _keyOnDragStart(axis.prop, scalar, sampled, frame),
      ),
    );
  }

  /// A colour row's one cell: a swatch showing the colour under the playhead,
  /// which writes all three channels as one step, into the keys at the
  /// playhead when they are animated.
  Widget _swatch(BridgeTransform transform, TransformGroup group, int frame) {
    final time = timeOfFrame(widget.comp, frame);
    final rgb = [
      for (final axis in group.axes)
        sampledScalar(read(transform, axis.prop), time)
    ];
    List<BridgeScalar> values(PickedColour p) => [
          for (final (i, v) in [p.r, p.g, p.b].indexed)
            scalarWithValueAt(
                read(transform, group.axes[i].prop), v, widget.comp, frame),
        ];
    final exact = PickedColour(rgb[0], rgb[1], rgb[2]);
    return Align(
      alignment: AlignmentDirectional.centerStart,
      child: ColourSwatchButton(
        key: ValueKey<String>(
            '${widget.keyPrefix}-${group.axes.first.prop.name}'),
        colour: exact.clipped,
        // The exact numbers, so an over-white light or an imported value is
        // not rounded to what a screen shows.
        initial: exact,
        scale: ColourScale.unit,
        max: 64,
        onPreview: (p) {
          var staged = transform;
          for (final (i, v) in values(p).indexed) {
            staged = writeScalar(staged, group.axes[i].prop, v);
          }
          final ui = _ui;
          _throttle.request(() => widget.comp.renderFrameWithTransformPreview(
                frame: ui.playheadFrame.value,
                scale: ui.viewerScale,
                layer: widget.layer,
                transform: staged,
              ));
        },
        onPicked: (p) {
          _throttle.cancel();
          widget.layer.setTransforms(
            props: [for (final axis in group.axes) axis.prop],
            values: values(p),
          );
          widget.onChanged();
        },
      ),
    );
  }

  /// The playhead has no key on this property and a drag is starting, so one is
  /// planted there holding the value already showing. Nothing moves —
  /// it is the same value — and the drag then has a key to carry, which is what
  /// makes it visible in the graph as it goes rather than only on release.
  /// Returns whether it planted one.
  bool _keyOnDragStart(
      BridgeTransformProp prop, BridgeScalar scalar, double value, int frame) {
    if (scalar is! BridgeScalar_Keyframed) return false;
    if (scalar.field0
        .any((k) => widget.comp.frameAtTime(time: k.time) == frame)) {
      return false;
    }
    widget.layer.setTransform(
      prop: prop,
      value: scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    widget.onChanged();
    return true;
  }

  /// A tick of a drag on an *animated* property: render the curve the release
  /// will write — the key at the playhead moved, or a linear one planted there
  /// — without writing it. The same patched-clone door a static drag
  /// uses, carrying a whole animation instead of one number.
  void _liveKeyed(BridgeTransformProp prop, BridgeScalar scalar, double value,
      int frame, List<BridgeTransformProp> partners) {
    final written = scalarWithValueAt(scalar, value, widget.comp, frame);
    rowValueDrag.value = RowValueDrag(
      layer: widget.layer.internallayerId.toString(),
      prop: prop.name,
      frame: frame,
      value: value,
      keys: (written as BridgeScalar_Keyframed).field0,
    );
    var staged = writeScalar(widget.transform, prop, written);
    // A linked row previews every axis, or the release would move the
    // picture one last time on a drag that looked finished.
    for (final partner in partners) {
      staged = writeScalar(
        staged,
        partner,
        scalarWithValueAt(read(widget.transform, partner),
            _linked(prop, partner, value, frame), widget.comp, frame),
      );
    }
    // The provisional truth, for anything drawing this layer. A keyed property
    // draws no box today — the boxes want a static value and this one is a
    // curve — so this changes nothing on screen yet; it is published because
    // the contract is "what the drag is doing", not "what the box can use".
    _publishLive(staged);
    final ui = _ui;
    _throttle.request(() => widget.comp.renderFrameWithTransformPreview(
          frame: ui.playheadFrame.value,
          scale: ui.viewerScale,
          layer: widget.layer,
          transform: staged,
        ));
  }

  /// Write `value` into the animated property's key at `frame` (or plant one
  /// there) — one op, one undo step. A linked row writes its partner
  /// axis in the same breath, and `setTransforms` batches the pair so it stays
  /// one step.
  void _commitKeyed(BridgeTransformProp prop, BridgeScalar scalar, double value,
      int frame, List<BridgeTransformProp> partners) {
    // The write is the last word: a held preview tick after it would put the
    // provisional picture back, and the graph reads the document again.
    _throttle.cancel();
    _clearLive();
    rowValueDrag.value = null;
    final written = scalarWithValueAt(scalar, value, widget.comp, frame);
    if (partners.isEmpty) {
      widget.layer.setTransform(prop: prop, value: written);
    } else {
      widget.layer.setTransforms(props: [
        prop,
        ...partners
      ], values: [
        written,
        for (final partner in partners)
          scalarWithValueAt(read(widget.transform, partner),
              _linked(prop, partner, value, frame), widget.comp, frame),
      ]);
    }
    _linkRatios.clear();
    widget.onChanged();
  }

  /// A drag tick: hold the new value locally and render it, without committing.
  void _live(BridgeTransformProp prop, double value,
      [List<BridgeTransformProp> partners = const [], int frame = 0]) {
    var staged = write(_staged ?? widget.transform, prop, value);
    for (final partner in partners) {
      staged = write(staged, partner, _linked(prop, partner, value, frame));
    }
    setState(() => _staged = staged);
    // The wireframe follows the picture: both are drawn from this value until
    // the drag is let go.
    _publishLive(staged);

    final ui = _ui;
    _throttle.request(() => widget.comp.renderFrameWithTransformPreview(
          frame: ui.playheadFrame.value,
          scale: ui.viewerScale,
          layer: widget.layer,
          transform: _staged ?? staged,
        ));
  }

  void _liveExpression(BridgeTransformProp prop, BridgeScalar value) {
    final staged = writeScalar(_staged ?? widget.transform, prop, value);
    setState(() => _staged = staged);
    _publishLive(staged);

    final ui = _ui;
    _throttle.request(() => widget.comp.renderFrameWithTransformPreview(
          frame: ui.playheadFrame.value,
          scale: ui.viewerScale,
          layer: widget.layer,
          transform: _staged ?? staged,
        ));
  }

  /// Release, or a typed value: one op for the one property that changed — or,
  /// on a linked row, one batch for the pair, which is still one step.
  void _commit(BridgeTransformProp prop, double value,
      [List<BridgeTransformProp> partners = const [], int frame = 0]) {
    // The commit is the last word on this gesture: a held preview tick after it
    // would put the provisional picture back.
    _throttle.cancel();
    _clearLive();
    if (partners.isEmpty) {
      widget.layer.setTransform(prop: prop, value: BridgeScalar.static_(value));
    } else {
      widget.layer.setTransforms(props: [
        prop,
        ...partners
      ], values: [
        BridgeScalar.static_(value),
        for (final partner in partners)
          BridgeScalar.static_(_linked(prop, partner, value, frame)),
      ]);
    }
    _linkRatios.clear();
    setState(() => _staged = null);
    widget.onChanged();
  }

  /// The row's axis menu: tell the pair's axes apart, or put them back
  /// together. Scale carries the link as well, because a scale that has stopped
  /// being proportional is nearly always a mistake — so it is a state you leave
  /// on purpose rather than one you fall out of.
  ///
  /// Every entry commits one op or one batch, so each is one undo step.
  List<Widget> _ownRows(TransformGroup group, VoidCallback close) {
    final pair = group.pair;
    if (pair == null) return _orderRows(close);
    final separated = group.mode == BridgeAxisMode.separated;
    void set(BridgeAxisMode mode) {
      close();
      widget.layer.setAxisMode(pair: pair, mode: mode);
      widget.onChanged();
    }

    return [
      if (pair == BridgeTransformPair.scale && !separated)
        MenuRow(
          key: ValueKey<String>('${widget.keyPrefix}-axis-link'),
          onPressed: () => set(group.mode == BridgeAxisMode.linked
              ? BridgeAxisMode.combined
              : BridgeAxisMode.linked),
          child: MenuLabel(group.mode == BridgeAxisMode.linked
              ? l10n.transformUnlinkAxes
              : l10n.transformLinkAxes),
        ),
      MenuRow(
        key: ValueKey<String>('${widget.keyPrefix}-axis-separate'),
        onPressed: () => set(separated
            ? (pair == BridgeTransformPair.scale
                ? BridgeAxisMode.linked
                : BridgeAxisMode.combined)
            : BridgeAxisMode.separated),
        child: MenuLabel(separated
            ? l10n.transformCombineAxes
            : l10n.transformSeparateAxes),
      ),
    ];
  }

  /// What this row's right-click menu offers.
  ParamMenu _menu(TransformGroup group, KeyframeControlsFrb keys,
      List<TransformAxis> cells, BridgeTransform transform) {
    final props = [for (final axis in group.axes) axis.prop];
    final defaults = [for (final p in props) _defaultOf(p)];
    final canDefault = defaults.every((d) => d != null);
    // Position and the Anchor go back where the Layer menu's Reset puts them,
    // worked out on the press. A camera or a light was placed, so has none.
    final placed = canDefault
        ? null
        : _ui.model.heldLayers
            .where((e) =>
                e.layer.internallayerId == widget.layer.internallayerId &&
                e.info.kind != BridgeLayerKind.camera &&
                e.info.kind != BridgeLayerKind.light)
            .firstOrNull;
    final playhead = _ui.playheadFrame;
    double shown(BridgeTransformProp p) => sampledScalar(
        read(widget.transform, p), timeOfFrame(widget.comp, playhead.value));
    final expression = [
      for (final p in props)
        if (read(widget.transform, p) is BridgeScalar_Expression) p,
    ];
    void writeAll(List<double> values) {
      widget.layer.setTransforms(
        props: props,
        values: [
          for (final (i, p) in props.indexed)
            scalarWithValueAt(read(widget.transform, p), values[i],
                widget.comp, playhead.value),
        ],
      );
      widget.onChanged();
    }

    // Seeded with the number showing, so turning one on moves nothing.
    void seed(BridgeTransformProp p) => _commitExpression(
        p,
        BridgeScalar.expression(
            shown(p).toString(), newExpressionLanguage(context)));

    return ParamMenu(
      linkTo: (link) => _linkRef(link, group, cells),
      linkFrom: _link(group, transform),
      own: group.pair != null || _turnsIn3d(group)
          ? (close) => _ownRows(group, close)
          : null,
      keys: expression.isEmpty ? keys : null,
      reset: canDefault
          ? () {
              widget.layer.setTransforms(props: props, values: [
                for (final d in defaults) BridgeScalar.static_(d!),
              ]);
              widget.onChanged();
            }
          : placed == null
              ? null
              : () {
                  final fresh = freshTransform(_ui, placed);
                  if (!props.every(fresh.containsKey)) return;
                  widget.layer.setTransforms(props: props, values: [
                    for (final p in props) BridgeScalar.static_(fresh[p]!),
                  ]);
                  widget.onChanged();
                },
      canReset: !canDefault ||
          [
            for (final (i, p) in props.indexed)
              read(widget.transform, p) is! BridgeScalar_Static ||
                  (shown(p) - defaults[i]!).abs() > 1e-5,
          ].any((differs) => differs),
      copy: expression.isEmpty ? () => copiedAxes(props.map(shown)) : null,
      paste: expression.isNotEmpty
          ? null
          : (text) {
              final n = pastedAxes(text, props.length);
              if (n != null) writeAll(n);
            },
      setExpression: cells.length == 1 && expression.isEmpty && !group.colour
          ? () => seed(props.first)
          : null,
      axisExpressions: cells.length < 2 || group.colour
          ? const []
          : [
              for (final (i, axis) in cells.indexed)
                if (!expression.contains(axis.prop))
                  (axis: axisName(i), set: () => seed(axis.prop)),
            ],
      removeExpression: expression.length != 1
          ? null
          : () => _commit(
              expression.first,
              sampleScalarWithContext(
                  scalar: read(widget.transform, expression.first),
                  time: timeOfFrame(widget.comp, playhead.value),
                  layer: widget.layer)),
      graphPath: expression.isEmpty
          ? transformGroupPath(
              widget.layer.internallayerId.toString(), group)
          : null,
      // No socket on a transform, so Wiggle is written as an expression on
      // every axis of the row, as one op. A tenth of a turn for a rotation, a
      // tenth of the row's own range otherwise.
      drive: group.colour || expression.isNotEmpty
          ? null
          : DriveRow(
              layer: widget.layer,
              wiggle: () {
                final language = newExpressionLanguage(context);
                final amount = _turnsIn3d(group) ||
                        props.first == BridgeTransformProp.rotation
                    ? 36.0
                    : props.first == BridgeTransformProp.opacity ||
                            props.first == BridgeTransformProp.scaleX
                        ? 10.0
                        : 50.0;
                _throttle.cancel();
                _clearLive();
                widget.layer.setTransforms(props: props, values: [
                  for (final (i, p) in props.indexed)
                    BridgeScalar.expression(
                        wiggleExpression(language, shown(p), amount, i),
                        language),
                ]);
                setState(() => _staged = null);
                widget.onChanged();
              },
            ),
    );
  }

  /// What [prop] goes back to, where there is one value for every layer: the
  /// rest depend on the layer's source.
  double? _defaultOf(BridgeTransformProp prop) => switch (prop) {
        BridgeTransformProp.scaleX ||
        BridgeTransformProp.scaleY ||
        BridgeTransformProp.opacity =>
          100,
        BridgeTransformProp.rotation ||
        BridgeTransformProp.rotationX ||
        BridgeTransformProp.rotationY ||
        BridgeTransformProp.positionZ ||
        BridgeTransformProp.orientationX ||
        BridgeTransformProp.orientationY ||
        BridgeTransformProp.orientationZ =>
          0,
        _ => null,
      };

  /// Whether [group] is one of X, Y and Z Rotation on a layer that turns in
  /// 3D, which has a rotation order to choose.
  bool _turnsIn3d(TransformGroup group) =>
      widget.transform.depth != null &&
      const {
        BridgeTransformProp.rotation,
        BridgeTransformProp.rotationX,
        BridgeTransformProp.rotationY,
      }.contains(group.axes.first.prop);

  /// The rotation rows' own row: the rotation order, ticked while the layer
  /// turns in it. A change keeps the pose and is one undo step.
  List<Widget> _orderRows(VoidCallback close) {
    final on = widget.transform.depth?.afterEffectsOrder ?? false;
    return [
      MenuRow(
        key: ValueKey<String>('${widget.keyPrefix}-rotation-order'),
        onPressed: () {
          close();
          try {
            widget.layer.setAfterEffectsOrder(on_: !on);
          } catch (_) {
            // An expression drives a rotation, so there is no pose to keep.
            return;
          }
          widget.onChanged();
        },
        child: MenuLabel(l10n.transformAfterEffectsOrder, ticked: on),
      ),
    ];
  }

  /// What this row is to an expression written elsewhere: one property, or
  /// the whole pair for a row of two.
  PropertyRef? _linkRef(
      PropertyLink link, TransformGroup group, List<TransformAxis> cells) {
    final at = widget.layer.internallayerId;
    if (group.colour || cells.any((c) => link.from.contains((at, c.prop)))) {
      return null;
    }
    final name = _ui.model.byId(at)?.info.name;
    if (name == null) return null;
    return PropertyRef.transform(
      name,
      cells.first.prop,
      whole: cells.length > 1,
      sameLayer: link.layer == widget.layer.internallayerId,
    );
  }

  /// The mark a link is dragged from: the reference lands on every axis of
  /// the row as one op, and the editor opens on the first.
  Widget? _linkMark(TransformGroup group, BridgeTransform transform) {
    final link = _link(group, transform);
    return link == null
        ? null
        : LinkMark(name: engineLabel(group.label), link: link);
  }

  /// What this row's link mark drags, and Paste link writes through.
  PropertyLink? _link(TransformGroup group, BridgeTransform transform) {
    final props = [for (final axis in group.axes) axis.prop];
    if (group.colour ||
        props.any((p) => read(transform, p) is BridgeScalar_Expression)) {
      return null;
    }
    return PropertyLink(
      layer: widget.layer.internallayerId,
      from: {for (final p in props) (widget.layer.internallayerId, p)},
      write: (text, language) {
        _throttle.cancel();
        _clearLive();
        noteSetExpression(select: false);
        widget.layer.setTransforms(props: props, values: [
          for (final _ in props) BridgeScalar.expression(text, language),
        ]);
        setState(() => _staged = null);
        widget.onChanged();
      },
    );
  }

  void _commitExpression(BridgeTransformProp prop, BridgeScalar value) {
    // The commit is the last word on this gesture: a held preview tick after it
    // would put the provisional picture back.
    _throttle.cancel();
    _clearLive();
    widget.layer.setTransform(prop: prop, value: value);
    setState(() => _staged = null);
    widget.onChanged();
  }
}

/// What Copy on a Transform heading put down, curves and all.
Map<BridgeTransformProp, BridgeScalar>? _copiedTransform;

/// The Transform heading's menu, in Effect controls and in the Timeline: a
/// keyframe on every transform property at the playhead, the Layer menu's own
/// Transform rows, and the whole transform as one thing to copy to other
/// layers. A heading outside the selection is about its own layer.
void showTransformMenu(BuildContext context, Offset at, LayerReference layer,
    {required VoidCallback onChanged}) {
  final app = Provider.of<LumitState>(context, listen: false);
  final ui = Provider.of<LumitUiState>(context, listen: false);
  if (!ui.selectedLayerIds.contains(layer.internallayerId)) {
    ui.setSelection([layer]);
  }
  final entry = ui.model.byId(layer.internallayerId);
  if (entry == null) return;
  final id = layer.internallayerId.toString();
  List<TransformGroup> groupsOf(BridgeLayerEntry e) => [
        for (final group in transformGroups(
          threeD: e.info.switches.threeD,
          modes: e.info.axisModes,
          kind: e.info.kind,
          twoNode: e.info.camera?.twoNode ?? false,
          light: e.info.light,
          material: e.info.material,
        ))
          if (group.section == TransformSection.transform) group,
      ];
  final groups = groupsOf(entry);
  final locked = entry.info.switches.locked;
  final channels = graphChannels(
    layers: ui.model.layers,
    selected: [for (final group in groups) transformGroupPath(id, group)],
  );
  final copied = _copiedTransform;
  showMenuEntriesAt(context, at, [
    MenuEntry(
        l10n.tipAddKeyframe,
        channels.isEmpty || locked
            ? null
            : () {
                if (toggleKeysAtPlayhead(app, ui, channels)) onChanged();
              },
        action: 'keyframe.toggle'),
    MenuEntry.divider(),
    ...transformRows(app, ui),
    MenuEntry.divider(),
    MenuEntry(l10n.menuCopy, () {
      _copiedTransform = {
        for (final group in groups)
          for (final axis in group.axes)
            axis.prop: read(entry.info.transform, axis.prop),
      };
    }),
    MenuEntry(
        l10n.menuPaste,
        copied == null
            ? null
            : onSelection(app, ui, (target) {
                // The rows this layer has: a 2D layer takes no z.
                final props = [
                  for (final group in groupsOf(target))
                    for (final axis in group.axes)
                      if (copied.containsKey(axis.prop)) axis.prop,
                ];
                target.layer.setTransforms(
                  props: props,
                  values: [for (final p in props) copied[p]!],
                );
                onChanged();
              })),
  ]);
}

/// What a camera or light channel reads on a transform that carries none -
/// every layer but a camera or a light. A still zero rather than a throw: the
/// rows are built from the kind, so nothing asks unless there is something to
/// answer with.
const BridgeScalar _noChannel = BridgeScalar.static_(0);

bool _isOrientation(BridgeTransformProp prop) =>
    prop == BridgeTransformProp.orientationX ||
    prop == BridgeTransformProp.orientationY ||
    prop == BridgeTransformProp.orientationZ;

/// One property out of a transform.
BridgeScalar read(BridgeTransform tf, BridgeTransformProp prop) =>
    switch (prop) {
      BridgeTransformProp.anchorX => tf.anchorX,
      BridgeTransformProp.anchorY => tf.anchorY,
      BridgeTransformProp.positionX => tf.positionX,
      BridgeTransformProp.positionY => tf.positionY,
      BridgeTransformProp.positionZ => tf.positionZ,
      BridgeTransformProp.scaleX => tf.scaleX,
      BridgeTransformProp.scaleY => tf.scaleY,
      BridgeTransformProp.rotation => tf.rotation,
      BridgeTransformProp.rotationX => tf.rotationX,
      BridgeTransformProp.rotationY => tf.rotationY,
      BridgeTransformProp.opacity => tf.opacity,
      BridgeTransformProp.orientationX => tf.depth?.orientationX ?? _noChannel,
      BridgeTransformProp.orientationY => tf.depth?.orientationY ?? _noChannel,
      BridgeTransformProp.orientationZ => tf.depth?.orientationZ ?? _noChannel,
      BridgeTransformProp.anchorZ => tf.depth?.anchorZ ?? _noChannel,
      BridgeTransformProp.scaleZ =>
        tf.depth?.scaleZ ?? const BridgeScalar.static_(100),
      BridgeTransformProp.poiX =>
        tf.camera?.poiX ?? tf.light?.poiX ?? _noChannel,
      BridgeTransformProp.poiY =>
        tf.camera?.poiY ?? tf.light?.poiY ?? _noChannel,
      BridgeTransformProp.poiZ =>
        tf.camera?.poiZ ?? tf.light?.poiZ ?? _noChannel,
      BridgeTransformProp.zoom => tf.camera?.zoom ?? _noChannel,
      BridgeTransformProp.focusDistance =>
        tf.camera?.focusDistance ?? _noChannel,
      BridgeTransformProp.aperture => tf.camera?.aperture ?? _noChannel,
      BridgeTransformProp.blurLevel => tf.camera?.blurLevel ?? _noChannel,
      BridgeTransformProp.lightIntensity => tf.light?.intensity ?? _noChannel,
      BridgeTransformProp.lightColourR => tf.light?.colourR ?? _noChannel,
      BridgeTransformProp.lightColourG => tf.light?.colourG ?? _noChannel,
      BridgeTransformProp.lightColourB => tf.light?.colourB ?? _noChannel,
      BridgeTransformProp.coneAngle => tf.light?.coneAngle ?? _noChannel,
      BridgeTransformProp.coneFeather => tf.light?.coneFeather ?? _noChannel,
      BridgeTransformProp.lightRadius => tf.light?.radius ?? _noChannel,
      BridgeTransformProp.falloffDistance =>
        tf.light?.falloffDistance ?? _noChannel,
      BridgeTransformProp.lightWidth => tf.light?.width ?? _noChannel,
      BridgeTransformProp.lightHeight => tf.light?.height ?? _noChannel,
      BridgeTransformProp.shadowDarkness =>
        tf.light?.shadowDarkness ?? _noChannel,
      BridgeTransformProp.shadowDiffusion =>
        tf.light?.shadowDiffusion ?? _noChannel,
      BridgeTransformProp.hazeAmount => tf.light?.hazeAmount ?? _noChannel,
      BridgeTransformProp.hazeDensity => tf.light?.hazeDensity ?? _noChannel,
      BridgeTransformProp.hazeColourR => tf.light?.hazeColourR ?? _noChannel,
      BridgeTransformProp.hazeColourG => tf.light?.hazeColourG ?? _noChannel,
      BridgeTransformProp.hazeColourB => tf.light?.hazeColourB ?? _noChannel,
      BridgeTransformProp.hazeFalloff => tf.light?.hazeFalloff ?? _noChannel,
      BridgeTransformProp.hazeScatter => tf.light?.hazeScatter ?? _noChannel,
      BridgeTransformProp.hazeNoise => tf.light?.hazeNoise ?? _noChannel,
      BridgeTransformProp.hazeNoiseSize =>
        tf.light?.hazeNoiseSize ?? _noChannel,
      BridgeTransformProp.hazeNoiseSpeed =>
        tf.light?.hazeNoiseSpeed ?? _noChannel,
      BridgeTransformProp.backgroundBlur =>
        tf.light?.backgroundBlur ?? _noChannel,
      BridgeTransformProp.ambient => tf.material?.ambient ?? _noChannel,
      BridgeTransformProp.diffuse => tf.material?.diffuse ?? _noChannel,
      BridgeTransformProp.specular => tf.material?.specular ?? _noChannel,
      BridgeTransformProp.shininess => tf.material?.shininess ?? _noChannel,
      BridgeTransformProp.metal => tf.material?.metal ?? _noChannel,
      BridgeTransformProp.lightTransmission =>
        tf.material?.lightTransmission ?? _noChannel,
      BridgeTransformProp.reflection => tf.material?.reflection ?? _noChannel,
    };

/// A copy of `tf` with one property replaced — what the preview renders.
///
/// Rebuilt field by field because the generated type has no `copyWith`: it is a
/// plain data class across the seam, which is the point of it.
BridgeTransform write(
        BridgeTransform tf, BridgeTransformProp prop, double value) =>
    writeScalar(tf, prop, BridgeScalar.static_(value));

/// A copy of `tf` with one property's whole animation replaced — what a graph
/// drag previews, where the provisional value is a curve rather than a number.
BridgeTransform writeScalar(
    BridgeTransform tf, BridgeTransformProp prop, BridgeScalar replacement) {
  BridgeScalar pick(BridgeTransformProp p, BridgeScalar current) =>
      p == prop ? replacement : current;

  final camera = tf.camera;
  final light = tf.light;
  final material = tf.material;
  final depth = tf.depth;
  return BridgeTransform(
    orient: tf.orient,
    anchorX: pick(BridgeTransformProp.anchorX, tf.anchorX),
    anchorY: pick(BridgeTransformProp.anchorY, tf.anchorY),
    positionX: pick(BridgeTransformProp.positionX, tf.positionX),
    positionY: pick(BridgeTransformProp.positionY, tf.positionY),
    positionZ: pick(BridgeTransformProp.positionZ, tf.positionZ),
    scaleX: pick(BridgeTransformProp.scaleX, tf.scaleX),
    scaleY: pick(BridgeTransformProp.scaleY, tf.scaleY),
    rotation: pick(BridgeTransformProp.rotation, tf.rotation),
    rotationX: pick(BridgeTransformProp.rotationX, tf.rotationX),
    rotationY: pick(BridgeTransformProp.rotationY, tf.rotationY),
    opacity: pick(BridgeTransformProp.opacity, tf.opacity),
    // A layer that is not a camera has no channels to replace one in, and a
    // camera prop never reaches it: the rows come from the kind.
    camera: camera == null
        ? null
        : BridgeCameraChannels(
            poiX: pick(BridgeTransformProp.poiX, camera.poiX),
            poiY: pick(BridgeTransformProp.poiY, camera.poiY),
            poiZ: pick(BridgeTransformProp.poiZ, camera.poiZ),
            zoom: pick(BridgeTransformProp.zoom, camera.zoom),
            focusDistance:
                pick(BridgeTransformProp.focusDistance, camera.focusDistance),
            aperture: pick(BridgeTransformProp.aperture, camera.aperture),
            blurLevel: pick(BridgeTransformProp.blurLevel, camera.blurLevel),
          ),
    light: light == null
        ? null
        : BridgeLightChannels(
            poiX: pick(BridgeTransformProp.poiX, light.poiX),
            poiY: pick(BridgeTransformProp.poiY, light.poiY),
            poiZ: pick(BridgeTransformProp.poiZ, light.poiZ),
            intensity: pick(BridgeTransformProp.lightIntensity, light.intensity),
            colourR: pick(BridgeTransformProp.lightColourR, light.colourR),
            colourG: pick(BridgeTransformProp.lightColourG, light.colourG),
            colourB: pick(BridgeTransformProp.lightColourB, light.colourB),
            coneAngle: pick(BridgeTransformProp.coneAngle, light.coneAngle),
            coneFeather:
                pick(BridgeTransformProp.coneFeather, light.coneFeather),
            radius: pick(BridgeTransformProp.lightRadius, light.radius),
            falloffDistance:
                pick(BridgeTransformProp.falloffDistance, light.falloffDistance),
            width: pick(BridgeTransformProp.lightWidth, light.width),
            height: pick(BridgeTransformProp.lightHeight, light.height),
            shadowDarkness:
                pick(BridgeTransformProp.shadowDarkness, light.shadowDarkness),
            shadowDiffusion:
                pick(BridgeTransformProp.shadowDiffusion, light.shadowDiffusion),
            hazeAmount: pick(BridgeTransformProp.hazeAmount, light.hazeAmount),
            hazeDensity:
                pick(BridgeTransformProp.hazeDensity, light.hazeDensity),
            hazeColourR:
                pick(BridgeTransformProp.hazeColourR, light.hazeColourR),
            hazeColourG:
                pick(BridgeTransformProp.hazeColourG, light.hazeColourG),
            hazeColourB:
                pick(BridgeTransformProp.hazeColourB, light.hazeColourB),
            hazeFalloff:
                pick(BridgeTransformProp.hazeFalloff, light.hazeFalloff),
            hazeScatter:
                pick(BridgeTransformProp.hazeScatter, light.hazeScatter),
            hazeNoise: pick(BridgeTransformProp.hazeNoise, light.hazeNoise),
            hazeNoiseSize:
                pick(BridgeTransformProp.hazeNoiseSize, light.hazeNoiseSize),
            hazeNoiseSpeed:
                pick(BridgeTransformProp.hazeNoiseSpeed, light.hazeNoiseSpeed),
            backgroundBlur:
                pick(BridgeTransformProp.backgroundBlur, light.backgroundBlur),
          ),
    material: material == null
        ? null
        : BridgeMaterialChannels(
            ambient: pick(BridgeTransformProp.ambient, material.ambient),
            diffuse: pick(BridgeTransformProp.diffuse, material.diffuse),
            specular: pick(BridgeTransformProp.specular, material.specular),
            shininess: pick(BridgeTransformProp.shininess, material.shininess),
            metal: pick(BridgeTransformProp.metal, material.metal),
            lightTransmission: pick(BridgeTransformProp.lightTransmission,
                material.lightTransmission),
            reflection: pick(BridgeTransformProp.reflection, material.reflection),
          ),
    depth: depth == null
        ? null
        : BridgeDepthChannels(
            orientationX:
                pick(BridgeTransformProp.orientationX, depth.orientationX),
            orientationY:
                pick(BridgeTransformProp.orientationY, depth.orientationY),
            orientationZ:
                pick(BridgeTransformProp.orientationZ, depth.orientationZ),
            anchorZ: pick(BridgeTransformProp.anchorZ, depth.anchorZ),
            scaleZ: pick(BridgeTransformProp.scaleZ, depth.scaleZ),
            afterEffectsOrder: depth.afterEffectsOrder,
          ),
  );
}

