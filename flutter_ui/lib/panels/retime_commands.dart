// What can be done to a layer's Retime, in one place, for the Retime panel and
// the Retime row's menu. Each command runs over every layer it is given as one
// undo step, and a layer that refuses is passed over.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
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
    fps: _uiOf(context).model.fps,
  );
  if (percent == null || !context.mounted) return;
  _forEach(
      context, targets, (entry) => entry.layer.stretch(speedPercent: percent));
}

void freezeAtPlayhead(BuildContext context, List<BridgeLayerEntry> targets) {
  final frame = _uiOf(context).playheadFrame.value;
  _forEach(
      context, targets, (entry) => entry.layer.freezeAtPlayhead(frame: frame));
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
        (entry) => entry.layer.setInterpolation(interpolation: interpolation));

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

/// The menu for [targets]' Retime, at [position].
void showRetimeMenu(BuildContext context, LumitUiState ui,
    List<BridgeLayerEntry> targets, Offset position) {
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
          l10n.menuFreezeFrame,
          targets.any((e) => canFreezeAt(e, frame))
              ? () => freezeAtPlayhead(context, targets)
              : null,
          keyName: 'retime-menu-freeze'),
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
    ],
  );
}

/// The menu for the row's own [layer]: the selection when the layer is in it,
/// else the layer alone.
void showRetimeMenuFor(
    BuildContext context, LayerReference layer, Offset position) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final id = layer.internallayerId;
  final picked = retimeTargets(ui);
  final targets = picked.any((e) => e.layer.internallayerId == id)
      ? picked
      : [
          for (final entry in ui.model.layers)
            if (entry.layer.internallayerId == id && canRetime(entry)) entry,
        ];
  showRetimeMenu(context, ui, targets, position);
}
