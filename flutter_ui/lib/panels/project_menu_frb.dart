// The Project panel's context menus: a row's, the blank ground's and the column
// headings'.
//
// **In plain terms**: the lists that appear when you right-click something in
// the Project panel, and the code that carries out whichever line was picked.
// Each is one function because a menu is one gesture: the rows it draws and the
// work it does after the click are two halves of the same answer.

import 'dart:async';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/share.dart'
    show BridgeFootageHere, BridgeFootageShare;
import 'package:provider/provider.dart';

import '../l10n/engine_labels.dart' show deleteStepName;
import '../l10n/strings.dart';
import '../shell/comp_settings_frb.dart';
import '../shell/dialog_frame.dart' show askLumitChoice;
import '../shell/export_dialog_frb.dart' show showExportDialogFrb;
import '../shell/interpret_footage_frb.dart';
import '../shell/model_settings_dialog.dart'
    show modelLightNames, showDroppedModelSettingsFrb;
import '../shell/status_line_frb.dart';
import '../state/dock.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'project_columns_frb.dart';

enum _ProjectMenuAction {
  open,
  export,
  addToComp,
  revealInComp,
  newCompFromSelection,
  openInSource,
  toggleFolder,
  newFolder,
  copy,
  paste,
  compSettings,
  duplicate,
  interpret,
  rememberInterpretation,
  applyInterpretation,
  rename,
  relink,
  reload,
  reveal,
  editInBlender,
  findMissing,
  addAudioOnly,
  setProxy,
  makeProxy,
  shareStandIn,
  shareOriginal,
  useProxy,
  clearProxy,
  moveToRoot,
  delete
}

/// Do [act] to each of [items] as one undo step, passing over any that has
/// gone, such as a clip deleted with its folder a moment ago.
void eachInOneUndo(LumitState state, List<ItemReference> items,
    void Function(ItemReference item) act,
    {String? name}) {
  final project = state.project;
  final group = (name != null || items.length > 1) && project != null;
  if (group) project.beginUndoGroup();
  if (group && name != null) project.nameUndoGroup(name: name);
  try {
    for (final item in items) {
      try {
        act(item);
      } catch (_) {}
    }
  } finally {
    if (group) project.endUndoGroup();
  }
}

/// Duplicate [items] and hand back the copies. The panel's Duplicate, its
/// `Mod+D` and its paste all come here. A comp asks about the comps nested in
/// it first, the way the Composition menu's Duplicate does.
Future<List<ItemReference>> duplicateProjectItems(
    BuildContext context, List<ItemReference> items) async {
  final state = Provider.of<LumitState>(context, listen: false);
  final copies = <ItemReference>[];
  final project = state.project;
  final group = items.length > 1 && project != null;
  if (group) project.beginUndoGroup();
  try {
    for (final item in items) {
      if (!context.mounted) break;
      if (item case ItemReference_Composition(:final field0)) {
        final copy = await duplicateCompositionFrb(context, field0);
        if (copy != null) copies.add(ItemReference.composition(copy));
        continue;
      }
      try {
        copies.add(item.duplicate());
      } catch (_) {
        // Gone from under the selection.
      }
    }
  } finally {
    if (group) project.endUndoGroup();
  }
  return copies;
}

/// Whether [from] is footage, or a folder with some in it at any depth.
bool _holdsFootage(ItemReference from) {
  if (from is ItemReference_Footage) return true;
  if (from is! ItemReference_Folder) return false;
  try {
    return from.field0.getChildren().any(_holdsFootage);
  } catch (_) {
    return false;
  }
}

/// Delete [items] as one undo step: the row menu's Delete and the panel's
/// `Delete` key. A folder takes what is inside it, and when compositions use
/// any of it the question comes first, since those uses go too. Returns
/// whether anything was deleted.
Future<bool> deleteProjectItems(
    BuildContext context, List<ItemReference> items) async {
  var uses = 0;
  for (final item in items) {
    try {
      uses += item.uses();
    } catch (_) {}
  }
  if (uses > 0) {
    final go = await askLumitChoice<bool>(
      context: context,
      title: l10n.deleteInUseTitle,
      question: l10n.deleteInUseQuestion(uses),
      answers: [(l10n.delete, true)],
    );
    if (go != true || !context.mounted) return false;
  }
  final names = <String>[];
  for (final item in items) {
    try {
      names.add(item.name());
    } catch (_) {
      names.add('');
    }
  }
  eachInOneUndo(Provider.of<LumitState>(context, listen: false), items,
      (item) => item.delete(),
      name: deleteStepName(names));
  return true;
}

/// A colour chip with a word for it, or the bare chip where the colour is its
/// own explanation — which is every chip but the neutral one (A12).
Widget _menuChipTip(String? message, Widget chip) =>
    message == null ? chip : LumitTooltip(message: message, child: chip);

/// How wide the panel's menus are, with room for a row's chord.
const double _menuWidth = 240;

/// A submenu's list, which scrolls when there is more of it than fits.
Widget _flyout(List<Widget> rows) => FloatSurface(
      width: _menuWidth,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 400),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: rows,
          ),
        ),
      ),
    );

/// Put [items] in the open composition, on top of its stack, as a drop on the
/// Timeline does: footage end to end, a comp as a precomp, and a solid as a
/// layer. One undo step. A comp that would hold itself is refused and passed
/// over.
void addItemsToOpenComp(BuildContext context, List<ItemReference> items) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final comp = ui.selectedComp;
  if (comp == null) return;
  final project = Provider.of<LumitState>(context, listen: false).project;
  project?.beginUndoGroup();
  try {
    final footage = [
      for (final item in items)
        if (item case ItemReference_Footage(:final field0)) field0,
    ];
    if (footage.isNotEmpty) {
      final placed = comp.addFootageLayers(
          footage: footage,
          asSequence: ui.workspace.interface.videoAsSequenceLayer,
          row: 0,
          arrival: ui.streamsArrival,
          endToEnd: true,
          lightNames: modelLightNames());
      // A 3D model asks how it sits as it lands.
      showDroppedModelSettingsFrb(context, placed);
    }
    for (final item in items) {
      try {
        switch (item) {
          case ItemReference_Composition(:final field0):
            comp.addPrecompLayer(comp: field0, row: 0);
          case ItemReference_Solid(:final field0):
            comp.addSolidItemLayer(solid: field0, row: 0);
          case _:
        }
      } catch (_) {}
    }
  } finally {
    project?.endUndoGroup();
  }
  ui.model.refresh();
}

/// The project context menu.
///
/// The item's own commands, then the clipboard, then where it is filed and its
/// colour, with Delete last.
Future<void> showProjectMenuFrb({
  required BuildContext context,
  required ItemReference item,
  required bool missing,
  required Offset position,
  required VoidCallback onLocalEdit,
  Future<void> Function()? onRelink,

  /// Narrow the list to what is missing. Null while nothing is.
  VoidCallback? onFindMissing,

  /// Put the row into its in-place rename editor. Null where the menu is
  /// raised from somewhere with no row to edit.
  VoidCallback? onStartRename,

  /// Tag the item. Null where the menu is raised with no row to tag, which is
  /// what makes the Label row absent rather than dead.
  ValueChanged<int>? onSetLabel,

  /// The tag the item wears now, so the strip can mark it.
  int label = 0,

  /// The colour the item would inherit from its folders if it wore none
  /// (A12), or 0 where it would inherit nothing. It is what the neutral chip
  /// means here: *inherit* inside a tagged folder, *no colour* outside one.
  int inherited = 0,

  /// This item's proxy, or null where it has none — which is what
  /// decides whether the menu offers *Use proxy* and *Clear proxy* at all.
  BridgeProxy? proxy,

  /// Where **Set proxy…** gets its path. The panel's own relink seam, so a
  /// test stubs one dialogue for both.
  Future<String?> Function()? proxyPicker,

  /// The folders **Move to folder** offers, name and handle, in the order the
  /// panel lists them. Empty — a project with no folders in it — leaves the
  /// entry off the menu rather than opening onto nothing.
  List<(String, ItemReference)> folders = const [],

  /// File the picked row, and the rest of the selection with it, into that
  /// folder.
  void Function(ItemReference folder)? onMoveToFolder,

  /// Select the items a command has just made. Null leaves the selection.
  void Function(List<ItemReference> items)? onPick,

  /// **What the commands act on**: the whole selection when [item] is
  /// part of it, and [item] alone when it is not — the panel's `_targets`,
  /// which **Move to folder** already took while Delete, Move to root and the
  /// two proxy switches beside it read the clicked row alone.
  ///
  /// Left off (a menu raised with no row behind it) means [item] alone.
  /// **Make proxy stays singular** whatever is passed: a transcode is one at a
  /// time by the engine's own design, and starting four would be refused three
  /// times over.
  List<ItemReference>? targets,

  /// Whether everything [targets] holds is already at the top of the project,
  /// where Move to root has nothing to do.
  bool atRoot = false,

  /// Whether the file has sound, once it has been probed.
  bool hasAudio = true,

  /// Take [targets] for the panel's paste.
  VoidCallback? onCopy,

  /// Paste what was taken. Null while nothing has been.
  VoidCallback? onPaste,

  /// A folder's row: whether it is open, and the twirl.
  bool folderOpen = true,
  VoidCallback? onToggleFolder,

  /// A new folder inside this one.
  VoidCallback? onNewFolder,

  /// Make a comp from these items, the panel's own New composition.
  void Function(List<FootageReference> footage)? onNewComposition,
}) async {
  final isFootage = item is ItemReference_Footage;
  // Where this footage is in a shared project, and who has it. Read once,
  // as the menu opens, and null when the project is not shared.
  BridgeFootageShare? shared;
  if (item case ItemReference_Footage(:final field0)) {
    final sharing =
        Provider.of<LumitState>(context, listen: false).share.active;
    try {
      if (sharing) shared = field0.shareState();
    } catch (_) {
      // The project closed as the menu opened.
    }
  }
  // In a project kept online the footage is on Lumit's server, and what it
  // holds is what the mover last heard. Read once, as the menu opens.
  final online = shared != null &&
      Provider.of<LumitState>(context, listen: false).share.online != null;
  final footageOnline =
      Provider.of<LumitUiState>(context, listen: false).onlineFootage;
  final (standInOnline, originalOnline) = switch (item) {
    ItemReference_Footage(:final field0) when online => (
        footageOnline.standInOnline(field0),
        footageOnline.originalOnline(field0),
      ),
    _ => (false, null),
  };
  final isComp = item is ItemReference_Composition;
  // Read here, as the menu opens: a gesture, not a rebuild.
  final blend = switch (item) {
    ItemReference_Footage(:final field0) when !missing && !kIsWeb =>
      _filePathOf(field0).toLowerCase().endsWith('.blend'),
    _ => false,
  };
  // The comp the sound would land in. Read once, here, rather than in
  // the row's build: the menu is a gesture, not a rebuild path.
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final openComp = ui.selectedComp;
  // The rate a run of stills plays at, null for everything that is not a run —
  // which is what decides whether the field below is offered at all. Read here
  // because a document read belongs to the gesture, not to a rebuild.
  final sequenceRate = switch (item) {
    ItemReference_Footage(:final field0) => field0.sequenceRate(),
    _ => null,
  };
  final picked = targets ?? [item];
  final several = picked.length > 1;
  // Where Reveal in composition can go: a list of them when there are two.
  final usedIn = several || item is ItemReference_Folder
      ? const <(CompositionReference, String)>[]
      : compsUsing(item);
  // Something picked can go in the open comp, which can't hold itself.
  final placeable = openComp != null &&
      picked.any((target) => switch (target) {
            ItemReference_Composition(:final field0) => field0 != openComp,
            ItemReference_Folder() => false,
            _ => true,
          });
  // Read here rather than inside the popup's builder: the popup is raised in
  // its own route, so it has no ThemeScope of the panel's above it.
  final menuTheme = ThemeScope.of(context).theme;
  final action = await showLumitPopup<_ProjectMenuAction>(
    context: context,
    position: position,
    builder: (close) {
      Widget row(
        _ProjectMenuAction act,
        String text, {
        String? key,
        String? chord,
        bool enabled = true,
        bool? ticked,
      }) =>
          MenuRow(
            key: key == null ? null : ValueKey<String>('project-menu-$key'),
            enabled: enabled,
            onPressed: () => close(act),
            child: MenuLabel(text,
                action: chord, enabled: enabled, ticked: ticked),
          );
      return FloatSurface(
        width: _menuWidth,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // What this kind of thing is for.
            if (item is ItemReference_Folder) ...[
              row(_ProjectMenuAction.toggleFolder,
                  folderOpen ? l10n.collapse : l10n.expand,
                  key: 'toggle-folder'),
              row(_ProjectMenuAction.newFolder, l10n.newFolder,
                  key: 'new-folder'),
              row(_ProjectMenuAction.makeProxy, l10n.makeProxies,
                  key: 'make-proxies',
                  enabled: (targets ?? [item]).any(_holdsFootage)),
            ],
            if (isComp) ...[
              row(_ProjectMenuAction.open, l10n.projectOpen,
                  key: 'open', enabled: !several),
              row(_ProjectMenuAction.compSettings,
                  l10n.compositionSettingsEllipsis,
                  key: 'comp-settings',
                  chord: 'comp.settings',
                  enabled: !several),
              row(_ProjectMenuAction.export, l10n.menuExport,
                  key: 'export', enabled: !several),
            ],
            if (item is! ItemReference_Folder) ...[
              row(_ProjectMenuAction.addToComp, l10n.projectAddToComp,
                  key: 'add-to-comp', enabled: placeable),
              if (usedIn.length > 1)
                SubmenuRow(
                  key: const ValueKey('project-menu-reveal-in-comp'),
                  closeParent: () => close(null),
                  submenu: (dismiss) => _flyout([
                    for (final (comp, compName) in usedIn)
                      MenuRow(
                        onPressed: () {
                          dismiss();
                          close(null);
                          revealInComposition(ui, item, comp: comp);
                        },
                        child: Text(compName, overflow: TextOverflow.ellipsis),
                      ),
                  ]),
                  child: Text(l10n.projectRevealInComposition),
                )
              else
                row(_ProjectMenuAction.revealInComp,
                    l10n.projectRevealInComposition,
                    key: 'reveal-in-comp', enabled: usedIn.isNotEmpty),
            ],
            if (isFootage) ...[
              row(_ProjectMenuAction.newCompFromSelection,
                  l10n.projectNewCompFromSelection,
                  key: 'new-comp'),
              if (openComp != null && hasAudio)
                row(_ProjectMenuAction.addAudioOnly, l10n.addAudioOnly,
                    key: 'add-audio-only'),
              if (ui.hasSourceView && !missing)
                row(_ProjectMenuAction.openInSource, l10n.projectOpenInSource,
                    key: 'open-in-source', enabled: !several),
            ],
            // What the item is on disc, which only footage has.
            if (isFootage) ...[
              const MenuDivider(),
              // One command under two names: a broken row is relinked, and a
              // row whose file is there is pointed at another one. The engine
              // takes either, so the menu only chooses the word.
              row(_ProjectMenuAction.relink,
                  missing ? l10n.relinkEllipsis : l10n.replaceFootageEllipsis,
                  key: 'relink', enabled: !several),
              if (onFindMissing != null)
                row(_ProjectMenuAction.findMissing, l10n.findMissingFootage,
                    key: 'find-missing'),
              if (!missing && !kIsWeb) ...[
                row(_ProjectMenuAction.reload, l10n.reloadFootage,
                    key: 'reload'),
                row(_ProjectMenuAction.reveal, l10n.projectShowInFolder,
                    key: 'reveal', enabled: !several),
              ],
              if (blend)
                row(_ProjectMenuAction.editInBlender, l10n.menuEditInBlender,
                    key: 'edit-in-blender'),
              // How the file is read. Remember takes this row's reading and
              // Apply gives the last one remembered to everything picked.
              if (!missing)
                SubmenuRow(
                  key: const ValueKey('project-menu-interpret-group'),
                  closeParent: () => close(null),
                  submenu: (dismiss) => _flyout([
                    row(_ProjectMenuAction.interpret,
                        l10n.interpretFootageEllipsis,
                        key: 'interpret', chord: 'footage.interpret'),
                    row(_ProjectMenuAction.rememberInterpretation,
                        l10n.rememberInterpretation,
                        key: 'remember-interpretation',
                        chord: 'footage.interpretation.remember'),
                    row(_ProjectMenuAction.applyInterpretation,
                        l10n.applyInterpretation,
                        key: 'apply-interpretation',
                        chord: 'footage.interpretation.apply',
                        enabled: rememberedInterpretation != null),
                  ]),
                  child: Text(l10n.projectInterpret),
                ),
              // Four commands and no dialogue: attach a file, make one, read
              // from it or not, forget it. The last two only once there is a
              // proxy, so the menu never lists a word that would do nothing.
              SubmenuRow(
                key: const ValueKey('project-menu-proxy-group'),
                closeParent: () => close(null),
                submenu: (dismiss) => _flyout([
                  row(_ProjectMenuAction.setProxy, l10n.setProxyEllipsis,
                      key: 'set-proxy',
                      enabled: !several,
                      ticked: proxy == null ? null : false),
                  row(_ProjectMenuAction.makeProxy, l10n.makeProxy,
                      key: 'make-proxy', ticked: proxy == null ? null : false),
                  if (proxy != null) ...[
                    const MenuDivider(),
                    row(_ProjectMenuAction.useProxy, l10n.useProxy,
                        key: 'use-proxy', ticked: proxy.enabled),
                    row(_ProjectMenuAction.clearProxy, l10n.clearProxy,
                        key: 'clear-proxy', ticked: false),
                  ],
                ]),
                child: Text(l10n.projectProxy),
              ),
            ],
            // **The rate of a run of stills** (docs/07 §3.1). Stills carry no
            // rate of their own, so an imported run plays at 25 until this
            // field says otherwise, and it is the one part of a sequence the
            // project stores. A field on the menu rather than a submenu of
            // choices: a run corrected from a 3D render is as likely to want
            // 12 as 24, and typing it is one gesture where hunting a list is
            // two.
            if (item case ItemReference_Footage(:final field0))
              if (sequenceRate case final rate?)
                Padding(
                  key: const ValueKey('project-menu-sequence-rate'),
                  padding:
                      const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                  child: Row(
                    children: [
                      // The label takes whatever the well and its unit leave,
                      // so a longer word in another language shortens rather
                      // than pushing the field off the menu.
                      Expanded(
                        child: Text(l10n.sequenceRate, style: menuTheme.small),
                      ),
                      _SequenceRateField(
                        theme: menuTheme,
                        rate: rate,
                        onRate: (fpsNum, fpsDen) {
                          close(null);
                          field0.setSequenceRate(fpsNum: fpsNum, fpsDen: fpsDen);
                          onLocalEdit();
                        },
                      ),
                      const SizedBox(width: 6),
                      Text(l10n.unitFps, style: menuTheme.small),
                    ],
                  ),
                ),
            // Footage this computer has not got, in a project kept online: it
            // comes from Lumit's server, when the server has it.
            if (shared != null &&
                online &&
                shared.here != BridgeFootageHere.original) ...[
              if (shared.here == BridgeFootageHere.missing && standInOnline)
                row(_ProjectMenuAction.shareStandIn, l10n.shareGetStandIn,
                    key: 'share-stand-in'),
              if (originalOnline != null)
                row(
                    _ProjectMenuAction.shareOriginal,
                    l10n.shareGetOriginal((originalOnline / (1 << 20)).ceil()),
                    key: 'share-original')
              else
                MenuRow(
                  key: const ValueKey('project-menu-share-original'),
                  onPressed: () {},
                  enabled: false,
                  child: Text(l10n.shareOnlineNoOriginal),
                ),
            ],
            // Footage this computer has not got, in a shared project where
            // someone has: ask them for a stand-in to cut with, or the file.
            if (shared != null &&
                shared.holders.isNotEmpty &&
                shared.here != BridgeFootageHere.original) ...[
              if (shared.here == BridgeFootageHere.missing)
                row(_ProjectMenuAction.shareStandIn, l10n.shareGetStandIn,
                    key: 'share-stand-in'),
              row(
                  _ProjectMenuAction.shareOriginal,
                  l10n.shareGetOriginal(
                      (shared.bytes.toInt() / (1 << 20)).ceil()),
                  key: 'share-original'),
            ],
            const MenuDivider(),
            row(_ProjectMenuAction.rename, l10n.rename,
                key: 'rename', chord: 'item.rename', enabled: !several),
            row(_ProjectMenuAction.copy, l10n.menuCopy,
                key: 'copy', chord: 'edit.copy'),
            row(_ProjectMenuAction.paste, l10n.menuPaste,
                key: 'paste', chord: 'edit.paste', enabled: onPaste != null),
            row(_ProjectMenuAction.duplicate, l10n.menuDuplicate,
                key: 'duplicate', chord: 'item.duplicate'),
            const MenuDivider(),
            // Filing, the mouse way: the drag onto a folder row is the other.
            if (onMoveToFolder != null && folders.isNotEmpty)
              SubmenuRow(
                key: const ValueKey('project-menu-move-to-folder'),
                closeParent: () => close(null),
                submenu: (dismiss) => _flyout([
                  for (final (name, folder) in folders)
                    MenuRow(
                      key: ValueKey<String>(
                          'project-menu-folder-${projectItemId(folder)}'),
                      onPressed: () {
                        dismiss();
                        onMoveToFolder(folder);
                      },
                      child: Text(name, overflow: TextOverflow.ellipsis),
                    ),
                ]),
                child: Text(l10n.moveToFolder),
              ),
            if (!atRoot)
              row(_ProjectMenuAction.moveToRoot, l10n.moveToRoot,
                  key: 'move-to-root'),
            // The colour tag, as the strip itself rather than a submenu: the
            // chips ARE the choice, so putting them on the menu row costs one
            // click where a submenu costs two and a hover in between. The same
            // shape the Timeline's layer swatch offers, and the same palette.
            if (onSetLabel != null)
              Padding(
                padding:
                    const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                child: Row(
                  children: [
                    Expanded(child: Text(l10n.label, style: menuTheme.body)),
                    for (var i = 0; i < LumitTheme.labelCount; i++)
                      _menuChipTip(
                        // The neutral chip is the way back, and what it goes
                        // back *to* depends on where the item sits (A12):
                        // inside a tagged folder it hands the row back to the
                        // folder's colour, outside one it leaves the row with
                        // none. Same chip, two words. The colours themselves
                        // say what they are, so only this one is named.
                        i == 0
                            ? (inherited != 0
                                ? l10n.tipLabelInherit
                                : l10n.tipLabelNoColour)
                            : null,
                        HoverMark(
                          swells: true,
                          builder: (context, lit) => GestureDetector(
                            key: ValueKey<String>('project-menu-label-$i'),
                            onTap: () {
                              onSetLabel(i);
                              close(null);
                            },
                            child: Container(
                              width: 10,
                              height: 10,
                              margin: const EdgeInsetsDirectional.only(start: 2),
                              decoration: BoxDecoration(
                                color: menuTheme.labelColour(i),
                                shape: BoxShape.circle,
                                border: i == label
                                    ? Border.all(color: menuTheme.textPrimary)
                                    : null,
                              ),
                              foregroundDecoration: BoxDecoration(
                                shape: BoxShape.circle,
                                border: Border.all(
                                    color: menuTheme.textPrimary
                                        .withValues(alpha: 0.7 * lit)),
                              ),
                            ),
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            const MenuDivider(),
            row(_ProjectMenuAction.delete, l10n.delete,
                key: 'delete', chord: 'edit.delete.selection'),
          ],
        ),
      );
    },
  );
  if (action == null) return;

  if (!context.mounted) return;
  // Everything the commands below act on, with a refusal per item so
  // that one that has gone, or cannot do it, leaves the rest standing.
  final acts = targets ?? [item];
  switch (action) {
    case _ProjectMenuAction.open:
      if (item case ItemReference_Composition(:final field0)) {
        ui.setSelectedComp(field0);
      }
    case _ProjectMenuAction.export:
      if (item case ItemReference_Composition(:final field0)) {
        await showExportDialogFrb(context: context, comp: field0);
      }
    case _ProjectMenuAction.addToComp:
      addItemsToOpenComp(context, acts);
      onLocalEdit();
    case _ProjectMenuAction.revealInComp:
      revealInComposition(ui, item);
    case _ProjectMenuAction.newCompFromSelection:
      onNewComposition?.call([
        for (final target in acts)
          if (target case ItemReference_Footage(:final field0)) field0,
      ]);
    case _ProjectMenuAction.openInSource:
      if (item case ItemReference_Footage(:final field0)) {
        ui.openFootageView(field0);
      }
    case _ProjectMenuAction.toggleFolder:
      onToggleFolder?.call();
    case _ProjectMenuAction.newFolder:
      onNewFolder?.call();
    case _ProjectMenuAction.copy:
      onCopy?.call();
    case _ProjectMenuAction.paste:
      onPaste?.call();
    case _ProjectMenuAction.compSettings:
      if (item case ItemReference_Composition(:final field0)) {
        // Reachable now that the dialog takes a CompositionReference rather than
        // an AppStateStub; the port had to drop this entry until it did.
        if (await showCompSettingsFrb(context: context, comp: field0)) {
          onLocalEdit();
        }
      }
    case _ProjectMenuAction.duplicate:
      final copies = await duplicateProjectItems(context, acts);
      if (copies.isNotEmpty) {
        onLocalEdit();
        onPick?.call(copies);
      }
    case _ProjectMenuAction.interpret:
      final picked = [
        for (final target in targets ?? [item])
          if (target case ItemReference_Footage(:final field0)) field0,
      ];
      final applied = await showInterpretFootageFrb(
        context: context,
        items: picked,
        subject: picked.length == 1
            ? item.name()
            : l10n.interpretSeveral(picked.length),
        colour: ui.colourSummary,
      );
      if (applied) onLocalEdit();
    case _ProjectMenuAction.rememberInterpretation:
      if (item case ItemReference_Footage(:final field0)) {
        await rememberInterpretation(field0);
      }
    case _ProjectMenuAction.applyInterpretation:
      applyRememberedInterpretation([
        for (final target in targets ?? [item])
          if (target case ItemReference_Footage(:final field0)) field0,
      ]);
      onLocalEdit();
    case _ProjectMenuAction.rename:
      onStartRename?.call();
    case _ProjectMenuAction.relink:
      await onRelink?.call();
    case _ProjectMenuAction.reload:
      eachInOneUndo(Provider.of<LumitState>(context, listen: false), acts,
          (target) {
        if (target case ItemReference_Footage(:final field0)) field0.reload();
      });
      onLocalEdit();
    case _ProjectMenuAction.reveal:
      if (item case ItemReference_Footage(:final field0)) field0.reveal();
    case _ProjectMenuAction.editInBlender:
      if (item case ItemReference_Footage(:final field0)) {
        unawaited(field0.editInBlender());
      }
    case _ProjectMenuAction.findMissing:
      onFindMissing?.call();
    case _ProjectMenuAction.addAudioOnly:
      eachInOneUndo(Provider.of<LumitState>(context, listen: false), acts,
          (target) {
        if (target case ItemReference_Footage(:final field0)) {
          openComp?.addAudioLayer(footage: field0, arrival: ui.streamsArrival);
        }
      });
      onLocalEdit();
    case _ProjectMenuAction.setProxy:
      if (item case ItemReference_Footage(:final field0)) {
        final path = proxyPicker != null
            ? await proxyPicker()
            : await pickFootage()
                .then((paths) => paths.isEmpty ? null : paths.first);
        if (path == null) return;
        field0.setProxy(path: path);
        onLocalEdit();
      }
    case _ProjectMenuAction.shareStandIn || _ProjectMenuAction.shareOriginal:
      if (item case ItemReference_Footage(:final field0)) {
        final original = action == _ProjectMenuAction.shareOriginal;
        try {
          online
              ? footageOnline.fetch(field0, original: original)
              : field0.shareFetch(original: original);
        } catch (_) {
          // The project closed between the menu and the click.
        }
      }
    case _ProjectMenuAction.makeProxy:
      // Every picked footage item, and everything inside a picked folder. The
      // engine runs one transcode at a time and queues the rest, so each
      // starts as the one before it lands.
      final wanted = <FootageReference>[];
      void gather(ItemReference from) {
        switch (from) {
          case ItemReference_Footage(:final field0):
            wanted.add(field0);
          case ItemReference_Folder(:final field0):
            try {
              field0.getChildren().forEach(gather);
            } catch (_) {
              // The folder went away under the menu.
            }
          default:
        }
      }
      acts.forEach(gather);
      var started = false;
      Object? refused;
      for (final footage in wanted) {
        // The engine's own refusal, nothing to read from on this machine,
        // reaches the status line as its notice rather than as an exception
        // out of a menu handler. One missing file does not stop the rest.
        try {
          footage.makeProxy();
          started = true;
        } catch (e) {
          refused = e;
        }
      }
      if (refused != null && !started && context.mounted) {
        Provider.of<LumitState>(context, listen: false)
            .postNotice(l10n.proxyFailed('$refused'), error: true);
      }
      // The transcode reports on the status line, where every other piece of
      // background work does; this is the start signal that gets the strip
      // polling. Each finished file attaches itself on the poll that sees it
      // land, and the item scope of that op is what brings this panel back.
      if (started) proxyJobChanged.value++;
    case _ProjectMenuAction.useProxy:
      // The clicked row's new state, for every picked footage item that has a
      // proxy to read from; one that has none is passed over rather than
      // switched on to nothing.
      final on = !(proxy?.enabled ?? false);
      eachInOneUndo(Provider.of<LumitState>(context, listen: false), acts,
          (target) {
        if (target case ItemReference_Footage(:final field0)) {
          if (field0.getProxy() != null) field0.setUseProxy(on_: on);
        }
      });
      onLocalEdit();
    case _ProjectMenuAction.clearProxy:
      eachInOneUndo(Provider.of<LumitState>(context, listen: false), acts,
          (target) {
        if (target case ItemReference_Footage(:final field0)) {
          if (field0.getProxy() != null) field0.clearProxy();
        }
      });
      onLocalEdit();
    case _ProjectMenuAction.moveToRoot:
      eachInOneUndo(Provider.of<LumitState>(context, listen: false), acts,
          (target) => target.moveToRoot());
      onLocalEdit();
    case _ProjectMenuAction.delete:
      if (await deleteProjectItems(context, acts)) onLocalEdit();
  }
}

/// The menu on the panel's own ground: what to make, what to paste, and how the
/// list is ordered and opened.
void showProjectGroundMenu({
  required BuildContext context,
  required Offset position,
  required VoidCallback? onNewComposition,
  required VoidCallback? onNewNodeGraph,
  required VoidCallback? onNewFolder,
  required VoidCallback? onImport,
  required VoidCallback? onImportFolder,
  required VoidCallback? onPaste,
  required VoidCallback? onSelectAll,
  required ProjectColumn? sort,
  required ValueChanged<ProjectColumn?> onSort,
  required VoidCallback? onExpandAll,
  required VoidCallback? onCollapseAll,
  required VoidCallback? onFindMissing,
  required bool useProxies,
  required VoidCallback? onToggleProxies,
  required bool thumbnails,
  required VoidCallback onToggleThumbnails,
}) {
  showLumitPopup<void>(
    context: context,
    position: position,
    builder: (close) {
      Widget row(String key, String text, VoidCallback? run,
              {String? chord, bool? ticked}) =>
          MenuRow(
            key: ValueKey<String>('project-ground-$key'),
            enabled: run != null,
            onPressed: () {
              close(null);
              run?.call();
            },
            child: MenuLabel(text,
                action: chord, enabled: run != null, ticked: ticked),
          );
      return FloatSurface(
        width: _menuWidth,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            row('new-comp', l10n.newComposition, onNewComposition,
                chord: 'comp.new'),
            row('new-node-graph', l10n.newNodeGraph, onNewNodeGraph),
            row('new-folder', l10n.newFolder, onNewFolder),
            const MenuDivider(),
            row('import', l10n.menuImport, onImport, chord: 'file.import'),
            row('import-folder', l10n.menuImportFolder, onImportFolder),
            const MenuDivider(),
            row('paste', l10n.menuPaste, onPaste, chord: 'edit.paste'),
            row('select-all', l10n.menuSelectAll, onSelectAll,
                chord: 'edit.select.all'),
            const MenuDivider(),
            SubmenuRow(
              key: const ValueKey('project-ground-sort'),
              closeParent: () => close(null),
              submenu: (dismiss) => _flyout(_sortRows(sort, (c) {
                dismiss();
                onSort(c);
              })),
              child: Text(l10n.projectSortBy),
            ),
            if (onExpandAll != null)
              row('expand-all', l10n.expandAll, onExpandAll),
            if (onCollapseAll != null)
              row('collapse-all', l10n.collapseAll, onCollapseAll),
            if (onFindMissing != null) ...[
              const MenuDivider(),
              row('find-missing', l10n.findMissingFootage, onFindMissing),
            ],
            const MenuDivider(),
            row('use-proxies', l10n.projectUseProxies, onToggleProxies,
                ticked: useProxies),
            row('thumbnails', l10n.settingsThumbnailsOnFootageRows,
                onToggleThumbnails,
                ticked: thumbnails),
          ],
        ),
      );
    },
  );
}

/// One row per thing the list can be sorted by, the project's own order first.
List<Widget> _sortRows(
        ProjectColumn? sort, ValueChanged<ProjectColumn?> onSort) =>
    [
      MenuRow(
        key: const ValueKey('project-sort-none'),
        onPressed: () => onSort(null),
        child: MenuLabel(l10n.projectSortOrder, ticked: sort == null),
      ),
      const MenuDivider(),
      for (final column in ProjectColumn.values)
        MenuRow(
          key: ValueKey<String>('project-sort-by-${column.name}'),
          onPressed: () => onSort(column),
          child: MenuLabel(projectColumnName(column), ticked: sort == column),
        ),
    ];

/// The word a column goes by in a menu.
String projectColumnName(ProjectColumn column) => switch (column) {
      ProjectColumn.name => l10n.name,
      ProjectColumn.items => l10n.projectColumnItems,
      ProjectColumn.size => l10n.projectColumnSize,
      ProjectColumn.fps => l10n.unitFps,
      ProjectColumn.duration => l10n.duration,
      ProjectColumn.date => l10n.projectColumnDate,
      ProjectColumn.path => l10n.projectColumnPath,
    };

/// The menu on the column headings: which columns show, how the list is
/// sorted, and back to how it all started. A tick is the choice; a column
/// with no room at this width comes back once the panel is wide enough.
void showProjectColumnMenu({
  required BuildContext context,
  required Offset position,
  required Set<ProjectColumn> hidden,
  required ValueChanged<ProjectColumn> onToggle,
  required ProjectColumn? sort,
  required ValueChanged<ProjectColumn?> onSort,
  required VoidCallback onReset,
}) {
  showLumitPopup<void>(
    context: context,
    position: position,
    builder: (close) => FloatSurface(
      width: _menuWidth,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final column in ProjectColumn.values)
            if (column != ProjectColumn.name)
              MenuRow(
                key: ValueKey<String>('project-column-${column.name}'),
                onPressed: () {
                  close(null);
                  onToggle(column);
                },
                child: MenuLabel(projectColumnName(column),
                    ticked: !hidden.contains(column)),
              ),
          const MenuDivider(),
          SubmenuRow(
            key: const ValueKey('project-column-sort'),
            closeParent: () => close(null),
            submenu: (dismiss) => _flyout(_sortRows(sort, (c) {
              dismiss();
              onSort(c);
            })),
            child: Padding(
              padding: const EdgeInsetsDirectional.only(start: 16),
              child: Text(l10n.projectSortBy),
            ),
          ),
          MenuRow(
            key: const ValueKey('project-column-reset'),
            onPressed: () {
              close(null);
              onReset();
            },
            child: MenuLabel(l10n.projectResetColumns, ticked: false),
          ),
        ],
      ),
    ),
  );
}

/// The rate field on the Project panel's menu: the speed of one run of stills,
/// typed rather than picked (docs/07 §3.1).
///
/// A widget of its own because it owns a controller, and because the menu is a
/// function. It writes on Enter alone — a rate half typed is not a rate — and
/// what it writes is the exact pair [parseRate] reads, so 23.976 goes into the
/// document as 24000/1001.
class _SequenceRateField extends StatefulWidget {
  const _SequenceRateField({
    required this.theme,
    required this.rate,
    required this.onRate,
  });

  final LumitTheme theme;
  final BridgeSequenceRate rate;
  final void Function(int fpsNum, int fpsDen) onRate;

  @override
  State<_SequenceRateField> createState() => _SequenceRateFieldState();
}

class _SequenceRateFieldState extends State<_SequenceRateField> {
  late final TextEditingController _text = TextEditingController(
      text: formatRate(widget.rate.fpsNum, widget.rate.fpsDen));

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SizedBox(
        width: 56,
        child: LumitTooltip(
          message: l10n.tipSequenceRate,
          child: HouseTextField(
            key: const ValueKey('project-menu-sequence-rate-field'),
            controller: _text,
            width: 56,
            fill: widget.theme.surface0,
            // The drawings right-align every numeric well, and a rate is one.
            textAlign: TextAlign.right,
            onSubmitted: (_) {
              final rate = parseRate(_text.text);
              // Nonsense leaves the run at the rate it had, and says so by
              // putting that rate back in the field.
              if (rate == null) {
                _text.text = formatRate(widget.rate.fpsNum, widget.rate.fpsDen);
                return;
              }
              widget.onRate(rate.$1, rate.$2);
            },
          ),
        ),
      );
}

/// [footage]'s file, or empty when the project has gone.
String _filePathOf(FootageReference footage) {
  try {
    return footage.filePath();
  } catch (_) {
    return '';
  }
}

/// The compositions that show [item], each with its name, in the order its
/// uses come. Read as a menu opens, never in a build.
List<(CompositionReference, String)> compsUsing(ItemReference item) {
  final out = <(CompositionReference, String)>[];
  try {
    for (final use in item.usedBy()) {
      final comp = CompositionReference(
          internalproject: use.internalprojectId,
          internalid: use.internalcompId);
      if (out.any((c) => c.$1 == comp)) continue;
      out.add((comp, comp.getSettings().name));
    }
  } catch (_) {
    // The project closed as the menu opened.
  }
  return out;
}

/// Open [comp], or the first composition that shows [item], with the layers
/// there that show it selected, and front the Timeline. False when nothing
/// does.
bool revealInComposition(LumitUiState ui, ItemReference item,
    {CompositionReference? comp}) {
  final List<LayerReference> uses;
  try {
    uses = item.usedBy();
  } catch (_) {
    return false;
  }
  final first = uses
      .where((l) => comp == null || l.internalcompId == comp.internalid)
      .firstOrNull;
  if (first == null) return false;
  ui.setSelectedComp(CompositionReference(
      internalproject: first.internalprojectId,
      internalid: first.internalcompId));
  ui.model.refresh();
  ui.setSelection([
    for (final layer in uses)
      if (layer.internalcompId == first.internalcompId) layer,
  ]);
  setPanelVisible(ui.split, Panel.timeline, true);
  activatePanelTab(ui.split, Panel.timeline);
  ui.workspace.touch();
  return true;
}
