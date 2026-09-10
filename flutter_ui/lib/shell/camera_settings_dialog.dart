// Layer ▸ Camera settings: everything about a camera that is not a keyframed
// row (docs/impl/camera.md §9).
//
// The shape is the Composition settings dialog's, because it is the same kind
// of question: a strip of rows, sections under a rule, Cancel and OK at the
// foot, and nothing written until the button is pressed.
//
// **Four numbers, one lens.** Zoom, angle of view, film size and focal length
// are four readings of the same thing, and editing any one moves the others.
// The arithmetic is the engine's (`cameraLens` and its inverses), so the
// window and the renderer cannot drift apart; the document holds the zoom in
// comp pixels and the film size in millimetres, and the other two are only
// ever shown. The same goes for the aperture and its F-stop.
//
// **An animated channel is read, not written.** A camera whose zoom is keyed
// says so where the well would be: writing a still number over a curve here
// would delete the animation, and this window is not where that decision
// belongs.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// The frame this drawing measures for itself, on the Composition settings
/// dialog's proportions.
const double cameraDialogWidth = 460;
const double cameraLabelColumn = 120;
const double cameraRowGap = 12;
const double cameraRowHeight = 30;
const double cameraWell = 90;
const double cameraSectionKicker = 24;
const double cameraSectionTop = 4;

/// Which units the window shows its distances in. Display only: the document
/// holds comp pixels and millimetres whichever of these is chosen.
enum CameraUnits { pixels, millimetres, inches }

/// Millimetres in an inch, for the third of them.
const double _mmPerInch = 25.4;

/// Edit a camera's settings. Returns true when the button was pressed, so the
/// caller can redraw; false when it was cancelled.
Future<bool> showCameraSettingsFrb({
  required BuildContext context,
  required LayerReference layer,
  required BridgeCameraSettings settings,
  required BridgeCameraChannels channels,
  required double compWidth,
}) async {
  final applied = await showLumitModal<bool>(
    context: context,
    id: 'camera-settings',
    builder: (close) => _CameraSettingsBody(
      layer: layer,
      settings: settings,
      channels: channels,
      compWidth: compWidth,
      onDone: close,
    ),
  );
  return applied ?? false;
}

/// The still value of a channel, or null when it is animated - which is what
/// makes the row read rather than edit.
double? staticCameraValue(BridgeScalar scalar) =>
    scalar is BridgeScalar_Static ? scalar.field0 : null;

class _CameraSettingsBody extends StatefulWidget {
  final LayerReference layer;
  final BridgeCameraSettings settings;
  final BridgeCameraChannels channels;
  final double compWidth;
  final void Function(bool?) onDone;

  const _CameraSettingsBody({
    required this.layer,
    required this.settings,
    required this.channels,
    required this.compWidth,
    required this.onDone,
  });

  @override
  State<_CameraSettingsBody> createState() => _CameraSettingsBodyState();
}

class _CameraSettingsBodyState extends State<_CameraSettingsBody> {
  late bool _twoNode;
  late bool _depthOfField;
  late bool _lockToZoom;
  late double _filmMm;
  late CameraUnits _units;

  /// The four animatable numbers this window can write, or null where the
  /// channel is animated and it cannot.
  double? _zoom;
  double? _focus;
  double? _aperture;
  double? _blur;

  /// What the zoom and the aperture read as in the other units the window
  /// shows. Held rather than computed per build: the maths is the engine's and
  /// a drag would otherwise cross the bridge on every tick.
  late BridgeCameraLens _lens;
  double _fStop = 0;

  /// The presets, read once: the list is the same for the life of the process.
  late final List<double> _presets = cameraPresetsMm().toList();

  @override
  void initState() {
    super.initState();
    _twoNode = widget.settings.twoNode;
    _depthOfField = widget.settings.depthOfField;
    _lockToZoom = widget.settings.lockToZoom;
    _filmMm = widget.settings.filmSizeMm;
    _units = CameraUnits.pixels;
    _zoom = staticCameraValue(widget.channels.zoom);
    _focus = staticCameraValue(widget.channels.focusDistance);
    _aperture = staticCameraValue(widget.channels.aperture);
    _blur = staticCameraValue(widget.channels.blurLevel);
    _recompute();
  }

  /// The readings that follow the zoom, the film size and the aperture.
  void _recompute() {
    final zoom = _zoom ?? 0;
    _lens = cameraLens(
        zoom: zoom, filmMm: _filmMm, compW: widget.compWidth);
    _fStop = cameraFStop(zoom: zoom, aperture: _aperture ?? 0);
  }

  /// A new zoom, with everything that reads off it brought along - including
  /// the focus distance while it is locked to the zoom.
  void _setZoom(double zoom) => setState(() {
        _zoom = zoom;
        if (_lockToZoom && _focus != null) _focus = zoom;
        _recompute();
      });

  void _setAperture(double aperture) => setState(() {
        _aperture = aperture;
        _recompute();
      });

  /// How many of the chosen units one comp pixel is. Pixels are the document's
  /// own, and the other two go through the film size the way a focal length
  /// does.
  double get _unitScale => switch (_units) {
        CameraUnits.pixels => 1,
        CameraUnits.millimetres => _filmMm / widget.compWidth,
        CameraUnits.inches => _filmMm / widget.compWidth / _mmPerInch,
      };

  /// How many decimals a distance well shows: whole pixels, but a fraction of
  /// a millimetre or of an inch.
  int get _unitDecimals => _units == CameraUnits.pixels ? 0 : 2;

  String get _unitLabel => switch (_units) {
        CameraUnits.pixels => l10n.unitSymbolPx,
        CameraUnits.millimetres => l10n.unitSymbolMm,
        CameraUnits.inches => l10n.unitSymbolInches,
      };

  /// The preset whose focal length the lens is currently at, or null for a
  /// lens of its own.
  double? get _preset {
    for (final mm in _presets) {
      if ((mm - _lens.focalMm).abs() < 0.05) return mm;
    }
    return null;
  }

  /// Write what changed and close. One settings op, and one transform batch
  /// carrying only the still channels that actually moved - an animated one is
  /// never among them.
  void _confirm() {
    widget.layer.setCameraSettings(
      settings: BridgeCameraSettings(
        twoNode: _twoNode,
        depthOfField: _depthOfField,
        lockToZoom: _lockToZoom,
        filmSizeMm: _filmMm,
      ),
    );
    final props = <BridgeTransformProp>[];
    final values = <BridgeScalar>[];
    void put(BridgeTransformProp prop, double? now, BridgeScalar was) {
      if (now == null) return;
      if (was is BridgeScalar_Static && was.field0 == now) return;
      props.add(prop);
      values.add(BridgeScalar.static_(now));
    }

    put(BridgeTransformProp.zoom, _zoom, widget.channels.zoom);
    put(BridgeTransformProp.focusDistance, _focus,
        widget.channels.focusDistance);
    put(BridgeTransformProp.aperture, _aperture, widget.channels.aperture);
    put(BridgeTransformProp.blurLevel, _blur, widget.channels.blurLevel);
    if (props.isNotEmpty) {
      widget.layer.setTransforms(props: props, values: values);
    }
    widget.onDone(true);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: cameraDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.cameraSettings,
          onClose: () => widget.onDone(false),
          keyPrefix: 'camera',
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(
              dialogPadding, dialogPadding, dialogPadding, 10),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _row(t, l10n.cameraType, _typeRow(t)),
              _row(t, l10n.exportPreset, _presetRow(t)),
              _row(t, l10n.cameraUnits, _unitsRow(t)),
            ],
          ),
        ),
        _section(t, l10n.cameraOptions, [
          _row(t, l10n.sourceZoom, _distanceRow(t, 'zoom', _zoom, _setZoom)),
          _row(t, l10n.cameraAngleOfView, _angleRow(t)),
          _row(t, l10n.cameraFilmSize, _filmRow(t)),
          _row(t, l10n.cameraFocalLength, _focalRow(t)),
        ]),
        _section(t, l10n.cameraDepthOfField, [
          _row(t, l10n.cameraDepthOfField, _depthRow(t)),
          _row(
              t,
              l10n.cameraFocusDistance,
              _distanceRow(t, 'focus', _focus,
                  (v) => setState(() => _focus = v))),
          _row(t, l10n.cameraLockToZoom, _lockRow(t)),
          // A finer drag than the two distances above it: an aperture is a
          // fraction of their size, and the F-stop beside it swings on it.
          _row(
              t,
              l10n.cameraAperture,
              _distanceRow(t, 'aperture', _aperture, _setAperture,
                  min: 0, speed: 0.5)),
          _row(t, l10n.cameraFStop, _fStopRow(t)),
          _row(t, l10n.cameraBlurLevel, _blurRow(t)),
        ]),
        dialogFooter(
          t,
          keyPrefix: 'camera',
          actions: [
            HouseButton(
              key: const ValueKey('camera-cancel'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: () => widget.onDone(false),
              child: Text(l10n.cancel),
            ),
            HouseButton(
              key: const ValueKey('camera-apply'),
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

  Widget _typeRow(LumitTheme t) => dialogDropdown<bool>(
        t,
        id: 'camera-type',
        value: _twoNode,
        options: const [false, true],
        label: (two) => two ? l10n.cameraTwoNode : l10n.cameraOneNode,
        onChanged: (two) => setState(() => _twoNode = two),
        width: cameraWell + 60,
      );

  Widget _presetRow(LumitTheme t) => dialogDropdown<double?>(
        t,
        id: 'camera-preset',
        value: _preset,
        options: [null, ..._presets],
        label: (mm) =>
            mm == null ? l10n.custom : l10n.cameraPresetMm(mm.toStringAsFixed(0)),
        // A preset is a focal length, so picking one is the focal-length well
        // being typed into - the zoom follows, and the angle of view with it.
        onChanged: (mm) {
          if (mm != null) _setFocal(mm);
        },
        width: cameraWell + 60,
      );

  Widget _unitsRow(LumitTheme t) => dialogDropdown<CameraUnits>(
        t,
        id: 'camera-units',
        value: _units,
        options: CameraUnits.values,
        label: (u) => switch (u) {
          CameraUnits.pixels => l10n.unitPixels,
          CameraUnits.millimetres => l10n.unitMillimetres,
          CameraUnits.inches => l10n.unitInches,
        },
        onChanged: (u) => setState(() => _units = u),
        width: cameraWell + 60,
      );

  /// One of the three distances the units apply to. [value] is null on an
  /// animated channel, and the row then says so instead of offering a well.
  Widget _distanceRow(
          LumitTheme t, String id, double? value, ValueChanged<double> set,
          {double min = 1, double speed = 4}) =>
      value == null
          ? _animated(t, id)
          : Row(
              children: [
                SizedBox(
                  width: cameraWell,
                  height: dialogControlHeight,
                  child: DragValueField(
                    key: ValueKey<String>('camera-$id'),
                    value: value * _unitScale,
                    min: min * _unitScale,
                    max: 1000000,
                    speed: speed,
                    decimals: _unitDecimals,
                    fill: t.surface0,
                    onChanged: (v) => set(v.toDouble() / _unitScale),
                  ),
                ),
                const SizedBox(width: 6),
                Text(_unitLabel, style: dialogMono(t)),
              ],
            );

  Widget _angleRow(LumitTheme t) => _zoom == null
      ? _animated(t, 'angle')
      : Row(
          children: [
            SizedBox(
              width: cameraWell,
              height: dialogControlHeight,
              child: DragValueField(
                key: const ValueKey('camera-angle'),
                value: _lens.angleDeg,
                min: 0.1,
                max: 179.9,
                decimals: 1,
                speed: 0.5,
                fill: t.surface0,
                onChanged: (v) => _setZoom(cameraZoomForAngle(
                    angleDeg: v.toDouble(), compW: widget.compWidth)),
              ),
            ),
            const SizedBox(width: 6),
            Text(l10n.unitSymbolDegrees, style: dialogMono(t)),
          ],
        );

  Widget _filmRow(LumitTheme t) => Row(
        children: [
          SizedBox(
            width: cameraWell,
            height: dialogControlHeight,
            child: DragValueField(
              key: const ValueKey('camera-film'),
              value: _filmMm,
              min: 1,
              max: 1000,
              decimals: 2,
              fill: t.surface0,
              // The zoom is left where it is, so the picture does not move:
              // a wider sensor is a longer lens taking in the same view.
              onChanged: (v) => setState(() {
                _filmMm = v.toDouble();
                _recompute();
              }),
            ),
          ),
          const SizedBox(width: 6),
          Text(l10n.unitSymbolMm, style: dialogMono(t)),
        ],
      );

  Widget _focalRow(LumitTheme t) => _zoom == null
      ? _animated(t, 'focal')
      : Row(
          children: [
            SizedBox(
              width: cameraWell,
              height: dialogControlHeight,
              child: DragValueField(
                key: const ValueKey('camera-focal'),
                value: _lens.focalMm,
                min: 1,
                max: 10000,
                decimals: 2,
                fill: t.surface0,
                onChanged: (v) => _setFocal(v.toDouble()),
              ),
            ),
            const SizedBox(width: 6),
            Text(l10n.unitSymbolMm, style: dialogMono(t)),
          ],
        );

  Widget _depthRow(LumitTheme t) => Align(
        alignment: Alignment.centerLeft,
        child: HouseCheckbox(
          key: const ValueKey('camera-dof'),
          value: _depthOfField,
          onChanged: (on) => setState(() => _depthOfField = on),
        ),
      );

  Widget _lockRow(LumitTheme t) => Align(
        alignment: Alignment.centerLeft,
        child: HouseCheckbox(
          key: const ValueKey('camera-lock'),
          value: _lockToZoom,
          // Ticking it is what makes the two agree; from then on the zoom
          // carries the focus distance with it.
          onChanged: (on) => setState(() {
            _lockToZoom = on;
            if (on && _zoom != null && _focus != null) _focus = _zoom;
          }),
        ),
      );

  Widget _fStopRow(LumitTheme t) => _aperture == null || _zoom == null
      ? _animated(t, 'fstop')
      : SizedBox(
          width: cameraWell,
          height: dialogControlHeight,
          child: DragValueField(
            key: const ValueKey('camera-fstop'),
            value: _fStop,
            min: 0.1,
            max: 1000,
            decimals: 1,
            speed: 0.1,
            fill: t.surface0,
            onChanged: (v) => _setAperture(cameraApertureForFStop(
                fStop: v.toDouble(), zoom: _zoom ?? 0)),
          ),
        );

  Widget _blurRow(LumitTheme t) => _blur == null
      ? _animated(t, 'blur')
      : Row(
          children: [
            SizedBox(
              width: cameraWell,
              height: dialogControlHeight,
              child: DragValueField(
                key: const ValueKey('camera-blur'),
                value: _blur!,
                min: 0,
                max: 100,
                decimals: 0,
                speed: 0.5,
                fill: t.surface0,
                onChanged: (v) => setState(() => _blur = v.toDouble()),
              ),
            ),
            const SizedBox(width: 6),
            Text(l10n.unitSymbolPercent, style: dialogMono(t)),
          ],
        );

  /// A new focal length: the zoom that gives it at the film size in force.
  void _setFocal(double mm) => _setZoom(cameraZoomForFocal(
      focalMm: mm, filmMm: _filmMm, compW: widget.compWidth));

  /// What an animated channel shows where its well would be.
  Widget _animated(LumitTheme t, String id) => Align(
        alignment: Alignment.centerLeft,
        child: Text(
          l10n.animated,
          key: ValueKey<String>('camera-$id-animated'),
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

  /// A named group of rows, on the Composition settings dialog's own pattern:
  /// a rule, a little air, a kicker band, and the rows under it.
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
                      child: Text(title.toUpperCase(), style: t.kicker),
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
