// Save preset: a name, and which parts of the layer go in it, into the preset
// library where Effects & presets lists it.
//
// Only a part the layer actually has is offered. The ticks are remembered from
// last time. The engine hands back the text and Dart writes it, as ever.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../shell/dialog_frame.dart';
import '../shell/settings_account.dart' show askBefore;
import '../state/file_dialogs.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';

/// Save [layer] as a `.lumfx` in the preset library, asking for a name and,
/// where there's a choice, which parts. [picker] names a file instead, for a
/// test. Returns whether a file was written.
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
  final choice =
      (hasEffects ? 1 : 0) + (hasKeys ? 1 : 0) + (hasAnimators ? 1 : 0) > 1;
  var effects = true, keys = hasKeys, animators = hasAnimators;
  final String? path;
  if (picker != null) {
    if (choice) {
      final asked = await _ask(context, ws, null,
          hasEffects: hasEffects, hasKeys: hasKeys, hasAnimators: hasAnimators);
      if (asked == null) return false;
      (_, effects, keys, animators) = asked;
    }
    path = await picker('preset.lumfx');
  } else {
    final asked = await _ask(context, ws, info?.name ?? '',
        hasEffects: hasEffects && choice,
        hasKeys: hasKeys && choice,
        hasAnimators: hasAnimators && choice);
    if (asked == null || !context.mounted) return false;
    final String named;
    (named, effects, keys, animators) = asked;
    if (!choice) (effects, keys, animators) = (true, hasKeys, hasAnimators);
    path = await presetPathFor(context, named);
  }
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

/// Where a preset called [name] goes in the library, asking first before one
/// of that name is written over. Null when that was refused, or there is no
/// library and the system's Save dialogue was closed.
Future<String?> presetPathFor(BuildContext context, String name,
    {String extension = 'lumfx', bool material = false}) async {
  final library = presetsDirPath();
  if (library == null) {
    return pickPresetSaveLocation('$name.$extension', material: material);
  }
  // The name can hold what a file's name can't.
  final stem = name.replaceAll(RegExp(r'[/\\:*?"<>|]'), '-');
  final file = File('$library${Platform.pathSeparator}$stem.$extension');
  if (file.existsSync() &&
      !await askBefore(
          context, l10n.presetReplace(name), l10n.gradeLookReplaceYes)) {
    return null;
  }
  return file.path;
}

/// A name for a preset of one effect or a material: the same window with no
/// ticks. Null when it was closed.
Future<String?> askPresetName(
        BuildContext context, Workspace ws, String suggested) async =>
    (await _ask(context, ws, suggested,
            hasEffects: false, hasKeys: false, hasAnimators: false))
        ?.$1;

/// The name, when [suggested] asks for one, and the three ticks, or null when
/// the window was closed. A part the layer has none of isn't drawn, and at
/// least one tick has to stay on while any is drawn.
Future<(String, bool, bool, bool)?> _ask(
  BuildContext context,
  Workspace ws,
  String? suggested, {
  required bool hasEffects,
  required bool hasKeys,
  required bool hasAnimators,
}) async {
  var effects = hasEffects && ws.presetSaveEffects;
  var keys = hasKeys && ws.presetSaveKeys;
  var animators = hasAnimators && ws.presetSaveAnimators;
  final name = TextEditingController(text: suggested ?? '');
  name.selection =
      TextSelection(baseOffset: 0, extentOffset: name.text.length);
  final asked = await showLumitModal<(String, bool, bool, bool)>(
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
      final ticks = hasEffects || hasKeys || hasAnimators;
      final ok = (!ticks || effects || keys || animators) &&
          (suggested == null || name.text.trim().isNotEmpty);
      void save() {
        if (!ok) return;
        if (ticks) {
          ws.setPresetSaveParts(
            effects: hasEffects ? effects : ws.presetSaveEffects,
            keys: hasKeys ? keys : ws.presetSaveKeys,
            animators: hasAnimators ? animators : ws.presetSaveAnimators,
          );
        }
        close((name.text.trim(), effects, keys, animators));
      }

      return dialogEnter(
        onEnter: save,
        child: DialogFrame(
          width: 320,
          children: [
            dialogTitleBar(t,
                title: l10n.presetSaveTitle,
                onClose: () => close(null),
                keyPrefix: 'preset-save'),
            Padding(
              padding: dialogBodyPadding,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                mainAxisSize: MainAxisSize.min,
                children: [
                  if (suggested != null)
                    dialogRow(
                      t,
                      l10n.name,
                      HouseTextField(
                        key: const ValueKey('preset-save-name'),
                        controller: name,
                        autofocus: true,
                        width: double.infinity,
                        onSubmitted: (_) => save(),
                      ),
                      labelColumn: 48,
                    ),
                  if (suggested != null && ticks) const SizedBox(height: 8),
                  if (hasEffects)
                    tick('preset-part-effects', l10n.presetPartEffects,
                        effects, (v) => effects = v),
                  if (hasKeys)
                    tick('preset-part-keys', l10n.presetPartTransformKeys,
                        keys, (v) => keys = v),
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
                  onPressed: ok ? save : null,
                  child: Text(l10n.save),
                ),
              ],
            ),
          ],
        ),
      );
    }),
  );
  name.dispose();
  return asked;
}
