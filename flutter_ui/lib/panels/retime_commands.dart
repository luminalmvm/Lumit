// What can be done to a layer's Retime, in one place, for the Retime panel and
// the Retime row's menu. Each command runs over every layer it is given as one
// undo step, and a layer that refuses is passed over.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/cut.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_layer_frb.dart' show selectedEntries;
import '../shell/stretch_dialog_frb.dart';
import '../widgets/controls.dart';
import 'graph_maths.dart';
import 'layer_fold_frb.dart';
import 'retime_popovers.dart';
import 'timeline_extras_frb.dart' show showMenuAt;

/// Whether a Retime means anything on [entry]: footage and precomps have frames
/// to choose between. A row of clips does not, since each clip has its own. One
/// that was given a Retime some other way can still have it taken off.
bool canRetime(BridgeLayerEntry entry) =>
    entry.info.clips.isEmpty &&
    (entry.info.retime != null ||
        entry.info.kind == BridgeLayerKind.footage ||
        entry.info.kind == BridgeLayerKind.precomp);

/// The selected layers a Retime command reaches.
List<BridgeLayerEntry> retimeTargets(LumitUiState ui) => [
      for (final entry in selectedEntries(ui))
        if (canRetime(entry) && !entry.info.switches.locked) entry,
    ];

/// The keys of [entry]'s Retime, or empty when it has none or an expression.
List<BridgeKeyframe> retimeKeysOf(BridgeLayerEntry entry) =>
    switch (entry.info.retime) {
      final BridgeScalar_Keyframed scalar => scalar.field0,
      _ => const [],
    };

/// The speed [entry] plays at across its whole length, in per cent: 100 with
/// no Retime, negative when it runs backwards.
double averageSpeed(BridgeLayerEntry entry, double fps) {
  final keys = retimeKeysOf(entry);
  final length = entry.info.outFrame - entry.info.inFrame;
  if (keys.isEmpty || length <= 0 || fps <= 0) return 100;
  final from = entry.info.inFrame / fps;
  final to = entry.info.outFrame / fps;
  return (evaluateKeys(keys, to) - evaluateKeys(keys, from)) /
      (to - from) *
      100;
}

LumitUiState _uiOf(BuildContext context) =>
    Provider.of<LumitUiState>(context, listen: false);

void _forEach(BuildContext context, List<BridgeLayerEntry> targets,
    void Function(BridgeLayerEntry entry) body) {
  asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
    for (final entry in targets) {
      try {
        body(entry);
      } catch (_) {}
    }
  });
  _uiOf(context).model.refresh();
}

/// Give every target a Retime, or take it away.
void setRetime(BuildContext context, List<BridgeLayerEntry> targets, bool on) =>
    _forEach(context, targets, (entry) {
      if ((entry.info.retime != null) != on) {
        entry.layer.toggleRetimeProperty();
      }
    });

/// Stretch every target to play at [percent] across its whole length. What it
/// plays at now decides how far to go, since a stretch is relative.
void stretchToSpeed(
    BuildContext context, List<BridgeLayerEntry> targets, double percent) {
  final fps = _uiOf(context).model.fps;
  _forEach(context, targets, (entry) {
    final now = averageSpeed(entry, fps).abs();
    if (now < 1e-6) return;
    entry.layer.stretch(speedPercent: percent.abs() / now * 100);
  });
}

/// Stretch every target by the same amount, the one that takes the first to
/// [frames] long.
void stretchToLength(
    BuildContext context, List<BridgeLayerEntry> targets, int frames) {
  if (targets.isEmpty || frames < 1) return;
  final was = targets.first.info.outFrame - targets.first.info.inFrame;
  if (was <= 0) return;
  _forEach(context, targets,
      (entry) => entry.layer.stretch(speedPercent: was / frames * 100));
}

/// Ask for a speed or a length, then stretch.
Future<void> askToStretch(
    BuildContext context, List<BridgeLayerEntry> targets) async {
  if (targets.isEmpty) return;
  final first = targets.first.info;
  final percent = await showStretchDialogFrb(
    context: context,
    durationFrames: first.outFrame - first.inFrame,
  );
  if (percent == null || !context.mounted) return;
  _forEach(
      context, targets, (entry) => entry.layer.stretch(speedPercent: percent));
}

/// A length in seconds as whole frames of this composition, at least one.
int _framesOf(LumitUiState ui, double seconds) {
  final frames = (seconds * ui.model.fps).round();
  return frames < 1 ? 1 : frames;
}

/// Hold the frame at the playhead on every target for the length last asked for.
void freezeAtPlayhead(BuildContext context, List<BridgeLayerEntry> targets) {
  final ui = _uiOf(context);
  final frame = ui.playheadFrame.value;
  final hold = _framesOf(ui, ui.workspace.interface.retimeHoldSeconds);
  _forEach(
      context,
      targets,
      (entry) =>
          entry.layer.freezeAtPlayhead(frame: frame, holdFrames: hold));
}

/// Every target playing at [percent] at the playhead, from a point there.
void setSpeedAtPlayhead(
    BuildContext context, List<BridgeLayerEntry> targets, double percent) {
  final ui = _uiOf(context);
  final (fpsNum, fpsDen) = ui.model.fpsExact;
  final fps = ui.model.fps;
  final at = ui.playheadFrame.value;
  _forEach(context, targets, (entry) {
    if (entry.info.retime == null) entry.layer.toggleRetimeProperty();
    final map = entry.layer.getRetimeProperty();
    if (map is! BridgeScalar_Keyframed) return;
    BridgeRational timeOf(double seconds) =>
        timeOfSubframe(seconds * fps, fpsNum, fpsDen);
    entry.layer.setRetimeProperty(
        value: BridgeScalar.keyframed(
            envelopeWithSpeedAt(map.field0, at / fps, percent, timeOf)));
  });
}

/// Ramp every target from the speed at the playhead to [percent] over [frames].
void rampLayers(BuildContext context, List<BridgeLayerEntry> targets,
    double percent, int frames, SpeedCurve curve) {
  final model = _uiOf(context).model;
  final (fpsNum, fpsDen) = model.fpsExact;
  final fps = model.fps;
  final at = _uiOf(context).playheadFrame.value;
  _forEach(context, targets, (entry) {
    if (entry.info.retime == null) entry.layer.toggleRetimeProperty();
    final map = entry.layer.getRetimeProperty();
    if (map is! BridgeScalar_Keyframed) return;
    entry.layer.setRetimeProperty(
        value: BridgeScalar.keyframed(rampEnvelope(
            map.field0,
            at / fps,
            (at + frames) / fps,
            percent,
            curve,
            (seconds) => timeOfSubframe(seconds * fps, fpsNum, fpsDen))));
  });
}

/// One picked clip, as the Retime panel reads and writes it.
typedef RetimeClip = ({LayerReference layer, BridgeClip clip});

/// The clips picked in the Cut timeline that a Retime command reaches, the
/// pressed one first.
List<RetimeClip> retimeClips(LumitUiState ui) {
  final out = <RetimeClip>[];
  for (final pick in ui.cutClips.value) {
    final entry = ui.model.byId(pick.layer.internallayerId);
    if (entry == null || entry.info.switches.locked) continue;
    for (final clip in entry.info.clips) {
      if (clip.id == pick.clip) out.add((layer: pick.layer, clip: clip));
    }
  }
  return out;
}

/// The speed [clip] plays at across its whole length, in per cent.
double clipAverageSpeed(BridgeClip clip, double fps) {
  final keys = keysOf(clip.retime);
  final length = (clip.endFrame - clip.startFrame).toDouble();
  if (keys.isEmpty || length <= 0 || fps <= 0) return 100;
  return (evaluateKeys(keys, length / fps) - evaluateKeys(keys, 0)) /
      (length / fps) *
      100;
}

void _forEachClip(BuildContext context, List<RetimeClip> clips,
    void Function(LayerReference layer, BridgeClip clip) body) {
  asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
    for (final (:layer, :clip) in clips) {
      try {
        body(layer, clip);
      } catch (_) {}
    }
  });
  Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
  _uiOf(context).model.refresh();
}

/// Every clip at one speed across its length, which keeps its place on the row
/// and plays more or less of its source. A clip running backwards stays so.
void setClipsSpeed(
    BuildContext context, List<RetimeClip> clips, double percent) {
  final fps = _uiOf(context).model.fps;
  _forEachClip(context, clips, (layer, clip) {
    final to = clipAverageSpeed(clip, fps) < 0 ? -percent.abs() : percent.abs();
    layer.setClipSpeed(clip: clip.id, percent: to, endPercent: to);
  });
}

/// Stretch every clip by what takes the first to [frames] long, from its end, as
/// a Shift drag on that edge does. A refusal comes back through the status line.
void stretchClipsToLength(
    BuildContext context, List<RetimeClip> clips, int frames) {
  final ui = _uiOf(context);
  final comp = ui.selectedComp;
  if (clips.isEmpty || frames < 1 || comp == null) return;
  final was = clips.first.clip.endFrame - clips.first.clip.startFrame;
  if (was <= 0) return;
  var refused = BridgeCutResult.done;
  asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
    for (final (layer: _, :clip) in clips) {
      final length = clip.endFrame - clip.startFrame;
      final result = comp.cutStretch(
        clip: clip.id,
        startFrame: clip.startFrame,
        endFrame: clip.startFrame + (length * frames / was).round(),
        ripple: ui.workspace.interface.cutAutoRipple,
        linked: ui.cutLinked.value,
      );
      if (result != BridgeCutResult.done) refused = result;
    }
  });
  ui.model.refresh();
  ui.reportCut(refused);
}

/// Play every clip backwards where it stands. Doing it twice puts it back.
void reverseClips(BuildContext context, List<RetimeClip> clips) {
  final model = _uiOf(context).model;
  final (fpsNum, fpsDen) = model.fpsExact;
  final fps = model.fps;
  _forEachClip(context, clips, (layer, clip) {
    layer.setClipRetime(
        clip: clip.id,
        value: BridgeScalar.keyframed(reversedKeys(keysOf(clip.retime),
            (seconds) => timeOfSubframe(seconds * fps, fpsNum, fpsDen))));
  });
}

/// Back to 100%, as Reset speed on the clip's menu has it.
void resetClips(BuildContext context, List<RetimeClip> clips) => _forEachClip(
    context,
    clips,
    (layer, clip) =>
        layer.setClipSpeed(clip: clip.id, percent: 100, endPercent: 100));

/// Ramp every clip the playhead is inside, from the speed there to [percent]
/// over [frames].
void rampClips(BuildContext context, List<RetimeClip> clips, double percent,
    int frames, SpeedCurve curve) {
  final ui = _uiOf(context);
  final (fpsNum, fpsDen) = ui.model.fpsExact;
  final fps = ui.model.fps;
  final at = ui.playheadFrame.value;
  _forEachClip(context, clips, (layer, clip) {
    if (at < clip.startFrame || at >= clip.endFrame) return;
    final from = (at - clip.startFrame.toInt()) / fps;
    layer.setClipRetime(
        clip: clip.id,
        value: BridgeScalar.keyframed(rampEnvelope(
            keysOf(clip.retime),
            from,
            from + frames / fps,
            percent,
            curve,
            (seconds) => timeOfSubframe(seconds * fps, fpsNum, fpsDen))));
  });
}

/// Hold the frame the first clip the playhead is inside shows, for the length
/// last asked for. Its linked clips are cut with it unless Settings keeps them
/// playing. One undo step, whatever else moves.
void freezeClips(BuildContext context, List<RetimeClip> clips) {
  final ui = _uiOf(context);
  final comp = ui.selectedComp;
  final at = ui.playheadFrame.value;
  if (comp == null) return;
  for (final (layer: _, :clip) in clips) {
    if (at < clip.startFrame || at >= clip.endFrame) continue;
    final settings = ui.workspace.interface;
    var result = BridgeCutResult.nothing;
    try {
      asOneUndoStep(
          Provider.of<LumitState>(context, listen: false).project,
          () => result = comp.cutFreeze(
                clip: clip.id,
                atFrame: at,
                holdFrames: _framesOf(ui, settings.cutFreezeSeconds),
                linked: ui.cutLinked.value && !settings.cutFreezeKeepsSound,
              ),
          name: 'Freeze frame');
    } catch (_) {
      return;
    }
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    ui.model.refresh();
    ui.reportCut(result);
    return;
  }
}

/// Freeze frame… on a clip: ask how long, then hold it, and remember the answer.
Future<void> askToFreezeClips(
    BuildContext context, List<RetimeClip> clips, Offset position) {
  final settings = _uiOf(context).workspace.interface;
  return showHoldPopover(
    context: context,
    position: position,
    seconds: settings.cutFreezeSeconds,
    onHold: (seconds) {
      settings.cutFreezeSeconds = seconds;
      _uiOf(context).workspace.settingsChanged();
      if (context.mounted) freezeClips(context, clips);
    },
  );
}

/// Ramp… on a layer or on clips: ask how, then write it.
Future<void> askToRamp(BuildContext context, List<BridgeLayerEntry> targets,
    List<RetimeClip> clips, Offset position) {
  final ui = _uiOf(context);
  return showRampPopover(
    context: context,
    position: position,
    fps: ui.model.fps,
    settings: ui.workspace.interface,
    saved: ui.workspace.settingsChanged,
    onRamp: (percent, frames, curve) {
      if (!context.mounted) return;
      if (clips.isNotEmpty) {
        rampClips(context, clips, percent, frames, curve);
      } else {
        rampLayers(context, targets, percent, frames, curve);
      }
    },
  );
}

/// Hold here: the picked clips where there are any, else the selected layers.
void holdHere(BuildContext context, LumitUiState ui) {
  final clips = retimeClips(ui);
  if (clips.isNotEmpty) {
    freezeClips(context, clips);
    return;
  }
  final frame = ui.playheadFrame.value;
  final targets = [
    for (final entry in retimeTargets(ui))
      if (canFreezeAt(entry, frame)) entry,
  ];
  if (targets.isNotEmpty) freezeAtPlayhead(context, targets);
}

/// Play every target backwards where it stands, giving it a Retime first where
/// it has none. Doing it twice puts it back.
void reverseLayers(BuildContext context, List<BridgeLayerEntry> targets) {
  final model = _uiOf(context).model;
  final (fpsNum, fpsDen) = model.fpsExact;
  final fps = model.fps;
  _forEach(context, targets, (entry) {
    if (entry.info.retime == null) entry.layer.toggleRetimeProperty();
    final map = entry.layer.getRetimeProperty();
    if (map is! BridgeScalar_Keyframed) return;
    entry.layer.setRetimeProperty(
        value: BridgeScalar.keyframed(reversedKeys(map.field0,
            (seconds) => timeOfSubframe(seconds * fps, fpsNum, fpsDen))));
  });
}

/// Reverse as a menu row on the selection, greyed when nothing picked can be
/// retimed. The layer menu and the Layer menu both use it.
VoidCallback? reverseRow(BuildContext context, LumitUiState ui) {
  final targets = retimeTargets(ui);
  return targets.isEmpty ? null : () => reverseLayers(context, targets);
}

/// One straight run at 100% again, keeping the Retime.
void resetSpeed(BuildContext context, List<BridgeLayerEntry> targets) =>
    _forEach(context, targets, (entry) {
      final keys = retimeKeysOf(entry);
      if (keys.length < 2) return;
      entry.layer.setRetimeProperty(
          value: BridgeScalar.keyframed(resetEnvelope(keys)));
    });

void setInterpolation(BuildContext context, List<BridgeLayerEntry> targets,
        BridgeRetimeInterp interpolation) =>
    _forEach(context, targets,
        (entry) {
      // The one it has already makes no undo step.
      if (entry.layer.getInterpolation() != interpolation) {
        entry.layer.setInterpolation(interpolation: interpolation);
      }
    });

/// The name of a way of finding in-between frames.
String interpLabel(BridgeRetimeInterp interp) => switch (interp) {
      BridgeRetimeInterp.nearest => l10n.interpNearest,
      BridgeRetimeInterp.blend => l10n.interpBlend,
      BridgeRetimeInterp.flow => l10n.flowSection,
    };

/// Whether a playhead at [frame] stands strictly inside [entry], which is where
/// a freeze has something to split.
bool canFreezeAt(BridgeLayerEntry entry, int frame) =>
    frame > entry.info.inFrame && frame < entry.info.outFrame;

/// The menu for [targets]' Retime, at [position], with [more] after its own rows.
void showRetimeMenu(BuildContext context, LumitUiState ui,
    List<BridgeLayerEntry> targets, Offset position,
    {List<Widget> Function(VoidCallback close)? more}) {
  if (targets.isEmpty) return;
  final retimed = targets.first.info.retime != null;
  Widget row(void Function(Object?) close, String label, VoidCallback? run,
          {String? action, String? keyName}) =>
      MenuRow(
        key: keyName == null ? null : ValueKey<String>(keyName),
        enabled: run != null,
        onPressed: () {
          close(null);
          run?.call();
        },
        child: MenuLabel(label, action: action, enabled: run != null),
      );
  final frame = ui.playheadFrame.value;
  showMenuAt<Object>(
    context: context,
    position: position,
    width: 228,
    rows: (close) => [
      row(close, l10n.menuStretch, () => askToStretch(context, targets),
          keyName: 'retime-menu-stretch'),
      row(
          close,
          l10n.retimeFreeze,
          targets.any((e) => canFreezeAt(e, frame))
              ? () => freezeAtPlayhead(context, targets)
              : null,
          action: 'layer.retime.hold',
          keyName: 'retime-menu-freeze'),
      row(
          close,
          l10n.retimeRamp,
          targets.any((e) =>
                  frame >= e.info.inFrame && frame < e.info.outFrame)
              ? () => askToRamp(context, targets, const [], position)
              : null,
          action: 'layer.retime.ramp',
          keyName: 'retime-menu-ramp'),
      row(close, l10n.easingReverse, () => reverseLayers(context, targets),
          keyName: 'retime-menu-reverse'),
      row(close, l10n.clipResetSpeed,
          retimed ? () => resetSpeed(context, targets) : null,
          keyName: 'retime-menu-reset'),
      if (targets.first.info.kind == BridgeLayerKind.footage) ...[
        const MenuDivider(),
        SubmenuRow(
          closeParent: () => close(null),
          submenu: (dismiss) {
            final now = targets.first.layer.getInterpolation();
            return FloatSurface(
              width: 150,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final interp in BridgeRetimeInterp.values)
                    MenuRow(
                      onPressed: () {
                        dismiss();
                        setInterpolation(context, targets, interp);
                      },
                      child:
                          MenuLabel(interpLabel(interp), ticked: interp == now),
                    ),
                ],
              ),
            );
          },
          child: Text(l10n.sourceInBetweenFrames),
        ),
      ],
      const MenuDivider(),
      row(close, retimed ? l10n.menuDisableRetime : l10n.menuEnableRetime,
          () => setRetime(context, targets, !retimed),
          action: 'layer.retime.enable', keyName: 'retime-menu-toggle'),
      if (more != null) ...[
        const MenuDivider(),
        ...more(() => close(null)),
      ],
    ],
  );
}

/// The panel's menu when clips are its subject, the same commands in the same
/// order as a layer's.
void showClipRetimeMenu(BuildContext context, LumitUiState ui,
    List<RetimeClip> clips, Offset position) {
  final frame = ui.playheadFrame.value;
  final inside =
      clips.any((c) => frame >= c.clip.startFrame && frame < c.clip.endFrame);
  final retimed = clips.any((c) => c.clip.retimed);
  Widget row(void Function(Object?) close, String key, String label,
          VoidCallback? run, {String? action}) =>
      MenuRow(
        key: ValueKey<String>(key),
        enabled: run != null,
        onPressed: () {
          close(null);
          run?.call();
        },
        child: MenuLabel(label, action: action, enabled: run != null),
      );
  showMenuAt<Object>(
    context: context,
    position: position,
    width: 228,
    rows: (close) => [
      row(close, 'retime-clip-menu-freeze', l10n.retimeFreeze,
          inside ? () => freezeClips(context, clips) : null,
          action: 'layer.retime.hold'),
      row(close, 'retime-clip-menu-ramp', l10n.retimeRamp,
          inside ? () => askToRamp(context, const [], clips, position) : null,
          action: 'layer.retime.ramp'),
      row(close, 'retime-clip-menu-reverse', l10n.easingReverse,
          () => reverseClips(context, clips)),
      row(close, 'retime-clip-menu-reset', l10n.clipResetSpeed,
          retimed ? () => resetClips(context, clips) : null),
    ],
  );
}

/// The menu for the row's own [layer]: the selection when the layer is in it,
/// else the layer alone.
void showRetimeMenuFor(
    BuildContext context, LayerReference layer, Offset position,
    {List<Widget> Function(VoidCallback close)? more}) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final id = layer.internallayerId;
  final picked = retimeTargets(ui);
  final targets = picked.any((e) => e.layer.internallayerId == id)
      ? picked
      : [
          for (final entry in ui.model.layers)
            if (entry.layer.internallayerId == id && canRetime(entry)) entry,
        ];
  showRetimeMenu(context, ui, targets, position, more: more);
}
