// Save preset: which parts of the layer go in the file, then where it goes.
//
// Only a part the layer actually has is offered, so a layer with nothing but
// effects goes straight to the file picker. The ticks are remembered from last
// time. The engine hands back the text and Dart writes it, as ever.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../shell/dialog_frame.dart';
import '../state/file_dialogs.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';

/// Save [layer] as a `.lumfx`, asking which parts where there's a choice.
/// [picker] names the file, or the library's save dialogue does. Returns
/// whether a file was written.
Future<bool> saveLayerPreset(
  BuildContext context,
  LumitUiState ui,
  LayerReference layer, {
  Future<String?> Function(String suggested)? picker,
}) async {
  final info = ui.model.byId(layer.internallayerId)?.info;
  final ids = [
    for (final e in info?.effects ?? const <BridgeEffectInstanceInfo>[]) e.id,
  ];
  final hasEffects = ids.isNotEmpty;
  final hasAnimators = info?.textAnimators.isNotEmpty ?? false;
  var hasKeys = false;
  try {
    hasKeys = layer.revealGroups(kind: BridgeRevealKind.animated).transform;
  } catch (_) {
    // The layer has gone; the write below says so.
  }
  final ws = ui.workspace;
  var effects = true, keys = false, animators = false;
  if ((hasEffects ? 1 : 0) + (hasKeys ? 1 : 0) + (hasAnimators ? 1 : 0) > 1) {
    final ticks = await _askParts(context, ws,
        hasEffects: hasEffects, hasKeys: hasKeys, hasAnimators: hasAnimators);
    if (ticks == null) return false;
    (effects, keys, animators) = ticks;
  } else {
    keys = hasKeys;
    animators = hasAnimators;
  }

  final path = await (picker ??
      (suggested) => pickPresetSaveLocation(suggested,
          initialDirectory: presetsDirPath()))('preset.lumfx');
  if (path == null) return false;
  final name = path.split(RegExp(r'[/\\]')).last.replaceAll('.lumfx', '');
  try {
    File(path).writeAsStringSync(layer.savePresetParts(
      name: name,
      effects: effects ? ids : const [],
      transformKeys: keys,
      textAnimators: animators,
    ));
  } catch (_) {
    // A folder that will not take the file, or a layer that went away under the
    // open dialogue. The document is untouched either way.
    return false;
  }
  ui.presetsChanged.value++;
  return true;
}

/// The three ticks, or null when the window was closed. A part the layer has
/// none of isn't drawn, and at least one tick has to stay on.
Future<(bool, bool, bool)?> _askParts(
  BuildContext context,
  Workspace ws, {
  required bool hasEffects,
  required bool hasKeys,
  required bool hasAnimators,
}) {
  var effects = hasEffects && ws.presetSaveEffects;
  var keys = hasKeys && ws.presetSaveKeys;
  var animators = hasAnimators && ws.presetSaveAnimators;
  return showLumitModal<(bool, bool, bool)>(
    context: context,
    builder: (close) => StatefulBuilder(builder: (context, set) {
      final t = ThemeScope.of(context).theme;
      Widget tick(String key, String label, bool on, ValueChanged<bool> flip) =>
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 3),
            child: HouseCheckbox(
              key: ValueKey<String>(key),
              value: on,
              label: label,
              onChanged: (v) => set(() => flip(v)),
            ),
          );
      final any = effects || keys || animators;
      return DialogFrame(
        width: 300,
        children: [
          dialogTitleBar(t,
              title: l10n.presetSaveTitle,
              onClose: () => close(null),
              keyPrefix: 'preset-save'),
          Padding(
            padding: dialogBodyPadding,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                if (hasEffects)
                  tick('preset-part-effects', l10n.presetPartEffects, effects,
                      (v) => effects = v),
                if (hasKeys)
                  tick('preset-part-keys', l10n.presetPartTransformKeys, keys,
                      (v) => keys = v),
                if (hasAnimators)
                  tick('preset-part-animators', l10n.presetPartTextAnimators,
                      animators, (v) => animators = v),
              ],
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'preset-save',
            actions: [
              HouseButton(
                key: const ValueKey('preset-save-cancel'),
                onPressed: () => close(null),
                child: Text(l10n.cancel),
              ),
              HouseButton(
                key: const ValueKey('preset-save-ok'),
                primary: true,
                onPressed: !any
                    ? null
                    : () {
                        ws.setPresetSaveParts(
                          effects: hasEffects ? effects : ws.presetSaveEffects,
                          keys: hasKeys ? keys : ws.presetSaveKeys,
                          animators:
                              hasAnimators ? animators : ws.presetSaveAnimators,
                        );
                        close((effects, keys, animators));
                      },
                child: Text(l10n.save),
              ),
            ],
          ),
        ],
      );
    }),
  );
}
