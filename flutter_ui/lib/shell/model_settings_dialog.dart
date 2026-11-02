// Layer ▸ Model settings, and the window a dropped 3D model opens: which way is
// up in the file, how many comp pixels a metre is, or the model fitted to the
// comp. Each change is written to the layer as it is made, so the Viewer shows
// it, and the whole visit is one undo step that Cancel takes back.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/model.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'camera_settings_dialog.dart'
    show
        cameraDialogWidth,
        cameraLabelColumn,
        cameraRowGap,
        cameraRowHeight,
        cameraWell;
import 'dialog_frame.dart';

/// The names the Ambient light and the Key light a first model brings with it
/// take, in that order.
List<String> modelLightNames() =>
    [l10n.lightLayerName(l10n.lightAmbient), l10n.keyLightName];

/// The Model settings of the first Model layer in [placed], the layers a drop
/// just made, if there is one.
Future<void> showDroppedModelSettingsFrb(
    BuildContext context, List<LayerReference> placed) async {
  for (final layer in placed) {
    if (layer.getKind() == BridgeLayerKind.model) {
      return showModelSettingsFrb(context: context, layer: layer);
    }
  }
}

/// Opens the window over Model layer [layer]. Does nothing on another kind.
Future<void> showModelSettingsFrb({
  required BuildContext context,
  required LayerReference layer,
}) async {
  final settings = layer.getModelSettings();
  if (settings == null) return;
  final app = Provider.of<LumitState>(context, listen: false);
  final project = app.project;
  var written = false;
  bool? kept;
  project?.beginUndoGroup();
  try {
    kept = await showLumitModal<bool>(
      context: context,
      id: 'model-settings',
      // The Viewer behind shows each change as it is made.
      dimBackground: false,
      builder: (close) => _ModelSettingsBody(
        layer: layer,
        settings: settings,
        onChanged: (s) {
          layer.setModelSettings(settings: s);
          written = true;
          app.notifyDocumentChanged();
        },
        onDone: close,
      ),
    );
  } finally {
    project?.endUndoGroup();
  }
  if (kept != true && written) {
    project?.undo();
    app.notifyDocumentChanged();
  }
}

class _ModelSettingsBody extends StatefulWidget {
  final LayerReference layer;
  final BridgeModelSettings settings;
  final ValueChanged<BridgeModelSettings> onChanged;
  final void Function(bool?) onDone;

  const _ModelSettingsBody({
    required this.layer,
    required this.settings,
    required this.onChanged,
    required this.onDone,
  });

  @override
  State<_ModelSettingsBody> createState() => _ModelSettingsBodyState();
}

class _ModelSettingsBodyState extends State<_ModelSettingsBody> {
  late BridgeModelSettings _s = widget.settings;

  /// What Fit to comp makes a metre, null until the model has been read.
  late double? _fitted = _fittedFor(_s.upAxis);

  /// Asks again until a model still being read has been.
  Timer? _waiting;

  @override
  void initState() {
    super.initState();
    if (_fitted != null) return;
    _waiting = Timer.periodic(const Duration(milliseconds: 250), (timer) {
      final fitted = _fittedFor(_s.upAxis);
      if (fitted == null) return;
      timer.cancel();
      setState(() => _fitted = fitted);
    });
  }

  @override
  void dispose() {
    _waiting?.cancel();
    super.dispose();
  }

  double? _fittedFor(BridgeUpAxis up) {
    try {
      return widget.layer.getModelFittedUnits(upAxis: up);
    } catch (_) {
      return null;
    }
  }

  void _set({BridgeUpAxis? upAxis, double? units, bool? fit}) {
    if (upAxis != null && upAxis != _s.upAxis) _fitted = _fittedFor(upAxis);
    setState(() => _s = BridgeModelSettings(
          upAxis: upAxis ?? _s.upAxis,
          // Fit let go keeps the size it gave, so the model stays put.
          units: units ?? (fit == false && _s.fit ? _fitted : null) ?? _s.units,
          fit: fit ?? _s.fit,
        ));
    widget.onChanged(_s);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: cameraDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.modelSettings,
          onClose: () => widget.onDone(null),
          keyPrefix: 'model',
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
                  l10n.modelUpAxis,
                  Align(
                    alignment: Alignment.centerLeft,
                    child: dialogDropdown<BridgeUpAxis>(
                      t,
                      id: 'model-up',
                      value: _s.upAxis,
                      options: BridgeUpAxis.values,
                      label: (a) => switch (a) {
                        BridgeUpAxis.y => l10n.viewportAxisY,
                        BridgeUpAxis.z => l10n.viewportAxisZ,
                      },
                      onChanged: (a) => _set(upAxis: a),
                      width: cameraWell,
                    ),
                  )),
              _row(
                  t,
                  l10n.modelFitToComp,
                  Align(
                    alignment: Alignment.centerLeft,
                    child: HouseCheckbox(
                      key: const ValueKey('model-fit'),
                      value: _s.fit,
                      onChanged: (on) => _set(fit: on),
                    ),
                  )),
              // A fitted model takes its size from the comp, so its units
              // show greyed, as the fit has them.
              _row(
                  t,
                  l10n.modelUnits,
                  IgnorePointer(
                    ignoring: _s.fit,
                    child: AnimatedOpacity(
                      opacity: _s.fit ? 0.45 : 1,
                      duration: const Duration(milliseconds: 120),
                      curve: Curves.easeOut,
                      child: Row(
                        children: [
                          SizedBox(
                            width: cameraWell,
                            height: dialogControlHeight,
                            child: DragValueField(
                              key: const ValueKey('model-units'),
                              value: _s.fit ? _fitted ?? _s.units : _s.units,
                              min: 0.001,
                              max: 1000000,
                              speed: 1,
                              decimals: 1,
                              fill: t.surface0,
                              onChanged: (v) => _set(units: v.toDouble()),
                            ),
                          ),
                          const SizedBox(width: 6),
                          Text(l10n.modelPixelsPerMetre, style: dialogMono(t)),
                        ],
                      ),
                    ),
                  )),
            ],
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'model',
          actions: [
            HouseButton(
              key: const ValueKey('model-cancel'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: () => widget.onDone(false),
              child: Text(l10n.cancel),
            ),
            HouseButton(
              key: const ValueKey('model-ok'),
              primary: true,
              autofocus: true,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              onPressed: () => widget.onDone(true),
              child: Text(l10n.done),
            ),
          ],
        ),
      ],
    );
  }

  Widget _row(LumitTheme t, String label, Widget control) => dialogRow(
        t,
        label,
        control,
        labelColumn: cameraLabelColumn,
        gap: cameraRowGap,
        minHeight: cameraRowHeight,
      );
}
