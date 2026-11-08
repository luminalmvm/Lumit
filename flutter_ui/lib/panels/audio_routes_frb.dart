// Where a track's sound goes, and the menu a sound layer wears wherever it is
// shown. The Audio timeline's track head, the Cut timeline's sound head and the
// Mixer's strip are the same object, so all three build their menu from these
// rows, and the two heads add the same track rows.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/routes.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/beats_notice.dart' show detectCompBeats;
import '../state/comp_time.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show byEffectLabel;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'normalise_commands.dart';
import 'timeline_extras_frb.dart' show labelChips;
import 'volume_band_frb.dart' show BandKind;

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

/// Whether a volume or a pan stands off zero at [frame]. A keyed one resets at
/// the playhead, so one already at zero there has nothing to reset.
bool levelMoved(BridgeScalar scalar, CompositionReference comp, int frame) =>
    (scalar is BridgeScalar_Static
        ? scalar.field0
        : sampledScalar(scalar, timeOfFrame(comp, frame))) !=
    0;

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
/// fader, and stands after the layer's own switches. A [pictureRow] is a picture
/// layer whose sound has yet to be detached, and until it is, nothing that mixes
/// it can act. [renameAction] and [duplicateAction] are the keys that do those
/// two on the surface the menu opened on, null where they do something else.
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
  bool pictureRow = false,
  String? renameAction = 'layer.rename',
  String? duplicateAction = 'layer.duplicate',
  double normaliseTargetDb = -1,
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

  final volumeOff =
      targets.any((t) => levelMoved(t.info.volumeDb, comp, frame));
  final panOff = targets.any((t) => levelMoved(t.info.pan, comp, frame));

  Widget switchRow(String key, String label, bool ticked, VoidCallback act,
          {String? action}) =>
      MenuRow(
        key: ValueKey<String>('audio-menu-$key'),
        onPressed: act,
        child: MenuLabel(label, ticked: ticked, action: action),
      );

  // A bus goes to the master and has nothing to choose.
  final isBus = info.kind == BridgeLayerKind.bus;
  // Asked as the menu opens, which is a gesture. Sound with a picture is what
  // can stand on a layer of its own.
  final canDetach = !isBus &&
      info.kind != BridgeLayerKind.audio &&
      switches.audible &&
      entry.layer.hasPicture();
  final rename = MenuRow(
    key: const ValueKey<String>('audio-menu-rename'),
    enabled: canRename,
    onPressed: () {
      closeParent();
      onRename();
    },
    child: MenuLabel(l10n.rename,
        action: renameAction, ticked: false, enabled: canRename),
  );
  final lock = switchRow('lock', l10n.switchLock, switches.locked,
      () => flip(BridgeLayerSwitch.locked, !switches.locked),
      action: 'layer.lock.toggle');
  final detach = MenuRow(
    key: const ValueKey<String>('audio-menu-detach'),
    onPressed: () async {
      closeParent();
      await detachLayerAudio(app, entry);
      onChanged();
    },
    child: MenuLabel(l10n.menuDetachAudio, ticked: false),
  );
  final delete = MenuRow(
    key: const ValueKey<String>('audio-menu-delete'),
    enabled: !locked,
    onPressed: () {
      closeParent();
      each((t) => t.layer.delete());
    },
    child: MenuLabel(l10n.delete,
        action: 'edit.delete.selection', ticked: false, enabled: !locked),
  );
  if (pictureRow) {
    return [rename, detach, lock, const MenuDivider(), delete];
  }
  return [
    rename,
    switchRow('mute', l10n.audioMute, !switches.audible,
        () => flip(BridgeLayerSwitch.audible, !switches.audible)),
    switchRow('solo', l10n.switchSolo, switches.solo,
        () => flip(BridgeLayerSwitch.solo, !switches.solo)),
    switchRow('bypass', l10n.cutBypassEffects, !switches.fx,
        () => flip(BridgeLayerSwitch.fx, !switches.fx)),
    lock,
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
    if (!isBus)
      MenuRow(
        key: const ValueKey<String>('audio-menu-normalise'),
        onPressed: () async {
          closeParent();
          await normaliseLayers(app, comp, frame, normaliseTargetDb, [
            for (final t in targets)
              if (t.info.kind != BridgeLayerKind.bus) t,
          ], then: onChanged);
        },
        child: MenuLabel(l10n.audioNormalise, ticked: false),
      ),
    // The clip menu's loudness list, over every clip of the picked layers. A
    // layer that is one file has no loudness call yet, so it needs clips.
    if (!isBus && targets.any((t) => t.info.clips.isNotEmpty))
      loudnessRow(
        key: 'audio-menu-loudness',
        close: closeParent,
        tickColumn: true,
        onLoudness: (lufs) => normaliseClips(app, lufs, [
          for (final t in targets)
            if (t.info.kind != BridgeLayerKind.bus)
              for (final c in t.info.clips) (t.layer, c.id),
        ], then: onChanged, loudness: true),
      ),
    // Composition ▸ Detect beats, from the music where a cutter looks for it.
    if (!isBus)
      MenuRow(
        key: const ValueKey<String>('audio-menu-beats'),
        onPressed: () {
          closeParent();
          detectCompBeats(app, comp, layer: entry.layer);
        },
        child: MenuLabel(l10n.menuDetectBeats, ticked: false),
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
    if (canDetach) detach,
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
          action: duplicateAction, ticked: false, enabled: !locked),
    ),
    SubmenuRow(
      key: const ValueKey<String>('audio-menu-label'),
      closeParent: closeParent,
      submenu: (dismiss) => FloatSurface(
        child: labelChips(theme, keyPrefix: 'audio-menu-label', onPick: (label) {
          dismiss();
          each((t) {
            if (t.layer.getLabel() != label) t.layer.setLabel(label: label);
          });
        }),
      ),
      child: MenuLabel(l10n.tipLabelColour, ticked: false),
    ),
    const MenuDivider(),
    delete,
  ];
}

/// A sound clip's own rows, the same in the Cut and Audio timelines: whether its
/// track keeps its pitch, and the picked clips' level brought to the Normalise
/// target or back to 0 dB. A row given no action is left out, except Reset gain
/// where [resetGainShown], which greys while no picked clip is off 0 dB.
/// [tickColumn] leaves room for a tick on every row, for a menu whose rows all
/// keep one. [onLoudness] normalises to a loudness, in LUFS, picked from a list.
List<Widget> soundClipRows({
  required VoidCallback close,
  bool keepPitch = false,
  VoidCallback? onKeepPitch,
  VoidCallback? onNormalise,
  ValueChanged<double>? onLoudness,
  bool resetGainShown = false,
  VoidCallback? onResetGain,
  bool tickColumn = false,
}) {
  Widget row(String key, String label, VoidCallback? act, {bool? ticked}) =>
      MenuRow(
        key: ValueKey<String>('sound-clip-$key'),
        enabled: act != null,
        onPressed: () {
          close();
          act?.call();
        },
        child: MenuLabel(label,
            ticked: ticked ?? (tickColumn ? false : null),
            enabled: act != null),
      );
  return [
    if (onKeepPitch != null)
      row('keep-pitch', l10n.menuKeepPitch, onKeepPitch, ticked: keepPitch),
    if (onNormalise != null) row('normalise', l10n.audioNormalise, onNormalise),
    if (onLoudness != null)
      loudnessRow(
          key: 'sound-clip-loudness',
          close: close,
          tickColumn: tickColumn,
          onLoudness: onLoudness),
    if (resetGainShown) row('reset-gain', l10n.clipResetGain, onResetGain),
  ];
}

/// Normalise loudness \u25b8, a list of targets in LUFS, for a clip and a layer.
Widget loudnessRow({
  required String key,
  required VoidCallback close,
  required ValueChanged<double> onLoudness,
  bool tickColumn = false,
}) =>
    SubmenuRow(
      key: ValueKey<String>(key),
      closeParent: close,
      submenu: (dismiss) => FloatSurface(
        width: 130,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // The common targets for streaming, podcasts and broadcast.
            for (final lufs in const [-14.0, -16.0, -23.0])
              MenuRow(
                key: ValueKey<String>('$key-${-lufs.round()}'),
                onPressed: () {
                  dismiss();
                  onLoudness(lufs);
                },
                child: Text(l10n.audioLoudnessTarget('\u2212${-lufs.round()}')),
              ),
          ],
        ),
      ),
      child: MenuLabel(l10n.audioNormaliseLoudness,
          ticked: tickColumn ? false : null),
    );

/// What a track head adds to [audioLayerRows], the same in the Cut and Audio
/// timelines. A row greys where it cannot act now: a track with no clips has
/// none to select, and the top one cannot go up. [lane] is the choice of
/// picture, where the lane has more than one to show.
List<Widget> soundHeadRows({
  required VoidCallback close,
  required bool Function(BandKind kind) envelopeShown,
  required void Function(BandKind kind) toggleEnvelope,
  Widget? lane,
  required VoidCallback? select,
  required VoidCallback? moveUp,
  required VoidCallback? moveDown,
  required VoidCallback? minimise,
  VoidCallback? resetHeight,
}) {
  Widget row(String key, String label, VoidCallback? act,
          {String? action, bool ticked = false}) =>
      MenuRow(
        key: ValueKey<String>('sound-head-$key'),
        enabled: act != null,
        onPressed: () {
          close();
          act?.call();
        },
        child: MenuLabel(label,
            action: action, ticked: ticked, enabled: act != null),
      );
  return [
    for (final (kind, label, action) in [
      (BandKind.volume, l10n.cutVolumeEnvelope, 'cut.envelope.volume'),
      (BandKind.pan, l10n.cutPanEnvelope, 'cut.envelope.pan'),
    ])
      row(kind.name, label, () => toggleEnvelope(kind),
          action: action, ticked: envelopeShown(kind)),
    if (lane != null) lane,
    const MenuDivider(),
    row('select', l10n.cutSelectTrack, select),
    row('up', l10n.moveUp, moveUp),
    row('down', l10n.moveDown, moveDown),
    row('minimise', l10n.cutMinimiseTracks, minimise,
        action: 'cut.tracks.minimise'),
    if (resetHeight != null) row('height', l10n.cutResetHeight, resetHeight),
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
