// The extensions installed on this machine, as the interface knows them.
//
// The engine owns the folder and judges every manifest. This holds the list
// it gave, the gestures that change it, and the two things about an
// extension that are the interface's own: which dock pane it is shown in,
// and which folders the person has let it watch.

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/workspace.dart';

class ExtensionService extends ChangeNotifier {
  final Workspace _workspace;

  ExtensionService(this._workspace) {
    extensionPaneTitle = (slot) => inSlot(slot)?.name;
    refresh();
  }

  /// Every extension installed, by id.
  List<BridgeExtension> installed = const [];

  /// An install is running. One at a time.
  bool busy = false;

  /// Moves on whenever an extension is installed over itself, so its panel
  /// loads the new page.
  int generation = 0;

  /// Read the folder again.
  void refresh() {
    try {
      installed = extensionList();
    } catch (_) {
      // No engine to ask, as in a widget test.
      installed = const [];
    }
    notifyListeners();
  }

  /// The pane an extension is shown in: its `Panel.extension` instance
  /// number. Given out once and kept, so a saved arrangement still means
  /// the same extension after another is installed or removed.
  int slot(String id) {
    final slots = _workspace.extensionSlots;
    final kept = slots[id];
    if (kept != null) return kept;
    var free = 1;
    while (slots.containsValue(free)) {
      free++;
    }
    _workspace.setExtensionSlot(id, free);
    return free;
  }

  PaneId pane(String id) => (panel: Panel.extension, instance: slot(id));

  /// The extension shown in the pane numbered [slot], if it is installed.
  BridgeExtension? inSlot(int slot) {
    for (final extension in installed) {
      if (_workspace.extensionSlots[extension.id] == slot) return extension;
    }
    return null;
  }

  /// The folders the person has picked for [id] to watch.
  Set<String> folders(String id) => {...?_workspace.extensionFolders[id]};

  void setFolders(String id, Set<String> folders) =>
      _workspace.setExtensionFolders(id, folders.toList());

  /// What the extension in [folder] says about itself and asks to be
  /// allowed. Nothing is installed.
  BridgeExtensionOutcome inspect(String folder) {
    try {
      return extensionInspect(folder: folder);
    } catch (_) {
      return const BridgeExtensionOutcome.failed();
    }
  }

  Future<BridgeExtensionOutcome> install(String folder) async {
    if (busy) return const BridgeExtensionOutcome.busy();
    busy = true;
    notifyListeners();
    BridgeExtensionOutcome outcome;
    try {
      outcome = await extensionInstall(folder: folder);
    } catch (_) {
      outcome = const BridgeExtensionOutcome.failed();
    }
    busy = false;
    if (outcome is BridgeExtensionOutcome_Ready) generation++;
    refresh();
    return outcome;
  }

  /// Remove an extension. What its page stored, and the folders picked for
  /// it, stay for if it is installed again.
  bool remove(String id) {
    var removed = false;
    try {
      removed = extensionRemove(id: id);
    } catch (_) {
      // Nothing was removed.
    }
    refresh();
    return removed;
  }
}
