// The menu bar, on the flutter_rust_bridge API — the second panel port.
//
// File / Edit / Composition, with the item set taken from the egui menu
// (shell/app_update.rs) and the v0 menu_bar.dart. Every engine-backed item calls
// straight through a reference handle, and the file pickers are injectable seams
// so a widget test never opens a plugin channel.
//
// Undo and Redo grey out from `ProjectReference.history()` rather than being
// always-enabled: an item you can see is disabled tells you the state of the
// document, where one that does nothing when pressed does not.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import '../state/dock.dart';
import '../state/file_dialogs.dart';
import '../widgets/controls.dart';
import 'command_palette_frb.dart';
import 'comp_settings_frb.dart';
import 'export_dialog_frb.dart';
import 'recovery_dialog_frb.dart';
import 'settings_window_frb.dart';

class LumitMenuBarFrb extends StatelessWidget {
  final LumitState app;

  /// File-picker seams. Defaulted to the real dialogues; a test injects its own,
  /// because a plugin channel cannot open in a widget test.
  final Future<String?> Function()? openPicker;
  final Future<String?> Function()? savePicker;
  final Future<List<String>> Function()? footagePicker;

  const LumitMenuBarFrb({
    super.key,
    required this.app,
    this.openPicker,
    this.savePicker,
    this.footagePicker,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final project = app.project;
    // Null while no project is loaded, so every document item is disabled rather
    // than throwing when pressed.
    final history = project?.history();

    return Container(
      height: 26,
      color: t.surface2,
      child: Row(
        children: [
          const SizedBox(width: 4),
          _menu(context, 'File', [
            _Item('New project', app.newProject),
            _Item('Open project…', () => _open(context)),
            _Item.divider(),
            // Save is only meaningful once there is a project; without a path it
            // behaves as Save as, which is what the engine's empty-path refusal
            // makes us handle explicitly.
            _Item('Save', project == null ? null : () => _save(context)),
            _Item(
                'Save as…',
                project == null
                    ? null
                    : () => _save(context, forcePicker: true)),
            _Item.divider(),
            _Item('Import footage…',
                project == null ? null : () => _import(context)),
            _Item.divider(),
            _Item(
              'Export…',
              context.read<LumitUiState>().selectedComp == null
                  ? null
                  : () => _export(context),
            ),
            _Item.divider(),
            _Item('Recover…', project == null ? null : () => _recover(context)),
          ]),
          _menu(context, 'Edit', [
            _Item(
              'Undo',
              (history?.canUndo ?? false) ? () => _undo(context) : null,
            ),
            _Item(
              'Redo',
              (history?.canRedo ?? false) ? () => _redo(context) : null,
            ),
          ]),
          _menu(context, 'Composition', [
            _Item('New composition',
                project == null ? null : () => _newComposition(context)),
            _Item.divider(),
            // Layer creation. Every one of these needs a composition to go
            // into, so they are disabled together rather than each checking.
            _Item(
                'Add solid layer', _onComp(context, (c) => c.addSolidLayer())),
            _Item('Add text layer', _onComp(context, (c) => c.addTextLayer())),
            _Item('Add camera layer',
                _onComp(context, (c) => c.addCameraLayer())),
            _Item('Add adjustment layer',
                _onComp(context, (c) => c.addAdjustmentLayer())),
            _Item('Add null object layer',
                _onComp(context, (c) => c.addNullObjectLayer())),
            _Item('Add sequence layer',
                _onComp(context, (c) => c.addSequenceLayer())),
            _Item.divider(),
            // The selected layer's Retime (K-197). Reachable here as well as
            // on the keyboard because Alt+Shift+T is the Windows
            // input-language switch on a machine with two layouts, which eats
            // the chord before the application sees it.
            _Item(
              _retimeLabel(context),
              _onSelectedLayer(context, (l) => app.toggleRetime(l)),
            ),
            _Item.divider(),
            _Item('Cut clip at playhead',
                _onComp(context, (c) => _cutAtPlayhead(context, c))),
            _Item('Add marker at playhead',
                _onComp(context, (c) => _markerAtPlayhead(context, c))),
            _Item.divider(),
            // Beat detection reads the whole comp's audio and can take
            // seconds, so it runs off-thread; a comp with no audio does
            // nothing rather than alarming.
            _Item(
                'Detect beats',
                _onComp(
                    context,
                    (c) => c
                        .detectBeats(sensitivityPercent: 50)
                        .then((_) {}, onError: (_) {}))),
            _Item('Clear beat markers',
                _onComp(context, (c) => c.clearBeatMarkers())),
            _Item.divider(),
            _Item(
              'Composition settings…',
              context.read<LumitUiState>().selectedComp == null
                  ? null
                  : () => _compSettings(context),
            ),
          ]),
          _menu(context, 'Window', [
            _Item('Command palette…', () => _palette(context)),
            // The four shipped presets (docs/07 §1.6) behind their own
            // heading rather than four siblings of everything else: pick an
            // arrangement and panels move, nothing closes or reloads.
            _Item.submenu(
              'Workspaces',
              [
                for (final preset in WorkspacePreset.values)
                  _Item(preset.title, () {
                    Provider.of<LumitUiState>(context, listen: false)
                        .workspace
                        .applyWorkspacePreset(preset);
                  }),
                _Item.divider(),
                _Item('Reset workspace',
                    () => context.read<LumitUiState>().resetLayout()),
              ],
            ),
            _Item.divider(),
            _Item('Settings…', () => showSettingsWindowFrb(context)),
          ]),
        ],
      ),
    );
  }

  /// Wrap a composition action: null (so the item greys out) when no
  /// composition is fronted, else the action followed by a redraw.
  VoidCallback? _onComp(
      BuildContext context, void Function(CompositionReference) run) {
    final comp = context.read<LumitUiState>().selectedComp;
    if (comp == null) return null;
    return () {
      run(comp);
      app.notifyDocumentChanged();
    };
  }

  /// The same, for a command that acts on the selected *layer* — greyed out
  /// with nothing selected rather than offered and inert.
  VoidCallback? _onSelectedLayer(
      BuildContext context, void Function(LayerReference) run) {
    final layer = context.read<LumitUiState>().selectedLayer.value;
    if (layer == null) return null;
    return () => run(layer);
  }

  /// What the Retime item says: the command names what it will do, so a layer
  /// that already has one offers to take it away.
  String _retimeLabel(BuildContext context) {
    final layer = context.read<LumitUiState>().selectedLayer.value;
    if (layer == null) return 'Enable Retime';
    try {
      return layer.getRetimeProperty() == null
          ? 'Enable Retime'
          : 'Disable Retime';
    } catch (_) {
      return 'Enable Retime';
    }
  }

  /// Razor the selected layer at the playhead. Only Sequence layers hold
  /// clips, so on anything else the engine declines and nothing happens.
  void _cutAtPlayhead(BuildContext context, CompositionReference comp) {
    final ui = context.read<LumitUiState>();
    final layer = ui.selectedLayer.value;
    if (layer == null) return;
    try {
      layer.cutClipAt(frame: ui.playheadFrame.value);
    } catch (_) {}
  }

  void _markerAtPlayhead(BuildContext context, CompositionReference comp) {
    final frame = context.read<LumitUiState>().playheadFrame.value;
    comp.setMarkers(markers: [
      ...comp.getMarkers(),
      BridgeMarker(
        id: UuidValue.fromString(const Uuid().v4()),
        time: comp.timeOfFrame(frame: frame),
        label: '',
      ),
    ]);
  }

  Future<void> _open(BuildContext context) async {
    final path = await (openPicker ?? pickProjectToOpen)();
    if (path == null) return;
    app.openProject(path);
  }

  /// Save, asking for a location only when there is not one already — or always,
  /// for Save as.
  ///
  /// The engine refuses an empty path on a project that has never been saved, so
  /// the decision of whether to prompt is made here from `path()` rather than by
  /// trying and handling the failure.
  Future<void> _save(BuildContext context, {bool forcePicker = false}) async {
    final project = app.project;
    if (project == null) return;

    var target = '';
    if (forcePicker || project.path() == null) {
      final picked = await (savePicker ?? pickProjectSaveLocation)();
      if (picked == null) return;
      target = picked;
    }
    try {
      final written = await project.save(path: target);
      app.postNotice('Saved to $written');
    } catch (_) {
      // The work is still in the document and the journal; say so calmly and
      // let the user pick somewhere writable.
      app.postNotice('Could not save the project', error: true);
    }
    app.notifyDocumentChanged();
  }

  Future<void> _import(BuildContext context) async {
    await app.importFootagePaths(await (footagePicker ?? pickFootage)());
  }

  Future<void> _newComposition(BuildContext context) async {
    final comp = await app.newComposition(context);
    // Front it, which is what the egui menu does — a comp you just made is the
    // one you want to work on.
    if (comp != null && context.mounted) {
      context.read<LumitUiState>().setSelectedComp(comp);
    }
  }

  Future<void> _compSettings(BuildContext context) async {
    final comp = context.read<LumitUiState>().selectedComp;
    if (comp == null) return;
    final applied = await showCompSettingsFrb(context: context, comp: comp);
    if (applied) app.notifyDocumentChanged();
  }

  Future<void> _export(BuildContext context) async {
    final comp = context.read<LumitUiState>().selectedComp;
    if (comp == null) return;
    await showExportDialogFrb(context: context, comp: comp);
  }

  /// Offer to recover work beside the open project.
  ///
  /// Only meaningful once the project has a path — recovery is about a *file*,
  /// and a project that has never been saved has nothing beside it.
  Future<void> _recover(BuildContext context) async {
    final path = app.project?.path();
    if (path == null) return;
    await showRecoveryDialogFrb(
      context: context,
      state: app,
      projectPath: path,
    );
  }

  /// The palette's commands are declared here, where the menu items are, so the
  /// two cannot drift apart into different ideas of what "New composition" does.
  /// Only shortcuts the key handler genuinely serves are taught — a palette
  /// that teaches a binding that does nothing is worse than one that is shy.
  /// Beyond commands it carries the other three categories docs/07 §12 asks
  /// for: every effect (applies to the selected layer), every comp (fronts
  /// it), and every panel (focuses it) — each under its own badge.
  Future<void> _palette(BuildContext context) async {
    final project = app.project;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    await showCommandPaletteFrb(
      context: context,
      commands: [
        PaletteCommand(
          label: 'New project',
          category: 'File',
          run: app.newProject,
        ),
        if (project != null) ...[
          PaletteCommand(
            label: 'Save',
            category: 'File',
            run: () => _save(context),
          ),
          PaletteCommand(
            label: 'Save as…',
            category: 'File',
            run: () => _save(context, forcePicker: true),
          ),
          PaletteCommand(
            label: 'Import footage…',
            category: 'File',
            run: () => _import(context),
          ),
          PaletteCommand(
            label: 'New composition',
            category: 'Composition',
            run: () => _newComposition(context),
          ),
          PaletteCommand(
            label: 'Undo',
            category: 'Edit',
            shortcut: 'Ctrl+Z',
            run: () => _undo(context),
          ),
          PaletteCommand(
            label: 'Redo',
            category: 'Edit',
            shortcut: 'Ctrl+Shift+Z',
            run: () => _redo(context),
          ),
          PaletteCommand(
            label: 'Export…',
            category: 'File',
            run: () => _export(context),
          ),
          // Every comp, by name: Enter fronts it in the Viewer and Timeline.
          for (final (comp, name) in app.comps())
            PaletteCommand(
              label: name,
              category: 'Comp',
              run: () => ui.setSelectedComp(comp),
            ),
          // Every built-in effect: Enter applies it to the selected layer;
          // with none selected it does nothing, exactly like the browser.
          for (final effect in listEffects())
            PaletteCommand(
              label: effect.label,
              category: 'Effect',
              run: () => ui.selectedLayer.value?.addEffect(name: effect.name),
            ),
        ],
        // Every panel: Enter focuses it in the dock.
        for (final panel in Panel.values)
          PaletteCommand(
            label: panel.title,
            category: 'Panel',
            run: () => ui.activePanel.value = panel,
          ),
        PaletteCommand(
          label: 'Settings…',
          category: 'File',
          run: () => showSettingsWindowFrb(context),
        ),
      ],
    );
  }

  void _undo(BuildContext context) {
    app.project?.undo();
    app.notifyDocumentChanged();
  }

  void _redo(BuildContext context) {
    app.project?.redo();
    app.notifyDocumentChanged();
  }

  Widget _menu(BuildContext context, String title, List<_Item> items) =>
      _MenuButton(title: title, items: items);
}

/// One menu row: a label and an action, or a divider. A null action renders the
/// row disabled rather than hiding it, so the menu's shape does not shift.
class _Item {
  final String? label;
  final VoidCallback? onPressed;
  final bool isDivider;

  /// The rows this one opens onto, for a heading like Window → Workspaces.
  final List<_Item>? children;

  _Item(this.label, this.onPressed)
      : isDivider = false,
        children = null;
  _Item.divider()
      : label = null,
        onPressed = null,
        isDivider = true,
        children = null;
  _Item.submenu(this.label, this.children)
      : onPressed = null,
        isDivider = false;
}

class _MenuButton extends StatelessWidget {
  final String title;
  final List<_Item> items;
  const _MenuButton({required this.title, required this.items});

  @override
  Widget build(BuildContext context) {
    return HouseButton(
      key: ValueKey<String>('menu-$title'),
      frameless: true,
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      onPressed: () => _open(context),
      child: Text(title),
    );
  }

  void _open(BuildContext context) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    final origin = box.localToGlobal(Offset(0, box.size.height));
    showLumitPopup<void>(
      context: context,
      position: origin,
      builder: (close) => _MenuList(items: items, close: () => close(null)),
    );
  }
}

class _MenuList extends StatelessWidget {
  final List<_Item> items;
  final VoidCallback close;
  const _MenuList({required this.items, required this.close});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // A long menu on a short window would otherwise run off the bottom, where
    // the last items cannot be clicked at all — so it scrolls once it no longer
    // fits. `- 40` leaves the menu bar itself and a margin.
    final maxHeight =
        (MediaQuery.of(context).size.height - 40).clamp(80.0, 1e6);
    return FloatSurface(
      width: 230,
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: maxHeight),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final item in items)
                if (item.isDivider)
                  Container(
                    height: 1,
                    margin: const EdgeInsets.symmetric(vertical: 4),
                    color: t.hairline,
                  )
                else if (item.children case final children?)
                  SubmenuRow(
                    key: ValueKey<String>('menu-sub-${item.label}'),
                    closeParent: close,
                    submenu: (dismiss) =>
                        _MenuList(items: children, close: dismiss),
                    child: Text(item.label ?? ''),
                  )
                else
                  MenuRow(
                    onPressed: item.onPressed == null
                        ? close
                        : () {
                            close();
                            item.onPressed!();
                          },
                    child: Text(
                      item.label ?? '',
                      style: item.onPressed == null
                          ? t.body.copyWith(color: t.textDisabled)
                          : null,
                    ),
                  ),
            ],
          ),
        ),
      ),
    );
  }
}
