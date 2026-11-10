// The layer menu: the one right-click menu a layer has wherever it is shown,
// in the Timeline's outline, on its bar, in the Hierarchy and over the
// picture. Built from the menu bar's own rows, so a row here runs the same
// code as the Layer and Edit menus and wears the same chord.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/streams.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart' show deleteStepName;
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart';
import '../shell/menu_layer_frb.dart';
import '../shell/create_captions_dialog_frb.dart' show createCaptionsFrb;
import '../shell/precompose_dialog_frb.dart' show showPrecomposeDialogFrb;
import '../shell/stretch_dialog_frb.dart';
import 'layer_source_menu.dart';
import 'text_animation_presets.dart' show textPresetRows;
import '../state/beats_notice.dart' show detectCompBeats;
import '../state/dock.dart';
import '../widgets/controls.dart';
import '../widgets/layer_picker.dart';
import 'audio_streams_menu_frb.dart';
import 'layer_fold_frb.dart' show asOneUndoStep, hasFlow;
import 'retime_commands.dart' show reverseRow;
import 'sequence_view_frb.dart';
import 'stagger_panel_frb.dart' show showStaggerPanel;
import 'timeline_extras_frb.dart' show showLabelPicker;
import 'timeline_group_row_frb.dart'
    show groupSelectedLayers, ungroupSelection;
import 'timeline_razor.dart' show splitSelectionAtPlayhead;
import 'transform_rows_frb.dart' show hasThreeDSwitch;

/// Front the Project panel with [item] picked in it. Picking a layer fronts
/// Effect controls, so the pick alone would land behind that tab.
void revealInProject(LumitUiState ui, ItemReference item) {
  setPanelVisible(ui.split, Panel.project, true);
  activatePanelTab(ui.split, Panel.project);
  ui.activePane.value = Panel.project.pane();
  ui.workspace.touch();
  ui.projectPickRequest.value = item;
}

/// A list of menu entries floated at [position], as a right-click menu.
Future<void> showMenuEntriesAt(
  BuildContext context,
  Offset position,
  List<MenuEntry> items, {
  double width = 250,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) =>
          menuEntryList(items, () => close(null), width: width),
    );

/// The layer menu at [position], for [layer]. A layer outside the selection is
/// selected first, so the menu is about the row that was clicked and every
/// row then acts on the whole selection. [onRename] starts the inline rename
/// where the caller has one; without it the row stays out. [view] is what the
/// panel the layer is shown in does with its own rows.
Future<void> showLayerMenuFrb(
  BuildContext context, {
  required LayerReference layer,
  required Offset position,
  VoidCallback? onRename,
  List<MenuEntry> view = const [],
}) {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  if (ui.selectedComp == null) return Future.value();
  final id = layer.internallayerId;
  if (!ui.selectedLayerIds.contains(id)) ui.setSelection([layer]);
  final entry = ui.model.layers
      .where((e) => e.layer.internallayerId == id)
      .firstOrNull;
  if (entry == null) return Future.value();
  return showMenuEntriesAt(
    context,
    position,
    layerMenuEntries(context, app, ui, entry, onRename: onRename, view: view),
  );
}

/// Every row of the layer menu for [entry], the one that was clicked, with the
/// rest of the selection behind it. Own commands first, then the clipboard,
/// then arranging and appearance, with Delete last. A row that could never
/// apply to this layer is left out; one that could with another selection is
/// greyed.
List<MenuEntry> layerMenuEntries(
  BuildContext context,
  LumitState app,
  LumitUiState ui,
  BridgeLayerEntry entry, {
  VoidCallback? onRename,
  List<MenuEntry> view = const [],
}) {
  final comp = ui.selectedComp!;
  final layer = entry.layer;
  final info = entry.info;
  final kind = info.kind;
  final entries = selectedEntries(ui);
  final many = entries.length > 1;
  final locked = info.switches.locked;
  final unlocked = [
    for (final e in entries)
      if (!e.info.switches.locked) e,
  ];

  // A locked layer holds its place and its contents: the rows that would
  // change it grey out, and copying stays, since copying is not editing.
  VoidCallback? editing(VoidCallback? run) =>
      unlocked.isEmpty ? null : run;

  final creates = _createRows(app, ui, entry);
  final source = _sourceItem(layer);

  return [
    if (source case final ItemReference_Composition precomp)
      MenuEntry(l10n.projectOpen, () => ui.openNestedComp(layer, precomp.field0)),
    if (onRename != null)
      MenuEntry(l10n.rename, many || locked ? null : onRename,
          action: 'layer.rename'),
    MenuEntry(l10n.menuLayerSettings,
        () => layerSettingsOf(context, app, ui, layer),
        action: 'layer.settings'),
    MenuEntry.divider(),
    if (view.isNotEmpty) ...[...view, MenuEntry.divider()],
    MenuEntry.submenu(l10n.menuEffect,
        effectMenuEntries(app, ui, [for (final e in unlocked) e.layer])),
    if (_styled(kind))
      MenuEntry.submenu(l10n.menuLayerStyles, styleRows(app, ui)),
    if (_sounds(kind))
      MenuEntry.submenu(
          l10n.menuAudio,
          audioRows(app, ui, entry,
              createCaptions: (l) => unawaited(createCaptionsFrb(context, l)))),
    if (kind != BridgeLayerKind.sequence)
      MenuEntry.submenu(l10n.retime, [
        MenuEntry(
            info.retime == null
                ? l10n.menuEnableRetime
                : l10n.menuDisableRetime,
            editing(() {
              for (final e in unlocked) {
                app.toggleRetime(e.layer);
              }
            }),
            action: 'layer.retime.enable'),
        MenuEntry(l10n.menuStretch,
            editing(() => _stretch(context, app, ui, entry, unlocked))),
        MenuEntry(
            l10n.retimeFreeze,
            editing(() {
              final frame = ui.playheadFrame.value;
              final held = ui.workspace.interface.retimeHoldSeconds;
              final hold = (held * ui.model.fps).round();
              asOneUndoStep(app.project, () {
                for (final e in unlocked) {
                  try {
                    e.layer.freezeAtPlayhead(
                        frame: frame, holdFrames: hold < 1 ? 1 : hold);
                  } catch (_) {}
                }
              });
              app.notifyDocumentChanged();
            }),
            action: 'layer.retime.hold'),
        MenuEntry(l10n.easingReverse, reverseRow(context, ui)),
      ]),
    MenuEntry.submenu(l10n.menuMarkers, markerRows(app, ui)),
    if (kind == BridgeLayerKind.text)
      MenuEntry.submenu(l10n.menuAnimateText, textPresetRows(app, ui)),
    if (canReplaceSource(entry))
      ReplaceSourceEntry(() => replaceSourceRows(app, ui)),
    if (creates.isNotEmpty) MenuEntry.submenu(l10n.menuCreate, creates),
    if (kind == BridgeLayerKind.footage)
      MenuEntry(
          l10n.menuConvertToSequenceLayer,
          editing(() => _convert(app, unlocked, toSequence: true))),
    if (kind == BridgeLayerKind.sequence) ...[
      MenuEntry(
          l10n.menuConvertToFootageLayer,
          editing(() => _convert(app, unlocked, toSequence: false))),
      MenuEntry(l10n.copySequenceShape, () {
        try {
          sequenceShapeClipboard = layer.copySequenceShape();
        } catch (_) {}
      }),
      MenuEntry(
          l10n.pasteSequenceShape,
          sequenceShapeClipboard == null
              ? null
              : editing(() {
                  final shape = sequenceShapeClipboard!;
                  asOneUndoStep(app.project, () {
                    for (final e in unlocked) {
                      if (e.info.kind != BridgeLayerKind.sequence) continue;
                      try {
                        e.layer.pasteSequenceShape(text: shape);
                      } catch (_) {}
                    }
                  });
                  app.notifyDocumentChanged();
                })),
    ],
    MenuEntry.divider(),
    MenuEntry(l10n.menuCut, editing(() => cutSelectionFrb(app, ui)),
        action: 'edit.cut'),
    MenuEntry(l10n.menuCopy, () => copySelectionFrb(ui),
        action: 'edit.copy'),
    MenuEntry(l10n.menuPaste, () => pasteSelectionFrb(app, ui, comp, layer),
        action: 'edit.paste'),
    MenuEntry(
        l10n.menuDuplicate,
        () => duplicateLayersFrb(app, ui, [for (final e in entries) e.layer]),
        action: 'layer.duplicate'),
    MenuEntry(l10n.menuSplitLayer, editing(() {
      if (splitSelectionAtPlayhead(ui, app.project)) {
        app.notifyDocumentChanged();
      }
    }), action: 'layer.split'),
    MenuEntry(
        l10n.menuTrimToWorkArea,
        comp.getWorkArea() == null
            ? null
            : editing(() => _trimToWorkArea(app, ui, comp, unlocked))),
    MenuEntry.divider(),
    MenuEntry.submenu(l10n.menuTransform, transformRows(app, ui)),
    MenuEntry.submenu(l10n.menuArrange, arrangeRows(app, ui)),
    MenuEntry.submenu(l10n.columnParent, _parentRows(app, ui, entry, unlocked)),
    MenuEntry.submenu(l10n.menuBlendingMode, blendRows(app, ui)),
    MenuEntry.submenu(l10n.menuTrackMatte, matteRows(app, ui)),
    MenuEntry(l10n.menuLabelColour, () async {
      final at = _menuOrigin(context);
      final picked =
          await showLabelPicker(context, at, keyPrefix: 'tl-label');
      if (picked == null) return;
      // A label is one of the writes a locked layer still takes.
      asOneUndoStep(app.project, () {
        for (final e in entries) {
          try {
            // The colour it already has makes no undo step.
            if (e.layer.getLabel() != picked) e.layer.setLabel(label: picked);
          } catch (_) {}
        }
      });
      app.notifyDocumentChanged();
    }),
    MenuEntry.divider(),
    MenuEntry.submenu(l10n.columnSwitches, [
      _switchRow(app, ui, comp, entry, entries, l10n.switchHidden,
          BridgeLayerSwitch.visible, !info.switches.visible,
          inverted: true, action: 'layer.toggle.visible'),
      lockRow(app, ui),
      MenuEntry.divider(),
      _switchRow(app, ui, comp, entry, entries, l10n.switchSolo,
          BridgeLayerSwitch.solo, info.switches.solo),
      _switchRow(app, ui, comp, entry, entries, l10n.switchShy,
          BridgeLayerSwitch.shy, info.switches.shy),
      _switchRow(app, ui, comp, entry, entries, l10n.switchGuide,
          BridgeLayerSwitch.guide, info.switches.guide),
      MenuEntry.divider(),
      if (hasThreeDSwitch(kind)) threeDRow(app, ui),
      _switchRow(app, ui, comp, entry, entries, l10n.switchAcceptsLights,
          BridgeLayerSwitch.acceptsLights, info.switches.acceptsLights),
      if (hasFlow(kind)) flowRow(app, ui),
    ]),
    MenuEntry.submenu(l10n.menuSelectSimilar, [
      MenuEntry(
          l10n.menuSelectSameKind,
          () => ui.setSelection([
                for (final e in ui.model.layers)
                  if (e.info.kind == kind) e.layer,
              ])),
      MenuEntry(
          l10n.menuSelectSameLabel,
          () => ui.setSelection([
                for (final e in ui.model.layers)
                  if (e.info.label == info.label) e.layer,
              ])),
    ]),
    MenuEntry.divider(),
    // One row for the light fold: out of the group the selection is in, or
    // into a new one.
    if (ui.model.groups.any((g) => g.members.any(ui.selectedLayerIds.contains)))
      MenuEntry(l10n.menuUngroup, () {
        if (ungroupSelection(comp: comp, layerIds: ui.selectedLayerIds)) {
          app.notifyDocumentChanged();
        }
      }, action: 'layer.ungroup')
    else
      MenuEntry(l10n.menuGroupLayers, () {
        if (groupSelectedLayers(
          comp: comp,
          layerIds: [for (final e in entries) e.layer.internallayerId],
          name: l10n.groupDefaultName,
        )) {
          app.notifyDocumentChanged();
        }
      }, action: 'layer.group'),
    MenuEntry(
        l10n.menuPreCompose,
        editing(() => showPrecomposeDialogFrb(
              context: context,
              comp: comp,
              selectedLayers: [for (final e in unlocked) e.layer],
              ui: ui,
              workspace: ui.workspace,
            )),
        action: 'layer.precompose'),
    if (many) MenuEntry(l10n.menuStaggerLayers, () => showStaggerPanel(ui)),
    if (source != null)
      MenuEntry(l10n.menuRevealInProject, () => revealInProject(ui, source)),
    MenuEntry.divider(),
    MenuEntry(
        l10n.delete,
        editing(() {
          asOneUndoStep(app.project, () {
            for (final e in unlocked) {
              try {
                e.layer.delete();
              } catch (_) {}
            }
          }, name: deleteStepName([for (final e in unlocked) e.info.name]));
          ui.clearSelection();
          app.notifyDocumentChanged();
        }),
        action: 'edit.delete.selection'),
  ];
}

/// The kinds that can carry sound, which is what the Audio rows are about.
bool _sounds(BridgeLayerKind kind) =>
    kind == BridgeLayerKind.footage ||
    kind == BridgeLayerKind.sequence ||
    kind == BridgeLayerKind.precomp ||
    kind == BridgeLayerKind.audio;

/// The kinds whose own flat picture a layer style draws around.
bool _styled(BridgeLayerKind kind) =>
    kind == BridgeLayerKind.footage ||
    kind == BridgeLayerKind.solid ||
    kind == BridgeLayerKind.precomp ||
    kind == BridgeLayerKind.text ||
    kind == BridgeLayerKind.shape ||
    kind == BridgeLayerKind.sequence;

/// Where the menu was opened, for a picker that follows it: the pointer has
/// not moved, so the menu's own anchor is the right place.
Offset _menuOrigin(BuildContext context) {
  final box = context.findRenderObject();
  return box is RenderBox ? box.localToGlobal(Offset.zero) : Offset.zero;
}

/// The project item a layer draws, or null for a layer that is its own
/// source, such as a solid made here, a text or a camera.
ItemReference? _sourceItem(LayerReference layer) {
  try {
    return layer.getSourceItem();
  } catch (_) {
    return null;
  }
}

/// A switch for the whole selection, written as one call and one undo step,
/// every layer taking the clicked row's new state. [inverted] rows name the
/// off state, as Hidden does.
MenuEntry _switchRow(
  LumitState app,
  LumitUiState ui,
  CompositionReference comp,
  BridgeLayerEntry entry,
  List<BridgeLayerEntry> entries,
  String label,
  BridgeLayerSwitch which,
  bool on, {
  bool inverted = false,
  String? action,
}) =>
    MenuEntry(
      label,
      () {
        try {
          comp.setSwitchOnLayers(
            clicked: entry.layer.internallayerId,
            layers: [for (final e in entries) e.layer.internallayerId],
            switch_: which,
            on_: inverted ? on : !on,
          );
          app.notifyDocumentChanged();
        } catch (_) {}
      },
      action: action,
      checked: on,
    );

/// Audio ▸: mute, the layer's sound onto its own row, which streams of its
/// file it plays, and whether a retimed sound keeps its pitch. The Audio
/// heading in the fold has the same rows.
List<MenuEntry> audioRows(
    LumitState app, LumitUiState ui, BridgeLayerEntry entry,
    {void Function(LayerReference layer)? createCaptions}) {
  final layer = entry.layer;
  final locked = entry.info.switches.locked;
  final entries = selectedEntries(ui);
  final comp = ui.selectedComp;
  BridgeAudioStreams? streams;
  if (!locked) {
    try {
      streams = layer.audioStreams();
    } catch (_) {}
  }
  final several = streams?.several ?? false;
  // The ticks are the layer's own choice, kept here so a press shows at once.
  final chosen = {...?streams?.chosen};
  return [
    if (comp != null) ...[
      _switchRow(app, ui, comp, entry, entries, l10n.audioMute,
          BridgeLayerSwitch.audible, !entry.info.switches.audible,
          inverted: true),
      MenuEntry.divider(),
    ],
    if (entry.info.kind != BridgeLayerKind.audio)
      MenuEntry(
        l10n.menuDetachAudio,
        locked
            ? null
            : () async {
                var refused = false;
                for (final e in entries) {
                  if (e.info.switches.locked ||
                      e.info.kind == BridgeLayerKind.audio) {
                    continue;
                  }
                  try {
                    await e.layer.detachAudio();
                  } catch (_) {
                    refused = true;
                  }
                }
                if (refused) app.postNotice(l10n.detachAudioNoSound);
                app.notifyDocumentChanged();
              },
      ),
    MenuEntry(
      l10n.menuKeepPitch,
      locked
          ? null
          : () {
              final follows = !entry.info.pitchFollowsSpeed;
              asOneUndoStep(app.project, () {
                for (final e in entries) {
                  if (e.info.switches.locked) continue;
                  try {
                    e.layer.setPitchFollowsSpeed(follows: follows);
                  } catch (_) {}
                }
              });
              app.notifyDocumentChanged();
            },
      checked: !entry.info.pitchFollowsSpeed,
    ),
    // Composition ▸ Detect beats, from the music's own row.
    if (comp != null)
      MenuEntry(l10n.menuDetectBeats,
          () => detectCompBeats(app, comp, layer: entry.layer)),
    if (streams != null && streams.streams.length > 1) ...[
      MenuEntry.divider(),
      for (final stream in streams.streams)
        MenuEntry.toggle(
          audioStreamLabel(stream),
          () {
            // A layer plays at least one stream. Silence is the mute's job.
            if (chosen.contains(stream.place) && chosen.length == 1) return;
            if (!chosen.remove(stream.place)) chosen.add(stream.place);
            setPickedAudioStreams(
              app.project,
              [
                for (final e in entries)
                  if (!e.info.switches.locked)
                    (
                      streams: () => e.layer.audioStreams(),
                      set: (List<int> places) =>
                          e.layer.setAudioStreams(streams: places),
                    ),
              ],
              chosen.toList()..sort(),
            );
            app.notifyDocumentChanged();
          },
          checked: () => chosen.contains(stream.place),
        ),
    ],
    if (several) ...[
      MenuEntry.divider(),
      MenuEntry(l10n.menuSplitAudioStreams,
          locked ? null : () => _streams(app, entries, split: true)),
      MenuEntry(l10n.menuMergeAudioStreams,
          locked ? null : () => _streams(app, entries, split: false)),
    ],
    if (createCaptions != null) ...[
      MenuEntry.divider(),
      MenuEntry(l10n.menuCreateCaptions, () => createCaptions(layer)),
    ],
  ];
}

/// Split or merge the selection's audio streams, one line in the status line
/// for the whole batch where nothing changed.
void _streams(LumitState app, List<BridgeLayerEntry> entries,
    {required bool split}) {
  var changed = false;
  asOneUndoStep(app.project, () {
    for (final e in entries) {
      if (e.info.switches.locked) continue;
      try {
        changed |=
            split ? e.layer.splitAudioStreams() : e.layer.mergeAudioStreams();
      } catch (_) {}
    }
  });
  if (changed) {
    app.notifyDocumentChanged();
  } else {
    app.postNotice(split
        ? l10n.audioStreamsNothingToSplit
        : l10n.audioStreamsNothingToMerge);
  }
}

/// Create ▸: what this layer has something to make of. The new layers land
/// above the original, as a duplicate does.
List<MenuEntry> _createRows(
    LumitState app, LumitUiState ui, BridgeLayerEntry entry) {
  final kind = entry.info.kind;
  final locked = entry.info.switches.locked;
  final typed = kind == BridgeLayerKind.text;
  var vector = false;
  if (kind == BridgeLayerKind.footage && !locked) {
    try {
      vector = entry.layer.isVectorLayer();
    } catch (_) {}
  }
  Future<void> shapes(Set<BridgeLayerKind> kinds, {bool apart = false}) async {
    final frame = ui.playheadFrame.value;
    for (final e in selectedEntries(ui)) {
      if (e.info.switches.locked || !kinds.contains(e.info.kind)) continue;
      try {
        await e.layer.createShapes(frame: frame, apart: apart);
      } catch (_) {}
    }
    app.notifyDocumentChanged();
  }

  return [
    if (typed)
      MenuEntry(l10n.menuCreateShapesFromText,
          locked ? null : () => shapes({BridgeLayerKind.text})),
    if (vector)
      MenuEntry(l10n.menuCreateShapesFromVector,
          locked ? null : () => shapes({BridgeLayerKind.footage})),
    if (typed || vector || kind == BridgeLayerKind.shape)
      MenuEntry(
          l10n.menuSplitIntoShapeLayers,
          locked
              ? null
              : () => shapes({
                    BridgeLayerKind.text,
                    BridgeLayerKind.footage,
                    BridgeLayerKind.shape,
                  }, apart: true)),
    if (typed)
      MenuEntry(
          l10n.menuCreatePointsFromText,
          locked
              ? null
              : () {
                  asOneUndoStep(app.project, () {
                    for (final e in selectedEntries(ui)) {
                      if (e.info.switches.locked ||
                          e.info.kind != BridgeLayerKind.text) {
                        continue;
                      }
                      try {
                        e.layer.createPointsFromText();
                      } catch (_) {}
                    }
                  });
                  app.notifyDocumentChanged();
                }),
  ];
}

/// Parent ▸: none, then every other layer in the comp, numbered as the
/// outline numbers them, ticked at the one in force.
List<MenuEntry> _parentRows(LumitState app, LumitUiState ui,
    BridgeLayerEntry entry, List<BridgeLayerEntry> unlocked) {
  final all = ui.model.layers;
  void set(UuidValue? parent) {
    final frame = ui.playheadFrame.value;
    asOneUndoStep(app.project, () {
      for (final e in unlocked) {
        // Its own parent can't be, and the one it has makes no undo step.
        if (e.layer.internallayerId == parent || e.info.parent == parent) {
          continue;
        }
        try {
          e.layer.setParent(parent: parent, frame: frame);
        } catch (_) {}
      }
    });
    app.notifyDocumentChanged();
  }

  return [
    MenuEntry(l10n.none, unlocked.isEmpty ? null : () => set(null),
        checked: entry.info.parent == null),
    MenuEntry.divider(),
    for (var i = 0; i < all.length; i++)
      if (all[i].layer.internallayerId != entry.layer.internallayerId)
        MenuEntry(
          numberedLayer(i + 1, all[i].info.name),
          unlocked.isEmpty ? null : () => set(all[i].layer.internallayerId),
          checked: entry.info.parent == all[i].layer.internallayerId,
        ),
  ];
}

/// In and out of the clip-editing surface. A row of several clips refuses to
/// come back out, and says nothing: which clip it would become is not the
/// command's decision.
void _convert(LumitState app, List<BridgeLayerEntry> unlocked,
    {required bool toSequence}) {
  asOneUndoStep(app.project, () {
    for (final e in unlocked) {
      try {
        if (toSequence && e.info.kind == BridgeLayerKind.footage) {
          e.layer.convertToSequenced();
        } else if (!toSequence && e.info.kind == BridgeLayerKind.sequence) {
          e.layer.convertFromSequenced();
        }
      } catch (_) {}
    }
  });
  app.notifyDocumentChanged();
}

/// Stretch…: the dialogue reads the clicked layer's length, and every picked
/// layer is stretched by the same speed, which is the number it asked in.
Future<void> _stretch(BuildContext context, LumitState app, LumitUiState ui,
    BridgeLayerEntry entry, List<BridgeLayerEntry> unlocked) async {
  final percent = await showStretchDialogFrb(
    context: context,
    durationFrames: entry.info.outFrame - entry.info.inFrame,
  );
  if (percent == null) return;
  asOneUndoStep(app.project, () {
    for (final e in unlocked) {
      try {
        e.layer.stretch(speedPercent: percent);
      } catch (_) {}
    }
  });
  app.notifyDocumentChanged();
}

/// Trim to work area: each picked layer's ends pulled in to the work area,
/// keeping where its content sits in time. A layer wholly outside it is left
/// alone, as there is nothing of it to keep.
void _trimToWorkArea(LumitState app, LumitUiState ui,
    CompositionReference comp, List<BridgeLayerEntry> unlocked) {
  final work = comp.getWorkArea();
  if (work == null) return;
  final start = comp.frameAtTime(time: work.inPoint);
  final end = comp.frameAtTime(time: work.outPoint);
  asOneUndoStep(app.project, () {
    for (final e in unlocked) {
      final span = e.info.span;
      final newIn = e.info.inFrame < start ? start : e.info.inFrame;
      final newOut = e.info.outFrame > end ? end : e.info.outFrame;
      if (newOut <= newIn) continue;
      if (newIn == e.info.inFrame && newOut == e.info.outFrame) continue;
      try {
        e.layer.setSpan(
          span: BridgeSpan(
            inPoint: comp.timeOfFrame(frame: newIn),
            outPoint: comp.timeOfFrame(frame: newOut),
            startOffset: span.startOffset,
          ),
        );
      } catch (_) {}
    }
  });
  app.notifyDocumentChanged();
}
