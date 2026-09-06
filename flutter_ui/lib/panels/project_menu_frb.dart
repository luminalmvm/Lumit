// The Project panel's context menu — everything a right-click on a row offers.
//
// **In plain terms**: the list that appears when you right-click something in
// the Project panel, and the code that carries out whichever line was picked.
// It is one function because the menu is one gesture: the rows it draws and
// the work it does after the click are two halves of the same answer.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart' show BridgeColourItem;
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/comp_settings_frb.dart';
import '../shell/status_line_frb.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'project_columns_frb.dart';

enum _ProjectMenuAction {
  compSettings,
  rename,
  relink,
  findMissing,
  addAudioOnly,
  setProxy,
  makeProxy,
  useProxy,
  clearProxy,
  moveToRoot,
  delete
}

/// A colour chip with a word for it, or the bare chip where the colour is its
/// own explanation — which is every chip but the neutral one (A12).
Widget _menuChipTip(String? message, Widget chip) =>
    message == null ? chip : LumitTooltip(message: message, child: chip);

/// The project context menu.
Future<void> showProjectMenuFrb({
  required BuildContext context,
  required ItemReference item,
  required bool missing,
  required Offset position,
  required VoidCallback onFindMissing,
  required VoidCallback onLocalEdit,
  Future<void> Function()? onRelink,

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
}) async {
  final isFootage = item is ItemReference_Footage;
  final isComp = item is ItemReference_Composition;
  // The comp the sound would land in. Read once, here, rather than in
  // the row's build: the menu is a gesture, not a rebuild path.
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final openComp = ui.selectedComp;
  // The colour config's own space names, off the summary the shell holds, and
  // what this item is set to now. Both read here — raising a menu is a
  // gesture, and `colour_space` is a document read.
  final colourSpaces =
      ui.colourSummary.loaded ? ui.colourSummary.spaces : const <String>[];
  final colourSpace = switch (item) {
    ItemReference_Footage(:final field0) => field0.colourSpace(),
    _ => null,
  };
  // Read here rather than inside the popup's builder: the popup is raised in
  // its own route, so it has no ThemeScope of the panel's above it.
  final menuTheme = ThemeScope.of(context).theme;
  final action = await showLumitPopup<_ProjectMenuAction>(
    context: context,
    position: position,
    builder: (close) => FloatSurface(
      width: 210,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (isComp)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.compSettings),
              child: Text(l10n.compositionSettingsEllipsis),
            ),
          // Every kind can be renamed from here. It matters most for a comp,
          // whose second click opens it rather than renaming it.
          MenuRow(
            key: const ValueKey('project-menu-rename'),
            onPressed: () => close(_ProjectMenuAction.rename),
            child: Text(l10n.rename),
          ),
          // Relink is offered only on a row that is actually broken.
          if (isFootage && missing)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.relink),
              child: Text(l10n.relinkEllipsis),
            ),
          if (isFootage)
            MenuRow(
              onPressed: () => close(_ProjectMenuAction.findMissing),
              child: Text(l10n.findMissingFootage),
            ),
          // The sound of this clip, on its own row. Offered only with a
          // comp open to put it in — placing a layer nowhere is not an action.
          if (isFootage && openComp != null)
            MenuRow(
              key: const ValueKey('project-menu-add-audio-only'),
              onPressed: () => close(_ProjectMenuAction.addAudioOnly),
              child: Text(l10n.addAudioOnly),
            ),
          // **Proxies, on the item's own menu** (docs/07 §3.3). Four
          // commands and no dialogue: attach a file, make one, read from it or
          // not, forget it. Offered on footage alone — a comp and a folder
          // have no media reference for a stand-in to stand in for — and the
          // last two only once there is a proxy, so the menu never lists a
          // word that would do nothing.
          if (isFootage) ...[
            MenuRow(
              key: const ValueKey('project-menu-set-proxy'),
              onPressed: () => close(_ProjectMenuAction.setProxy),
              child: Text(l10n.setProxyEllipsis),
            ),
            MenuRow(
              key: const ValueKey('project-menu-make-proxy'),
              onPressed: () => close(_ProjectMenuAction.makeProxy),
              child: Text(l10n.makeProxy),
            ),
            if (proxy != null) ...[
              // Ticked, in the shape the layer menu's Accepts lights uses: a
              // word says what the tick means, where a glyph could not.
              MenuRow(
                key: const ValueKey('project-menu-use-proxy'),
                onPressed: () => close(_ProjectMenuAction.useProxy),
                child: Row(
                  children: [
                    menuTick(proxy.enabled),
                    Expanded(child: Text(l10n.useProxy)),
                  ],
                ),
              ),
              MenuRow(
                key: const ValueKey('project-menu-clear-proxy'),
                onPressed: () => close(_ProjectMenuAction.clearProxy),
                child: Text(l10n.clearProxy),
              ),
            ],
          ],
          // **What colour space this footage arrived in** (docs/impl/ocio.md
          // §6.5). A submenu rather than a row, because the answer is a name
          // out of the project's colour config and there may be forty of them.
          // This is the smallest honest surface until *Interpret footage…*
          // exists as drawn (docs/07 §3.2), and it is replaced when that
          // dialogue lands.
          //
          // The names are the config's own and cross verbatim. A name
          // assigned while a config that has since gone was loaded is still
          // listed, ticked, because it is the user's statement about the file
          // and the menu must not pretend it was never made.
          if (item case ItemReference_Footage(:final field0))
            SubmenuRow(
              key: const ValueKey('project-menu-colour-space'),
              closeParent: () => close(null),
              submenu: (dismiss) => FloatSurface(
                width: 210,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    // The built-in interpretation: what a file says about
                    // itself, which is where every footage item starts.
                    MenuRow(
                      key: const ValueKey('project-menu-colour-space-none'),
                      onPressed: () {
                        dismiss();
                        field0.setColourSpace();
                        onLocalEdit();
                      },
                      child: Row(children: [
                        menuTick(colourSpace == null),
                        Expanded(child: Text(l10n.colourSpaceFromFile)),
                      ]),
                    ),
                    if (colourSpaces.isNotEmpty ||
                        (colourSpace != null &&
                            !colourSpaces.contains(colourSpace)))
                      Padding(
                        padding: const EdgeInsets.fromLTRB(10, 6, 10, 2),
                        child: Text(l10n.colourSpaceFromConfig,
                            style: menuTheme.small
                                .copyWith(color: menuTheme.textMuted)),
                      ),
                    for (final space in [
                      if (colourSpace != null &&
                          !colourSpaces.contains(colourSpace))
                        colourSpace,
                      ...colourSpaces,
                    ])
                      // A space the config cannot read footage from stays
                      // listed, drawn quiet, with its reason on hover.
                      if (colourItemProblem(
                              ui.colourSummary, BridgeColourItem.input, space)
                          case final why?)
                        LumitTooltip(
                          message: why,
                          child: MenuRow(
                            key: ValueKey<String>(
                                'project-menu-colour-space-$space'),
                            onPressed: () {},
                            child: Row(children: [
                              menuTick(space == colourSpace),
                              Expanded(
                                  child: Text(space,
                                      style: TextStyle(
                                          color: menuTheme.textDisabled))),
                            ]),
                          ),
                        )
                      else
                        MenuRow(
                          key: ValueKey<String>(
                              'project-menu-colour-space-$space'),
                          onPressed: () {
                            dismiss();
                            field0.setColourSpace(space: space);
                            onLocalEdit();
                          },
                          child: Row(children: [
                            menuTick(space == colourSpace),
                            Expanded(child: Text(space)),
                          ]),
                        ),
                  ],
                ),
              ),
              child: Text(l10n.colourSpace),
            ),
          // The colour tag, as the strip itself rather than a submenu: the
          // chips ARE the choice, so putting them on the menu row costs one
          // click where a submenu costs two and a hover in between. The same
          // shape the Timeline's layer swatch offers, and the same palette.
          if (onSetLabel != null)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(l10n.label, style: menuTheme.small),
                  const SizedBox(width: 6),
                  for (var i = 0; i < LumitTheme.labelCount; i++)
                    _menuChipTip(
                      // The neutral chip is the way back, and what it goes back
                      // *to* depends on where the item sits (A12): inside a
                      // tagged folder it hands the row back to the folder's
                      // colour, outside one it leaves the row with none. Same
                      // chip, two words — the colours themselves say what they
                      // are, so only this one is named.
                      i == 0
                          ? (inherited != 0
                              ? l10n.tipLabelInherit
                              : l10n.tipLabelNoColour)
                          : null,
                      GestureDetector(
                        key: ValueKey<String>('project-menu-label-$i'),
                        onTap: () {
                          onSetLabel(i);
                          close(null);
                        },
                        child: Container(
                          width: 10,
                          height: 10,
                          margin: const EdgeInsets.only(right: 2),
                          decoration: BoxDecoration(
                            color: menuTheme.labelColour(i),
                            shape: BoxShape.circle,
                            border: i == label
                                ? Border.all(color: menuTheme.textPrimary)
                                : null,
                          ),
                        ),
                      ),
                    ),
                ],
              ),
            ),
          // Filing, the mouse way — the drag onto a folder row is the other
          // one. A submenu rather than a dialogue: the folders are a short
          // list the panel already knows, so picking one is one hover and one
          // click, the same shape Effects & presets offers its categories in.
          if (onMoveToFolder != null && folders.isNotEmpty)
            SubmenuRow(
              key: const ValueKey('project-menu-move-to-folder'),
              closeParent: () => close(null),
              submenu: (dismiss) => FloatSurface(
                width: 210,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (final (name, folder) in folders)
                      MenuRow(
                        key: ValueKey<String>(
                            'project-menu-folder-${projectItemId(folder)}'),
                        onPressed: () {
                          dismiss();
                          onMoveToFolder(folder);
                        },
                        child: Text(name),
                      ),
                  ],
                ),
              ),
              child: Text(l10n.moveToFolder),
            ),
          MenuRow(
            onPressed: () => close(_ProjectMenuAction.moveToRoot),
            child: Text(l10n.moveToRoot),
          ),
          MenuRow(
            onPressed: () => close(_ProjectMenuAction.delete),
            child: Text(l10n.delete),
          ),
        ],
      ),
    ),
  );
  if (action == null) return;

  if (!context.mounted) return;
  // Everything the commands below act on, with a refusal per item so
  // that one that has gone, or cannot do it, leaves the rest standing.
  final acts = targets ?? [item];
  switch (action) {
    case _ProjectMenuAction.compSettings:
      if (item case ItemReference_Composition(:final field0)) {
        // Reachable now that the dialog takes a CompositionReference rather than
        // an AppStateStub; the port had to drop this entry until it did.
        if (await showCompSettingsFrb(context: context, comp: field0)) {
          onLocalEdit();
        }
      }
    case _ProjectMenuAction.rename:
      onStartRename?.call();
    case _ProjectMenuAction.relink:
      await onRelink?.call();
    case _ProjectMenuAction.findMissing:
      onFindMissing();
    case _ProjectMenuAction.addAudioOnly:
      if (item case ItemReference_Footage(:final field0)) {
        openComp?.addAudioLayer(footage: field0);
        onLocalEdit();
      }
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
    case _ProjectMenuAction.makeProxy:
      // **The one command here that stays singular**: the engine runs
      // one transcode at a time by design, so starting four would be three
      // refusals and a notice apiece. The clicked row's, and only that.
      if (item case ItemReference_Footage(:final field0)) {
        // The engine's own refusals — one transcode at a time, and nothing to
        // read from on this machine — reach the status line as its notice,
        // rather than as an exception out of a menu handler.
        try {
          field0.makeProxy();
        } catch (e) {
          if (context.mounted) {
            Provider.of<LumitState>(context, listen: false)
                .postNotice(l10n.proxyFailed('$e'), error: true);
          }
          return;
        }
        // The transcode reports on the status line, where every other piece of
        // background work does; this is the start signal that gets the strip
        // polling. The finished file attaches itself on the poll that sees it
        // land, and the item scope of that op is what brings this panel back.
        proxyJobChanged.value++;
      }
    case _ProjectMenuAction.useProxy:
      // The clicked row's new state, for every picked footage item that has a
      // proxy to read from; one that has none is passed over rather than
      // switched on to nothing.
      final on = !(proxy?.enabled ?? false);
      for (final target in acts) {
        if (target case ItemReference_Footage(:final field0)) {
          if (field0.getProxy() == null) continue;
          try {
            field0.setUseProxy(on_: on);
          } catch (_) {}
        }
      }
      onLocalEdit();
    case _ProjectMenuAction.clearProxy:
      for (final target in acts) {
        if (target case ItemReference_Footage(:final field0)) {
          if (field0.getProxy() == null) continue;
          try {
            field0.clearProxy();
          } catch (_) {}
        }
      }
      onLocalEdit();
    case _ProjectMenuAction.moveToRoot:
      for (final target in acts) {
        try {
          target.moveToRoot();
        } catch (_) {}
      }
      onLocalEdit();
    case _ProjectMenuAction.delete:
      // No confirmation: it is one undo step, matching egui.
      for (final target in acts) {
        try {
          target.delete();
        } catch (_) {
          // Already gone - a folder deleted with its parent a moment ago.
        }
      }
      onLocalEdit();
  }
}
