// What can be done to one effect, and the menu that offers it. The heading in
// Effect controls, the heading in the Timeline's fold-out and the box on the
// layer's graph all come here, so one effect has one menu wherever it is shown.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/effect_presets.dart'
    show listPresetsFor;
import 'package:lumit_flutter/src/rust/api/graph.dart' show BridgeNodeRef;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/template.dart' show BridgeRowKind;
import 'package:lumit_flutter/src/rust/api/model.dart'
    show listMaterialPresets, materialPreset;
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/clipboard.dart';
import '../state/external_links.dart';
import '../state/layer_bounds.dart' show geometryOptionsNames;
import '../widgets/controls.dart';
import 'control_rows.dart' show layerControlRow;
import 'effect_param_row_frb.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'param_row_menu.dart' show menuGroups;
import 'preset_save.dart' show askPresetName, presetPathFor;

/// One effect, on whichever list holds it, and everything its menu can do.
///
/// Holds no handles. Each command reads the list as it stands when it runs, as
/// frb consumes a handle passed by value and a neighbour can have gone since.
class EffectCommands {
  const EffectCommands({
    required this.info,
    required this.layer,
    required this.comp,
    required this.onStackChanged,
    this.style = false,
    this.inputs = false,
    this.material = false,
    this.group,
    this.clip,
  });

  final BridgeEffectInstanceInfo info;

  /// The layer writes go through. For a group's effect that's the group's first
  /// member, and the engine routes it to the group's own list.
  final LayerReference layer;
  final CompositionReference comp;

  final VoidCallback onStackChanged;

  /// A layer style, which keeps a fixed place in a fixed list.
  final bool style;

  /// A placed graph's Inputs: its rows and a Reset, nothing else.
  final bool inputs;

  final bool material;

  final UuidValue? group;
  final UuidValue? clip;

  bool get onStack => !style && !inputs && group == null && clip == null;

  List<BridgeEffectInstance> instances() {
    if (clip case final c?) {
      return layer.getClipEffects(clip: c, onCompClock: true);
    }
    return switch (group) {
      final g? => comp.getGroupEffects(group: g),
      null when inputs => graphInputsOf(layer),
      null when material => layer.getMaterials(),
      null => style ? layer.getStyles() : layer.getEffects(),
    };
  }

  /// This effect and the rest of the picked run it's part of, in stack order.
  /// A style, the Inputs and a group's or a clip's effect are never in a run.
  List<BridgeEffectInstance> _handles(BuildContext context) {
    if (!onStack) {
      return [
        for (final candidate in instances())
          if (candidate.id() == info.id) candidate,
      ];
    }
    final ids = Provider.of<LumitUiState>(context, listen: false)
        .effectsToCopy(layer, info.id)
        .toSet();
    return [
      for (final candidate in layer.getEffects())
        if (ids.contains(candidate.getInfo().id)) candidate,
    ];
  }

  /// Every row this instance draws: those its effect declares, then those its
  /// own state derives. The Inputs are only the graph's own rows.
  List<BridgeParamInfo> get rows => relistedRows(
        inputs ? info.derivedParams : instanceParameters(info),
        info.rowOptions,
      );

  bool get atDefault {
    final held = {for (final v in info.values) v.id: v.value};
    for (final param in rows) {
      final fallback = defaultEffectValue(param.kind);
      final value = held[param.id];
      if (fallback != null &&
          value != null &&
          !isDefaultEffectValue(value, fallback,
              owner: layer.internallayerId)) {
        return false;
      }
    }
    return true;
  }

  /// Run [op] on each of the handles, in stack order, as one undo step.
  void withEach(BuildContext context, void Function(BridgeEffectInstance) op,
      {bool reversed = false, String? name}) {
    final handles = _handles(context);
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      for (final handle in reversed ? handles.reversed : handles) {
        op(handle);
      }
    }, name: name);
  }

  /// On or off. All of them take this effect's new state rather than each
  /// flipping its own, so a run of mixed ticks comes out even.
  void setEnabled(BuildContext context, bool on) {
    withEach(context, (e) => layer.setEffectEnabled(effect: e, enabled: on));
    onStackChanged();
  }

  /// The name, staged on a fresh handle and committed with the list: one op, and
  /// one undo step. An empty name goes back to the effect's own.
  void rename(String name) {
    final stack = instances();
    for (final instance in stack) {
      if (instance.id() != info.id) continue;
      instance.setCustomName(name: name);
      try {
        layer.setEffects(effects: stack);
      } catch (_) {
        // The stack changed under us; re-reading is the recovery.
      }
      break;
    }
    onStackChanged();
  }

  /// Every parameter back to what its schema declares with any curve dropped,
  /// as one op. A material goes back to its file.
  void reset(BuildContext context) {
    if (material) {
      for (final e in instances().where((e) => e.id() == info.id)) {
        layer.removeEffect(effect: e);
      }
      onStackChanged();
      return;
    }
    final run = {for (final e in _handles(context)) e.id()};
    final stack = instances();
    for (final instance in stack) {
      if (!run.contains(instance.id())) continue;
      final params = instance.id() == info.id
          ? rows
          : cachedListParameters(instance.getInfo().name);
      for (final param in params) {
        if (defaultEffectValue(param.kind) case final value?) {
          instance.setValue(id: param.id, value: value);
        }
      }
    }
    try {
      layer.setEffects(effects: stack);
    } catch (_) {
      // The stack changed under us; re-reading is the recovery.
    }
    onStackChanged();
  }

  void remove(BuildContext context) {
    final names = [
      for (final e in _handles(context))
        e.getInfo().customName ?? effectLabelOf(e.getInfo().name),
    ];
    withEach(context, (e) => layer.removeEffect(effect: e),
        name: deleteStepName(names));
    onStackChanged();
  }

  void _applyPreset(BuildContext context, int preset) {
    final at = Provider.of<LumitUiState>(context, listen: false)
        .playheadFrame
        .value;
    withEach(context, (e) {
      if (e.getInfo().name != info.name) return;
      layer.applyEffectPreset(effect: e, preset: preset, atFrame: at);
    });
    onStackChanged();
  }

  Future<void> _applySavedEffect(BuildContext context, String path) async {
    final app = Provider.of<LumitState>(context, listen: false);
    final at = Provider.of<LumitUiState>(context, listen: false)
        .playheadFrame
        .value;
    final String text;
    try {
      text = await readEffectPreset(path: path);
    } catch (_) {
      app.postNotice(l10n.presetFileNotPreset, error: true);
      return;
    }
    if (!context.mounted) return;
    withEach(context, (e) {
      if (e.getInfo().name != info.name) return;
      try {
        layer.applySavedPreset(effect: e, text: text, atFrame: at);
      } catch (_) {}
    });
    onStackChanged();
  }

  Future<void> _applySaved(LumitState app, String path, int at) async {
    try {
      final text = await readEffectPreset(path: path);
      for (final instance in instances()) {
        if (instance.id() == info.id) {
          layer.applyMaterialPreset(material: instance, text: text, atFrame: at);
        }
      }
    } catch (_) {
      app.postNotice(l10n.presetFileNotPreset, error: true);
      return;
    }
    onStackChanged();
  }

  void _copy(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    try {
      ui.copyEffectsToClipboard(
        layer.copyEffects(effects: ui.effectsToCopy(layer, info.id)),
      );
    } catch (_) {
    }
  }

  Future<void> _savePreset(BuildContext context) async {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final kind = material ? 'lummat' : 'lumfx';
    final named = await askPresetName(
        context, ui.workspace, info.customName ?? effectLabelOf(info.name));
    if (named == null || !context.mounted) return;
    final path = await presetPathFor(context, named,
        extension: kind, material: material);
    if (path == null) return;
    final name = path.split(RegExp(r'[/\\]')).last.replaceAll('.$kind', '');
    try {
      File(path).writeAsStringSync(material
          ? materialPreset(
              material: instances().firstWhere((e) => e.id() == info.id),
              name: name)
          : layer.savePreset(name: name, only: info.id));
    } catch (_) {
      return;
    }
    ui.presetsChanged.value++;
    if (context.mounted) {
      Provider.of<LumitState>(context, listen: false)
          .postNotice(l10n.presetSavedNotice);
    }
  }

  /// Put [text] on the stack just below this effect, or below the picked run
  /// it's part of, as one undo step. Effects copied elsewhere arrive timed to
  /// the playhead, and [atFrame] is that frame.
  void _pasteBelow(BuildContext context, String text, int atFrame) {
    final project = Provider.of<LumitState>(context, listen: false).project;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final run = ui.effectsToCopy(layer, info.id).toSet();
    try {
      asOneUndoStep(project, () {
        final before = layer.getEffects();
        final was = {for (final e in before) e.id()};
        var to = 0;
        for (final (i, e) in before.indexed) {
          if (run.contains(e.id())) to = i + 1;
        }
        layer.pasteEffects(text: text, atFrame: atFrame);
        for (final e in layer.getEffects()) {
          if (was.contains(e.id())) continue;
          layer.reorderEffect(effect: e, newIndex: to);
          to += 1;
        }
      });
    } catch (_) {
    }
    onStackChanged();
  }

  void duplicate(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    try {
      ui.setEffectSelection(layer,
          layer.duplicateEffects(effects: ui.effectsToCopy(layer, info.id)));
    } catch (_) {
      return;
    }
    onStackChanged();
  }

  /// The effect's menu at [at]: the same rows in the same order from its
  /// heading in Effect controls, its heading in the Timeline and its box on
  /// the layer's graph. An instance that has gone has none.
  ///
  /// [onStartRename] opens the surface's own name editor, [onFoldAll] twirls
  /// the whole stack, and [compositing] is the Compositing fold where the
  /// heading has one. Each is left out where the surface has no such thing.
  /// [box] is what the box adds, and from the box the reveal goes the other
  /// way, to Effect controls.
  void showMenu(
    BuildContext context,
    Offset at, {
    VoidCallback? onStartRename,
    ValueChanged<bool>? onFoldAll,
    ({bool open, VoidCallback toggle})? compositing,
    EffectBoxRows Function(VoidCallback close)? box,
  }) {
    final stack = instances();
    final index = stack.indexWhere((e) => e.id() == info.id);
    if (index < 0) return;
    final count = stack.length;
    final id = info.id;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final locked =
        ui.model.byId(layer.internallayerId)?.info.switches.locked ?? false;
    final presets = [
      for (final instance in stack)
        if (instance.id() == info.id) ...instance.presetNames(),
    ];
    final saved = material
        ? listMaterialPresets()
        : style || inputs
            ? const <BridgePresetInfo>[]
            : listPresetsFor(name: info.name);
    final pastable = onStack && ui.clipboard.kind == ClipboardKind.effects;
    // Whether the effect is on, as a control of the composition's graphics
    // template. Only an effect of the layer's own stack or styles can be
    // named, and nothing inside a node graph.
    final controls = ui.templateControls;
    final onOff = inputs ||
            material ||
            group != null ||
            clip != null ||
            !controls.offers(comp)
        ? null
        : layerControlRow(BridgeRowKind.effectOn, layer.internallayerId,
            info.customName ?? effectLabelOf(info.name),
            owner: info.id);
    final isControl = onOff != null && controls.controlFor(onOff) != null;
    final copies = !style && !inputs && !material;
    final effectInfo = inputs
        ? null
        : listEffects().where((e) => e.name == info.name).firstOrNull;
    final manual = effectInfo == null
        ? null
        : effectManualUrl(
            name: effectInfo.name,
            category: effectInfo.category,
            label: effectInfo.label,
            namespace: effectInfo.namespace,
          );
    void move(int to) {
      // **The order several picked effects land in**. Each one is
      // taken out of the stack and put back *at* `to`, so the last one moved
      // ends up in front of the others: an upward move therefore takes the
      // bottom-most first and a downward one the topmost, and either way the
      // run arrives together with its own order intact.
      withEach(context, (e) => layer.reorderEffect(effect: e, newIndex: to),
          reversed: to <= index);
      onStackChanged();
    }

    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) {
        void done() => close(null);
        MenuRow row(String name, String label, VoidCallback run,
                {String? action, bool? ticked, bool enabled = true}) =>
            MenuRow(
              key: ValueKey<String>('fx-menu-$name-$id'),
              enabled: enabled,
              onPressed: () {
                done();
                run();
              },
              child: MenuLabel(label,
                  action: action, ticked: ticked, enabled: enabled),
            );
        final extra = box?.call(done);
        return FloatSurface(
          width: 210,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: menuGroups([
              [
                if (!inputs)
                  row('bypass', l10n.fxBypass,
                      () => setEnabled(context, !info.enabled),
                      ticked: !info.enabled, enabled: !locked),
                if (onStartRename case final start?)
                  row('rename', l10n.rename, start,
                      action: 'effect.rename',
                      ticked: inputs ? null : false,
                      enabled: !locked),
                if (!material)
                  row('reset', l10n.reset, () => reset(context),
                      enabled: !atDefault && !locked,
                      ticked: inputs ? null : false),
                if (onOff != null)
                  row(
                      'control',
                      l10n.controlsEffectOnOff,
                      () => isControl
                          ? controls.remove([onOff])
                          : controls.add([onOff]),
                      ticked: isControl),
              ],
              extra?.own ?? const [],
              [
                if (copies) ...[
                  row('cut', l10n.menuCut, () {
                    _copy(context);
                    remove(context);
                  }, action: 'edit.cut', ticked: false, enabled: !locked),
                  row('copy', l10n.menuCopy, () => _copy(context),
                      action: 'edit.copy', ticked: false),
                ],
                if (onStack) ...[
                  row(
                    'paste',
                    l10n.menuPaste,
                    () => _pasteBelow(context, ui.clipboard.text!,
                        ui.playheadFrame.value),
                    action: 'edit.paste',
                    enabled: pastable && !locked,
                    ticked: false,
                  ),
                  row('duplicate', l10n.menuDuplicate,
                      () => duplicate(context),
                      action: 'effect.duplicate',
                      ticked: false,
                      enabled: !locked),
                ],
              ],
              // Only the moves this effect can make are listed: nine named
              // slots in a pinned order have nowhere to move to.
              [
                if (!style && !inputs && count > 1 && !locked)
                  SubmenuRow(
                    key: ValueKey<String>('fx-menu-move-$id'),
                    closeParent: done,
                    submenu: (dismiss) {
                      MenuRow step(String name, String label, int to) => MenuRow(
                            key: ValueKey<String>('fx-menu-$name-$id'),
                            onPressed: () {
                              dismiss();
                              move(to);
                            },
                            child: Text(label),
                          );
                      return FloatSurface(
                        width: 170,
                        child: Column(
                          mainAxisSize: MainAxisSize.min,
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            if (index > 0) ...[
                              step('up', l10n.moveUp, index - 1),
                              step('top', l10n.moveToTop, 0),
                            ],
                            if (index < count - 1) ...[
                              step('down', l10n.moveDown, index + 1),
                              step('bottom', l10n.moveToBottom, count - 1),
                            ],
                          ],
                        ),
                      );
                    },
                    child: Padding(
                      padding: const EdgeInsetsDirectional.only(start: 16),
                      child: Text(l10n.fxMenuMove),
                    ),
                  ),
                ...?extra?.arrange,
              ],
              [
                if ((presets.isNotEmpty || saved.isNotEmpty) && !locked)
                  SubmenuRow(
                    key: ValueKey<String>('fx-menu-presets-$id'),
                    closeParent: done,
                    submenu: (dismiss) => FloatSurface(
                      width: 190,
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (final (i, name) in presets.indexed)
                            MenuRow(
                              key: ValueKey<String>('fx-menu-preset-$id-$i'),
                              onPressed: () {
                                dismiss();
                                _applyPreset(context, i);
                              },
                              child: Text(engineLabel(name)),
                            ),
                          if (presets.isNotEmpty && saved.isNotEmpty)
                            const MenuDivider(),
                          for (final (i, preset) in saved.indexed)
                            MenuRow(
                              key: ValueKey<String>('fx-menu-saved-$id-$i'),
                              onPressed: () {
                                dismiss();
                                if (material) {
                                  _applySaved(
                                      Provider.of<LumitState>(context,
                                          listen: false),
                                      preset.path,
                                      ui.playheadFrame.value);
                                } else {
                                  _applySavedEffect(context, preset.path);
                                }
                              },
                              child: Text(preset.name),
                            ),
                        ],
                      ),
                    ),
                    child: Padding(
                      padding: const EdgeInsetsDirectional.only(start: 16),
                      child: Text(l10n.effectPresets),
                    ),
                  ),
                if ((!style || material) &&
                    !inputs &&
                    group == null &&
                    !geometryOptionsNames.contains(info.name))
                  row('save-preset', l10n.savePresetEllipsis,
                      () => _savePreset(context),
                      ticked: false),
                if (manual case final url?)
                  row('manual', l10n.fxMenuManual, () async {
                    if (await openExternalLink(url)) return;
                    if (context.mounted) {
                      Provider.of<LumitState>(context, listen: false)
                          .postNotice(l10n.couldNotOpenLink(url), error: true);
                    }
                  }, ticked: false),
              ],
              [
                ...?extra?.view,
                if (onStack && !style && !material && !inputs)
                  box == null
                      ? row(
                          'reveal-graph',
                          l10n.fxMenuRevealGraph,
                          () => ui.revealBox(
                              layer, BridgeNodeRef.effect(info.id)),
                          ticked: false)
                      : row('reveal-controls', l10n.graphRevealEffect,
                          () => ui.revealEffectCard(layer, info.id),
                          ticked: false),
              ],
              [
                if (compositing case final fold?)
                  row(
                    'compositing',
                    l10n.fxFoldCompositing,
                    fold.toggle,
                    ticked: fold.open,
                  ),
                if (onFoldAll case final fold?) ...[
                  row('expand', l10n.fxMenuExpandAll, () => fold(true),
                      ticked: inputs ? null : false),
                  row('collapse', l10n.fxMenuCollapseAll, () => fold(false),
                      ticked: inputs ? null : false),
                ],
                ...?extra?.look,
              ],
              [
                if (!inputs)
                  row(
                    'remove',
                    material ? l10n.reset : l10n.delete,
                    () => remove(context),
                    action: onStack ? 'edit.delete.selection' : null,
                    ticked: false,
                    enabled: !locked,
                  ),
              ],
            ]),
          ),
        );
      },
    );
  }
}

/// The rows only an effect's box on the layer's graph has, each list in its
/// place in the effect's menu: [own] after Bypass, Rename and Reset,
/// [arrange] beside Move, [view] before the reveal and [look] before Delete.
typedef EffectBoxRows = ({
  List<Widget> own,
  List<Widget> arrange,
  List<Widget> view,
  List<Widget> look,
});
