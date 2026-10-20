// The rows of a layer that are a choice rather than a number: a light's type
// and falloff. They sit at the head of their section, above the keyframed
// rows, in both the Timeline's fold-out and the Effect controls card.

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
  lightFalloff;

  /// The heading the row sits under.
  TransformSection get section => TransformSection.light;

  String get label => switch (this) {
        lightType => l10n.lightType,
        lightFalloff => l10n.lightFalloff,
      };
}

/// What a light reads when the read model has none to give, which is never
/// for a light: the shape of a fresh one.
const BridgeLightSettings plainLight = BridgeLightSettings(
  kind: BridgeLightKind.point,
  falloff: BridgeLightFalloff.none,
  twoNode: false,
);

/// The choice rows a layer shows, in order.
///
/// Falloff is only for the kinds that are somewhere: a parallel light is too
/// far away to fall off, and an ambient one is not anywhere at all.
List<LayerSetting> layerSettings({
  required BridgeLayerKind kind,
  BridgeLightSettings? light,
}) {
  if (kind != BridgeLayerKind.light) return const [];
  final s = light ?? plainLight;
  return [
    LayerSetting.lightType,
    if (lightFallsOff(s.kind)) LayerSetting.lightFalloff,
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
}) =>
    BridgeLightSettings(
      kind: kind ?? s.kind,
      falloff: falloff ?? s.falloff,
      twoNode: twoNode ?? s.twoNode,
    );

/// One choice row. Each pick is one op, so one undo step.
class LayerSettingRowFrb extends StatelessWidget {
  final LayerReference layer;
  final LayerSetting setting;
  final BridgeLightSettings? light;
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
    required this.onChanged,
    this.valueColumn,
    this.keyPrefix = 'tf',
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final s = light ?? plainLight;
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
