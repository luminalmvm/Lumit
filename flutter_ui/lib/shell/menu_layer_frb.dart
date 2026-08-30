// The Layer menu's built rows (K-244): the commands that were listed and
// marked "(Not implemented)" while the engine calls behind them already
// existed.
//
// In plain terms: none of this decides anything. Each row reads what a layer
// currently holds out of the Dart read model (K-184) and writes the changed
// thing back through the layer's own handle — the same calls the Timeline's
// cells make. The menu is a second door onto them, which is the whole of
// K-244: a command with one route is a command that disappears the day
// something intercepts that route.
//
// **Every row acts on the whole selection** (K-523), on each layer that can
// perform it, as one undo step. A row whose command needs something that is
// not there — no mask picked, no layer above to gate with — greys out rather
// than failing when pressed.
//
// They live here rather than in menu_bar_frb.dart because that file is the
// *tree*: the shape of the bar, and how the two renderers draw it. What a
// Layer row does is not part of that shape.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../panels/keyframe_controls_frb.dart'
    show scalarWithValueAt, scaledScalar;
import '../panels/graph_maths.dart' show evaluateKeys;
import '../panels/layer_fold_frb.dart';
import '../panels/timeline_mask_rows_frb.dart' show maskModeLabel, maskWith;
import 'menu_bar_frb.dart' show MenuEntry;
import 'number_dialog_frb.dart';

/// The engine's blend-mode names, read once and held.
///
/// `listBlendModes` is a table of constants, but it is still a bridge call, and
/// the menu tree is rebuilt whenever the selection changes — the same reason
/// the Timeline's own dropdown caches it (K-184's rule about rebuild paths).
List<String>? _blendModes;

List<String> blendModeNames() => _blendModes ??= listBlendModes();

/// What [scalar] reads at [seconds] — the number a dialogue opens showing.
///
/// The same three cases `GraphChannel.valueAt` answers: a static value is
/// itself, a curve is sampled, and an expression computes engine-side and so
/// has nothing to seed a well with.
double scalarValueAt(BridgeScalar scalar, double seconds) => switch (scalar) {
      BridgeScalar_Static(:final field0) => field0,
      BridgeScalar_Keyframed(:final field0) => evaluateKeys(field0, seconds),
      BridgeScalar_Expression() => 0,
    };

/// The selected layers, paired with what the read model says they hold.
///
/// Everything below draws from this rather than from `getInfo()` per row: the
/// model is one bridge call for the whole comp, already made, and a menu that
/// asked the engine per row per rebuild is the cost K-184 exists to remove.
List<BridgeLayerEntry> selectedEntries(LumitUiState ui) {
  final ids = {
    for (final layer in ui.selectedLayers.value) layer.internallayerId,
  };
  if (ids.isEmpty) return const [];
  return [
    for (final entry in ui.model.layers)
      if (ids.contains(entry.layer.internallayerId)) entry,
  ];
}

/// Run [body] over every selected layer as **one** undo step (K-523), then
/// redraw. Null — so the row greys out — with nothing selected.
///
/// A layer that refuses is skipped rather than taking the interface down: a
/// command invoked on a mixed selection does what it can and says nothing,
/// which is what "on all of them that can" means.
VoidCallback? onSelection(
  LumitState app,
  LumitUiState ui,
  void Function(BridgeLayerEntry) body, {
  bool Function(BridgeLayerEntry)? when,
}) {
  final entries = [
    for (final entry in selectedEntries(ui))
      if (when == null || when(entry)) entry,
  ];
  if (entries.isEmpty) return null;
  return () {
    asOneUndoStep(app.project, () {
      for (final entry in entries) {
        try {
          body(entry);
        } catch (_) {
          // The layer went away between the draw and the click, or refused
          // this particular command. The rest of the selection still runs.
        }
      }
    });
    app.notifyDocumentChanged();
  };
}

// ---------------------------------------------------------------------------
// Mask
// ---------------------------------------------------------------------------

/// The mask a Timeline row has picked, with the layer it sits on — or null
/// when no mask row is selected.
///
/// The Timeline publishes its picked property paths (`selectedProperties`,
/// K-341) and a mask's rows live under `<layer>/masks/<id>`, so "which mask do
/// these commands mean" is already answered without the menu inventing a
/// selection of its own.
({LayerReference layer, BridgeMask mask})? pickedMask(LumitUiState ui) {
  for (final path in ui.selectedProperties.value) {
    final layerId = layerIdOfPath(path);
    if (layerId == null) continue;
    final prefix = '${masksPath(layerId)}/';
    if (!path.startsWith(prefix)) continue;
    final id = path.substring(prefix.length).split('/').first;
    for (final entry in ui.model.layers) {
      if (entry.layer.internallayerId.toString() != layerId) continue;
      for (final mask in entry.info.masks) {
        if (mask.id.toString() == id) return (layer: entry.layer, mask: mask);
      }
    }
  }
  return null;
}

/// The shape item a Timeline row has picked, the same way (K-606).
({LayerReference layer, BridgeShapeItem item})? pickedShape(LumitUiState ui) {
  for (final path in ui.selectedProperties.value) {
    final layerId = layerIdOfPath(path);
    if (layerId == null) continue;
    final prefix = '${contentsPath(layerId)}/';
    if (!path.startsWith(prefix)) continue;
    final id = path.substring(prefix.length).split('/').first;
    for (final entry in ui.model.layers) {
      if (entry.layer.internallayerId.toString() != layerId) continue;
      for (final item in entry.info.shapeContents) {
        if (item.id.toString() == id) return (layer: entry.layer, item: item);
      }
    }
  }
  return null;
}

/// Layer ▸ Mask: what to do to the mask whose row is picked.
///
/// Every row is dead — the whole submenu greys — until a mask row is selected
/// in the Timeline, because "the mask" is otherwise a guess. The seven modes
/// stand as rows with a tick rather than behind a *Mode* flyout: picking one is
/// one gesture either way, and a tick beside the mode in force says which it is
/// without opening anything.
List<MenuEntry> maskRows(BuildContext context, LumitState app, LumitUiState ui) {
  final picked = pickedMask(ui);

  void write(BridgeMask next) {
    if (picked == null) return;
    try {
      picked.layer.setMask(mask: next);
      app.notifyDocumentChanged();
    } catch (_) {
      // The mask or its layer went away under the open menu.
    }
  }

  Future<void> ask({
    required String title,
    required BridgeScalar current,
    required double max,
    required BridgeMask Function(BridgeMask, BridgeScalar) put,
  }) async {
    if (picked == null) return;
    final comp = ui.selectedComp;
    final asked = await askNumberFrb(
      context: context,
      title: title,
      value: scalarValueAt(current, ui.playheadFrame.value / ui.model.fps),
      min: 0,
      max: max,
    );
    if (asked == null || comp == null) return;
    write(put(
      picked.mask,
      scalarWithValueAt(current, asked, comp, ui.playheadFrame.value),
    ));
  }

  return [
    for (final mode in BridgeMaskMode.values)
      MenuEntry(
        maskModeLabel(mode),
        picked == null ? null : () => write(maskWith(picked.mask, mode: mode)),
        checked: picked?.mask.mode == mode,
      ),
    MenuEntry.divider(),
    MenuEntry.toggle(
      l10n.tipInverted,
      picked == null
          ? () {}
          : () => write(maskWith(picked.mask, inverted: !picked.mask.inverted)),
      checked: () => pickedMask(ui)?.mask.inverted ?? false,
    ),
    // Feather and expansion are layer pixels off one signed-distance field
    // (K-338); opacity is a per cent. Both are animatable (K-340), so a typed
    // number lands on the key at the playhead when the value is keyed rather
    // than flattening the curve.
    MenuEntry(
      l10n.menuMaskFeather,
      picked == null
          ? null
          : () => ask(
                title: l10n.maskFeather,
                current: picked.mask.feather,
                max: 10000,
                put: (m, v) => maskWith(m, feather: v),
              ),
    ),
    MenuEntry(
      l10n.menuMaskOpacity,
      picked == null
          ? null
          : () => ask(
                title: l10n.maskOpacity,
                current: picked.mask.opacity,
                max: 100,
                put: (m, v) => maskWith(m, opacity: v),
              ),
    ),
  ];
}

/// Layer ▸ Mask and shape path: keying the **shape** itself (K-339, K-606).
///
/// One row, because a path key holds a whole path: it is planted at the
/// playhead holding the shape the item already shows, or the key already there
/// is taken away. Which path it means is the picked row's — a mask's, or a
/// shape item's — so the two live under one heading, as the specification
/// names them.
List<MenuEntry> maskAndShapePathRows(LumitState app, LumitUiState ui) {
  final mask = pickedMask(ui);
  final shape = mask == null ? pickedShape(ui) : null;
  final comp = ui.selectedComp;

  // The playhead's *time* is asked for inside the press, never while the row
  // is drawn: the bar is rebuilt on a selection change, and a composition that
  // has gone under it must leave the menu disabled rather than throwing on the
  // way to being drawn (the rule the dead-reference test holds).
  VoidCallback? toggle() {
    if (comp == null) return null;
    if (mask != null) {
      return () {
        try {
          mask.layer.toggleMaskPathKey(
              id: mask.mask.id,
              time: comp.timeOfFrame(frame: ui.playheadFrame.value));
          app.notifyDocumentChanged();
        } catch (_) {}
      };
    }
    if (shape != null) {
      return () {
        try {
          shape.layer.toggleShapePathKey(
              id: shape.item.id,
              time: comp.timeOfFrame(frame: ui.playheadFrame.value));
          app.notifyDocumentChanged();
        } catch (_) {}
      };
    }
    return null;
  }

  return [MenuEntry(l10n.menuTogglePathKey, toggle())];
}

// ---------------------------------------------------------------------------
// Transform
// ---------------------------------------------------------------------------

/// What a transform property reads on a fresh layer
/// (`lumit_core::model::TransformGroup::default`): nothing anywhere, except
/// the two that mean "unchanged" at a hundred.
double _transformDefault(BridgeTransformProp prop) => switch (prop) {
      BridgeTransformProp.scaleX ||
      BridgeTransformProp.scaleY ||
      BridgeTransformProp.opacity =>
        100,
      _ => 0,
    };

/// Layer ▸ Transform: the four computed writes, and the axis modes.
///
/// **Reset** puts every property back to the value a fresh layer has, animation
/// and all — a reset that kept the keyframes would not be one. The other three
/// keep it: a flip negates the whole curve (`scaledScalar`, the same arithmetic
/// a chained pair's other half is scaled by), and centring writes the middle of
/// the composition onto the key at the playhead when position is keyed, exactly
/// as typing into the row does.
List<MenuEntry> transformRows(LumitState app, LumitUiState ui) {
  final comp = ui.selectedComp;
  final frame = ui.playheadFrame.value;

  void flip(BridgeLayerEntry entry, bool horizontal) {
    final prop = horizontal
        ? BridgeTransformProp.scaleX
        : BridgeTransformProp.scaleY;
    final scalar = horizontal
        ? entry.info.transform.scaleX
        : entry.info.transform.scaleY;
    entry.layer.setTransform(prop: prop, value: scaledScalar(scalar, -1));
  }

  return [
    MenuEntry(
      l10n.reset,
      onSelection(app, ui, (entry) {
        entry.layer.setTransforms(
          props: BridgeTransformProp.values,
          values: [
            for (final prop in BridgeTransformProp.values)
              BridgeScalar.static_(_transformDefault(prop)),
          ],
        );
      }),
    ),
    MenuEntry(l10n.menuFlipHorizontally,
        onSelection(app, ui, (entry) => flip(entry, true))),
    MenuEntry(l10n.menuFlipVertically,
        onSelection(app, ui, (entry) => flip(entry, false))),
    MenuEntry(
      l10n.menuCentreInView,
      comp == null
          ? null
          : onSelection(app, ui, (entry) {
              final settings = comp.getSettings();
              entry.layer.setTransforms(
                props: const [
                  BridgeTransformProp.positionX,
                  BridgeTransformProp.positionY,
                ],
                values: [
                  scalarWithValueAt(entry.info.transform.positionX,
                      settings.width / 2, comp, frame),
                  scalarWithValueAt(entry.info.transform.positionY,
                      settings.height / 2, comp, frame),
                ],
              );
            }),
    ),
    MenuEntry.divider(),
    ...axisModeRows(app, ui),
  ];
}

/// The three pairs, each offering to separate its axes or put them back
/// (K-571, docs/03 §6.5) — Layer ▸ Transform's tail, and the whole of
/// Animation ▸ Separate dimensions, which is the same op named the way After
/// Effects names it.
///
/// The row says what pressing it will do, so a pair already separated offers
/// to combine. Scale's third state — linked, the ratio held — is the Timeline
/// row's own menu and not here: this row is the one question the Animation
/// menu asks.
List<MenuEntry> axisModeRows(LumitState app, LumitUiState ui) {
  BridgeAxisMode modeOf(BridgeLayerEntry entry, BridgeTransformPair pair) =>
      switch (pair) {
        BridgeTransformPair.anchor => entry.info.axisModes.anchor,
        BridgeTransformPair.position => entry.info.axisModes.position,
        BridgeTransformPair.scale => entry.info.axisModes.scale,
      };

  // What the row reads as follows the *first* selected layer: a mixed
  // selection has no single answer, and the leader is the one whose rows are
  // on screen.
  final lead = selectedEntries(ui).firstOrNull;

  return [
    for (final (pair, label) in [
      (BridgeTransformPair.anchor, l10n.transformAnchorPoint),
      (BridgeTransformPair.position, l10n.transformPosition),
      (BridgeTransformPair.scale, l10n.transformScale),
    ])
      MenuEntry(
        lead != null && modeOf(lead, pair) == BridgeAxisMode.separated
            ? l10n.menuCombineAxesOf(label)
            : l10n.menuSeparateAxesOf(label),
        onSelection(app, ui, (entry) {
          final now = modeOf(entry, pair);
          entry.layer.setAxisMode(
            pair: pair,
            mode: now == BridgeAxisMode.separated
                ? BridgeAxisMode.combined
                : BridgeAxisMode.separated,
          );
        }),
      ),
  ];
}

// ---------------------------------------------------------------------------
// Switches, markers, blend and matte
// ---------------------------------------------------------------------------

/// Layer ▸ Flow (K-088): optical flow on this layer, offered only on Footage,
/// which is the only kind with source frames to interpolate between.
///
/// A ticked row rather than a [MenuEntry.toggle]: the tick says what the switch
/// is, but this is a **document edit** and one is enough — K-520's menu that
/// stays open is for the panel switches, which are used several at a time.
MenuEntry flowRow(LumitState app, LumitUiState ui) {
  bool footage(BridgeLayerEntry e) => e.info.kind == BridgeLayerKind.footage;
  return MenuEntry(
    l10n.menuFlow,
    onSelection(
      app,
      ui,
      (entry) => entry.layer.setFlowEnabled(on_: !entry.info.flow),
      when: footage,
    ),
    checked: selectedEntries(ui).where(footage).firstOrNull?.info.flow ?? false,
  );
}

/// Layer ▸ 3D layer (K-023): the switch the Timeline's own column carries,
/// reached from the menu as well.
MenuEntry threeDRow(LumitState app, LumitUiState ui) => MenuEntry(
      l10n.menu3dLayer,
      onSelection(
        app,
        ui,
        (entry) => entry.layer.setSwitch(
          switch_: BridgeLayerSwitch.threeD,
          on_: !entry.info.switches.threeD,
        ),
      ),
      checked:
          selectedEntries(ui).firstOrNull?.info.switches.threeD ?? false,
    );

/// Layer ▸ Markers (K-254): the layer's own cues, which travel with its bar
/// and are its own copy — deleting one never reaches the comp it came from.
List<MenuEntry> markerRows(LumitState app, LumitUiState ui) {
  final comp = ui.selectedComp;
  final frame = ui.playheadFrame.value;
  return [
    MenuEntry(
      l10n.addAtPlayhead,
      comp == null
          ? null
          : onSelection(app, ui, (entry) {
              // A layer marker's time is the layer's **own**, so the playhead
              // has to have the layer's start offset taken back off it — the
              // same carrying the read model does the other way to say which
              // comp frame a marker lands on. At frame granularity, which is
              // all a cue has.
              final offset =
                  comp.frameAtTime(time: entry.layer.getSpan().startOffset);
              // The comp's own placement rule, applied to the layer's list:
              // markers do not stack, so whatever is already on that frame
              // gives way to the newcomer.
              entry.layer.setMarkers(markers: [
                for (final m in entry.info.markers)
                  if (m.frame.toInt() != frame) m.marker,
                BridgeMarker(
                  id: UuidValue.fromString(const Uuid().v4()),
                  time: comp.timeOfFrame(frame: frame - offset),
                  label: '',
                  isBeat: false,
                ),
              ]);
            }),
    ),
    MenuEntry(
      l10n.deleteAllMarkers,
      onSelection(
        app,
        ui,
        (entry) => entry.layer.setMarkers(markers: const []),
        when: (entry) => entry.info.markers.isNotEmpty,
      ),
    ),
  ];
}

/// Layer ▸ Blending mode: the engine's list, ticked at the one in force.
List<MenuEntry> blendRows(LumitState app, LumitUiState ui) {
  final modes = blendModeNames();
  final lead = selectedEntries(ui).firstOrNull;
  return [
    for (var i = 0; i < modes.length; i++)
      MenuEntry(
        engineLabel(modes[i]),
        onSelection(app, ui, (entry) => entry.layer.setBlend(index: i)),
        checked: lead?.info.blend == i,
      ),
  ];
}

/// Layer ▸ Next / Previous blending mode: one step along the same list.
///
/// It stops at the ends rather than wrapping. Stepping is for *trying* the
/// neighbours — a key held down that fell off Luminosity back onto Normal
/// would take the picture somewhere nobody asked for.
MenuEntry blendStepRow(LumitState app, LumitUiState ui, {required int by}) {
  final last = blendModeNames().length - 1;
  return MenuEntry(
    by > 0 ? l10n.menuNextBlendingMode : l10n.menuPreviousBlendingMode,
    onSelection(
      app,
      ui,
      (entry) =>
          entry.layer.setBlend(index: (entry.info.blend + by).clamp(0, last)),
      when: (entry) =>
          by > 0 ? entry.info.blend < last : entry.info.blend > 0,
    ),
  );
}

/// The kinds with a picture to gate another layer with (K-194) — everything
/// except the four that draw no pixels at all.
bool _gates(BridgeLayerEntry entry) => switch (entry.info.kind) {
      BridgeLayerKind.camera ||
      BridgeLayerKind.light ||
      BridgeLayerKind.nullLayer ||
      BridgeLayerKind.audio =>
        false,
      _ => true,
    };

/// Layer ▸ Matte: gate the selected layers with **the layer above** each of
/// them, the way After Effects' own menu does.
///
/// Which layer is not asked: the Timeline's dropdown is where an arbitrary one
/// is chosen (docs/07 §2, item 6.13). What this menu is for is the everyday
/// case — the shape you just drew above the picture — so the four rows are the
/// four readings of that layer, and *No matte* takes it off again.
List<MenuEntry> matteRows(LumitState app, LumitUiState ui) {
  final all = ui.model.layers;

  /// The nearest layer above [entry] that has a picture, or null when there
  /// is none — the top layer of a comp has nothing to be gated by.
  UuidValue? above(BridgeLayerEntry entry) {
    final at = all.indexWhere(
        (e) => e.layer.internallayerId == entry.layer.internallayerId);
    if (at < 0) return null;
    for (var i = at - 1; i >= 0; i--) {
      if (_gates(all[i])) return all[i].layer.internallayerId;
    }
    return null;
  }

  MenuEntry set(String label, {required bool luma, required bool inverted}) =>
      MenuEntry(
        label,
        onSelection(
          app,
          ui,
          (entry) => entry.layer.setMatte(
            matte: BridgeMatte(
                layer: above(entry)!, luma: luma, inverted: inverted),
          ),
          when: (entry) => above(entry) != null,
        ),
        checked: () {
          final matte = selectedEntries(ui).firstOrNull?.info.matte;
          return matte != null &&
              matte.luma == luma &&
              matte.inverted == inverted;
        }(),
      );

  return [
    MenuEntry(
      l10n.noMatte,
      onSelection(app, ui, (entry) => entry.layer.setMatte(),
          when: (entry) => entry.info.matte != null),
      checked: selectedEntries(ui).firstOrNull?.info.matte == null,
    ),
    MenuEntry.divider(),
    set(l10n.menuMatteAlpha, luma: false, inverted: false),
    set(l10n.menuMatteAlphaInverted, luma: false, inverted: true),
    set(l10n.menuMatteLuma, luma: true, inverted: false),
    set(l10n.menuMatteLumaInverted, luma: true, inverted: true),
  ];
}
