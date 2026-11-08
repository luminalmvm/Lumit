// The rows of a layer that are a choice rather than a number: a light's type,
// falloff and shadows, and how a 3D layer takes lights and shadows. They sit
// at the head of their section, above the keyframed rows, in both the
// Timeline's fold-out and the Effect controls card.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/orient.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/ui_state.dart';
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import '../widgets/layer_picker.dart';
import 'fx_section.dart';
import 'transform_rows_frb.dart';

/// What a layer's Orient can be. Towards camera is for a 3D layer, and stays on
/// the list of one that has it, so the row can always say what it holds.
List<BridgeOrient> orientChoices({
  required bool threeD,
  required BridgeLayerKind kind,
  BridgeOrient now = BridgeOrient.off,
}) =>
    [
      BridgeOrient.off,
      BridgeOrient.alongPath,
      if ((threeD && kind != BridgeLayerKind.camera) ||
          now == BridgeOrient.towardsCamera)
        BridgeOrient.towardsCamera,
    ];

String orientLabel(BridgeOrient orient) => switch (orient) {
      BridgeOrient.off => l10n.off,
      BridgeOrient.alongPath => l10n.orientAlongPath,
      BridgeOrient.towardsCamera => l10n.orientTowardsCamera,
    };

/// One choice row.
enum LayerSetting {
  orient,
  lightType,
  lightSource,
  lightBackground,
  lightFalloff,
  lightCastsShadows,
  castsShadows,
  acceptsShadows,
  holdout,
  acceptsLights;

  /// The heading the row sits under.
  TransformSection get section => switch (this) {
        orient => TransformSection.transform,
        lightType ||
        lightSource ||
        lightBackground ||
        lightFalloff ||
        lightCastsShadows =>
          TransformSection.light,
        castsShadows || acceptsShadows || holdout || acceptsLights =>
          TransformSection.material,
      };

  String get label => switch (this) {
        orient => l10n.transformOrient,
        lightType => l10n.lightType,
        lightSource => l10n.lightSource,
        lightBackground => l10n.lightShowInBackground,
        lightFalloff => l10n.lightFalloff,
        lightCastsShadows || castsShadows => l10n.castsShadows,
        acceptsShadows => l10n.acceptsShadows,
        holdout => l10n.holdout,
        acceptsLights => l10n.switchAcceptsLights,
      };
}

/// What a light reads when the read model has none to give, which is never
/// for a light: the shape of a fresh one.
const BridgeLightSettings plainLight = BridgeLightSettings(
  kind: BridgeLightKind.point,
  falloff: BridgeLightFalloff.none,
  twoNode: false,
  castsShadows: false,
  inAir: false,
  noisy: false,
  source: null,
  background: false,
);

/// The same for a material, on After Effects' defaults.
const BridgeMaterialSettings plainMaterial = BridgeMaterialSettings(
  castsShadows: BridgeShadowMode.off,
  acceptsShadows: BridgeShadowMode.on_,
  holdout: false,
);

/// The choice rows a layer shows, in order.
///
/// Falloff is only for the kinds that are somewhere: a parallel light is too
/// far away to fall off, and an ambient one is not anywhere at all.
List<LayerSetting> layerSettings({
  required BridgeLayerKind kind,
  BridgeLightSettings? light,
  bool threeD = false,
  bool twoNode = false,
}) {
  if (kind != BridgeLayerKind.light) {
    return [
      // A two-node camera is aimed already, and sound has nothing to turn.
      if (!twoNode &&
          kind != BridgeLayerKind.audio &&
          kind != BridgeLayerKind.bus)
        LayerSetting.orient,
      if (threeD &&
          kind != BridgeLayerKind.camera &&
          kind != BridgeLayerKind.nullLayer) ...[
        LayerSetting.castsShadows,
        LayerSetting.acceptsShadows,
        if (kind == BridgeLayerKind.model || kind == BridgeLayerKind.mesh)
          LayerSetting.holdout,
        LayerSetting.acceptsLights,
      ],
    ];
  }
  final s = light ?? plainLight;
  final surround = s.kind == BridgeLightKind.environment;
  return [
    LayerSetting.lightType,
    if (surround) ...[LayerSetting.lightSource, LayerSetting.lightBackground],
    if (lightFallsOff(s.kind)) LayerSetting.lightFalloff,
    // Ambient and environment light come from no one place, so they have
    // nowhere to cast from.
    if (lightCasts(s.kind)) LayerSetting.lightCastsShadows,
  ];
}

/// Whether layer [l] has a picture an Environment light can read. A model, a
/// light or an adjustment doesn't, whatever else it draws.
bool canSurround(BridgeLayerEntry l) => switch (l.info.kind) {
      BridgeLayerKind.footage ||
      BridgeLayerKind.sequence ||
      BridgeLayerKind.solid ||
      BridgeLayerKind.precomp ||
      BridgeLayerKind.text ||
      BridgeLayerKind.shape =>
        l.layer.hasPicture(),
      _ => false,
    };

/// Whether layer [id] is an Environment light's picture, and so never drawn.
bool feedsEnvironment(List<BridgeLayerEntry> stack, UuidValue id) =>
    stack.any((e) =>
        e.info.light?.kind == BridgeLightKind.environment &&
        e.info.light?.source == id);

/// Whether a light of this kind weakens with distance.
bool lightFallsOff(BridgeLightKind kind) =>
    kind == BridgeLightKind.point ||
    kind == BridgeLightKind.spot ||
    kind == BridgeLightKind.area;

/// Whether a light of this kind comes from somewhere a shadow can be cast
/// from.
bool lightCasts(BridgeLightKind kind) =>
    kind != BridgeLightKind.ambient && kind != BridgeLightKind.environment;

/// Whether a light of this kind has a direction to aim.
bool lightAims(BridgeLightKind kind) =>
    kind == BridgeLightKind.spot ||
    kind == BridgeLightKind.parallel ||
    kind == BridgeLightKind.area;

String lightKindLabel(BridgeLightKind kind) => switch (kind) {
      BridgeLightKind.point => l10n.lightPoint,
      BridgeLightKind.spot => l10n.lightSpot,
      BridgeLightKind.area => l10n.lightArea,
      BridgeLightKind.parallel => l10n.lightParallel,
      BridgeLightKind.ambient => l10n.lightAmbient,
      BridgeLightKind.environment => l10n.lightEnvironment,
    };

String shadowModeLabel(BridgeShadowMode mode) => switch (mode) {
      BridgeShadowMode.off => l10n.off,
      BridgeShadowMode.on_ => l10n.shadowsOn,
      BridgeShadowMode.only => l10n.shadowsOnly,
    };

String lightFalloffLabel(BridgeLightFalloff falloff) => switch (falloff) {
      BridgeLightFalloff.none => l10n.none,
      BridgeLightFalloff.smooth => l10n.lightFalloffSmooth,
      BridgeLightFalloff.inverseSquareClamped =>
        l10n.lightFalloffInverseSquare,
    };

/// The same settings with one thing changed.
BridgeLightSettings lightSettingsWith(
  BridgeLightSettings s, {
  BridgeLightKind? kind,
  BridgeLightFalloff? falloff,
  bool? twoNode,
  bool? castsShadows,
  (UuidValue?,)? source,
  bool? background,
}) =>
    BridgeLightSettings(
      kind: kind ?? s.kind,
      falloff: falloff ?? s.falloff,
      twoNode: twoNode ?? s.twoNode,
      castsShadows: castsShadows ?? s.castsShadows,
      inAir: s.inAir,
      noisy: s.noisy,
      source: source == null ? s.source : source.$1,
      background: background ?? s.background,
    );

/// One choice row. Each pick is one op, so one undo step.
class LayerSettingRowFrb extends StatelessWidget {
  final LayerReference layer;
  final LayerSetting setting;
  final BridgeLightSettings? light;
  final BridgeMaterialSettings? material;
  final bool acceptsLights;
  final VoidCallback onChanged;

  /// The layer's Orient, and what decides which choices it has.
  final BridgeOrient orient;
  final bool threeD;
  final BridgeLayerKind kind;

  /// The Timeline's value column, or null in the Effect controls panel, which
  /// lays the row out as its two columns.
  final ValueColumn? valueColumn;
  final String keyPrefix;

  const LayerSettingRowFrb({
    super.key,
    required this.layer,
    required this.setting,
    required this.light,
    this.material,
    this.acceptsLights = true,
    required this.onChanged,
    this.orient = BridgeOrient.off,
    this.threeD = false,
    this.kind = BridgeLayerKind.footage,
    this.valueColumn,
    this.keyPrefix = 'tf',
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final s = light ?? plainLight;
    final m = material ?? plainMaterial;
    void shadows(
        {BridgeShadowMode? casts, BridgeShadowMode? accepts, bool? holdout}) {
      layer.setMaterialSettings(
        settings: BridgeMaterialSettings(
          castsShadows: casts ?? m.castsShadows,
          acceptsShadows: accepts ?? m.acceptsShadows,
          holdout: holdout ?? m.holdout,
        ),
      );
      onChanged();
    }

    void write(BridgeLightSettings next) {
      layer.setLightSettings(settings: next);
      onChanged();
    }

    final key = ValueKey<String>('$keyPrefix-${setting.name}');
    final control = switch (setting) {
      LayerSetting.orient => BareDropdown<BridgeOrient>(
          key: key,
          dense: valueColumn != null,
          value: orient,
          options: orientChoices(threeD: threeD, kind: kind, now: orient),
          label: orientLabel,
          onChanged: (o) {
            layer.setOrient(orient: o);
            onChanged();
          },
        ),
      LayerSetting.lightType => BareDropdown<BridgeLightKind>(
          key: key,
          dense: valueColumn != null,
          value: s.kind,
          options: BridgeLightKind.values,
          label: lightKindLabel,
          onChanged: (k) => write(lightSettingsWith(s, kind: k)),
        ),
      // Any layer of the comp with a picture, read as a panorama.
      LayerSetting.lightSource => LayerPicker(
          key: key,
          dense: valueColumn != null,
          layers: Provider.of<LumitUiState>(context, listen: false)
              .model
              .layers,
          value: s.source,
          empty: l10n.none,
          missing: l10n.missingLayer,
          offered: canSurround,
          onChanged: (id) => write(lightSettingsWith(s, source: (id,))),
        ),
      LayerSetting.lightBackground => Align(
          alignment: Alignment.centerLeft,
          child: HouseCheckbox(
            key: key,
            value: s.background,
            onChanged: (on) => write(lightSettingsWith(s, background: on)),
          ),
        ),
      LayerSetting.lightFalloff => BareDropdown<BridgeLightFalloff>(
          key: key,
          dense: valueColumn != null,
          value: s.falloff,
          options: BridgeLightFalloff.values,
          label: lightFalloffLabel,
          onChanged: (f) => write(lightSettingsWith(s, falloff: f)),
        ),
      LayerSetting.lightCastsShadows => Align(
          alignment: AlignmentDirectional.centerStart,
          child: HouseCheckbox(
            key: key,
            value: s.castsShadows,
            onChanged: (on) => write(lightSettingsWith(s, castsShadows: on)),
          ),
        ),
      LayerSetting.castsShadows => BareDropdown<BridgeShadowMode>(
          key: key,
          dense: valueColumn != null,
          value: m.castsShadows,
          options: BridgeShadowMode.values,
          label: shadowModeLabel,
          onChanged: (v) => shadows(casts: v),
        ),
      LayerSetting.acceptsShadows => BareDropdown<BridgeShadowMode>(
          key: key,
          dense: valueColumn != null,
          value: m.acceptsShadows,
          options: BridgeShadowMode.values,
          label: shadowModeLabel,
          onChanged: (v) => shadows(accepts: v),
        ),
      LayerSetting.holdout => Align(
          alignment: AlignmentDirectional.centerStart,
          child: HouseCheckbox(
            key: key,
            value: m.holdout,
            onChanged: (on) => shadows(holdout: on),
          ),
        ),
      // The same switch as the layer's right-click item, one op a click.
      LayerSetting.acceptsLights => Align(
          alignment: AlignmentDirectional.centerStart,
          child: HouseCheckbox(
            key: key,
            value: acceptsLights,
            onChanged: (on) {
              layer.setSwitch(
                  switch_: BridgeLayerSwitch.acceptsLights, on_: on);
              onChanged();
            },
          ),
        ),
    };
    final name = Text(t.propertyCase(setting.label),
        style: t.body, overflow: TextOverflow.ellipsis);
    final col = valueColumn;
    if (col == null) {
      return fxTwoColumnRow(context: context, name: name, control: control);
    }
    return Row(
      children: [
        // Where a stopwatch's button and the gap after it would be, so the
        // name lines up with the keyframed rows'.
        const SizedBox(width: fxKeyframeGutter + 10),
        Expanded(child: name),
        SizedBox(
          width: col.width,
          child: Align(alignment: AlignmentDirectional.centerStart, child: control),
        ),
        SizedBox(width: col.rightInset),
      ],
    );
  }
}
