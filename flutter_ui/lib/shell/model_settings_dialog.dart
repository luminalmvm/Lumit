// Layer ▸ Model settings, and the window a dropped 3D model opens: which way is
// up in the file, how many comp pixels a metre is, or the model fitted to the
// comp. Each change is written to the layer as it is made, so the Viewer shows
// it, and the whole visit is one undo step that Cancel takes back.

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
      builder: (close) => _ModelSettingsBody(
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
  final BridgeModelSettings settings;
  final ValueChanged<BridgeModelSettings> onChanged;
  final void Function(bool?) onDone;

  const _ModelSettingsBody({
    required this.settings,
    required this.onChanged,
    required this.onDone,
  });

  @override
  State<_ModelSettingsBody> createState() => _ModelSettingsBodyState();
}

class _ModelSettingsBodyState extends State<_ModelSettingsBody> {
  late BridgeModelSettings _s = widget.settings;

  void _set({BridgeUpAxis? upAxis, double? units, bool? fit}) {
    setState(() => _s = BridgeModelSettings(
          upAxis: upAxis ?? _s.upAxis,
          units: units ?? _s.units,
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
                  dialogDropdown<BridgeUpAxis>(
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
              // A fitted model takes its size from the comp, so there are no
              // units to set.
              if (!_s.fit)
                _row(
                    t,
                    l10n.modelUnits,
                    Row(
                      children: [
                        SizedBox(
                          width: cameraWell,
                          height: dialogControlHeight,
                          child: DragValueField(
                            key: const ValueKey('model-units'),
                            value: _s.units,
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
