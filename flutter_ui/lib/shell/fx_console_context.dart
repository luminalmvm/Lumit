// What the Ctrl+Space console's radial menu offers, given what is selected
// (K-324) — and the snapshot the console's camera button writes.
//
// **In plain terms.** A radial menu is only worth having if what is in it is
// what you were about to do. So the ring is not one fixed set of commands: it
// is chosen from the selection, in the order the panels themselves are worked
// in. An effect picked out in the stack offers the things you do to an effect;
// a layer selected with no effect picked offers the things you do to *that*
// layer — creation sits one flick further, behind a New slice that expands
// into the Layer ▸ New ring (K-325), never loose beside the selection's own
// actions; a composition open with nothing selected offers the new-layer menu
// directly, because that is what an empty timeline is for; and with no
// composition at all the ring offers the two ways to get one.
//
// Each ring is at most six entries, on purpose. The whole value of a radial
// menu is that a direction becomes muscle memory, and a ring of twelve is a
// ring nobody learns — the long tail belongs in the search bar beside it,
// which is where it is.
//
// This file is kept apart from `fx_console_frb.dart` so the console widget
// stays a thing that draws what it is given: the widget knows nothing about
// the document, and this is where the document knowledge lives.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../main.dart';
import '../panels/effect_param_row_frb.dart' show effectLabelOf;
import '../panels/transform_rows_frb.dart'
    show TransformGroup, transformGroups, read;
import '../src/rust/api/composition.dart';
import '../src/rust/api/effect.dart';
import '../src/rust/api/export.dart';
import '../src/rust/api/layer.dart';
import '../src/rust/api/project_item.dart';
import '../state/dock.dart';
import 'fx_console_frb.dart';
import 'menu_bar_frb.dart';
import 'precompose_dialog_frb.dart';
import 'status_line_frb.dart';

/// What the ring is about, drawn in its middle so the context is never a
/// guess: the picked effect's name, the selected layer's, the composition's,
/// or a plain hint when there is nothing to act on.
String fxConsoleContextTitle(LumitUiState ui) {
  final item = fxConsoleProjectItem(ui);
  if (item != null) {
    // A stale handle (the item deleted, the project switched) degrades to the
    // other contexts rather than taking the console down.
    try {
      return item.name();
    } on Object {
      // Fall through.
    }
  }
  final effect = _pickedEffectName(ui);
  if (effect != null) return effect;
  final layer = ui.selectedLayer.value;
  if (layer != null) {
    final entry = ui.model.byId(layer.internallayerId);
    if (entry != null) return entry.info.name;
  }
  final comp = ui.selectedComp;
  if (comp != null) return comp.getSettings().name;
  return l10n.fxConsoleNothingSelected;
}

/// The Project panel's picked item, counted only while the Project panel is
/// the active one (K-327): the console follows where the user stands, the way
/// the keymap's contexts do. A layer selected in the Timeline keeps meaning
/// "the layer" everywhere else.
ItemReference? fxConsoleProjectItem(LumitUiState ui) =>
    ui.activePanel.value == Panel.project ? ui.selectedProjectItem.value : null;

/// The picked effect's display name, or null when none is picked.
String? _pickedEffectName(LumitUiState ui) {
  final picked = ui.selectedEffects.value;
  final layer = ui.selectedEffectsLayer;
  if (picked.isEmpty || layer == null) return null;
  final entry = ui.model.byId(layer.internallayerId);
  if (entry == null) return null;
  for (final effect in entry.info.effects) {
    if (effect.id == picked.first) return effectLabelOf(effect.name);
  }
  return null;
}

/// The ring for the current selection — see this file's header for the order
/// the four contexts are tried in.
List<RadialEntry> fxConsoleRadial(
  BuildContext context,
  LumitState app,
  LumitUiState ui,
) {
  final comp = ui.selectedComp;
  final layer = ui.selectedLayer.value;
  final picked = ui.selectedEffects.value;
  final effectsLayer = ui.selectedEffectsLayer;

  void done() => app.notifyDocumentChanged();

  // 0. Standing in the Project panel with an item picked: the one thing you
  //    do from there is put the item in the comp (K-327) — never the
  //    new-layer ring this used to fall through to, whose slices had nothing
  //    to do with the selection. The slice stays put when it cannot run —
  //    no comp open, a folder, a comp that would nest into itself — dimmed,
  //    so the direction is learned once and keeps meaning the same thing.
  final item = fxConsoleProjectItem(ui);
  if (item != null) {
    return [_addToCompEntry(item, comp, ui, done)];
  }

  // 1. An effect is picked: what you do to an effect.
  if (picked.isNotEmpty && effectsLayer != null) {
    final entry = ui.model.byId(effectsLayer.internallayerId);
    final instances = entry?.info.effects ?? const [];
    final target = instances.where((e) => e.id == picked.first).firstOrNull;
    return [
      RadialEntry(
        label: target?.enabled ?? true ? l10n.tipDisable : l10n.tipEnable,
        enabled: target != null,
        // Every picked effect, not just the one the label read from
        // (K-523): the ring is raised on a selection, so the slice acts on
        // the selection. They all take the state the label promised, which
        // is also what makes the switch a switch rather than an inverter.
        run: () {
          final wanted = !(target?.enabled ?? true);
          for (final instance in effectsLayer.getEffects()) {
            if (picked.contains(instance.id())) {
              effectsLayer.setEffectEnabled(effect: instance, enabled: wanted);
            }
          }
          done();
        },
      ),
      RadialEntry(
        label: l10n.menuCopy,
        enabled: target != null,
        run: () => copySelectionFrb(ui),
      ),
      RadialEntry(
        label: l10n.tipRemove,
        enabled: target != null,
        run: () {
          for (final instance in effectsLayer.getEffects()) {
            if (picked.contains(instance.id())) {
              effectsLayer.removeEffect(effect: instance);
            }
          }
          done();
        },
      ),
      RadialEntry(
        label: l10n.fxConsoleAddEffect,
        run: () => ui.activePanel.value = Panel.effectsAndPresets,
      ),
    ];
  }

  // The new-layer ring, in the order Layer ▸ New lists them, so the two
  // surfaces teach the same directions for the same things.
  List<RadialEntry> newLayers() => [
        RadialEntry(
          label: l10n.menuSolid,
          run: () {
            comp!.addSolidLayer();
            done();
          },
        ),
        RadialEntry(
          label: l10n.menuText,
          run: () {
            comp!.addTextLayer();
            done();
          },
        ),
        RadialEntry(
          label: l10n.menuCamera,
          run: () {
            comp!.addCameraLayer();
            done();
          },
        ),
        RadialEntry(
          label: l10n.menuAreaLight,
          run: () {
            comp!.addLightLayer(kind: 2);
            done();
          },
        ),
        RadialEntry(
          label: l10n.menuAdjustment,
          run: () {
            comp!.addAdjustmentLayer();
            done();
          },
        ),
        RadialEntry(
          label: l10n.menuNull,
          run: () {
            comp!.addNullLayer();
            done();
          },
        ),
        RadialEntry(
          label: l10n.menuSequence,
          run: () {
            comp!.addSequenceLayer();
            done();
          },
        ),
      ];

  // 2. A layer is selected: what you do to THIS layer — never a grab-bag of
  //    creation commands beside it (K-325). Creating sits one level down,
  //    behind a New slice that expands into the Layer ▸ New ring, so it is
  //    reachable without being mistaken for something about the selection.
  if (layer != null && comp != null) {
    return [
      RadialEntry(
        label: l10n.menuDuplicate,
        // Every selected layer (K-523). Pre-compose two slices along already
        // takes `selectedLayers`; these two read the anchor alone, so the
        // same ring meant "these four" in one direction and "this one" in
        // another.
        run: () {
          for (final l in ui.selectedLayers.value) {
            l.duplicate();
          }
          done();
        },
      ),
      RadialEntry(
        label: l10n.fxConsoleAddEffect,
        run: () => ui.activePanel.value = Panel.effectsAndPresets,
      ),
      RadialEntry(
        label: l10n.menuPreCompose,
        run: () => showPrecomposeDialogFrb(
          context: context,
          comp: comp,
          selectedLayers: ui.selectedLayers.value,
          ui: ui,
          workspace: ui.workspace,
        ),
      ),
      RadialEntry(
        label: l10n.delete,
        run: () {
          for (final l in ui.selectedLayers.value) {
            l.delete();
          }
          ui.clearSelection();
          done();
        },
      ),
      RadialEntry(label: l10n.menuNew, children: newLayers()),
      RadialEntry(
        label: l10n.fxConsoleKeyframe,
        children: fxConsoleKeyframeRing(app, ui, layer, comp),
      ),
    ];
  }

  // 3. A composition, nothing selected in it: the new-layer menu directly,
  //    which is what an empty timeline is asking for — plus Import, the other
  //    way something gets into a comp.
  if (comp != null) {
    return [
      ...newLayers().take(5),
      RadialEntry(
        label: l10n.menuImport,
        run: () => importFootageFrb(app),
      ),
    ];
  }

  // 4. Nothing open at all: the two ways to get somewhere.
  return [
    RadialEntry(
      label: l10n.newComposition,
      run: () => newCompositionFrb(context, app),
    ),
    RadialEntry(
      label: l10n.menuImport,
      enabled: app.project != null,
      run: () => importFootageFrb(app),
    ),
  ];
}

/// The Project item's one slice (K-327): put the picked item in the open
/// comp, exactly as dropping it on the Timeline would — footage becomes a
/// footage layer (honouring the Vegas preference, K-246), a composition
/// nests as a precomp. A folder has nothing to place and a solid has no
/// engine path from the panel yet, so those dim; so does a comp offered to
/// itself, which the engine would refuse. Any engine refusal — a stale
/// handle after a delete, above all — dims rather than throws.
RadialEntry _addToCompEntry(
  ItemReference item,
  CompositionReference? comp,
  LumitUiState ui,
  VoidCallback done,
) {
  var enabled = false;
  VoidCallback run = () {};
  try {
    switch (item) {
      case ItemReference_Footage(:final field0):
        enabled = comp != null;
        run = () {
          comp!.addFootageLayer(
            footage: field0,
            asSequence: ui.workspace.interface.videoAsSequenceLayer,
          );
          done();
        };
      case ItemReference_Composition(:final field0):
        // A comp cannot nest into itself; the slice says so up front rather
        // than no-opping after the flick.
        enabled =
            comp != null && !item.equals(item: ItemReference.composition(comp));
        run = () {
          try {
            comp!.addPrecompLayer(comp: field0);
            done();
          } on Object {
            // The engine refused (a cycle deeper than self-nesting); the
            // document is untouched and there is nothing to report beyond
            // nothing happening.
          }
        };
      case ItemReference_Solid():
      case ItemReference_Folder():
        enabled = false;
    }
  } on Object {
    // A stale handle: the item was deleted, or the project was switched,
    // since the panel published it. Dimmed, not thrown.
    enabled = false;
  }
  return RadialEntry(
    label: l10n.fxConsoleAddToComp,
    enabled: enabled,
    run: run,
  );
}

/// The Keyframe ring (K-326): one slice per transform row, so "key this
/// where I am" is a flick rather than a trip through the fold-out.
///
/// Choosing a slice plants a key at the playhead holding the value already
/// there — nothing moves, exactly as the stopwatch behaves — and then fronts
/// the Timeline with that row open, so the key just made is on screen. A row
/// already keyed at the playhead skips the write and just reveals.
///
/// The five everyday rows, not the 3D extras: a ring is capped at six
/// (docs/07 §12.2), and Rotation X/Y stay the fold-out's business. A row
/// driven by an expression is dimmed rather than dropped — writing keys over
/// an expression would delete it.
List<RadialEntry> fxConsoleKeyframeRing(
  LumitState app,
  LumitUiState ui,
  LayerReference layer,
  CompositionReference comp,
) {
  final entry = ui.model.byId(layer.internallayerId);
  if (entry == null) return const [];
  final transform = entry.info.transform;
  return [
    for (final group in transformGroups(threeD: entry.info.switches.threeD))
      if (group.axes.first.prop.name != 'rotationX' &&
          group.axes.first.prop.name != 'rotationY')
        RadialEntry(
          label: group.label,
          enabled: group.axes.every(
              (a) => read(transform, a.prop) is! BridgeScalar_Expression),
          run: () => _keyTransformGroup(app, ui, layer, comp, group, transform),
        ),
  ];
}

void _keyTransformGroup(
  LumitState app,
  LumitUiState ui,
  LayerReference layer,
  CompositionReference comp,
  TransformGroup group,
  BridgeTransform transform,
) {
  final frame = ui.playheadFrame.value;
  final time = comp.timeOfFrame(frame: frame);
  // A key already under the playhead: nothing to add, only to show. Compared
  // by frame, as the diamond does — the same key to the user either way.
  final lead = read(transform, group.axes.first.prop);
  final onKey = lead is BridgeScalar_Keyframed &&
      lead.field0.any((k) => comp.frameAtTime(time: k.time) == frame);
  if (!onKey) {
    // Every axis of the row keys together, at the value it reads now, so the
    // picture does not move — the row invariant the stopwatch keeps.
    final next = <BridgeScalar>[];
    for (final axis in group.axes) {
      final scalar = read(transform, axis.prop);
      final value = sampleScalar(scalar: scalar, time: time);
      final keys = switch (scalar) {
        BridgeScalar_Keyframed(:final field0) => field0,
        _ => const <BridgeKeyframe>[],
      };
      // In order, as the engine requires — inserted, not appended and hoped.
      final added = [
        ...keys,
        BridgeKeyframe(
          time: time,
          value: value,
          interpIn: const BridgeSideInterp.linear(),
          interpOut: const BridgeSideInterp.linear(),
        ),
      ]..sort((a, b) => comp
          .frameAtTime(time: a.time)
          .compareTo(comp.frameAtTime(time: b.time)));
      next.add(BridgeScalar.keyframed(added));
    }
    layer.setTransforms(
      props: [for (final axis in group.axes) axis.prop],
      values: next,
    );
    app.notifyDocumentChanged();
  }
  // Show the key just made: the Timeline fronted, the row open. The reveal
  // action is the row's axis name — 'anchorX' becomes 'reveal.anchor', the
  // same words the P/S/R/T/A keys use, so one mapping serves both.
  final axis = group.axes.first.prop.name.replaceFirst(RegExp(r'[XYZ]$'), '');
  ui.requestRevealProperty(layer.internallayerId, 'reveal.$axis');
  ui.activePanel.value = Panel.timeline;
}

/// Write the frame on screen to a PNG (K-324).
///
/// It is a one-frame **export**, not a new engine path: the exporter already
/// writes PNGs (`codec: 'png'`, K-201) and is the tested way a Lumit frame
/// becomes a file, so a snapshot is that with the range set to the playhead
/// and the frame after it. The alternative — a second still-writer beside the
/// exporter — is a second thing to keep correct about colour and size for no
/// gain. The engine numbers an image sequence `<stem>.00001.png`, so the frame
/// number lands in the file name whatever this passes.
///
/// Nothing is reported here beyond what the status line already says: the
/// strip polls the exporter and shows the finished path itself, which is the
/// same feedback any other export gives.
void saveSnapshotFrb(LumitState app, LumitUiState ui) {
  final comp = ui.selectedComp;
  if (comp == null) return;
  final path = snapshotPathFor(
    compName: comp.getSettings().name,
    projectPath: app.project?.path(),
  );
  try {
    comp.startExport(
      // Every field of the spec, because a snapshot is an export like any
      // other and the seam carries the whole of one (K-485): the composition's
      // own frame and rate, one frame of range, no sound, and every picture
      // and render setting at what an ordinary export takes.
      spec: BridgeExportSpec(
        preset: '',
        codec: 'png',
        width: 0,
        height: 0,
        bitrateMbps: 0,
        peakMbps: 0,
        bitrateAuto: false,
        fps: 0,
        rangeStartFrame: ui.playheadFrame.value,
        rangeEndFrame: ui.playheadFrame.value + 1,
        includeAudio: false,
        audioBitRate: 0,
        depth: 8,
        alphaChannel: false,
        straightAlpha: false,
        colourSpace: '',
        cropTop: 0,
        cropLeft: 0,
        cropBottom: 0,
        cropRight: 0,
        useRegionOfInterest: false,
        region: Float64List(0),
        metadata: const [],
        qualityDivisor: 1,
        diskCacheReadOnly: false,
        effects: true,
        honourSolo: true,
        makeANoise: false,
        openFolder: false,
      ),
      path: path,
    );
    // Wake the status line, which polls only while an export is live.
    statusLineExportStarted.value++;
  } on Object {
    // An export already running is the everyday refusal, and a calm one: the
    // status line is already showing that export's progress, which answers
    // the question better than a window over a console just dismissed.
  }
}

/// Where a snapshot goes: a `Snapshots` folder beside the saved project, so
/// the stills of a job live with the job. An unsaved project has nowhere of
/// its own, so those land in the user's pictures folder instead — never in
/// whatever directory the application happens to have been started from,
/// which is where a bare file name would put them.
///
/// The composition's name is the file's, with anything a file name cannot
/// carry taken out; the engine appends the frame number.
String snapshotPathFor({
  required String compName,
  String? projectPath,
  Map<String, String>? environment,
}) {
  final env = environment ?? Platform.environment;
  final sep = Platform.pathSeparator;
  final safe = compName.replaceAll(RegExp(r'[^A-Za-z0-9 _-]'), '').trim();
  final stem = safe.isEmpty ? 'snapshot' : safe;

  if (projectPath != null && projectPath.trim().isNotEmpty) {
    final cut = projectPath.lastIndexOf(RegExp(r'[/\\]'));
    if (cut > 0) {
      return '${projectPath.substring(0, cut)}${sep}Snapshots$sep$stem.png';
    }
  }
  final home = env['USERPROFILE'] ?? env['HOME'] ?? '.';
  return '$home${sep}Pictures${sep}Lumit$sep$stem.png';
}
