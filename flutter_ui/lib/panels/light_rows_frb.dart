// The rows of a layer that are a choice rather than a number: a light's type,
// falloff and shadows, and how a 3D layer takes lights and shadows. They sit
// at the head of their section, above the keyframed rows, in both the
// Timeline's fold-out and the Effect controls card.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import 'fx_section.dart';
import 'transform_rows_frb.dart';

/// One choice row.
enum LayerSetting {
  lightType,
  lightFalloff,
  lightCastsShadows,
  castsShadows,
  acceptsShadows,
  acceptsLights;

  /// The heading the row sits under.
  TransformSection get section => switch (this) {
        lightType || lightFalloff || lightCastsShadows =>
          TransformSection.light,
        castsShadows || acceptsShadows || acceptsLights =>
          TransformSection.material,
      };

  String get label => switch (this) {
        lightType => l10n.lightType,
        lightFalloff => l10n.lightFalloff,
        lightCastsShadows || castsShadows => l10n.castsShadows,
        acceptsShadows => l10n.acceptsShadows,
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
);

/// The same for a material, on After Effects' defaults.
const BridgeMaterialSettings plainMaterial = BridgeMaterialSettings(
  castsShadows: BridgeShadowMode.off,
  acceptsShadows: BridgeShadowMode.on_,
);

/// The choice rows a layer shows, in order.
///
/// Falloff is only for the kinds that are somewhere: a parallel light is too
/// far away to fall off, and an ambient one is not anywhere at all.
List<LayerSetting> layerSettings({
  required BridgeLayerKind kind,
  BridgeLightSettings? light,
  bool threeD = false,
}) {
  if (kind != BridgeLayerKind.light) {
    return [
      if (threeD &&
          kind != BridgeLayerKind.camera &&
          kind != BridgeLayerKind.nullLayer) ...[
        LayerSetting.castsShadows,
        LayerSetting.acceptsShadows,
        LayerSetting.acceptsLights,
      ],
    ];
  }
  final s = light ?? plainLight;
  return [
    LayerSetting.lightType,
    if (lightFallsOff(s.kind)) LayerSetting.lightFalloff,
    // An ambient light comes from nowhere, so it has nowhere to cast from.
    if (s.kind != BridgeLightKind.ambient) LayerSetting.lightCastsShadows,
  ];
}

/// Whether a light of this kind weakens with distance.
bool lightFallsOff(BridgeLightKind kind) =>
    kind == BridgeLightKind.point ||
    kind == BridgeLightKind.spot ||
    kind == BridgeLightKind.area;

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
}) =>
    BridgeLightSettings(
      kind: kind ?? s.kind,
      falloff: falloff ?? s.falloff,
      twoNode: twoNode ?? s.twoNode,
      castsShadows: castsShadows ?? s.castsShadows,
    );

/// One choice row. Each pick is one op, so one undo step.
class LayerSettingRowFrb extends StatelessWidget {
  final LayerReference layer;
  final LayerSetting setting;
  final BridgeLightSettings? light;
  final BridgeMaterialSettings? material;
  final bool acceptsLights;
  final VoidCallback onChanged;

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
    this.valueColumn,
    this.keyPrefix = 'tf',
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final s = light ?? plainLight;
    final m = material ?? plainMaterial;
    void shadows({BridgeShadowMode? casts, BridgeShadowMode? accepts}) {
      layer.setMaterialSettings(
        settings: BridgeMaterialSettings(
          castsShadows: casts ?? m.castsShadows,
          acceptsShadows: accepts ?? m.acceptsShadows,
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
      LayerSetting.lightType => BareDropdown<BridgeLightKind>(
          key: key,
          dense: valueColumn != null,
          value: s.kind,
          options: BridgeLightKind.values,
          label: lightKindLabel,
          onChanged: (k) => write(lightSettingsWith(s, kind: k)),
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
          alignment: Alignment.centerLeft,
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
      // The same switch as the layer's right-click item, one op a click.
      LayerSetting.acceptsLights => Align(
          alignment: Alignment.centerLeft,
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
          child: Align(alignment: Alignment.centerLeft, child: control),
        ),
        SizedBox(width: col.rightInset),
      ],
    );
  }
}
