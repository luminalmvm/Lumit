// The Animation menu's built rows (K-244).
//
// In plain terms: everything here acts on **the property rows the Timeline has
// picked** (`selectedProperties`, K-341) and on the keys those rows carry
// under the playhead. That is the whole of the menu's idea of a selection, and
// it is deliberate: the keyframe selection itself belongs to the panel holding
// it and is never published (see `LumitUiState.easingApply`), so a menu that
// claimed to act on "the selected keys" would be guessing. The playhead is a
// place both the menu and the user can see.
//
// The curve maths is not here. `graphChannels` turns a picked path into the
// scalar behind it, `commitChannelEdits` writes a set of them back in the
// fewest ops, and the graph editor's own strip goes through the very same
// pair — so a key eased from this menu and one eased on the strip cannot come
// out different.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/track.dart' show fireEffectAction;

import '../l10n/strings.dart';
import '../panels/graph_channels.dart';
import '../panels/graph_edits.dart';
import '../panels/graph_maths.dart';
import '../panels/text_animator_rows_frb.dart' show addTextAnimator;
import 'expression_dialog_frb.dart';
import 'keyframe_dialogs_frb.dart';
import 'menu_bar_frb.dart' show MenuEntry;
import 'menu_layer_frb.dart' show onSelection;

/// The curves behind the property rows the Timeline has picked.
///
/// Entirely from the read model, so building the menu costs no bridge call —
/// which matters because the tree is rebuilt whenever the selection changes.
List<GraphChannel> selectedChannels(LumitUiState ui) => graphChannels(
      layers: ui.model.layers,
      selected: ui.selectedProperties.value,
    );

/// The first key sitting on the playhead across the picked rows, or null.
///
/// What the dialogues open showing: a set of keys asked about together has to
/// be seeded from one of them, and the first is the one whose row is highest.
({List<BridgeKeyframe> keys, int index})? keyAtPlayhead(LumitUiState ui) {
  final fps = ui.model.fps;
  final frame = ui.playheadFrame.value;
  for (final channel in selectedChannels(ui)) {
    if (channel.isMaskPath) continue;
    final keys = channel.keys;
    for (var i = 0; i < keys.length; i++) {
      if (keyFrame(keys[i], fps).round() == frame) {
        return (keys: keys, index: i);
      }
    }
  }
  return null;
}

/// Rewrite every key sitting on the playhead, across every picked row.
///
/// [change] is handed the whole key list and the index of the one on the
/// playhead, because a side's speed is worked out from its *neighbours* — a
/// key on its own does not know what an automatic tangent should aim at.
///
/// A mask's **shape** row is skipped: a path key holds a whole path rather than
/// a number, and the mask's own control is what plants and eases one (K-340).
/// Returns whether anything was written.
bool editKeysAtPlayhead(
  LumitUiState ui,
  BridgeKeyframe Function(List<BridgeKeyframe> keys, int index) change,
) {
  final fps = ui.model.fps;
  final frame = ui.playheadFrame.value;
  final edits = <GraphChannel, BridgeScalar>{};
  for (final channel in selectedChannels(ui)) {
    if (channel.isMaskPath) continue;
    final keys = channel.keys;
    var touched = false;
    final next = <BridgeKeyframe>[];
    for (var i = 0; i < keys.length; i++) {
      if (keyFrame(keys[i], fps).round() == frame) {
        touched = true;
        next.add(change(keys, i));
      } else {
        next.add(keys[i]);
      }
    }
    if (touched) edits[channel] = BridgeScalar.keyframed(next);
  }
  if (edits.isEmpty) return false;
  commitChannelEdits(edits);
  return true;
}

/// Which of the four a side currently is.
KeyframeInterp interpOf(BridgeSideInterp side) => switch (side) {
      BridgeSideInterp_Hold() => KeyframeInterp.hold,
      BridgeSideInterp_Linear() => KeyframeInterp.linear,
      BridgeSideInterp_Auto() => KeyframeInterp.auto,
      BridgeSideInterp_Bezier() => KeyframeInterp.bezier,
    };

/// How far a side's handle reaches, as the per cent the dialogue shows. A
/// straight or held side has no reach of its own, so it offers the easy-ease
/// third rather than a zero nobody asked for.
double influenceOf(BridgeSideInterp side) => switch (side) {
      BridgeSideInterp_Bezier(:final field0) => field0.influence * 100,
      BridgeSideInterp_Auto(:final field0) => field0.influence * 100,
      _ => 100 / 3,
    };

/// The side [want] names, for the key at [index].
///
/// A side that is *already* a curve keeps the shape it had when it stays one —
/// switching a hand-aimed handle to automatic and back must not throw the ease
/// away, which is exactly what `withTangentMode` files and hands out again. A
/// straight or held side becoming a curve is given the easy-ease reach at the
/// speed its neighbours imply, which is what F9 does.
BridgeSideInterp sideFor(
  KeyframeInterp want,
  List<BridgeKeyframe> keys,
  int index, {
  required bool isOut,
}) {
  final was = isOut ? keys[index].interpOut : keys[index].interpIn;
  final curved = was is BridgeSideInterp_Bezier || was is BridgeSideInterp_Auto
      ? was
      : sideWithInfluence(keys, index, isOut, 100 / 3);
  return switch (want) {
    KeyframeInterp.hold => const BridgeSideInterp.hold(),
    KeyframeInterp.linear => const BridgeSideInterp.linear(),
    KeyframeInterp.bezier => withTangentMode(curved, TangentMode.free),
    KeyframeInterp.auto => withTangentMode(curved, TangentMode.auto),
  };
}

/// [key] with one or both sides replaced.
BridgeKeyframe _keyWith(
  BridgeKeyframe key, {
  BridgeSideInterp? inSide,
  BridgeSideInterp? outSide,
}) =>
    BridgeKeyframe(
      time: key.time,
      value: key.value,
      interpIn: inSide ?? key.interpIn,
      interpOut: outSide ?? key.interpOut,
    );

/// Animation ▸ Set keyframe: plant one at the playhead on every picked row.
///
/// A row with nothing keyed is left alone — turning a static property into an
/// animated one is the stopwatch's job (K-447: there is no auto-key, and the
/// stopwatch is the whole model) — and so is a row that already has a key
/// there, because two keys at one time is not a curve the engine will take.
MenuEntry setKeyframeRow(LumitState app, LumitUiState ui) {
  final channels = selectedChannels(ui);
  return MenuEntry(
    l10n.menuSetKeyframe,
    channels.isEmpty
        ? null
        : () {
            final (fpsNum, fpsDen) = ui.model.fpsExact;
            if (plantKeyOnChannels(
              channels: channels,
              frame: ui.playheadFrame.value.toDouble(),
              fps: ui.model.fps,
              fpsNum: fpsNum,
              fpsDen: fpsDen,
            )) {
              app.notifyDocumentChanged();
            }
          },
  );
}

/// Animation ▸ Toggle hold keyframe: the key under the playhead holds its
/// value until the next one, or goes back to running straight into it.
///
/// The **out** side, which is the one a hold is about: nothing moves *after*
/// this key. Toggling, so the row is one key rather than two.
MenuEntry toggleHoldRow(LumitState app, LumitUiState ui) {
  final at = keyAtPlayhead(ui);
  return MenuEntry(
    l10n.menuToggleHoldKeyframe,
    at == null
        ? null
        : () {
            if (editKeysAtPlayhead(
              ui,
              (keys, i) => _keyWith(
                keys[i],
                outSide: keys[i].interpOut is BridgeSideInterp_Hold
                    ? const BridgeSideInterp.linear()
                    : const BridgeSideInterp.hold(),
              ),
            )) {
              app.notifyDocumentChanged();
            }
          },
    checked: at != null &&
        at.keys[at.index].interpOut is BridgeSideInterp_Hold,
  );
}

/// Animation ▸ Keyframe interpolation…: how the keys under the playhead are
/// approached and left.
MenuEntry keyframeInterpolationRow(
    BuildContext context, LumitState app, LumitUiState ui) {
  final at = keyAtPlayhead(ui);
  return MenuEntry(
    l10n.menuKeyframeInterpolation,
    at == null
        ? null
        : () async {
            final key = at.keys[at.index];
            final asked = await showKeyframeInterpolationFrb(
              context: context,
              inSide: interpOf(key.interpIn),
              outSide: interpOf(key.interpOut),
            );
            if (asked == null) return;
            if (editKeysAtPlayhead(
              ui,
              (keys, i) => _keyWith(
                keys[i],
                inSide: sideFor(asked.inSide, keys, i, isOut: false),
                outSide: sideFor(asked.outSide, keys, i, isOut: true),
              ),
            )) {
              app.notifyDocumentChanged();
            }
          },
  );
}

/// Animation ▸ Keyframe speed…: how far each side's handle reaches (K-505).
///
/// A side that was straight becomes a curve that looks exactly as it did — the
/// only way to give a straight side a reach at all — because the speed comes
/// from the chord it was already lying along.
MenuEntry keyframeSpeedRow(
    BuildContext context, LumitState app, LumitUiState ui) {
  final at = keyAtPlayhead(ui);
  return MenuEntry(
    l10n.menuKeyframeSpeed,
    at == null
        ? null
        : () async {
            final key = at.keys[at.index];
            final asked = await showKeyframeSpeedFrb(
              context: context,
              inPercent: influenceOf(key.interpIn),
              outPercent: influenceOf(key.interpOut),
            );
            if (asked == null) return;
            if (editKeysAtPlayhead(
              ui,
              (keys, i) => _keyWith(
                keys[i],
                inSide:
                    sideWithInfluence(keys, i, false, asked.inPercent),
                outSide:
                    sideWithInfluence(keys, i, true, asked.outPercent),
              ),
            )) {
              app.notifyDocumentChanged();
            }
          },
  );
}

/// Animation ▸ Animate text (K-609): one more animator on every selected Text
/// layer, each arriving with its five property groups and its one range
/// selector already on it.
MenuEntry animateTextRow(LumitState app, LumitUiState ui) => MenuEntry(
      l10n.menuAnimateText,
      onSelection(
        app,
        ui,
        (entry) => addTextAnimator(entry.layer),
        when: (entry) => entry.info.kind == BridgeLayerKind.text,
      ),
    );

/// Animation ▸ Add expression (K-305): a Rhai expression on every picked
/// property row, replacing whatever it held.
MenuEntry addExpressionRow(
    BuildContext context, LumitState app, LumitUiState ui) {
  final channels = [
    for (final channel in selectedChannels(ui))
      if (!channel.isMaskPath) channel,
  ];
  return MenuEntry(
    l10n.menuAddExpression,
    channels.isEmpty
        ? null
        : () async {
            final was = channels.first.scalar;
            final asked = await showExpressionDialogFrb(
              context: context,
              initial:
                  was is BridgeScalar_Expression ? was.field0 : '',
            );
            // Blank changes nothing. Taking an expression *off* a property is
            // the row's own control (docs/07 §4.3, still owed), not a menu row
            // that would have to invent a value to put back.
            if (asked == null || asked.trim().isEmpty) return;
            commitChannelEdits({
              for (final channel in channels)
                channel: BridgeScalar.expression(asked),
            });
            app.notifyDocumentChanged();
          },
  );
}

/// Animation ▸ Track camera (K-417): the tracker is an **effect**, so this
/// applies it and presses its own Analyse.
///
/// Only on the two kinds that have footage to track — a solve is keyed to the
/// unaltered source, and a Null has none. A layer that already carries the
/// effect is analysed again rather than given a second one: the row means
/// "track this", and two trackers on one layer is not a thing to have.
MenuEntry trackCameraRow(LumitState app, LumitUiState ui) => MenuEntry(
      l10n.menuTrackCamera,
      onSelection(
        app,
        ui,
        (entry) {
          if (!entry.info.effects.any((fx) => fx.name == _cameraTrack)) {
            entry.layer.addEffect(name: _cameraTrack);
          }
          for (final fx in entry.layer.getInfo().effects) {
            if (fx.name != _cameraTrack) continue;
            fireEffectAction(
                layer: entry.layer, effect: fx.id, param: 'analyse');
            return;
          }
        },
        when: (entry) =>
            entry.info.kind == BridgeLayerKind.footage ||
            entry.info.kind == BridgeLayerKind.precomp,
      ),
    );

/// The tracker's own name in the effect registry.
const String _cameraTrack = 'camera_track';
