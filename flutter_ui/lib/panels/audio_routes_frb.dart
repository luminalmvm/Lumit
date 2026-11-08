// Where a track's sound goes, and the menu a sound layer wears wherever it is
// shown. The Audio timeline's track head and the Mixer's strip are the same
// object, so both build their menu from these rows.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/routes.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show byEffectLabel;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'timeline_extras_frb.dart' show labelChips;

/// Change one of [layer]'s sends, keeping the rest of its routing. Read at
/// the moment of the write, from a gesture, never from a build.
void writeSend(LayerReference layer, UuidValue bus,
    {BridgeScalar? level, bool? pre}) {
  final info = layer.getInfo();
  layer.setAudioRoutes(
    output: info.output,
    sends: [
      for (final s in info.sends)
        s.bus == bus
            ? BridgeSend(bus: bus, level: level ?? s.level, pre: pre ?? s.pre)
            : s,
    ],
  );
}

/// A picture layer's sound moved on to an Audio layer of its own, and the
/// picture layer muted. A layer with no sound to move says so in the status line.
Future<void> detachLayerAudio(LumitState app, BridgeLayerEntry entry) async {
  try {
    await entry.layer.detachAudio();
  } catch (_) {
    app.postNotice(l10n.detachAudioNoSound);
  }
}

/// The layers a menu opened on [clicked] acts on: everything picked when it is
/// among the picks, and otherwise itself.
List<BridgeLayerEntry> audioMenuTargets(BridgeLayerEntry clicked,
    List<BridgeLayerEntry> layers, Set<UuidValue> picked) {
  if (!picked.contains(clicked.layer.internallayerId)) return [clicked];
  return [
    for (final entry in layers)
      if (picked.contains(entry.layer.internallayerId)) entry,
  ];
}

/// The audio effects, sorted, as a flyout. Opened from a strip, a track head or
/// a clip, and [onAdd] puts the one picked where that row means.
Widget audioEffectFlyout(VoidCallback dismiss, ValueChanged<String> onAdd) {
  final effects = [
    for (final e in listEffects())
      if (e.category == 'audio') e,
  ]..sort(byEffectLabel);
  return FloatSurface(
    width: 200,
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxHeight: 380),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final effect in effects)
              MenuRow(
                key: ValueKey<String>('audio-add-effect-${effect.name}'),
                onPressed: () {
                  dismiss();
                  onAdd(effect.name);
                },
                child: Text(engineLabel(effect.label)),
              ),
          ],
        ),
      ),
    ),
  );
}

/// The menu a sound layer wears, grouped: the layer's own commands, where its
/// sound goes, the copy, its label, and Delete last. [extra] is what only the
/// surface it opened on knows about, such as a track's lane picture or a strip's
/// fader, and stands after the layer's own switches.
List<Widget> audioLayerRows({
  required LumitTheme theme,
  required CompositionReference comp,
  required LumitState app,
  required BridgeLayerEntry entry,
  required List<BridgeLayerEntry> targets,
  required List<BridgeLayerEntry> buses,
  required int frame,
  required VoidCallback closeParent,
  required VoidCallback onRename,
  required VoidCallback onChanged,
  List<Widget> extra = const [],
}) {
  final info = entry.info;
  final switches = info.switches;
  final ids = [for (final t in targets) t.layer.internallayerId];
  final locked = targets.every((t) => t.info.switches.locked);
  final canRename = !switches.locked && targets.length == 1;
  void flip(BridgeLayerSwitch which, bool on) {
    closeParent();
    try {
      comp.setSwitchOnLayers(
          clicked: entry.layer.internallayerId,
          layers: ids,
          switch_: which,
          on_: on);
    } catch (_) {
      // A locked layer refuses, and quietly.
    }
    onChanged();
  }

  void each(void Function(BridgeLayerEntry target) write) {
    asOneUndoStep(app.project, () {
      for (final t in targets) {
        try {
          write(t);
        } catch (_) {
          // A locked layer refuses, and the rest still take it.
        }
      }
    });
    onChanged();
  }

  // A keyed level resets at the playhead, as typing a number would write it.
  bool off(BridgeScalar scalar) =>
      scalar is! BridgeScalar_Static || scalar.field0 != 0;
  final volumeOff = targets.any((t) => off(t.info.volumeDb));
  final panOff = targets.any((t) => off(t.info.pan));

  Widget switchRow(String key, String label, bool ticked, VoidCallback act) =>
      MenuRow(
        key: ValueKey<String>('audio-menu-$key'),
        onPressed: act,
        child: MenuLabel(label, ticked: ticked),
      );

  // A bus goes to the master and has nothing to choose.
  final isBus = info.kind == BridgeLayerKind.bus;
  // Asked as the menu opens, which is a gesture. Sound with a picture is what
  // can stand on a layer of its own.
  final canDetach = !isBus &&
      info.kind != BridgeLayerKind.audio &&
      switches.audible &&
      entry.layer.hasPicture();
  return [
    MenuRow(
      key: const ValueKey<String>('audio-menu-rename'),
      enabled: canRename,
      onPressed: () {
        closeParent();
        onRename();
      },
      child: MenuLabel(l10n.rename,
          action: 'layer.rename', ticked: false, enabled: canRename),
    ),
    switchRow('mute', l10n.audioMute, !switches.audible,
        () => flip(BridgeLayerSwitch.audible, !switches.audible)),
    switchRow('solo', l10n.switchSolo, switches.solo,
        () => flip(BridgeLayerSwitch.solo, !switches.solo)),
    switchRow('bypass', l10n.cutBypassEffects, !switches.fx,
        () => flip(BridgeLayerSwitch.fx, !switches.fx)),
    const MenuDivider(),
    MenuRow(
      key: const ValueKey<String>('audio-menu-reset-volume'),
      enabled: volumeOff,
      onPressed: () {
        closeParent();
        each((t) => t.layer.setVolumeDb(
            value: scalarWithValueAt(t.info.volumeDb, 0, comp, frame)));
      },
      child: MenuLabel(l10n.audioResetVolume, ticked: false, enabled: volumeOff),
    ),
    MenuRow(
      key: const ValueKey<String>('audio-menu-reset-pan'),
      enabled: panOff,
      onPressed: () {
        closeParent();
        each((t) => t.layer.setPan(
            value: scalarWithValueAt(t.info.pan, 0, comp, frame)));
      },
      child: MenuLabel(l10n.audioResetPan, ticked: false, enabled: panOff),
    ),
    ...extra,
    const MenuDivider(),
    if (!isBus)
      ...audioRouteRows(
        entry: entry,
        targets: targets,
        app: app,
        buses: buses,
        closeParent: closeParent,
        onChanged: onChanged,
      ),
    if (canDetach)
      MenuRow(
        key: const ValueKey<String>('audio-menu-detach'),
        onPressed: () async {
          closeParent();
          await detachLayerAudio(app, entry);
          onChanged();
        },
        child: MenuLabel(l10n.menuDetachAudio, ticked: false),
      ),
    SubmenuRow(
      key: const ValueKey<String>('audio-menu-add-effect'),
      closeParent: closeParent,
      submenu: (dismiss) => audioEffectFlyout(dismiss,
          (name) => each((t) => t.layer.addEffect(name: name))),
      child: MenuLabel(l10n.audioAddEffect, ticked: false),
    ),
    const MenuDivider(),
    MenuRow(
      key: const ValueKey<String>('audio-menu-duplicate'),
      enabled: !locked,
      onPressed: () {
        closeParent();
        each((t) => t.layer.duplicate());
      },
      child: MenuLabel(l10n.menuDuplicate,
          action: 'layer.duplicate', ticked: false, enabled: !locked),
    ),
    SubmenuRow(
      key: const ValueKey<String>('audio-menu-label'),
      closeParent: closeParent,
      submenu: (dismiss) => FloatSurface(
        child: labelChips(theme, keyPrefix: 'audio-menu-label', onPick: (label) {
          dismiss();
          each((t) => t.layer.setLabel(label: label));
        }),
      ),
      child: MenuLabel(l10n.tipLabelColour, ticked: false),
    ),
    const MenuDivider(),
    MenuRow(
      key: const ValueKey<String>('audio-menu-delete'),
      enabled: !locked,
      onPressed: () {
        closeParent();
        each((t) => t.layer.delete());
      },
      child: MenuLabel(l10n.delete,
          action: 'edit.delete.selection', ticked: false, enabled: !locked),
    ),
  ];
}

/// The two routing rows: **Output**, the master or one bus, and **Send to**, a
/// tick per bus that adds or takes away a send to it. On several layers the
/// ticks read the one clicked, and a pick is written on all of them.
List<Widget> audioRouteRows({
  required BridgeLayerEntry entry,
  required List<BridgeLayerEntry> targets,
  required LumitState app,
  required List<BridgeLayerEntry> buses,
  required VoidCallback closeParent,
  required VoidCallback onChanged,
}) {
  final info = entry.info;
  Widget tickRow(String key, bool on, String label, VoidCallback act,
          VoidCallback dismiss) =>
      MenuRow(
        key: ValueKey<String>(key),
        onPressed: () {
          dismiss();
          act();
          onChanged();
        },
        child: Row(children: [menuTick(on), Expanded(child: Text(label))]),
      );
  void each(void Function(BridgeLayerEntry target) write) =>
      asOneUndoStep(app.project, () {
        for (final t in targets) {
          if (t.info.kind == BridgeLayerKind.bus) continue;
          try {
            write(t);
          } catch (_) {
            // A locked layer refuses, and the rest still take it.
          }
        }
      });
  return [
    SubmenuRow(
      key: const ValueKey<String>('atl-route-output'),
      closeParent: closeParent,
      submenu: (dismiss) => FloatSurface(
        width: 180,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            tickRow(
                'atl-route-master',
                info.output == null,
                l10n.audioRouteMaster,
                () => each((t) => t.layer
                    .setAudioRoutes(output: null, sends: t.info.sends)),
                dismiss),
            for (final bus in buses)
              tickRow(
                  'atl-route-output-${bus.layer.internallayerId}',
                  info.output == bus.layer.internallayerId,
                  bus.info.name,
                  () => each((t) => t.layer.setAudioRoutes(
                      output: bus.layer.internallayerId, sends: t.info.sends)),
                  dismiss),
          ],
        ),
      ),
      child: MenuLabel(l10n.audioRouteOutput, ticked: false),
    ),
    if (buses.isNotEmpty)
      SubmenuRow(
        key: const ValueKey<String>('atl-route-send'),
        closeParent: closeParent,
        submenu: (dismiss) => FloatSurface(
          width: 180,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final bus in buses)
                () {
                  final id = bus.layer.internallayerId;
                  final on = info.sends.any((s) => s.bus == id);
                  return tickRow(
                      'atl-route-send-$id',
                      on,
                      bus.info.name,
                      () => each((t) => t.layer.setAudioRoutes(
                            output: t.info.output,
                            sends: [
                              for (final s in t.info.sends)
                                if (s.bus != id) s,
                              if (!on)
                                BridgeSend(
                                    bus: id,
                                    level: const BridgeScalar.static_(0),
                                    pre: false),
                            ],
                          )),
                      dismiss);
                }(),
            ],
          ),
        ),
        child: MenuLabel(l10n.audioRouteSendTo, ticked: false),
      ),
  ];
}
