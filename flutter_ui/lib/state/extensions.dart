// The extensions installed on this machine, as the interface knows them.
//
// The engine owns the folder and judges every manifest. This holds the list
// it gave, the gestures that change it, and the two things about an
// extension that are the interface's own: which dock pane it is shown in,
// and which folders the person has let it watch.

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show effectSchema;
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

  /// The ones with a panel of their own. One that carries only effects has
  /// no pane.
  Iterable<BridgeExtension> get panels => installed.where((e) => e.panel);

  /// The folders extensions are found in.
  BridgeExtensionFolders scanFolders =
      const BridgeExtensionFolders(machine: [], extra: []);

  /// An install is running. One at a time.
  bool busy = false;

  /// Moves on whenever an extension is installed over itself, so its panel
  /// loads the new page.
  int generation = 0;

  int _reads = 0;

  /// Read the folder again. A read overtaken by a later one is dropped.
  Future<void> refresh() async {
    final read = ++_reads;
    List<BridgeExtension> list;
    BridgeExtensionFolders? folders;
    try {
      list = await extensionList();
      folders = await extensionFolders();
    } catch (_) {
      // No engine to ask, as in a widget test.
      list = const [];
    }
    if (read != _reads) return;
    installed = list;
    if (folders != null) scanFolders = folders;
    notifyListeners();
  }

  /// The engine has registered or dropped effects, so the effect lists are
  /// read again.
  Future<void> _effectsChanged() {
    effectSchema.forget();
    return refresh();
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
    for (final extension in panels) {
      if (_workspace.extensionSlots[extension.id] == slot) return extension;
    }
    return null;
  }

  /// The installed extension that carries the LFX effect [matchName], and
  /// that effect, or null when none does.
  (BridgeExtension, BridgeExtensionEffect)? carrying(String matchName) {
    for (final extension in installed) {
      for (final effect in extension.effects) {
        if (effect.matchName == matchName) return (extension, effect);
      }
    }
    return null;
  }

  /// The folders the person has picked for [id] to watch.
  Set<String> folders(String id) => {...?_workspace.extensionFolders[id]};

  void setFolders(String id, Set<String> folders) =>
      _workspace.setExtensionFolders(id, folders.toList());

  /// What the extension in [folder] says about itself and asks to be
  /// allowed. Nothing is installed.
  Future<BridgeExtensionOutcome> inspect(String folder) async {
    try {
      return await extensionInspect(folder: folder);
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
    await _effectsChanged();
    return outcome;
  }

  /// What the `.lfx` package at [path] holds and asks to be allowed.
  /// Nothing is unpacked.
  Future<BridgeExtensionOutcome> inspectPackage(String path) async {
    try {
      return await extensionInspectPackage(path: path);
    } catch (_) {
      return const BridgeExtensionOutcome.failed();
    }
  }

  /// Install the package at [path] into [folder], or Lumit's own folder
  /// when null.
  Future<BridgeExtensionOutcome> installPackage(String path,
      {String? folder}) async {
    if (busy) return const BridgeExtensionOutcome.busy();
    busy = true;
    notifyListeners();
    BridgeExtensionOutcome outcome;
    try {
      outcome = await extensionInstallPackage(path: path, folder: folder);
    } catch (_) {
      outcome = const BridgeExtensionOutcome.failed();
    }
    busy = false;
    if (outcome is BridgeExtensionOutcome_Ready) generation++;
    await _effectsChanged();
    return outcome;
  }

  /// Pack the extension in [folder] as [out], checked as an install would be.
  Future<BridgeExtensionOutcome> makePackage(String folder, String out) async {
    if (busy) return const BridgeExtensionOutcome.busy();
    busy = true;
    notifyListeners();
    BridgeExtensionOutcome outcome;
    try {
      outcome = await extensionMakePackage(folder: folder, out: out);
    } catch (_) {
      outcome = const BridgeExtensionOutcome.failed();
    }
    busy = false;
    await _effectsChanged();
    return outcome;
  }

  /// Register what the folders carry now and read them again, so an
  /// extension another Lumit installed is picked up.
  Future<void> rescan() async {
    try {
      await extensionRescan();
    } catch (_) {
      // No engine to ask, as in a widget test.
    }
    await _effectsChanged();
  }

  /// Scan [folder] for extensions too.
  Future<void> addFolder(String folder) async {
    try {
      await extensionAddFolder(folder: folder);
    } catch (_) {
      // Nothing was added.
    }
    await _effectsChanged();
  }

  /// Stop scanning a folder the person added. Nothing in it is deleted.
  Future<void> removeFolder(String folder) async {
    try {
      await extensionRemoveFolder(folder: folder);
    } catch (_) {
      // Nothing was taken off.
    }
    await _effectsChanged();
  }

  /// Remove an extension. What its page stored, and the folders picked for
  /// it, stay for if it is installed again.
  Future<BridgeExtensionRemoval> remove(String id) async {
    var removed = BridgeExtensionRemoval.failed;
    try {
      removed = await extensionRemove(id: id);
    } catch (_) {
      // Nothing was removed.
    }
    await _effectsChanged();
    return removed;
  }
}
