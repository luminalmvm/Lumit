// Replace source: the selected layers take another item of the project, and keep
// everything else they have.

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show MenuEntry, PickListEntry;
import '../shell/menu_layer_frb.dart' show selectedEntries;
import '../state/file_dialogs.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;

/// Whether a layer has a source to replace. Asks the engine, so it's for a menu
/// opening or a drop, not for a rebuild.
bool canReplaceSource(BridgeLayerEntry entry) {
  if (entry.info.switches.locked) return false;
  try {
    return entry.layer.canReplaceSource();
  } catch (_) {
    return false;
  }
}

/// The row, its list read as it opens: the menu bar rebuilds on every edit, and
/// walking the project is no part of that.
class ReplaceSourceEntry extends PickListEntry {
  final List<MenuEntry> Function() _rows;

  ReplaceSourceEntry(List<MenuEntry> Function() rows)
      : _rows = rows,
        super(l10n.menuReplaceSource, const []);

  @override
  List<MenuEntry> get children => _rows();
}

/// Point [targets] at [item], as one undo step, and say what didn't come along.
void replaceLayerSources(LumitState app, List<BridgeLayerEntry> targets,
    ItemReference item) {
  var refused = false;
  var unretimed = false;
  asOneUndoStep(app.project, () {
    for (final entry in targets) {
      try {
        // A solid has no time to remap, so a swap to one takes the Retime.
        if (entry.layer.setSourceItem(item: item) &&
            item is ItemReference_Solid &&
            entry.info.retime != null) {
          unretimed = true;
        }
      } catch (_) {
        refused = true;
      }
    }
  });
  if (refused) app.postNotice(l10n.replaceSourceRefused);
  if (unretimed) app.postNotice(l10n.replaceSourceRetimeRemoved);
  app.notifyDocumentChanged();
}

/// Every footage item, solid and composition under [roots].
Iterable<ItemReference> _sources(List<ItemReference> roots) sync* {
  for (final item in roots) {
    if (item case ItemReference_Folder(:final field0)) {
      yield* _sources(field0.getChildren());
    } else {
      yield item;
    }
  }
}

/// The project's items under their folders for the selected layers, then
/// Choose… for a file.
List<MenuEntry> replaceSourceRows(LumitState app, LumitUiState ui) {
  final project = app.project;
  if (project == null) return const [];
  final targets = selectedEntries(ui).where(canReplaceSource).toList();
  final current = targets.firstOrNull?.layer.getSourceItem();
  // Only what every picked layer can take, by the engine's own test.
  bool takes(ItemReference item) => targets.every((e) {
        try {
          return e.layer.canTakeSource(item: item);
        } catch (_) {
          return false;
        }
      });

  List<MenuEntry> rows(List<ItemReference> items) {
    final out = <MenuEntry>[];
    for (final item in items) {
      if (item case ItemReference_Folder(:final field0)) {
        final inside = rows(field0.getChildren());
        if (inside.isNotEmpty) out.add(MenuEntry.submenu(item.name(), inside));
      } else if (takes(item)) {
        out.add(MenuEntry(
          item.name(),
          targets.isEmpty
              ? null
              : () => replaceLayerSources(app, targets, item),
          checked: current != null && current.equals(item: item),
        ));
      }
    }
    return out;
  }

  return [
    ...rows(project.getItems()),
    MenuEntry.divider(),
    MenuEntry(
      l10n.replaceSourceChoose,
      targets.isEmpty ? null : () => _choose(app, targets),
    ),
  ];
}

/// Bring a file in and point the layers at it, as one undo step with the
/// import.
Future<void> _choose(LumitState app, List<BridgeLayerEntry> targets) async {
  final project = app.project;
  if (project == null) return;
  final picked = await pickFootage();
  if (picked.isEmpty) return;
  final before = _sources(project.getItems()).toList();
  project.beginUndoGroup();
  try {
    if (!await app.importFootagePaths([picked.first])) return;
    final fresh = _sources(project.getItems())
        .where((i) =>
            i is ItemReference_Footage && !before.any((b) => b.equals(item: i)))
        .firstOrNull;
    if (fresh != null) replaceLayerSources(app, targets, fresh);
  } finally {
    project.endUndoGroup();
  }
  app.notifyDocumentChanged();
}
