// Layer ▸ New ▸ Light… and Layer ▸ Light settings: a light's type and its
// options in one window, as After Effects asks for them when a light is made.
//
// The shape is the Camera settings dialog's: a strip of rows, a section under a
// rule, Cancel and OK at the foot, and nothing written until the button is
// pressed. The rows follow the type, so a point light offers no cone.
//
// **An animated channel is read, not written**, as in the camera's window: a
// keyed intensity says so where its well would be.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../panels/layer_fold_frb.dart' show asOneUndoStep;
import '../panels/light_rows_frb.dart';
import '../state/app_state.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'camera_settings_dialog.dart'
    show
        cameraDialogWidth,
        cameraLabelColumn,
        cameraRowGap,
        cameraRowHeight,
        cameraSectionKicker,
        cameraSectionTop,
        cameraWell,
        staticCameraValue;
import 'dialog_frame.dart';

/// What the window edits: the settings, and the still value of each channel,
/// or null where the channel is animated and the window leaves it alone.
class _LightValues {
  BridgeLightSettings settings;
  double? intensity;
  List<double>? colour;
  double? coneAngle;
  double? coneFeather;
  double? radius;
  double? falloffDistance;
  double? width;
  double? height;

  _LightValues({
    required this.settings,
    required this.intensity,
    required this.colour,
    required this.coneAngle,
    required this.coneFeather,
    required this.radius,
    required this.falloffDistance,
    required this.width,
    required this.height,
  });

  _LightValues.of(BridgeLightSettings s, BridgeLightChannels c)
      : settings = s,
        intensity = staticCameraValue(c.intensity),
        colour = _stillColour(c),
        coneAngle = staticCameraValue(c.coneAngle),
        coneFeather = staticCameraValue(c.coneFeather),
        radius = staticCameraValue(c.radius),
        falloffDistance = staticCameraValue(c.falloffDistance),
        width = staticCameraValue(c.width),
        height = staticCameraValue(c.height);

  _LightValues copy() => _LightValues(
        settings: settings,
        intensity: intensity,
        colour: colour == null ? null : [...colour!],
        coneAngle: coneAngle,
        coneFeather: coneFeather,
        radius: radius,
        falloffDistance: falloffDistance,
        width: width,
        height: height,
      );

  static List<double>? _stillColour(BridgeLightChannels c) {
    final rgb = [
      staticCameraValue(c.colourR),
      staticCameraValue(c.colourG),
      staticCameraValue(c.colourB),
    ];
    return rgb.any((v) => v == null) ? null : [for (final v in rgb) v!];
  }

  /// The channels these values write, beside what the layer held before.
  /// Only the ones that moved: a still channel left alone is not rewritten,
  /// and an animated one is never among them.
  (List<BridgeTransformProp>, List<BridgeScalar>) writes(
      BridgeLightChannels? was) {
    final props = <BridgeTransformProp>[];
    final values = <BridgeScalar>[];
    void put(BridgeTransformProp prop, double? now, BridgeScalar? before) {
      if (now == null) return;
      if (before is BridgeScalar_Static && before.field0 == now) return;
      props.add(prop);
      values.add(BridgeScalar.static_(now));
    }

    put(BridgeTransformProp.lightIntensity, intensity, was?.intensity);
    put(BridgeTransformProp.lightColourR, colour?[0], was?.colourR);
    put(BridgeTransformProp.lightColourG, colour?[1], was?.colourG);
    put(BridgeTransformProp.lightColourB, colour?[2], was?.colourB);
    put(BridgeTransformProp.coneAngle, coneAngle, was?.coneAngle);
    put(BridgeTransformProp.coneFeather, coneFeather, was?.coneFeather);
    put(BridgeTransformProp.lightRadius, radius, was?.radius);
    put(BridgeTransformProp.falloffDistance, falloffDistance,
        was?.falloffDistance);
    put(BridgeTransformProp.lightWidth, width, was?.width);
    put(BridgeTransformProp.lightHeight, height, was?.height);
    return (props, values);
  }
}

/// What the last light made with this window was, so the next one starts
/// there, as After Effects does. Null until one has been made.
_LightValues? _lastNew;

/// Layer ▸ New ▸ Light…: ask for the light, then make it directly above
/// [row] as one undo step. Does nothing when cancelled.
Future<void> newLightFrb({
  required BuildContext context,
  required CompositionReference comp,
  int? row,
  VoidCallback? onDone,
}) async {
  final project = Provider.of<LumitState>(context, listen: false).project;
  final size = comp.getSettings();
  final (w, h) = (size.width.toDouble(), size.height.toDouble());
  final start = _lastNew?.copy() ??
      _LightValues(
        settings: const BridgeLightSettings(
          kind: BridgeLightKind.spot,
          falloff: BridgeLightFalloff.none,
          twoNode: true,
        ),
        intensity: 100,
        colour: [1, 1, 1],
        coneAngle: 90,
        coneFeather: 50,
        radius: 500,
        falloffDistance: 500,
        width: w * 0.2,
        height: h * 0.2,
      );
  final picked = await showLumitModal<(_LightValues, String)>(
    context: context,
    id: 'light-settings',
    builder: (close) =>
        _LightSettingsBody(values: start, name: null, onDone: close),
  );
  if (picked == null) return;
  final (values, name) = picked;
  _lastNew = values.copy();
  asOneUndoStep(project, () {
    final layer = comp.addLightLayer(
      kind: values.settings.kind.index,
      row: row,
    );
    layer.setLightSettings(settings: values.settings);
    final (props, scalars) = values.writes(null);
    layer.setTransforms(props: props, values: scalars);
    if (name.isNotEmpty) layer.rename(name: name);
  });
  onDone?.call();
}

/// Layer ▸ Light settings: the same window over a light that is already
/// there. Returns true when the button was pressed.
Future<bool> showLightSettingsFrb({
  required BuildContext context,
  required LayerReference layer,
  required String name,
  required BridgeLightSettings settings,
  required BridgeLightChannels channels,
  required double compWidth,
  required double compHeight,
}) async {
  final project = Provider.of<LumitState>(context, listen: false).project;
  final picked = await showLumitModal<(_LightValues, String)>(
    context: context,
    id: 'light-settings',
    builder: (close) => _LightSettingsBody(
      values: _LightValues.of(settings, channels),
      name: name,
      onDone: close,
    ),
  );
  if (picked == null) return false;
  final (values, wanted) = picked;
  asOneUndoStep(project, () {
    if (values.settings != settings) {
      layer.setLightSettings(settings: values.settings);
    }
    final (props, scalars) = values.writes(channels);
    // A light from an older file has never had a point to aim at, so the
    // first time it is aimed it looks at the comp's middle, as a new one does.
    const zero = BridgeScalar.static_(0);
    if (!settings.twoNode &&
        values.settings.twoNode &&
        channels.poiX == zero &&
        channels.poiY == zero &&
        channels.poiZ == zero) {
      props.addAll(const [BridgeTransformProp.poiX, BridgeTransformProp.poiY]);
      scalars.addAll([
        BridgeScalar.static_(compWidth / 2),
        BridgeScalar.static_(compHeight / 2),
      ]);
    }
    if (props.isNotEmpty) layer.setTransforms(props: props, values: scalars);
    if (wanted.isNotEmpty && wanted != name) layer.rename(name: wanted);
  });
  return true;
}

class _LightSettingsBody extends StatefulWidget {
  final _LightValues values;

  /// The light's name, or null for a light still to be made, whose name
  /// follows its type until it is typed over.
  final String? name;
  final void Function((_LightValues, String)?) onDone;

  const _LightSettingsBody({
    required this.values,
    required this.name,
    required this.onDone,
  });

  @override
  State<_LightSettingsBody> createState() => _LightSettingsBodyState();
}

class _LightSettingsBodyState extends State<_LightSettingsBody> {
  late final _LightValues _v = widget.values.copy();
  late final TextEditingController _name =
      TextEditingController(text: widget.name ?? _kindName);

  String get _kindName => l10n.lightLayerName(lightKindLabel(_s.kind));

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  BridgeLightSettings get _s => _v.settings;

  void _confirm() => widget.onDone((_v, _name.text.trim()));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final kind = _s.kind;
    return DialogFrame(
      width: cameraDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.lightSettings,
          onClose: () => widget.onDone(null),
          keyPrefix: 'light',
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(
              dialogPadding, dialogPadding, dialogPadding, 10),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _row(
                  t,
                  l10n.name,
                  HouseTextField(
                    key: const ValueKey('light-name'),
                    controller: _name,
                    width: double.infinity,
                    onSubmitted: (_) => _confirm(),
                  )),
              _row(t, l10n.lightType, _typeRow(t)),
            ],
          ),
        ),
        _section(t, l10n.lightOptions, [
          _row(t, l10n.lightColour, _colourRow(t)),
          _row(
              t,
              l10n.lightIntensity,
              _well(t, 'intensity', _v.intensity, (v) => _v.intensity = v,
                  unit: l10n.unitSymbolPercent, min: -100000, speed: 0.5)),
          if (kind == BridgeLightKind.spot) ...[
            _row(
                t,
                l10n.lightConeAngle,
                _well(t, 'cone', _v.coneAngle, (v) => _v.coneAngle = v,
                    unit: l10n.unitSymbolDegrees,
                    max: 180,
                    decimals: 1,
                    speed: 0.5)),
            _row(
                t,
                l10n.lightConeFeather,
                _well(t, 'feather', _v.coneFeather, (v) => _v.coneFeather = v,
                    unit: l10n.unitSymbolPercent, max: 100, speed: 0.5)),
          ],
          if (kind == BridgeLightKind.area)
            _row(t, l10n.size, _sizeRow(t)),
          if (lightFallsOff(kind)) ...[
            _row(t, l10n.lightFalloff, _falloffRow(t)),
            if (_s.falloff != BridgeLightFalloff.none)
              _row(
                  t,
                  l10n.lightRadius,
                  _well(t, 'radius', _v.radius, (v) => _v.radius = v,
                      unit: l10n.unitSymbolPx, speed: 2)),
            if (_s.falloff == BridgeLightFalloff.smooth)
              _row(
                  t,
                  l10n.lightFalloffDistance,
                  _well(t, 'distance', _v.falloffDistance,
                      (v) => _v.falloffDistance = v,
                      unit: l10n.unitSymbolPx, speed: 2)),
          ],
          if (lightAims(kind))
            _row(
                t,
                l10n.transformPointOfInterest,
                Align(
                  alignment: Alignment.centerLeft,
                  child: HouseCheckbox(
                    key: const ValueKey('light-aim'),
                    value: _s.twoNode,
                    onChanged: (on) => setState(() =>
                        _v.settings = lightSettingsWith(_s, twoNode: on)),
                  ),
                )),
        ]),
        dialogFooter(
          t,
          keyPrefix: 'light',
          actions: [
            HouseButton(
              key: const ValueKey('light-cancel'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: () => widget.onDone(null),
              child: Text(l10n.cancel),
            ),
            HouseButton(
              key: const ValueKey('light-apply'),
              primary: true,
              autofocus: true,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              onPressed: _confirm,
              child: Text(l10n.apply),
            ),
          ],
        ),
      ],
    );
  }

  // ---- the rows ------------------------------------------------------------

  Widget _typeRow(LumitTheme t) => dialogDropdown<BridgeLightKind>(
        t,
        id: 'light-type',
        value: _s.kind,
        options: BridgeLightKind.values,
        label: lightKindLabel,
        onChanged: (k) => setState(() {
          // A new light's name follows its type until it is typed over.
          final was = _kindName;
          _v.settings = lightSettingsWith(_s, kind: k);
          if (widget.name == null && _name.text == was) _name.text = _kindName;
        }),
        width: cameraWell + 60,
      );

  Widget _falloffRow(LumitTheme t) => dialogDropdown<BridgeLightFalloff>(
        t,
        id: 'light-falloff',
        value: _s.falloff,
        options: BridgeLightFalloff.values,
        label: lightFalloffLabel,
        onChanged: (f) =>
            setState(() => _v.settings = lightSettingsWith(_s, falloff: f)),
        width: cameraWell + 60,
      );

  Widget _colourRow(LumitTheme t) {
    final c = _v.colour;
    if (c == null) return _animated(t, 'colour');
    final exact = PickedColour(c[0], c[1], c[2]);
    return Align(
      alignment: Alignment.centerLeft,
      child: ColourSwatchButton(
        key: const ValueKey('light-colour'),
        colour: exact.clipped,
        // The exact numbers, scene-linear as the light's row is.
        initial: exact,
        scale: ColourScale.unit,
        max: 64,
        onPicked: (p) => setState(() => _v.colour = [p.r, p.g, p.b]),
      ),
    );
  }

  Widget _sizeRow(LumitTheme t) => Row(
        children: [
          _field(t, 'width', _v.width, (v) => _v.width = v, speed: 2),
          const SizedBox(width: 6),
          _field(t, 'height', _v.height, (v) => _v.height = v, speed: 2),
          const SizedBox(width: 6),
          Text(l10n.unitSymbolPx, style: dialogMono(t)),
        ],
      );

  /// One number with its unit after it, or the word for an animated one.
  Widget _well(LumitTheme t, String id, double? value, ValueChanged<double> set,
          {required String unit,
          double min = 0,
          double max = 1000000,
          int decimals = 0,
          double speed = 1}) =>
      value == null
          ? _animated(t, id)
          : Row(
              children: [
                _field(t, id, value, set,
                    min: min, max: max, decimals: decimals, speed: speed),
                const SizedBox(width: 6),
                Text(unit, style: dialogMono(t)),
              ],
            );

  Widget _field(LumitTheme t, String id, double? value, ValueChanged<double> set,
          {double min = 0,
          double max = 1000000,
          int decimals = 0,
          double speed = 1}) =>
      SizedBox(
        width: cameraWell,
        height: dialogControlHeight,
        child: value == null
            ? _animated(t, id)
            : DragValueField(
                key: ValueKey<String>('light-$id'),
                value: value,
                min: min,
                max: max,
                speed: speed,
                decimals: decimals,
                fill: t.surface0,
                onChanged: (v) => setState(() => set(v.toDouble())),
              ),
      );

  /// What an animated channel shows where its well would be.
  Widget _animated(LumitTheme t, String id) => Align(
        alignment: Alignment.centerLeft,
        child: Text(
          l10n.animated,
          key: ValueKey<String>('light-$id-animated'),
          style: t.small.copyWith(color: t.textMuted),
        ),
      );

  // ---- the pieces ----------------------------------------------------------

  Widget _row(LumitTheme t, String label, Widget control) => dialogRow(
        t,
        label,
        control,
        labelColumn: cameraLabelColumn,
        gap: cameraRowGap,
        minHeight: cameraRowHeight,
      );

  /// A named group of rows, on the Camera settings dialog's pattern.
  Widget _section(LumitTheme t, String title, List<Widget> rows) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(height: 1, color: t.hairline),
          Padding(
            padding: const EdgeInsets.fromLTRB(
                dialogPadding, cameraSectionTop, dialogPadding, 10),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox(
                  height: cameraSectionKicker,
                  child: Padding(
                    padding: const EdgeInsets.only(top: 8, bottom: 4),
                    child: Align(
                      alignment: Alignment.centerLeft,
                      child: Text(t.kickerCase(title), style: t.kicker),
                    ),
                  ),
                ),
                ...rows,
              ],
            ),
          ),
        ],
      );
}
